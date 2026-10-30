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
//! value types plus `apply_*` entry points so those modules can publish
//! without a dependency cycle.

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

/// Fixed catalog mirrored from `ChatGPTAuth` ( ChatGPTAuth.swift ):
/// default model, catalog, and the hand-maintained reasoning-effort table.
/// Constructed here so `ProviderStore` needs no dependency on the auth module.
#[derive(Debug, Clone)]
pub struct ChatGptCatalog {
    pub default_model: String,
    pub models: Vec<String>,
    pub supported_efforts: HashMap<String, Vec<String>>,
    pub default_efforts: HashMap<String, String>,
}

pub fn chatgpt_catalog() -> ChatGptCatalog {
    let models = vec![
        "gpt-5.5".to_string(),
        "gpt-5.4".to_string(),
        "gpt-5.4-mini".to_string(),
        "gpt-5.3-codex-spark".to_string(),
    ];
    let mut supported_efforts = HashMap::new();
    let mut default_efforts = HashMap::new();
    for model in &models {
        supported_efforts.insert(
            model.clone(),
            vec![
                "low".to_string(),
                "medium".to_string(),
                "high".to_string(),
                "xhigh".to_string(),
            ],
        );
        let fallback = if model == "gpt-5.3-codex-spark" {
            "high"
        } else {
            "medium"
        };
        default_efforts.insert(model.clone(), fallback.to_string());
    }
    ChatGptCatalog {
        default_model: "gpt-5.5".to_string(),
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
    known_model_efforts: HashMap<Uuid, HashMap<String, Vec<String>>>,
    #[serde(default)]
    default_model_efforts: HashMap<Uuid, HashMap<String, String>>,
    #[serde(default)]
    selected_model_efforts: HashMap<Uuid, HashMap<String, String>>,
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
    pub providers: Vec<Provider>,
    pub selected_id: Uuid,
    /// Last-fetched `/models` list per provider — persisted so the switcher
    /// works offline.
    pub known_models: HashMap<Uuid, Vec<String>>,
    pub selected_models: HashMap<Uuid, String>,
    pub known_model_efforts: HashMap<Uuid, HashMap<String, Vec<String>>>,
    pub default_model_efforts: HashMap<Uuid, HashMap<String, String>>,
    pub selected_model_efforts: HashMap<Uuid, HashMap<String, String>>,
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
    pub const CHATGPT_PROVIDER_NAME: &'static str = "OpenAI subscription (unofficial)";

    pub fn load(secrets: &SecretStore) -> Self {
        let state: PersistedProviderState = std::fs::read(store_file_path())
            .ok()
            .and_then(|data| serde_json::from_slice(&data).ok())
            .unwrap_or_default();
        let mut migrated = state.providers.unwrap_or_else(Self::presets);
        let selected_id = state.selected_id;

        // Retired presets: drop the untouched ones, keep anything set up.
        migrated.retain(|provider| {
            if !(provider.is_preset
                && Self::retired_preset_names().contains(&provider.name.as_str()))
            {
                return true;
            }
            let configured = !secrets
                .get(&provider.id.to_string())
                .unwrap_or_default()
                .is_empty()
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

        let mut store = ProviderStore {
            selected_id: selected_id.unwrap_or_else(|| migrated[0].id),
            providers: migrated,
            known_models: state.known_models,
            selected_models: state.selected_models,
            known_model_efforts: state.known_model_efforts,
            default_model_efforts: state.default_model_efforts,
            selected_model_efforts: state.selected_model_efforts,
            model_capabilities: state.model_capabilities,
            capability_exceptions: state.capability_exceptions,
            pdf_pass_through: state.pdf_pass_through,
            model_fetch_errors: HashMap::new(),
            fetching_providers: HashSet::new(),
            codex_app_server_status: CodexAppServerStatus::Unknown,
            auto_fetched: HashSet::new(),
            codex_generation: 0,
        };
        // Fresh install (no stored selection): prefer whatever IS configured
        // over the positional fallback.
        if selected_id.is_none() {
            let configured = store
                .providers
                .iter()
                .find(|p| {
                    let key = secrets.get(&p.id.to_string()).unwrap_or_default();
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
        store.save();
        store
    }

    pub fn save(&self) {
        if let Err(error) = self.try_save() {
            crate::logf!("[providers] could not save provider store: {error}");
        }
    }

    pub fn try_save(&self) -> anyhow::Result<()> {
        let state = PersistedProviderState {
            providers: Some(self.providers.clone()),
            selected_id: Some(self.selected_id),
            known_models: self.known_models.clone(),
            selected_models: self.selected_models.clone(),
            known_model_efforts: self.known_model_efforts.clone(),
            default_model_efforts: self.default_model_efforts.clone(),
            selected_model_efforts: self.selected_model_efforts.clone(),
            model_capabilities: self.model_capabilities.clone(),
            capability_exceptions: self.capability_exceptions.clone(),
            pdf_pass_through: self.pdf_pass_through.clone(),
        };
        let path = store_file_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let data = serde_json::to_vec_pretty(&state)?;
        crate::paths::atomic_write(&path, &data)?;
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

    /// Seeds the fixed ChatGPT catalog onto the ChatGPT provider
    /// specifically — not `selected_provider`.
    pub fn apply_chatgpt_catalog(&mut self, catalog: &ChatGptCatalog) {
        let Some(provider) = self
            .providers
            .iter()
            .find(|p| p.kind == ProviderKind::ChatGPT)
        else {
            return;
        };
        let id = provider.id;
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
        self.known_model_efforts.remove(&id);
        self.default_model_efforts.remove(&id);
        self.selected_model_efforts.remove(&id);
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
