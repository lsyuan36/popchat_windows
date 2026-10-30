//! Conversation + app-config persistence, ported from
//! `Chat/Conversations.swift`, `Chat/PromptHistory.swift`, the `UserDefaults`
//! keys in `PanelController`/`SettingsView`/`ProviderStore`, and the README
//! storage table.
//!
//! Layout (Windows):
//! - Conversations: `%APPDATA%/PopChat/conversations/<uuid>.json`
//!   (one JSON file per conversation; forks store a parent pointer plus only
//!   the divergent tail, resolved on load — same tree design as macOS).
//! - Everything `UserDefaults` held: `%APPDATA%/PopChat/config.json`
//!   (`panelWidth`, `panelExpandedHeight`, `accentColor`, `promptHistory`,
//!   provider list, …).
//!
//! Cross-version tolerance: unknown JSON fields are ignored (no
//! `deny_unknown_fields` anywhere), a missing `kind` backfills to
//! [`ProviderKind::OpenAICompatible`], unknown `kind` strings map there too
//! rather than failing the whole provider list (parity with Swift's custom
//! `init(from:)`), and timestamps accept both RFC 3339 and Apple's ISO 8601
//! (`+0800`-style offsets included).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

use crate::chat::attach::Attachment;

/// Cap on stored conversations; oldest are pruned (materializing children).
pub const MAX_STORED: usize = 50;
/// Prompt-history cap (mirrors `PromptHistory.maxEntries`).
pub const MAX_PROMPT_HISTORY: usize = 50;

// ---------------------------------------------------------------------------
// Directories
// ---------------------------------------------------------------------------

static OVERRIDE_DIR: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();

fn override_dir() -> std::sync::MutexGuard<'static, Option<PathBuf>> {
    OVERRIDE_DIR
        .get_or_init(|| Mutex::new(None))
        .lock()
        .expect("override dir lock")
}

/// Test hook: point the store at a scratch directory so tests never touch the
/// user's real history (mirrors `ConversationStore.overrideDirectory`).
pub fn set_override_dir(dir: PathBuf) {
    *override_dir() = Some(dir);
}

/// Clear the test hook set by [`set_override_dir`].
pub fn clear_override_dir() {
    *override_dir() = None;
}

/// `%APPDATA%/PopChat` (via the `directories` crate, with env fallback).
pub fn app_dir() -> PathBuf {
    if let Some(dir) = override_dir().clone() {
        return dir;
    }
    if let Some(profile) = crate::paths::isolated_profile() {
        return profile;
    }
    if let Some(base) = directories::BaseDirs::new() {
        return base.data_dir().join("PopChat");
    }
    if let Ok(appdata) = std::env::var("APPDATA") {
        return PathBuf::from(appdata).join("PopChat");
    }
    PathBuf::from("PopChat")
}

/// `%APPDATA%/PopChat/conversations`.
pub fn conversations_dir() -> PathBuf {
    let dir = app_dir().join("conversations");
    std::fs::create_dir_all(&dir).ok();
    dir
}

/// `%APPDATA%/PopChat/config.json`.
pub fn config_path() -> PathBuf {
    let dir = app_dir();
    std::fs::create_dir_all(&dir).ok();
    dir.join("config.json")
}

fn conversation_path(id: &Uuid) -> PathBuf {
    conversations_dir().join(format!("{id}.json"))
}

// ---------------------------------------------------------------------------
// Tolerant (de)serialization helpers
// ---------------------------------------------------------------------------

fn default_role() -> ChatRole {
    ChatRole::User
}

fn de_chat_role<'de, D: Deserializer<'de>>(d: D) -> Result<ChatRole, D::Error> {
    let s = Option::<String>::deserialize(d)?.unwrap_or_default();
    Ok(match s.as_str() {
        "assistant" => ChatRole::Assistant,
        "error" => ChatRole::Error,
        "activity" => ChatRole::Activity,
        _ => ChatRole::User,
    })
}

fn de_provider_kind<'de, D: Deserializer<'de>>(d: D) -> Result<ProviderKind, D::Error> {
    // Unknown future kinds map to `OpenAICompatible` instead of throwing and
    // resetting every provider (parity with Swift's raw-string decode).
    let s = Option::<String>::deserialize(d)?.unwrap_or_default();
    Ok(match s.as_str() {
        "chatGPT" => ProviderKind::ChatGPT,
        "codexAppServer" => ProviderKind::CodexAppServer,
        _ => ProviderKind::OpenAICompatible,
    })
}

/// Accept RFC 3339 plus Apple's ISO 8601 output (`Z`, `±hh:mm`, `±hhmm`,
/// with/without fractional seconds, or naive local time assumed UTC).
fn de_datetime<'de, D: Deserializer<'de>>(d: D) -> Result<DateTime<Utc>, D::Error> {
    let s = String::deserialize(d)?;
    parse_datetime_flexible(&s)
        .ok_or_else(|| serde::de::Error::custom(format!("invalid datetime: {s}")))
}

fn se_datetime<S: Serializer>(dt: &DateTime<Utc>, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&dt.to_rfc3339_opts(SecondsFormat::Millis, true))
}

fn parse_datetime_flexible(s: &str) -> Option<DateTime<Utc>> {
    if let Ok(dt) = s.parse::<DateTime<Utc>>() {
        return Some(dt);
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    for fmt in [
        "%Y-%m-%dT%H:%M:%S%.f%z",
        "%Y-%m-%dT%H:%M:%S%z",
        "%Y-%m-%dT%H:%M:%S%.fZ",
        "%Y-%m-%dT%H:%M:%SZ",
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%d %H:%M:%S",
    ] {
        if let Ok(dt) = DateTime::parse_from_str(s, fmt) {
            return Some(dt.with_timezone(&Utc));
        }
        if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(s, fmt) {
            return Some(naive.and_utc());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Chat model (wire-compatible with the macOS JSON keys)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    #[default]
    User,
    Assistant,
    /// UI-only row for explicit errors/warnings — never sent to the API.
    Error,
    /// UI-only row documenting tool use — never sent to the API.
    Activity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    #[serde(default = "Uuid::new_v4")]
    pub id: Uuid,
    #[serde(default = "default_role", deserialize_with = "de_chat_role")]
    pub role: ChatRole,
    #[serde(default)]
    pub text: String,
    /// What actually goes over the wire when it differs from the display text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wire_text: Option<String>,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    /// Persisted with the message but never sent back. Optional so
    /// conversations written before it existed still decode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
}

impl ChatMessage {
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            role: ChatRole::User,
            text: text.into(),
            wire_text: None,
            attachments: Vec::new(),
            reasoning: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Conversation {
    pub id: Uuid,
    #[serde(default)]
    pub title: String,
    #[serde(
        deserialize_with = "de_datetime",
        serialize_with = "se_datetime",
        default = "Utc::now"
    )]
    pub updated_at: DateTime<Utc>,
    /// For a fork: only the messages AFTER the fork point — the shared prefix
    /// lives in the parent chain and is resolved on load.
    #[serde(default)]
    pub messages: Vec<ChatMessage>,
    /// Conversation this one was forked from; nil for a root conversation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<Uuid>,
    /// Last shared message (inclusive) in the parent's resolved transcript.
    /// (macOS key is `forkMessageID`.)
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "forkMessageID"
    )]
    pub fork_message_id: Option<Uuid>,
}

// The macOS keys are `parentID` (capital D), not serde-camelCase `parentId`.
impl Conversation {
    fn fix_parent_key(value: &mut serde_json::Value) {
        if let Some(obj) = value.as_object_mut() {
            if let Some(v) = obj.remove("parentID") {
                obj.insert("parentId".to_string(), v);
            }
        }
    }

    fn restore_parent_key(value: &mut serde_json::Value) {
        if let Some(obj) = value.as_object_mut() {
            if let Some(v) = obj.remove("parentId") {
                obj.insert("parentID".to_string(), v);
            }
        }
    }
}

fn conversation_from_value(mut value: serde_json::Value) -> Option<Conversation> {
    Conversation::fix_parent_key(&mut value);
    serde_json::from_value(value).ok()
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
        let last = messages
            .iter()
            .rev()
            .find(|m| matches!(m.role, ChatRole::User | ChatRole::Assistant) && !m.text.is_empty());
        last.map(|m| m.text.replace('\n', " ").chars().take(120).collect())
            .unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// Provider model (for config.json; tolerant `kind`)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ProviderKind {
    #[serde(rename = "openAICompatible")]
    #[default]
    OpenAICompatible,
    #[serde(rename = "chatGPT")]
    ChatGPT,
    #[serde(rename = "codexAppServer")]
    CodexAppServer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Provider {
    pub id: Uuid,
    #[serde(default)]
    pub name: String,
    #[serde(default, rename = "baseURL")]
    pub base_url: String,
    #[serde(default, rename = "isPreset")]
    pub is_preset: bool,
    #[serde(default, rename = "defaultModel")]
    pub default_model: String,
    /// Missing → [`ProviderKind::OpenAICompatible`]; unknown strings map
    /// there too (never fail the whole list).
    #[serde(default, deserialize_with = "de_provider_kind")]
    pub kind: ProviderKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub free_template: Option<String>,
}

// ---------------------------------------------------------------------------
// App config (the old `UserDefaults` surface, now `config.json`)
// ---------------------------------------------------------------------------

/// Everything the mac build kept in `UserDefaults`, keyed with the same
/// camelCase names (`panelWidth`, `panelExpandedHeight`, `accentColor`,
/// `promptHistory`, `providers`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct AppConfig {
    pub panel_width: Option<f64>,
    pub panel_expanded_height: Option<f64>,
    pub accent_color: Option<String>,
    pub prompt_history: Vec<String>,
    pub providers: Vec<Provider>,
}

impl AppConfig {
    pub fn load() -> Self {
        let path = config_path();
        let data = std::fs::read(&path).unwrap_or_default();
        if data.is_empty() {
            return Self::default();
        }
        serde_json::from_slice(&data).unwrap_or_default()
    }

    pub fn save(&self) {
        let path = config_path();
        if let Ok(data) = serde_json::to_string_pretty(self) {
            // Best-effort atomic write: temp file + rename.
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, data).is_ok() {
                std::fs::rename(&tmp, &path).ok();
            }
        }
    }

    /// Record a sent prompt at the front, deduplicated and capped
    /// (mirrors `PromptHistory.record`).
    pub fn record_prompt(&mut self, text: &str) {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }
        self.prompt_history.retain(|p| p != trimmed);
        self.prompt_history.insert(0, trimmed.to_string());
        self.prompt_history.truncate(MAX_PROMPT_HISTORY);
    }
}

// ---------------------------------------------------------------------------
// Conversation store
// ---------------------------------------------------------------------------

pub fn save_conversation(conversation: &Conversation) {
    // Serialize with the macOS `parentID` key so files stay interchangeable.
    let mut value = serde_json::to_value(conversation).unwrap_or(serde_json::Value::Null);
    Conversation::restore_parent_key(&mut value);
    let data = serde_json::to_string(&value).unwrap_or_default();
    if data.is_empty() || data == "null" {
        return;
    }
    let path = conversation_path(&conversation.id);
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, data).is_ok() {
        std::fs::rename(&tmp, &path).ok();
    }
}

pub fn load_conversation(id: &Uuid) -> Option<Conversation> {
    let data = std::fs::read(conversation_path(id)).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&data).ok()?;
    conversation_from_value(value)
}

pub fn delete_conversation(id: &Uuid) {
    std::fs::remove_file(conversation_path(id)).ok();
}

fn load_all() -> HashMap<Uuid, Conversation> {
    let mut result = HashMap::new();
    let dir = conversations_dir();
    let entries = std::fs::read_dir(&dir).into_iter().flatten().flatten();
    for entry in entries {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) == Some("tmp") {
            continue;
        }
        let Ok(data) = std::fs::read(&path) else {
            continue;
        };
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(&data) else {
            continue;
        };
        if let Some(conversation) = conversation_from_value(value) {
            result.insert(conversation.id, conversation);
        }
    }
    result
}

/// Full transcript: the parent chain's shared prefix plus this conversation's
/// own tail. `missing_parent` is true when the chain is broken.
pub fn resolve_messages(conversation: &Conversation) -> (Vec<ChatMessage>, bool) {
    resolve_with(conversation, &|id| load_conversation(id), &{
        let mut visited = HashSet::new();
        visited.insert(conversation.id);
        visited
    })
}

fn resolve_with(
    conversation: &Conversation,
    lookup: &dyn Fn(&Uuid) -> Option<Conversation>,
    visited: &HashSet<Uuid>,
) -> (Vec<ChatMessage>, bool) {
    let (Some(parent_id), Some(fork_id)) = (
        conversation.parent_id.as_ref(),
        conversation.fork_message_id.as_ref(),
    ) else {
        return (conversation.messages.clone(), false);
    };
    if visited.contains(parent_id) {
        return (conversation.messages.clone(), true);
    }
    let Some(parent) = lookup(parent_id) else {
        return (conversation.messages.clone(), true);
    };
    let mut next_visited = visited.clone();
    next_visited.insert(*parent_id);
    let (parent_messages, parent_broken) = resolve_with(&parent, lookup, &next_visited);
    let Some(cut) = parent_messages.iter().position(|m| &m.id == fork_id) else {
        return (conversation.messages.clone(), true);
    };
    let mut full = parent_messages[..=cut].to_vec();
    full.extend(conversation.messages.iter().cloned());
    (full, parent_broken)
}

pub fn load_resolved(id: &Uuid) -> Option<(Conversation, Vec<ChatMessage>, bool)> {
    let conversation = load_conversation(id)?;
    let (messages, missing_parent) = resolve_messages(&conversation);
    Some((conversation, messages, missing_parent))
}

/// Direct children of `id` absorb the shared prefix and become standalone
/// roots. Call before a transcript stops being able to serve them.
pub fn materialize_children(parent_id: &Uuid) {
    let all = load_all();
    for child in all
        .values()
        .filter(|c| c.parent_id.as_ref() == Some(parent_id))
    {
        materialize(child, &all);
    }
}

/// Delete with fork safety: direct children absorb the shared prefix first.
pub fn delete_materializing_children(id: &Uuid) {
    materialize_children(id);
    delete_conversation(id);
}

fn materialize(conversation: &Conversation, all: &HashMap<Uuid, Conversation>) {
    let mut standalone = conversation.clone();
    let lookup = |id: &Uuid| all.get(id).cloned();
    let (messages, _) = resolve_with(conversation, &lookup, &HashSet::from([conversation.id]));
    standalone.messages = messages;
    standalone.parent_id = None;
    standalone.fork_message_id = None;
    save_conversation(&standalone);
}

fn load_all_resolved() -> (HashMap<Uuid, Conversation>, HashMap<Uuid, Vec<ChatMessage>>) {
    let all = load_all();
    let mut resolved = HashMap::new();
    for conversation in all.values() {
        if resolved.contains_key(&conversation.id) {
            continue;
        }
        let lookup = |id: &Uuid| all.get(id).cloned();
        let (messages, _) = resolve_with(conversation, &lookup, &HashSet::from([conversation.id]));
        resolved.insert(conversation.id, messages);
    }
    (all, resolved)
}

/// Every stored conversation's resolved transcript flattened to one
/// searchable string each. Attachment base64 is skipped entirely — only the
/// title, message text, and reasoning are indexed. (Reasoning is included so
/// ⌘F-style content search finds it; the UI decides what to paint.)
pub fn full_text_index() -> HashMap<Uuid, String> {
    let (all, resolved) = load_all_resolved();
    resolved
        .into_iter()
        .map(|(id, messages)| {
            let title = all.get(&id).map(|c| c.title.as_str()).unwrap_or("");
            let mut parts = vec![title.to_string()];
            for m in messages
                .iter()
                .filter(|m| matches!(m.role, ChatRole::User | ChatRole::Assistant))
            {
                if !m.text.is_empty() {
                    parts.push(m.text.clone());
                }
                if let Some(reasoning) = m.reasoning.as_ref() {
                    if !reasoning.is_empty() {
                        parts.push(reasoning.clone());
                    }
                }
            }
            (id, parts.join("\n"))
        })
        .collect()
}

/// Text around the first case-insensitive hit, for a one-line search row.
pub fn excerpt_of(text: &str, query: &str, context: usize) -> Option<String> {
    if query.is_empty() {
        return None;
    }
    let lower = text.to_lowercase();
    let q = query.to_lowercase();
    let byte_pos = lower.find(q.as_str())?;
    let chars: Vec<char> = text.chars().collect();
    // Char offset of the hit in the lowercased text; the original has the same
    // char count for the case mappings we care about here.
    let prefix_len = lower[..byte_pos].chars().count().min(chars.len());
    let s = prefix_len.saturating_sub(context);
    let e = (prefix_len + q.chars().count() + context * 2).min(chars.len());
    let mut excerpt: String = chars[s..e].iter().collect();
    excerpt = excerpt.replace('\n', " ").trim().to_string();
    if s > 0 {
        excerpt = format!("…{excerpt}");
    }
    if e < chars.len() {
        excerpt.push('…');
    }
    Some(excerpt)
}

/// Newest first, with fork-safe pruning beyond [`MAX_STORED`].
pub fn list_recent() -> Vec<ConversationMeta> {
    let (all, resolved) = load_all_resolved();
    let mut metas: Vec<ConversationMeta> = all
        .values()
        .map(|c| ConversationMeta {
            id: c.id,
            title: c.title.clone(),
            updated_at: c.updated_at,
            snippet: ConversationMeta::snippet_for(
                resolved
                    .get(&c.id)
                    .map(Vec::as_slice)
                    .unwrap_or(&c.messages),
            ),
            is_fork: c.parent_id.is_some(),
        })
        .collect();
    metas.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    for stale in metas.iter().skip(MAX_STORED) {
        if let Some(conversation) = all.get(&stale.id) {
            for child in all
                .values()
                .filter(|c| c.parent_id == Some(conversation.id))
            {
                materialize(child, &all);
            }
            delete_conversation(&stale.id);
        }
    }
    metas.truncate(MAX_STORED);
    metas
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::attach::base64_encode;

    /// The override directory is process-global, so persistence tests that
    /// touch the store must not run concurrently with each other.
    static TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn test_lock() -> std::sync::MutexGuard<'static, ()> {
        TEST_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("test lock")
    }

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "popchat-persist-{}-{}",
            std::process::id(),
            Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        set_override_dir(dir.clone());
        dir
    }

    fn unscratch(dir: &PathBuf) {
        clear_override_dir();
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn legacy_file_without_kind_fields_still_decodes() {
        // Pre-`kind` provider list + conversation without fork keys, unknown
        // fields sprinkled in — everything must be ignored/backfilled.
        let provider_json = serde_json::json!({
            "id": Uuid::new_v4(),
            "name": "Old",
            "baseURL": "https://example.com/v1",
            "isPreset": false,
            "defaultModel": "m",
            "futureField": 123,
        });
        let provider: Provider = serde_json::from_value(provider_json).unwrap();
        assert_eq!(provider.kind, ProviderKind::OpenAICompatible);

        let weird_kind = serde_json::json!({
            "id": Uuid::new_v4(),
            "name": "New",
            "baseURL": "",
            "isPreset": false,
            "defaultModel": "",
            "kind": "quantumTeleport",
        });
        let provider: Provider = serde_json::from_value(weird_kind).unwrap();
        assert_eq!(provider.kind, ProviderKind::OpenAICompatible);

        let convo_json = serde_json::json!({
            "id": Uuid::new_v4(),
            "title": "Old chat",
            "updatedAt": "2026-08-04T12:00:00.123+0800",
            "messages": [
                {"id": Uuid::new_v4(), "role": "user", "text": "hi", "attachments": [], "brandNewField": true},
                {"text": "no id, no role"}
            ],
            "unknownTopLevel": "ignore me",
        });
        let mut value = convo_json.clone();
        Conversation::fix_parent_key(&mut value);
        let convo: Conversation = serde_json::from_value(value).unwrap();
        assert_eq!(convo.title, "Old chat");
        assert_eq!(convo.messages.len(), 2);
        assert_eq!(convo.messages[0].role, ChatRole::User);
        // Missing id/role backfilled.
        assert_eq!(convo.messages[1].role, ChatRole::User);
        assert_eq!(convo.messages[1].text, "no id, no role");
        assert_eq!(convo.parent_id, None);
        // Apple-style +0800 offset parsed as UTC+8 → 04:00Z.
        assert_eq!(
            convo.updated_at.to_rfc3339(),
            "2026-08-04T04:00:00.123+00:00"
        );
    }

    #[test]
    fn save_load_roundtrip_with_macos_keys() {
        let _guard = test_lock();
        let dir = scratch();
        let convo = Conversation {
            id: Uuid::new_v4(),
            title: "Round trip".to_string(),
            updated_at: Utc::now(),
            messages: vec![ChatMessage::user("hello")],
            parent_id: Some(Uuid::new_v4()),
            fork_message_id: Some(Uuid::new_v4()),
        };
        save_conversation(&convo);
        // File uses the macOS `parentID` key…
        let raw = std::fs::read_to_string(conversation_path(&convo.id)).unwrap();
        assert!(raw.contains("parentID"), "got: {raw}");
        assert!(!raw.contains("parentId"), "got: {raw}");
        // …and loads back identically.
        let back = load_conversation(&convo.id).unwrap();
        assert_eq!(back.id, convo.id);
        assert_eq!(back.messages[0].text, "hello");
        unscratch(&dir);
    }

    #[test]
    fn full_text_index_skips_attachment_base64() {
        let _guard = test_lock();
        let dir = scratch();
        let secret_b64 = base64_encode(b"SUPERSECRETBASE64PAYLOAD");
        let mut msg = ChatMessage::user("visible question");
        msg.reasoning = Some("hidden deliberation".to_string());
        msg.attachments.push(Attachment {
            filename: "shot.png".to_string(),
            mime: "image/png".to_string(),
            data_base64: secret_b64.clone(),
            kind: crate::chat::attach::AttachKind::Image,
            note: None,
            note_is_warning: false,
            extracted_text: None,
        });
        let convo = Conversation {
            id: Uuid::new_v4(),
            title: "Index me".to_string(),
            updated_at: Utc::now(),
            messages: vec![msg],
            parent_id: None,
            fork_message_id: None,
        };
        save_conversation(&convo);
        let index = full_text_index();
        let text = index.get(&convo.id).unwrap();
        assert!(text.contains("visible question"), "got: {text}");
        assert!(text.contains("hidden deliberation"), "got: {text}");
        assert!(text.contains("Index me"), "got: {text}");
        assert!(
            !text.contains(&secret_b64),
            "attachment base64 must not be indexed"
        );
        unscratch(&dir);
    }

    #[test]
    fn config_roundtrip_and_prompt_history() {
        let _guard = test_lock();
        let dir = scratch();
        let mut cfg = AppConfig::load();
        assert!(cfg.prompt_history.is_empty());
        cfg.panel_width = Some(420.0);
        cfg.accent_color = Some("#ff0000".to_string());
        cfg.record_prompt("  hello  ");
        cfg.record_prompt("hello");
        cfg.record_prompt("");
        assert_eq!(cfg.prompt_history, vec!["hello".to_string()]);
        cfg.save();
        let back = AppConfig::load();
        assert_eq!(back.panel_width, Some(420.0));
        assert_eq!(back.accent_color.as_deref(), Some("#ff0000"));
        assert_eq!(back.prompt_history, vec!["hello".to_string()]);
        // Raw file uses the camelCase UserDefaults key names.
        let raw = std::fs::read_to_string(config_path()).unwrap();
        assert!(raw.contains("panelWidth"));
        assert!(raw.contains("promptHistory"));
        unscratch(&dir);
    }
}
