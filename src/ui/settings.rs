//! 設定草稿與 UI。ProviderStore 的 UUID 是服務身分，SecretStore 是金鑰來源。
//!
//! 開啟時呼叫 `sync_provider`；`show` 只回報儲存意圖。呼叫端先儲存該 UUID
//! 的金鑰（包含空值）與 provider，再呼叫 `save` / `report_save_result`。
//! `settings.json` 的舊 API key 仍可讀取供遷移，但永不重新序列化。

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, TryRecvError};

use crate::chat::providers::{ModelCapabilities, Provider, ProviderKind, ProviderStore};
use crate::chat::store::{PromptShortcut, SecretStore, ShortcutStore};
use crate::ui::{fonts, theme, window_prefs};
use eframe::egui;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 附帶的四個 accent 預設（對應 Theme.accentOptions 的「四預設 + 自訂」結構）。
pub const ACCENT_PRESETS: [&str; 4] = theme::ACCENT_PRESETS;
pub const DEFAULT_ACCENT: &str = theme::DEFAULT_ACCENT_HEX;

/// Web Search 引擎選項（對應 SearchEngineChoice；完整 catalog 待補）。
pub const SEARCH_ENGINES: [&str; 4] = ["DuckDuckGo", "Brave", "Tavily", "Provider-native"];

/// 外觀（對應 AppearanceChoice Auto / Light / Dark）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Appearance {
    Light,
    Dark,
    Auto,
}

impl Default for Appearance {
    fn default() -> Self {
        Self::Auto
    }
}

/// Settings 持久化內容（讀寫 `settings.json`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppSettings {
    pub provider_name: String,
    pub base_url: String,
    /// Legacy migration input only; never write the secret back to settings.json.
    #[serde(default, skip_serializing)]
    pub api_key: String,
    pub model: String,
    pub appearance: Appearance,
    pub accent_hex: String,
    #[serde(default)]
    pub custom_accent_hex: String,
    pub search_engine: String,
    pub web_search_enabled: bool,
    #[serde(
        default = "default_system_prompt",
        deserialize_with = "deserialize_system_prompt"
    )]
    pub system_prompt: String,
    #[serde(default, skip_serializing)]
    pub search_engine_key: String,
    #[serde(default)]
    pub codex_executable_path: String,
    #[serde(default)]
    pub pdf_pass_through: bool,
    #[serde(default = "default_true")]
    pub automatic_update_checks: bool,
    #[serde(default)]
    pub streaming_mode: crate::chat::store::StreamingMode,
    #[serde(default)]
    pub bubble_style: theme::BubbleStyle,
    #[serde(default)]
    pub font_preferences: fonts::FontPreferences,
    #[serde(
        default = "default_chat_font_size",
        deserialize_with = "deserialize_chat_font_size"
    )]
    pub chat_font_size: u16,
    #[serde(default)]
    pub window_preferences: window_prefs::WindowPreferences,
    #[serde(default = "default_hotkey_binding")]
    pub hotkey_binding: String,
}

fn default_hotkey_binding() -> String {
    "Alt+Down".to_owned()
}

fn default_chat_font_size() -> u16 {
    crate::ui::typography::DEFAULT_SIZE
}

fn deserialize_chat_font_size<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<u16, D::Error> {
    u16::deserialize(deserializer).map(crate::ui::typography::normalize)
}

fn default_true() -> bool {
    true
}

fn default_system_prompt() -> String {
    crate::chat::store::ChatStore::DEFAULT_SYSTEM_PROMPT.to_owned()
}

fn deserialize_system_prompt<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<String, D::Error> {
    let text = String::deserialize(deserializer)?;
    Ok(crate::chat::commands::localized_builtin_system_prompt(&text).to_string())
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            provider_name: "OpenAI".to_owned(),
            base_url: "https://api.openai.com/v1".to_owned(),
            api_key: String::new(),
            model: String::new(),
            appearance: Appearance::Auto,
            accent_hex: DEFAULT_ACCENT.to_owned(),
            custom_accent_hex: String::new(),
            search_engine: SEARCH_ENGINES[0].to_owned(),
            web_search_enabled: true,
            system_prompt: default_system_prompt(),
            search_engine_key: String::new(),
            codex_executable_path: String::new(),
            pdf_pass_through: false,
            automatic_update_checks: true,
            streaming_mode: crate::chat::store::StreamingMode::default(),
            bubble_style: theme::BubbleStyle::default(),
            font_preferences: fonts::FontPreferences::default(),
            chat_font_size: default_chat_font_size(),
            window_preferences: window_prefs::WindowPreferences::default(),
            hotkey_binding: default_hotkey_binding(),
        }
    }
}

// This snapshot is deliberately neither Debug nor serializable: it includes a key.
#[derive(Clone, PartialEq, Eq)]
struct ConnectionConfig {
    provider_id: Uuid,
    kind: ProviderKind,
    base_url: String,
    api_key: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct ModelScope {
    provider_id: Uuid,
    generation: u64,
}

struct ModelList {
    scope: ModelScope,
    models: Vec<String>,
    fetched: bool,
    capabilities: HashMap<String, ModelCapabilities>,
}

#[derive(Default)]
struct ModelCatalog {
    models: Vec<String>,
    capabilities: HashMap<String, ModelCapabilities>,
}

impl From<Vec<String>> for ModelCatalog {
    fn from(models: Vec<String>) -> Self {
        Self {
            models,
            ..Self::default()
        }
    }
}

struct ModelFetch {
    scope: ModelScope,
    rx: Receiver<Result<ModelCatalog, String>>,
    task: Option<tokio::task::AbortHandle>,
}

impl Drop for ModelFetch {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum SettingsTab {
    #[default]
    Service,
    Appearance,
    Search,
    Commands,
    Hotkey,
    Mcp,
}

impl SettingsTab {
    const ALL: [(Self, &'static str); 6] = [
        (Self::Service, "服務"),
        (Self::Appearance, "外觀"),
        (Self::Search, "搜尋"),
        (Self::Commands, "指令"),
        (Self::Hotkey, "快捷鍵"),
        (Self::Mcp, "MCP"),
    ];
}

#[derive(Debug, Clone)]
pub enum SettingsAction {
    ManageMcp,
    /// Register with the OS first, then call `commit_hotkey_binding` on success.
    ChangeHotkey(String),
    InstallUpdate(crate::updater::PreparedUpdate),
    SignInChatGpt,
    CancelChatGptSignIn,
    SignOutChatGpt,
    InspectCodex,
    CodexPathChanged,
    AddProvider,
    AddProviderTemplate(String),
    CommandsChanged(Vec<PromptShortcut>),
    RemoveProvider(Uuid),
    SelectModel {
        provider_id: Uuid,
        model: String,
    },
    SetEffort {
        provider_id: Uuid,
        model: String,
        effort: String,
    },
    SetContextLimit {
        provider_id: Uuid,
        model: String,
        limit: Option<u32>,
    },
    ClearCapabilityException {
        provider_id: Uuid,
        model: String,
    },
}

#[derive(Default)]
struct ChatGptState {
    signed_in: bool,
    email: Option<String>,
    plan: Option<String>,
    signing_in: bool,
    error: Option<String>,
}

#[derive(Default)]
struct ContextLimitDraft {
    text: String,
    result: Option<Result<(), String>>,
}

/// 設定視窗的草稿；服務切換只載入該 UUID 的設定，不修改 stores。
pub struct SettingsUi {
    pub settings: AppSettings,
    provider: Option<Provider>,
    connection: Option<ConnectionConfig>,
    generation: u64,
    model_list: Option<ModelList>,
    fetch: Option<ModelFetch>,
    fetch_status: String,
    fetch_failed: bool,
    credential_cleared: bool,
    credential_generation: u64,
    reveal_key: bool,
    saved_settings: AppSettings,
    saved_provider_id: Option<Uuid>,
    save_result: Option<Result<(), String>>,
    hotkey_label: Option<String>,
    hotkey_draft: String,
    hotkey_update_error: Option<String>,
    current_window_position: Option<egui::Pos2>,
    active_tab: SettingsTab,
    pane_transition: crate::ui::transition::PageTransition,
    actions: Vec<SettingsAction>,
    chatgpt: ChatGptState,
    codex_checking: bool,
    codex_message: String,
    command_shortcuts: Vec<PromptShortcut>,
    commands_dirty: bool,
    search_key_engine: String,
    pending_delete: Option<Uuid>,
    updates: crate::update::UpdateChecker,
    update_download: crate::update::UpdateDownload,
    update_install_message: Option<String>,
    update_install_busy: bool,
    startup_enabled: bool,
    startup_error: Option<String>,
    command_save_error: Option<String>,
    command_focus: Option<Uuid>,
    codex_picker: Option<Receiver<Option<std::path::PathBuf>>>,
    preference_save_error: Option<String>,
    search_key_dirty: bool,
    accent_picker: crate::ui::accent::AccentPicker,
    context_limit_drafts: HashMap<(Uuid, String), ContextLimitDraft>,
}

impl SettingsUi {
    fn config_path() -> Option<std::path::PathBuf> {
        crate::paths::config_dir().map(|dir| dir.join("settings.json"))
    }

    pub fn load() -> Self {
        let mut settings = Self::config_path()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice::<AppSettings>(&b).ok())
            .unwrap_or_default();
        settings.search_engine_key = SecretStore::new()
            .get(&search_secret_key(&settings.search_engine))
            .unwrap_or_default();
        let mut ui = Self::from_settings(settings);
        ui.command_shortcuts = ShortcutStore::load().shortcuts;
        match crate::startup::enabled() {
            Ok(enabled) => ui.startup_enabled = enabled,
            Err(error) => ui.startup_error = Some(format!("讀取登入啟動狀態失敗：{error:#}")),
        }
        ui
    }

    fn from_settings(mut settings: AppSettings) -> Self {
        // The shared theme parser slices ASCII hex pairs; validate before use,
        // including before main applies the accent loaded from an older file.
        settings.accent_hex =
            normalized_accent(&settings.accent_hex).unwrap_or_else(|| DEFAULT_ACCENT.to_owned());
        Self {
            search_key_engine: settings.search_engine.clone(),
            hotkey_draft: settings.hotkey_binding.clone(),
            saved_settings: settings.clone(),
            settings,
            provider: None,
            connection: None,
            generation: 0,
            model_list: None,
            fetch: None,
            fetch_status: String::new(),
            fetch_failed: false,
            credential_cleared: false,
            credential_generation: 0,
            reveal_key: false,
            saved_provider_id: None,
            save_result: None,
            hotkey_label: None,
            hotkey_update_error: None,
            current_window_position: None,
            active_tab: SettingsTab::default(),
            pane_transition: crate::ui::transition::PageTransition::default(),
            actions: Vec::new(),
            chatgpt: ChatGptState::default(),
            codex_checking: false,
            codex_message: String::new(),
            command_shortcuts: Vec::new(),
            commands_dirty: false,
            pending_delete: None,
            updates: crate::update::UpdateChecker::default(),
            update_download: crate::update::UpdateDownload::default(),
            update_install_message: None,
            update_install_busy: false,
            startup_enabled: false,
            startup_error: None,
            command_save_error: None,
            command_focus: None,
            codex_picker: None,
            preference_save_error: None,
            search_key_dirty: false,
            accent_picker: crate::ui::accent::AccentPicker::default(),
            context_limit_drafts: HashMap::new(),
        }
    }

    /// 只寫入一般設定。呼叫端必須先成功儲存金鑰與 provider，再回報整體結果。
    pub fn save(&self) -> Result<(), String> {
        let mut secrets = SecretStore::new();
        let key_id = search_secret_key(&self.settings.search_engine);
        if self.settings.search_engine_key.trim().is_empty() {
            secrets
                .delete(&key_id)
                .map_err(|e| format!("無法清除搜尋金鑰：{e:#}"))?;
        } else {
            secrets
                .set(self.settings.search_engine_key.trim(), &key_id)
                .map_err(|e| format!("無法儲存搜尋金鑰：{e:#}"))?;
        }
        if self.commands_dirty {
            ShortcutStore::save_entries(&self.command_shortcuts)
                .map_err(|e| format!("無法儲存指令：{e:#}"))?;
        }
        let path = Self::config_path().ok_or_else(|| "找不到設定目錄".to_owned())?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("無法建立設定目錄：{e}"))?;
        }
        let bytes = serde_json::to_vec_pretty(&self.settings).map_err(|e| e.to_string())?;
        crate::paths::atomic_write(&path, &bytes).map_err(|e| format!("無法寫入設定：{e}"))
    }

    pub fn report_save_result(&mut self, result: Result<(), String>) {
        if result.is_ok() {
            self.saved_settings = self.settings.clone();
            self.saved_provider_id = self.selected_provider_id();
            self.credential_cleared = false;
            self.commands_dirty = false;
            self.command_save_error = None;
            self.preference_save_error = None;
            self.search_key_dirty = false;
        }
        self.save_result = Some(result);
    }

    /// 儲存時用此 UUID 查找既有 provider，保留其 kind、名稱與其他 metadata。
    pub fn selected_provider_id(&self) -> Option<Uuid> {
        self.provider.as_ref().map(|provider| provider.id)
    }

    /// 僅回傳目前連線版本成功取得的模型；呼叫端可在儲存成功時按回傳 UUID 快取。
    /// 請在更新 provider 之後使用；不要依 ProviderStore 的舊 selected_id 寫入。
    pub fn fetched_models(&self) -> Option<(Uuid, &[String])> {
        let list = self.model_list.as_ref()?;
        (list.fetched
            && Some(list.scope) == self.model_scope()
            && self.connection == self.connection_config())
        .then_some((list.scope.provider_id, list.models.as_slice()))
    }

    pub fn fetched_capabilities(&self) -> Option<(Uuid, HashMap<String, ModelCapabilities>)> {
        let (id, _) = self.fetched_models()?;
        Some((id, self.model_list.as_ref()?.capabilities.clone()))
    }

    pub fn command_shortcuts(&self) -> &[PromptShortcut] {
        &self.command_shortcuts
    }

    pub fn take_actions(&mut self) -> Vec<SettingsAction> {
        std::mem::take(&mut self.actions)
    }

    pub fn hotkey_recorder_visible(&self) -> bool {
        self.active_tab == SettingsTab::Hotkey
    }

    pub fn set_update_install_message(&mut self, message: Option<String>, busy: bool) {
        self.update_install_message = message;
        self.update_install_busy = busy;
    }

    pub fn set_chatgpt_state(
        &mut self,
        signed_in: bool,
        email: Option<String>,
        plan: Option<String>,
        signing_in: bool,
        error: Option<String>,
    ) {
        self.chatgpt = ChatGptState {
            signed_in,
            email,
            plan,
            signing_in,
            error,
        };
    }

    pub fn set_codex_state(&mut self, checking: bool, message: String) {
        self.codex_checking = checking;
        self.codex_message = message;
    }

    pub fn search_key_for_send(&self) -> Option<String> {
        (!self.settings.search_engine_key.trim().is_empty())
            .then(|| self.settings.search_engine_key.trim().to_owned())
    }

    pub fn poll_services(&mut self, ctx: &egui::Context, runtime: &tokio::runtime::Handle) {
        self.updates
            .poll(ctx, runtime, self.settings.automatic_update_checks);
        self.update_download.poll(ctx);
        let chosen = match self.codex_picker.as_ref().map(|rx| rx.try_recv()) {
            Some(Ok(path)) => Some(path),
            Some(Err(TryRecvError::Disconnected)) => Some(None),
            _ => None,
        };
        if let Some(chosen) = chosen {
            self.codex_picker = None;
            if let Some(path) = chosen {
                if let Some(path) = path.to_str() {
                    self.settings.codex_executable_path = path.to_owned();
                    self.actions.push(SettingsAction::CodexPathChanged);
                } else {
                    self.codex_message = "執行檔路徑不是有效 Unicode，請選擇其他位置。".to_owned();
                }
            }
        }
        if self.codex_picker.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    pub fn has_native_picker(&self) -> bool {
        self.codex_picker.is_some()
    }

    pub fn persist_codex_path(&self) -> Result<(), String> {
        let path = Self::config_path().ok_or_else(|| "找不到設定目錄".to_owned())?;
        save_string_preference_only(
            &path,
            "codex_executable_path",
            &self.settings.codex_executable_path,
        )
    }

    /// 在初始化／每次開啟時載入實際選取服務；先完成 legacy key 遷移再呼叫。
    /// 此操作捨棄服務草稿，取消先前模型請求；不要在每幀呼叫。
    pub fn sync_provider(&mut self, providers: &ProviderStore, secrets: &SecretStore) {
        if let Some(provider) = providers
            .selected_provider()
            .or_else(|| providers.providers.first())
        {
            self.select_provider(provider.id, providers, secrets);
        } else {
            self.invalidate_models();
            self.provider = None;
            self.connection = None;
            self.settings.provider_name.clear();
            self.settings.base_url.clear();
            self.settings.api_key.clear();
            self.settings.model.clear();
            self.credential_cleared = false;
            self.reveal_key = false;
        }
        // Reopening reloads provider data, but appearance/search may still be
        // unsaved previews. Keep their last successful save as the baseline.
        self.saved_settings.provider_name = self.settings.provider_name.clone();
        self.saved_settings.base_url = self.settings.base_url.clone();
        self.saved_settings.api_key = self.settings.api_key.clone();
        self.saved_settings.model = self.settings.model.clone();
        self.saved_provider_id = providers.selected_provider().map(|provider| provider.id);
        self.save_result = None;
    }

    /// 傳入 HotkeyManager::label()；None 表示尚未成功註冊，不顯示假定的預設鍵。
    pub fn set_hotkey_label(&mut self, label: Option<&str>) {
        self.hotkey_label = label.map(str::to_owned);
    }

    /// Report the actual registered binding and any registration/persistence error.
    /// Unlike the per-frame label setter, a successful result also normalizes the
    /// editor. Errors leave the user's input intact for correction or retry.
    pub fn set_hotkey_result(&mut self, binding: Option<&str>, error: Option<String>) {
        self.set_hotkey_label(binding);
        if error.is_none() {
            if let Some(binding) = binding {
                self.hotkey_draft = binding.to_owned();
            }
        }
        self.hotkey_update_error = error;
    }

    /// Call only after HotkeyManager has successfully registered this binding.
    /// Keep runtime settings accurate even if writing the preference fails;
    /// only a successful write advances the saved-settings snapshot.
    pub fn commit_hotkey_binding(&mut self, binding: &str) -> Result<(), String> {
        self.settings.hotkey_binding = binding.to_owned();
        let path = Self::config_path().ok_or_else(|| "找不到設定目錄".to_owned())?;
        self.persist_hotkey_binding_at(&path, binding)
    }

    fn persist_hotkey_binding_at(
        &mut self,
        path: &std::path::Path,
        binding: &str,
    ) -> Result<(), String> {
        self.settings.hotkey_binding = binding.to_owned();
        save_string_preference_only(path, "hotkey_binding", binding)?;
        self.saved_settings.hotkey_binding = binding.to_owned();
        Ok(())
    }

    /// Update each frame with the current viewport position (egui points).
    pub fn set_current_window_position(&mut self, pos: egui::Pos2) {
        self.current_window_position = Some(pos);
    }

    pub fn select_provider(&mut self, id: Uuid, providers: &ProviderStore, secrets: &SecretStore) {
        let Some(provider) = providers
            .providers
            .iter()
            .find(|provider| provider.id == id)
        else {
            return;
        };
        let key = if provider.kind == ProviderKind::OpenAICompatible {
            providers.key_for(id, secrets)
        } else {
            String::new()
        };
        self.load_provider(
            provider,
            key,
            providers
                .settings_models
                .get(&id)
                .cloned()
                .unwrap_or_else(|| providers.remembered_model(id)),
            providers.known_models.get(&id).cloned().unwrap_or_default(),
        );
        self.settings.pdf_pass_through = providers
            .pdf_pass_through
            .get(&id)
            .copied()
            .unwrap_or(false);
        if let Some(list) = &mut self.model_list {
            list.capabilities = providers
                .model_capabilities
                .get(&id)
                .cloned()
                .unwrap_or_default();
        }
    }

    // A value-only boundary keeps provider/credential switching testable without
    // loading user configuration or accessing the OS credential store.
    fn load_provider(
        &mut self,
        provider: &Provider,
        key: String,
        model: String,
        models: Vec<String>,
    ) {
        self.invalidate_models();
        self.provider = Some(provider.clone());
        self.settings.provider_name = provider.name.clone();
        self.settings.base_url = provider.base_url.clone();
        self.settings.api_key = if provider.kind == ProviderKind::OpenAICompatible {
            key
        } else {
            String::new()
        };
        self.settings.model = model;
        self.connection = self.connection_config();
        self.model_list = self.model_scope().map(|scope| ModelList {
            scope,
            models,
            fetched: false,
            capabilities: HashMap::new(),
        });
        self.credential_cleared = false;
        self.credential_generation = self.credential_generation.wrapping_add(1);
        self.reveal_key = false;
        self.save_result = None;
    }

    fn connection_config(&self) -> Option<ConnectionConfig> {
        let provider = self.provider.as_ref()?;
        Some(ConnectionConfig {
            provider_id: provider.id,
            kind: provider.kind,
            base_url: self.settings.base_url.clone(),
            api_key: self.settings.api_key.clone(),
        })
    }

    fn model_scope(&self) -> Option<ModelScope> {
        self.selected_provider_id().map(|provider_id| ModelScope {
            provider_id,
            generation: self.generation,
        })
    }

    fn supports_api(&self) -> bool {
        self.provider.as_ref().map(|provider| provider.kind) == Some(ProviderKind::OpenAICompatible)
    }

    fn invalidate_models(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.fetch = None; // Drops the receiver and aborts the HTTP task.
        self.model_list = None;
        self.fetch_status.clear();
        self.fetch_failed = false;
    }

    fn reconcile_connection(&mut self) {
        if self.connection == self.connection_config() {
            return;
        }
        if let Some(previous) = &self.connection {
            if previous.base_url != self.settings.base_url {
                let old_origin = endpoint_origin(&previous.base_url);
                let new_origin = endpoint_origin(&self.settings.base_url);
                // Invalid intermediate edits must not keep a credential that
                // could later accompany a different origin. Slash/path edits
                // on the same scheme/host/port intentionally retain the key.
                if old_origin != new_origin || new_origin.is_none() {
                    self.credential_cleared |= !self.settings.api_key.is_empty();
                    self.settings.api_key.clear();
                    self.credential_generation = self.credential_generation.wrapping_add(1);
                    self.reveal_key = false;
                }
            }
        }
        self.invalidate_models();
        self.connection = self.connection_config();
        self.save_result = None;
    }

    /// 每幀套用外觀（對應 AppearanceChoice.applyCurrent；Auto 跟系統）。
    /// 備援入口也走共用 theme，避免回退至 egui 的純白淺色表面。
    #[allow(dead_code)]
    pub fn apply_visuals(&self, ctx: &egui::Context) {
        let appearance = match self.settings.appearance {
            Appearance::Light => "Light",
            Appearance::Dark => "Dark",
            Appearance::Auto => "Auto",
        };
        theme::apply(ctx, appearance, &self.settings.accent_hex);
    }

    /// 在有界內容區繪製可捲動區段與固定儲存列。true 只代表使用者要求儲存。
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        runtime: &tokio::runtime::Handle,
        providers: &ProviderStore,
        secrets: &SecretStore,
    ) -> bool {
        self.poll_services(ui.ctx(), runtime);
        // Also heal a removed provider or a backend-kind change while open.
        let stored = self.selected_provider_id().and_then(|id| {
            providers
                .providers
                .iter()
                .find(|provider| provider.id == id)
        });
        match (self.provider.as_ref(), stored) {
            (Some(draft), Some(stored)) if draft.kind != stored.kind => {
                self.select_provider(stored.id, providers, secrets);
            }
            (None, _) | (_, None) => self.sync_provider(providers, secrets),
            _ => {}
        }
        self.reconcile_connection();
        let before = self.settings.clone();
        let before_id = self.selected_provider_id();
        if self.active_tab != SettingsTab::Service {
            self.poll_model_fetch();
        }

        ui.scope(|ui| {
            settings_style(ui);
            let palette = theme::current_palette(ui.ctx());
            ui.label(egui::RichText::new("設定").size(26.0).strong());
            muted(ui, "讓 PopChat 更適合你的工作方式");
            if let Some(path) = crate::paths::config_dir() {
                let label = if crate::paths::isolated_profile().is_some() {
                    "自訂資料目錄"
                } else {
                    "預設資料目錄"
                };
                let text = format!("{label}：{}", path.display());
                ui.add(egui::Label::new(egui::RichText::new(&text).small().weak()).truncate())
                    .on_hover_text(text);
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                for (tab, title) in SettingsTab::ALL {
                    let selected = self.active_tab == tab;
                    let response = ui.add_sized(
                        [80.0, 34.0],
                        egui::Button::new(egui::RichText::new(title).color(if selected {
                            palette.accent
                        } else {
                            secondary(palette.dark)
                        }))
                        .fill(if selected {
                            palette.accent.gamma_multiply(0.10)
                        } else {
                            egui::Color32::TRANSPARENT
                        })
                        .stroke(egui::Stroke::NONE)
                        .corner_radius(6),
                    );
                    if response.clicked() {
                        self.active_tab = tab;
                    }
                }
            });
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);

            // Reserve a finite body and footer. Short panes never show an idle
            // scrollbar; long model lists/errors scroll without moving Save.
            let body_height = (ui.available_height()
                - 58.0
                - if self.preference_save_error.is_some() {
                    80.0
                } else {
                    0.0
                })
            .max(0.0);
            let pane_effect = self.pane_transition.update(
                self.active_tab as u64,
                ui.input(|input| input.time),
                crate::ui::transition::motion_enabled(ui.ctx()),
            );
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), body_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    crate::ui::transition::show(ui, pane_effect, |ui| {
                        egui::ScrollArea::vertical()
                            .id_salt(("settings_pane", self.active_tab as u8))
                            .auto_shrink([false, false])
                            .max_height(body_height)
                            .min_scrolled_height(0.0)
                            .scroll_bar_visibility(
                                egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded,
                            )
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                match self.active_tab {
                                    SettingsTab::Service => {
                                        self.providers_section(ui, runtime, providers, secrets);
                                    }
                                    SettingsTab::Appearance => self.appearance_section(ui),
                                    SettingsTab::Search => self.search_section(ui),
                                    SettingsTab::Commands => self.commands_section(ui),
                                    SettingsTab::Hotkey => self.hotkey_section(ui, runtime),
                                    SettingsTab::Mcp => self.mcp_section(ui),
                                }
                            });
                    });
                },
            );

            if before != self.settings || before_id != self.selected_provider_id() {
                self.save_result = None;
            }
            let retry_preferences =
                self.preference_save_error.is_some() && ui.button("重試儲存偏好設定").clicked();
            if general_preferences(&self.settings) != general_preferences(&before)
                || (self.search_key_dirty
                    && self.settings.search_engine_key != before.search_engine_key)
                || retry_preferences
            {
                self.persist_general_preferences();
            }
            if let Some(error) = &self.preference_save_error {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);
            let mut save_requested = false;
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    save_requested = ui
                        .add_enabled(
                            self.provider.is_some() && providers.load_error().is_none(),
                            egui::Button::new(
                                egui::RichText::new("儲存設定").color(palette.accent_text),
                            )
                            .fill(palette.accent)
                            .min_size(egui::vec2(108.0, 34.0)),
                        )
                        .clicked();
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        match &self.save_result {
                            Some(Ok(())) => muted(ui, "已儲存 · 所有設定已更新"),
                            Some(Err(error)) => {
                                let message = format!("儲存失敗：{error}");
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(&message)
                                            .size(12.0)
                                            .color(ui.visuals().error_fg_color),
                                    )
                                    .truncate(),
                                )
                                .on_hover_text(message);
                            }
                            None => {
                                let dirty = self.settings != self.saved_settings
                                    || self.selected_provider_id() != self.saved_provider_id;
                                muted(
                                    ui,
                                    if dirty {
                                        "有尚未儲存的變更"
                                    } else {
                                        "所有變更已儲存"
                                    },
                                );
                            }
                        }
                    });
                });
            });
            save_requested
        })
        .inner
    }

    fn poll_model_fetch(&mut self) {
        self.reconcile_connection();
        let result = match self.fetch.as_ref().map(|fetch| fetch.rx.try_recv()) {
            Some(Ok(result)) => Some(result),
            Some(Err(TryRecvError::Disconnected)) => {
                Some(Err("模型請求已中斷，請重試。".to_owned()))
            }
            Some(Err(TryRecvError::Empty)) | None => None,
        };
        let Some(result) = result else {
            return;
        };
        if let Some(fetch) = self.fetch.take() {
            self.publish_model_result(fetch.scope, result);
        }
    }

    fn persist_general_preferences(&mut self) {
        let result = (|| {
            if self.search_key_dirty {
                let mut secrets = SecretStore::new();
                let account = search_secret_key(&self.settings.search_engine);
                if self.settings.search_engine_key.trim().is_empty() {
                    secrets
                        .delete(&account)
                        .map_err(|e| format!("無法清除搜尋金鑰：{e:#}"))?;
                } else {
                    secrets
                        .set(self.settings.search_engine_key.trim(), &account)
                        .map_err(|e| format!("無法儲存搜尋金鑰：{e:#}"))?;
                }
            }
            let path = Self::config_path().ok_or_else(|| "找不到設定目錄".to_owned())?;
            save_preference_values_only(&path, general_preferences(&self.settings))?;
            Ok::<(), String>(())
        })();
        match result {
            Ok(()) => {
                self.saved_settings.appearance = self.settings.appearance;
                self.saved_settings.accent_hex = self.settings.accent_hex.clone();
                self.saved_settings.custom_accent_hex = self.settings.custom_accent_hex.clone();
                self.saved_settings.search_engine = self.settings.search_engine.clone();
                self.saved_settings.search_engine_key = self.settings.search_engine_key.clone();
                self.saved_settings.web_search_enabled = self.settings.web_search_enabled;
                self.saved_settings.streaming_mode = self.settings.streaming_mode;
                self.saved_settings.bubble_style = self.settings.bubble_style;
                self.saved_settings.automatic_update_checks = self.settings.automatic_update_checks;
                self.saved_settings.font_preferences = self.settings.font_preferences.clone();
                self.saved_settings.chat_font_size = self.settings.chat_font_size;
                self.saved_settings.window_preferences = self.settings.window_preferences.clone();
                self.search_key_dirty = false;
                self.preference_save_error = None;
            }
            Err(error) => self.preference_save_error = Some(error),
        }
    }

    fn publish_model_result(&mut self, scope: ModelScope, result: Result<ModelCatalog, String>) {
        if Some(scope) != self.model_scope() || self.connection != self.connection_config() {
            return;
        }
        match result {
            Ok(catalog) => {
                self.fetch_status =
                    format!("已取得 {} 個模型，可從下方清單選擇。", catalog.models.len());
                self.fetch_failed = false;
                self.model_list = Some(ModelList {
                    scope,
                    models: catalog.models,
                    capabilities: catalog.capabilities,
                    fetched: true,
                });
            }
            Err(error) => {
                self.fetch_status = format!("取得模型失敗：{error}");
                self.fetch_failed = true;
            }
        }
    }

    fn start_model_fetch(&mut self, runtime: &tokio::runtime::Handle, ctx: egui::Context) {
        self.reconcile_connection();
        if self.fetch.is_some()
            || !self.supports_api()
            || endpoint_origin(&self.settings.base_url).is_none()
        {
            return;
        }
        let Some(scope) = self.model_scope() else {
            return;
        };
        let provider_name = self.settings.provider_name.clone();
        let base_url = self.settings.base_url.trim().to_owned();
        let api_key = self.settings.api_key.trim().to_owned();
        let (tx, rx) = mpsc::channel();
        let task = runtime.spawn(async move {
            let result = fetch_catalog(&provider_name, &base_url, &api_key).await;
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        self.fetch = Some(ModelFetch {
            scope,
            rx,
            task: Some(task.abort_handle()),
        });
        self.fetch_status = "正在取得模型…".to_owned();
        self.fetch_failed = false;
    }

    fn providers_section(
        &mut self,
        ui: &mut egui::Ui,
        runtime: &tokio::runtime::Handle,
        providers: &ProviderStore,
        secrets: &SecretStore,
    ) {
        if providers.providers.is_empty() {
            ui.label("尚未建立模型服務。");
        }
        if let Some(id) = self.provider_picker(ui, &providers.providers) {
            self.select_provider(id, providers, secrets);
        }
        ui.horizontal(|ui| {
            if ui.button("新增自訂服務").clicked() {
                self.actions.push(SettingsAction::AddProvider);
            }
            ui.menu_button("從免費方案範本新增", |ui| {
                for template in crate::chat::providers::FREE_PROVIDER_TEMPLATES {
                    if ui.button(template.name).clicked() {
                        self.actions.push(SettingsAction::AddProviderTemplate(
                            template.name.to_owned(),
                        ));
                        ui.close();
                    }
                }
                ui.separator();
                ui.hyperlink_to(
                    "完整社群目錄",
                    crate::chat::providers::FREE_PROVIDER_DIRECTORY_URL,
                );
            });
            if self.provider.as_ref().is_some_and(|p| !p.is_preset)
                && ui.button("刪除此服務…").clicked()
            {
                self.pending_delete = self.selected_provider_id();
            }
        });
        if let Some(id) = self.pending_delete {
            ui.horizontal(|ui| {
                ui.label("確定刪除此服務及儲存的金鑰？");
                if ui.button("確認刪除").clicked() {
                    self.actions.push(SettingsAction::RemoveProvider(id));
                    self.pending_delete = None;
                }
                if ui.button("取消").clicked() {
                    self.pending_delete = None;
                }
            });
        }
        if self.provider.as_ref().is_some_and(|p| !p.is_preset) {
            field_label(ui, "服務名稱");
            ui.add(
                egui::TextEdit::singleline(&mut self.settings.provider_name)
                    .desired_width(f32::INFINITY),
            );
        }
        if let Some(template) = self
            .provider
            .as_ref()
            .and_then(|p| p.free_template.as_deref())
            .and_then(crate::chat::providers::free_template_named)
        {
            info_card(
                ui,
                &format!("{} · 免費方案範本", template.name),
                template
                    .note
                    .unwrap_or("免費額度與限制依服務供應商當下公告為準。"),
            );
            ui.hyperlink_to("取得 API 金鑰", template.key_url);
        }
        self.connection_fields(ui, runtime);
        if let Some(id) = self.selected_provider_id() {
            let model = self.settings.model.trim().to_owned();
            self.context_limit_editor(
                ui,
                id,
                &model,
                providers.context_limit_override(id, &model),
                providers.load_error().is_none(),
            );
        }
        if self.supports_api() {
            if self.provider.as_ref().is_some_and(|p| !p.is_preset) {
                ui.checkbox(
                    &mut self.settings.pdf_pass_through,
                    "將 PDF 原檔傳給此端點（需模型支援）",
                );
            }
            if let Some(id) = self.selected_provider_id() {
                if providers
                    .capability_exceptions
                    .get(&id)
                    .is_some_and(|m| m.contains_key(&self.settings.model))
                    && ui.button("清除此模型已記錄的附件限制").clicked()
                {
                    self.actions.push(SettingsAction::ClearCapabilityException {
                        provider_id: id,
                        model: self.settings.model.clone(),
                    });
                }
            }
        }
    }

    fn provider_picker(&self, ui: &mut egui::Ui, providers: &[Provider]) -> Option<Uuid> {
        field_label(ui, "服務供應商");
        let mut selected = self.selected_provider_id();
        egui::ComboBox::from_id_salt("settings_provider")
            .selected_text(&self.settings.provider_name)
            .width(ui.available_width())
            .height(220.0)
            .truncate()
            .show_ui(ui, |ui| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                for provider in providers {
                    ui.selectable_value(&mut selected, Some(provider.id), &provider.name)
                        .on_hover_text(format!("{}\n{}", provider.base_url, provider.id));
                }
            });
        muted(ui, "切換前請先儲存變更，每個服務的連線與金鑰皆獨立保存。");
        ui.add_space(8.0);
        selected.filter(|id| Some(*id) != self.selected_provider_id())
    }

    /// Independent per-model preference editor. Rendering/typing never saves
    /// the provider form, changes the live model or touches credentials.
    fn context_limit_editor(
        &mut self,
        ui: &mut egui::Ui,
        provider_id: Uuid,
        model: &str,
        stored: Option<u32>,
        writable: bool,
    ) {
        use crate::chat::context::{
            context_limit_label, default_context_limit, parse_context_limit,
        };

        ui.add_space(8.0);
        field_label(ui, "模型上下文上限（tokens）");
        if model.is_empty() {
            muted(ui, "先選擇或輸入模型 ID，再設定此模型的上限。");
            return;
        }
        let default = default_context_limit(model);
        let effective = stored.unwrap_or(default);
        ui.add(
            egui::Label::new(format!(
                "{model} · 目前生效 {} tokens（{}）",
                context_limit_label(effective),
                if stored.is_some() { "自訂" } else { "預設" },
            ))
            .wrap(),
        );
        let draft = self
            .context_limit_drafts
            .entry((provider_id, model.to_owned()))
            .or_insert_with(|| ContextLimitDraft {
                text: stored.map(|value| value.to_string()).unwrap_or_default(),
                result: None,
            });
        let mut apply = None;
        ui.push_id(("context_limit", provider_id, model), |ui| {
            ui.add_enabled_ui(writable, |ui| {
                if ui
                    .add_sized(
                        [ui.available_width(), 32.0],
                        egui::TextEdit::singleline(&mut draft.text)
                            .id_salt("value")
                            .hint_text(format!(
                                "留空用預設 {}；例如 128K、1M",
                                context_limit_label(default)
                            )),
                    )
                    .changed()
                {
                    draft.result = None;
                }
                let parsed = parse_context_limit(&draft.text);
                if let Err(error) = &parsed {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(*error).color(ui.visuals().error_fg_color),
                        )
                        .wrap(),
                    );
                }
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(
                            parsed.as_ref().is_ok_and(|limit| *limit != stored),
                            egui::Button::new("儲存此模型上限"),
                        )
                        .clicked()
                    {
                        apply = parsed.ok();
                    }
                    if ui
                        .add_enabled(
                            stored.is_some() || !draft.text.is_empty(),
                            egui::Button::new("使用預設"),
                        )
                        .clicked()
                    {
                        if stored.is_some() {
                            apply = Some(None);
                        } else {
                            draft.text.clear();
                            draft.result = None;
                        }
                    }
                });
            });
        });
        if let Some(result) = &draft.result {
            match result {
                Ok(()) => muted(ui, "此服務／模型的上限已儲存。"),
                Err(error) => {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(error).color(ui.visuals().error_fg_color),
                        )
                        .wrap(),
                    );
                }
            }
        }
        muted(ui, "GPT 預設 1M，其他預設 256K；K＝1,000、M＝1,000,000。只儲存此服務／模型的上限，不會儲存 API 草稿。");
        muted(
            ui,
            "這是整體上下文預算，不是單次回答長度或服務商保證。接近上限會先摘要舊文字，保留近期原文與完整聊天紀錄；摘要失敗會停止送出。",
        );
        muted(ui, "圖片／原始 PDF 不會被摘要丟棄；若無法安全縮減會顯示錯誤。Codex app-server 使用其原生上下文管理。摘要會額外使用目前模型的額度。");
        if !writable {
            muted(ui, "服務資料讀取失敗，未開放寫入，以保留原資料。");
        }
        if let Some(limit) = apply {
            self.actions.push(SettingsAction::SetContextLimit {
                provider_id,
                model: model.to_owned(),
                limit,
            });
        }
    }

    fn mcp_section(&mut self, ui: &mut egui::Ui) {
        ui.heading("MCP 工具服務");
        ui.add(egui::Label::new("支援 stdio 與 Streamable HTTP。服務預設停用；對話可提出草稿，但須由你確認儲存。每次工具呼叫另行確認。憑證只在 MCP 設定輸入，不要貼進聊天。").wrap());
        if ui.button("管理 MCP 服務／連線測試").clicked() {
            self.actions.push(SettingsAction::ManageMcp);
        }
    }

    pub fn report_context_limit_save(
        &mut self,
        provider_id: Uuid,
        model: &str,
        limit: Option<u32>,
        result: Result<(), String>,
    ) {
        if let Some(draft) = self
            .context_limit_drafts
            .get_mut(&(provider_id, model.to_owned()))
        {
            if result.is_ok() {
                draft.text = limit.map(|value| value.to_string()).unwrap_or_default();
            }
            draft.result = Some(result);
        }
    }

    fn connection_fields(&mut self, ui: &mut egui::Ui, runtime: &tokio::runtime::Handle) {
        let api = self.supports_api();
        if !api {
            let chatgpt = self
                .provider
                .as_ref()
                .is_some_and(|p| p.kind == ProviderKind::ChatGPT);
            let (title, explanation) = if chatgpt {
                (
                    "透過 ChatGPT 訂閱連線",
                    "不需要 API 金鑰。這是非官方訂閱連線，僅供個人使用，可能因服務端變更而失效。",
                )
            } else {
                ("透過本機 Codex 連線", "使用 Codex app-server 的登入狀態，不需要 API 金鑰。請先在本機執行 codex login。")
            };
            info_card(ui, title, explanation);
            if chatgpt {
                self.chatgpt_controls(ui);
                muted(
                    ui,
                    "內建模型清單：2026-11-02 更新；實際可用模型依訂閱方案、工作區與開放進度而定。",
                );
                ui.hyperlink_to(
                    "OpenAI 官方訂閱模型說明",
                    "https://learn.chatgpt.com/docs/models",
                );
            } else {
                self.codex_controls(ui);
            }
            ui.add_space(12.0);
        }

        let provider_id = self.selected_provider_id();
        if api {
            field_label(ui, "API 端點");
            ui.add_sized(
                [ui.available_width(), 32.0],
                egui::TextEdit::singleline(&mut self.settings.base_url)
                    .id_salt(("settings_endpoint", provider_id))
                    .hint_text("https://api.example.com/v1"),
            );
            // Reconcile before the key field: a new key typed for the edited
            // origin is not mistaken for the old host's credential.
            self.reconcile_connection();
            if endpoint_origin(&self.settings.base_url).is_none() {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new("請輸入完整的 http:// 或 https:// API 端點。")
                            .color(ui.visuals().error_fg_color),
                    )
                    .wrap(),
                );
            }
            ui.add_space(8.0);
            field_label(ui, "API 金鑰");
            ui.horizontal(|ui| {
                let input_width = (ui.available_width() - 124.0).max(40.0);
                ui.allocate_ui_with_layout(
                    egui::vec2(input_width, 32.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        if self.api_key_field(ui).changed() {
                            self.credential_cleared = false;
                        }
                    },
                );
                if ui
                    .add_sized(
                        [54.0, 32.0],
                        egui::Button::new(if self.reveal_key { "隱藏" } else { "顯示" }),
                    )
                    .clicked()
                {
                    self.reveal_key = !self.reveal_key;
                }
                if ui
                    .add_enabled(
                        !self.settings.api_key.is_empty(),
                        egui::Button::new("清除").min_size(egui::vec2(54.0, 32.0)),
                    )
                    .on_hover_text("儲存後移除此服務的金鑰")
                    .clicked()
                {
                    self.settings.api_key.clear();
                    self.credential_generation = self.credential_generation.wrapping_add(1);
                    self.reveal_key = false;
                }
            });
            if self.credential_cleared {
                ui.add(
                    egui::Label::new("端點來源已變更，已清除原金鑰。請輸入新來源的金鑰。").wrap(),
                );
            } else {
                muted(ui, "金鑰僅存於本機憑證庫。留空並儲存即可移除。");
            }
            ui.add_space(8.0);
        }

        // Read completions only after provider/endpoint/key edits for this frame.
        self.poll_model_fetch();
        field_label(ui, "模型");
        ui.horizontal(|ui| {
            let input_width = if api {
                (ui.available_width() - 112.0).max(40.0)
            } else {
                ui.available_width()
            };
            ui.add_sized(
                [input_width, 32.0],
                egui::TextEdit::singleline(&mut self.settings.model)
                    .id_salt(("settings_model", provider_id))
                    .hint_text("輸入模型 ID"),
            );
            if api
                && ui
                    .add_enabled(
                        self.fetch.is_none() && endpoint_origin(&self.settings.base_url).is_some(),
                        egui::Button::new("取得模型").min_size(egui::vec2(106.0, 32.0)),
                    )
                    .on_hover_text("使用目前端點與金鑰查詢 /models")
                    .clicked()
            {
                self.start_model_fetch(runtime, ui.ctx().clone());
            }
        });
        if self.fetch.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                muted(ui, "正在取得模型…");
                if ui.button("取消").clicked() {
                    self.fetch = None;
                    self.fetch_status = "已取消取得模型。".to_owned();
                    self.fetch_failed = false;
                }
            });
        }
        if let Some(list) = &self.model_list {
            if Some(list.scope) == self.model_scope() && !list.models.is_empty() {
                egui::ComboBox::from_id_salt(("settings_models", list.scope.provider_id))
                    .selected_text(if self.settings.model.is_empty() {
                        "選擇模型…"
                    } else {
                        &self.settings.model
                    })
                    .width(ui.available_width())
                    .height(200.0)
                    .truncate()
                    .show_ui(ui, |ui| {
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                        for model in &list.models {
                            ui.selectable_value(&mut self.settings.model, model.clone(), model)
                                .on_hover_text(model);
                        }
                    });
            }
        }
        if !self.fetch_status.is_empty() {
            let text = egui::RichText::new(&self.fetch_status).small();
            let text = if self.fetch_failed {
                text.color(ui.visuals().error_fg_color)
            } else {
                text.weak()
            };
            ui.add(egui::Label::new(text).wrap());
        }
    }

    fn api_key_field(&mut self, ui: &mut egui::Ui) -> egui::Response {
        // egui keeps undo history by widget ID. Isolate it on every provider
        // reload/origin change/explicit clear, while keeping focus during typing.
        let salt = (
            "settings_api_key",
            self.selected_provider_id(),
            self.credential_generation,
        );
        ui.add_sized(
            [ui.available_width(), 32.0],
            egui::TextEdit::singleline(&mut self.settings.api_key)
                .id_salt(salt)
                .password(!self.reveal_key)
                .hint_text("本機服務通常可留空"),
        )
    }

    fn chatgpt_controls(&mut self, ui: &mut egui::Ui) {
        if self.chatgpt.signed_in {
            let mut label = self
                .chatgpt
                .email
                .clone()
                .unwrap_or_else(|| "已登入 ChatGPT".to_owned());
            if let Some(plan) = &self.chatgpt.plan {
                label.push_str(&format!(" · {plan}"));
            }
            ui.label(label);
            if ui.button("登出 ChatGPT").clicked() {
                self.actions.push(SettingsAction::SignOutChatGpt);
            }
        } else if self.chatgpt.signing_in {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("請在瀏覽器完成登入…");
                if ui.button("取消登入").clicked() {
                    self.actions.push(SettingsAction::CancelChatGptSignIn);
                }
            });
        } else if ui.button("使用 ChatGPT 登入").clicked() {
            self.actions.push(SettingsAction::SignInChatGpt);
        }
        if let Some(error) = &self.chatgpt.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
    }

    fn codex_controls(&mut self, ui: &mut egui::Ui) {
        field_label(ui, "Codex 執行檔（留空自動從 PATH 尋找）");
        ui.horizontal(|ui| {
            if ui
                .add(
                    egui::TextEdit::singleline(&mut self.settings.codex_executable_path)
                        .desired_width((ui.available_width() - 90.0).max(40.0))
                        .hint_text("C:\\…\\codex.exe 或 codex.cmd"),
                )
                .changed()
            {
                self.actions.push(SettingsAction::CodexPathChanged);
            }
            if ui
                .add_enabled(self.codex_picker.is_none(), egui::Button::new("選擇檔案…"))
                .clicked()
            {
                let current = std::path::PathBuf::from(self.settings.codex_executable_path.trim());
                let ctx = ui.ctx().clone();
                let (tx, rx) = mpsc::channel();
                match std::thread::Builder::new()
                    .name("popchat-codex-picker".to_owned())
                    .spawn(move || {
                        let mut dialog = rfd::FileDialog::new()
                            .set_title("選擇 Codex 執行檔")
                            .add_filter("Windows executable", &["exe", "cmd", "bat"]);
                        if let Some(parent) =
                            current.parent().filter(|path| !path.as_os_str().is_empty())
                        {
                            dialog = dialog.set_directory(parent);
                        }
                        let _ = tx.send(dialog.pick_file());
                        ctx.request_repaint();
                    }) {
                    Ok(_) => self.codex_picker = Some(rx),
                    Err(error) => self.codex_message = format!("無法開啟檔案選擇器：{error}"),
                }
            }
        });
        if ui
            .add_enabled(!self.codex_checking, egui::Button::new("檢查連線與模型"))
            .clicked()
        {
            self.actions.push(SettingsAction::InspectCodex);
        }
        if self.codex_checking {
            ui.spinner();
        }
        if !self.codex_message.is_empty() {
            muted(ui, &self.codex_message);
        }
    }

    fn commands_section(&mut self, ui: &mut egui::Ui) {
        let prompt_before = self.settings.system_prompt.clone();
        let commands_before = self.command_shortcuts.clone();
        pane_heading(
            ui,
            "指令與系統提示",
            "以 /名稱 呼叫；{input} 會替換為這次輸入的內容。",
        );
        field_label(ui, "系統提示（留空即不傳送 system 訊息）");
        // Original editors have fixed heights and independent scrolling. Do
        // not grow this field to the whole prompt and push Commands offscreen.
        egui::ScrollArea::vertical()
            .id_salt("system_prompt_editor_scroll")
            .max_height(148.0)
            .show(ui, |ui| {
                ui.add_sized(
                    [ui.available_width(), 148.0],
                    egui::TextEdit::multiline(&mut self.settings.system_prompt)
                        .desired_width(f32::INFINITY),
                );
            });
        if ui
            .add_enabled(
                self.settings.system_prompt != default_system_prompt(),
                egui::Button::new("還原預設提示"),
            )
            .clicked()
        {
            self.settings.system_prompt = default_system_prompt();
        }
        ui.add_space(12.0);
        let mut remove = None;
        for shortcut in &mut self.command_shortcuts {
            ui.push_id(shortcut.id, |ui| {
                ui.horizontal(|ui| {
                    ui.label("/");
                    let name = ui.add(
                        egui::TextEdit::singleline(&mut shortcut.name)
                            .desired_width((ui.available_width() - 80.0).max(40.0)),
                    );
                    self.commands_dirty |= name.changed();
                    if self.command_focus == Some(shortcut.id) {
                        name.request_focus();
                        self.command_focus = None;
                    }
                    if ui.button("刪除").clicked() {
                        remove = Some(shortcut.id);
                    }
                });
                egui::ScrollArea::vertical()
                    .id_salt(("command_template_scroll", shortcut.id))
                    .max_height(56.0)
                    .show(ui, |ui| {
                        self.commands_dirty |= ui
                            .add_sized(
                                [ui.available_width(), 56.0],
                                egui::TextEdit::multiline(&mut shortcut.template)
                                    .desired_width(f32::INFINITY),
                            )
                            .changed();
                    });
                ui.separator();
            });
        }
        if let Some(id) = remove {
            self.command_shortcuts.retain(|s| s.id != id);
            self.commands_dirty = true;
        }
        if ui.button("新增指令").clicked() {
            let id = Uuid::new_v4();
            self.command_shortcuts.push(PromptShortcut {
                id,
                name: "new-shortcut".to_owned(),
                template: "…{input}".to_owned(),
            });
            self.commands_dirty = true;
            self.command_focus = Some(id);
            ui.ctx().request_repaint();
        }
        let retry = self.command_save_error.is_some() && ui.button("重試儲存指令").clicked();
        let commands_changed = commands_before != self.command_shortcuts;
        if commands_changed {
            self.actions.push(SettingsAction::CommandsChanged(
                self.command_shortcuts.clone(),
            ));
        }
        if prompt_before != self.settings.system_prompt || commands_changed || retry {
            let result = (|| {
                let path = Self::config_path().ok_or_else(|| "找不到設定目錄".to_owned())?;
                save_system_prompt_only(&path, &self.settings.system_prompt)?;
                ShortcutStore::save_entries(&self.command_shortcuts)
                    .map_err(|e| format!("無法儲存指令：{e:#}"))?;
                Ok::<(), String>(())
            })();
            match result {
                Ok(()) => {
                    self.saved_settings.system_prompt = self.settings.system_prompt.clone();
                    self.commands_dirty = false;
                    self.command_save_error = None;
                }
                Err(error) => self.command_save_error = Some(error),
            }
        }
        if let Some(error) = &self.command_save_error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
    }

    fn hotkey_section(&mut self, ui: &mut egui::Ui, runtime: &tokio::runtime::Handle) {
        pane_heading(ui, "隨時喚出 PopChat", "不用切換視窗，就能開始下一段對話。");
        field_label(ui, "目前的全域快捷鍵");
        let palette = theme::current_palette(ui.ctx());
        egui::Frame::new()
            .fill(control_fill(palette.dark))
            .corner_radius(8)
            .inner_margin(20)
            .show(ui, |ui| {
                ui.set_width((ui.available_width()).max(0.0));
                if let Some(label) = &self.hotkey_label {
                    ui.label(egui::RichText::new(label).size(22.0).color(palette.accent));
                } else {
                    ui.label(egui::RichText::new("尚未註冊成功").size(18.0));
                }
            });
        ui.add_space(8.0);
        field_label(ui, "自訂全域快捷鍵");
        if crate::ui::hotkey_recorder::show(ui, &mut self.hotkey_draft) {
            self.hotkey_update_error = None;
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !crate::ui::hotkey_recorder::is_recording(ui.ctx()),
                    egui::Button::new("套用快捷鍵"),
                )
                .clicked()
            {
                self.hotkey_update_error = None;
                self.actions
                    .push(SettingsAction::ChangeHotkey(self.hotkey_draft.clone()));
            }
            if ui.button("還原預設").clicked() {
                crate::ui::hotkey_recorder::cancel(ui.ctx());
                self.hotkey_draft = default_hotkey_binding();
                self.hotkey_update_error = None;
                self.actions
                    .push(SettingsAction::ChangeHotkey(self.hotkey_draft.clone()));
            }
        });
        muted(
            ui,
            "預設 Alt+Down；Alt+Space 是 Windows 系統選單快捷鍵，建議選擇其他組合。",
        );
        muted(
            ui,
            "套用成功後才會儲存；若組合已被占用，原快捷鍵會繼續生效。",
        );
        if let Some(error) = &self.hotkey_update_error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        ui.add_space(12.0);
        field_label(ui, "聊天快捷鍵");
        crate::ui::shortcut_list::show(ui);
        muted(ui, "也可直接將檔案拖入聊天視窗；不需要先點附件按鈕。");
        ui.add_space(12.0);
        let was_enabled = self.startup_enabled;
        if ui
            .checkbox(
                &mut self.startup_enabled,
                "登入 Windows 時啟動 PopChat（隱藏待命）",
            )
            .changed()
        {
            match crate::startup::set_enabled(self.startup_enabled) {
                Ok(()) => self.startup_error = None,
                Err(error) => {
                    self.startup_enabled = was_enabled;
                    self.startup_error = Some(format!("無法更新登入啟動：{error:#}"));
                }
            }
        }
        if let Some(error) = &self.startup_error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        ui.add_space(8.0);
        ui.checkbox(
            &mut self.settings.automatic_update_checks,
            "自動檢查更新（每天一次）",
        );
        ui.horizontal_wrapped(|ui| {
            ui.label(format!("PopChat {}", env!("CARGO_PKG_VERSION")));
            if ui
                .add_enabled(
                    self.updates.state != crate::update::CheckState::Checking,
                    egui::Button::new("立即檢查更新"),
                )
                .clicked()
            {
                self.updates.check(ui.ctx(), runtime, true);
            }
        });
        muted(ui, &format!("Windows 移植與維護：{}", crate::AUTHOR));
        ui.horizontal_wrapped(|ui| {
            ui.hyperlink_to("Windows 專案", env!("CARGO_PKG_REPOSITORY"));
            ui.label("·");
            ui.hyperlink_to("原專案：lec77/PopChat", "https://github.com/lec77/PopChat");
        });
        match &self.updates.state {
            crate::update::CheckState::Idle => {}
            crate::update::CheckState::Checking => {
                ui.spinner();
            }
            crate::update::CheckState::UpToDate => muted(ui, "目前已是最新版本。"),
            crate::update::CheckState::Available(release) => {
                ui.hyperlink_to(
                    format!("已有新版本 {} · 開啟發行頁面", release.version),
                    &release.page_url,
                );
                let ready = matches!(
                    self.update_download.state,
                    crate::update::DownloadState::Ready(_)
                );
                let downloading = matches!(
                    self.update_download.state,
                    crate::update::DownloadState::Downloading { .. }
                );
                if !downloading
                    && (!ready
                        || (self.update_install_message.is_some() && !self.update_install_busy))
                    && ui
                        .button(if ready {
                            "重新下載更新"
                        } else {
                            "下載更新"
                        })
                        .clicked()
                {
                    self.update_install_message = None;
                    self.update_download.start(ui.ctx(), runtime, release);
                }
            }
            crate::update::CheckState::Failed(error) => {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
        }
        match &self.update_download.state {
            crate::update::DownloadState::Idle => {}
            crate::update::DownloadState::Downloading { received, total } => {
                let _ = update_download_progress(ui, *received, *total);
            }
            crate::update::DownloadState::Ready(prepared) => {
                ui.label(format!("{} 已下載並通過完整性驗證。", prepared.version()));
                if ui
                    .add_enabled(
                        !self.update_install_busy,
                        egui::Button::new("安裝並重新啟動"),
                    )
                    .clicked()
                {
                    self.actions
                        .push(SettingsAction::InstallUpdate(prepared.clone()));
                }
            }
            crate::update::DownloadState::Failed(error) => {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(format!("更新下載失敗：{error}"))
                            .color(ui.visuals().error_fg_color),
                    )
                    .wrap(),
                );
            }
        }
        if let Some(message) = &self.update_install_message {
            ui.add(egui::Label::new(message).wrap());
        }
    }

    /// Light/Dark/Auto + accent 四預設 + 自訂色。
    fn appearance_section(&mut self, ui: &mut egui::Ui) {
        pane_heading(
            ui,
            "你的專屬介面",
            "選擇舒適的顯示模式與一點個人色彩，變更會即時預覽。",
        );
        field_label(ui, "顯示模式");
        ui.horizontal(|ui| {
            let width = ((ui.available_width() - 16.0) / 3.0).max(40.0);
            ui.spacing_mut().item_spacing.x = 8.0;
            for (value, label) in [
                (Appearance::Auto, "跟隨系統"),
                (Appearance::Light, "淺色"),
                (Appearance::Dark, "深色"),
            ] {
                if appearance_option(ui, value, label, self.settings.appearance == value, width)
                    .clicked()
                {
                    self.settings.appearance = value;
                }
            }
        });
        ui.add_space(18.0);
        field_label(ui, "強調色");
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            for (hex, name) in ACCENT_PRESETS.into_iter().zip(["藍", "紫", "橘", "綠"]) {
                let color = theme::hex_to_color32(hex);
                let selected = self.settings.accent_hex.eq_ignore_ascii_case(hex);
                let response = ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new(name).color(theme::contrasting_text_on(hex)),
                        )
                        .fill(color)
                        .corner_radius(16)
                        .min_size(egui::vec2(64.0, 32.0)),
                    )
                    .on_hover_text(hex);
                if selected {
                    ui.painter().rect_stroke(
                        response.rect.expand(3.0),
                        18,
                        egui::Stroke::new(1.5_f32, theme::current_palette(ui.ctx()).accent),
                        egui::StrokeKind::Outside,
                    );
                }
                if response.clicked() {
                    self.settings.accent_hex = hex.to_owned();
                }
            }
            self.accent_picker.show(
                ui,
                &mut self.settings.accent_hex,
                &mut self.settings.custom_accent_hex,
            );
        });
        ui.add_space(12.0);
        field_label(ui, "提問氣泡樣式");
        ui.horizontal(|ui| {
            ui.selectable_value(
                &mut self.settings.bubble_style,
                theme::BubbleStyle::AccentTint,
                "強調色淡底",
            );
            ui.selectable_value(
                &mut self.settings.bubble_style,
                theme::BubbleStyle::AccentFill,
                "強調色填滿",
            );
        });
        ui.add_space(12.0);
        field_label(ui, "串流文字顯示");
        ui.horizontal(|ui| {
            ui.selectable_value(
                &mut self.settings.streaming_mode,
                crate::chat::store::StreamingMode::PerCharacter,
                "逐字",
            );
            ui.selectable_value(
                &mut self.settings.streaming_mode,
                crate::chat::store::StreamingMode::PerSentence,
                "逐句",
            );
        });
        ui.add_space(22.0);
        fonts::show_font_preferences(ui, &mut self.settings.font_preferences);
        self.chat_font_size_preferences(ui);
        ui.add_space(22.0);
        window_prefs::show_preferences(
            ui,
            &mut self.settings.window_preferences,
            self.current_window_position,
        );
    }

    /// Chat-only typography; the surrounding settings UI keeps its normal scale.
    fn chat_font_size_preferences(&mut self, ui: &mut egui::Ui) -> egui::Response {
        use crate::ui::{preference_layout as form, typography};
        ui.add_space(12.0);
        form::heading(ui, "文字大小");
        form::card(ui, |ui| {
            let slider = form::row(ui, "聊天文字大小", |ui| {
                ui.spacing_mut().slider_width = (ui.available_width() - 58.0).max(60.0);
                let response = ui.add(
                    egui::Slider::new(
                        &mut self.settings.chat_font_size,
                        typography::MIN_SIZE..=typography::MAX_SIZE,
                    )
                    .show_value(false)
                    .step_by(1.0),
                );
                ui.label(format!("{} pt", self.settings.chat_font_size));
                response
            });
            form::caption(
                ui,
                "套用至訊息、Markdown、表格、程式碼、思考內容與輸入框；不改變介面縮放。",
            );
            ui.add_space(4.0);
            if ui.small_button("還原預設大小").clicked() {
                self.settings.chat_font_size = typography::DEFAULT_SIZE;
            }
            form::divider(ui);
            form::caption(ui, "大小預覽");
            ui.scope(|ui| {
                typography::apply(ui, self.settings.chat_font_size);
                ui.add(
                    egui::Label::new("讓想法，自然成為對話。\nThe quick brown fox · 0123456789")
                        .wrap(),
                );
            });
            slider
        })
        .inner
    }

    /// Web Search 引擎選擇。
    fn search_section(&mut self, ui: &mut egui::Ui) {
        pane_heading(
            ui,
            "讓回答連結更多資訊",
            "允許模型在需要時搜尋網頁，取得對話以外的資訊。",
        );
        let palette = theme::current_palette(ui.ctx());
        egui::Frame::new()
            .fill(control_fill(palette.dark))
            .corner_radius(8)
            .inner_margin(16)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.checkbox(&mut self.settings.web_search_enabled, "啟用網路搜尋");
                muted(ui, "實際搜尋能力依使用的模型服務而定。");
            });
        ui.add_space(18.0);
        ui.add_enabled_ui(self.settings.web_search_enabled, |ui| {
            field_label(ui, "偏好的搜尋引擎");
            egui::ComboBox::from_id_salt("settings_search_engine")
                .selected_text(search_engine_label(&self.settings.search_engine))
                .width(ui.available_width())
                .truncate()
                .show_ui(ui, |ui| {
                    for engine in SEARCH_ENGINES {
                        ui.selectable_value(
                            &mut self.settings.search_engine,
                            engine.to_owned(),
                            search_engine_label(engine),
                        );
                    }
                });
            if self.settings.search_engine != self.search_key_engine {
                self.settings.search_engine_key = SecretStore::new()
                    .get(&search_secret_key(&self.settings.search_engine))
                    .unwrap_or_default();
                self.search_key_engine = self.settings.search_engine.clone();
                self.search_key_dirty = false;
            }
            if matches!(self.settings.search_engine.as_str(), "Brave" | "Tavily") {
                ui.add_space(8.0);
                field_label(ui, "搜尋引擎 API 金鑰");
                self.search_key_dirty |= ui
                    .add(
                        egui::TextEdit::singleline(&mut self.settings.search_engine_key)
                            .id_salt(("settings_search_key", self.settings.search_engine.as_str()))
                            .password(true)
                            .desired_width(f32::INFINITY),
                    )
                    .changed();
                muted(
                    ui,
                    "僅儲存於 Windows 憑證庫。未填時會明確提示並改用 DuckDuckGo。",
                );
            }
        });
    }
}

fn search_secret_key(engine: &str) -> String {
    format!("web-search-{}", engine.to_lowercase())
}

fn save_system_prompt_only(path: &std::path::Path, prompt: &str) -> Result<(), String> {
    save_string_preference_only(path, "system_prompt", prompt)
}

fn save_string_preference_only(
    path: &std::path::Path,
    key: &str,
    value: &str,
) -> Result<(), String> {
    let mut preferences = serde_json::Map::new();
    preferences.insert(key.to_owned(), value.into());
    save_preference_values_only(path, preferences)
}

fn general_preferences(settings: &AppSettings) -> serde_json::Map<String, serde_json::Value> {
    let mut preferences = serde_json::Map::new();
    // Explicit allowlist. Never serialize API/search keys or the catalog draft
    // or an unapplied hotkey as a side effect of a general preference change.
    let value =
        serde_json::to_value(settings).expect("AppSettings contains only JSON-safe primitives");
    for name in [
        "appearance",
        "accent_hex",
        "custom_accent_hex",
        "search_engine",
        "web_search_enabled",
        "streaming_mode",
        "bubble_style",
        "automatic_update_checks",
        "font_preferences",
        "chat_font_size",
        "window_preferences",
    ] {
        preferences.insert(name.to_owned(), value[name].clone());
    }
    preferences
}

fn save_preference_values_only(
    path: &std::path::Path,
    preferences: serde_json::Map<String, serde_json::Value>,
) -> Result<(), String> {
    let mut stored = match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice::<serde_json::Value>(&bytes)
            .map_err(|error| format!("設定檔無法讀取；保留原檔：{error}"))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            serde_json::to_value(AppSettings::default()).map_err(|e| e.to_string())?
        }
        Err(error) => return Err(format!("設定檔無法讀取：{error}")),
    };
    let object = stored
        .as_object_mut()
        .ok_or_else(|| "設定檔格式錯誤；保留原檔".to_owned())?;
    object.extend(preferences);
    // Only this preference is committed: an unsaved provider endpoint/key or
    // appearance draft must not accidentally piggyback on Commands autosave.
    crate::paths::atomic_write(
        path,
        &serde_json::to_vec_pretty(&stored).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("無法儲存偏好設定：{e}"))
}

async fn fetch_catalog(
    provider: &str,
    base_url: &str,
    api_key: &str,
) -> Result<ModelCatalog, String> {
    let (models, mut capabilities) =
        crate::chat::providers::fetch_models_list(provider, base_url, api_key).await?;
    // Preserve authoritative /models metadata, then supplement local Ollama's
    // OpenAI-compatible surface using its native capability endpoint.
    let native = crate::chat::providers::probe_ollama_capabilities(base_url, &models).await;
    capabilities.extend(native);
    Ok(ModelCatalog {
        models,
        capabilities,
    })
}

fn secondary(dark: bool) -> egui::Color32 {
    if dark {
        egui::Color32::from_rgb(0xA2, 0xA8, 0xB6)
    } else {
        theme::LIGHT_SECONDARY
    }
}

fn control_fill(dark: bool) -> egui::Color32 {
    if dark {
        egui::Color32::from_rgb(0x22, 0x23, 0x2A)
    } else {
        theme::LIGHT_CONTROL
    }
}

fn settings_style(ui: &mut egui::Ui) {
    let palette = theme::current_palette(ui.ctx());
    let style = ui.style_mut();
    style.spacing.item_spacing = egui::vec2(6.0, 4.0);
    style.spacing.button_padding = egui::vec2(10.0, 6.0);
    style.spacing.interact_size.y = 32.0;
    style.visuals.extreme_bg_color = control_fill(palette.dark);
    style.visuals.widgets.inactive.bg_fill = control_fill(palette.dark);
    style.visuals.widgets.inactive.weak_bg_fill = control_fill(palette.dark);
    style.visuals.widgets.inactive.bg_stroke = egui::Stroke::NONE;
    style.visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(6);
    style.visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(6);
    style.visuals.widgets.active.corner_radius = egui::CornerRadius::same(6);
    style.visuals.widgets.hovered.bg_stroke =
        egui::Stroke::new(1.0_f32, palette.accent.gamma_multiply(0.35));
    style.visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0_f32, palette.accent);
}

fn muted(ui: &mut egui::Ui, text: &str) {
    let palette = theme::current_palette(ui.ctx());
    ui.add(
        egui::Label::new(
            egui::RichText::new(text)
                .size(12.0)
                .color(secondary(palette.dark)),
        )
        .wrap(),
    );
}

/// Download feedback is informational, not a 32pt-tall settings control.
/// Keep text outside the track so a readable label never makes the bar thick.
fn update_download_progress(
    ui: &mut egui::Ui,
    received: u64,
    total: Option<u64>,
) -> Option<egui::Response> {
    let total = total.filter(|total| *total > 0);
    let progress = total.map(|total| (received as f64 / total as f64).clamp(0.0, 1.0) as f32);
    let palette = theme::current_palette(ui.ctx());
    ui.horizontal(|ui| {
        if progress.is_none() {
            ui.add(egui::Spinner::new().size(12.0));
        }
        muted(ui, "下載並驗證更新中…");
        if let Some(progress) = progress {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(format!("{:.0}%", (progress * 100.0).floor()))
                        .size(11.0)
                        .monospace()
                        .color(secondary(palette.dark)),
                );
            });
        }
    });
    let response = progress.map(|progress| {
        ui.add(
            egui::ProgressBar::new(progress)
                .desired_width(ui.available_width())
                .desired_height(3.0)
                .corner_radius(2)
                .fill(palette.accent.gamma_multiply(0.85)),
        )
    });
    let mib = received as f64 / 1_048_576.0;
    muted(
        ui,
        &match total {
            Some(total) => format!("{mib:.1} / {:.1} MiB", total as f64 / 1_048_576.0),
            None => format!("已下載 {mib:.1} MiB"),
        },
    );
    response
}

fn field_label(ui: &mut egui::Ui, text: &str) {
    muted(ui, text);
}

fn pane_heading(ui: &mut egui::Ui, title: &str, subtitle: &str) {
    ui.label(egui::RichText::new(title).size(17.0).strong());
    muted(ui, subtitle);
    ui.add_space(16.0);
}

fn info_card(ui: &mut egui::Ui, title: &str, explanation: &str) {
    let palette = theme::current_palette(ui.ctx());
    egui::Frame::new()
        .fill(palette.accent.gamma_multiply(0.07))
        .corner_radius(8)
        .inner_margin(16)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(egui::RichText::new(title).size(14.0).strong());
            muted(ui, explanation);
        });
}

fn appearance_option(
    ui: &mut egui::Ui,
    value: Appearance,
    label: &str,
    selected: bool,
    width: f32,
) -> egui::Response {
    let palette = theme::current_palette(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 88.0), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            label,
        )
    });
    let fill = if response.hovered() {
        ui.visuals().widgets.hovered.weak_bg_fill
    } else {
        control_fill(palette.dark)
    };
    ui.painter().rect(
        rect,
        8,
        fill,
        if selected {
            egui::Stroke::new(1.5_f32, palette.accent)
        } else {
            egui::Stroke::NONE
        },
        egui::StrokeKind::Inside,
    );
    let preview = egui::Rect::from_min_size(
        rect.min + egui::vec2(14.0, 12.0),
        egui::vec2((width - 28.0).max(12.0), 34.0),
    );
    let dark = value == Appearance::Dark || (value == Appearance::Auto && palette.dark);
    let background = if dark {
        egui::Color32::from_rgb(0x17, 0x18, 0x1D)
    } else {
        theme::panel_fill(false)
    };
    ui.painter().rect_filled(preview, 4, background);
    ui.painter().rect_filled(
        egui::Rect::from_min_size(
            preview.min + egui::vec2(7.0, 7.0),
            egui::vec2((preview.width() * 0.45).max(4.0), 4.0),
        ),
        2,
        secondary(dark).gamma_multiply(0.5),
    );
    ui.painter().rect_filled(
        egui::Rect::from_min_size(
            preview.min + egui::vec2(7.0, 17.0),
            egui::vec2((preview.width() - 14.0).max(4.0), 10.0),
        ),
        3,
        palette.accent.gamma_multiply(0.3),
    );
    ui.painter().text(
        egui::pos2(rect.center().x, rect.bottom() - 21.0),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(13.0),
        if selected {
            palette.accent
        } else {
            ui.visuals().text_color()
        },
    );
    response
}

fn endpoint_origin(endpoint: &str) -> Option<String> {
    let url = reqwest::Url::parse(endpoint.trim()).ok()?;
    (matches!(url.scheme(), "http" | "https")
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none())
    .then(|| url.origin().ascii_serialization())
}

fn normalized_accent(hex: &str) -> Option<String> {
    let digits = hex.trim().strip_prefix('#').unwrap_or(hex.trim());
    (digits.len() == 6 && digits.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| theme::normalize_hex(hex))
}

fn search_engine_label(engine: &str) -> &str {
    match engine {
        "Provider-native" => "服務內建搜尋",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_progress_is_a_thin_track_despite_settings_control_height() {
        for dark in [false, true] {
            for received in [0, 524_288, 1_048_576, u64::MAX] {
                let ctx = egui::Context::default();
                theme::apply(
                    &ctx,
                    if dark { "Dark" } else { "Light" },
                    theme::DEFAULT_ACCENT_HEX,
                );
                let mut rect = None;
                let output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(360.0, 200.0),
                        )),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            settings_style(ui);
                            assert_eq!(ui.spacing().interact_size.y, 32.0);
                            rect = update_download_progress(ui, received, Some(1_048_576))
                                .map(|response| response.rect);
                        });
                    },
                );
                let rect = rect.unwrap();
                assert_eq!(rect.height(), 3.0);
                assert!(rect.width() > 250.0);
                assert!(rect.right() <= 360.0);
                let text = output
                    .shapes
                    .iter()
                    .filter_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) => Some(text.galley.job.text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                assert!(text.contains(&"下載並驗證更新中…"));
                let expected = if received == 0 {
                    "0%"
                } else if received == 524_288 {
                    "50%"
                } else {
                    "100%"
                };
                assert!(
                    text.contains(&expected),
                    "missing separate percentage: {text:?}"
                );
            }
        }
    }

    #[test]
    fn update_progress_without_content_length_does_not_invent_a_percentage() {
        for total in [None, Some(0)] {
            let ctx = egui::Context::default();
            let output = ctx.run(Default::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    settings_style(ui);
                    assert!(update_download_progress(ui, 1_048_576, total).is_none());
                });
            });
            let text = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) => Some(text.galley.job.text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert!(text.contains(&"已下載 1.0 MiB"));
            assert!(text.iter().all(|text| !text.contains('%')));
        }
    }

    fn provider(id: u128, endpoint: &str) -> Provider {
        Provider::new(
            Uuid::from_u128(id),
            "Same display name",
            endpoint,
            false,
            "default",
            ProviderKind::OpenAICompatible,
            None,
        )
    }

    fn draft() -> SettingsUi {
        let mut ui = SettingsUi::from_settings(AppSettings::default());
        ui.load_provider(
            &provider(1, "https://a.example/v1"),
            "key-a".to_owned(),
            "manual-model".to_owned(),
            vec!["cached-a".to_owned()],
        );
        ui
    }

    fn queue_request(ui: &mut SettingsUi) -> mpsc::Sender<Result<ModelCatalog, String>> {
        let (tx, rx) = mpsc::channel();
        ui.fetch = Some(ModelFetch {
            scope: ui.model_scope().unwrap(),
            rx,
            task: None,
        });
        tx
    }

    fn context_editor_frame(
        draft: &mut SettingsUi,
        ctx: &egui::Context,
        model: &str,
        stored: Option<u32>,
        time: f64,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(480.0, 600.0),
                )),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    settings_style(ui);
                    draft.context_limit_editor(ui, Uuid::from_u128(1), model, stored, true);
                    assert!(ui.min_rect().width() <= 480.5);
                });
            },
        )
    }

    fn text_position(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
        fn find(shape: &egui::Shape, label: &str) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Text(text) if text.galley.job.text.contains(label) => {
                    Some(text.pos + text.galley.size() / 2.0)
                }
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| find(shape, label)),
                _ => None,
            }
        }
        output
            .shapes
            .iter()
            .find_map(|shape| find(&shape.shape, label))
            .expect("context setting control must be visible")
    }

    fn pointer_button(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn context_limit_real_typing_and_save_only_emit_the_model_preference() {
        let ctx = egui::Context::default();
        let mut draft = draft();
        let before = draft.settings.clone();
        let _ = context_editor_frame(&mut draft, &ctx, "gpt-4.1", None, 0.0, vec![]);
        let output = context_editor_frame(&mut draft, &ctx, "gpt-4.1", None, 0.1, vec![]);
        let input = text_position(&output, "留空用預設");
        let _ = context_editor_frame(
            &mut draft,
            &ctx,
            "gpt-4.1",
            None,
            0.2,
            vec![
                egui::Event::PointerMoved(input),
                pointer_button(input, true),
            ],
        );
        let _ = context_editor_frame(
            &mut draft,
            &ctx,
            "gpt-4.1",
            None,
            0.3,
            vec![pointer_button(input, false)],
        );
        let output = context_editor_frame(
            &mut draft,
            &ctx,
            "gpt-4.1",
            None,
            0.4,
            vec![egui::Event::Text("128K".into())],
        );
        assert_eq!(
            draft.context_limit_drafts[&(Uuid::from_u128(1), "gpt-4.1".into())].text,
            "128K"
        );
        assert!(
            draft.take_actions().is_empty(),
            "typing must not save anything"
        );
        let save = text_position(&output, "儲存此模型上限");
        let _ = context_editor_frame(
            &mut draft,
            &ctx,
            "gpt-4.1",
            None,
            0.5,
            vec![egui::Event::PointerMoved(save), pointer_button(save, true)],
        );
        let _ = context_editor_frame(
            &mut draft,
            &ctx,
            "gpt-4.1",
            None,
            0.6,
            vec![pointer_button(save, false)],
        );
        let actions = draft.take_actions();
        assert!(
            matches!(actions.as_slice(), [SettingsAction::SetContextLimit { provider_id, model, limit: Some(128_000) }] if *provider_id == Uuid::from_u128(1) && model == "gpt-4.1")
        );
        assert!(
            draft.settings == before,
            "API endpoint/key/model drafts must remain unchanged"
        );
        draft.report_context_limit_save(
            Uuid::from_u128(1),
            "gpt-4.1",
            Some(128_000),
            Err("fixture write failure".into()),
        );
        let state = &draft.context_limit_drafts[&(Uuid::from_u128(1), "gpt-4.1".into())];
        assert_eq!(state.text, "128K");
        assert!(state.result.as_ref().unwrap().is_err());
    }

    #[test]
    fn context_limit_default_button_restores_only_that_model() {
        let ctx = egui::Context::default();
        let mut draft = draft();
        let before = draft.settings.clone();
        let _ = context_editor_frame(&mut draft, &ctx, "gpt-4.1", Some(128_000), 0.0, vec![]);
        let _ = context_editor_frame(&mut draft, &ctx, "other-model", None, 0.1, vec![]);
        draft
            .context_limit_drafts
            .get_mut(&(Uuid::from_u128(1), "other-model".into()))
            .unwrap()
            .text = "64K".into();
        let output = context_editor_frame(&mut draft, &ctx, "gpt-4.1", Some(128_000), 0.2, vec![]);
        let reset = text_position(&output, "使用預設");
        let _ = context_editor_frame(
            &mut draft,
            &ctx,
            "gpt-4.1",
            Some(128_000),
            0.3,
            vec![
                egui::Event::PointerMoved(reset),
                pointer_button(reset, true),
            ],
        );
        let _ = context_editor_frame(
            &mut draft,
            &ctx,
            "gpt-4.1",
            Some(128_000),
            0.4,
            vec![pointer_button(reset, false)],
        );
        let actions = draft.take_actions();
        assert!(
            matches!(actions.as_slice(), [SettingsAction::SetContextLimit { model, limit: None, .. }] if model == "gpt-4.1")
        );
        draft.report_context_limit_save(Uuid::from_u128(1), "gpt-4.1", None, Ok(()));
        assert!(
            draft.context_limit_drafts[&(Uuid::from_u128(1), "gpt-4.1".into())]
                .text
                .is_empty()
        );
        assert_eq!(
            draft.context_limit_drafts[&(Uuid::from_u128(1), "other-model".into())].text,
            "64K"
        );
        assert!(draft.settings == before);
    }

    #[test]
    fn context_limit_invalid_draft_does_not_emit_an_action() {
        let ctx = egui::Context::default();
        let mut draft = draft();
        let _ = context_editor_frame(&mut draft, &ctx, "model", None, 0.0, vec![]);
        draft
            .context_limit_drafts
            .get_mut(&(Uuid::from_u128(1), "model".into()))
            .unwrap()
            .text = "0".into();
        let output = context_editor_frame(&mut draft, &ctx, "model", None, 0.1, vec![]);
        let save = text_position(&output, "儲存此模型上限");
        let _ = context_editor_frame(
            &mut draft,
            &ctx,
            "model",
            None,
            0.2,
            vec![egui::Event::PointerMoved(save), pointer_button(save, true)],
        );
        let _ = context_editor_frame(
            &mut draft,
            &ctx,
            "model",
            None,
            0.3,
            vec![pointer_button(save, false)],
        );
        assert!(draft.take_actions().is_empty());
    }

    #[test]
    fn fallback_visuals_entry_uses_shared_light_palette() {
        let ctx = egui::Context::default();
        let mut settings = AppSettings::default();
        settings.appearance = Appearance::Light;
        SettingsUi::from_settings(settings).apply_visuals(&ctx);
        assert_eq!(ctx.style().visuals.panel_fill, theme::LIGHT_PANEL);
        assert_eq!(ctx.style().visuals.extreme_bg_color, theme::LIGHT_CONTROL);
    }

    #[test]
    fn light_appearance_preview_paints_actual_cream_panel_in_either_host_theme() {
        for host in ["Light", "Dark"] {
            let ctx = egui::Context::default();
            theme::apply(&ctx, host, DEFAULT_ACCENT);
            let mut preview = egui::Rect::NOTHING;
            let output = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    settings_style(ui);
                    let response = appearance_option(ui, Appearance::Light, "淺色", true, 140.0);
                    preview = egui::Rect::from_min_size(
                        response.rect.min + egui::vec2(14.0, 12.0),
                        egui::vec2(112.0, 34.0),
                    );
                    if host == "Light" {
                        assert_eq!(ui.visuals().extreme_bg_color, theme::LIGHT_CONTROL);
                    }
                });
            });
            assert!(output.shapes.iter().any(|shape| matches!(
                &shape.shape,
                egui::Shape::Rect(rect) if rect.rect == preview && rect.fill == theme::LIGHT_PANEL
            )), "{host}: the Light option must show the actual cream panel");
        }
    }

    #[test]
    fn settings_controls_use_theme_aware_borderless_surfaces() {
        for dark in [false, true] {
            let ctx = egui::Context::default();
            let palette = theme::apply(&ctx, if dark { "Dark" } else { "Light" }, DEFAULT_ACCENT);
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    settings_style(ui);
                    assert_eq!(ui.visuals().extreme_bg_color, control_fill(dark));
                    assert_eq!(ui.visuals().widgets.inactive.bg_stroke, egui::Stroke::NONE);
                    assert_eq!(
                        ui.visuals().widgets.inactive.corner_radius,
                        egui::CornerRadius::same(6)
                    );
                    assert_eq!(ui.spacing().interact_size.y, 32.0);
                    assert_ne!(secondary(dark), control_fill(dark));
                    assert_eq!(theme::current_palette(ctx).accent, palette.accent);
                });
            });
        }
        assert_ne!(control_fill(true), control_fill(false));
    }

    #[test]
    fn focused_panes_fit_the_settings_body_in_both_themes() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        for dark in [false, true] {
            for (tab, _) in SettingsTab::ALL {
                let ctx = egui::Context::default();
                theme::apply(&ctx, if dark { "Dark" } else { "Light" }, DEFAULT_ACCENT);
                // Match the app's larger global type and padding, then prove
                // the compact settings overrides stay inside the actual card.
                ctx.style_mut(|style| {
                    style
                        .text_styles
                        .insert(egui::TextStyle::Body, egui::FontId::proportional(15.0));
                    style
                        .text_styles
                        .insert(egui::TextStyle::Small, egui::FontId::proportional(12.0));
                    style
                        .text_styles
                        .insert(egui::TextStyle::Button, egui::FontId::proportional(13.0));
                    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
                    style.spacing.button_padding = egui::vec2(12.0, 7.0);
                    style.spacing.interact_size.y = 30.0;
                });
                let mut draft = draft();
                draft.settings.provider_name = "A very long provider name that must truncate rather than widen the settings card".repeat(3);
                let before = draft.settings.clone();
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(636.0, 350.0),
                    )),
                    ..Default::default()
                };
                let _ = ctx.run(input, |ctx| {
                    egui::CentralPanel::default()
                        .frame(egui::Frame::NONE)
                        .show(ctx, |ui| {
                            settings_style(ui);
                            // The real settings body is a bounded scroll area;
                            // Commands/startup/update controls are intentionally
                            // longer than one viewport, while Save stays fixed.
                            egui::ScrollArea::vertical().max_height(350.0).show(
                                ui,
                                |ui| match tab {
                                    SettingsTab::Service => {
                                        assert!(draft
                                            .provider_picker(ui, &[draft.provider.clone().unwrap()])
                                            .is_none());
                                        draft.connection_fields(ui, runtime.handle());
                                    }
                                    SettingsTab::Appearance => draft.appearance_section(ui),
                                    SettingsTab::Search => draft.search_section(ui),
                                    SettingsTab::Commands => draft.commands_section(ui),
                                    SettingsTab::Hotkey => {
                                        draft.hotkey_section(ui, runtime.handle())
                                    }
                                    SettingsTab::Mcp => draft.mcp_section(ui),
                                },
                            );
                            assert!(
                                ui.min_rect().width() <= 636.5,
                                "pane exceeds available width: {:?}",
                                ui.min_rect()
                            );
                            assert!(
                                ui.min_rect().height() <= 350.5,
                                "pane exceeds available height: {:?}",
                                ui.min_rect()
                            );
                        });
                });
                assert!(draft.settings == before);
            }
        }
    }

    #[test]
    fn subscription_panes_show_connection_guidance_instead_of_api_fields() {
        fn collect_text(shape: &egui::Shape, text: &mut String) {
            match shape {
                egui::Shape::Text(shape) => text.push_str(&shape.galley.job.text),
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        collect_text(shape, text);
                    }
                }
                _ => {}
            }
        }

        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        for kind in [ProviderKind::ChatGPT, ProviderKind::CodexAppServer] {
            let ctx = egui::Context::default();
            let mut draft = draft();
            let mut backend = provider(2, "https://example.com/v1");
            backend.kind = kind;
            draft.load_provider(&backend, String::new(), "backend-model".to_owned(), vec![]);
            let output = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    settings_style(ui);
                    draft.connection_fields(ui, runtime.handle());
                });
            });
            let mut text = String::new();
            for shape in output.shapes {
                collect_text(&shape.shape, &mut text);
            }
            assert!(text.contains("不需要 API 金鑰"));
            assert!(text.contains("backend-model"));
            assert!(!text.contains("API 端點"));
            assert!(!text.contains("取得模型"));
        }
    }

    #[test]
    fn fetched_models_belong_to_the_selected_identity_and_preserve_manual_choice() {
        let mut ui = draft();
        queue_request(&mut ui)
            .send(Ok(vec!["first".to_owned(), "second".to_owned()].into()))
            .unwrap();
        ui.poll_model_fetch();

        let (id, models) = ui.fetched_models().unwrap();
        assert_eq!(Some(id), ui.selected_provider_id());
        assert_eq!(models, ["first", "second"]);
        assert_eq!(ui.settings.model, "manual-model");
        assert!(ui.fetch.is_none());
    }

    #[test]
    fn model_fetch_error_is_visible_and_keeps_existing_model() {
        let mut ui = draft();
        queue_request(&mut ui)
            .send(Err("HTTP 401".to_owned()))
            .unwrap();
        ui.poll_model_fetch();

        assert!(ui.fetch_status.contains("HTTP 401"));
        assert!(ui.fetch_failed);
        assert_eq!(ui.settings.model, "manual-model");
        assert!(ui.fetch.is_none());
    }

    #[test]
    fn provider_switch_reloads_keys_by_identity_and_discards_queued_result() {
        let mut ui = draft();
        queue_request(&mut ui)
            .send(Ok(vec!["late-a".to_owned()].into()))
            .unwrap();
        ui.settings.api_key = "unsaved-key-a".to_owned();
        ui.reveal_key = true;
        // Same display name and origin must still be a distinct credential identity.
        let b = provider(2, "https://a.example/v1");
        ui.load_provider(
            &b,
            "key-b".to_owned(),
            "model-b".to_owned(),
            vec!["cached-b".to_owned()],
        );
        ui.poll_model_fetch();

        assert_eq!(ui.selected_provider_id(), Some(b.id));
        assert_eq!(ui.settings.api_key, "key-b");
        assert_eq!(ui.settings.model, "model-b");
        assert_eq!(ui.model_list.as_ref().unwrap().models, ["cached-b"]);
        assert!(ui.fetched_models().is_none());
        assert!(!ui.reveal_key);

        ui.load_provider(
            &provider(1, "https://a.example/v1"),
            "key-a".to_owned(),
            "model-a".to_owned(),
            vec![],
        );
        assert_eq!(ui.settings.api_key, "key-a");
        assert_eq!(ui.settings.model, "model-a");
    }

    #[test]
    fn undo_cannot_restore_another_providers_or_origins_credential() {
        fn edit_frame(ui: &mut SettingsUi, ctx: &egui::Context, time: f64, undo: bool) {
            let input = egui::RawInput {
                time: Some(time),
                events: if undo {
                    vec![egui::Event::Key {
                        key: egui::Key::Z,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::COMMAND,
                    }]
                } else {
                    vec![]
                },
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |egui_ui| {
                    ui.api_key_field(egui_ui).request_focus();
                });
            });
        }

        let ctx = egui::Context::default();
        let mut ui = draft();
        edit_frame(&mut ui, &ctx, 0.0, false);
        ui.settings.api_key = "edited-key-a".to_owned();
        edit_frame(&mut ui, &ctx, 1.0, false);
        edit_frame(&mut ui, &ctx, 2.0, false);
        ui.load_provider(
            &provider(2, "https://b.example/v1"),
            "key-b".to_owned(),
            "b".to_owned(),
            vec![],
        );
        edit_frame(&mut ui, &ctx, 3.0, false);
        edit_frame(&mut ui, &ctx, 4.0, true);
        assert_eq!(ui.settings.api_key, "key-b");

        ui.settings.base_url = "https://c.example/v1".to_owned();
        ui.reconcile_connection();
        edit_frame(&mut ui, &ctx, 5.0, false);
        edit_frame(&mut ui, &ctx, 6.0, true);
        assert!(ui.settings.api_key.is_empty());
    }

    #[test]
    fn returning_to_same_provider_and_configuration_rejects_old_generation() {
        let mut ui = draft();
        let old_scope = ui.model_scope().unwrap();
        let late = queue_request(&mut ui);
        ui.load_provider(
            &provider(2, "https://b.example/v1"),
            "key-b".to_owned(),
            "b".to_owned(),
            vec![],
        );
        assert!(late.send(Ok(vec!["late-a".to_owned()].into())).is_err());
        ui.load_provider(
            &provider(1, "https://a.example/v1"),
            "key-a".to_owned(),
            "manual-model".to_owned(),
            vec![],
        );
        queue_request(&mut ui)
            .send(Ok(vec!["current-a".to_owned()].into()))
            .unwrap();
        ui.poll_model_fetch();
        // An already-completed callback is also fenced even after A → B → A.
        ui.publish_model_result(old_scope, Ok(vec!["stale-a".to_owned()].into()));
        ui.publish_model_result(old_scope, Err("stale-error".to_owned()));
        assert_eq!(ui.fetched_models().unwrap().1, ["current-a"]);
        assert!(!ui.fetch_status.contains("stale"));
    }

    #[test]
    fn endpoint_and_key_edits_invalidate_lists_even_before_next_frame() {
        for change_endpoint in [false, true] {
            let mut ui = draft();
            queue_request(&mut ui)
                .send(Ok(vec!["old-model".to_owned()].into()))
                .unwrap();
            ui.poll_model_fetch();
            let late = queue_request(&mut ui);
            if change_endpoint {
                ui.settings.base_url.push('/');
            } else {
                ui.settings.api_key = "replacement-key".to_owned();
            }
            assert!(ui.fetched_models().is_none());
            ui.poll_model_fetch();
            assert!(late.send(Ok(vec!["late-model".to_owned()].into())).is_err());
            assert!(ui.model_list.is_none());
            assert!(ui.fetch_status.is_empty());
            assert_eq!(ui.settings.model, "manual-model");
        }
    }

    #[tokio::test]
    async fn changing_key_aborts_in_flight_task_without_network_or_storage() {
        let mut ui = draft();
        let _sender = queue_request(&mut ui);
        let task = tokio::spawn(std::future::pending::<()>());
        ui.fetch.as_mut().unwrap().task = Some(task.abort_handle());
        ui.settings.api_key.clear();
        ui.reconcile_connection();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(ui.fetch.is_none());
    }

    #[test]
    fn endpoint_edits_only_retain_credentials_for_same_valid_origin() {
        for endpoint in [
            "https://a.example/v1/",
            "https://A.EXAMPLE:443/v2",
            " https://a.example/api/v1 ",
        ] {
            let mut ui = draft();
            ui.settings.base_url = endpoint.to_owned();
            ui.reconcile_connection();
            assert_eq!(ui.settings.api_key, "key-a", "{endpoint}");
            assert!(ui.model_list.is_none());
        }
        for endpoint in [
            "https://b.example/v1",
            "http://a.example/v1",
            "https://a.example:8443/v1",
            "https://a.example.evil/v1",
            "https://a.example@b.example/v1",
            "not a URL",
            "",
        ] {
            let mut ui = draft();
            ui.settings.base_url = endpoint.to_owned();
            ui.reconcile_connection();
            assert!(ui.settings.api_key.is_empty(), "{endpoint}");
            assert!(ui.credential_cleared);
        }
        let mut ui = draft();
        ui.settings.base_url = "https://b.example/v1".to_owned();
        ui.reconcile_connection();
        ui.settings.api_key = "key-for-b".to_owned();
        ui.reconcile_connection();
        assert_eq!(ui.settings.api_key, "key-for-b");
    }

    #[test]
    fn subscription_backends_do_not_load_api_credentials_or_fetch_over_http() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        for kind in [ProviderKind::ChatGPT, ProviderKind::CodexAppServer] {
            let mut ui = draft();
            let mut backend = provider(2, "https://example.com/v1");
            backend.kind = kind;
            ui.load_provider(
                &backend,
                "must-not-use".to_owned(),
                "backend-model".to_owned(),
                vec![],
            );
            ui.start_model_fetch(runtime.handle(), egui::Context::default());
            assert!(ui.settings.api_key.is_empty());
            assert!(ui.fetch.is_none());
            assert_eq!(ui.provider.as_ref().unwrap().kind, kind);
            assert_eq!(ui.settings.model, "backend-model");
        }
    }

    #[test]
    fn invalid_unicode_accent_cannot_reach_the_theme_parser() {
        let ui = SettingsUi::from_settings(AppSettings {
            accent_hex: "#你好".to_owned(),
            ..AppSettings::default()
        });
        assert_eq!(
            theme::hex_to_color32(&ui.settings.accent_hex),
            theme::hex_to_color32(DEFAULT_ACCENT)
        );
        assert!(normalized_accent("#你好").is_none());
        assert_eq!(normalized_accent(" 0a84ff ").as_deref(), Some("#0A84FF"));
    }

    #[test]
    fn settings_serialization_never_writes_api_key_but_reads_legacy_value() {
        let mut settings = AppSettings::default();
        settings.api_key = "do-not-persist-this".to_owned();
        let encoded = serde_json::to_string(&settings).unwrap();
        assert!(!encoded.contains("do-not-persist-this"));
        assert!(!encoded.contains("api_key"));

        let legacy = serde_json::json!({
            "provider_name": "OpenAI",
            "base_url": "https://api.openai.com/v1",
            "api_key": "legacy-key",
            "model": "gpt-4o-mini",
            "appearance": "Auto",
            "accent_hex": "#0A84FF",
            "search_engine": "DuckDuckGo",
            "web_search_enabled": true
        });
        let decoded: AppSettings = serde_json::from_value(legacy).unwrap();
        assert_eq!(decoded.api_key, "legacy-key");
        assert_eq!(decoded.font_preferences, fonts::FontPreferences::default());
        assert_eq!(
            decoded.window_preferences,
            window_prefs::WindowPreferences::default()
        );
        assert_eq!(decoded.hotkey_binding, "Alt+Down");
    }

    #[test]
    fn new_preferences_round_trip_without_losing_settings_equality() {
        fn requires_eq<T: Eq>(_: &T) {}
        let mut settings = AppSettings::default();
        settings.font_preferences.english = "Test English".to_owned();
        settings.font_preferences.chinese = "測試中文字型".to_owned();
        settings.window_preferences.opacity_percent = 75;
        settings.window_preferences.fixed_x = -240;
        settings.window_preferences.fixed_y = 180;
        settings.hotkey_binding = "Ctrl+Shift+Space".to_owned();
        requires_eq(&settings);
        let restored: AppSettings =
            serde_json::from_slice(&serde_json::to_vec(&settings).unwrap()).unwrap();
        assert_eq!(restored, settings);
    }

    #[test]
    fn hotkey_editor_tracks_confirmed_results_not_per_frame_labels() {
        let mut ui = SettingsUi::from_settings(AppSettings {
            hotkey_binding: "Ctrl+Shift+Space".to_owned(),
            ..AppSettings::default()
        });
        assert_eq!(ui.hotkey_draft, "Ctrl+Shift+Space");
        ui.hotkey_draft = "invalid draft".to_owned();
        ui.set_hotkey_label(Some("Ctrl+Shift+Space"));
        assert_eq!(ui.hotkey_draft, "invalid draft");
        ui.set_hotkey_result(Some("Ctrl+Shift+Space"), Some("already in use".to_owned()));
        assert_eq!(ui.hotkey_draft, "invalid draft");
        assert_eq!(ui.hotkey_update_error.as_deref(), Some("already in use"));
        assert_eq!(ui.settings.hotkey_binding, "Ctrl+Shift+Space");
        assert_eq!(ui.saved_settings.hotkey_binding, "Ctrl+Shift+Space");
        ui.set_hotkey_result(Some("Alt+Down"), None);
        assert_eq!(ui.hotkey_draft, "Alt+Down");
        assert_eq!(ui.hotkey_label.as_deref(), Some("Alt+Down"));
        assert!(ui.hotkey_update_error.is_none());
        // Reporting a label alone never substitutes for the explicit commit.
        assert_eq!(ui.settings.hotkey_binding, "Ctrl+Shift+Space");
        ui.set_current_window_position(egui::pos2(-20.0, 150.0));
        assert_eq!(ui.current_window_position, Some(egui::pos2(-20.0, 150.0)));
    }

    #[test]
    fn confirmed_hotkey_persistence_never_commits_other_drafts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let saved = AppSettings {
            base_url: "https://saved.example/v1".to_owned(),
            model: "saved-model".to_owned(),
            ..AppSettings::default()
        };
        let mut stored = serde_json::to_value(&saved).unwrap();
        stored["future_preference"] = serde_json::json!({"keep": true});
        std::fs::write(&path, serde_json::to_vec(&stored).unwrap()).unwrap();
        let mut ui = SettingsUi::from_settings(saved);
        ui.settings.base_url = "https://unsaved.example/v1".to_owned();
        ui.settings.model = "unsaved-model".to_owned();
        ui.settings.api_key = "never-write-api-key".to_owned();
        ui.settings.search_engine_key = "never-write-search-key".to_owned();
        ui.settings.font_preferences.english = "Unsaved font".to_owned();
        ui.settings.window_preferences.opacity_percent = 65;
        ui.persist_hotkey_binding_at(&path, "Ctrl+Shift+Space")
            .unwrap();
        assert_eq!(ui.settings.hotkey_binding, "Ctrl+Shift+Space");
        assert_eq!(ui.saved_settings.hotkey_binding, "Ctrl+Shift+Space");
        let bytes = std::fs::read(&path).unwrap();
        let restored: AppSettings = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(restored.base_url, "https://saved.example/v1");
        assert_eq!(restored.model, "saved-model");
        assert_eq!(restored.hotkey_binding, "Ctrl+Shift+Space");
        assert_eq!(restored.font_preferences, fonts::FontPreferences::default());
        assert_eq!(
            restored.window_preferences,
            window_prefs::WindowPreferences::default()
        );
        let text = String::from_utf8(bytes).unwrap();
        assert!(!text.contains("never-write"));
        let restored: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(restored["future_preference"], stored["future_preference"]);
        std::fs::write(&path, b"malformed settings, do not truncate").unwrap();
        assert!(ui.persist_hotkey_binding_at(&path, "Alt+Down").is_err());
        assert_eq!(ui.settings.hotkey_binding, "Alt+Down");
        assert_eq!(ui.saved_settings.hotkey_binding, "Ctrl+Shift+Space");
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"malformed settings, do not truncate"
        );
    }

    #[test]
    fn commands_preserve_explicit_empty_prompt_and_search_secrets_are_not_json() {
        let settings = AppSettings {
            system_prompt: String::new(),
            search_engine_key: "search-secret-do-not-write".to_owned(),
            ..AppSettings::default()
        };
        let json = serde_json::to_string(&settings).unwrap();
        assert!(!json.contains("search-secret-do-not-write"));
        assert!(!json.contains("search_engine_key"));
        assert_eq!(
            serde_json::from_str::<AppSettings>(&json)
                .unwrap()
                .system_prompt,
            ""
        );
        let mut legacy = serde_json::to_value(AppSettings::default()).unwrap();
        legacy.as_object_mut().unwrap().remove("system_prompt");
        assert_eq!(
            serde_json::from_value::<AppSettings>(legacy)
                .unwrap()
                .system_prompt,
            default_system_prompt()
        );
    }

    #[test]
    fn only_exact_legacy_builtin_system_prompt_is_localized_on_load() {
        let mut value = serde_json::to_value(AppSettings::default()).unwrap();
        value["system_prompt"] =
            serde_json::json!(crate::chat::commands::LEGACY_DEFAULT_SYSTEM_PROMPT_EN);
        assert_eq!(
            serde_json::from_value::<AppSettings>(value.clone())
                .unwrap()
                .system_prompt,
            default_system_prompt()
        );
        for prompt in ["My custom English system prompt", "", "自訂提示詞"] {
            value["system_prompt"] = serde_json::json!(prompt);
            assert_eq!(
                serde_json::from_value::<AppSettings>(value.clone())
                    .unwrap()
                    .system_prompt,
                prompt
            );
        }
    }

    #[tokio::test]
    async fn actual_settings_fetch_retains_models_metadata_and_probes_ollama() {
        use wiremock::matchers::{body_json, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{"id":"vision-model", "architecture":{"input_modalities":["text", "image"]}}]
            }))).mount(&server).await;
        Mock::given(method("GET"))
            .and(path("/api/version"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"version":"0.9"})),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/show"))
            .and(body_json(serde_json::json!({"model":"vision-model"})))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"capabilities":["vision"]})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let endpoint = format!("{}/v1", server.uri());
        let catalog = fetch_catalog("Custom", &endpoint, "").await.unwrap();
        let mut draft = draft();
        let scope = draft.model_scope().unwrap();
        draft.publish_model_result(scope, Ok(catalog));
        let (id, caps) = draft.fetched_capabilities().unwrap();
        assert_eq!(Some(id), draft.selected_provider_id());
        assert_eq!(caps["vision-model"].images, Some(true));
        assert_eq!(caps["vision-model"].files, Some(false));
        draft.settings.base_url.push('/');
        assert!(draft.fetched_capabilities().is_none());
    }

    #[test]
    fn settings_actions_are_consumed_once_and_keep_catalog_identity() {
        let mut draft = draft();
        let id = draft.selected_provider_id().unwrap();
        draft.actions.push(SettingsAction::RemoveProvider(id));
        assert!(
            matches!(draft.take_actions().as_slice(), [SettingsAction::RemoveProvider(owner)] if *owner == id)
        );
        assert!(draft.take_actions().is_empty());
        draft
            .actions
            .push(SettingsAction::ChangeHotkey("Alt+Down".to_owned()));
        assert!(matches!(
            draft.take_actions().as_slice(),
            [SettingsAction::ChangeHotkey(binding)] if binding == "Alt+Down"
        ));
        assert!(draft.take_actions().is_empty());
    }

    #[test]
    fn command_autosave_changes_only_its_own_preference_and_preserves_unknown_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let mut stored = serde_json::to_value(AppSettings::default()).unwrap();
        stored["base_url"] = "https://saved.example/v1".into();
        stored["future_preference"] = serde_json::json!({"keep":true});
        std::fs::write(&path, serde_json::to_vec(&stored).unwrap()).unwrap();
        save_system_prompt_only(&path, "").unwrap();
        let result: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(result["system_prompt"], "");
        assert_eq!(result["base_url"], "https://saved.example/v1");
        assert_eq!(result["future_preference"], stored["future_preference"]);
        save_string_preference_only(&path, "codex_executable_path", "C:\\tools\\codex.cmd")
            .unwrap();
        let result: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(result["codex_executable_path"], "C:\\tools\\codex.cmd");
        assert_eq!(result["base_url"], "https://saved.example/v1");
        let new_path = dir.path().join("first-launch.json");
        save_system_prompt_only(&new_path, "custom").unwrap();
        assert_eq!(
            serde_json::from_slice::<AppSettings>(&std::fs::read(new_path).unwrap())
                .unwrap()
                .system_prompt,
            "custom"
        );
        std::fs::write(&path, b"malformed settings, do not truncate").unwrap();
        assert!(save_system_prompt_only(&path, "replace").is_err());
        assert_eq!(
            std::fs::read(path).unwrap(),
            b"malformed settings, do not truncate"
        );
    }

    #[test]
    fn general_preference_autosave_never_commits_provider_drafts_or_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let saved = AppSettings {
            base_url: "https://saved.example/v1".to_owned(),
            model: "saved-model".to_owned(),
            hotkey_binding: "Ctrl+Shift+Space".to_owned(),
            ..AppSettings::default()
        };
        std::fs::write(&path, serde_json::to_vec(&saved).unwrap()).unwrap();
        let mut draft = AppSettings {
            base_url: "https://unsaved.example/v1".to_owned(),
            api_key: "never-write-api-key".to_owned(),
            search_engine_key: "never-write-search-key".to_owned(),
            model: "unsaved-model".to_owned(),
            appearance: Appearance::Dark,
            streaming_mode: crate::chat::store::StreamingMode::PerSentence,
            bubble_style: theme::BubbleStyle::AccentFill,
            web_search_enabled: false,
            hotkey_binding: "unapplied-invalid-hotkey".to_owned(),
            ..saved
        };
        draft.font_preferences.english = "Test English".to_owned();
        draft.font_preferences.chinese = "測試中文字型".to_owned();
        draft.chat_font_size = 22;
        draft.window_preferences.opacity_percent = 70;
        draft.window_preferences.fixed_x = -100;
        draft.window_preferences.fixed_y = 200;
        let fields = general_preferences(&draft);
        assert!(!fields.contains_key("api_key"));
        assert!(!fields.contains_key("search_engine_key"));
        assert!(!fields.contains_key("base_url"));
        assert!(!fields.contains_key("model"));
        assert!(!fields.contains_key("hotkey_binding"));
        save_preference_values_only(&path, fields.clone()).unwrap();
        let bytes = std::fs::read(path).unwrap();
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(!text.contains("never-write"));
        let restored: AppSettings = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(restored.base_url, "https://saved.example/v1");
        assert_eq!(restored.model, "saved-model");
        assert_eq!(restored.appearance, Appearance::Dark);
        assert_eq!(
            restored.streaming_mode,
            crate::chat::store::StreamingMode::PerSentence
        );
        assert!(!restored.web_search_enabled);
        assert_eq!(restored.bubble_style, theme::BubbleStyle::AccentFill);
        assert_eq!(restored.font_preferences, draft.font_preferences);
        assert_eq!(restored.chat_font_size, 22);
        assert_eq!(restored.window_preferences, draft.window_preferences);
        assert_eq!(restored.hotkey_binding, "Ctrl+Shift+Space");

        // First-launch autosave also starts from defaults, not API/hotkey drafts.
        let new_path = dir.path().join("first-launch.json");
        save_preference_values_only(&new_path, fields).unwrap();
        let restored: AppSettings =
            serde_json::from_slice(&std::fs::read(new_path).unwrap()).unwrap();
        assert_eq!(restored.base_url, AppSettings::default().base_url);
        assert_eq!(restored.model, "");
        assert_eq!(restored.hotkey_binding, "Alt+Down");
        assert!(restored.api_key.is_empty());
        assert!(restored.search_engine_key.is_empty());
        assert_eq!(restored.font_preferences, draft.font_preferences);
        assert_eq!(restored.chat_font_size, 22);
        assert_eq!(restored.window_preferences, draft.window_preferences);
    }

    #[test]
    fn chat_font_size_missing_fields_default_and_out_of_range_is_bounded() {
        let mut old = serde_json::to_value(AppSettings::default()).unwrap();
        old.as_object_mut().unwrap().remove("chat_font_size");
        assert_eq!(
            serde_json::from_value::<AppSettings>(old.clone())
                .unwrap()
                .chat_font_size,
            15
        );
        for (stored, expected) in [(0, 12), (12, 12), (19, 19), (24, 24), (65535, 24)] {
            old["chat_font_size"] = stored.into();
            let decoded: AppSettings = serde_json::from_value(old.clone()).unwrap();
            assert_eq!(decoded.chat_font_size, expected);
            let encoded = serde_json::to_value(decoded).unwrap();
            assert_eq!(encoded["chat_font_size"], expected);
        }
    }

    #[test]
    fn chat_font_size_preference_preview_is_local_and_responsive() {
        for width in [300.0, 380.0, 520.0] {
            for dark in [false, true] {
                for size in [12, 15, 24] {
                    let ctx = egui::Context::default();
                    theme::apply(&ctx, if dark { "Dark" } else { "Light" }, DEFAULT_ACCENT);
                    let mut draft = draft();
                    draft.settings.chat_font_size = size;
                    let output = ctx.run(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(width, 800.0),
                            )),
                            ..Default::default()
                        },
                        |ctx| {
                            egui::CentralPanel::default().show(ctx, |ui| {
                                let before = ui.style().text_styles.clone();
                                draft.chat_font_size_preferences(ui);
                                assert_eq!(ui.style().text_styles, before);
                                assert!(ui.min_rect().right() <= width + 0.5);
                            });
                        },
                    );
                    assert_eq!(draft.settings.chat_font_size, size);
                    assert!(draft.take_actions().is_empty());
                    assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
                        egui::Shape::Text(text) if text.galley.job.text.contains("The quick brown fox")
                            && text.galley.job.sections.iter().all(|section| (section.format.font_id.size - size as f32).abs() < 0.1))));
                }
            }
        }
    }

    #[test]
    fn actual_font_size_slider_keyboard_edit_changes_preview_and_only_general_preferences() {
        let ctx = egui::Context::default();
        theme::apply(&ctx, "Light", DEFAULT_ACCENT);
        let mut draft = draft();
        draft.settings.base_url = "https://unsubmitted.invalid/v1".into();
        let before = draft.settings.clone();
        let frame = |events| egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(520.0, 700.0),
            )),
            events,
            ..Default::default()
        };
        let _ = ctx.run(frame(vec![]), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                draft.chat_font_size_preferences(ui).request_focus();
            });
        });
        let output = ctx.run(
            frame(vec![egui::Event::Key {
                key: egui::Key::ArrowRight,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }]),
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    draft.chat_font_size_preferences(ui);
                });
            },
        );
        assert_eq!(draft.settings.chat_font_size, 16);
        assert_eq!(draft.settings.base_url, before.base_url);
        assert!(
            draft.take_actions().is_empty(),
            "size editing must not Apply API or hotkeys"
        );
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Text(text) if text.galley.job.text.contains("The quick brown fox")
                && text.galley.job.sections.iter().all(|section| (section.format.font_id.size - 16.0).abs() < 0.1))));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let saved = AppSettings::default();
        std::fs::write(&path, serde_json::to_vec(&saved).unwrap()).unwrap();
        save_preference_values_only(&path, general_preferences(&draft.settings)).unwrap();
        let stored: AppSettings = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(stored.chat_font_size, 16);
        assert_eq!(stored.base_url, saved.base_url);
    }
}
