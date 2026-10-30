//! Chat state ported from `ChatStore.swift`, `Conversations.swift`,
//! `PromptHistory.swift`, `SecretStore.swift` and `ShortcutStore.swift`.
//!
//! Covers the Foundation-only logic:
//!
//! * [`ChatRole`] / [`ChatMessage`] — `reasoning` is persisted with the
//!   message but **never sent back**: history carries the answer only.
//! * [`wire_content_for_message`] / [`history_for_wire`] — attachment-aware
//!   wire mapping with the same capability policy as the Swift client.
//! * [`ChatStore`] — transcript, streaming-turn state (`is_streaming`,
//!   `pending_status`, `pending_reasoning`), fork/truncate/retry, and the
//!   pure typewriter-pacing helpers the UI reveal reuses.
//! * [`ConversationStore`] — one JSON file per conversation, fork-tree
//!   resolution, cross-chat full-text index (reasoning excluded), excerpts,
//!   recent list with pruning.
//! * [`PromptHistory`] — sent prompts with ↑/↓ recall.
//! * [`SecretStore`] — Windows Credential Manager by default, with migration
//!   from `%APPDATA%/PopChat/secrets.json` and a retryable fallback.
//! * [`ShortcutStore`] / [`PromptShortcut`] — `/name input` expansion.
//!
//! The egui application drives [`ChatStore::tick_reveal`] with a UI clock.
//! Full arrived text remains separate for copy, wire history and persistence.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::chat::client::{
    ChatStreamEvent, ProviderConfig, SearchEngineConfig, WebAccess, WireContent, WireMessage,
    WirePart,
};
use crate::chat::providers::{
    AttachmentCapabilities, ContentCapability, ExceptionSource, ProviderKind, ProviderStore,
};

// MARK: - App data dir

/// `%APPDATA%/PopChat` on Windows (via `APPDATA`, falling back to the
/// platform config dir) — the counterpart of macOS Application Support.
pub fn app_data_dir() -> PathBuf {
    if let Some(profile) = crate::paths::isolated_profile() {
        return profile;
    }
    if let Ok(appdata) = std::env::var("APPDATA") {
        if !appdata.is_empty() {
            return PathBuf::from(appdata).join("PopChat");
        }
    }
    if let Some(base) = directories::BaseDirs::new() {
        return base.config_dir().join("PopChat");
    }
    std::env::temp_dir().join("PopChat")
}

fn read_json_file<T: for<'de> Deserialize<'de> + Default>(path: &PathBuf) -> T {
    std::fs::read(path)
        .ok()
        .and_then(|data| serde_json::from_slice(&data).ok())
        .unwrap_or_default()
}

fn write_json_file<T: Serialize>(path: &PathBuf, value: &T) {
    if let Ok(data) = serde_json::to_vec_pretty(value) {
        if let Err(error) = crate::paths::atomic_write(path, &data) {
            crate::logf!("[store] could not save {}: {error}", path.display());
        }
    }
}

// MARK: - Attachments (data types; loading lives in attach.rs)
//
// `attach.rs` (AttachmentLoader port) constructs these; this module owns the
// data shape plus the send-time wire mapping, mirroring how `ChatStore`
// owns `wireContent` in Swift.

/// A file attached to a draft message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    pub id: Uuid,
    pub filename: String,
    pub content: AttachmentContent,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub note_kind: NoteKind,
}

impl Attachment {
    pub fn new(filename: impl Into<String>, content: AttachmentContent) -> Self {
        Attachment {
            id: Uuid::new_v4(),
            filename: filename.into(),
            content,
            note: None,
            note_kind: NoteKind::Warning,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum AttachmentContent {
    Text(String),
    Image {
        #[serde(rename = "dataURL")]
        data_url: String,
    },
    /// A PDF keeps BOTH forms: capability is a property of the
    /// provider+model picked at SEND time, so the choice between the raw
    /// bytes and the extracted text waits until [`wire_content_for_message`].
    Pdf {
        #[serde(rename = "dataURL")]
        data_url: String,
        #[serde(rename = "extractedText")]
        extracted_text: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NoteKind {
    /// Routine processing info — shown quietly, never alarming.
    Info,
    /// Lossy or suspicious extraction — shown prominently and passed to the
    /// model with the file.
    #[default]
    Warning,
}

// MARK: - ChatRole / ChatMessage

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    User,
    Assistant,
    /// UI-only row for explicit errors/warnings — never sent to the API.
    Error,
    /// UI-only row documenting tool use — never sent to the API.
    Activity,
}

impl ChatRole {
    pub fn as_wire_str(self) -> &'static str {
        match self {
            ChatRole::User => "user",
            ChatRole::Assistant => "assistant",
            ChatRole::Error => "error",
            ChatRole::Activity => "activity",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: Uuid,
    pub role: ChatRole,
    pub text: String,
    /// What actually goes over the wire when it differs from the display
    /// text (slash-command expansion). `None` means `text` is the wire text.
    #[serde(rename = "wireText", default)]
    pub wire_text: Option<String>,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    /// The model's thinking for an assistant turn, when the provider streams
    /// it. Persisted with the message (so reopening a chat keeps it) but
    /// never sent back: history carries the answer only. `None` so
    /// conversations written before this existed still decode.
    #[serde(default)]
    pub reasoning: Option<String>,
}

impl ChatMessage {
    pub fn new(role: ChatRole, text: impl Into<String>) -> Self {
        ChatMessage {
            id: Uuid::new_v4(),
            role,
            text: text.into(),
            wire_text: None,
            attachments: Vec::new(),
            reasoning: None,
        }
    }
}

// MARK: - Wire mapping

/// Inlines text attachments ahead of the typed text; images become content
/// parts. Stays a bare string when there are no typed parts, for maximum
/// provider compatibility. Capability-aware and re-decided EVERY send:
/// images are ALWAYS sent (PopChat does no image→text conversion — a
/// known-refusing model warns in the composer instead), and a PDF goes as
/// the original file exactly when the provider+model resolved to
/// `files == true`, else as its extracted text.
pub fn wire_content_for_message(
    message: &ChatMessage,
    capabilities: &AttachmentCapabilities,
) -> WireContent {
    let base = message.wire_text.as_deref().unwrap_or(&message.text);
    let mut text_blocks: Vec<String> = Vec::new();
    let mut typed_parts: Vec<WirePart> = Vec::new();
    for attachment in &message.attachments {
        // Only real warnings travel to the model (it should know data is
        // partial); routine processing notes are user-facing only.
        let suffix = match (&attachment.note_kind, &attachment.note) {
            (NoteKind::Warning, Some(note)) => format!(" — {note}"),
            _ => String::new(),
        };
        match &attachment.content {
            AttachmentContent::Text(text) => {
                text_blocks.push(format!("[File: {}{suffix}]\n{text}", attachment.filename));
            }
            AttachmentContent::Image { data_url } => {
                typed_parts.push(WirePart::image_data_url(data_url.clone()));
            }
            AttachmentContent::Pdf {
                data_url,
                extracted_text,
            } => {
                if capabilities.files == Some(true) {
                    typed_parts.push(WirePart::file_data_url(
                        attachment.filename.clone(),
                        data_url.clone(),
                    ));
                } else if !extracted_text.is_empty() {
                    text_blocks.push(format!(
                        "[File: {}{suffix}]\n{extracted_text}",
                        attachment.filename
                    ));
                } else {
                    // Never silent: a scanned PDF on a text-only path states
                    // its absence instead of contributing an empty block.
                    text_blocks.push(format!(
                        "[File: {} — a PDF with no extractable text; its content could not be included because this model does not accept PDF files directly]",
                        attachment.filename
                    ));
                }
            }
        }
    }
    let mut blocks: Vec<&str> = text_blocks.iter().map(|s| s.as_str()).collect();
    if !base.is_empty() {
        blocks.push(base);
    }
    let combined = blocks.join("\n\n");
    if typed_parts.is_empty() {
        return WireContent::Text(combined);
    }
    typed_parts.push(WirePart::text(if combined.is_empty() {
        "See the attached file(s).".to_string()
    } else {
        combined
    }));
    WireContent::Parts(typed_parts)
}

/// The request history: user + assistant rows only, mapped through
/// [`wire_content_for_message`], with the system prompt inserted first.
/// `reasoning` is excluded by construction — it is not part of `text` and
/// is never written into a [`WireMessage`].
pub fn history_for_wire(
    messages: &[ChatMessage],
    system_prompt: &str,
    capabilities: &AttachmentCapabilities,
) -> Vec<WireMessage> {
    let mut history: Vec<WireMessage> = messages
        .iter()
        .filter(|m| m.role == ChatRole::User || m.role == ChatRole::Assistant)
        .map(|m| {
            WireMessage::new(
                m.role.as_wire_str(),
                Some(wire_content_for_message(m, capabilities)),
                None,
                None,
            )
        })
        .collect();
    if !system_prompt.trim().is_empty() {
        history.insert(
            0,
            WireMessage::new(
                "system",
                Some(WireContent::Text(system_prompt.trim().to_string())),
                None,
                None,
            ),
        );
    }
    history
}

// MARK: - Web access resolution

/// The Settings search-engine choice (persisted as a string like Swift's
/// `SearchEngineChoice` raw value).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchEngine {
    #[default]
    DuckDuckGo,
    Tavily,
    Brave,
    ProviderNative,
}

impl SearchEngine {
    pub fn from_raw(raw: &str) -> Self {
        match raw {
            "tavily" => SearchEngine::Tavily,
            "brave" => SearchEngine::Brave,
            "providerNative" => SearchEngine::ProviderNative,
            _ => SearchEngine::DuckDuckGo,
        }
    }

    pub fn as_raw(self) -> &'static str {
        match self {
            SearchEngine::DuckDuckGo => "duckduckgo",
            SearchEngine::Tavily => "tavily",
            SearchEngine::Brave => "brave",
            SearchEngine::ProviderNative => "providerNative",
        }
    }
}

/// Resolves the Settings search-engine choice against the active provider,
/// falling back with an explicit warning when the choice can't apply.
/// Returns the [`WebAccess`] plus warning rows the caller appends to the
/// transcript (mirroring Swift's inline `messages.append`).
pub fn resolve_web_access(
    provider_base_url: &str,
    web_search_enabled: bool,
    engine: SearchEngine,
    engine_key: Option<&str>,
) -> (Option<WebAccess>, Vec<String>) {
    if !web_search_enabled {
        return (None, Vec::new());
    }
    let mut warnings = Vec::new();
    let keyed = |label: &str, engine_name: &str, warnings: &mut Vec<String>| -> WebAccess {
        match engine_key.filter(|k| !k.is_empty()) {
            Some(key) => WebAccess::LocalTools(SearchEngineConfig::keyed(engine_name, key)),
            None => {
                warnings.push(format!(
                    "{label} needs an API key (Settings → Web Search) — using DuckDuckGo for this message."
                ));
                WebAccess::LocalTools(SearchEngineConfig::duckduckgo())
            }
        }
    };
    let access = match engine {
        SearchEngine::DuckDuckGo => WebAccess::LocalTools(SearchEngineConfig::duckduckgo()),
        SearchEngine::Tavily => keyed("Tavily", "tavily", &mut warnings),
        SearchEngine::Brave => keyed("Brave", "brave", &mut warnings),
        SearchEngine::ProviderNative => {
            if provider_base_url.contains("openrouter") {
                WebAccess::OpenRouterPlugin
            } else {
                warnings.push(
                    "Provider-native search only works on OpenRouter — using DuckDuckGo for this message."
                        .to_string(),
                );
                WebAccess::LocalTools(SearchEngineConfig::duckduckgo())
            }
        }
    };
    (Some(access), warnings)
}

// MARK: - ChatStore

/// Session-only context naming the error row that offers "don't send X to
/// this model again".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityFailureContext {
    pub error_message_id: Uuid,
    pub provider_id: Uuid,
    pub model: String,
    pub sent_images: bool,
    pub sent_files: bool,
}

/// A turn accepted for streaming: the wire history, the resolved config,
/// and the bookkeeping [`ActiveTurn`] the caller feeds events into.
#[derive(Debug)]
pub struct ActiveTurn {
    pub assistant_id: Uuid,
    pub provider_id: Uuid,
    pub model: String,
    pub sent_images: bool,
    pub sent_files: bool,
    pub reasoning: String,
    pub reasoning_committed: bool,
}

#[derive(Debug)]
pub struct OutgoingTurn {
    pub history: Vec<WireMessage>,
    pub config: ProviderConfig,
    pub web_access: Option<WebAccess>,
    pub turn: ActiveTurn,
}

#[derive(Debug)]
pub enum SendOutcome {
    /// Validation failed; the error rows are already in the transcript.
    Aborted,
    /// The caller runs the client stream, feeds events to
    /// [`ChatStore::apply_event`], then calls [`ChatStore::finish_turn`].
    Ready(OutgoingTurn),
}

/// Validation + history-building inputs for [`ChatStore::send`].
pub struct SendOptions {
    pub web_search_enabled: bool,
    pub system_prompt: String,
    pub search_engine: SearchEngine,
    pub search_engine_key: Option<String>,
    pub chatgpt_signed_in: bool,
}

impl Default for SendOptions {
    fn default() -> Self {
        SendOptions {
            web_search_enabled: true,
            system_prompt: ChatStore::DEFAULT_SYSTEM_PROMPT.to_string(),
            search_engine: SearchEngine::DuckDuckGo,
            search_engine_key: None,
            chatgpt_signed_in: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub enum StreamingMode {
    #[default]
    #[serde(rename = "perCharacter")]
    PerCharacter,
    // Preserve the source's old preference value after its UI label rename.
    #[serde(rename = "byChunk")]
    PerSentence,
}

impl<'de> Deserialize<'de> for StreamingMode {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        Ok(if value.as_str() == Some("byChunk") {
            Self::PerSentence
        } else {
            Self::PerCharacter
        })
    }
}

/// `Reveal.swift/TextReveal.Stop`: consecutive bands counted BACKWARDS from
/// the end, newest/dimmest first. Lengths use Swift Character (graphemes).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RevealStop {
    pub length: usize,
    pub alpha: f64,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TextReveal {
    pub stops: Vec<RevealStop>,
    /// A renderer's trailing caret is not part of the fade.
    pub trailing_skip: usize,
}

impl TextReveal {
    pub fn is_empty(&self) -> bool {
        self.stops
            .iter()
            .all(|stop| stop.length == 0 || stop.alpha >= 1.0)
    }

    pub fn eased(t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        1.0 - (1.0 - t) * (1.0 - t)
    }

    /// Source default: eight quantized ease-out bands over six drain generations.
    pub fn ramp(head: usize) -> Self {
        Self::ramp_with_bands(head, 8)
    }

    pub fn ramp_with_bands(head: usize, bands: usize) -> Self {
        if head == 0 || bands == 0 {
            return Self::default();
        }
        let width = head as f64 / bands as f64;
        let mut placed = 0;
        let mut stops = Vec::new();
        for band in 0..bands {
            let up_to = ((band + 1) as f64 * width).round() as usize;
            let length = up_to.saturating_sub(placed);
            if length > 0 {
                placed = up_to;
                stops.push(RevealStop {
                    length,
                    alpha: Self::eased((band as f64 + 0.5) / bands as f64),
                });
            }
        }
        Self {
            stops,
            trailing_skip: 0,
        }
    }

    /// `TextReveal.groups`: (length, age_ms), newest first. Fully opaque
    /// groups are retained here (and skipped by paint), exactly like Swift.
    pub fn groups(groups: &[(usize, u64)], duration_ms: u64) -> Self {
        Self {
            stops: groups
                .iter()
                .filter(|(length, _)| *length > 0)
                .map(|&(length, age)| RevealStop {
                    length,
                    alpha: Self::eased(age as f64 / duration_ms.max(1) as f64),
                })
                .collect(),
            trailing_skip: 0,
        }
    }
}

/// Session-only paint snapshot; never serialized or substituted for full text.
#[derive(Debug, Clone, PartialEq)]
pub struct RevealState {
    pub message_id: Uuid,
    pub fade: TextReveal,
    /// The displayed SOURCE prefix, for fitting bands to the final segment,
    /// even when its markdown is rendered as several separate egui galleys.
    pub text: String,
}

struct RevealDrain {
    message_id: Uuid,
    displayed: String,
    mode: StreamingMode,
    next_tick: std::time::Instant,
    head: usize,
    last_step: usize,
    groups: Vec<(usize, u64)>,
    since_commit_ms: u64,
}

impl RevealDrain {
    fn new(message_id: Uuid, mode: StreamingMode, now: std::time::Instant) -> Self {
        Self {
            message_id,
            displayed: String::new(),
            mode,
            next_tick: now,
            head: 0,
            last_step: 0,
            groups: Vec::new(),
            since_commit_ms: u64::MAX,
        }
    }

    fn fade(&self) -> TextReveal {
        match self.mode {
            StreamingMode::PerCharacter => TextReveal::ramp(self.head),
            StreamingMode::PerSentence => {
                TextReveal::groups(&self.groups, ChatStore::SENTENCE_FADE_MS)
            }
        }
    }
}

pub struct ChatStore {
    pub messages: Vec<ChatMessage>,
    pub is_streaming: bool,
    /// Latest transient lifecycle note, rendered inside the waiting row
    /// while the reply is still empty. Cleared on first text / turn end.
    pub pending_status: Option<String>,
    /// The turn's thinking so far, while the reply is still empty. Same
    /// lifecycle as `pending_status`; the durable copy is committed to
    /// `ChatMessage.reasoning` separately.
    pub pending_reasoning: Option<String>,
    pub recent: Vec<ConversationMeta>,
    pub conversation_id: Uuid,
    /// Cross-chat search corpus, built once and reused until the stored set
    /// changes. Invalidated on every write/delete.
    search_corpus: Option<HashMap<Uuid, String>>,
    restore_tick: u64,
    restored_into_empty_panel: bool,
    // Fork state: `messages` holds the full resolved transcript, but a
    // forked conversation persists only its divergent tail.
    fork_parent_id: Option<Uuid>,
    fork_message_id: Option<Uuid>,
    shared_prefix_count: usize,
    /// Complete arrived snapshot, separate from the UI-clock reveal prefix.
    pub stream_target: String,
    streaming_message_id: Option<Uuid>,
    pub capability_failure: Option<CapabilityFailureContext>,
    pub prompts: PromptHistory,
    streaming_mode: StreamingMode,
    reveal_drain: Option<RevealDrain>,
}

impl ChatStore {
    /// Teaches the model the pasteable-block format the renderer
    /// recognizes. An explicitly empty value means "send no system prompt".
    pub const DEFAULT_SYSTEM_PROMPT: &'static str = "You are a helpful assistant in a small desktop chat panel. Be concise and direct.\n\nWhen a reply includes content the user will copy and reuse verbatim — a prompt, template, code snippet, configuration, or other reusable text — wrap exactly that content in a pasteable block:\n\n<pasteable title=\"Short label\">\nthe exact reusable content\n</pasteable>\n\nPasteable rules:\n- The tags go on their own lines, with a short descriptive title.\n- Inside the tags, put only the content to copy — no commentary.\n- Keep explanations outside the block, in normal prose.\n- Use pasteable blocks only for content meant to be copied; use normal markdown code fences for illustrative code.";

    /// Typewriter reveal speed (chars/s). The per-tick step floors at
    /// "drain the backlog in 6s" and triples once the stream is done.
    pub const TYPEWRITER_CHARS_PER_SECOND: usize = 180;
    pub const REVEAL_GENERATIONS: usize = 6;
    pub const SENTENCE_GAP_MS: u64 = 140;
    pub const SENTENCE_TAIL_GAP_MS: u64 = 60;
    pub const SENTENCE_FADE_MS: u64 = 350;
    /// Past this much backlog, per-sentence commits whole paragraphs rather
    /// than accelerating.
    pub const SENTENCE_PARAGRAPH_BACKLOG: usize = 1_500;

    /// Resume the most recent conversation across restarts.
    pub fn new() -> Self {
        let mut store = ChatStore {
            messages: Vec::new(),
            is_streaming: false,
            pending_status: None,
            pending_reasoning: None,
            recent: ConversationStore::list_recent(),
            conversation_id: Uuid::new_v4(),
            search_corpus: None,
            restore_tick: 0,
            restored_into_empty_panel: false,
            fork_parent_id: None,
            fork_message_id: None,
            shared_prefix_count: 0,
            stream_target: String::new(),
            streaming_message_id: None,
            capability_failure: None,
            prompts: PromptHistory::load(),
            streaming_mode: StreamingMode::default(),
            reveal_drain: None,
        };
        if let Some(latest) = store.recent.first().cloned() {
            if let Some(loaded) = ConversationStore::load_resolved(latest.id) {
                store.adopt(loaded.conversation, loaded.messages, loaded.missing_parent);
            }
        }
        store
    }

    fn adopt(
        &mut self,
        conversation: Conversation,
        messages: Vec<ChatMessage>,
        missing_parent: bool,
    ) {
        // Announced BEFORE `messages` changes so observers reacting to the
        // list already see the restore flags.
        self.restored_into_empty_panel = self.messages.is_empty();
        self.restore_tick += 1;
        self.conversation_id = conversation.id;
        self.messages = messages;
        if missing_parent {
            self.fork_parent_id = None;
            self.fork_message_id = None;
            self.shared_prefix_count = 0;
            let notice = "This fork's parent conversation is missing — showing only the messages added after the fork.";
            if !self
                .messages
                .iter()
                .any(|m| m.role == ChatRole::Error && m.text == notice)
            {
                self.messages
                    .push(ChatMessage::new(ChatRole::Error, notice));
            }
        } else {
            self.fork_parent_id = conversation.parent_id;
            self.fork_message_id = conversation.fork_message_id;
            self.shared_prefix_count = self
                .messages
                .len()
                .saturating_sub(conversation.messages.len());
        }
    }

    pub fn restore_tick(&self) -> u64 {
        self.restore_tick
    }

    pub fn restored_into_empty_panel(&self) -> bool {
        self.restored_into_empty_panel
    }

    pub fn stream_target_length(&self) -> usize {
        self.stream_target.chars().count()
    }

    /// Resolved text of every stored conversation, for body search. Built
    /// once after any change; callers should build it off the hot path.
    pub fn searchable_text(&mut self) -> HashMap<Uuid, String> {
        if let Some(corpus) = &self.search_corpus {
            return corpus.clone();
        }
        let corpus = ConversationStore::full_text_index();
        self.search_corpus = Some(corpus.clone());
        corpus
    }

    /// The error-row action: records the user's own diagnosis as a manual
    /// exception for the provider+model that just failed.
    pub fn record_manual_capability_exception(
        &mut self,
        capability: ContentCapability,
        providers: &mut ProviderStore,
    ) {
        let Some(context) = self.capability_failure.clone() else {
            return;
        };
        providers.record_capability_exception(
            capability,
            false,
            ExceptionSource::Manual,
            context.provider_id,
            &context.model,
        );
        self.capability_failure = None;
    }

    pub fn send(
        &mut self,
        text: &str,
        attachments: Vec<Attachment>,
        providers: &mut ProviderStore,
        shortcuts: &ShortcutStore,
        secrets: &SecretStore,
        options: &SendOptions,
    ) -> SendOutcome {
        let text = text.trim();
        if (text.is_empty() && attachments.is_empty()) || self.is_streaming {
            return SendOutcome::Aborted;
        }
        // Recorded here rather than in the composer: this is where a prompt
        // becomes a SENT prompt, so ↑ also recalls retries.
        self.prompts.record(text);
        // A previous reply may still be revealing; complete it instantly.
        self.flush_reveal();

        // Slash-command expansion: display stays compact, the template goes
        // on the wire.
        let mut wire_text: Option<String> = None;
        if let Some(body) = text.strip_prefix('/') {
            let name_end = body.find(char::is_whitespace).unwrap_or(body.len());
            let (name, rest) = body.split_at(name_end);
            let input = rest.trim();
            match shortcuts.find(name) {
                None => {
                    self.messages.push(ChatMessage::new(ChatRole::User, text));
                    self.messages.push(ChatMessage::new(
                        ChatRole::Error,
                        format!(
                            "Unknown shortcut “/{name}”. Define it in Settings → Slash Commands, or escape the leading slash."
                        ),
                    ));
                    self.persist();
                    return SendOutcome::Aborted;
                }
                Some(shortcut) => {
                    wire_text = Some(shortcut.expand(input));
                }
            }
        }

        let mut user_message = ChatMessage::new(ChatRole::User, text);
        user_message.wire_text = wire_text;
        user_message.attachments = attachments;
        self.messages.push(user_message);

        // Explicit-warning policy: catch misconfiguration up front.
        let Some(config) = providers.current_config(secrets) else {
            self.messages.push(ChatMessage::new(
                ChatRole::Error,
                "No provider selected — open Settings…",
            ));
            self.persist();
            return SendOutcome::Aborted;
        };
        let provider_name = providers
            .selected_provider()
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "provider".to_string());
        if config.model.is_empty() {
            self.messages.push(ChatMessage::new(
                ChatRole::Error,
                format!(
                    "No model set for {provider_name}. Pick one from the model menu (fetch the list first) or Settings."
                ),
            ));
            self.persist();
            return SendOutcome::Aborted;
        }
        match config.kind {
            ProviderKind::ChatGPT => {
                if !options.chatgpt_signed_in {
                    self.messages.push(ChatMessage::new(
                        ChatRole::Error,
                        "Not signed in to ChatGPT. Open Settings… → Providers → Sign in with ChatGPT.",
                    ));
                    self.persist();
                    return SendOutcome::Aborted;
                }
            }
            ProviderKind::CodexAppServer => {
                // No pre-flight on purpose: the backend reports a missing
                // install / missing login as an error EVENT through the same
                // path this branch would use.
            }
            ProviderKind::OpenAICompatible => {
                let is_local =
                    config.base_url.contains("localhost") || config.base_url.contains("127.0.0.1");
                if config.api_key.is_empty() && !is_local {
                    self.messages.push(ChatMessage::new(
                        ChatRole::Error,
                        format!(
                            "No API key for {provider_name}. Add one in Settings → Providers, or pick a configured provider from the model menu."
                        ),
                    ));
                    self.persist();
                    return SendOutcome::Aborted;
                }
            }
        }

        // Codex owns its own tool loop, so client-side tools can't be
        // injected into it — the globe instead switches Codex's NATIVE
        // web_search on at launch.
        let web_access = if config.kind == ProviderKind::CodexAppServer {
            options
                .web_search_enabled
                .then_some(WebAccess::CodexNativeSearch)
        } else {
            let (access, warnings) = resolve_web_access(
                &config.base_url,
                options.web_search_enabled,
                options.search_engine,
                options.search_engine_key.as_deref(),
            );
            for warning in warnings {
                self.messages
                    .push(ChatMessage::new(ChatRole::Error, warning));
            }
            access
        };

        // Capability is resolved at SEND time against the live
        // provider+model.
        let send_provider_id = providers.selected_id;
        let capabilities = providers.attachment_capabilities(send_provider_id, &config.model);
        self.capability_failure = None;

        let history = history_for_wire(&self.messages, &options.system_prompt, &capabilities);
        // What this request actually carries, for the unattributed-failure
        // action on the error row.
        let mut sent_images = false;
        let mut sent_files = false;
        for message in &history {
            if let Some(WireContent::Parts(parts)) = &message.content {
                for part in parts {
                    if part.part_type == "image_url" {
                        sent_images = true;
                    }
                    if part.part_type == "file" {
                        sent_files = true;
                    }
                }
            }
        }

        self.persist();
        let assistant = ChatMessage::new(ChatRole::Assistant, "");
        let assistant_id = assistant.id;
        self.messages.push(assistant);
        self.is_streaming = true;
        self.pending_status = None;
        self.stream_target = String::new();
        self.streaming_message_id = Some(assistant_id);

        SendOutcome::Ready(OutgoingTurn {
            history,
            config: config.clone(),
            web_access,
            turn: ActiveTurn {
                assistant_id,
                provider_id: send_provider_id,
                model: config.model.clone(),
                sent_images,
                sent_files,
                reasoning: String::new(),
                reasoning_committed: false,
            },
        })
    }

    /// Folds one stream event into the transcript. The streaming assistant
    /// row stays last; activity rows are inserted above it.
    pub fn apply_event(
        &mut self,
        turn: &mut ActiveTurn,
        event: ChatStreamEvent,
        providers: &mut ProviderStore,
    ) {
        match event {
            ChatStreamEvent::Partial(text) | ChatStreamEvent::Done(text) => {
                self.accept_arrived_text(turn.assistant_id, &text);
                if !text.is_empty() {
                    self.pending_status = None;
                    self.pending_reasoning = None;
                    // Hand the thinking over to the message's own disclosure
                    // as the waiting row stops showing it. Once per turn.
                    if !turn.reasoning.is_empty() && !turn.reasoning_committed {
                        turn.reasoning_committed = true;
                        self.commit_reasoning(&turn.reasoning, turn.assistant_id);
                    }
                }
            }
            ChatStreamEvent::Status(text) => {
                self.pending_status = Some(text);
            }
            ChatStreamEvent::Reasoning(text) => {
                // The visible copy only — NOT a write into `messages`
                // (publishing the array per chunk would re-diff every row
                // many times a second). Committed once, where the answer
                // takes over.
                turn.reasoning = text.clone();
                self.pending_reasoning = Some(text);
            }
            ChatStreamEvent::Activity(text) => {
                let row = ChatMessage::new(ChatRole::Activity, text);
                match self.messages.iter().position(|m| m.id == turn.assistant_id) {
                    Some(index) => self.messages.insert(index, row),
                    None => self.messages.push(row),
                }
            }
            ChatStreamEvent::Error(message) => {
                let row = ChatMessage::new(ChatRole::Error, message);
                // The rejection wasn't attributable, but the request carried
                // typed parts — offer the manual exception on this row.
                if turn.sent_images || turn.sent_files {
                    self.capability_failure = Some(CapabilityFailureContext {
                        error_message_id: row.id,
                        provider_id: turn.provider_id,
                        model: turn.model.clone(),
                        sent_images: turn.sent_images,
                        sent_files: turn.sent_files,
                    });
                }
                self.messages.push(row);
            }
            ChatStreamEvent::ContentRejected {
                capability,
                message,
            } => {
                providers.record_capability_exception(
                    capability,
                    false,
                    ExceptionSource::Learned,
                    turn.provider_id,
                    &turn.model,
                );
                let noun = if capability == ContentCapability::Images {
                    "images"
                } else {
                    "PDF files"
                };
                self.messages.push(ChatMessage::new(
                    ChatRole::Error,
                    format!(
                        "{message}\n\nRecorded: {} didn't accept {noun} — the composer will warn before sending it {noun} again. Clear this in Settings → Providers if the rejection was transient.",
                        turn.model
                    ),
                ));
            }
        }
    }

    fn accept_arrived_text(&mut self, message_id: Uuid, text: &str) {
        self.stream_target = text.to_owned();
        if self.reveal_drain.is_none() {
            self.reveal_drain = Some(RevealDrain::new(
                message_id,
                self.streaming_mode,
                std::time::Instant::now(),
            ));
        }
        // The visible prefix belongs ONLY to the drain. Keep arrived partials
        // in the durable transcript too: persist/history must not save an empty
        // or half-revealed assistant when the app is interrupted.
        if let Some(message) = self.messages.iter_mut().find(|m| m.id == message_id) {
            message.text = text.to_owned();
        }
    }

    /// Ends the network turn: hands the composer back immediately (the UI
    /// reveal may still be animating), commits the arrived text + thinking
    /// onto the assistant row by id, and persists.
    pub fn finish_turn(&mut self, turn: &ActiveTurn) {
        // "Still ours": flush_reveal() clears streaming_message_id when a
        // new turn takes over mid-reveal.
        let still_ours = self.streaming_message_id == Some(turn.assistant_id);
        let final_text = if still_ours {
            self.stream_target.clone()
        } else {
            // A stopped/previous turn must never copy the NEXT turn's target.
            self.messages
                .iter()
                .find(|m| m.id == turn.assistant_id)
                .map(|m| m.text.clone())
                .unwrap_or_default()
        };
        if still_ours || self.streaming_message_id.is_none() {
            self.is_streaming = false;
            self.pending_status = None;
            self.pending_reasoning = None;
        }
        if let Some(index) = self.messages.iter().position(|m| m.id == turn.assistant_id) {
            if final_text.is_empty() {
                self.messages.remove(index);
            } else {
                self.messages[index].text = final_text;
                // Also runs on Stop, so a stopped turn keeps the reasoning
                // it had produced.
                if !turn.reasoning.is_empty() {
                    self.messages[index].reasoning = Some(turn.reasoning.clone());
                }
            }
        }
        if still_ours {
            self.streaming_message_id = None;
        }
        self.persist();
    }

    /// Writes a turn's thinking onto its assistant row, by id — a new turn
    /// can already have appended rows by the time this runs.
    fn commit_reasoning(&mut self, reasoning: &str, message_id: Uuid) {
        if let Some(index) = self.messages.iter().position(|m| m.id == message_id) {
            self.messages[index].reasoning = Some(reasoning.to_string());
        }
    }

    /// Ends any reveal still in flight, showing the arrived text in full.
    /// Called by `stop` and at the head of `send`.
    fn flush_reveal(&mut self) {
        if let Some(id) = self.streaming_message_id {
            if let Some(index) = self.messages.iter().position(|m| m.id == id) {
                self.messages[index].text = self.stream_target.clone();
            }
        }
        self.streaming_message_id = None;
        self.reveal_drain = None;
    }

    pub fn stop(&mut self) {
        // Don't wait for the cancelled task's tail — the waiting row must
        // not keep pulsing after the user said stop.
        self.pending_status = None;
        self.pending_reasoning = None;
        self.flush_reveal();
    }

    /// The complete text of a message: for the row still revealing,
    /// everything that has ARRIVED rather than what is displayed so far.
    pub fn full_text(&self, message_id: Uuid) -> String {
        if Some(message_id) == self.streaming_message_id && !self.stream_target.is_empty() {
            return self.stream_target.clone();
        }
        self.messages
            .iter()
            .find(|m| m.id == message_id)
            .map(|m| m.text.clone())
            .unwrap_or_default()
    }

    pub fn set_streaming_mode(&mut self, mode: StreamingMode) {
        self.streaming_mode = mode;
    }

    /// The transcript's visible prefix. Copy/history/request callers must keep
    /// using full_text, just as Swift's fullText(of:) bypasses the drain.
    pub fn display_text(&self, message_id: Uuid) -> String {
        if let Some(drain) = &self.reveal_drain {
            if drain.message_id == message_id {
                return drain.displayed.clone();
            }
        }
        self.full_text(message_id)
    }

    /// Paint-only alpha bands for this row. Continue calling tick_reveal even
    /// after display_text == full_text: a network pause also runs out the fade.
    pub fn reveal_for(&self, message_id: Uuid) -> Option<RevealState> {
        let drain = self
            .reveal_drain
            .as_ref()
            .filter(|d| d.message_id == message_id)?;
        let fade = drain.fade();
        (!fade.is_empty()).then(|| RevealState {
            message_id,
            fade,
            text: drain.displayed.clone(),
        })
    }

    /// UI-clock implementation of Swift startDrain. Persisted messages keep
    /// the complete network result even while its visible tail catches up.
    pub fn tick_reveal(&mut self, now: std::time::Instant) -> Option<std::time::Duration> {
        use unicode_segmentation::UnicodeSegmentation;
        let drain = self.reveal_drain.as_mut()?;
        if !self
            .messages
            .iter()
            .any(|message| message.id == drain.message_id)
        {
            self.reveal_drain = None;
            return None;
        }
        if now < drain.next_tick {
            return Some(drain.next_tick - now);
        }
        let finished = !self.is_streaming;
        let target_len = self.stream_target.graphemes(true).count();
        let revealed = drain.displayed.graphemes(true).count();
        // The fade clock is adaptive in BOTH modes. Sentence gap is a commit
        // gate, not the sleep period (ChatStore.swift startDrain lines 498/525).
        let interval = Self::typewriter_interval_ms(target_len);
        // Swift hasPrefix is Character-aware. A newly arrived combining mark
        // or ZWJ can extend the last previously displayed grapheme without
        // changing the count; a byte-prefix check would strand that old glyph.
        let is_character_prefix = self.stream_target.starts_with(&drain.displayed)
            && self
                .stream_target
                .graphemes(true)
                .take(revealed)
                .map(str::len)
                .sum::<usize>()
                == drain.displayed.len();
        if !is_character_prefix {
            // Tool rounds/retries can replace snapshots, not only append.
            drain.displayed = self.stream_target.clone();
            drain.head = 0;
            drain.groups.clear();
        } else if revealed >= target_len {
            // Do NOT strand the trailing ramp when the network pauses or when
            // the final target arrives. It advances six more generations.
            drain.head = drain.head.saturating_sub(drain.last_step.max(1));
        } else if drain.mode == StreamingMode::PerCharacter {
            let step = Self::typewriter_step(target_len - revealed, interval, finished);
            drain.displayed = self
                .stream_target
                .graphemes(true)
                .take(revealed + step)
                .collect();
            drain.last_step = step;
            drain.head = (revealed + step).min(step * Self::REVEAL_GENERATIONS);
        } else if drain.since_commit_ms
            >= if finished {
                Self::SENTENCE_TAIL_GAP_MS
            } else {
                Self::SENTENCE_GAP_MS
            }
        {
            let commit = Self::sentence_commit_length(&self.stream_target, revealed, finished);
            if commit > revealed {
                drain.displayed = self.stream_target.graphemes(true).take(commit).collect();
                drain.groups.insert(0, (commit - revealed, 0));
                drain.since_commit_ms = 0;
            }
        }
        if drain.mode == StreamingMode::PerSentence {
            for (_, age) in &mut drain.groups {
                *age = age.saturating_add(interval);
            }
            drain
                .groups
                .retain(|(_, age)| *age < Self::SENTENCE_FADE_MS);
            drain.since_commit_ms = drain.since_commit_ms.saturating_add(interval);
        }
        if finished
            && drain.displayed == self.stream_target
            && drain.head == 0
            && drain.groups.is_empty()
        {
            self.reveal_drain = None;
            return None;
        }
        let delay = std::time::Duration::from_millis(interval);
        drain.next_tick = now + delay;
        Some(delay)
    }

    /// Tick period in ms. Each tick re-lays-out the growing message, and
    /// that cost scales with its length.
    pub fn typewriter_interval_ms(target_len: usize) -> u64 {
        if target_len > 12_000 {
            100
        } else if target_len > 4_000 {
            66
        } else {
            33
        }
    }

    /// Characters to reveal on one tick. Paces at a FIXED rate while the
    /// answer is still arriving and lets the backlog grow — any catch-up
    /// term proportional to the backlog settles where reveal rate EQUALS
    /// arrival rate, which destroys the typewriter feel. Floored at "drain
    /// the backlog in 6s", tripled once the stream is done.
    pub fn typewriter_step(backlog: usize, interval_ms: u64, finished: bool) -> usize {
        let mut rate = std::cmp::max(Self::TYPEWRITER_CHARS_PER_SECOND, backlog / 6);
        if finished {
            rate *= 3;
        }
        backlog.min(std::cmp::max(1, rate * interval_ms as usize / 1000))
    }

    /// Character index the per-sentence reveal may commit up to — the end of
    /// a sentence, line, or paragraph — or `revealed` when the text has not
    /// reached a boundary yet, so a half-sentence never shows. Positions
    /// inside an OPEN code fence are never boundaries. Once the stream is
    /// over, the end of the text is ALWAYS a boundary.
    ///
    /// Counts extended grapheme clusters, matching Swift Character rather
    /// than splitting combining marks or emoji ZWJ sequences.
    pub fn sentence_commit_length(text: &str, revealed: usize, finished: bool) -> usize {
        use unicode_segmentation::UnicodeSegmentation;
        let chars: Vec<&str> = text.graphemes(true).collect();
        if revealed >= chars.len() {
            return revealed;
        }
        let mut boundaries: Vec<usize> = Vec::new();
        let mut paragraph_ends: HashSet<usize> = HashSet::new();
        let mut inside_fence = false;
        let mut at_line_start = true;
        let mut index = 0;
        while index < chars.len() {
            if at_line_start
                && chars
                    .get(index..)
                    .map(|s| s.starts_with(&["`", "`", "`"][..]))
                    .unwrap_or(false)
            {
                inside_fence = !inside_fence;
            }
            at_line_start = false;
            let char = chars[index];
            if char == "\n" || char == "\r\n" {
                at_line_start = true;
                if !inside_fence {
                    boundaries.push(index + 1);
                    if index + 1 < chars.len() && matches!(chars[index + 1], "\n" | "\r\n") {
                        paragraph_ends.insert(index + 1);
                    } else if index + 1 == chars.len() {
                        paragraph_ends.insert(index + 1);
                    }
                }
            } else if !inside_fence && matches!(char, "." | "!" | "?") {
                let next = index + 1;
                if next == chars.len() {
                    if finished {
                        boundaries.push(next);
                    }
                } else if chars[next].chars().all(char::is_whitespace) {
                    boundaries.push(next);
                }
            }
            index += 1;
        }
        if finished && boundaries.last() != Some(&chars.len()) {
            boundaries.push(chars.len());
            paragraph_ends.insert(chars.len());
        }
        let pending: Vec<usize> = boundaries.into_iter().filter(|b| *b > revealed).collect();
        let Some(&last) = pending.last() else {
            return revealed;
        };
        let backlog = chars.len() - revealed;
        if backlog > Self::SENTENCE_PARAGRAPH_BACKLOG {
            return pending
                .iter()
                .rev()
                .find(|b| paragraph_ends.contains(b))
                .copied()
                .unwrap_or(last);
        }
        let beats = std::cmp::max(1, 6_000 / Self::SENTENCE_GAP_MS as usize);
        let per_beat = std::cmp::max(1, (pending.len() + beats - 1) / beats);
        let candidate = pending[per_beat.min(pending.len()) - 1];
        // Groups never span a paragraph break — units stay semantic.
        pending
            .iter()
            .find(|b| paragraph_ends.contains(b) && **b < candidate)
            .copied()
            .unwrap_or(candidate)
    }

    /// Start a new conversation sharing history up to and including
    /// `message_id` — stored as a tree branch (parent pointer + divergent
    /// tail only), so the shared prefix is never duplicated on disk.
    pub fn fork(&mut self, message_id: Uuid) {
        if self.is_streaming {
            return;
        }
        let Some(index) = self.messages.iter().position(|m| m.id == message_id) else {
            return;
        };
        self.flush_reveal();
        self.persist(); // parent must be current before the branch points at it
        self.fork_parent_id = Some(self.conversation_id);
        self.fork_message_id = Some(message_id);
        self.conversation_id = Uuid::new_v4();
        self.shared_prefix_count = index + 1;
        self.messages.truncate(index + 1);
        self.messages.push(ChatMessage::new(
            ChatRole::Activity,
            "Forked here — this branch diverges from the original conversation.",
        ));
        self.persist();
    }

    // MARK: - Re-running the last turn

    /// The last user message, if there is one and nothing is in flight.
    /// Only the LAST turn is re-runnable on purpose (revisiting an earlier
    /// point non-destructively is what Fork is for).
    fn last_user_index(&self) -> Option<usize> {
        if self.is_streaming {
            return None;
        }
        self.messages.iter().rposition(|m| m.role == ChatRole::User)
    }

    pub fn can_rerun_last_turn(&self) -> bool {
        self.last_user_index().is_some()
    }

    /// Re-send payload for the last user message, dropping whatever it
    /// produced. The caller passes it back through [`Self::send`], which
    /// re-resolves provider/model — switching the picker first is how you
    /// ask a different model the same thing.
    pub fn retry_last_turn(&mut self) -> Option<(String, Vec<Attachment>)> {
        let prompt = self.take_last_user_message_for_editing()?;
        // The display text, not `wire_text`: send() re-expands a slash
        // command from it against the CURRENT template.
        Some((prompt.text, prompt.attachments))
    }

    /// Pull the last user message out of the transcript for editing,
    /// dropping it and its reply.
    pub fn take_last_user_message_for_editing(&mut self) -> Option<ChatMessage> {
        let index = self.last_user_index()?;
        let prompt = self.messages[index].clone();
        self.truncate(index);
        Some(prompt)
    }

    /// Cut the transcript down to its first `count` messages. Materializes
    /// direct children first (never-orphan-a-fork), and a cut reaching into
    /// the shared prefix makes this conversation standalone.
    fn truncate(&mut self, count: usize) {
        if count >= self.messages.len() {
            return;
        }
        self.stop();
        self.persist();
        ConversationStore::materialize_children(self.conversation_id);
        if count < self.shared_prefix_count {
            self.fork_parent_id = None;
            self.fork_message_id = None;
            self.shared_prefix_count = 0;
        }
        self.messages.truncate(count);
        if self.messages.is_empty() {
            // persist() skips empty conversations, so delete explicitly or
            // the file keeps serving the pre-truncation transcript.
            ConversationStore::delete(self.conversation_id);
            self.recent.retain(|m| m.id != self.conversation_id);
        } else {
            self.persist();
        }
    }

    pub fn delete_conversation(&mut self, id: Uuid) {
        self.search_corpus = None;
        ConversationStore::delete_materializing_children(id);
        self.recent = ConversationStore::list_recent();
        if id == self.conversation_id {
            self.stop();
            self.conversation_id = Uuid::new_v4();
            self.messages.clear();
            self.fork_parent_id = None;
            self.fork_message_id = None;
            self.shared_prefix_count = 0;
        } else if Some(id) == self.fork_parent_id {
            // Our file was just materialized as standalone; align with it.
            self.fork_parent_id = None;
            self.fork_message_id = None;
            self.shared_prefix_count = 0;
        }
    }

    pub fn new_chat(&mut self) {
        self.stop();
        self.persist();
        self.conversation_id = Uuid::new_v4();
        self.messages.clear();
        self.fork_parent_id = None;
        self.fork_message_id = None;
        self.shared_prefix_count = 0;
    }

    pub fn load_conversation(&mut self, id: Uuid) {
        if id == self.conversation_id {
            return;
        }
        self.stop();
        self.persist();
        self.complete_conversation_load(id, ConversationStore::load_resolved(id));
    }

    /// UI resolves files on a background worker. Its generation fence must
    /// accept the result before calling this common adoption/error boundary.
    pub fn complete_conversation_load(&mut self, id: Uuid, loaded: Option<ResolvedConversation>) {
        match loaded {
            Some(loaded) => self.adopt(loaded.conversation, loaded.messages, loaded.missing_parent),
            None => {
                self.recent.retain(|m| m.id != id);
                self.messages.push(ChatMessage::new(
                    ChatRole::Error,
                    "Couldn't load that conversation — its file is missing or corrupt.",
                ));
            }
        }
    }

    /// Writes the current conversation to disk (a fork stores only its
    /// divergent tail) and refreshes its slot in the recent list. No-op for
    /// empty conversations (avoids junk files).
    pub fn persist(&mut self) {
        if self.messages.is_empty() {
            return;
        }
        self.search_corpus = None; // the stored set changed; rebuild on next search
        let title = Self::title_for(&self.messages);
        let conversation = Conversation {
            id: self.conversation_id,
            title: title.clone(),
            updated_at: Utc::now(),
            messages: self.messages[self.shared_prefix_count.min(self.messages.len())..].to_vec(),
            parent_id: self.fork_parent_id,
            fork_message_id: self.fork_message_id,
        };
        ConversationStore::save(&conversation);
        self.recent.retain(|m| m.id != self.conversation_id);
        self.recent.insert(
            0,
            ConversationMeta {
                id: self.conversation_id,
                title,
                updated_at: conversation.updated_at,
                snippet: ConversationMeta::snippet_for(&self.messages),
                is_fork: self.fork_parent_id.is_some(),
            },
        );
    }

    fn title_for(messages: &[ChatMessage]) -> String {
        let first = messages
            .iter()
            .find(|m| m.role == ChatRole::User)
            .map(|m| m.text.clone())
            .unwrap_or_else(|| "New chat".to_string());
        let flattened: String = first
            .chars()
            .map(|c| if c == '\n' { ' ' } else { c })
            .collect();
        if flattened.chars().count() > 40 {
            let prefix: String = flattened.chars().take(40).collect();
            format!("{prefix}…")
        } else {
            flattened
        }
    }
}

impl Default for ChatStore {
    fn default() -> Self {
        ChatStore::new()
    }
}

// MARK: - Conversations

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    pub id: Uuid,
    pub title: String,
    pub updated_at: DateTime<Utc>,
    /// For a fork: only the messages AFTER the fork point — the shared
    /// prefix lives in the parent chain and is resolved on load.
    pub messages: Vec<ChatMessage>,
    /// Conversation this one was forked from; `None` for a root.
    #[serde(rename = "parentID", default)]
    pub parent_id: Option<Uuid>,
    /// Last shared message (inclusive) in the parent's resolved transcript.
    #[serde(rename = "forkMessageID", default)]
    pub fork_message_id: Option<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationMeta {
    pub id: Uuid,
    pub title: String,
    pub updated_at: DateTime<Utc>,
    /// One-line preview (last real message).
    pub snippet: String,
    pub is_fork: bool,
}

impl ConversationMeta {
    pub fn snippet_for(messages: &[ChatMessage]) -> String {
        let last = messages.iter().rev().find(|m| {
            (m.role == ChatRole::User || m.role == ChatRole::Assistant) && !m.text.is_empty()
        });
        let flattened: String = last
            .map(|m| m.text.clone())
            .unwrap_or_default()
            .chars()
            .map(|c| if c == '\n' { ' ' } else { c })
            .collect();
        flattened.chars().take(120).collect()
    }
}

pub struct ResolvedConversation {
    pub conversation: Conversation,
    pub messages: Vec<ChatMessage>,
    pub missing_parent: bool,
}

/// Test hook backing store: point it at a scratch directory so synthetic
/// conversations never touch the user's real history. (A free static:
/// associated `static` items are not allowed in `impl` blocks on stable.)
static CONVERSATION_OVERRIDE_DIR: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();

/// One JSON file per conversation. Forked conversations form a tree: a
/// fork's file stores a parent pointer plus only the messages added after
/// the fork. Capped at `max_stored` conversations; oldest are pruned.
pub struct ConversationStore;

impl ConversationStore {
    pub const MAX_STORED: usize = 50;

    /// Test hook: point this at a scratch directory so synthetic
    /// conversations never touch the user's real history.
    pub fn set_override_dir(dir: PathBuf) {
        Self::override_lock().lock().unwrap().replace(dir);
    }

    pub fn clear_override_dir() {
        *Self::override_lock().lock().unwrap() = None;
    }

    fn override_lock() -> &'static Mutex<Option<PathBuf>> {
        CONVERSATION_OVERRIDE_DIR.get_or_init(|| Mutex::new(None))
    }

    fn directory() -> PathBuf {
        if let Some(dir) = Self::override_lock().lock().unwrap().clone() {
            let _ = std::fs::create_dir_all(&dir);
            return dir;
        }
        let dir = app_data_dir().join("conversations");
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    fn url_for(id: Uuid) -> PathBuf {
        Self::directory().join(format!("{}.json", id.hyphenated()))
    }

    pub fn save(conversation: &Conversation) {
        if let Ok(data) = serde_json::to_vec(conversation) {
            let _ = std::fs::write(Self::url_for(conversation.id), data);
        }
    }

    pub fn load(id: Uuid) -> Option<Conversation> {
        let data = std::fs::read(Self::url_for(id)).ok()?;
        serde_json::from_slice(&data).ok()
    }

    pub fn delete(id: Uuid) {
        let _ = std::fs::remove_file(Self::url_for(id));
    }

    // MARK: - Fork resolution

    /// Full transcript: the parent chain's shared prefix plus this
    /// conversation's own tail.
    pub fn resolve_messages(conversation: &Conversation) -> (Vec<ChatMessage>, bool) {
        Self::resolve_messages_with(
            conversation,
            &|id| Self::load(id),
            &mut std::iter::once(conversation.id).collect(),
        )
    }

    fn resolve_messages_with(
        conversation: &Conversation,
        lookup: &dyn Fn(Uuid) -> Option<Conversation>,
        visited: &mut HashSet<Uuid>,
    ) -> (Vec<ChatMessage>, bool) {
        let (Some(parent_id), Some(fork_message_id)) =
            (conversation.parent_id, conversation.fork_message_id)
        else {
            return (conversation.messages.clone(), false);
        };
        if visited.contains(&parent_id) {
            return (conversation.messages.clone(), true);
        }
        let Some(parent) = lookup(parent_id) else {
            return (conversation.messages.clone(), true);
        };
        visited.insert(parent_id);
        let (parent_messages, parent_broken) =
            Self::resolve_messages_with(&parent, lookup, visited);
        let Some(cut) = parent_messages.iter().position(|m| m.id == fork_message_id) else {
            return (conversation.messages.clone(), true);
        };
        let mut resolved = parent_messages[..=cut].to_vec();
        resolved.extend(conversation.messages.iter().cloned());
        (resolved, parent_broken)
    }

    pub fn load_resolved(id: Uuid) -> Option<ResolvedConversation> {
        let conversation = Self::load(id)?;
        let (messages, missing_parent) = Self::resolve_messages(&conversation);
        Some(ResolvedConversation {
            conversation,
            messages,
            missing_parent,
        })
    }

    /// Direct children of `id` absorb the shared prefix and become
    /// standalone roots. Call before a conversation's transcript stops being
    /// able to serve them.
    pub fn materialize_children(id: Uuid) {
        let all = Self::load_all();
        for child in all.values().filter(|c| c.parent_id == Some(id)) {
            Self::materialize(child, &all);
        }
    }

    /// Delete with fork safety.
    pub fn delete_materializing_children(id: Uuid) {
        Self::materialize_children(id);
        Self::delete(id);
    }

    fn materialize(child: &Conversation, all: &HashMap<Uuid, Conversation>) {
        let mut standalone = child.clone();
        let (messages, _) = Self::resolve_messages_with(
            child,
            &|id| all.get(&id).cloned(),
            &mut std::iter::once(child.id).collect(),
        );
        standalone.messages = messages;
        standalone.parent_id = None;
        standalone.fork_message_id = None;
        Self::save(&standalone);
    }

    fn load_all_resolved() -> (HashMap<Uuid, Conversation>, HashMap<Uuid, Vec<ChatMessage>>) {
        let all = Self::load_all();
        let mut resolved = HashMap::new();
        for conversation in all.values() {
            if resolved.contains_key(&conversation.id) {
                continue;
            }
            let (messages, _) = Self::resolve_messages_with(
                conversation,
                &|id| all.get(&id).cloned(),
                &mut std::iter::once(conversation.id).collect(),
            );
            resolved.insert(conversation.id, messages);
        }
        (all, resolved)
    }

    fn load_all() -> HashMap<Uuid, Conversation> {
        let mut result = HashMap::new();
        let entries = std::fs::read_dir(Self::directory()).ok();
        for entry in entries.into_iter().flatten().flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Ok(data) = std::fs::read(&path) else {
                continue;
            };
            let Ok(conversation) = serde_json::from_slice::<Conversation>(&data) else {
                continue;
            };
            result.insert(conversation.id, conversation);
        }
        result
    }

    // MARK: - Cross-conversation search

    /// Every stored conversation's resolved transcript, flattened to one
    /// searchable string each. Reasoning is excluded: it lives behind a
    /// collapsed disclosure, so a hit there would send the user to a chat
    /// with nothing visibly matching.
    pub fn full_text_index() -> HashMap<Uuid, String> {
        Self::load_all_resolved()
            .1
            .into_iter()
            .map(|(id, messages)| {
                let text = messages
                    .iter()
                    .filter(|m| m.role == ChatRole::User || m.role == ChatRole::Assistant)
                    .map(|m| m.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                (id, text)
            })
            .collect()
    }

    /// The text around the first case-insensitive hit, for a search result
    /// row: enough before it to read as a sentence, tail-padded, newlines
    /// collapsed so it stays exactly one line.
    pub fn excerpt(text: &str, query: &str, context: usize) -> Option<String> {
        let (hit_start, hit_len) = case_insensitive_find(text, query)?;
        // Walk back ~`context` chars; forward ~`context * 2` chars.
        let mut start = hit_start;
        for _ in 0..context {
            if start == 0 {
                break;
            }
            match text[..start].chars().next_back() {
                Some(c) => start -= c.len_utf8(),
                None => break,
            }
        }
        let mut end = (hit_start + hit_len).min(text.len());
        let mut taken = 0;
        for c in text[end..].chars() {
            if taken >= context * 2 {
                break;
            }
            end += c.len_utf8();
            taken += 1;
        }
        let mut excerpt: String = text[start..end]
            .chars()
            .map(|c| if c == '\n' { ' ' } else { c })
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if start > 0 {
            excerpt = format!("…{excerpt}");
        }
        if end < text.len() {
            excerpt.push('…');
        }
        Some(excerpt)
    }

    /// Newest first, snippets from the fully-resolved transcript. Also
    /// prunes storage beyond `max_stored` (materializing children of pruned
    /// parents).
    pub fn list_recent() -> Vec<ConversationMeta> {
        let (all, resolved) = Self::load_all_resolved();
        let mut metas: Vec<ConversationMeta> = all
            .values()
            .map(|conversation| ConversationMeta {
                id: conversation.id,
                title: conversation.title.clone(),
                updated_at: conversation.updated_at,
                snippet: ConversationMeta::snippet_for(
                    resolved
                        .get(&conversation.id)
                        .map(|m| m.as_slice())
                        .unwrap_or(&conversation.messages),
                ),
                is_fork: conversation.parent_id.is_some(),
            })
            .collect();
        metas.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        for stale in metas.iter().skip(Self::MAX_STORED) {
            if let Some(conversation) = all.get(&stale.id) {
                for child in all
                    .values()
                    .filter(|c| c.parent_id == Some(conversation.id))
                {
                    Self::materialize(child, &all);
                }
                Self::delete(stale.id);
            }
        }
        metas.truncate(Self::MAX_STORED);
        metas
    }
}

/// Byte offset of the first case-insensitive occurrence of `query` in
/// `text`, plus the byte length of the match in `text`.
fn case_insensitive_find(text: &str, query: &str) -> Option<(usize, usize)> {
    if query.is_empty() {
        return None;
    }
    let query_lower: Vec<char> = query.chars().flat_map(|c| c.to_lowercase()).collect();
    let text_chars: Vec<(usize, char)> = text.char_indices().collect();
    if query_lower.is_empty() || text_chars.len() < query_lower.len() {
        return None;
    }
    for i in 0..=text_chars.len() - query_lower.len() {
        let matches = text_chars
            .iter()
            .skip(i)
            .take(query_lower.len())
            .map(|(_, c)| c)
            .zip(query_lower.iter())
            // Multi-char lowercase expansions (e.g. İ) only fail to match;
            // byte offsets below always come from `char_indices`, so they
            // can never misalign.
            .all(|(c, wanted)| c.to_lowercase().any(|lc| lc == *wanted));
        if matches {
            let start = text_chars[i].0;
            let end = text_chars
                .get(i + query_lower.len())
                .map(|(b, _)| *b)
                .unwrap_or(text.len());
            return Some((start, end - start));
        }
    }
    None
}

// MARK: - PromptHistory

/// The prompts you have actually sent, newest first — what ↑ walks back
/// through from an empty composer. Deliberately global rather than
/// per-conversation. Recorded by [`ChatStore::send`], not by the composer.
#[derive(Debug, Clone, Default)]
pub struct PromptHistory {
    entries: Vec<String>,
    /// Recall cursor: `None` means "sitting on the live draft".
    cursor: Option<usize>,
    /// The draft that was in the composer when recall started, restored
    /// when ↓ walks past the newest entry.
    saved_draft: String,
    /// An explicit destination lets tests exercise persistence in isolation.
    storage_path: Option<PathBuf>,
}

impl PromptHistory {
    pub const MAX_ENTRIES: usize = 50;

    fn file_path() -> PathBuf {
        app_data_dir().join("prompt_history.json")
    }

    pub fn load() -> Self {
        let entries: Vec<String> = read_json_file(&Self::file_path());
        PromptHistory {
            entries: entries.into_iter().take(Self::MAX_ENTRIES).collect(),
            cursor: None,
            saved_draft: String::new(),
            storage_path: None,
        }
    }

    /// An explicit profile file for isolated callers/tests. Never falls back
    /// to the user's global prompt history on a missing/corrupt fixture.
    pub fn with_storage_path(path: PathBuf) -> Self {
        let entries: Vec<String> = read_json_file(&path);
        Self {
            entries: entries.into_iter().take(Self::MAX_ENTRIES).collect(),
            storage_path: Some(path),
            ..Self::default()
        }
    }

    fn save(&self) {
        let path = self.storage_path.clone().unwrap_or_else(Self::file_path);
        write_json_file(&path, &self.entries);
    }

    pub fn entries(&self) -> &[String] {
        &self.entries
    }

    /// Records a sent prompt at the front, deduplicated: asking the same
    /// thing twice should not cost two ↑ presses to get past.
    pub fn record(&mut self, text: &str) {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }
        self.entries.retain(|e| e != trimmed);
        self.entries.insert(0, trimmed.to_string());
        self.entries.truncate(Self::MAX_ENTRIES);
        self.cursor = None;
        self.save();
    }

    /// ↑ from the composer: first press saves the live draft and shows the
    /// newest entry; further presses walk older.
    pub fn recall_older(&mut self, draft: &str) -> Option<String> {
        if self.entries.is_empty() {
            return None;
        }
        match self.cursor {
            None => {
                self.saved_draft = draft.to_string();
                self.cursor = Some(0);
                Some(self.entries[0].clone())
            }
            Some(i) => {
                let next = (i + 1).min(self.entries.len() - 1);
                self.cursor = Some(next);
                Some(self.entries[next].clone())
            }
        }
    }

    /// ↓: walks back toward the live draft, restoring it past the newest.
    pub fn recall_newer(&mut self) -> Option<String> {
        match self.cursor {
            None => None,
            Some(0) => {
                self.cursor = None;
                Some(self.saved_draft.clone())
            }
            Some(i) => {
                self.cursor = Some(i - 1);
                Some(self.entries[i - 1].clone())
            }
        }
    }

    pub fn reset_recall(&mut self) {
        self.cursor = None;
        self.saved_draft.clear();
    }
}

// MARK: - SecretStore

use anyhow::Context as _;

/// Which storage backs [`SecretStore`]. Windows prefers Credential Manager;
/// an existing JSON file is retained as a read-only migration fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretBackend {
    /// Explicit file storage; the default on non-Windows platforms.
    File,
    /// OS credential store via the `keyring` crate.
    Keyring,
}

impl Default for SecretBackend {
    fn default() -> Self {
        if cfg!(windows) {
            Self::Keyring
        } else {
            Self::File
        }
    }
}

/// Native operations are injectable per store so tests never replace keyring's
/// process-global builder or access the user's credentials.
trait SecretCredentials: std::fmt::Debug + Send + Sync {
    fn get(&self, account: &str) -> anyhow::Result<Option<String>>;
    fn set(&self, value: &str, account: &str) -> anyhow::Result<()>;
    /// Deleting an absent credential is successful.
    fn delete(&self, account: &str) -> anyhow::Result<()>;
}

#[derive(Debug)]
struct NativeCredentials;

#[cfg(windows)]
fn credential_account(account: &str) -> String {
    use sha2::{Digest, Sha256};
    match crate::paths::isolated_profile() {
        Some(path) => {
            // Preview/test profiles must not load or overwrite the normal
            // install's OAuth or search-engine credentials with fixed names.
            let digest = Sha256::digest(path.to_string_lossy().as_bytes());
            let namespace: String = digest[..12]
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            format!("profile-{namespace}-{account}")
        }
        None => account.to_owned(),
    }
}

#[cfg(windows)]
impl SecretCredentials for NativeCredentials {
    fn get(&self, account: &str) -> anyhow::Result<Option<String>> {
        match keyring::Entry::new("PopChat", &credential_account(account))?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn set(&self, value: &str, account: &str) -> anyhow::Result<()> {
        keyring::Entry::new("PopChat", &credential_account(account))?.set_password(value)?;
        Ok(())
    }

    fn delete(&self, account: &str) -> anyhow::Result<()> {
        match keyring::Entry::new("PopChat", &credential_account(account))?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

#[cfg(not(windows))]
impl SecretCredentials for NativeCredentials {
    fn get(&self, _account: &str) -> anyhow::Result<Option<String>> {
        anyhow::bail!("Native credential storage is only available on Windows")
    }

    fn set(&self, _value: &str, _account: &str) -> anyhow::Result<()> {
        anyhow::bail!("Native credential storage is only available on Windows")
    }

    fn delete(&self, _account: &str) -> anyhow::Result<()> {
        anyhow::bail!("Native credential storage is only available on Windows")
    }
}

/// Per-account secret storage keyed by provider id. Native writes must succeed
/// and be readable from a fresh credential entry before legacy copies are
/// removed. Failed writes never create new plaintext fallback values.
#[derive(Debug)]
pub struct SecretStore {
    backend: SecretBackend,
    path: PathBuf,
    credentials: Box<dyn SecretCredentials>,
    migration_error: Option<String>,
}

impl Default for SecretStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SecretStore {
    pub fn new() -> Self {
        Self::with_backend(SecretBackend::default())
    }

    pub fn with_backend(backend: SecretBackend) -> Self {
        Self::with_storage(
            backend,
            app_data_dir().join("secrets.json"),
            Box::new(NativeCredentials),
        )
    }

    fn with_storage(
        backend: SecretBackend,
        path: PathBuf,
        credentials: Box<dyn SecretCredentials>,
    ) -> Self {
        let mut store = SecretStore {
            backend,
            path,
            credentials,
            migration_error: None,
        };
        if backend == SecretBackend::Keyring {
            store.migration_error = store.migrate_legacy_file().err().map(|e| format!("{e:#}"));
        }
        store
    }

    /// Startup migration/cleanup failures for the caller to display. The
    /// original legacy file is retained when migration is incomplete.
    pub fn migration_error(&self) -> Option<&str> {
        self.migration_error.as_deref()
    }

    fn load_file(&self) -> anyhow::Result<HashMap<String, String>> {
        let data = match std::fs::read(&self.path) {
            Ok(data) => data,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(HashMap::new());
            }
            Err(error) => return Err(error).context("Could not read legacy secret storage"),
        };
        serde_json::from_slice(&data).context("Could not decode legacy secret storage")
    }

    /// File-backend writes and legacy cleanup replace the file atomically, so
    /// an unsuccessful write cannot truncate other accounts' only copies.
    fn save_file(&self, secrets: &HashMap<String, String>) -> anyhow::Result<()> {
        use std::io::Write;

        if secrets.is_empty() {
            return match std::fs::remove_file(&self.path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error).context("Could not remove legacy secret storage"),
            };
        }
        let parent = self
            .path
            .parent()
            .context("Secret storage has no parent directory")?;
        std::fs::create_dir_all(parent).context("Could not create secret storage directory")?;
        let data = serde_json::to_vec_pretty(secrets)?;
        let temporary = parent.join(format!(".secrets-{}.tmp", Uuid::new_v4()));
        let result = (|| -> anyhow::Result<()> {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temporary)?;
            file.write_all(&data)?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&temporary, &self.path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result.context("Could not save secret storage")
    }

    fn persist_native(&self, value: &str, account: &str) -> anyhow::Result<()> {
        self.credentials
            .set(value, account)
            .context("Could not save secret to Windows Credential Manager")?;
        let saved = self
            .credentials
            .get(account)
            .context("Could not verify the saved credential")?;
        anyhow::ensure!(
            saved.as_deref() == Some(value),
            "The credential store did not retain the saved secret"
        );
        Ok(())
    }

    fn migrate_legacy_file(&self) -> anyhow::Result<()> {
        let legacy = self.load_file()?;
        let mut failures = Vec::new();
        for (account, value) in &legacy {
            // Native credentials are authoritative. A stale file left behind
            // by failed cleanup must never roll back a newer native value.
            let result = match self.credentials.get(account) {
                Ok(Some(_)) => Ok(()),
                Ok(None) => self.persist_native(value, account),
                Err(error) => Err(error).context("Could not check existing native credential"),
            };
            if let Err(error) = result {
                failures.push(format!("{account}: {error:#}"));
            }
        }
        // Leave the entire original file intact until every account has a
        // verified durable copy. Successful accounts are skipped on retry.
        anyhow::ensure!(
            failures.is_empty(),
            "Legacy secrets were retained; migration failed: {}",
            failures.join("; ")
        );
        if !legacy.is_empty() {
            self.save_file(&HashMap::new())?;
        }
        Ok(())
    }

    pub fn get(&self, account: &str) -> Option<String> {
        match self.backend {
            SecretBackend::File => self.load_file().ok()?.get(account).cloned(),
            SecretBackend::Keyring => self.credentials.get(account).ok().flatten().or_else(|| {
                // Only pre-existing legacy values can be used as fallback;
                // failed user edits are never written into this file.
                self.load_file().ok()?.get(account).cloned()
            }),
        }
    }

    pub fn set(&mut self, value: &str, account: &str) -> anyhow::Result<()> {
        if value.is_empty() {
            return self.delete(account);
        }
        match self.backend {
            SecretBackend::File => {
                let mut secrets = self.load_file()?;
                secrets.insert(account.to_owned(), value.to_owned());
                self.save_file(&secrets)
            }
            SecretBackend::Keyring => {
                self.persist_native(value, account)?;
                self.remove_legacy_secret(account)
                    .context("Secret saved, but its legacy plaintext copy could not be removed")
            }
        }
    }

    pub fn delete(&mut self, account: &str) -> anyhow::Result<()> {
        // Remove the legacy source first. If cleanup fails, keeping the native
        // credential prevents a future migration from resurrecting an old key.
        self.remove_legacy_secret(account)?;
        if self.backend == SecretBackend::Keyring {
            self.credentials
                .delete(account)
                .context("Could not delete secret from Windows Credential Manager")?;
            anyhow::ensure!(
                self.credentials
                    .get(account)
                    .context("Could not verify credential deletion")?
                    .is_none(),
                "The credential store still contains the deleted secret"
            );
        }
        Ok(())
    }

    fn remove_legacy_secret(&self, account: &str) -> anyhow::Result<()> {
        let mut secrets = self.load_file()?;
        if secrets.remove(account).is_some() {
            self.save_file(&secrets)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod secret_tests {
    use super::*;
    use std::sync::Arc;

    #[derive(Debug, Default)]
    struct MemoryState {
        values: HashMap<String, String>,
        fail_reads: bool,
        fail_writes: HashSet<String>,
        discard_writes: bool,
        fail_deletes: bool,
        discard_deletes: bool,
        writes: Vec<String>,
    }

    #[derive(Debug, Clone, Default)]
    struct MemoryCredentials(Arc<Mutex<MemoryState>>);

    impl SecretCredentials for MemoryCredentials {
        fn get(&self, account: &str) -> anyhow::Result<Option<String>> {
            let state = self.0.lock().unwrap();
            anyhow::ensure!(!state.fail_reads, "injected credential read failure");
            Ok(state.values.get(account).cloned())
        }

        fn set(&self, value: &str, account: &str) -> anyhow::Result<()> {
            let mut state = self.0.lock().unwrap();
            anyhow::ensure!(
                !state.fail_writes.contains(account),
                "injected credential write failure"
            );
            state.writes.push(account.to_owned());
            if !state.discard_writes {
                state.values.insert(account.to_owned(), value.to_owned());
            }
            Ok(())
        }

        fn delete(&self, account: &str) -> anyhow::Result<()> {
            let mut state = self.0.lock().unwrap();
            anyhow::ensure!(!state.fail_deletes, "injected credential delete failure");
            if !state.discard_deletes {
                state.values.remove(account);
            }
            Ok(())
        }
    }

    struct Fixture {
        _directory: tempfile::TempDir,
        path: PathBuf,
        credentials: MemoryCredentials,
    }

    impl Fixture {
        fn new() -> Self {
            let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("target")
                .join("secret-store-tests");
            std::fs::create_dir_all(&base).unwrap();
            let directory = tempfile::tempdir_in(base).unwrap();
            let path = directory.path().join("secrets.json");
            Fixture {
                _directory: directory,
                path,
                credentials: MemoryCredentials::default(),
            }
        }

        fn store(&self, backend: SecretBackend) -> SecretStore {
            SecretStore::with_storage(
                backend,
                self.path.clone(),
                Box::new(self.credentials.clone()),
            )
        }

        fn legacy(&self, entries: &[(&str, &str)]) -> Vec<u8> {
            let entries: HashMap<_, _> = entries.iter().copied().collect();
            let bytes = serde_json::to_vec_pretty(&entries).unwrap();
            std::fs::write(&self.path, &bytes).unwrap();
            bytes
        }

        fn seed_native(&self, account: &str, value: &str) {
            self.credentials
                .0
                .lock()
                .unwrap()
                .values
                .insert(account.to_owned(), value.to_owned());
        }
    }

    #[test]
    fn platform_backend_uses_persistent_native_credentials_on_windows() {
        #[cfg(windows)]
        {
            assert_eq!(SecretBackend::default(), SecretBackend::Keyring);
            assert!(matches!(
                keyring::default::default_credential_builder().persistence(),
                keyring::credential::CredentialPersistence::UntilDelete
            ));
        }
        #[cfg(not(windows))]
        assert_eq!(SecretBackend::default(), SecretBackend::File);
    }

    #[test]
    fn migration_removes_legacy_only_after_verified_persistence() {
        let fixture = Fixture::new();
        fixture.legacy(&[("account", "legacy-secret")]);
        let store = fixture.store(SecretBackend::Keyring);

        assert!(store.migration_error().is_none());
        assert!(!fixture.path.exists());
        assert_eq!(store.get("account").as_deref(), Some("legacy-secret"));
        assert_eq!(
            fixture
                .store(SecretBackend::Keyring)
                .get("account")
                .as_deref(),
            Some("legacy-secret")
        );
    }

    #[test]
    fn migration_preserves_native_value_over_stale_plaintext() {
        let fixture = Fixture::new();
        fixture.legacy(&[("account", "stale-secret")]);
        fixture.seed_native("account", "current-secret");
        let store = fixture.store(SecretBackend::Keyring);

        assert!(store.migration_error().is_none());
        assert_eq!(store.get("account").as_deref(), Some("current-secret"));
        assert!(fixture.credentials.0.lock().unwrap().writes.is_empty());
        assert!(!fixture.path.exists());
    }

    #[test]
    fn partial_migration_keeps_original_file_and_retries_only_missing_keys() {
        let fixture = Fixture::new();
        let original = fixture.legacy(&[("ready", "first"), ("pending", "second")]);
        fixture
            .credentials
            .0
            .lock()
            .unwrap()
            .fail_writes
            .insert("pending".into());
        let store = fixture.store(SecretBackend::Keyring);

        assert!(store.migration_error().is_some());
        assert_eq!(std::fs::read(&fixture.path).unwrap(), original);
        assert_eq!(store.get("ready").as_deref(), Some("first"));
        assert_eq!(store.get("pending").as_deref(), Some("second"));
        assert_eq!(fixture.credentials.get("pending").unwrap(), None);

        fixture.credentials.0.lock().unwrap().fail_writes.clear();
        let retried = fixture.store(SecretBackend::Keyring);
        assert!(retried.migration_error().is_none());
        assert!(!fixture.path.exists());
        assert_eq!(retried.get("pending").as_deref(), Some("second"));
        let state = fixture.credentials.0.lock().unwrap();
        assert_eq!(
            state
                .writes
                .iter()
                .filter(|account| *account == "ready")
                .count(),
            1
        );
    }

    #[test]
    fn migration_does_not_trust_success_without_readback() {
        let fixture = Fixture::new();
        let original = fixture.legacy(&[("account", "only-copy")]);
        fixture.credentials.0.lock().unwrap().discard_writes = true;
        let mut store = fixture.store(SecretBackend::Keyring);

        assert!(store.migration_error().is_some());
        assert_eq!(std::fs::read(&fixture.path).unwrap(), original);
        assert_eq!(store.get("account").as_deref(), Some("only-copy"));
        assert!(store.set("replacement", "account").is_err());
        assert_eq!(std::fs::read(&fixture.path).unwrap(), original);
    }

    #[test]
    fn migration_read_error_preserves_existing_native_and_legacy_values() {
        let fixture = Fixture::new();
        let original = fixture.legacy(&[("account", "stale-secret")]);
        fixture.seed_native("account", "current-secret");
        fixture.credentials.0.lock().unwrap().fail_reads = true;
        let store = fixture.store(SecretBackend::Keyring);

        assert!(store.migration_error().is_some());
        assert_eq!(std::fs::read(&fixture.path).unwrap(), original);
        assert!(fixture.credentials.0.lock().unwrap().writes.is_empty());
        fixture.credentials.0.lock().unwrap().fail_reads = false;
        assert_eq!(store.get("account").as_deref(), Some("current-secret"));
    }

    #[test]
    fn failed_native_update_is_reported_without_creating_plaintext_fallback() {
        let fixture = Fixture::new();
        fixture.seed_native("account", "old-secret");
        fixture
            .credentials
            .0
            .lock()
            .unwrap()
            .fail_writes
            .insert("account".into());
        let mut store = fixture.store(SecretBackend::Keyring);

        assert!(store.set("replacement", "account").is_err());
        assert_eq!(store.get("account").as_deref(), Some("old-secret"));
        assert!(!fixture.path.exists());

        let original = fixture.legacy(&[("account", "legacy-secret")]);
        assert!(store.set("replacement", "account").is_err());
        assert_eq!(std::fs::read(&fixture.path).unwrap(), original);
    }

    #[test]
    fn deletion_reports_native_errors_and_verifies_absence() {
        let fixture = Fixture::new();
        fixture.seed_native("account", "old-secret");
        let mut store = fixture.store(SecretBackend::Keyring);
        fixture.credentials.0.lock().unwrap().fail_deletes = true;

        assert!(store.delete("account").is_err());
        assert_eq!(store.get("account").as_deref(), Some("old-secret"));
        {
            let mut state = fixture.credentials.0.lock().unwrap();
            state.fail_deletes = false;
            state.discard_deletes = true;
        }
        assert!(store.delete("account").is_err());
        fixture.credentials.0.lock().unwrap().discard_deletes = false;
        store.set("", "account").unwrap();
        assert_eq!(store.get("account"), None);
        store.delete("account").unwrap();
    }

    #[test]
    fn native_updates_and_deletes_preserve_other_legacy_accounts() {
        let fixture = Fixture::new();
        let mut store = fixture.store(SecretBackend::Keyring);
        fixture.legacy(&[("account", "old-secret"), ("other", "only-copy")]);

        store.set("new-secret", "account").unwrap();
        assert_eq!(store.get("account").as_deref(), Some("new-secret"));
        assert_eq!(
            store.load_file().unwrap(),
            HashMap::from([("other".to_owned(), "only-copy".to_owned())])
        );
        store.delete("other").unwrap();
        assert!(!fixture.path.exists());
        assert_eq!(store.get("other"), None);
        assert_eq!(store.get("account").as_deref(), Some("new-secret"));
    }

    #[test]
    fn file_backend_persists_changes_and_treats_empty_value_as_deletion() {
        let fixture = Fixture::new();
        let mut store = fixture.store(SecretBackend::File);
        store.set("first", "account").unwrap();
        store.set("second", "other").unwrap();
        assert_eq!(
            fixture.store(SecretBackend::File).get("account").as_deref(),
            Some("first")
        );
        store.set("", "account").unwrap();
        assert_eq!(store.get("account"), None);
        assert_eq!(store.get("other").as_deref(), Some("second"));
        store.delete("other").unwrap();
        store.delete("absent").unwrap();
        assert!(!fixture.path.exists());
    }

    #[test]
    fn malformed_or_unwritable_file_storage_is_not_reported_as_saved() {
        let fixture = Fixture::new();
        std::fs::write(&fixture.path, "not valid JSON").unwrap();
        let mut store = fixture.store(SecretBackend::File);
        assert!(store.set("new-secret", "account").is_err());
        assert!(store.delete("account").is_err());
        assert_eq!(
            std::fs::read_to_string(&fixture.path).unwrap(),
            "not valid JSON"
        );

        std::fs::remove_file(&fixture.path).unwrap();
        std::fs::create_dir(&fixture.path).unwrap();
        assert!(store.set("new-secret", "account").is_err());
        assert_eq!(store.get("account"), None);
    }

    #[cfg(windows)]
    fn lock_legacy_file(path: &std::path::Path) -> std::fs::File {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_SHARE_READ: allow inspection but reject writes and replacement.
        std::fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(path)
            .unwrap()
    }

    #[cfg(windows)]
    #[test]
    fn failed_cleanup_is_reported_and_cannot_roll_back_a_native_update() {
        let fixture = Fixture::new();
        let mut store = fixture.store(SecretBackend::Keyring);
        let original = fixture.legacy(&[("account", "stale-secret")]);
        let lock = lock_legacy_file(&fixture.path);

        assert!(store.set("new-secret", "account").is_err());
        assert_eq!(store.get("account").as_deref(), Some("new-secret"));
        assert_eq!(std::fs::read(&fixture.path).unwrap(), original);
        let restarted = fixture.store(SecretBackend::Keyring);
        assert!(restarted.migration_error().is_some());
        assert_eq!(restarted.get("account").as_deref(), Some("new-secret"));
        drop(lock);

        let retried = fixture.store(SecretBackend::Keyring);
        assert!(retried.migration_error().is_none());
        assert_eq!(retried.get("account").as_deref(), Some("new-secret"));
        assert!(!fixture.path.exists());
    }

    #[cfg(windows)]
    #[test]
    fn deletion_stops_before_native_delete_if_legacy_cleanup_fails() {
        let fixture = Fixture::new();
        fixture.seed_native("account", "current-secret");
        let mut store = fixture.store(SecretBackend::Keyring);
        let original = fixture.legacy(&[("account", "old-secret"), ("other", "only-copy")]);
        let lock = lock_legacy_file(&fixture.path);

        assert!(store.delete("account").is_err());
        assert_eq!(store.get("account").as_deref(), Some("current-secret"));
        assert_eq!(std::fs::read(&fixture.path).unwrap(), original);
        drop(lock);
        store.delete("account").unwrap();
        assert_eq!(store.get("account"), None);
        assert_eq!(store.get("other").as_deref(), Some("only-copy"));
    }

    #[cfg(windows)]
    #[test]
    fn failed_atomic_file_replacement_keeps_old_values_without_caching_edit() {
        let fixture = Fixture::new();
        let original = fixture.legacy(&[("account", "old-secret"), ("other", "only-copy")]);
        let mut store = fixture.store(SecretBackend::File);
        let lock = lock_legacy_file(&fixture.path);

        assert!(store.set("new-secret", "account").is_err());
        assert_eq!(std::fs::read(&fixture.path).unwrap(), original);
        assert_eq!(store.get("account").as_deref(), Some("old-secret"));
        drop(lock);
        store.set("new-secret", "account").unwrap();
        assert_eq!(store.get("account").as_deref(), Some("new-secret"));
        assert_eq!(store.get("other").as_deref(), Some("only-copy"));
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "uses Windows Credential Manager with a unique disposable account"]
    fn windows_native_credential_round_trip() {
        struct Cleanup(String);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = NativeCredentials.delete(&self.0);
            }
        }

        let fixture = Fixture::new();
        let cleanup = Cleanup(format!("opencode-secret-test-{}", Uuid::new_v4()));
        let mut store = SecretStore::with_storage(
            SecretBackend::Keyring,
            fixture.path.clone(),
            Box::new(NativeCredentials),
        );
        store
            .set("synthetic-integration-value", &cleanup.0)
            .unwrap();
        let reopened = SecretStore::with_storage(
            SecretBackend::Keyring,
            fixture.path.clone(),
            Box::new(NativeCredentials),
        );
        let value = reopened.get(&cleanup.0);
        store.delete(&cleanup.0).unwrap();
        assert_eq!(value.as_deref(), Some("synthetic-integration-value"));
        assert_eq!(NativeCredentials.get(&cleanup.0).unwrap(), None);
        assert!(!fixture.path.exists());
    }
}

// MARK: - ShortcutStore

/// A user-defined prompt shortcut, invoked as `/name input`. `{input}` in
/// the template marks where the typed remainder goes; without a placeholder
/// the input is appended on a new line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptShortcut {
    pub id: Uuid,
    pub name: String,
    pub template: String,
}

impl PromptShortcut {
    pub fn expand(&self, input: &str) -> String {
        if self.template.contains("{input}") {
            self.template.replace("{input}", input)
        } else if input.is_empty() {
            self.template.clone()
        } else {
            format!("{}\n\n{input}", self.template)
        }
    }
}

pub struct ShortcutStore {
    pub shortcuts: Vec<PromptShortcut>,
}

impl ShortcutStore {
    fn file_path() -> PathBuf {
        app_data_dir().join("shortcuts.json")
    }

    pub fn load() -> Self {
        let stored: Option<Vec<PromptShortcut>> = std::fs::read(Self::file_path())
            .ok()
            .and_then(|data| serde_json::from_slice(&data).ok());
        match stored {
            Some(shortcuts) => ShortcutStore { shortcuts },
            None => {
                let store = ShortcutStore {
                    shortcuts: vec![
                        PromptShortcut {
                            id: Uuid::new_v4(),
                            name: "translate".to_string(),
                            template: "Translate the following into natural, idiomatic English. Reply with the translation only:\n\n{input}".to_string(),
                        },
                        PromptShortcut {
                            id: Uuid::new_v4(),
                            name: "explain".to_string(),
                            template: "Explain the following clearly and concisely, assuming a technical reader:\n\n{input}".to_string(),
                        },
                    ],
                };
                store.persist();
                store
            }
        }
    }

    /// Case-insensitive name match, like the Swift `match(name:)`.
    pub fn find(&self, name: &str) -> Option<&PromptShortcut> {
        self.shortcuts
            .iter()
            .find(|s| s.name.to_lowercase() == name.to_lowercase())
    }

    pub fn add(&mut self) {
        self.shortcuts.push(PromptShortcut {
            id: Uuid::new_v4(),
            name: "new-shortcut".to_string(),
            template: "…{input}".to_string(),
        });
        self.persist();
    }

    pub fn remove(&mut self, id: Uuid) {
        self.shortcuts.retain(|s| s.id != id);
        self.persist();
    }

    fn persist(&self) {
        write_json_file(&Self::file_path(), &self.shortcuts);
    }

    /// Settings edits the catalog as a draft. Persist only after Save and
    /// report I/O errors instead of silently declaring a successful update.
    pub fn save_entries(entries: &[PromptShortcut]) -> anyhow::Result<()> {
        let path = Self::file_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        crate::paths::atomic_write(&path, &serde_json::to_vec_pretty(entries)?)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::providers::ModelCapabilities;

    // ConversationStore tests share one process-global override directory,
    // so they take this lock to keep `set/clear_override_dir` serialized.
    #[cfg(test)]
    static CONVERSATION_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn caps_all() -> AttachmentCapabilities {
        AttachmentCapabilities {
            images: Some(true),
            files: Some(true),
        }
    }

    fn caps_text_only() -> AttachmentCapabilities {
        AttachmentCapabilities {
            images: None,
            files: Some(false),
        }
    }

    fn reveal_store(mode: StreamingMode, target: &str) -> ChatStore {
        let message = ChatMessage::new(ChatRole::Assistant, target);
        let id = message.id;
        ChatStore {
            messages: vec![message],
            is_streaming: true,
            pending_status: None,
            pending_reasoning: None,
            recent: vec![],
            conversation_id: Uuid::new_v4(),
            search_corpus: None,
            restore_tick: 0,
            restored_into_empty_panel: false,
            fork_parent_id: None,
            fork_message_id: None,
            shared_prefix_count: 0,
            stream_target: target.to_owned(),
            streaming_message_id: Some(id),
            capability_failure: None,
            prompts: PromptHistory::with_storage_path(
                std::env::temp_dir()
                    .join(format!("popchat-reveal-{}", Uuid::new_v4()))
                    .join("prompts.json"),
            ),
            streaming_mode: mode,
            reveal_drain: Some(RevealDrain::new(id, mode, std::time::Instant::now())),
        }
    }

    #[test]
    fn actual_reveal_clock_paces_text_and_copy_still_sees_arrived_target() {
        let target = "a".repeat(100);
        let mut store = reveal_store(StreamingMode::PerCharacter, &target);
        let id = store.messages[0].id;
        let now = std::time::Instant::now();
        assert_eq!(store.full_text(id), target);
        assert_eq!(store.display_text(id), "");
        assert_eq!(
            store.tick_reveal(now),
            Some(std::time::Duration::from_millis(33))
        );
        assert_eq!(store.display_text(id).len(), 5); // 180 chars/s × 33ms
        store.tick_reveal(now + std::time::Duration::from_millis(10));
        assert_eq!(store.display_text(id).len(), 5);
        store.tick_reveal(now + std::time::Duration::from_millis(33));
        assert_eq!(store.display_text(id).len(), 10);
        store.is_streaming = false;
        store.messages[0].text = target.clone();
        store.streaming_message_id = None;
        store.tick_reveal(now + std::time::Duration::from_millis(66));
        assert_eq!(store.display_text(id).len(), 27); // completed tail drains 3×
        store.stop();
        assert_eq!(store.display_text(id), target);
        assert!(store.reveal_drain.is_none());
    }

    #[test]
    fn actual_sentence_reveal_waits_for_boundary_and_drains_unclosed_fence_on_finish() {
        let mut store = reveal_store(
            StreamingMode::PerSentence,
            "First sentence. Second unfinished",
        );
        let id = store.messages[0].id;
        let now = std::time::Instant::now();
        store.tick_reveal(now);
        assert_eq!(store.display_text(id), "First sentence.");
        store.tick_reveal(now + std::time::Duration::from_millis(140));
        assert_eq!(store.display_text(id), "First sentence.");
        store.stream_target = "```rust\nlet x = 1;".to_owned();
        store.tick_reveal(now + std::time::Duration::from_millis(280));
        // Replaced snapshots jump without retaining the old round's prefix.
        assert_eq!(store.display_text(id), store.stream_target);
        store.reveal_drain.as_mut().unwrap().displayed.clear();
        store.is_streaming = false;
        store.messages[0].text = store.stream_target.clone();
        store.streaming_message_id = None;
        store.tick_reveal(now + std::time::Duration::from_millis(420));
        assert_eq!(store.display_text(id), store.stream_target);
        // The committed text has arrived in full, but its sentence fade must
        // survive rather than vanishing on that same frame.
        assert!(store.reveal_for(id).is_some());
        for tick in 1..=11 {
            store.tick_reveal(now + std::time::Duration::from_millis(420 + tick * 33));
        }
        assert!(store.reveal_drain.is_none());
    }

    #[test]
    fn swift_text_reveal_eased_ramp_and_groups_match_source_values() {
        assert_eq!(TextReveal::eased(-1.0), 0.0);
        assert_eq!(TextReveal::eased(0.0), 0.0);
        assert_eq!(TextReveal::eased(0.5), 0.75);
        assert_eq!(TextReveal::eased(1.0), 1.0);
        assert_eq!(TextReveal::eased(2.0), 1.0);
        assert!(TextReveal::ramp(0).is_empty());
        assert!(TextReveal::ramp_with_bands(20, 0).is_empty());
        let ramp = TextReveal::ramp(30);
        assert_eq!(
            ramp.stops.iter().map(|s| s.length).collect::<Vec<_>>(),
            vec![4, 4, 3, 4, 4, 4, 3, 4]
        );
        assert_eq!(
            ramp.stops.iter().map(|s| s.alpha).collect::<Vec<_>>(),
            vec![
                0.12109375, 0.33984375, 0.52734375, 0.68359375, 0.80859375, 0.90234375, 0.96484375,
                0.99609375,
            ]
        );
        // Swift rounds half away from zero and drops zero-width bands.
        assert_eq!(
            TextReveal::ramp(1).stops,
            vec![RevealStop {
                length: 1,
                alpha: 0.68359375
            }]
        );
        for head in 1..100 {
            assert_eq!(
                TextReveal::ramp(head)
                    .stops
                    .iter()
                    .map(|s| s.length)
                    .sum::<usize>(),
                head
            );
        }
        let groups = TextReveal::groups(&[(7, 0), (0, 50), (11, 175), (4, 350), (3, 700)], 350);
        assert_eq!(
            groups.stops,
            vec![
                RevealStop {
                    length: 7,
                    alpha: 0.0
                },
                RevealStop {
                    length: 11,
                    alpha: 0.75
                },
                RevealStop {
                    length: 4,
                    alpha: 1.0
                },
                RevealStop {
                    length: 3,
                    alpha: 1.0
                },
            ]
        );
        assert!(TextReveal::groups(&[(7, 350)], 350).is_empty());
    }

    #[test]
    fn character_fade_keeps_ticking_during_network_pause_and_after_completion() {
        use std::time::{Duration, Instant};
        let mut store = reveal_store(StreamingMode::PerCharacter, &"x".repeat(30));
        let id = store.messages[0].id;
        let now = Instant::now();
        for tick in 0..6 {
            store.tick_reveal(now + Duration::from_millis(tick * 33));
        }
        assert_eq!(store.display_text(id), store.full_text(id));
        assert_eq!(store.reveal_for(id).unwrap().fade, TextReveal::ramp(30));
        store.tick_reveal(now + Duration::from_millis(198));
        assert_eq!(store.reveal_for(id).unwrap().fade, TextReveal::ramp(25));
        for tick in 7..12 {
            store.tick_reveal(now + Duration::from_millis(tick * 33));
        }
        assert!(store.reveal_for(id).is_none());
        assert!(store.reveal_drain.is_some()); // still awaiting the network
        store.is_streaming = false;
        assert_eq!(store.tick_reveal(now + Duration::from_millis(396)), None);
        assert!(store.reveal_drain.is_none());

        let mut store = reveal_store(StreamingMode::PerCharacter, &"x".repeat(100));
        let id = store.messages[0].id;
        store.is_streaming = false;
        let now = Instant::now();
        let mut full_tick = None;
        for tick in 0..20 {
            let delay = store.tick_reveal(now + Duration::from_millis(tick * 33));
            if store.display_text(id) == store.full_text(id) && full_tick.is_none() {
                full_tick = Some(tick);
                assert!(store.reveal_for(id).is_some());
                assert!(delay.is_some());
            }
            if delay.is_none() {
                assert!(tick > full_tick.unwrap());
                break;
            }
        }
        assert!(store.reveal_drain.is_none());
    }

    #[test]
    fn sentence_commits_are_gated_but_overlapping_fades_tick_at_adaptive_rate() {
        use std::time::{Duration, Instant};
        let mut store = reveal_store(StreamingMode::PerSentence, "One. Two. Three. ");
        let id = store.messages[0].id;
        let now = Instant::now();
        assert_eq!(store.tick_reveal(now), Some(Duration::from_millis(33)));
        assert_eq!(store.display_text(id), "One.");
        assert_eq!(
            store.reveal_for(id).unwrap().fade,
            TextReveal::groups(&[(4, 33)], 350)
        );
        for tick in 1..=4 {
            store.tick_reveal(now + Duration::from_millis(tick * 33));
            assert_eq!(store.display_text(id), "One.");
        }
        store.tick_reveal(now + Duration::from_millis(165));
        assert_eq!(store.display_text(id), "One. Two.");
        assert_eq!(
            store.reveal_for(id).unwrap().fade,
            TextReveal::groups(&[(5, 33), (4, 198)], 350)
        );
        store.is_streaming = false;
        store.tick_reveal(now + Duration::from_millis(198));
        assert_eq!(store.display_text(id), "One. Two.");
        store.tick_reveal(now + Duration::from_millis(231)); // >=60ms, not >=140ms
        assert_eq!(store.display_text(id), "One. Two. Three.");
        assert_eq!(store.reveal_for(id).unwrap().fade.stops.len(), 3);

        let mut store = reveal_store(StreamingMode::PerSentence, "Complete.");
        let id = store.messages[0].id;
        store.is_streaming = false;
        let now = Instant::now();
        for tick in 0..10 {
            assert!(store
                .tick_reveal(now + Duration::from_millis(tick * 33))
                .is_some());
            assert!(store.reveal_for(id).is_some());
        }
        assert_eq!(store.tick_reveal(now + Duration::from_millis(330)), None);
        assert!(store.reveal_for(id).is_none());
        for (length, interval) in [(4_001, 66), (12_001, 100)] {
            let mut store = reveal_store(StreamingMode::PerSentence, &"x".repeat(length));
            let now = Instant::now();
            assert_eq!(
                store.tick_reveal(now),
                Some(Duration::from_millis(interval))
            );
        }
    }

    #[test]
    fn replaced_snapshots_and_new_turn_flush_drop_stale_alpha_bands() {
        use std::time::{Duration, Instant};
        for mode in [StreamingMode::PerCharacter, StreamingMode::PerSentence] {
            let mut store = reveal_store(mode, "First sentence. Next sentence. ");
            let id = store.messages[0].id;
            let now = Instant::now();
            store.tick_reveal(now);
            assert!(store.reveal_for(id).is_some());
            store.accept_arrived_text(id, "Replaced round");
            store.tick_reveal(now + Duration::from_millis(33));
            assert_eq!(store.display_text(id), "Replaced round");
            assert!(store.reveal_for(id).is_none());
            store.accept_arrived_text(id, "Replaced round. A longer new tail. ");
            for tick in 2..8 {
                store.tick_reveal(now + Duration::from_millis(tick * 33));
            }
            store.flush_reveal(); // also the very first operation of send()
            assert!(store.reveal_for(id).is_none());
            assert_eq!(store.display_text(id), store.messages[0].text);
            assert_eq!(store.display_text(id), store.stream_target);
            assert!(store.streaming_message_id.is_none());
        }
    }

    #[test]
    fn partial_arrival_is_durable_and_wire_copy_never_use_faded_prefix() {
        let _guard = CONVERSATION_TEST_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        ConversationStore::set_override_dir(temp.path().join("conversations"));
        let mut store = reveal_store(StreamingMode::PerCharacter, "");
        let id = store.messages[0].id;
        store.accept_arrived_text(id, "Partial answer 👩‍👩‍👧‍👦 not fully revealed");
        store.tick_reveal(std::time::Instant::now());
        assert_ne!(store.display_text(id), store.full_text(id));
        let arrived = store.full_text(id);
        assert_eq!(store.messages[0].text, arrived);
        let wire = history_for_wire(&store.messages, "", &caps_all());
        assert_eq!(wire[0].content, Some(WireContent::Text(arrived.clone())));
        store.persist();
        let loaded = ConversationStore::load_resolved(store.conversation_id).unwrap();
        assert_eq!(loaded.messages[0].text, arrived);
        store.stop();
        assert_eq!(store.display_text(id), arrived);
        assert!(store.reveal_for(id).is_none());
        let old_turn = ActiveTurn {
            assistant_id: id,
            provider_id: Uuid::new_v4(),
            model: "fixture".to_owned(),
            sent_images: false,
            sent_files: false,
            reasoning: "old thinking".to_owned(),
            reasoning_committed: false,
        };
        let next = ChatMessage::new(ChatRole::Assistant, "");
        let next_id = next.id;
        store.messages.push(next);
        store.streaming_message_id = Some(next_id);
        store.is_streaming = true;
        store.pending_status = Some("next turn".to_owned());
        store.accept_arrived_text(next_id, "next answer");
        store.finish_turn(&old_turn);
        assert_eq!(store.full_text(id), arrived);
        assert_eq!(store.full_text(next_id), "next answer");
        assert!(store.is_streaming);
        assert_eq!(store.pending_status.as_deref(), Some("next turn"));
        assert_eq!(store.streaming_message_id, Some(next_id));
        ConversationStore::clear_override_dir();
    }

    #[test]
    fn reveal_uses_swift_character_graphemes_not_scalar_codepoints() {
        let glyph = "👩‍👩‍👧‍👦";
        let mut store = reveal_store(StreamingMode::PerCharacter, &glyph.repeat(10));
        let id = store.messages[0].id;
        store.tick_reveal(std::time::Instant::now());
        assert_eq!(store.display_text(id), glyph.repeat(5));
        assert_eq!(
            ChatStore::sentence_commit_length("e\u{301}. next", 0, false),
            2
        );
        assert_eq!(
            serde_json::from_str::<StreamingMode>("\"byChunk\"").unwrap(),
            StreamingMode::PerSentence
        );
        assert_eq!(
            serde_json::from_str::<StreamingMode>("\"unknown future value\"").unwrap(),
            StreamingMode::PerCharacter
        );
    }

    #[test]
    fn append_can_extend_a_previously_displayed_grapheme_without_stranding_drain() {
        use std::time::{Duration, Instant};
        for (first, extended) in [("e", "e\u{301}"), ("👩", "👩‍👩‍👧‍👦")] {
            let mut store = reveal_store(StreamingMode::PerCharacter, first);
            let id = store.messages[0].id;
            let now = Instant::now();
            store.tick_reveal(now);
            assert_eq!(store.display_text(id), first);
            store.accept_arrived_text(id, extended);
            store.is_streaming = false;
            assert_eq!(store.tick_reveal(now + Duration::from_millis(33)), None);
            assert_eq!(store.display_text(id), extended);
            assert!(store.reveal_for(id).is_none());
        }
    }

    #[test]
    fn reasoning_is_persisted_but_never_sent() {
        let mut assistant = ChatMessage::new(ChatRole::Assistant, "the answer");
        assistant.reasoning = Some("the thinking".to_string());
        let user = ChatMessage::new(ChatRole::User, "hi");
        let history = history_for_wire(&[user, assistant], "", &caps_all());
        assert_eq!(history.len(), 2);
        let encoded = serde_json::to_string(&history).unwrap();
        // The answer goes on the wire…
        assert!(encoded.contains("the answer"));
        // …the thinking never does.
        assert!(!encoded.contains("the thinking"));
        assert!(!encoded.contains("reasoning"));
    }

    #[test]
    fn error_and_activity_rows_never_reach_the_wire() {
        let messages = vec![
            ChatMessage::new(ChatRole::User, "q"),
            ChatMessage::new(ChatRole::Activity, "Searching: x"),
            ChatMessage::new(ChatRole::Error, "boom"),
            ChatMessage::new(ChatRole::Assistant, "a"),
        ];
        let history = history_for_wire(&messages, "sys", &caps_all());
        // system + user + assistant only.
        assert_eq!(history.len(), 3);
        assert_eq!(history[0].role, "system");
        assert_eq!(history[1].role, "user");
        assert_eq!(history[2].role, "assistant");
    }

    #[test]
    fn wire_content_sends_images_always_and_pdf_by_capability() {
        let mut message = ChatMessage::new(ChatRole::User, "look");
        message.attachments.push(Attachment::new(
            "a.jpg",
            AttachmentContent::Image {
                data_url: "data:image/jpeg;base64,AAA".to_string(),
            },
        ));
        message.attachments.push(Attachment::new(
            "d.pdf",
            AttachmentContent::Pdf {
                data_url: "data:application/pdf;base64,BBB".to_string(),
                extracted_text: "pdf text".to_string(),
            },
        ));

        // Native PDFs: file part + image part + combined text.
        let native = wire_content_for_message(&message, &caps_all());
        let parts = match native {
            WireContent::Parts(p) => p,
            WireContent::Text(_) => panic!("expected parts"),
        };
        assert!(parts.iter().any(|p| p.part_type == "image_url"));
        assert!(parts.iter().any(|p| p.part_type == "file"));
        assert!(parts.iter().any(|p| p.part_type == "text"));

        // Text-only path: the PDF degrades to extracted text, images still sent.
        let degraded = wire_content_for_message(&message, &caps_text_only());
        let parts = match degraded {
            WireContent::Parts(p) => p,
            WireContent::Text(_) => panic!("expected parts (image still typed)"),
        };
        assert!(parts.iter().any(|p| p.part_type == "image_url"));
        assert!(!parts.iter().any(|p| p.part_type == "file"));
        let text = parts
            .iter()
            .find(|p| p.part_type == "text")
            .and_then(|p| p.text.clone())
            .unwrap();
        assert!(text.contains("pdf text"));
    }

    #[test]
    fn wire_content_stays_bare_string_without_typed_parts() {
        let mut message = ChatMessage::new(ChatRole::User, "hello");
        message.attachments.push(Attachment::new(
            "n.txt",
            AttachmentContent::Text("file body".to_string()),
        ));
        assert_eq!(
            wire_content_for_message(&message, &caps_text_only()),
            WireContent::Text("[File: n.txt]\nfile body\n\nhello".to_string())
        );
    }

    #[test]
    fn scanned_pdf_on_text_path_states_its_absence() {
        let mut message = ChatMessage::new(ChatRole::User, "");
        message.attachments.push(Attachment::new(
            "scan.pdf",
            AttachmentContent::Pdf {
                data_url: "data:application/pdf;base64,BBB".to_string(),
                extracted_text: String::new(),
            },
        ));
        let WireContent::Text(text) = wire_content_for_message(&message, &caps_text_only()) else {
            panic!("expected bare text");
        };
        assert!(text.contains("no extractable text"));
    }

    #[test]
    fn full_text_index_excludes_reasoning() {
        let _guard = CONVERSATION_TEST_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("popchat-test-{}", Uuid::new_v4()));
        ConversationStore::set_override_dir(dir);
        let id = Uuid::new_v4();
        let mut assistant = ChatMessage::new(ChatRole::Assistant, "visible answer");
        assistant.reasoning = Some("seekrit thinking xyzzy".to_string());
        ConversationStore::save(&Conversation {
            id,
            title: "t".to_string(),
            updated_at: Utc::now(),
            messages: vec![ChatMessage::new(ChatRole::User, "hello"), assistant],
            parent_id: None,
            fork_message_id: None,
        });
        let index = ConversationStore::full_text_index();
        let text = &index[&id];
        assert!(text.contains("visible answer"));
        assert!(!text.contains("seekrit"));
        ConversationStore::clear_override_dir();
    }

    #[test]
    fn fork_chain_resolves_and_missing_parent_flags() {
        let _guard = CONVERSATION_TEST_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("popchat-test-{}", Uuid::new_v4()));
        ConversationStore::set_override_dir(dir);
        let m1 = ChatMessage::new(ChatRole::User, "one");
        let m2 = ChatMessage::new(ChatRole::Assistant, "two");
        let parent_id = Uuid::new_v4();
        ConversationStore::save(&Conversation {
            id: parent_id,
            title: "p".to_string(),
            updated_at: Utc::now(),
            messages: vec![m1.clone(), m2.clone()],
            parent_id: None,
            fork_message_id: None,
        });
        let child_id = Uuid::new_v4();
        let m3 = ChatMessage::new(ChatRole::User, "three");
        ConversationStore::save(&Conversation {
            id: child_id,
            title: "c".to_string(),
            updated_at: Utc::now(),
            messages: vec![m3.clone()],
            parent_id: Some(parent_id),
            fork_message_id: Some(m2.id),
        });
        let resolved = ConversationStore::load_resolved(child_id).unwrap();
        assert!(!resolved.missing_parent);
        assert_eq!(
            resolved
                .messages
                .iter()
                .map(|m| m.text.clone())
                .collect::<Vec<_>>(),
            vec!["one", "two", "three"]
        );
        // Delete the parent without materializing: child flags the break.
        ConversationStore::delete(parent_id);
        let broken = ConversationStore::load_resolved(child_id).unwrap();
        assert!(broken.missing_parent);
        assert_eq!(broken.messages.len(), 1);
        ConversationStore::clear_override_dir();
    }

    #[test]
    fn prompt_history_dedups_and_recalls_up_down() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("prompt_history.json");
        let mut history = PromptHistory {
            entries: Vec::new(),
            cursor: None,
            saved_draft: String::new(),
            storage_path: Some(path.clone()),
        };
        history.record("first");
        history.record("second");
        history.record("first"); // dedup: back to front, no double cost
        assert_eq!(
            history.entries(),
            &["first".to_string(), "second".to_string()]
        );
        history.record("  "); // blank ignored
        assert_eq!(history.entries().len(), 2);
        let persisted: Vec<String> = read_json_file(&path);
        assert_eq!(persisted, history.entries());

        assert_eq!(history.recall_older("draft").as_deref(), Some("first"));
        assert_eq!(history.recall_older("draft").as_deref(), Some("second"));
        // Clamped at the oldest.
        assert_eq!(history.recall_older("draft").as_deref(), Some("second"));
        assert_eq!(history.recall_newer().as_deref(), Some("first"));
        // Past the newest restores the live draft.
        assert_eq!(history.recall_newer().as_deref(), Some("draft"));
        assert_eq!(history.recall_newer(), None);
    }

    #[test]
    fn excerpt_finds_case_insensitive_hit_with_ellipses() {
        let text = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. TARGET phrase here. Sed do eiusmod tempor incididunt ut labore et dolore magna aliqua enim ad minim veniam quis nostrud.";
        let excerpt = ConversationStore::excerpt(text, "target phrase", 44).unwrap();
        assert!(excerpt.contains("TARGET phrase"));
        assert!(excerpt.starts_with('…'));
        assert!(excerpt.ends_with('…'));
        assert!(!excerpt.contains('\n'));
        assert!(ConversationStore::excerpt(text, "absent", 44).is_none());
        assert!(ConversationStore::excerpt(text, "", 44).is_none());
    }

    #[test]
    fn metadata_capabilities_flow_into_wire_choice() {
        // A model known (via OpenRouter-style metadata) to refuse files gets
        // extracted text; images still go as parts.
        let metadata = ModelCapabilities {
            images: Some(true),
            files: Some(false),
        };
        let caps = crate::chat::providers::resolve_capabilities(
            None,
            Some(&metadata),
            &AttachmentCapabilities::default(),
        );
        let mut message = ChatMessage::new(ChatRole::User, "read this");
        message.attachments.push(Attachment::new(
            "d.pdf",
            AttachmentContent::Pdf {
                data_url: "data:application/pdf;base64,BBB".to_string(),
                extracted_text: "extracted".to_string(),
            },
        ));
        let WireContent::Text(text) = wire_content_for_message(&message, &caps) else {
            panic!("no typed parts remain: must be a bare string");
        };
        assert!(text.contains("extracted"));
    }
}
