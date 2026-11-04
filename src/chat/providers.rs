//! Provider catalog ported from `ProviderStore.swift` (macOS PopChat).
//!
//! Covers the Foundation-only logic: [`ProviderKind`], [`Provider`] (with the
//! unknown-`kind` fallback so a newer build's kind never fails the whole list
//! decode), the [`FREE_PROVIDER_TEMPLATES`] static table, `/models` fetching
//! (`GET {baseURL}/models` + `Authorization: Bearer`), attachment-capability
//! resolution, and the [`ProviderStore`] state machine.
//!
//! UI-layer pieces (`ObservableObject`, `@Published`, `UserDefaults`) are
//! replaced by an explicit [`ProviderStore::save`]/[`ProviderStore::load`]
//! pair over `%APPDATA%/PopChat/provider_store.json`. ChatGPT OAuth state and
//! the Codex app-server inspection process live in `chatgpt_auth.rs` /
//! `codex.rs`; this file exposes [`ChatGptCatalog`] and [`CodexInspection`]
//! value types plus `apply_*` entry points for publishing their results. The
//! subscription catalog derives from the auth module's pure metadata helpers.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use serde::{Deserialize, Deserializer, Serialize};
use uuid::Uuid;

use crate::chat::client::ProviderConfig;
use crate::chat::store::{app_data_dir, SecretStore};

// MARK: - ProviderKind

/// How PopChat talks to a provider. Almost everything is OpenAI-compatible
/// chat completions with an API key; the two subscription kinds delegate to
/// the backends in `chatgpt_auth.rs` / `codex.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum ProviderKind {
    #[serde(rename = "openAICompatible")]
    OpenAICompatible,
    #[serde(rename = "chatGPT")]
    ChatGPT,
    #[serde(rename = "codexAppServer")]
    CodexAppServer,
}

impl Default for ProviderKind {
    fn default() -> Self {
        ProviderKind::OpenAICompatible
    }
}

impl ProviderKind {
    /// Raw-wire mapping (`openAICompatible` / `chatGPT` / `codexAppServer`).
    /// Unknown strings map to `None` and the caller falls back — see below.
    pub fn from_raw(raw: &str) -> Option<ProviderKind> {
        match raw {
            "openAICompatible" => Some(ProviderKind::OpenAICompatible),
            "chatGPT" => Some(ProviderKind::ChatGPT),
            "codexAppServer" => Some(ProviderKind::CodexAppServer),
            _ => None,
        }
    }

    pub fn as_raw(self) -> &'static str {
        match self {
            ProviderKind::OpenAICompatible => "openAICompatible",
            ProviderKind::ChatGPT => "chatGPT",
            ProviderKind::CodexAppServer => "codexAppServer",
        }
    }
}

// Decode via the raw string and map unknown values to `OpenAICompatible`
// rather than failing the whole `[Provider]` decode — silently resetting
// every provider (and orphaning their per-UUID secrets/models) the first
// time a newer build's provider kind is seen by this build.
impl<'de> Deserialize<'de> for ProviderKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = Option::<String>::deserialize(deserializer)?;
        Ok(raw
            .as_deref()
            .and_then(ProviderKind::from_raw)
            .unwrap_or_default())
    }
}

// MARK: - Provider

/// A provider is an identity — base URL + display name. API keys live in the
/// secrets file keyed by provider id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provider {
    pub id: Uuid,
    pub name: String,
    #[serde(rename = "baseURL")]
    pub base_url: String,
    #[serde(rename = "isPreset")]
    pub is_preset: bool,
    #[serde(rename = "defaultModel")]
    pub default_model: String,
    #[serde(default)]
    pub kind: ProviderKind,
    /// Name of the [`FreeProviderTemplate`] this row was created from, `None`
    /// for everything else. Drives the "Free" badge / get-a-key notice.
    #[serde(rename = "freeTemplate", default)]
    pub free_template: Option<String>,
}

impl Provider {
    pub fn new(
        id: Uuid,
        name: impl Into<String>,
        base_url: impl Into<String>,
        is_preset: bool,
        default_model: impl Into<String>,
        kind: ProviderKind,
        free_template: Option<String>,
    ) -> Self {
        Provider {
            id,
            name: name.into(),
            base_url: base_url.into(),
            is_preset,
            default_model: default_model.into(),
            kind,
            free_template,
        }
    }
}

// MARK: - FreeProviderTemplate

/// A curated free-tier provider the Add Provider menu can instantiate
/// preconfigured: the correct OpenAI-compatible base URL plus where to get a
/// key. Static facts only — models come from the live `/models` fetch once a
/// key is set. Every entry's `/models` route was live-verified OpenAI-shaped
/// on 2026-08-04.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FreeProviderTemplate {
    pub name: &'static str,
    pub base_url: &'static str,
    /// Where the user creates an API key.
    pub key_url: &'static str,
    /// One caveat worth stating up front, or `None`.
    pub note: Option<&'static str>,
}

pub static FREE_PROVIDER_TEMPLATES: &[FreeProviderTemplate] = &[
    FreeProviderTemplate {
        name: "Groq",
        base_url: "https://api.groq.com/openai/v1",
        key_url: "https://console.groq.com/keys",
        note: None,
    },
    FreeProviderTemplate {
        name: "Google Gemini",
        base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
        key_url: "https://aistudio.google.com/apikey",
        note: None,
    },
    FreeProviderTemplate {
        name: "Mistral",
        base_url: "https://api.mistral.ai/v1",
        key_url: "https://console.mistral.ai/api-keys",
        note: Some("free use needs the Experiment plan enabled in the console"),
    },
    FreeProviderTemplate {
        name: "Cerebras",
        base_url: "https://api.cerebras.ai/v1",
        key_url: "https://cloud.cerebras.ai",
        note: None,
    },
    FreeProviderTemplate {
        name: "NVIDIA NIM",
        base_url: "https://integrate.api.nvidia.com/v1",
        key_url: "https://build.nvidia.com",
        note: None,
    },
    FreeProviderTemplate {
        name: "Cohere",
        base_url: "https://api.cohere.ai/compatibility/v1",
        key_url: "https://dashboard.cohere.com/api-keys",
        note: Some("trial keys have a monthly request cap"),
    },
    FreeProviderTemplate {
        name: "Hugging Face",
        base_url: "https://router.huggingface.co/v1",
        key_url: "https://huggingface.co/settings/tokens",
        note: Some("a small monthly credit rather than an unlimited tier"),
    },
];

/// The upstream community catalog this list is curated from.
pub const FREE_PROVIDER_DIRECTORY_URL: &str = "https://github.com/cheahjs/free-llm-api-resources";

pub fn free_template_named(name: &str) -> Option<&'static FreeProviderTemplate> {
    FREE_PROVIDER_TEMPLATES.iter().find(|t| t.name == name)
}

// MARK: - Attachment capabilities

/// The two attachment content kinds whose provider support varies by model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContentCapability {
    Images,
    Files,
}

/// Authoritative per-model capability metadata — only ever written from a
/// source that actually states it. A `None` field means the source didn't
/// say, never a guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ModelCapabilities {
    #[serde(default)]
    pub images: Option<bool>,
    #[serde(default)]
    pub files: Option<bool>,
}

/// A per-model deviation from the resolved default — learned from a provider
/// rejection or set by the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityException {
    #[serde(default)]
    pub images: Option<bool>,
    #[serde(default)]
    pub files: Option<bool>,
    pub source: ExceptionSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExceptionSource {
    Learned,
    Manual,
}

/// What the send path acts on for one provider+model. `None` = unknown: send
/// optimistically and let a rejection surface (and be learned) explicitly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AttachmentCapabilities {
    pub images: Option<bool>,
    pub files: Option<bool>,
}

/// Priority: manual/learned exception > authoritative metadata > kind
/// default — resolved per FIELD, so an exception recording "no images"
/// doesn't erase what metadata says about files.
pub fn resolve_capabilities(
    exception: Option<&CapabilityException>,
    metadata: Option<&ModelCapabilities>,
    kind_default: &AttachmentCapabilities,
) -> AttachmentCapabilities {
    AttachmentCapabilities {
        images: exception
            .and_then(|e| e.images)
            .or_else(|| metadata.and_then(|m| m.images))
            .or(kind_default.images),
        files: exception
            .and_then(|e| e.files)
            .or_else(|| metadata.and_then(|m| m.files))
            .or(kind_default.files),
    }
}

/// What a provider kind promises when nothing model-specific is known. The
/// OpenAI preset defaults to fully capable (its API exposes no capability
/// metadata at all); everything else OpenAI-compatible stays unknown for
/// images (send optimistically) and off for direct PDFs unless the
/// endpoint's toggle says otherwise.
pub fn kind_default_capabilities(
    provider: &Provider,
    pdf_pass_through: bool,
) -> AttachmentCapabilities {
    match provider.kind {
        ProviderKind::ChatGPT | ProviderKind::CodexAppServer => {
            // Fixed catalogs: vision yes; neither path has a file-upload
            // channel, so PDFs stay locally extracted.
            AttachmentCapabilities {
                images: Some(true),
                files: Some(false),
            }
        }
        ProviderKind::OpenAICompatible => {
            if provider.is_preset && provider.base_url.contains("api.openai.com") {
                AttachmentCapabilities {
                    images: Some(true),
                    files: Some(true),
                }
            } else {
                AttachmentCapabilities {
                    images: None,
                    files: Some(!provider.is_preset && pdf_pass_through),
                }
            }
        }
    }
}

/// Records one field of a deviation. `manual` never downgrades back to
/// `learned`: `existing.source == .manual` pins the entry.
pub fn merged_exception(
    existing: Option<&CapabilityException>,
    capability: ContentCapability,
    supported: bool,
    source: ExceptionSource,
) -> CapabilityException {
    let mut entry = existing.copied().unwrap_or(CapabilityException {
        images: None,
        files: None,
        source,
    });
    match capability {
        ContentCapability::Images => entry.images = Some(supported),
        ContentCapability::Files => entry.files = Some(supported),
    }
    entry.source = if existing.map(|e| e.source) == Some(ExceptionSource::Manual) {
        ExceptionSource::Manual
    } else {
        source
    };
    entry
}

// MARK: - /models decode + fetch

#[derive(Debug, Deserialize)]
struct ModelList {
    data: Vec<ModelListEntry>,
}

#[derive(Debug, Deserialize)]
struct ModelListEntry {
    id: String,
    #[serde(default)]
    architecture: Option<ModelArchitecture>,
}

#[derive(Debug, Deserialize)]
struct ModelArchitecture {
    #[serde(rename = "input_modalities", default)]
    input_modalities: Option<Vec<String>>,
}

/// Ids as before, plus capabilities for exactly the entries that state
/// their modalities — absence stays absence, never a guess.
pub fn decode_model_list(
    data: &[u8],
) -> Result<(Vec<String>, HashMap<String, ModelCapabilities>), serde_json::Error> {
    let list: ModelList = serde_json::from_slice(data)?;
    let mut capabilities = HashMap::new();
    for entry in &list.data {
        if let Some(modalities) = entry
            .architecture
            .as_ref()
            .and_then(|a| a.input_modalities.as_ref())
        {
            capabilities.insert(
                entry.id.clone(),
                ModelCapabilities {
                    images: Some(modalities.iter().any(|m| m == "image")),
                    files: Some(modalities.iter().any(|m| m == "file")),
                },
            );
        }
    }
    let mut ids: Vec<String> = list.data.into_iter().map(|e| e.id).collect();
    ids.sort();
    Ok((ids, capabilities))
}

fn models_url(base_url: &str) -> String {
    format!("{}/models", base_url.trim_end_matches('/'))
}

/// `GET {baseURL}/models` with `Authorization: Bearer` when a key is set.
/// Returns `(sorted ids, list-stated capabilities)` or a user-facing message.
pub async fn fetch_models_list(
    provider_name: &str,
    base_url: &str,
    api_key: &str,
) -> Result<(Vec<String>, HashMap<String, ModelCapabilities>), String> {
    if base_url.trim().is_empty() {
        return Err(format!("Add a base URL for {provider_name} first."));
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| format!("Fetching models failed: {e}"))?;
    let mut request = client.get(models_url(base_url));
    if !api_key.is_empty() {
        request = request.header("Authorization", format!("Bearer {api_key}"));
    }
    let response = request
        .send()
        .await
        .map_err(|e| format!("Fetching models failed: {e}"))?;
    if response.status() != reqwest::StatusCode::OK {
        let code = response.status().as_u16();
        let body: String = response
            .text()
            .await
            .unwrap_or_default()
            .chars()
            .take(300)
            .collect();
        return Err(format!("HTTP {code} fetching models: {body}"));
    }
    let data = response
        .bytes()
        .await
        .map_err(|e| format!("Fetching models failed: {e}"))?;
    let (ids, capabilities) =
        decode_model_list(&data).map_err(|e| format!("Fetching models failed: {e}"))?;
    if ids.is_empty() {
        return Err(format!("{provider_name} returned an empty model list."));
    }
    Ok((ids, capabilities))
}

/// Ollama's OpenAI-compatible surface exposes no capabilities, but its native
/// API does: `/api/show` lists them per model ("vision" is the image answer;
/// there is no file input at all). Detection is by probing `/api/version` at
/// the same host rather than by preset name, so a custom provider pointed at
/// an Ollama gets the same authoritative answer.
pub async fn probe_ollama_capabilities(
    base_url: &str,
    models: &[String],
) -> HashMap<String, ModelCapabilities> {
    let root = base_url.trim_end_matches('/').to_string();
    let Some(host_root) = root.strip_suffix("/v1") else {
        return HashMap::new();
    };
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(_) => return HashMap::new(),
    };
    #[allow(dead_code)]
    #[derive(Deserialize)]
    struct Version {
        version: String,
    }
    let version_url = format!("{host_root}/api/version");
    let is_ollama = client
        .get(&version_url)
        .timeout(Duration::from_secs(4))
        .send()
        .await
        .ok()
        .filter(|r| r.status() == reqwest::StatusCode::OK);
    let Some(resp) = is_ollama else {
        return HashMap::new();
    };
    let Ok(_version) = resp.json::<Version>().await else {
        return HashMap::new();
    };
    #[derive(Deserialize)]
    struct Show {
        #[serde(default)]
        capabilities: Option<Vec<String>>,
    }
    let show_url = format!("{host_root}/api/show");
    // A local server answers these in milliseconds; the cap is a backstop
    // against someone proxying a huge catalog through an Ollama-shaped URL.
    // A JoinSet aborts all outstanding probes when Settings cancels/replaces
    // this catalog request. Detached JoinHandles outlived the old draft.
    let mut tasks = tokio::task::JoinSet::new();
    for model in models.iter().take(100).cloned() {
        let client = client.clone();
        let url = show_url.clone();
        tasks.spawn(async move {
            let resp = client
                .post(&url)
                .json(&serde_json::json!({ "model": model }))
                .timeout(Duration::from_secs(10))
                .send()
                .await
                .ok()?;
            if resp.status() != reqwest::StatusCode::OK {
                return None;
            }
            let show: Show = resp.json().await.ok()?;
            let caps = show.capabilities?;
            Some((
                model,
                ModelCapabilities {
                    images: Some(caps.iter().any(|c| c == "vision")),
                    files: Some(false),
                },
            ))
        });
    }
    let mut result = HashMap::new();
    while let Some(task) = tasks.join_next().await {
        if let Ok(Some((model, caps))) = task {
            result.insert(model, caps);
        }
    }
    result
}

// MARK: - ChatGPT catalog (fixed; backend has no /models endpoint)

/// Fixed subscription catalog derived from `chatgpt_auth`'s single source of
/// truth: default model, catalog, and hand-maintained reasoning-effort table.
#[derive(Debug, Clone)]
pub struct ChatGptCatalog {
    pub default_model: String,
    pub models: Vec<String>,
    pub supported_efforts: HashMap<String, Vec<String>>,
    pub default_efforts: HashMap<String, String>,
}

pub fn chatgpt_catalog() -> ChatGptCatalog {
    use super::chatgpt_auth::{
        default_reasoning_effort, supported_reasoning_efforts, DEFAULT_MODEL, MODEL_CATALOG,
    };

    let models: Vec<String> = MODEL_CATALOG
        .iter()
        .map(|model| (*model).to_owned())
        .collect();
    let mut supported_efforts = HashMap::new();
    let mut default_efforts = HashMap::new();
    for model in &models {
        supported_efforts.insert(
            model.clone(),
            supported_reasoning_efforts(model)
                .iter()
                .map(|effort| (*effort).to_owned())
                .collect(),
        );
        if let Some(effort) = default_reasoning_effort(model) {
            default_efforts.insert(model.clone(), effort.to_owned());
        }
    }
    ChatGptCatalog {
        default_model: DEFAULT_MODEL.to_owned(),
        models,
        supported_efforts,
        default_efforts,
    }
}

// MARK: - Codex app-server state (inspection itself lives in codex.rs)

/// Mirrors `CodexAppServerStatus` (CodexAppServerClient.swift).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodexAppServerStatus {
    Unknown,
    Checking,
    Ready {
        email: Option<String>,
        plan: Option<String>,
    },
    Missing,
    NotSignedIn,
    Failed(String),
}

impl Default for CodexAppServerStatus {
    fn default() -> Self {
        CodexAppServerStatus::Unknown
    }
}

/// What `codex.rs` publishes after inspecting the installed Codex process.
/// Mirrors `CodexAppServerClient.Inspection`.
#[derive(Debug, Clone, Default)]
pub struct CodexInspection {
    pub email: Option<String>,
    pub plan: Option<String>,
    pub models: Vec<String>,
    pub default_model: Option<String>,
    pub supported_efforts: HashMap<String, Vec<String>>,
    pub default_efforts: HashMap<String, String>,
}

/// Typed failure reason — mapped from the typed reason, never from message
/// prose. Mirrors `CodexAppServerClient.ClientError.reason`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexFailureReason {
    Missing,
    NotSignedIn,
    ProtocolFailure,
}

#[derive(Debug, Clone)]
pub struct CodexFailure {
    pub message: String,
    pub reason: CodexFailureReason,
}

// MARK: - ProviderStore

fn store_file_path() -> std::path::PathBuf {
    app_data_dir().join("provider_store.json")
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct PersistedProviderState {
    #[serde(default)]
    providers: Option<Vec<Provider>>,
    #[serde(default)]
    selected_id: Option<Uuid>,
    #[serde(default)]
    known_models: HashMap<Uuid, Vec<String>>,
    #[serde(default)]
    selected_models: HashMap<Uuid, String>,
    #[serde(default)]
    settings_models: HashMap<Uuid, String>,
    #[serde(default)]
    known_model_efforts: HashMap<Uuid, HashMap<String, Vec<String>>>,
    #[serde(default)]
    default_model_efforts: HashMap<Uuid, HashMap<String, String>>,
    #[serde(default)]
    selected_model_efforts: HashMap<Uuid, HashMap<String, String>>,
    #[serde(default)]
    context_limits: HashMap<Uuid, HashMap<String, u32>>,
    #[serde(default)]
    model_capabilities: HashMap<Uuid, HashMap<String, ModelCapabilities>>,
    #[serde(default)]
    capability_exceptions: HashMap<Uuid, HashMap<String, CapabilityException>>,
    #[serde(default)]
    pdf_pass_through: HashMap<Uuid, bool>,
}

/// What [`ProviderStore::lazy_fetch_plan`] tells the UI to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LazyFetchPlan {
    Nothing,
    FetchModels,
    RefreshCodex,
}

pub struct ProviderStore {
    storage_path: std::path::PathBuf,
    /// A failed load permanently disables writes for this instance, so later
    /// background saves cannot replace unreadable data with fallback presets.
    load_error: Option<String>,
    pub providers: Vec<Provider>,
    pub selected_id: Uuid,
    /// Last-fetched `/models` list per provider — persisted so the switcher
    /// works offline.
    pub known_models: HashMap<Uuid, Vec<String>>,
    pub selected_models: HashMap<Uuid, String>,
    /// Explicitly saved Settings editor choices, separate from live selection.
    /// Legacy stores have no entries: the editor falls back to remembered_model.
    pub settings_models: HashMap<Uuid, String>,
    pub known_model_efforts: HashMap<Uuid, HashMap<String, Vec<String>>>,
    pub default_model_efforts: HashMap<Uuid, HashMap<String, String>>,
    pub selected_model_efforts: HashMap<Uuid, HashMap<String, String>>,
    /// Explicit context-window overrides, scoped by provider UUID + model ID.
    /// Kept separate from API drafts and output-token generation parameters.
    context_limits: HashMap<Uuid, HashMap<String, u32>>,
    pub model_capabilities: HashMap<Uuid, HashMap<String, ModelCapabilities>>,
    pub capability_exceptions: HashMap<Uuid, HashMap<String, CapabilityException>>,
    /// Custom endpoints only: send PDFs as native `file` content parts.
    pub pdf_pass_through: HashMap<Uuid, bool>,
    /// Per-provider fetch errors, rendered next to the provider they belong to.
    pub model_fetch_errors: HashMap<Uuid, String>,
    pub fetching_providers: HashSet<Uuid>,
    pub codex_app_server_status: CodexAppServerStatus,
    /// Providers the switcher has already auto-fetched this launch.
    auto_fetched: HashSet<Uuid>,
    /// Bumped whenever the configured Codex executable changes; a check that
    /// started under an older generation must be dropped, not published.
    codex_generation: u64,
}

impl ProviderStore {
    #[cfg(test)]
    pub(crate) fn isolated_file_for_tests(path: std::path::PathBuf) -> Self {
        Self::load_at(path, |_| None)
    }

    pub const CHATGPT_PROVIDER_NAME: &'static str = "OpenAI subscription (unofficial)";

    pub fn load(secrets: &SecretStore) -> Self {
        Self::load_at(store_file_path(), |id| secrets.get(&id.to_string()))
    }

    fn load_at(
        storage_path: std::path::PathBuf,
        mut lookup_key: impl FnMut(Uuid) -> Option<String>,
    ) -> Self {
        let loaded = match std::fs::read(&storage_path) {
            Ok(data) => (|| {
                // Serde structs can also accept JSON arrays. Require the actual
                // on-disk object schema, including a catalog, before migration.
                let value: serde_json::Value = serde_json::from_slice(&data).ok()?;
                let providers = value.as_object()?.get("providers")?.as_array()?;
                if !providers.iter().all(serde_json::Value::is_object) {
                    return None;
                }
                serde_json::from_value::<PersistedProviderState>(value).ok()
            })()
            .ok_or("Provider storage could not be decoded; existing data was preserved and saving is disabled until restart."),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(PersistedProviderState::default())
            }
            Err(_) => Err("Provider storage could not be read; existing data was preserved and saving is disabled until restart."),
        };
        let (state, load_error) = match loaded {
            Ok(state) => (state, None),
            Err(error) => (PersistedProviderState::default(), Some(error.to_owned())),
        };
        // Never access credentials for fallback identities after a failed load.
        let mut key_for = |id| {
            if load_error.is_none() {
                lookup_key(id).unwrap_or_default()
            } else {
                String::new()
            }
        };
        let mut migrated = state.providers.unwrap_or_else(Self::presets);
        let selected_id = state.selected_id;

        // Retired presets: drop the untouched ones, keep anything set up.
        migrated.retain(|provider| {
            if !(provider.is_preset
                && Self::retired_preset_names().contains(&provider.name.as_str()))
            {
                return true;
            }
            let configured = !key_for(provider.id).is_empty()
                || !state
                    .known_models
                    .get(&provider.id)
                    .map(|m| m.is_empty())
                    .unwrap_or(true)
                || state.selected_models.contains_key(&provider.id);
            configured || Some(provider.id) == selected_id
        });
        // Provider lists saved before the ChatGPT preset existed: add it once
        // (or heal a round-trip through a build that stripped its `kind`).
        if !migrated.iter().any(|p| p.kind == ProviderKind::ChatGPT) {
            if let Some(index) = migrated.iter().position(|p| {
                p.is_preset && Self::chatgpt_provider_names().contains(&p.name.as_str())
            }) {
                migrated[index].kind = ProviderKind::ChatGPT;
                migrated[index].base_url = String::new();
                migrated[index].default_model = chatgpt_catalog().default_model;
            } else {
                let at = migrated.len().min(1);
                migrated.insert(at, Self::chatgpt_preset());
            }
        }
        // Same for the local Codex bridge.
        if !migrated
            .iter()
            .any(|p| p.kind == ProviderKind::CodexAppServer)
        {
            if let Some(index) = migrated
                .iter()
                .position(|p| p.is_preset && p.name == Self::codex_app_server_provider_name())
            {
                migrated[index].kind = ProviderKind::CodexAppServer;
                migrated[index].base_url = String::new();
            } else {
                let at = migrated.len().min(1);
                migrated.insert(at, Self::codex_app_server_preset());
            }
        }
        // Keep the direct route's risk label current without replacing its id.
        for provider in migrated.iter_mut().filter(|p| {
            p.is_preset && p.kind == ProviderKind::ChatGPT && p.name != Self::CHATGPT_PROVIDER_NAME
        }) {
            provider.name = Self::CHATGPT_PROVIDER_NAME.to_string();
        }

        // A stale selection must not hide an otherwise intact catalog.
        let selected_id = selected_id.filter(|id| migrated.iter().any(|p| p.id == *id));
        let mut store = ProviderStore {
            storage_path,
            load_error: load_error.clone(),
            selected_id: selected_id.unwrap_or_else(|| migrated[0].id),
            providers: migrated,
            known_models: state.known_models,
            selected_models: state.selected_models,
            settings_models: state.settings_models,
            known_model_efforts: state.known_model_efforts,
            default_model_efforts: state.default_model_efforts,
            selected_model_efforts: state.selected_model_efforts,
            context_limits: state.context_limits,
            model_capabilities: state.model_capabilities,
            capability_exceptions: state.capability_exceptions,
            pdf_pass_through: state.pdf_pass_through,
            model_fetch_errors: HashMap::new(),
            fetching_providers: HashSet::new(),
            codex_app_server_status: CodexAppServerStatus::Unknown,
            auto_fetched: HashSet::new(),
            codex_generation: 0,
        };
        // Missing/stale selection: prefer whatever IS configured over the
        // positional fallback.
        if selected_id.is_none() {
            let configured = store
                .providers
                .iter()
                .find(|p| {
                    let key = key_for(p.id);
                    Self::is_configured_static(
                        p,
                        &key,
                        &store.known_models,
                        &store.selected_models,
                        false,
                        &store.codex_app_server_status,
                    )
                })
                .map(|p| p.id);
            if let Some(id) = configured {
                store.selected_id = id;
            }
        }
        store.apply_chatgpt_catalog(&chatgpt_catalog());
        if store.load_error.is_none() {
            store.save();
        }
        store
    }

    /// Privacy-safe startup warning. No source JSON, credentials or path is
    /// included. A failed-load instance refuses all persistence until reloaded.
    pub fn load_error(&self) -> Option<&str> {
        self.load_error.as_deref()
    }

    pub fn save(&self) {
        if let Err(error) = self.try_save() {
            crate::logf!("[providers] could not save provider store: {error}");
        }
    }

    pub fn try_save(&self) -> anyhow::Result<()> {
        if let Some(error) = self.load_error() {
            anyhow::bail!("{error}");
        }
        let state = PersistedProviderState {
            providers: Some(self.providers.clone()),
            selected_id: Some(self.selected_id),
            known_models: self.known_models.clone(),
            selected_models: self.selected_models.clone(),
            settings_models: self.settings_models.clone(),
            known_model_efforts: self.known_model_efforts.clone(),
            default_model_efforts: self.default_model_efforts.clone(),
            selected_model_efforts: self.selected_model_efforts.clone(),
            context_limits: self.context_limits.clone(),
            model_capabilities: self.model_capabilities.clone(),
            capability_exceptions: self.capability_exceptions.clone(),
            pdf_pass_through: self.pdf_pass_through.clone(),
        };
        let path = &self.storage_path;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let data = serde_json::to_vec_pretty(&state)?;
        crate::paths::atomic_write(path, &data)?;
        Ok(())
    }

    pub fn chatgpt_preset() -> Provider {
        Provider::new(
            Uuid::new_v4(),
            Self::CHATGPT_PROVIDER_NAME,
            "",
            true,
            chatgpt_catalog().default_model,
            ProviderKind::ChatGPT,
            None,
        )
    }

    pub fn codex_app_server_provider_name() -> &'static str {
        "OpenAI (Codex app-server)"
    }

    pub fn codex_app_server_preset() -> Provider {
        Provider::new(
            Uuid::new_v4(),
            Self::codex_app_server_provider_name(),
            "",
            true,
            "",
            ProviderKind::CodexAppServer,
            None,
        )
    }

    pub fn presets() -> Vec<Provider> {
        vec![
            Self::chatgpt_preset(),
            Self::codex_app_server_preset(),
            Provider::new(
                Uuid::new_v4(),
                "OpenAI",
                "https://api.openai.com/v1",
                true,
                "gpt-4o",
                ProviderKind::OpenAICompatible,
                None,
            ),
            Provider::new(
                Uuid::new_v4(),
                "OpenRouter",
                "https://openrouter.ai/api/v1",
                true,
                "openrouter/auto",
                ProviderKind::OpenAICompatible,
                None,
            ),
            Provider::new(
                Uuid::new_v4(),
                "Ollama (local)",
                "http://localhost:11434/v1",
                true,
                "",
                ProviderKind::OpenAICompatible,
                None,
            ),
        ]
    }

    fn retired_preset_names() -> &'static [&'static str] {
        &["DeepSeek", "LM Studio (local)"]
    }

    fn chatgpt_provider_names() -> Vec<&'static str> {
        vec![
            Self::CHATGPT_PROVIDER_NAME,
            "OpenAI (subscription)",
            "OpenAI (ChatGPT)",
        ]
    }

    // MARK: - Selection

    pub fn selected_provider(&self) -> Option<&Provider> {
        self.providers.iter().find(|p| p.id == self.selected_id)
    }

    fn is_configured_static(
        provider: &Provider,
        key: &str,
        known_models: &HashMap<Uuid, Vec<String>>,
        selected_models: &HashMap<Uuid, String>,
        chatgpt_signed_in: bool,
        codex_status: &CodexAppServerStatus,
    ) -> bool {
        if provider.kind == ProviderKind::ChatGPT {
            return chatgpt_signed_in;
        }
        if provider.kind == ProviderKind::CodexAppServer {
            return match codex_status {
                CodexAppServerStatus::Ready { .. } => true,
                // Per-PROCESS status starts at Unknown every launch; a
                // persisted catalog is evidence the last check succeeded.
                CodexAppServerStatus::Unknown | CodexAppServerStatus::Checking => !known_models
                    .get(&provider.id)
                    .map(|m| m.is_empty())
                    .unwrap_or(true),
                CodexAppServerStatus::Missing
                | CodexAppServerStatus::NotSignedIn
                | CodexAppServerStatus::Failed(_) => false,
            };
        }
        !key.is_empty()
            || !known_models
                .get(&provider.id)
                .map(|m| m.is_empty())
                .unwrap_or(true)
            || selected_models.contains_key(&provider.id)
    }

    /// Is this provider set up enough to talk to? The single predicate behind
    /// both the switcher rail and the Settings green dots — don't fork it.
    pub fn is_configured(
        &self,
        provider: &Provider,
        secrets: &SecretStore,
        chatgpt_signed_in: bool,
    ) -> bool {
        let key = secrets.get(&provider.id.to_string()).unwrap_or_default();
        Self::is_configured_static(
            provider,
            &key,
            &self.known_models,
            &self.selected_models,
            chatgpt_signed_in,
            &self.codex_app_server_status,
        )
    }

    /// Providers worth offering in the quick switcher. The active provider is
    /// always included even if it stopped qualifying.
    pub fn configured_providers(
        &self,
        secrets: &SecretStore,
        chatgpt_signed_in: bool,
    ) -> Vec<Provider> {
        self.providers
            .iter()
            .filter(|p| {
                self.is_configured(p, secrets, chatgpt_signed_in) || p.id == self.selected_id
            })
            .cloned()
            .collect()
    }

    /// "Was this ever set up?" (for the header pill) as opposed to
    /// `is_configured`'s "is it usable right now?".
    pub fn has_setup_evidence(
        &self,
        provider: &Provider,
        secrets: &SecretStore,
        chatgpt_signed_in: bool,
    ) -> bool {
        if provider.kind == ProviderKind::ChatGPT {
            return chatgpt_signed_in;
        }
        let key = secrets.get(&provider.id.to_string()).unwrap_or_default();
        !key.is_empty()
            || !self
                .known_models
                .get(&provider.id)
                .map(|m| m.is_empty())
                .unwrap_or(true)
            || self.selected_models.contains_key(&provider.id)
    }

    pub fn current_model(&self) -> String {
        let Some(provider) = self.selected_provider() else {
            return String::new();
        };
        self.selected_models
            .get(&provider.id)
            .cloned()
            .unwrap_or_else(|| provider.default_model.clone())
    }

    pub fn current_reasoning_effort(&self) -> Option<String> {
        self.remembered_effort(self.selected_id, &self.current_model())
    }

    pub fn context_limit_override(&self, id: Uuid, model: &str) -> Option<u32> {
        self.context_limits
            .get(&id)
            .and_then(|limits| limits.get(model.trim()))
            .copied()
            .filter(|tokens| *tokens > 0)
    }

    pub fn context_limit(&self, id: Uuid, model: &str) -> u32 {
        self.context_limit_override(id, model)
            .unwrap_or_else(|| super::context::default_context_limit(model))
    }

    pub fn current_context_limit(&self) -> u32 {
        self.context_limit(self.selected_id, &self.current_model())
    }

    /// Explicit preference save only: no model selection, endpoint or secret
    /// changes. Restore in-memory state too if the atomic write fails.
    pub fn set_context_limit(
        &mut self,
        id: Uuid,
        model: &str,
        limit: Option<u32>,
    ) -> anyhow::Result<()> {
        if let Some(error) = self.load_error() {
            anyhow::bail!("{error}");
        }
        let model = model.trim();
        anyhow::ensure!(
            self.providers.iter().any(|provider| provider.id == id),
            "Provider no longer exists"
        );
        anyhow::ensure!(
            !model.is_empty(),
            "Select a model before setting its context limit"
        );
        anyhow::ensure!(limit != Some(0), "Context limit must be positive");
        let previous = self.context_limits.get(&id).cloned();
        if let Some(limit) = limit {
            self.context_limits
                .entry(id)
                .or_default()
                .insert(model.to_owned(), limit);
        } else if let Some(limits) = self.context_limits.get_mut(&id) {
            limits.remove(model);
            if limits.is_empty() {
                self.context_limits.remove(&id);
            }
        }
        if let Err(error) = self.try_save() {
            match previous {
                Some(limits) => {
                    self.context_limits.insert(id, limits);
                }
                None => {
                    self.context_limits.remove(&id);
                }
            }
            return Err(error);
        }
        Ok(())
    }

    /// The model a provider would come back with — its remembered choice,
    /// else its default.
    pub fn remembered_model(&self, id: Uuid) -> String {
        self.selected_models.get(&id).cloned().unwrap_or_else(|| {
            self.providers
                .iter()
                .find(|p| p.id == id)
                .map(|p| p.default_model.clone())
                .unwrap_or_default()
        })
    }

    pub fn set_model(&mut self, model: impl Into<String>) {
        let id = self.selected_id;
        self.set_model_for(model, id);
    }

    pub fn set_model_for(&mut self, model: impl Into<String>, id: Uuid) {
        self.selected_models.insert(id, model.into());
        self.save();
    }

    pub fn supported_efforts(&self, id: Uuid, model: &str) -> Vec<String> {
        self.known_model_efforts
            .get(&id)
            .and_then(|m| m.get(model))
            .cloned()
            .unwrap_or_default()
    }

    pub fn remembered_effort(&self, id: Uuid, model: &str) -> Option<String> {
        let supported = self.supported_efforts(id, model);
        if supported.is_empty() {
            return None;
        }
        if let Some(selected) = self
            .selected_model_efforts
            .get(&id)
            .and_then(|m| m.get(model))
        {
            if supported.contains(selected) {
                return Some(selected.clone());
            }
        }
        if let Some(default) = self
            .default_model_efforts
            .get(&id)
            .and_then(|m| m.get(model))
        {
            if supported.contains(default) {
                return Some(default.clone());
            }
        }
        supported.into_iter().next()
    }

    /// The effort the user actually PICKED for this pair, if any — as opposed
    /// to `remembered_effort`, which falls back to the advertised default.
    pub fn chosen_effort(&self, id: Uuid, model: &str) -> Option<String> {
        self.selected_model_efforts
            .get(&id)
            .and_then(|m| m.get(model))
            .cloned()
    }

    pub fn set_effort(&mut self, effort: impl Into<String>, id: Uuid, model: &str) {
        let effort = effort.into();
        if !self.supported_efforts(id, model).contains(&effort) {
            return;
        }
        self.selected_model_efforts
            .entry(id)
            .or_default()
            .insert(model.to_string(), effort);
        self.save();
    }

    pub fn has_effort_models(&self, id: Uuid) -> bool {
        self.known_model_efforts
            .get(&id)
            .map(|m| m.values().any(|e| !e.is_empty()))
            .unwrap_or(false)
    }

    // MARK: - Attachment capabilities

    /// The one answer the send path, the composer warning and the switcher
    /// glyphs all read.
    pub fn attachment_capabilities(&self, id: Uuid, model: &str) -> AttachmentCapabilities {
        let Some(provider) = self.providers.iter().find(|p| p.id == id) else {
            return AttachmentCapabilities::default();
        };
        resolve_capabilities(
            self.capability_exceptions
                .get(&id)
                .and_then(|m| m.get(model)),
            self.model_capabilities.get(&id).and_then(|m| m.get(model)),
            &kind_default_capabilities(provider, self.pdf_pass_through.get(&id) == Some(&true)),
        )
    }

    pub fn current_attachment_capabilities(&self) -> AttachmentCapabilities {
        let model = self.current_model();
        self.attachment_capabilities(self.selected_id, &model)
    }

    /// Records that this provider+model refused a content kind.
    pub fn record_capability_exception(
        &mut self,
        capability: ContentCapability,
        supported: bool,
        source: ExceptionSource,
        provider_id: Uuid,
        model: &str,
    ) {
        let entry = {
            let existing = self
                .capability_exceptions
                .get(&provider_id)
                .and_then(|m| m.get(model));
            merged_exception(existing, capability, supported, source)
        };
        self.capability_exceptions
            .entry(provider_id)
            .or_default()
            .insert(model.to_string(), entry);
        self.save();
    }

    /// The Settings ✕: clear back to the default and simply try again.
    pub fn clear_capability_exception(&mut self, provider_id: Uuid, model: &str) {
        if let Some(for_provider) = self.capability_exceptions.get_mut(&provider_id) {
            for_provider.remove(model);
            if for_provider.is_empty() {
                self.capability_exceptions.remove(&provider_id);
            }
            self.save();
        }
    }

    /// Provider + model committed together — the switcher's unit of choice.
    pub fn select(&mut self, id: Uuid, model: &str, effort: Option<&str>) {
        if !model.is_empty() {
            self.selected_models.insert(id, model.to_string());
        }
        if let Some(effort) = effort {
            self.set_effort(effort, id, model);
        }
        self.selected_id = id;
        self.save();
    }

    /// Seeds the fixed ChatGPT catalog onto the ChatGPT provider specifically,
    /// not `selected_provider`. Retired cached choices fall back to its current
    /// default; custom/API providers and the live provider identity stay intact.
    pub fn apply_chatgpt_catalog(&mut self, catalog: &ChatGptCatalog) {
        let Some(provider) = self
            .providers
            .iter_mut()
            .find(|p| p.kind == ProviderKind::ChatGPT)
        else {
            return;
        };
        let id = provider.id;
        provider.default_model = catalog.default_model.clone();
        for selections in [&mut self.selected_models, &mut self.settings_models] {
            if let Some(model) = selections.get_mut(&id) {
                if super::chatgpt_auth::RETIRED_SUBSCRIPTION_MODELS.contains(&model.as_str()) {
                    *model = catalog.default_model.clone();
                }
            }
        }
        self.known_models.insert(id, catalog.models.clone());
        self.known_model_efforts
            .insert(id, catalog.supported_efforts.clone());
        self.default_model_efforts
            .insert(id, catalog.default_efforts.clone());
    }

    // MARK: - Codex app-server publishing (inspection runs in codex.rs)

    /// Marks the check in flight; the caller runs the inspection and then
    /// calls [`Self::apply_codex_inspection`]. Returns the provider id.
    pub fn mark_codex_checking(&mut self) -> Option<Uuid> {
        let provider = self
            .providers
            .iter()
            .find(|p| p.kind == ProviderKind::CodexAppServer)?;
        let id = provider.id;
        self.codex_app_server_status = CodexAppServerStatus::Checking;
        self.fetching_providers.insert(id);
        Some(id)
    }

    /// Publishes an inspection result. Drops results from a stale generation
    /// (the executable path changed mid-check) and reports whether the
    /// result was applied.
    pub fn apply_codex_inspection(
        &mut self,
        generation: u64,
        include_models: bool,
        result: Result<CodexInspection, CodexFailure>,
    ) -> bool {
        let Some(provider) = self
            .providers
            .iter()
            .find(|p| p.kind == ProviderKind::CodexAppServer)
            .cloned()
        else {
            return false;
        };
        if generation != self.codex_generation {
            return false;
        }
        self.fetching_providers.remove(&provider.id);
        match result {
            Ok(inspection) => {
                self.model_fetch_errors.remove(&provider.id);
                if include_models {
                    let mut models = inspection.models;
                    models.sort();
                    self.known_models.insert(provider.id, models.clone());
                    self.known_model_efforts
                        .insert(provider.id, inspection.supported_efforts);
                    self.default_model_efforts
                        .insert(provider.id, inspection.default_efforts);
                    // The discovered default belongs on the PROVIDER, not in
                    // selected_models: a background refresh must never write a
                    // selection for a provider the user isn't on.
                    let discovered = inspection.default_model.or_else(|| models.first().cloned());
                    if let Some(discovered) = discovered {
                        if let Some(entry) = self.providers.iter_mut().find(|p| p.id == provider.id)
                        {
                            if entry.default_model != discovered {
                                entry.default_model = discovered;
                            }
                        }
                    }
                }
                self.codex_app_server_status = CodexAppServerStatus::Ready {
                    email: inspection.email,
                    plan: inspection.plan,
                };
            }
            Err(failure) => {
                // Mapped from the typed reason, never from message prose.
                self.codex_app_server_status = match failure.reason {
                    CodexFailureReason::Missing => CodexAppServerStatus::Missing,
                    CodexFailureReason::NotSignedIn => CodexAppServerStatus::NotSignedIn,
                    CodexFailureReason::ProtocolFailure => {
                        CodexAppServerStatus::Failed(failure.message.clone())
                    }
                };
                self.model_fetch_errors.insert(provider.id, failure.message);
            }
        }
        self.save();
        true
    }

    /// Invalidates everything learned about the installed Codex. Called when
    /// the configured path changes.
    pub fn invalidate_codex_app_server(&mut self) {
        self.codex_generation += 1;
        self.codex_app_server_status = CodexAppServerStatus::Unknown;
        let Some(provider) = self
            .providers
            .iter()
            .find(|p| p.kind == ProviderKind::CodexAppServer)
            .cloned()
        else {
            return;
        };
        self.auto_fetched.remove(&provider.id);
        self.fetching_providers.remove(&provider.id);
        self.model_fetch_errors.remove(&provider.id);
    }

    pub fn codex_generation(&self) -> u64 {
        self.codex_generation
    }

    pub fn current_key(&self, secrets: &SecretStore) -> String {
        self.key_for(self.selected_id, secrets)
    }

    pub fn key_for(&self, id: Uuid, secrets: &SecretStore) -> String {
        secrets.get(&id.to_string()).unwrap_or_default()
    }

    pub fn set_key(&self, key: &str, id: Uuid, secrets: &mut SecretStore) -> anyhow::Result<()> {
        secrets.set(key, &id.to_string())
    }

    pub fn current_config(&self, secrets: &SecretStore) -> Option<ProviderConfig> {
        let provider = self.selected_provider()?;
        Some(ProviderConfig {
            base_url: provider.base_url.clone(),
            api_key: self.current_key(secrets),
            model: self.current_model(),
            reasoning_effort: self.current_reasoning_effort(),
            kind: provider.kind,
        })
    }

    // MARK: - Custom providers

    /// Adds a half-configured provider. Deliberately does NOT select it:
    /// Settings manages the catalog, the pill owns the live conversation.
    pub fn add_custom(&mut self) -> Uuid {
        let provider = Provider::new(
            Uuid::new_v4(),
            self.unique_name("Custom"),
            "",
            false,
            "",
            ProviderKind::OpenAICompatible,
            None,
        );
        let id = provider.id;
        self.providers.push(provider);
        self.save();
        id
    }

    /// Adds a row preconfigured from a free-tier template — a normal custom
    /// provider that remembers its origin. Same never-select law.
    pub fn add_template(&mut self, template: &FreeProviderTemplate) -> Uuid {
        let provider = Provider::new(
            Uuid::new_v4(),
            self.unique_name(template.name),
            template.base_url,
            false,
            "",
            ProviderKind::OpenAICompatible,
            Some(template.name.to_string()),
        );
        let id = provider.id;
        self.providers.push(provider);
        self.save();
        id
    }

    fn unique_name(&self, base: &str) -> String {
        let mut name = base.to_string();
        let mut suffix = 2;
        while self.providers.iter().any(|p| p.name == name) {
            name = format!("{base} {suffix}");
            suffix += 1;
        }
        name
    }

    /// Deletes a custom provider (presets stay), dropping its key and cached
    /// model state too.
    pub fn remove(&mut self, id: Uuid, secrets: &mut SecretStore) -> anyhow::Result<()> {
        if self.providers.len() <= 1 {
            return Ok(());
        }
        let Some(index) = self.providers.iter().position(|p| p.id == id) else {
            return Ok(());
        };
        if self.providers[index].is_preset {
            return Ok(());
        }
        secrets.delete(&id.to_string())?;
        self.known_models.remove(&id);
        self.selected_models.remove(&id);
        self.settings_models.remove(&id);
        self.known_model_efforts.remove(&id);
        self.default_model_efforts.remove(&id);
        self.selected_model_efforts.remove(&id);
        self.context_limits.remove(&id);
        self.model_capabilities.remove(&id);
        self.capability_exceptions.remove(&id);
        self.pdf_pass_through.remove(&id);
        self.model_fetch_errors.remove(&id);
        self.auto_fetched.remove(&id);
        self.providers.remove(index);
        if self.selected_id == id {
            self.selected_id = self.providers[0].id;
        }
        self.save();
        Ok(())
    }

    // MARK: - Model list

    pub fn is_fetching(&self, id: Uuid) -> bool {
        self.fetching_providers.contains(&id)
    }

    /// What the switcher should do when it lands on a provider: at most one
    /// auto-fetch per provider per launch.
    pub fn lazy_fetch_plan(&mut self, id: Uuid, api_key: &str) -> LazyFetchPlan {
        let Some(provider) = self.providers.iter().find(|p| p.id == id).cloned() else {
            return LazyFetchPlan::Nothing;
        };
        if provider.kind == ProviderKind::CodexAppServer {
            if self.auto_fetched.contains(&id)
                || self.codex_app_server_status == CodexAppServerStatus::Checking
            {
                return LazyFetchPlan::Nothing;
            }
            self.auto_fetched.insert(id);
            return LazyFetchPlan::RefreshCodex;
        }
        if !self
            .known_models
            .get(&id)
            .map(|m| m.is_empty())
            .unwrap_or(true)
            || self.auto_fetched.contains(&id)
            || self.is_fetching(id)
        {
            return LazyFetchPlan::Nothing;
        }
        let has_key = !api_key.is_empty();
        let is_local =
            provider.base_url.contains("localhost") || provider.base_url.contains("127.0.0.1");
        if !(provider.kind == ProviderKind::ChatGPT || has_key || is_local) {
            return LazyFetchPlan::Nothing;
        }
        self.auto_fetched.insert(id);
        LazyFetchPlan::FetchModels
    }

    pub async fn fetch_models(&mut self, id: Uuid, api_key: &str) {
        self.fetch_models_for(id, api_key).await;
    }

    pub async fn fetch_models_for(&mut self, id: Uuid, api_key: &str) {
        let Some(provider) = self.providers.iter().find(|p| p.id == id).cloned() else {
            return;
        };
        // The ChatGPT backend has no /models endpoint — the catalog is fixed.
        if provider.kind == ProviderKind::ChatGPT {
            self.apply_chatgpt_catalog(&chatgpt_catalog());
            self.model_fetch_errors.remove(&id);
            return;
        }
        if provider.kind == ProviderKind::CodexAppServer {
            // Driven by codex.rs via mark_codex_checking/apply_codex_inspection.
            return;
        }
        self.fetching_providers.insert(id);
        self.model_fetch_errors.remove(&id);

        let result = fetch_models_list(&provider.name, &provider.base_url, api_key).await;
        self.fetching_providers.remove(&id);
        match result {
            Ok((ids, mut capabilities)) => {
                self.known_models.insert(id, ids.clone());
                // Only seed a model for a provider that has no default of its
                // own — and never for one the user isn't on.
                if !self.selected_models.contains_key(&id)
                    && provider.default_model.is_empty()
                    && id == self.selected_id
                {
                    if let Some(first) = ids.first() {
                        self.selected_models.insert(id, first.clone());
                    }
                }
                // Capability metadata rides the same fetch. The Ollama probe
                // answers for endpoints whose /v1 list said nothing.
                let native = probe_ollama_capabilities(&provider.base_url, &ids).await;
                capabilities.extend(native);
                if capabilities.is_empty() {
                    self.model_capabilities.remove(&id);
                } else {
                    self.model_capabilities.insert(id, capabilities);
                }
            }
            Err(message) => {
                self.model_fetch_errors.insert(id, message);
            }
        }
        self.save();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn provider_json(kind_value: Option<&str>) -> String {
        let kind_field = match kind_value {
            Some(k) => format!(r#","kind":"{k}""#),
            None => String::new(),
        };
        format!(
            r#"{{"id":"123e4567-e89b-12d3-a456-426614174000","name":"X","baseURL":"https://x.example/v1","isPreset":false,"defaultModel":"m"{kind_field}}}"#
        )
    }

    #[test]
    fn subscription_catalog_matches_auth_source_and_has_valid_defaults() {
        use super::super::chatgpt_auth;

        let catalog = chatgpt_catalog();
        assert!(!catalog.models.is_empty());
        assert_eq!(catalog.default_model, chatgpt_auth::DEFAULT_MODEL);
        assert_eq!(catalog.models, chatgpt_auth::MODEL_CATALOG);
        assert!(catalog.models.contains(&catalog.default_model));
        for model in &catalog.models {
            let efforts = &catalog.supported_efforts[model];
            assert!(!efforts.is_empty());
            assert_eq!(efforts, chatgpt_auth::supported_reasoning_efforts(model));
            let default = &catalog.default_efforts[model];
            assert!(efforts.contains(default));
            assert_eq!(
                Some(default.as_str()),
                chatgpt_auth::default_reasoning_effort(model)
            );
        }
    }

    #[test]
    fn subscription_catalog_repairs_only_subscription_choices_and_preserves_identity() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("provider_store.json");
        let mut store = ProviderStore::load_at(path.clone(), |_| None);
        let subscription = store
            .providers
            .iter_mut()
            .find(|provider| provider.kind == ProviderKind::ChatGPT)
            .unwrap();
        let subscription_id = subscription.id;
        subscription.default_model = "gpt-5.5".into();
        let mut custom: Provider = serde_json::from_str(&provider_json(None)).unwrap();
        custom.default_model = "gpt-5.5".into();
        let custom_id = custom.id;
        store.providers.push(custom.clone());
        let api = store
            .providers
            .iter()
            .find(|provider| provider.name == "OpenAI")
            .unwrap()
            .clone();
        store.selected_id = custom_id;
        for id in [subscription_id, custom_id, api.id] {
            store.known_models.insert(id, vec!["gpt-5.5".into()]);
            store.selected_models.insert(id, "gpt-5.5".into());
            store.settings_models.insert(id, "gpt-5.4".into());
        }
        let catalog = chatgpt_catalog();
        store.apply_chatgpt_catalog(&catalog);

        assert_eq!(store.selected_id, custom_id);
        assert_eq!(store.selected_provider(), Some(&custom));
        assert_eq!(
            store
                .providers
                .iter()
                .find(|provider| provider.id == subscription_id)
                .unwrap()
                .default_model,
            catalog.default_model
        );
        assert_eq!(store.known_models[&subscription_id], catalog.models);
        assert_eq!(
            store.selected_models[&subscription_id],
            catalog.default_model
        );
        assert_eq!(
            store.settings_models[&subscription_id],
            catalog.default_model
        );
        for id in [custom_id, api.id] {
            assert_eq!(store.known_models[&id], ["gpt-5.5"]);
            assert_eq!(store.selected_models[&id], "gpt-5.5");
            assert_eq!(store.settings_models[&id], "gpt-5.4");
        }
        assert_eq!(
            store
                .providers
                .iter()
                .find(|provider| provider.id == api.id),
            Some(&api)
        );

        // Existing supported choices survive both refresh and restart.
        let supported_choice = catalog.models.last().unwrap().clone();
        store
            .selected_models
            .insert(subscription_id, supported_choice.clone());
        store
            .settings_models
            .insert(subscription_id, supported_choice.clone());
        store.try_save().unwrap();
        let reloaded = ProviderStore::load_at(path, |_| {
            panic!("catalog repair must not look up credentials")
        });
        assert_eq!(reloaded.selected_id, custom_id);
        assert_eq!(reloaded.selected_provider(), Some(&custom));
        assert_eq!(reloaded.selected_models[&subscription_id], supported_choice);
        assert_eq!(reloaded.settings_models[&subscription_id], supported_choice);
    }

    #[test]
    fn subscription_snapshot_never_erases_a_newer_manually_entered_model() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("provider_store.json");
        let mut store = ProviderStore::load_at(path.clone(), |_| None);
        let id = store
            .providers
            .iter()
            .find(|p| p.kind == ProviderKind::ChatGPT)
            .unwrap()
            .id;
        store
            .selected_models
            .insert(id, "future-account-model".into());
        store
            .settings_models
            .insert(id, "future-editor-model".into());
        store.try_save().unwrap();
        let reloaded = ProviderStore::load_at(path, |_| None);
        assert_eq!(reloaded.selected_models[&id], "future-account-model");
        assert_eq!(reloaded.settings_models[&id], "future-editor-model");
    }

    #[test]
    fn persistence_missing_file_initializes_a_writable_store() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nested/provider_store.json");
        let store = ProviderStore::load_at(path.clone(), |_| None);

        assert!(store.load_error().is_none());
        assert!(!store.providers.is_empty());
        assert!(store.selected_provider().is_some());
        assert!(store.settings_models.is_empty());
        store.try_save().unwrap();
        let state: PersistedProviderState =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(state.providers.unwrap(), store.providers);
        assert_eq!(state.selected_id, Some(store.selected_id));
    }

    #[test]
    fn persistence_custom_identity_and_all_maps_round_trip_without_changing_live_model() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("provider_store.json");
        let mut store = ProviderStore::load_at(path.clone(), |_| None);
        let provider: Provider = serde_json::from_str(&provider_json(None)).unwrap();
        let id = provider.id;
        store.providers.push(provider.clone());
        store.selected_id = id;
        store
            .known_models
            .insert(id, vec!["live-model".into(), "editor-model".into()]);
        store.selected_models.insert(id, "live-model".into());
        store.settings_models.insert(id, "editor-model".into());
        store.known_model_efforts.insert(
            id,
            HashMap::from([("live-model".into(), vec!["low".into(), "high".into()])]),
        );
        store
            .default_model_efforts
            .insert(id, HashMap::from([("live-model".into(), "low".into())]));
        store
            .selected_model_efforts
            .insert(id, HashMap::from([("live-model".into(), "high".into())]));
        store.model_capabilities.insert(
            id,
            HashMap::from([(
                "live-model".into(),
                ModelCapabilities {
                    images: Some(true),
                    files: Some(false),
                },
            )]),
        );
        store.capability_exceptions.insert(
            id,
            HashMap::from([(
                "live-model".into(),
                CapabilityException {
                    images: Some(false),
                    files: None,
                    source: ExceptionSource::Manual,
                },
            )]),
        );
        store.pdf_pass_through.insert(id, true);
        store.try_save().unwrap();
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();

        let reloaded = ProviderStore::load_at(path.clone(), |_| {
            panic!("a valid existing selection does not need credential lookup")
        });
        assert!(reloaded.load_error().is_none());
        assert_eq!(reloaded.selected_provider(), Some(&provider));
        assert_eq!(reloaded.current_model(), "live-model");
        assert_eq!(reloaded.settings_models[&id], "editor-model");
        assert_eq!(reloaded.selected_provider().unwrap().default_model, "m");
        assert_eq!(reloaded.current_reasoning_effort().as_deref(), Some("high"));
        let resaved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(resaved, saved);
    }

    #[test]
    fn persistence_legacy_store_without_settings_models_uses_remembered_fallback() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("provider_store.json");
        let provider: serde_json::Value = serde_json::from_str(&provider_json(None)).unwrap();
        let id: Uuid = serde_json::from_value(provider["id"].clone()).unwrap();
        std::fs::write(
            &path,
            serde_json::to_vec(&serde_json::json!({
                "providers": [provider],
                "selected_id": id,
                "selected_models": {id.to_string(): "remembered-model"}
            }))
            .unwrap(),
        )
        .unwrap();

        let store = ProviderStore::load_at(path, |_| None);
        assert!(store.load_error().is_none());
        assert!(store.settings_models.is_empty());
        assert_eq!(store.remembered_model(id), "remembered-model");
    }

    #[test]
    fn persistence_malformed_or_wrong_schema_is_retained_and_blocks_every_save() {
        let provider: serde_json::Value = serde_json::from_str(&provider_json(None)).unwrap();
        let mut invalid_id = provider.clone();
        invalid_id["id"] = "private-fixture-invalid-uuid".into();
        let mut missing_property = provider.clone();
        missing_property
            .as_object_mut()
            .unwrap()
            .remove("defaultModel");
        let mut invalid_documents = vec![
            b"private-fixture-malformed-json".to_vec(),
            b"[]".to_vec(),
            b"[[]]".to_vec(),
            b"null".to_vec(),
            b"{}".to_vec(),
            br#"{"providers":null}"#.to_vec(),
            br#"{"providers":{}}"#.to_vec(),
            br#"{"providers":[[]]}"#.to_vec(),
        ];
        invalid_documents.extend(
            [
                serde_json::json!({"providers": [invalid_id]}),
                serde_json::json!({"providers": [missing_property]}),
                serde_json::json!({"providers": [provider.clone()], "selected_id": "invalid"}),
                serde_json::json!({"providers": [provider], "known_models": {"invalid": []}}),
            ]
            .iter()
            .map(|value| serde_json::to_vec(value).unwrap()),
        );

        for original in invalid_documents {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("provider_store.json");
            std::fs::write(&path, &original).unwrap();
            let mut store = ProviderStore::load_at(path.clone(), |_| {
                panic!("failed loads must never look up credentials")
            });
            let error = store.load_error().expect("invalid input must be reported");
            assert!(!error.contains("private-fixture"));
            assert!(!error.contains(directory.path().to_str().unwrap()));
            assert_eq!(std::fs::read(&path).unwrap(), original);
            assert!(store.try_save().is_err());
            assert!(store
                .set_context_limit(store.selected_id, "gpt-4.1", Some(128_000))
                .is_err());
            assert_eq!(
                store.context_limit_override(store.selected_id, "gpt-4.1"),
                None
            );
            // Exercise the unchecked save path used by catalog/background work.
            store.add_custom();
            store.save();
            assert!(store.try_save().is_err());
            assert_eq!(std::fs::read(&path).unwrap(), original);

            // External repair does not silently unlock this fallback instance.
            std::fs::write(&path, br#"{"providers":[]}"#).unwrap();
            assert!(store.try_save().is_err());
            assert_eq!(std::fs::read(path).unwrap(), br#"{"providers":[]}"#);
        }
    }

    #[test]
    fn persistence_read_failure_retains_original_and_blocks_save() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("provider_store.json");
        // A directory cannot be read as a file, independently of user ACLs.
        std::fs::create_dir(&path).unwrap();
        let sentinel = path.join("keep");
        std::fs::write(&sentinel, b"private-fixture-original").unwrap();
        let mut store = ProviderStore::load_at(path.clone(), |_| {
            panic!("read failures must never look up credentials")
        });
        assert!(store.load_error().unwrap().contains("could not be read"));
        assert!(store.try_save().is_err());
        store.add_custom();
        assert!(store.try_save().is_err());
        assert!(path.is_dir());
        assert_eq!(
            std::fs::read(sentinel).unwrap(),
            b"private-fixture-original"
        );
    }

    #[test]
    fn persistence_stale_selection_is_repaired_without_losing_custom_rows() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("provider_store.json");
        let provider: Provider = serde_json::from_str(&provider_json(None)).unwrap();
        let id = provider.id;
        let stale_id = Uuid::new_v4();
        let state = PersistedProviderState {
            providers: Some(vec![provider.clone()]),
            selected_id: Some(stale_id),
            selected_models: HashMap::from([(id, "configured-model".into())]),
            ..Default::default()
        };
        std::fs::write(&path, serde_json::to_vec(&state).unwrap()).unwrap();

        let store = ProviderStore::load_at(path.clone(), |_| None);
        assert!(store.load_error().is_none());
        assert_eq!(store.selected_id, id);
        assert_eq!(store.selected_provider(), Some(&provider));
        assert_eq!(store.current_model(), "configured-model");
        let reloaded = ProviderStore::load_at(path, |_| None);
        assert_eq!(reloaded.selected_id, id);
        assert_eq!(reloaded.selected_provider(), Some(&provider));
    }

    #[test]
    fn context_limits_are_isolated_by_provider_and_model_and_survive_reload() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("provider_store.json");
        let mut store = ProviderStore::load_at(path.clone(), |_| None);
        let first = store.add_custom();
        let second = store.add_custom();
        store.selected_id = second;
        store.selected_models.insert(first, "gpt-4.1".into());
        store.selected_models.insert(second, "other-model".into());
        store
            .settings_models
            .insert(first, "settings-only-model".into());
        store.pdf_pass_through.insert(first, true);
        store.try_save().unwrap();
        let mut before: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();

        assert_eq!(store.context_limit(first, "gpt-4.1"), 1_000_000);
        assert_eq!(store.current_context_limit(), 256_000);
        store
            .set_context_limit(first, "gpt-4.1", Some(128_000))
            .unwrap();
        store
            .set_context_limit(first, "other-model", Some(64_000))
            .unwrap();
        assert_eq!(store.context_limit(first, "gpt-4.1"), 128_000);
        assert_eq!(store.context_limit(first, "other-model"), 64_000);
        assert_eq!(store.context_limit(second, "gpt-4.1"), 1_000_000);
        assert_eq!(store.current_context_limit(), 256_000);
        assert_eq!(store.selected_id, second);
        assert_eq!(store.current_model(), "other-model");
        let mut after: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        before.as_object_mut().unwrap().remove("context_limits");
        after.as_object_mut().unwrap().remove("context_limits");
        assert_eq!(
            before, after,
            "only the context-limit preference may change"
        );

        let mut reloaded = ProviderStore::load_at(path.clone(), |_| None);
        assert_eq!(reloaded.context_limit(first, "gpt-4.1"), 128_000);
        assert_eq!(reloaded.context_limit(first, "other-model"), 64_000);
        reloaded.set_context_limit(first, "gpt-4.1", None).unwrap();
        assert_eq!(reloaded.context_limit(first, "gpt-4.1"), 1_000_000);
        assert_eq!(reloaded.context_limit(first, "other-model"), 64_000);
        reloaded
            .set_context_limit(first, "other-model", None)
            .unwrap();
        assert!(!reloaded.context_limits.contains_key(&first));
        assert_eq!(
            ProviderStore::load_at(path, |_| None).context_limit(first, "other-model"),
            256_000
        );
    }

    #[test]
    fn context_limit_invalid_input_or_failed_save_preserves_preferences() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("provider_store.json");
        let mut store = ProviderStore::load_at(path.clone(), |_| None);
        let id = store.add_custom();
        store
            .set_context_limit(id, "gpt-4.1", Some(128_000))
            .unwrap();
        let before = std::fs::read(&path).unwrap();
        for (provider, model, limit) in [
            (id, "gpt-4.1", Some(0)),
            (id, "  ", Some(1000)),
            (Uuid::nil(), "gpt-4.1", Some(1000)),
        ] {
            assert!(store.set_context_limit(provider, model, limit).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), before);
            assert_eq!(store.context_limit(id, "gpt-4.1"), 128_000);
        }
        // Force an atomic-write failure using an owned directory as the file.
        store.storage_path = directory.path().to_owned();
        assert!(store
            .set_context_limit(id, "gpt-4.1", Some(512_000))
            .is_err());
        assert!(store.set_context_limit(id, "gpt-4.1", None).is_err());
        assert!(store
            .set_context_limit(id, "different-model", Some(64_000))
            .is_err());
        assert_eq!(store.context_limit(id, "gpt-4.1"), 128_000);
        assert_eq!(store.context_limit_override(id, "different-model"), None);
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn context_limits_legacy_stores_use_defaults_without_model_selection_changes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("provider_store.json");
        let provider: Provider = serde_json::from_str(&provider_json(None)).unwrap();
        let id = provider.id;
        std::fs::write(
            &path,
            serde_json::to_vec(&serde_json::json!({
                "providers": [provider], "selected_id": id,
                "selected_models": { id.to_string(): "gpt-4.1" },
                "settings_models": { id.to_string(): "different-settings-model" }
            }))
            .unwrap(),
        )
        .unwrap();
        let store = ProviderStore::load_at(path, |_| None);
        assert!(store.load_error().is_none());
        assert_eq!(store.current_context_limit(), 1_000_000);
        assert_eq!(store.current_model(), "gpt-4.1");
        assert_eq!(store.settings_models[&id], "different-settings-model");
        assert_eq!(store.context_limit_override(id, "gpt-4.1"), None);
    }

    #[test]
    fn persistence_storage_paths_remain_bound_to_their_instances() {
        let directory = tempfile::tempdir().unwrap();
        let first_path = directory.path().join("first/provider_store.json");
        let second_path = directory.path().join("second/provider_store.json");
        let mut first = ProviderStore::load_at(first_path.clone(), |_| None);
        let mut second = ProviderStore::load_at(second_path.clone(), |_| None);
        let first_id = first.add_custom();
        let second_id = second.add_custom();
        first.settings_models.insert(first_id, "first-model".into());
        second
            .settings_models
            .insert(second_id, "second-model".into());
        second.try_save().unwrap();
        let second_bytes = std::fs::read(&second_path).unwrap();
        first.try_save().unwrap();

        assert_eq!(std::fs::read(&second_path).unwrap(), second_bytes);
        let first_reloaded = ProviderStore::load_at(first_path, |_| None);
        let second_reloaded = ProviderStore::load_at(second_path, |_| None);
        assert_eq!(first_reloaded.settings_models[&first_id], "first-model");
        assert!(!first_reloaded.settings_models.contains_key(&second_id));
        assert_eq!(second_reloaded.settings_models[&second_id], "second-model");
        assert!(!second_reloaded.settings_models.contains_key(&first_id));
    }

    #[test]
    fn unknown_kind_falls_back_to_openai_compatible() {
        let provider: Provider = serde_json::from_str(&provider_json(Some("brainLinkV2"))).unwrap();
        assert_eq!(provider.kind, ProviderKind::OpenAICompatible);
        assert_eq!(provider.free_template, None);
    }

    #[test]
    fn missing_kind_decodes_as_openai_compatible() {
        let provider: Provider = serde_json::from_str(&provider_json(None)).unwrap();
        assert_eq!(provider.kind, ProviderKind::OpenAICompatible);
    }

    #[test]
    fn known_kinds_round_trip() {
        for (raw, kind) in [
            ("openAICompatible", ProviderKind::OpenAICompatible),
            ("chatGPT", ProviderKind::ChatGPT),
            ("codexAppServer", ProviderKind::CodexAppServer),
        ] {
            let provider: Provider = serde_json::from_str(&provider_json(Some(raw))).unwrap();
            assert_eq!(provider.kind, kind);
            let encoded = serde_json::to_string(&provider).unwrap();
            assert!(encoded.contains(&format!(r#""kind":"{raw}""#)));
        }
    }

    #[test]
    fn decode_model_list_reads_openrouter_modalities() {
        let data = br#"{"data":[
            {"id":"b-model"},
            {"id":"a-model","architecture":{"input_modalities":["text","image"]}},
            {"id":"c-model","architecture":{"input_modalities":["text","file"]}}
        ]}"#;
        let (ids, caps) = decode_model_list(data).unwrap();
        assert_eq!(ids, vec!["a-model", "b-model", "c-model"]);
        assert_eq!(
            caps["a-model"],
            ModelCapabilities {
                images: Some(true),
                files: Some(false)
            }
        );
        assert_eq!(
            caps["c-model"],
            ModelCapabilities {
                images: Some(false),
                files: Some(true)
            }
        );
        assert!(!caps.contains_key("b-model"));
    }

    #[tokio::test]
    async fn fetch_models_list_sends_bearer_and_decodes_models() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .and(header("authorization", "Bearer test-secret"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [
                    {"id": "z-model"},
                    {"id": "a-model", "architecture": {"input_modalities": ["text", "image"]}}
                ]
            })))
            .mount(&server)
            .await;

        let (models, capabilities) = fetch_models_list(
            "Mock provider",
            &format!("{}/v1", server.uri()),
            "test-secret",
        )
        .await
        .unwrap();

        assert_eq!(models, vec!["a-model", "z-model"]);
        assert_eq!(capabilities["a-model"].images, Some(true));
    }

    #[tokio::test]
    async fn fetch_models_list_reports_http_errors_to_settings_ui() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(ResponseTemplate::new(401).set_body_string("invalid key"))
            .mount(&server)
            .await;

        let error = fetch_models_list("Mock provider", &server.uri(), "bad-key")
            .await
            .unwrap_err();
        assert!(error.contains("HTTP 401"));
        assert!(error.contains("invalid key"));
    }

    #[test]
    fn resolve_capabilities_prefers_exception_per_field() {
        let exception = CapabilityException {
            images: Some(false),
            files: None,
            source: ExceptionSource::Learned,
        };
        let metadata = ModelCapabilities {
            images: Some(true),
            files: Some(true),
        };
        let kind_default = AttachmentCapabilities {
            images: Some(true),
            files: Some(false),
        };
        let resolved = resolve_capabilities(Some(&exception), Some(&metadata), &kind_default);
        // Exception wins for images; metadata survives for files.
        assert_eq!(resolved.images, Some(false));
        assert_eq!(resolved.files, Some(true));
        let fallback = resolve_capabilities(None, None, &kind_default);
        assert_eq!(fallback.images, Some(true));
        assert_eq!(fallback.files, Some(false));
    }

    #[test]
    fn merged_exception_keeps_manual_source() {
        let existing = CapabilityException {
            images: Some(false),
            files: None,
            source: ExceptionSource::Manual,
        };
        let merged = merged_exception(
            Some(&existing),
            ContentCapability::Files,
            false,
            ExceptionSource::Learned,
        );
        assert_eq!(merged.files, Some(false));
        assert_eq!(merged.images, Some(false));
        assert_eq!(merged.source, ExceptionSource::Manual);
    }

    #[test]
    fn free_templates_cover_expected_providers() {
        for name in [
            "Groq",
            "Google Gemini",
            "Mistral",
            "Cerebras",
            "NVIDIA NIM",
            "Cohere",
            "Hugging Face",
        ] {
            let template = free_template_named(name).expect(name);
            assert!(template.base_url.starts_with("https://"));
            assert!(template.key_url.starts_with("https://"));
        }
        assert!(free_template_named("Nope").is_none());
    }
}
