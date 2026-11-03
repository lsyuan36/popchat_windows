//! PopChat Windows 浮窗殼（eframe/egui）：對應原版 AppDelegate + PanelController +
//! ChatView 的 Windows 實作。
//!
//! - 視窗 580x440、置頂、無邊框（可拖曳見 TopBar 的 drag handle）
//! - Alt+Down：由 [`hotkey::HotkeyManager`] 註冊全域熱鍵，visibility-driver
//!   即使面板移出螢幕也能切換顯示；Alt+Space 保留給 Windows 系統視窗選單。
//! - Esc：隱藏（對應 FloatingPanel.onCancel → dismiss）
//! - 失焦自動隱藏（對應 windowDidResignKey；需先聚焦過才生效，避免 show 後誤殺）
//! - 聊天：BottomBar Enter/Send 走 [`ChatStore::send`] 驗證鏈（slash 展開、
//!   無 key 警告列），`Ready` 經 tokio 串流 [`client::run`]（reqwest SSE），
//!   [`ChatStreamEvent`] 由背景 task 經 std mpsc 送回 UI 執行緒即時更新。

// A desktop application, not a console executable. Keep test harness output.
#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]

use eframe::egui;
use popchat_windows::chat::client::{self, ChatStreamEvent};
use popchat_windows::chat::commands::ShortcutStore as CommandsShortcutStore;
use popchat_windows::chat::providers::{ContentCapability, Provider, ProviderKind, ProviderStore};
use popchat_windows::chat::store::{
    ActiveTurn, Attachment, ChatMessage, ChatRole, ChatStore, ConversationMeta, ConversationStore,
    OutgoingTurn, ResolvedConversation, SearchEngine, SecretStore, SendOptions, SendOutcome,
    ShortcutStore,
};
use popchat_windows::chat::{attach, backends, chatgpt_auth, clipboard, codex};
use popchat_windows::instance::{InstanceGuard, InstanceLaunch, ReopenRequest};
use popchat_windows::logf;
use popchat_windows::ui::{
    chat_layout,
    composer::{Composer, ComposerAction},
    controls,
    file_drop::FileDropFeedback,
    fonts::{self, install_cjk_font},
    history,
    hotkey::{HotkeyEvent, HotkeyManager},
    icons::{self, Icon},
    layout::ViewportSizing,
    panel::{self, MathTextures, PanelGeometry},
    paste::{self, NativePaste},
    reasoning::{self, WaitingView},
    settings::{SettingsAction, SettingsUi},
    surface::{self, CARD_MARGIN},
    switcher, theme, transition,
    tray::{PopchatTray, TrayEvent},
    window_prefs,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::{collections::HashMap, path::PathBuf};
use uuid::Uuid;

#[path = "ui/visibility.rs"]
mod visibility;
use visibility::{ViewportSnapshot, VisibilityController};

/// 背景串流轉發：tokio task 把 [`client::run`] 的事件經 std mpsc 送回 UI 執行緒。
/// UI 每幀 `try_recv` 排空，`Done`/`Error`/`ContentRejected` 到達即
/// [`ChatStore::finish_turn`] 結束；Stop 按鈕走 `handle.abort()` 取消。
struct ActiveStream {
    rx: std::sync::mpsc::Receiver<ChatStreamEvent>,
    turn: ActiveTurn,
    handle: tokio::task::JoinHandle<()>,
}

enum AppEvent {
    HistoryIndex(u64, HashMap<Uuid, String>),
    Loaded(u64, Uuid, Option<ResolvedConversation>),
    Attachments(u64, Vec<Attachment>, Vec<String>, bool),
    SignedIn(u64, Result<chatgpt_auth::Tokens, String>),
    SignedOut(Result<(), String>),
    Codex(
        u64,
        Result<
            popchat_windows::chat::providers::CodexInspection,
            popchat_windows::chat::providers::CodexFailure,
        >,
    ),
    Models(
        Uuid,
        u64,
        Result<
            (
                Vec<String>,
                HashMap<String, popchat_windows::chat::providers::ModelCapabilities>,
            ),
            String,
        >,
    ),
}

#[derive(Default)]
struct FindState {
    open: bool,
    query: String,
    index: usize,
    focus: bool,
    scroll: bool,
}

#[derive(Clone, Copy)]
enum MessageAction {
    Retry,
    Edit,
    Fork(Uuid),
}

enum PreparedMessageAction {
    None,
    Retry(String, Vec<Attachment>),
    Edit(ChatMessage),
    Forked,
}

fn prepare_message_action(chat: &mut ChatStore, action: MessageAction) -> PreparedMessageAction {
    match action {
        MessageAction::Retry => chat
            .retry_last_turn()
            .map(|(text, attachments)| PreparedMessageAction::Retry(text, attachments))
            .unwrap_or(PreparedMessageAction::None),
        MessageAction::Edit => chat
            .take_last_user_message_for_editing()
            .map(PreparedMessageAction::Edit)
            .unwrap_or(PreparedMessageAction::None),
        MessageAction::Fork(id) => {
            if !chat.messages.iter().any(|message| message.id == id) {
                return PreparedMessageAction::None;
            }
            chat.fork(id);
            PreparedMessageAction::Forked
        }
    }
}

/// This dialog only returns a decision; it never saves drafts, downloads,
/// starts a helper or closes a viewport by itself.
fn update_confirmation_dialog(ctx: &egui::Context, version: &str, streaming: bool) -> Option<bool> {
    let response = egui::Modal::new(egui::Id::new("install-update-confirmation")).show(ctx, |ui| {
        ui.set_width((ctx.screen_rect().width() - 64.0).clamp(200.0, 400.0));
        ui.heading(format!("安裝 PopChat {version}？"));
        ui.add_space(8.0);
        ui.add(egui::Label::new("將關閉目前視窗，安裝已驗證的更新並重新啟動。設定、服務、金鑰與歷史會保留。").wrap());
        ui.add_space(6.0);
        ui.add(egui::Label::new(egui::RichText::new("尚未送出的訊息與未儲存的 API 編輯不會自動保存；請先返回完成編輯。會立即停止仍在生成中的回覆。")
            .color(ui.visuals().warn_fg_color)).wrap());
        if streaming {
            ui.add(egui::Label::new("目前正在生成回覆。確認安裝後將停止生成。").wrap());
        }
        ui.add_space(12.0);
        ui.horizontal_wrapped(|ui| {
            if ui.button("返回編輯").clicked() { return Some(false); }
            if ui.button("安裝並重新啟動").clicked() { return Some(true); }
            None
        }).inner
    });
    if response.should_close() {
        Some(false)
    } else {
        response.inner
    }
}

struct PopChatApp {
    chat: ChatStore,
    providers: ProviderStore,
    secrets: SecretStore,
    shortcuts: ShortcutStore,
    runtime: tokio::runtime::Runtime,
    active_stream: Option<ActiveStream>,
    input: String,
    settings_open: bool,
    settings_ui: SettingsUi,
    settings_was_open: bool,
    page_transition: transition::PageTransition,
    height_transition: transition::WindowHeightTransition,
    update_confirmation: Option<popchat_windows::updater::PreparedUpdate>,
    update_install: Option<popchat_windows::updater::InstallSession>,
    legacy_migration_error: Option<String>,
    secret_migration_warning: Option<String>,
    geometry: PanelGeometry,
    /// UI/熱鍵/托盤的狀態與 viewport 命令由同一把鎖序列化。
    visibility: Arc<VisibilityController>,
    /// 本幀 UI 動作的 generation；舊幀不得關掉新一次熱鍵叫出的面板。
    visibility_generation: u64,
    _visibility_driver: Option<VisibilityDriver>,
    tray: Option<PopchatTray>,
    tray_error: Option<String>,
    hotkey: Option<HotkeyManager>,
    hotkey_error: Option<String>,
    /// ⌘P 對應的釘選：true 時失焦不隱藏（原版 pin 語義）。
    pinned: bool,
    /// 首幀字型自檢已記 log（run() 後才能查 fontsface）。
    font_logged: bool,
    composer: Composer,
    math: MathTextures,
    backend: Arc<backends::DelegatedBackend>,
    events_tx: std::sync::mpsc::Sender<AppEvent>,
    events_rx: std::sync::mpsc::Receiver<AppEvent>,
    history_open: bool,
    history_query: String,
    history_selected: usize,
    history_focus: bool,
    history_index: HashMap<Uuid, String>,
    history_generation: u64,
    load_generation: u64,
    loading_conversation: bool,
    draft_generation: u64,
    attachment_tasks: usize,
    picker_open: bool,
    find: FindState,
    ui_error: Option<String>,
    auth_generation: u64,
    auth_task: Option<tokio::task::JoinHandle<()>>,
    auth_error: Option<String>,
    auth_signing_out: bool,
    signed_in: bool,
    auth_email: Option<String>,
    auth_plan: Option<String>,
    codex_task: Option<tokio::task::JoinHandle<()>>,
    codex_path: Option<PathBuf>,
    model_generations: HashMap<Uuid, u64>,
    ime_composing: bool,
    _instance: Arc<InstanceGuard>,
    dismiss_hint: Arc<AtomicBool>,
    first_run_hint_due: Option<std::time::Instant>,
    viewport_sizing: ViewportSizing,
    monitor_work_areas: Vec<egui::Rect>,
    geometry_generation: u64,
    native_resizing: bool,
    geometry_save_due: Option<std::time::Instant>,
    size_limits: Option<(egui::Vec2, egui::Vec2)>,
    ever_shown: bool,
    waiting: WaitingView,
    copied_messages: HashMap<Uuid, std::time::Instant>,
    native_paste: Arc<NativePaste>,
    file_drop_feedback: FileDropFeedback,
    attachment_drop_frame: Option<u64>,
    attachment_paste_frame: Option<u64>,
    file_paste_consumed_frame: Option<u64>,
    attachment_interaction_until: Option<std::time::Instant>,
}

impl PopChatApp {
    fn new(
        cc: &eframe::CreationContext<'_>,
        instance: Arc<InstanceGuard>,
        first_launch: bool,
        native_paste: Arc<NativePaste>,
    ) -> Self {
        native_paste.bind(cc);
        let geometry = PanelGeometry::load();
        let mut settings_ui = SettingsUi::load();
        // 首幀前先套一次（之後每幀由 update 經 theme::apply 跟隨設定）。
        let appearance0 = match settings_ui.settings.appearance {
            popchat_windows::ui::settings::Appearance::Light => "light",
            popchat_windows::ui::settings::Appearance::Dark => "dark",
            popchat_windows::ui::settings::Appearance::Auto => "auto",
        };
        theme::apply(&cc.egui_ctx, appearance0, &settings_ui.settings.accent_hex);
        let font_error =
            fonts::apply_font_preferences(&cc.egui_ctx, &settings_ui.settings.font_preferences)
                .err();
        if font_error.is_some() {
            install_cjk_font(&cc.egui_ctx);
        }

        let mut tray_error = None;
        let (tray, tray_drv_rx) = match PopchatTray::new() {
            Ok((t, rx)) => (Some(t), Some(rx)),
            Err(e) => {
                tray_error = Some(format!("{e:#}"));
                (None, None)
            }
        };
        let mut hotkey_error = None;
        let (hotkey, hotkey_drv_rx) =
            match HotkeyManager::new_with_binding(&settings_ui.settings.hotkey_binding) {
                Ok((h, rx)) => (Some(h), Some(rx)),
                Err(e) => {
                    // 極少見：連管理器都建不起來（不是佔用問題）。
                    hotkey_error = Some(format!("{e:#}"));
                    (None, None)
                }
            };
        if let Some(err) = hotkey.as_ref().and_then(|h| h.register_error()) {
            hotkey_error = Some(err.to_owned());
        }

        let chat = ChatStore::new();
        let login_item = std::env::args().any(|arg| arg == "--login-item");
        let settings_requested = std::env::args().any(|arg| arg == "--settings");
        let preview = std::env::var_os("POPCHAT_PINNED").is_some() && !login_item;
        let launch_visible = popchat_windows::startup::should_present_launch(
            first_launch,
            login_item,
            settings_requested,
        ) || preview;
        let monitor_work_areas = panel::available_monitor_rects(cc.egui_ctx.pixels_per_point());
        let mut launch_geometry = geometry.clone();
        if settings_requested {
            launch_geometry.expanded_height = 620.0;
        }
        let initial = window_prefs::placement_rect(
            &launch_geometry,
            &settings_ui.settings.window_preferences,
            !chat.messages.is_empty() || settings_requested,
            if hotkey_error.is_some() || tray_error.is_some() {
                320.0
            } else {
                248.0
            },
            egui::vec2(24.0, 24.0),
            &monitor_work_areas,
            panel::desktop_cursor_position(cc.egui_ctx.pixels_per_point()),
        );
        let visibility = Arc::new(VisibilityController::new_at(
            std::time::Instant::now(),
            initial.map_or(egui::pos2(100.0, 100.0), |rect| rect.min),
        ));
        if !launch_visible {
            visibility.dismiss(
                &cc.egui_ctx,
                visibility.generation(),
                ViewportSnapshot::default(),
            );
        }
        if let Some(initial) = initial {
            visibility.restore_position(&cc.egui_ctx, visibility.generation(), initial.min);
            cc.egui_ctx
                .send_viewport_cmd(egui::ViewportCommand::InnerSize(initial.size()));
        }
        let dismiss_hint = Arc::new(AtomicBool::new(false));
        let hotkey_suppressed = hotkey.as_ref().map_or_else(
            || Arc::new(AtomicBool::new(false)),
            HotkeyManager::suppression_flag,
        );
        popchat_windows::ui::hotkey_recorder::set_suppression_flag(
            &cc.egui_ctx,
            hotkey_suppressed.clone(),
        );
        let visibility_driver = Some(spawn_visibility_driver(
            hotkey_drv_rx,
            tray_drv_rx,
            cc.egui_ctx.clone(),
            visibility.clone(),
            instance.clone(),
            dismiss_hint.clone(),
            hotkey_suppressed,
        ));

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime for chat streaming");

        let mut secrets = SecretStore::new();
        let secret_migration_warning = secrets.migration_error().map(str::to_owned);
        let mut providers = ProviderStore::load(&secrets);
        let shortcuts = ShortcutStore::load();
        // Migrate legacy credentials only to a matching API endpoint, never
        // blindly into the selected ChatGPT/Codex/provider identity.
        let legacy_migration_error =
            migrate_legacy_settings(&mut settings_ui, &mut providers, &mut secrets).err();
        settings_ui.sync_provider(&providers, &secrets);
        settings_ui.set_hotkey_label(hotkey.as_ref().and_then(|h| h.label()));
        let backend = Arc::new(backends::DelegatedBackend::new());
        let codex_path = optional_path(&settings_ui.settings.codex_executable_path);
        backend.set_codex_executable(codex_path.clone());
        client::set_delegated_backend(backend.clone());
        let (events_tx, events_rx) = std::sync::mpsc::channel();
        let auth = chatgpt_auth::NativeTokenStore::default().load();
        // 測試 hook：POPCHAT_PINNED=1 啟動即釘選（終端啟動時焦點不穩，
        // 釘選可讓視窗不自動隱藏、方便截圖驗證）；正常使用不受影響。
        let pinned = std::env::var("POPCHAT_PINNED").is_ok();

        Self {
            chat,
            providers,
            secrets,
            shortcuts,
            runtime,
            active_stream: None,
            input: String::new(),
            settings_open: std::env::args().any(|arg| arg == "--settings"),
            settings_ui,
            settings_was_open: false,
            page_transition: transition::PageTransition::default(),
            height_transition: transition::WindowHeightTransition::default(),
            update_confirmation: None,
            update_install: None,
            legacy_migration_error,
            secret_migration_warning,
            geometry,
            visibility,
            visibility_generation: 0,
            _visibility_driver: visibility_driver,
            tray,
            tray_error,
            hotkey,
            hotkey_error,
            pinned,
            font_logged: false,
            composer: Composer::new(),
            math: MathTextures::new(),
            backend,
            events_tx,
            events_rx,
            history_open: false,
            history_query: String::new(),
            history_selected: 0,
            history_focus: false,
            history_index: HashMap::new(),
            history_generation: 0,
            load_generation: 0,
            loading_conversation: false,
            draft_generation: 0,
            attachment_tasks: 0,
            picker_open: false,
            find: FindState::default(),
            ui_error: font_error,
            auth_generation: 0,
            auth_task: None,
            auth_error: None,
            auth_signing_out: false,
            signed_in: auth.is_some(),
            auth_email: auth.as_ref().and_then(|tokens| tokens.email.clone()),
            auth_plan: auth.as_ref().and_then(|tokens| tokens.plan.clone()),
            codex_task: None,
            codex_path,
            model_generations: HashMap::new(),
            ime_composing: false,
            _instance: instance,
            dismiss_hint,
            first_run_hint_due: (first_launch && !login_item)
                .then(|| std::time::Instant::now() + std::time::Duration::from_millis(350)),
            viewport_sizing: ViewportSizing::default(),
            monitor_work_areas,
            geometry_generation: 0,
            native_resizing: false,
            geometry_save_due: None,
            size_limits: None,
            ever_shown: launch_visible,
            waiting: WaitingView::default(),
            copied_messages: HashMap::new(),
            native_paste,
            file_drop_feedback: FileDropFeedback::default(),
            attachment_drop_frame: None,
            attachment_paste_frame: None,
            file_paste_consumed_frame: None,
            attachment_interaction_until: None,
        }
    }

    fn retry_hotkey(&mut self) {
        let Some(hotkey) = &mut self.hotkey else {
            return;
        };
        // 同一個 manager 重試（監聽執行緒與 driver channel 不變，
        // 成功後事件自動送達 driver）。已有生效鍵就不重試。
        if hotkey.label().is_some() {
            return;
        }
        match hotkey.try_register() {
            Ok(_) => {
                self.hotkey_error = None;
            }
            Err(e) => {
                self.hotkey_error = Some(format!("{e:#}"));
            }
        }
        self.settings_ui.set_hotkey_label(hotkey.label());
    }

    fn invalidate_load(&mut self) {
        self.load_generation += 1;
        self.loading_conversation = false;
    }

    fn poll_update_install(&mut self, ctx: &egui::Context) {
        let Some(session) = &mut self.update_install else {
            return;
        };
        match session.poll() {
            Ok(true) => {
                // The helper already validated both executables, the target
                // identity and staging hashes. It will wait for this PID to
                // fully exit before replacing anything or opening a new UI.
                self.update_install = None;
                self.visibility.close(ctx);
            }
            Ok(false) => ctx.request_repaint_after(std::time::Duration::from_millis(50)),
            Err(error) => {
                self.update_install = None;
                self.settings_ui.set_update_install_message(
                    Some(format!(
                        "更新安裝尚未開始：{error:#}。目前程式未關閉，請重試或從發行頁手動下載。"
                    )),
                    false,
                );
            }
        }
    }

    fn show_update_confirmation(&mut self, ctx: &egui::Context) {
        let Some(prepared) = &self.update_confirmation else {
            return;
        };
        let decision = update_confirmation_dialog(ctx, prepared.version(), self.chat.is_streaming);
        match decision {
            Some(false) => {
                self.update_confirmation = None;
                self.settings_ui.set_update_install_message(None, false);
            }
            Some(true) => {
                let prepared = self
                    .update_confirmation
                    .take()
                    .expect("confirmation owns the staged update");
                match popchat_windows::updater::begin_install(&prepared) {
                    Ok(session) => {
                        self.update_install = Some(session);
                        self.settings_ui.set_update_install_message(
                            Some("正在驗證更新助手；確認可安全安裝後才會關閉目前視窗…".into()),
                            true,
                        );
                        ctx.request_repaint();
                    }
                    Err(error) => self.settings_ui.set_update_install_message(
                        Some(format!("無法準備更新：{error:#}")),
                        false,
                    ),
                }
            }
            None => {}
        }
    }

    fn reset_draft(&mut self) {
        self.draft_generation += 1;
        self.attachment_tasks = 0;
        self.picker_open = false;
        self.file_drop_feedback = FileDropFeedback::default();
        self.attachment_interaction_until = None;
        self.composer.clear();
        self.composer.request_focus();
    }

    fn new_chat(&mut self) {
        self.stop_active_stream();
        self.invalidate_load();
        self.chat.new_chat();
        self.reset_draft();
        self.history_open = false;
        self.find = FindState::default();
        self.settings_open = false;
    }

    fn open_history(&mut self, ctx: &egui::Context) {
        self.history_open = !self.history_open;
        if !self.history_open {
            self.composer.request_focus();
            return;
        }
        self.settings_open = false;
        self.history_query.clear();
        self.history_selected = 0;
        self.history_focus = true;
        self.history_index.clear();
        self.refresh_history_index(ctx);
    }

    fn refresh_history_index(&mut self, ctx: &egui::Context) {
        self.history_generation += 1;
        let generation = self.history_generation;
        let tx = self.events_tx.clone();
        let ctx = ctx.clone();
        self.runtime.spawn_blocking(move || {
            let corpus = ConversationStore::full_text_index();
            let _ = tx.send(AppEvent::HistoryIndex(generation, corpus));
            ctx.request_repaint();
        });
    }

    fn load_chat(&mut self, id: Uuid, ctx: &egui::Context) {
        self.history_open = false;
        if id == self.chat.conversation_id {
            self.invalidate_load();
            return;
        }
        self.stop_active_stream();
        self.chat.persist();
        self.load_generation += 1;
        let generation = self.load_generation;
        self.loading_conversation = true;
        let tx = self.events_tx.clone();
        let ctx = ctx.clone();
        self.runtime.spawn_blocking(move || {
            let loaded = ConversationStore::load_resolved(id);
            let _ = tx.send(AppEvent::Loaded(generation, id, loaded));
            ctx.request_repaint();
        });
    }

    fn delete_chat(&mut self, id: Uuid, ctx: &egui::Context) {
        self.stop_active_stream();
        self.invalidate_load();
        let current = id == self.chat.conversation_id;
        // Use the core operation: it materializes children before deleting
        // and realigns a live fork whose parent has just been removed.
        self.chat.delete_conversation(id);
        if current {
            self.reset_draft();
            self.find = FindState::default();
        }
        self.refresh_history_index(ctx);
    }

    fn message_action(&mut self, action: MessageAction) {
        self.stop_active_stream();
        self.invalidate_load();
        match prepare_message_action(&mut self.chat, action) {
            PreparedMessageAction::Retry(text, attachments) => self.send_payload(text, attachments),
            PreparedMessageAction::Edit(message) => {
                self.reset_draft();
                self.composer.set_draft(message.text, message.attachments);
            }
            PreparedMessageAction::Forked => {
                self.reset_draft();
                self.find = FindState::default();
            }
            PreparedMessageAction::None => {}
        }
    }

    fn pick_attachments(&mut self, ctx: &egui::Context) {
        if self.picker_open {
            return;
        }
        self.picker_open = true;
        self.attachment_tasks += 1;
        let generation = self.draft_generation;
        let tx = self.events_tx.clone();
        let ctx = ctx.clone();
        self.runtime.spawn_blocking(move || {
            let paths = rfd::FileDialog::new()
                .set_title("加入附件")
                .pick_files()
                .unwrap_or_default();
            let (attachments, errors) = load_attachment_paths(paths);
            let _ = tx.send(AppEvent::Attachments(generation, attachments, errors, true));
            ctx.request_repaint();
        });
    }

    fn load_attachments(&mut self, paths: Vec<PathBuf>, clipboard: bool, ctx: &egui::Context) {
        self.queue_attachment_load(ctx, move || {
            let (mut attachments, mut errors) = load_attachment_paths(paths);
            if clipboard {
                match attach::load_clipboard_store_attachment() {
                    Ok(Some(attachment)) => attachments.push(attachment),
                    Ok(None) => {}
                    Err(error) => errors.push(format!("無法貼上圖片：{error:#}")),
                }
            }
            (attachments, errors)
        });
    }

    fn queue_attachment_load(
        &mut self,
        ctx: &egui::Context,
        load: impl FnOnce() -> (Vec<Attachment>, Vec<String>) + Send + 'static,
    ) {
        self.attachment_tasks += 1;
        let generation = self.draft_generation;
        let tx = self.events_tx.clone();
        let ctx = ctx.clone();
        self.runtime.spawn_blocking(move || {
            let (attachments, errors) = load();
            let _ = tx.send(AppEvent::Attachments(
                generation,
                attachments,
                errors,
                false,
            ));
            ctx.request_repaint();
        });
    }

    fn handle_attachment_input(&mut self, ctx: &egui::Context, native_paste: bool) {
        if self.settings_open || self.history_open || self.loading_conversation {
            return;
        }
        let frame = ctx.cumulative_frame_nr();
        let files = ctx.input_mut(|input| std::mem::take(&mut input.raw.dropped_files));
        if !files.is_empty() && self.attachment_drop_frame != Some(frame) {
            self.attachment_drop_frame = Some(frame);
            self.attachment_interaction_until =
                Some(std::time::Instant::now() + std::time::Duration::from_millis(600));
            self.queue_attachment_load(ctx, move || load_dropped_attachments(files));
            // A deliberate drop is a user request to edit this draft. Use the
            // same serialized/native focus path as show, never synthetic keys.
            self.visibility
                .activate_visible(ctx, std::time::Instant::now());
            self.composer.request_focus();
        }
        let requested = native_paste
            || ctx.input(|input| {
                input
                    .events
                    .iter()
                    .any(|event| matches!(event, egui::Event::Paste(_)))
                    || (input.key_pressed(egui::Key::V) && input.modifiers.ctrl)
            });
        let paste_allowed = !ctx.wants_keyboard_input() || self.composer.has_input_focus();
        if requested && paste_allowed && self.attachment_paste_frame != Some(frame) {
            self.attachment_paste_frame = Some(frame);
            match clipboard::file_paths() {
                Ok(Some(paths)) => {
                    self.file_paste_consumed_frame = Some(frame);
                    self.load_attachments(paths, false, ctx);
                    self.composer.request_focus();
                }
                Ok(None) if clipboard::image_available() => {
                    // Normal text paste stays in egui; an image-only clipboard
                    // is reached by the native observer even without Paste(text).
                    self.load_attachments(Vec::new(), true, ctx);
                }
                Ok(None) => {}
                Err(error) => {
                    self.file_drop_feedback.failed();
                    self.ui_error = Some(format!("無法貼上檔案：{error:#}"));
                }
            }
        }
        if self.file_paste_consumed_frame == Some(frame) {
            // Some file managers advertise both files and their path text.
            // Consume only the handled file paste, including discarded UI passes.
            ctx.input_mut(|input| {
                input
                    .events
                    .retain(|event| !matches!(event, egui::Event::Paste(_)));
                input.consume_key(egui::Modifiers::CTRL, egui::Key::V);
                input.consume_key(egui::Modifiers::SHIFT, egui::Key::Insert);
            });
        }
    }

    fn poll_background(&mut self, ctx: &egui::Context) {
        while let Ok(event) = self.events_rx.try_recv() {
            match event {
                AppEvent::HistoryIndex(generation, corpus)
                    if generation == self.history_generation =>
                {
                    self.history_index = corpus;
                }
                AppEvent::Loaded(generation, id, loaded) if generation == self.load_generation => {
                    self.loading_conversation = false;
                    self.chat.complete_conversation_load(id, loaded);
                    self.reset_draft();
                    self.find = FindState::default();
                }
                AppEvent::Attachments(generation, attachments, errors, picker)
                    if generation == self.draft_generation =>
                {
                    self.attachment_tasks = self.attachment_tasks.saturating_sub(1);
                    if picker {
                        self.picker_open = false;
                    }
                    self.file_drop_feedback.files_added(attachments.len());
                    if attachments.is_empty() && !errors.is_empty() {
                        self.file_drop_feedback.failed();
                    }
                    self.composer.add_attachments(attachments);
                    if !errors.is_empty() {
                        self.ui_error = Some(errors.join("\n"));
                    }
                }
                AppEvent::SignedIn(generation, result) if generation == self.auth_generation => {
                    self.auth_task = None;
                    match result {
                        Ok(tokens) => {
                            self.signed_in = true;
                            self.auth_email = tokens.email;
                            self.auth_plan = tokens.plan;
                            self.auth_error = None;
                        }
                        Err(error) => self.auth_error = Some(error),
                    }
                    self.providers.apply_chatgpt_catalog(&popchat_windows::chat::providers::chatgpt_catalog());
                }
                AppEvent::SignedOut(result) => {
                    self.auth_signing_out = false;
                    match result {
                        Ok(()) => {
                            self.signed_in = false;
                            self.auth_email = None;
                            self.auth_plan = None;
                            self.auth_error = None;
                        }
                        Err(error) => self.auth_error = Some(error),
                    }
                }
                AppEvent::Codex(generation, result) => {
                    if self
                        .providers
                        .apply_codex_inspection(generation, true, result)
                    {
                        self.codex_task = None;
                        use popchat_windows::chat::providers::CodexAppServerStatus;
                        let message = match &self.providers.codex_app_server_status {
                            CodexAppServerStatus::Ready { email, plan } => format!(
                                "Codex 已就緒{}{}",
                                email
                                    .as_ref()
                                    .map(|email| format!(" · {email}"))
                                    .unwrap_or_default(),
                                plan.as_ref()
                                    .map(|plan| format!(" · {plan}"))
                                    .unwrap_or_default()
                            ),
                            CodexAppServerStatus::Missing => codex::MISSING_MESSAGE.to_owned(),
                            CodexAppServerStatus::NotSignedIn => {
                                "請先在終端機執行 codex login，再重新檢查。".to_owned()
                            }
                            CodexAppServerStatus::Failed(error) => error.clone(),
                            _ => "尚未檢查 Codex。".to_owned(),
                        };
                        self.settings_ui.set_codex_state(false, message);
                    }
                }
                AppEvent::Models(id, generation, result)
                    if self.model_generations.get(&id) == Some(&generation) =>
                {
                    self.providers.fetching_providers.remove(&id);
                    if self
                        .providers
                        .providers
                        .iter()
                        .any(|provider| provider.id == id)
                    {
                        match result {
                            Ok((models, capabilities)) => {
                                self.providers.known_models.insert(id, models);
                                self.providers.model_capabilities.insert(id, capabilities);
                                self.providers.model_fetch_errors.remove(&id);
                                self.providers.save();
                            }
                            Err(error) => {
                                self.providers.model_fetch_errors.insert(id, error);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        self.settings_ui.set_chatgpt_state(
            self.signed_in,
            self.auth_email.clone(),
            self.auth_plan.clone(),
            self.auth_task.is_some(),
            self.auth_error.clone(),
        );
        if self.loading_conversation
            || self.attachment_tasks > 0
            || self.auth_task.is_some()
            || self.codex_task.is_some()
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
    }

    fn inspect_codex(&mut self, ctx: &egui::Context) {
        if self.codex_task.is_some() {
            return;
        }
        self.providers.mark_codex_checking();
        self.settings_ui
            .set_codex_state(true, "正在檢查 Codex…".to_owned());
        let generation = self.providers.codex_generation();
        let path = self.codex_path.clone();
        let tx = self.events_tx.clone();
        let ctx = ctx.clone();
        self.codex_task = Some(self.runtime.spawn(async move {
            let result = codex::inspect(path)
                .await
                .map_err(|error| codex::inspection_failure(&error));
            let _ = tx.send(AppEvent::Codex(generation, result));
            ctx.request_repaint();
        }));
    }

    fn lazy_fetch_models(&mut self, id: Uuid, ctx: &egui::Context) {
        use popchat_windows::chat::providers::{self, LazyFetchPlan};
        let key = self.providers.key_for(id, &self.secrets);
        match self.providers.lazy_fetch_plan(id, &key) {
            LazyFetchPlan::Nothing => {}
            LazyFetchPlan::RefreshCodex => self.inspect_codex(ctx),
            LazyFetchPlan::FetchModels => {
                let Some(provider) = self
                    .providers
                    .providers
                    .iter()
                    .find(|provider| provider.id == id)
                    .cloned()
                else {
                    return;
                };
                if provider.kind == ProviderKind::ChatGPT {
                    self.providers
                        .apply_chatgpt_catalog(&providers::chatgpt_catalog());
                    return;
                }
                self.providers.fetching_providers.insert(id);
                let generation = *self.model_generations.entry(id).or_default();
                let tx = self.events_tx.clone();
                let ctx = ctx.clone();
                self.runtime.spawn(async move {
                    let result = match providers::fetch_models_list(
                        &provider.name,
                        &provider.base_url,
                        &key,
                    )
                    .await
                    {
                        Ok((models, mut capabilities)) => {
                            capabilities.extend(
                                providers::probe_ollama_capabilities(&provider.base_url, &models)
                                    .await,
                            );
                            Ok((models, capabilities))
                        }
                        Err(error) => Err(error),
                    };
                    let _ = tx.send(AppEvent::Models(id, generation, result));
                    ctx.request_repaint();
                });
            }
        }
    }

    fn settings_actions(&mut self, ctx: &egui::Context) {
        for action in self.settings_ui.take_actions() {
            if let Some(error) = self.providers.load_error() {
                if matches!(
                    &action,
                    SettingsAction::AddProvider
                        | SettingsAction::AddProviderTemplate(_)
                        | SettingsAction::RemoveProvider(_)
                        | SettingsAction::SelectModel { .. }
                        | SettingsAction::SetEffort { .. }
                        | SettingsAction::SetContextLimit { .. }
                        | SettingsAction::ClearCapabilityException { .. }
                ) {
                    self.ui_error = Some(format!("原服務資料讀取失敗，未執行變更：{error}"));
                    continue;
                }
            }
            match action {
                SettingsAction::SignInChatGpt => {
                    if self.auth_task.is_some() || self.auth_signing_out {
                        continue;
                    }
                    self.auth_generation += 1;
                    self.auth_error = None;
                    let generation = self.auth_generation;
                    let tx = self.events_tx.clone();
                    let ctx = ctx.clone();
                    self.auth_task = Some(self.runtime.spawn(async move {
                        let result =
                            chatgpt_auth::sign_in(&chatgpt_auth::NativeTokenStore::default())
                                .await
                                .map_err(|e| format!("{e:#}"));
                        let _ = tx.send(AppEvent::SignedIn(generation, result));
                        ctx.request_repaint();
                    }));
                }
                SettingsAction::CancelChatGptSignIn => {
                    self.auth_generation += 1;
                    if let Some(task) = self.auth_task.take() {
                        task.abort();
                    }
                }
                SettingsAction::SignOutChatGpt => {
                    if self.auth_signing_out {
                        continue;
                    }
                    self.stop_active_stream();
                    self.auth_generation += 1;
                    let previous = self.auth_task.take();
                    if let Some(task) = &previous {
                        task.abort();
                    }
                    self.auth_signing_out = true;
                    let tx = self.events_tx.clone();
                    let ctx = ctx.clone();
                    self.runtime.spawn(async move {
                        // Wait for cancellation before deletion, so a login
                        // already committing credentials cannot resurrect them.
                        if let Some(task) = previous {
                            let _ = task.await;
                        }
                        let result = tokio::task::spawn_blocking(|| {
                            chatgpt_auth::sign_out(&chatgpt_auth::NativeTokenStore::default())
                                .map_err(|e| format!("{e:#}"))
                        })
                        .await
                        .map_err(|e| format!("{e:#}"))
                        .and_then(|result| result);
                        let _ = tx.send(AppEvent::SignedOut(result));
                        ctx.request_repaint();
                    });
                }
                SettingsAction::InspectCodex => {
                    // Check the path actually shown in the editor. Do not
                    // inspect a previous saved executable behind that label.
                    let path = optional_path(&self.settings_ui.settings.codex_executable_path);
                    if path != self.codex_path {
                        if let Some(task) = self.codex_task.take() {
                            task.abort();
                        }
                        self.codex_path = path.clone();
                        self.backend.set_codex_executable(path);
                        self.providers.invalidate_codex_app_server();
                    }
                    self.inspect_codex(ctx);
                }
                SettingsAction::InstallUpdate(prepared) => {
                    if self.update_confirmation.is_none() && self.update_install.is_none() {
                        popchat_windows::ui::hotkey_recorder::cancel(ctx);
                        self.update_confirmation = Some(prepared);
                        self.settings_ui.set_update_install_message(None, false);
                        ctx.request_repaint();
                    }
                }
                SettingsAction::ChangeHotkey(binding) => {
                    let result = self
                        .hotkey
                        .as_mut()
                        .ok_or_else(|| "全域熱鍵管理器無法啟動，請重新啟動 PopChat。".to_owned())
                        .and_then(|manager| {
                            manager
                                .set_binding(&binding)
                                .map(str::to_owned)
                                .map_err(|error| format!("{error:#}"))
                        });
                    match result {
                        Ok(actual) => {
                            self.hotkey_error = None;
                            let error = self.settings_ui.commit_hotkey_binding(&actual).err();
                            self.settings_ui.set_hotkey_result(Some(&actual), error);
                        }
                        Err(error) => {
                            let actual = self
                                .hotkey
                                .as_ref()
                                .and_then(|manager| manager.label())
                                .map(str::to_owned);
                            self.hotkey_error = if actual.is_none() {
                                Some(error.clone())
                            } else {
                                None
                            };
                            self.settings_ui
                                .set_hotkey_result(actual.as_deref(), Some(error));
                        }
                    }
                    ctx.request_repaint();
                }
                SettingsAction::CodexPathChanged => {
                    let path = optional_path(&self.settings_ui.settings.codex_executable_path);
                    if path != self.codex_path {
                        if let Some(task) = self.codex_task.take() {
                            task.abort();
                        }
                        self.codex_path = path.clone();
                        self.backend.set_codex_executable(path);
                        self.providers.invalidate_codex_app_server();
                        self.settings_ui
                            .set_codex_state(false, "執行檔路徑已更新，請重新檢查。".to_owned());
                    }
                    if let Err(error) = self.settings_ui.persist_codex_path() {
                        self.settings_ui.set_codex_state(false, error);
                    }
                }
                SettingsAction::AddProvider => {
                    let live = self.providers.selected_id;
                    let id = self.providers.add_custom();
                    self.providers.selected_id = live;
                    self.providers.save();
                    self.settings_ui
                        .select_provider(id, &self.providers, &self.secrets);
                }
                SettingsAction::AddProviderTemplate(name) => {
                    if let Some(template) =
                        popchat_windows::chat::providers::free_template_named(&name)
                    {
                        let id = self.providers.add_template(template);
                        self.settings_ui
                            .select_provider(id, &self.providers, &self.secrets);
                    }
                }
                SettingsAction::CommandsChanged(shortcuts) => {
                    self.shortcuts.shortcuts = shortcuts;
                }
                SettingsAction::RemoveProvider(id) => {
                    if id == self.providers.selected_id {
                        self.stop_active_stream();
                    }
                    match self.providers.remove(id, &mut self.secrets) {
                        Ok(()) => self
                            .settings_ui
                            .sync_provider(&self.providers, &self.secrets),
                        Err(error) => self.ui_error = Some(format!("無法移除服務：{error:#}")),
                    }
                }
                SettingsAction::SelectModel { provider_id, model } => {
                    self.providers.select(provider_id, &model, None)
                }
                SettingsAction::SetEffort {
                    provider_id,
                    model,
                    effort,
                } => self.providers.set_effort(effort, provider_id, &model),
                SettingsAction::SetContextLimit {
                    provider_id,
                    model,
                    limit,
                } => {
                    let result = self
                        .providers
                        .set_context_limit(provider_id, &model, limit)
                        .map_err(|error| format!("無法儲存此模型上限：{error:#}"));
                    self.settings_ui
                        .report_context_limit_save(provider_id, &model, limit, result);
                }
                SettingsAction::ClearCapabilityException { provider_id, model } => self
                    .providers
                    .clear_capability_exception(provider_id, &model),
            }
        }
    }

    fn find_hits(&self) -> Vec<(Uuid, usize)> {
        if !self.find.open {
            return Vec::new();
        }
        self.chat
            .messages
            .iter()
            .flat_map(|message| {
                let count = searchable_message_strings(message, &self.chat.full_text(message.id))
                    .iter()
                    .map(|text| find_ranges(text, self.find.query.trim()).len())
                    .sum::<usize>();
                (0..count).map(move |occurrence| (message.id, occurrence))
            })
            .collect()
    }

    fn step_find(&mut self, delta: isize) {
        let count = self.find_hits().len();
        if count > 0 {
            self.find.index =
                (self.find.index as isize + delta).rem_euclid(count as isize) as usize;
        }
        self.find.scroll = true;
    }

    fn toggle_find(&mut self) {
        if self.find.open {
            self.find = FindState::default();
            self.composer.request_focus();
        } else if !self.chat.messages.is_empty() {
            self.find.open = true;
            self.find.focus = true;
            self.find.scroll = true;
        }
    }

    fn show_history(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let mut open = None;
        let mut delete = None;
        let up = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp));
        let down = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown));
        let enter = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        let remove = ui.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::Backspace));
        let field = ui.add(
            egui::TextEdit::singleline(&mut self.history_query)
                .hint_text("搜尋聊天標題與完整內容…")
                .desired_width(f32::INFINITY),
        );
        if self.history_focus {
            field.request_focus();
            self.history_focus = false;
        }
        if field.changed() {
            self.history_selected = 0;
        }
        let rows = history_results(&self.chat.recent, &self.history_index, &self.history_query);
        self.history_selected = self.history_selected.min(rows.len().saturating_sub(1));
        if up {
            self.history_selected = self.history_selected.saturating_sub(1);
        }
        if down {
            self.history_selected = (self.history_selected + 1).min(rows.len().saturating_sub(1));
        }
        if enter {
            open = rows.get(self.history_selected).map(|(meta, _)| meta.id);
        }
        let mut hovered = None;
        ui.add_space(8.0);
        egui::ScrollArea::vertical()
            .id_salt("history_list")
            .max_height((ui.available_height() - 16.0).max(80.0))
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                if rows.is_empty() {
                    ui.label(if self.chat.recent.is_empty() {
                        "尚無儲存的聊天"
                    } else {
                        "沒有符合的聊天"
                    });
                }
                let mut last_day = String::new();
                for (index, (meta, subtitle)) in rows.iter().enumerate() {
                    let day = meta
                        .updated_at
                        .with_timezone(&chrono::Local)
                        .format("%Y-%m-%d")
                        .to_string();
                    if last_day != day {
                        ui.add_space(5.0);
                        ui.label(egui::RichText::new(&day).size(11.0).weak());
                        last_day = day;
                    }
                    let current = meta.id == self.chat.conversation_id;
                    let title = format!("{}{}", if meta.is_fork { "⑂ " } else { "" }, meta.title,);
                    let subtitle_job = (!subtitle.is_empty()).then(|| {
                        let mut job = highlight_job(subtitle, &self.history_query, None, ui);
                        for section in &mut job.sections {
                            if section.format.background == egui::Color32::TRANSPARENT {
                                section.format.color = ui.visuals().weak_text_color();
                            }
                        }
                        job
                    });
                    let time = meta
                        .updated_at
                        .with_timezone(&chrono::Local)
                        .format("%H:%M")
                        .to_string();
                    let row = history::row(
                        ui,
                        egui::Id::new(("history_row", meta.id)),
                        &title,
                        subtitle_job,
                        &time,
                        index == self.history_selected,
                        current,
                    );
                    if row.delete {
                        delete = Some(meta.id);
                    } else if row.open {
                        open = Some(meta.id);
                    }
                    let response = row.response;
                    if response.hovered() || response.contains_pointer() {
                        hovered = Some(meta.id);
                    }
                    if index == self.history_selected && (up || down) {
                        response.scroll_to_me(Some(egui::Align::Center));
                    }
                }
            });
        if remove {
            delete = hovered.or_else(|| rows.get(self.history_selected).map(|(meta, _)| meta.id));
        }
        if let Some(id) = delete {
            self.delete_chat(id, &ctx);
        } else if let Some(id) = open {
            self.load_chat(id, &ctx);
        }
    }

    fn show_find(&mut self, ui: &mut egui::Ui) {
        let id = ui.make_persistent_id("chat_find");
        if ui.memory(|memory| memory.focused() == Some(id)) {
            if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp))
                || ui.input_mut(|i| i.consume_key(egui::Modifiers::SHIFT, egui::Key::Enter))
            {
                self.step_find(-1);
            } else if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown))
                || ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
            {
                self.step_find(1);
            }
        }
        ui.horizontal(|ui| {
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.find.query)
                    .hint_text("在此聊天中尋找…")
                    .id(id)
                    .desired_width((ui.available_width() - 160.0).max(90.0)),
            );
            if self.find.focus {
                response.request_focus();
                self.find.focus = false;
            }
            if response.changed() {
                self.find.index = 0;
                self.find.scroll = true;
            }
            let count = self.find_hits().len();
            self.find.index = self.find.index.min(count.saturating_sub(1));
            ui.label(
                egui::RichText::new(if count == 0 {
                    "0 / 0".to_owned()
                } else {
                    format!("{} / {count}", self.find.index + 1)
                })
                .size(11.0)
                .weak(),
            );
            if icons::hover_help(
                ui.add_enabled(count > 0, egui::Button::new("↑")),
                if count > 0 {
                    "上一筆 · Ctrl+Shift+G"
                } else {
                    "上一筆（目前沒有搜尋結果） · Ctrl+Shift+G"
                },
            )
            .clicked()
            {
                self.step_find(-1);
            }
            if icons::hover_help(
                ui.add_enabled(count > 0, egui::Button::new("↓")),
                if count > 0 {
                    "下一筆 · Ctrl+G"
                } else {
                    "下一筆（目前沒有搜尋結果） · Ctrl+G"
                },
            )
            .clicked()
            {
                self.step_find(1);
            }
            if icons::hover_help(ui.small_button("×"), "關閉搜尋 · Esc").clicked() {
                self.find = FindState::default();
                self.composer.request_focus();
            }
        });
        ui.add_space(7.0);
    }

    /// Commit the exact provider UUID being edited, preserving backend routing.
    fn apply_settings_to_provider(&mut self) {
        let result = self.save_settings();
        self.settings_ui.report_save_result(result);
    }

    fn save_settings(&mut self) -> Result<(), String> {
        if let Some(error) = self.providers.load_error() {
            return Err(format!(
                "服務資料讀取失敗，未改寫服務或金鑰；請修復原檔後重啟：{error}"
            ));
        }
        if let Some(error) = &self.legacy_migration_error {
            return Err(format!(
                "舊金鑰尚未安全遷移；原設定檔已保留。請解決後重啟：{error}"
            ));
        }
        let id = self
            .settings_ui
            .selected_provider_id()
            .ok_or("請先選擇服務")?;
        let settings = self.settings_ui.settings.clone();
        if settings.provider_name.trim().is_empty() {
            return Err("請輸入服務名稱".to_owned());
        }
        let index = self
            .providers
            .providers
            .iter()
            .position(|p| p.id == id)
            .ok_or("找不到此服務")?;
        if self.providers.providers[index].kind == ProviderKind::OpenAICompatible {
            validate_api_endpoint(&settings.base_url)?;
            if settings.model.trim().is_empty() {
                return Err("請輸入或選擇模型名稱".to_owned());
            }
            let key_changed = self.providers.key_for(id, &self.secrets) != settings.api_key.trim();
            self.providers
                .set_key(settings.api_key.trim(), id, &mut self.secrets)
                .map_err(|error| format!("無法儲存金鑰：{error:#}"))?;
            let endpoint = settings.base_url.trim().trim_end_matches('/').to_owned();
            if self.providers.providers[index].base_url != endpoint || key_changed {
                *self.model_generations.entry(id).or_default() += 1;
                self.providers.fetching_providers.remove(&id);
                self.providers.known_models.remove(&id);
                self.providers.model_capabilities.remove(&id);
                self.providers.capability_exceptions.remove(&id);
            }
            self.providers.providers[index].base_url = endpoint;
        }
        self.providers.providers[index].name = settings.provider_name.trim().to_owned();
        self.providers
            .pdf_pass_through
            .insert(id, settings.pdf_pass_through);
        if let Some((owner, models)) = self.settings_ui.fetched_models() {
            self.providers.known_models.insert(owner, models.to_vec());
        }
        // A manually entered model joins this provider's catalog. Saving a
        // catalog draft never changes the pill's live provider/model/effort.
        if !settings.model.trim().is_empty() {
            self.providers
                .settings_models
                .insert(id, settings.model.trim().to_owned());
            let models = self.providers.known_models.entry(id).or_default();
            if !models.iter().any(|model| model == settings.model.trim()) {
                models.push(settings.model.trim().to_owned());
            }
        }
        if let Some((owner, capabilities)) = self.settings_ui.fetched_capabilities() {
            self.providers
                .model_capabilities
                .insert(owner, capabilities);
        }
        self.providers
            .try_save()
            .map_err(|e| format!("無法儲存服務：{e}"))?;
        self.settings_ui.save()?;
        self.shortcuts.shortcuts = self.settings_ui.command_shortcuts().to_vec();
        let path = optional_path(&settings.codex_executable_path);
        if path != self.codex_path {
            self.codex_path = path.clone();
            self.backend.set_codex_executable(path);
            self.providers.invalidate_codex_app_server();
            if let Some(task) = self.codex_task.take() {
                task.abort();
            }
            self.settings_ui
                .set_codex_state(false, "執行檔路徑已更新，請重新檢查。".to_owned());
        }
        Ok(())
    }

    /// 保留 off-screen 隱藏；轉移與 viewport 命令必須一起完成。
    fn hide(&mut self, ctx: &egui::Context) {
        popchat_windows::ui::hotkey_recorder::cancel(ctx);
        self.first_run_hint_due = None;
        if let Some(tray) = &self.tray {
            tray.dismiss_first_run_hint();
        }
        if !self.visibility.is_visible() {
            return;
        }
        let view = ViewportSnapshot::read(ctx);
        if self
            .visibility
            .dismiss(ctx, self.visibility_generation, view)
        {
            logf!("[panel] hide -> off-screen");
            self.save_panel_geometry(view.inner_rect);
        }
    }

    fn save_panel_geometry(&mut self, _inner_rect: Option<egui::Rect>) {
        // Size preferences change only on an actual USER resize. Hiding a
        // compact/monitor-fitted panel must never replace expanded_height.
        self.geometry
            .remember_top_left(self.visibility.shown_position());
        self.geometry.save();
        self.geometry_save_due = None;
    }

    fn update_window_geometry(&mut self, ctx: &egui::Context) {
        let now = std::time::Instant::now();
        let scale = ctx.pixels_per_point();
        let work_areas = panel::available_monitor_rects(scale);
        let monitors_changed = work_areas != self.monitor_work_areas;
        self.monitor_work_areas = work_areas;
        let mode = if self.settings_open {
            1
        } else if self.history_open {
            2
        } else if self.composer.is_large_editor_open() {
            3
        } else if self.chat.messages.is_empty() && !self.chat.is_streaming {
            0
        } else {
            4
        };
        let mut height = match mode {
            1 => 620.0,
            2 => 500.0,
            3 => self.geometry.expanded_height_clamped().max(460.0),
            0 => {
                let line_height = popchat_windows::ui::typography::line_height(
                    self.settings_ui.settings.chat_font_size,
                );
                let extra = (self.composer.inline_rows() - 1) as f32 * line_height
                    + (line_height - 19.0).max(0.0)
                    + self.composer.attachment_height()
                    + if self.find.open { 38.0 } else { 0.0 }
                    + if self.attachment_tasks > 0 { 24.0 } else { 0.0 }
                    + if self.attachment_warning().is_some() {
                        42.0
                    } else {
                        0.0
                    };
                (if self.hotkey_error.is_some() || self.tray_error.is_some() {
                    320.0
                } else {
                    248.0
                }) + extra
            }
            _ => self.geometry.expanded_height_clamped(),
        };
        let padding = egui::Vec2::splat(f32::from(CARD_MARGIN) * 2.0);
        let current = ctx.input(|input| input.viewport().inner_rect.map(|rect| rect.size()));
        let host = self
            .monitor_work_areas
            .iter()
            .find(|area| area.contains(self.visibility.shown_position() + egui::Vec2::splat(1.0)))
            .copied()
            .or_else(|| self.monitor_work_areas.first().copied());
        if let Some(current) = current {
            if self.viewport_sizing.observe(
                current,
                mode,
                scale,
                monitors_changed,
                self.native_resizing,
                now,
            ) {
                let available =
                    host.map_or(egui::vec2(10000.0, 10000.0), |area| area.size()) - padding;
                let constrained = self.geometry.constrain_user_resize(
                    current - padding,
                    mode == 4,
                    height,
                    available,
                );
                self.geometry
                    .record_user_resize(constrained + padding, padding, mode == 4);
                self.geometry_save_due = Some(now + std::time::Duration::from_millis(180));
                if mode == 4 {
                    height = self.geometry.expanded_height_clamped();
                }
            }
        }
        let mut display_geometry = self.geometry.clone();
        display_geometry.expanded_height = height;
        if self.geometry_generation != self.visibility_generation || monitors_changed {
            if self.ever_shown || self.geometry.last_x.is_some() {
                display_geometry.remember_top_left(self.visibility.shown_position());
            }
            let placement = if self.geometry_generation != self.visibility_generation {
                window_prefs::placement_rect(
                    &display_geometry,
                    &self.settings_ui.settings.window_preferences,
                    true,
                    height,
                    padding,
                    &self.monitor_work_areas,
                    panel::desktop_cursor_position(scale),
                )
            } else {
                // A monitor change validates the current dragged position; only
                // startup/new SHOW reapplies the chosen default placement.
                display_geometry.placement(
                    true,
                    height,
                    padding,
                    &self.monitor_work_areas,
                    panel::desktop_cursor_position(scale),
                )
            };
            if let Some(placement) = placement {
                self.visibility
                    .restore_position(ctx, self.visibility_generation, placement.min);
            }
            self.geometry_generation = self.visibility_generation;
        }
        let position = self.visibility.shown_position();
        let host = self
            .monitor_work_areas
            .iter()
            .find(|area| area.contains(position + egui::Vec2::splat(1.0)))
            .copied()
            .or_else(|| self.monitor_work_areas.first().copied());
        let basis = egui::Rect::from_min_size(
            position,
            current.unwrap_or(egui::vec2(self.geometry.width, height) + padding),
        );
        let target = display_geometry.height_change_rect(
            basis,
            true,
            height,
            padding,
            &self.monitor_work_areas,
        );
        let max_width = host.map_or(self.geometry.width + padding.x, |area| area.width());
        let mut wanted = egui::vec2(
            (self.geometry.width + padding.x).min(max_width),
            target.height().max(48.0),
        );
        let height_frame = self.height_transition.update(
            mode,
            wanted.y,
            current.map(|size| size.y),
            ctx.input(|input| input.time),
            transition::motion_enabled(ctx) && !monitors_changed && !self.native_resizing,
        );
        wanted.y = height_frame.height;
        if height_frame.active {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
        let max_height = if mode == 4 {
            host.map_or(wanted.y, |area| area.height())
        } else {
            wanted.y
        };
        let minimum = egui::vec2(
            (panel::MIN_WIDTH + padding.x).min(max_width),
            if mode == 4 {
                (panel::EXPANDED_MIN_HEIGHT + padding.y).min(wanted.y)
            } else {
                wanted.y
            },
        );
        let maximum = egui::vec2(max_width, max_height.max(minimum.y));
        if self.size_limits != Some((minimum, maximum)) {
            // Changing native size limits can itself resize the HWND. Treat
            // animation/clamp events as programmatic, not saved user geometry.
            self.viewport_sizing.requested(wanted, now);
            ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(minimum));
            ctx.send_viewport_cmd(egui::ViewportCommand::MaxInnerSize(maximum));
            self.size_limits = Some((minimum, maximum));
        }
        if !self.native_resizing && current.is_some_and(|size| (size - wanted).length() > 2.0) {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(wanted));
            self.viewport_sizing.requested(wanted, now);
            ctx.request_repaint();
        }
        if let Some(due) = self.geometry_save_due {
            if now >= due && !self.native_resizing {
                self.save_panel_geometry(None);
            } else {
                ctx.request_repaint_after(
                    due.saturating_duration_since(now)
                        .max(std::time::Duration::from_millis(33)),
                );
            }
        }
        self.ever_shown = true;
    }

    /// 背景執行緒事件 + 應用內 shortcut + Esc + 失焦自動隱藏。
    ///
    /// 熱鍵/托盤 Toggle 由 driver 交給共享 controller；UI 端只排空副本。
    fn poll_events(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        if let Some(hotkey) = &self.hotkey {
            hotkey.consume_local_binding(ctx);
        }
        self.visibility_generation = self.visibility.generation();
        // 先讀 egui，然後才進 controller，避免相反的鎖順序。
        let view = ViewportSnapshot::read(ctx);
        let frame_nr = ctx.cumulative_frame_nr();
        // driver 執行緒在隱藏時開了設定窗：同步進來。
        if self.visibility.take_settings_request() {
            self.settings_open = true;
        }
        if let Some(hotkey) = &self.hotkey {
            while hotkey.events().try_recv() == Ok(HotkeyEvent::Toggle) {
                // driver 已處理顯示/隱藏，此處丟棄。
            }
        }
        if let Some(tray) = &self.tray {
            while let Ok(event) = tray.events().try_recv() {
                match event {
                    TrayEvent::TogglePanel => {}
                    TrayEvent::OpenSettings => {
                        self.settings_open = true;
                    }
                    TrayEvent::Quit => {
                        self.visibility.close(ctx);
                    }
                }
            }
        }

        // 應用內 Alt+Down 已移除：全域熱鍵在視窗有焦點時同樣會觸發，
        // 若這裡再 consume 一次，一次按鍵會翻轉兩次（隱藏又立刻顯示），
        // 之前「消失又回來/叫不回」的抖動即來自此雙重觸發。
        // Alt+Space 在 Windows 是系統視窗選單保留鍵，不再使用。

        if self.update_confirmation.is_none() && self.update_install.is_none() {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::W)) {
                self.settings_open = false;
                self.history_open = false;
                self.hide(ctx);
            }
            if self.visibility.is_visible() {
                if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::N)) {
                    self.new_chat();
                }
                if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::Y)) {
                    self.open_history(ctx);
                }
                if !self.settings_open
                    && !self.history_open
                    && !self.loading_conversation
                    && ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::O))
                {
                    self.pick_attachments(ctx);
                }
                if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::F)) {
                    if self.history_open {
                        self.history_focus = true;
                    } else {
                        self.settings_open = false;
                        self.toggle_find();
                    }
                }
                if self.find.open {
                    if ctx.input_mut(|i| {
                        i.consume_key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::G)
                    }) {
                        self.step_find(-1);
                    } else if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::G))
                    {
                        self.step_find(1);
                    }
                }
            }

            // Esc：優先級 設定窗 > 自動完成彈窗/大編輯器（讓位給 composer 自己關）
            // > 隱藏浮窗（對應 onCancel/dismiss）。
            let ime_blocked = ctx.input(|i| {
                popchat_windows::ui::composer::ime_blocks_shortcuts(
                    &mut self.ime_composing,
                    &i.events,
                )
            });
            if !ime_blocked && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                if self.history_open {
                    self.history_open = false;
                    self.composer.request_focus();
                    ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
                } else if self.find.open {
                    self.find = FindState::default();
                    self.composer.request_focus();
                    ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
                } else if self.settings_open {
                    self.settings_open = false;
                } else if !self.composer.popup_visible() && !self.composer.is_large_editor_open() {
                    self.hide(ctx);
                }
            }

            // Ctrl+P：釘選/取消（對應原版 ⌘P pin）。
            if ctx.input(|i| i.key_pressed(egui::Key::P) && i.modifiers.command) {
                self.pinned = !self.pinned;
            }
        }

        // 失焦自動隱藏（對應 windowDidResignKey；設定窗開著時不藏，
        // 避免使用者切出去複製 API key 時浮窗消失——對應 onUserDismiss 語義；
        // 釘選中、大編輯器開著時同樣不藏）。
        let update = self.visibility.ui_frame(
            ctx,
            self.visibility_generation,
            frame_nr,
            view,
            std::time::Instant::now(),
            !self.settings_open
                && !self.pinned
                && !self.composer.is_large_editor_open()
                && !self.picker_open
                && !self.native_resizing
                && !ctx.input(|input| {
                    !input.raw.hovered_files.is_empty() || !input.raw.dropped_files.is_empty()
                })
                && self
                    .attachment_interaction_until
                    .is_none_or(|until| std::time::Instant::now() >= until)
                && !self.settings_ui.has_native_picker(),
            || visibility::activate_window(frame),
        );
        if update.request_composer_focus {
            self.composer.request_focus();
        }
        if update.hidden {
            logf!("[panel] focus-loss hide -> off-screen");
            self.save_panel_geometry(view.inner_rect);
        }
    }

    /// 每幀把背景串流事件排空、fold 進 [`ChatStore`]。
    /// 終端事件（Done/Error/ContentRejected）到達即 `finish_turn` 結束；
    /// sender 已斷開且 task 結束卻無終端事件時同樣收尾，避免卡在 streaming。
    fn poll_stream(&mut self, ctx: &egui::Context) {
        if self.active_stream.is_none() {
            return;
        }
        let mut events = Vec::new();
        let mut disconnected = false;
        if let Some(active) = &self.active_stream {
            loop {
                match active.rx.try_recv() {
                    Ok(ev) => events.push(ev),
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
        }
        let mut finished = false;
        if let Some(active) = self.active_stream.as_mut() {
            for ev in events {
                let terminal = matches!(
                    &ev,
                    ChatStreamEvent::Done(_)
                        | ChatStreamEvent::Error(_)
                        | ChatStreamEvent::ContentRejected { .. }
                );
                self.chat
                    .apply_event(&mut active.turn, ev, &mut self.providers);
                if terminal {
                    finished = true;
                }
            }
            if disconnected && active.handle.is_finished() {
                finished = true;
            }
        }
        if finished {
            if let Some(active) = self.active_stream.take() {
                active.handle.abort();
                self.chat.finish_turn(&active.turn);
            }
            // Invalid-grant refresh can clear credentials without a Settings
            // click. Reflect that real state instead of leaving a stale badge.
            let tokens = chatgpt_auth::NativeTokenStore::default().load();
            self.signed_in = tokens.is_some();
            self.auth_email = tokens.as_ref().and_then(|tokens| tokens.email.clone());
            self.auth_plan = tokens.as_ref().and_then(|tokens| tokens.plan.clone());
            ctx.request_repaint();
        } else if self.active_stream.is_some() {
            ctx.request_repaint();
        }
    }

    /// 頂欄：模型 + 新聊天/歷史/釘選/設定/關閉，同列圖示按鈕。
    /// 只有品牌及控制項間空白可拖，不攔截按鈕點擊。
    fn header_row(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let pill = match self.providers.selected_provider() {
            Some(p) => panel::provider_pill_text(&p.name, &self.providers.current_model()),
            None => panel::provider_pill_text("", ""),
        };
        ui.horizontal(|ui| {
            let brand = ui.horizontal(|ui| {
                icons::mark(ui, 24.0);
                ui.label(egui::RichText::new("PopChat").size(15.0).strong());
            });
            if !self.native_resizing {
                surface::drag_window(
                    ui,
                    brand.response.rect,
                    ui.make_persistent_id("brand_window_drag"),
                );
            }
            ui.add_space(6.0);
            let controls = controls::header_actions_width(ui, self.settings_open);
            let pill_width = (ui.available_width() - controls - 24.0)
                .min(330.0)
                .max(100.0);
            let title = if self.settings_open {
                "返回聊天".to_owned()
            } else {
                self.providers.current_model()
            };
            let (model_rect, model_response) =
                ui.allocate_exact_size(egui::vec2(pill_width, 28.0), egui::Sense::click());
            if model_response.hovered() {
                ui.painter()
                    .rect_filled(model_rect, 7, ui.visuals().faint_bg_color);
            }
            let text_rect = model_rect.shrink2(egui::vec2(10.0, 0.0));
            let clip = egui::Rect::from_min_max(
                text_rect.min,
                egui::pos2(text_rect.right() - 16.0, text_rect.bottom()),
            );
            let color = ui.visuals().weak_text_color();
            ui.painter().with_clip_rect(clip).text(
                text_rect.left_center(),
                egui::Align2::LEFT_CENTER,
                title,
                egui::FontId::proportional(12.0),
                color,
            );
            let center = egui::pos2(model_rect.right() - 10.0, model_rect.center().y);
            ui.painter().add(egui::Shape::line(
                vec![
                    center + egui::vec2(-3.0, -1.5),
                    center + egui::vec2(0.0, 1.5),
                    center + egui::vec2(3.0, -1.5),
                ],
                egui::Stroke::new(1.2_f32, color),
            ));
            model_response.clone().on_hover_text(if self.settings_open {
                "返回聊天"
            } else {
                &pill
            });
            if self.settings_open {
                if model_response.clicked() {
                    self.settings_open = false;
                }
            } else {
                if model_response.clicked() {
                    let providers = self
                        .providers
                        .configured_providers(&self.secrets, self.signed_in);
                    for provider in providers {
                        self.lazy_fetch_models(provider.id, &ctx);
                    }
                }
                let provider_menu_width = (ctx.screen_rect().width() * 0.34).clamp(132.0, 220.0);
                egui::Popup::menu(&model_response)
                    .width(provider_menu_width)
                    .show(|ui| {
                        let viewport = ui.ctx().screen_rect();
                        // Root and child menus must fit the native floating
                        // window, not the much larger desktop monitor.
                        ui.set_max_width((viewport.width() * 0.34).clamp(132.0, 220.0));
                        let providers = self
                            .providers
                            .configured_providers(&self.secrets, self.signed_in);
                        egui::ScrollArea::vertical()
                            .id_salt("provider_menu_scroll")
                            .max_height((viewport.height() - 110.0).clamp(64.0, 300.0))
                            .show(ui, |ui| {
                                for provider in providers {
                                    ui.menu_button(&provider.name, |ui| {
                                        if self.providers.is_fetching(provider.id) {
                                            ui.horizontal(|ui| {
                                                ui.spinner();
                                                ui.label("正在載入模型…");
                                            });
                                        }
                                        if let Some(error) =
                                            self.providers.model_fetch_errors.get(&provider.id)
                                        {
                                            ui.label(
                                                egui::RichText::new(error)
                                                    .small()
                                                    .color(ui.visuals().error_fg_color),
                                            );
                                        }
                                        let remembered =
                                            self.providers.remembered_model(provider.id);
                                        let mut models = self
                                            .providers
                                            .known_models
                                            .get(&provider.id)
                                            .cloned()
                                            .unwrap_or_default();
                                        if !models.contains(&remembered) && !remembered.is_empty() {
                                            models.insert(0, remembered.clone());
                                        }
                                        let rows = models
                                            .into_iter()
                                            .map(|model| {
                                                let selected = self.providers.selected_id
                                                    == provider.id
                                                    && self.providers.current_model() == model;
                                                let chosen = self
                                                    .providers
                                                    .chosen_effort(provider.id, &model);
                                                let efforts = self
                                                    .providers
                                                    .supported_efforts(provider.id, &model)
                                                    .into_iter()
                                                    .map(|effort| {
                                                        let selected = chosen.as_deref()
                                                            == Some(effort.as_str());
                                                        (effort, selected)
                                                    })
                                                    .collect();
                                                switcher::ModelRow {
                                                    model,
                                                    selected,
                                                    efforts,
                                                }
                                            })
                                            .collect::<Vec<_>>();
                                        if let Some(choice) =
                                            switcher::model_list(ui, provider.id, &rows)
                                        {
                                            self.providers.select(
                                                provider.id,
                                                &choice.model,
                                                choice.effort.as_deref(),
                                            );
                                            ui.close();
                                        }
                                    });
                                }
                            });
                        ui.separator();
                        if ui.button("管理服務與模型…").clicked() {
                            self.settings_open = true;
                            ui.close();
                        }
                    });
            }

            // Only the space between controls is draggable, so it cannot
            // intercept clicks on the model selector or the action buttons.
            let drag = ui.allocate_response(
                egui::vec2((ui.available_width() - controls).max(0.0), 28.0),
                egui::Sense::drag(),
            );
            if drag.drag_started() && !self.native_resizing {
                ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
            if !self.settings_open {
                match controls::chat_navigation_inline(ui, self.history_open) {
                    controls::ChatNavigation::NewChat => self.new_chat(),
                    controls::ChatNavigation::History => self.open_history(&ctx),
                    controls::ChatNavigation::None => {}
                }
            }
            if icons::button(
                ui,
                Icon::Pin,
                self.pinned,
                if self.pinned {
                    "取消釘選 · Ctrl+P"
                } else {
                    "釘選視窗 · Ctrl+P"
                },
            )
            .clicked()
            {
                self.pinned = !self.pinned;
            }
            if icons::button(
                ui,
                Icon::Settings,
                self.settings_open,
                if self.settings_open {
                    "返回聊天 · Ctrl+,"
                } else {
                    "開啟設定 · Ctrl+,"
                },
            )
            .clicked()
            {
                self.settings_open = !self.settings_open;
            }
            if icons::button(
                ui,
                Icon::Close,
                false,
                if self.settings_open {
                    "返回聊天 · Esc"
                } else {
                    "收起視窗 · Esc"
                },
            )
            .clicked()
            {
                if self.settings_open {
                    self.settings_open = false;
                } else {
                    self.hide(&ctx);
                }
            }
        });
        ui.add_space(10.0);
    }

    /// 訊息串（卡片中段）：ScrollArea 佔 `height`，貼底；
    /// error role 紅列（不 throw）、activity 弱色；user 氣泡靠右、
    /// assistant 氣泡靠左（氣泡框在 panel::show_assistant_message 內）。
    fn show_list(&mut self, ui: &mut egui::Ui, height: f32) {
        popchat_windows::ui::typography::apply(ui, self.settings_ui.settings.chat_font_size);
        let now = std::time::Instant::now();
        self.copied_messages.retain(|_, until| *until > now);
        let hits = self.find_hits();
        let active_hit = hits.get(self.find.index).copied();
        let mut action = None;
        let mut manual_exception = None;
        let last_user = self
            .chat
            .messages
            .iter()
            .rposition(|message| message.role == ChatRole::User);
        let last_response = last_user.and_then(|index| {
            self.chat
                .messages
                .iter()
                .enumerate()
                .skip(index + 1)
                .filter(|(_, message)| {
                    matches!(message.role, ChatRole::Assistant | ChatRole::Error)
                })
                .map(|(index, _)| index)
                .last()
        });
        let find_scroll = self.find.scroll;
        egui::ScrollArea::vertical()
            .max_height(height)
            .min_scrolled_height(0.0)
            .auto_shrink([false; 2])
            .stick_to_bottom(!self.find.open)
            .show(ui, |ui| {
                chat_layout::transcript_contents(ui, |ui| {
                    for (message_index, msg) in self.chat.messages.iter().enumerate() {
                        let row_streaming = msg.role == ChatRole::Assistant
                            && self.chat.is_streaming
                            && self
                                .active_stream
                                .as_ref()
                                .is_some_and(|active| active.turn.assistant_id == msg.id);
                        if row_streaming && self.chat.stream_target.is_empty() {
                            continue;
                        }
                        let mut shape_start = ui.ctx().graphics(|graphics| {
                            graphics
                                .get(ui.layer_id())
                                .map(|list| list.next_idx().0)
                                .unwrap_or(0)
                        });
                        let mut shape_end = None;
                        let row_start = ui.cursor().min;
                        match msg.role {
                            ChatRole::User => {
                                let dark = ui.visuals().dark_mode;
                                let pal = theme::current_palette(ui.ctx())
                                    .for_bubble_style(self.settings_ui.settings.bubble_style);
                                let w = ui.available_width();
                                ui.with_layout(theme::bubble_layout(true), |ui| {
                                    ui.set_max_width(w * 0.85);
                                    theme::bubble_frame(true, pal, dark).show(ui, |ui| {
                                        // RTL bubble layout defaults labels to Extend.
                                        // Long HTML attributes/URLs must wrap instead
                                        // of widening and shifting the transcript.
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(&msg.text).color(pal.user_text),
                                            )
                                            .wrap(),
                                        );
                                        shape_end = ui.ctx().graphics(|graphics| {
                                            graphics
                                                .get(ui.layer_id())
                                                .map(|list| list.next_idx().0)
                                        });
                                        for attachment in &msg.attachments {
                                            ui.label(
                                                egui::RichText::new(format!(
                                                    "▧ {}",
                                                    attachment.filename
                                                ))
                                                .size(11.0),
                                            )
                                            .on_hover_text(
                                                attachment.note.as_deref().unwrap_or("附件"),
                                            );
                                        }
                                    });
                                });
                            }
                            ChatRole::Assistant => {
                                // full_text 回 owned String，避開與 for 迴圈對 self.chat 的orrow。
                                let text = self.chat.display_text(msg.id);
                                let w = ui.available_width();
                                // Keep each bubble's width local. Resetting the
                                // scrolling parent's max width to infinity lets
                                // full-width code headers expand later frames and
                                // pushes the panel's header controls off-screen.
                                ui.scope(|ui| {
                                    ui.set_max_width(w - 6.0);
                                    ui.spacing_mut().interact_size.y = 18.0;
                                    ui.horizontal(|ui| {
                                        icons::mark(ui, 18.0);
                                        ui.label(egui::RichText::new("PopChat").size(12.0).weak());
                                    });
                                    if let Some(reasoning) =
                                        msg.reasoning.as_ref().filter(|text| !text.is_empty())
                                    {
                                        reasoning::show_disclosure(
                                            ui,
                                            egui::Id::new(("reasoning", msg.id)),
                                            reasoning,
                                            &mut self.math,
                                        );
                                    }
                                    ui.add_space(9.0);
                                    shape_start = ui.ctx().graphics(|graphics| {
                                        graphics
                                            .get(ui.layer_id())
                                            .map(|list| list.next_idx().0)
                                            .unwrap_or(0)
                                    });
                                    self.math.show_assistant_message(ui, &text);
                                    shape_end = ui.ctx().graphics(|graphics| {
                                        graphics.get(ui.layer_id()).map(|list| list.next_idx().0)
                                    });
                                    if let Some(reveal) = self.chat.reveal_for(msg.id) {
                                        popchat_windows::ui::reveal::paint_reveal(
                                            ui,
                                            &reveal,
                                            shape_start,
                                            shape_end,
                                        );
                                    }
                                });
                            }
                            ChatRole::Error => {
                                ui.label(
                                    egui::RichText::new(format!("⚠ {}", msg.text))
                                        .color(egui::Color32::from_rgb(0xFF, 0x6B, 0x6B)),
                                );
                                if let Some(failure) = self
                                    .chat
                                    .capability_failure
                                    .as_ref()
                                    .filter(|failure| failure.error_message_id == msg.id)
                                {
                                    ui.horizontal_wrapped(|ui| {
                                        if failure.sent_images
                                            && controls::button(ui, "記錄此模型不支援圖片")
                                                .clicked()
                                        {
                                            manual_exception = Some(ContentCapability::Images);
                                        }
                                        if failure.sent_files
                                            && controls::button(ui, "記錄此模型不支援 PDF")
                                                .clicked()
                                        {
                                            manual_exception = Some(ContentCapability::Files);
                                        }
                                    });
                                }
                                ui.add_space(6.0);
                            }
                            ChatRole::Activity => {
                                ui.label(egui::RichText::new(format!("… {}", msg.text)).weak());
                                ui.add_space(4.0);
                            }
                        }
                        if self.find.open && hits.iter().any(|(id, _)| *id == msg.id) {
                            let active = active_hit
                                .filter(|(id, _)| *id == msg.id)
                                .map(|(_, occurrence)| occurrence);
                            let active_rect = paint_find_matches(
                                ui,
                                shape_start,
                                shape_end,
                                self.find.query.trim(),
                                active,
                            );
                            if find_scroll && active.is_some() {
                                ui.scroll_to_rect(
                                    active_rect.unwrap_or_else(|| {
                                        egui::Rect::from_min_max(row_start, ui.cursor().min)
                                    }),
                                    Some(egui::Align::Center),
                                );
                            }
                        }
                        if matches!(
                            msg.role,
                            ChatRole::User | ChatRole::Assistant | ChatRole::Error
                        ) {
                            let toolbar = if row_streaming {
                                egui::UiBuilder::new().invisible()
                            } else {
                                egui::UiBuilder::new()
                            };
                            ui.scope_builder(toolbar, |ui| {
                                let copied = self.copied_messages.get(&msg.id).copied();
                                if let Some(until) = copied {
                                    ui.ctx().request_repaint_after(
                                        until.saturating_duration_since(now),
                                    );
                                }
                                let rerunnable_row = Some(message_index) == last_response
                                    || (last_response.is_none()
                                        && Some(message_index) == last_user);
                                match controls::message_actions_row(
                                    ui,
                                    msg.role == ChatRole::User,
                                    copied.is_some(),
                                    !self.chat.is_streaming,
                                    rerunnable_row,
                                ) {
                                    controls::MessageControl::Copy => {
                                        ui.ctx().copy_text(self.chat.full_text(msg.id));
                                        self.copied_messages.insert(
                                            msg.id,
                                            now + std::time::Duration::from_millis(1200),
                                        );
                                        ui.ctx().request_repaint_after(
                                            std::time::Duration::from_millis(1200),
                                        );
                                    }
                                    controls::MessageControl::Fork => {
                                        action = Some(MessageAction::Fork(msg.id))
                                    }
                                    controls::MessageControl::Retry => {
                                        action = Some(MessageAction::Retry)
                                    }
                                    controls::MessageControl::Edit => {
                                        action = Some(MessageAction::Edit)
                                    }
                                    controls::MessageControl::None => {}
                                }
                            });
                            if msg.role == ChatRole::Error {
                                ui.add_space(8.0);
                            }
                        }
                    }
                    // Waiting row：正文還沒到時顯示 transient 狀態；
                    // 正文一到就由 assistant row 即時長大，此列退場。
                    if self.chat.is_streaming && self.chat.stream_target.is_empty() {
                        let status = panel::waiting_label(self.chat.pending_status.as_deref());
                        self.waiting
                            .show(ui, &status, self.chat.pending_reasoning.as_deref());
                        ui.add_space(6.0);
                    }
                });
            });
        self.find.scroll = false;
        if let Some(capability) = manual_exception {
            self.chat
                .record_manual_capability_exception(capability, &mut self.providers);
        }
        if let Some(action) = action {
            self.message_action(action);
        }
    }

    /// Composer 膠囊（卡片底段）：圓角膠囊包住輸入列；串流中改顯示 Stop。
    /// Send 走 [`ChatStore::send`] 既有路徑；slash/autocomplete 只做 UI，
    /// 展開仍由 send 驗證鏈執行。
    fn show_composer_capsule(&mut self, ui: &mut egui::Ui) {
        popchat_windows::ui::typography::apply(ui, self.settings_ui.settings.chat_font_size);
        if let Some(warning) = self.attachment_warning() {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(warning)
                        .size(12.0)
                        .color(egui::Color32::from_rgb(190, 125, 30)),
                )
                .wrap(),
            );
            ui.add_space(6.0);
        }
        let dark = ui.visuals().dark_mode;
        let hairline = if dark {
            egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(0x39, 0x3D, 0x49))
        } else {
            egui::Stroke::new(1.0_f32, theme::LIGHT_BORDER)
        };
        let composer = egui::Frame::new()
            .fill(if dark {
                egui::Color32::from_rgb(0x24, 0x27, 0x31)
            } else {
                theme::LIGHT_CONTROL
            })
            .stroke(hairline)
            .corner_radius(egui::CornerRadius::same(17))
            .inner_margin(egui::Margin::symmetric(16, 12))
            .show(ui, |ui| {
                // store::ShortcutStore（Uuid id、磁碟格式）轉 commands:: 版
                //（String id、complete() 語義）供自動完成；唯讀鏡像，不回寫。
                let cmd_store = CommandsShortcutStore {
                    shortcuts: self
                        .shortcuts
                        .shortcuts
                        .iter()
                        .map(|s| popchat_windows::chat::commands::PromptShortcut {
                            id: s.id.to_string(),
                            name: s.name.clone(),
                            template: s.template.clone(),
                        })
                        .collect(),
                };
                self.composer.set_send_enabled(self.attachment_tasks == 0);
                self.composer.set_streaming(self.chat.is_streaming);
                let action = self.composer.show(ui, &cmd_store, &mut self.chat.prompts);
                match action {
                    ComposerAction::Send(text) => {
                        self.input = text;
                        self.send_current();
                    }
                    ComposerAction::PickAttachments => self.pick_attachments(ui.ctx()),
                    ComposerAction::Stop => self.stop_active_stream(),
                    ComposerAction::None => {}
                }
            });
        self.file_drop_feedback
            .set_composer_rect(composer.response.rect);
    }

    /// 整張卡片：透明視窗上的圓角浮窗（對應原版非搶焦點浮面板的視覺）。
    /// 頂欄 →（錯誤列/空態）→ 訊息串 → Composer 膠囊。
    fn show_card(&mut self, ctx: &egui::Context) {
        let dark = ctx.style().visuals.dark_mode;
        egui::CentralPanel::default()
            .frame(surface::canvas_frame())
            .show(ctx, |ui| {
                let rim = ui.available_rect_before_wrap();
                let vertical = !self.chat.messages.is_empty()
                    && !self.settings_open
                    && !self.history_open
                    && !self.composer.is_large_editor_open();
                if panel::resize_edges(ui, rim, vertical)
                    && ui.input(|input| input.pointer.primary_pressed())
                {
                    self.native_resizing = true;
                }
                if !self.native_resizing {
                    let drag_band = egui::Rect::from_min_max(
                        egui::pos2(rim.left() + 28.0, rim.top() + 5.0),
                        egui::pos2(rim.right() - 28.0, rim.top() + 18.0),
                    );
                    surface::drag_window(ui, drag_band, ui.make_persistent_id("top_window_drag"));
                }
                surface::card_frame_with_opacity(
                    dark,
                    window_prefs::opacity(&self.settings_ui.settings.window_preferences),
                )
                .show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    self.header_row(ui);
                    if self.settings_open && !self.settings_was_open {
                        self.settings_ui
                            .sync_provider(&self.providers, &self.secrets);
                    }
                    self.settings_was_open = self.settings_open;
                    if let Some(error) = self.providers.load_error() {
                        ui.colored_label(
                            ui.visuals().error_fg_color,
                            format!(
                                "無法讀取已儲存的服務，原檔已保留且停止寫入。請修復後重啟：{error}"
                            ),
                        );
                    }
                    let page = if self.settings_open {
                        2
                    } else if self.history_open {
                        1
                    } else if self.composer.is_large_editor_open() {
                        3
                    } else if self.loading_conversation {
                        4
                    } else {
                        0
                    };
                    let effect = self.page_transition.update(
                        page,
                        ui.input(|input| input.time),
                        transition::motion_enabled(ctx),
                    );
                    transition::show(ui, effect, |ui| {
                        if self.settings_open {
                            if let Some(error) = self
                                .legacy_migration_error
                                .as_ref()
                                .or(self.secret_migration_warning.as_ref())
                            {
                                ui.colored_label(
                                    ui.visuals().error_fg_color,
                                    format!("憑證遷移未完成，原始資料已保留：{error}"),
                                );
                            }
                            let runtime = self.runtime.handle().clone();
                            let save =
                                self.settings_ui
                                    .show(ui, &runtime, &self.providers, &self.secrets);
                            if save {
                                self.apply_settings_to_provider();
                            }
                            self.settings_actions(ctx);
                            return;
                        }
                        if let Some(error) = self.ui_error.clone() {
                            ui.horizontal_wrapped(|ui| {
                                ui.colored_label(ui.visuals().error_fg_color, error);
                                if icons::hover_help(ui.small_button("×"), "關閉錯誤提示").clicked()
                                {
                                    self.ui_error = None;
                                }
                            });
                        }
                        if self.history_open {
                            self.show_history(ui);
                            return;
                        }
                        if self.loading_conversation {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label("正在載入聊天…");
                            });
                            return;
                        }
                        if self.composer.is_large_editor_open() {
                            self.show_composer_capsule(ui);
                            return;
                        }
                        // 熱鍵/托盤註冊失敗：紅字 + 重試（原版無此路徑，
                        // Windows 保留鍵衝突才需要）。
                        if self.hotkey_error.is_some() || self.tray_error.is_some() {
                            if let Some(err) = self.hotkey_error.as_deref() {
                                ui.label(
                                    egui::RichText::new(err)
                                        .color(egui::Color32::from_rgb(0xFF, 0x6B, 0x6B))
                                        .size(11.0),
                                );
                                if controls::button(ui, "重試熱鍵註冊").clicked() {
                                    self.retry_hotkey();
                                }
                            }
                            if let Some(err) = self.tray_error.as_deref() {
                                ui.label(
                                    egui::RichText::new(err)
                                        .color(egui::Color32::from_rgb(0xFF, 0x6B, 0x6B))
                                        .size(11.0),
                                );
                            }
                            ui.add_space(4.0);
                        }
                        // Measure the real bottom stack first. A fixed 92pt reserve
                        // omitted footer/inter-widget spacing and clipped the card.
                        chat_layout::bottom_panel(ui, egui::Id::new("chat_bottom"), |ui| {
                            ui.add_space(10.0);
                            if self.find.open {
                                self.show_find(ui);
                            }
                            if self.attachment_tasks > 0 {
                                ui.horizontal(|ui| {
                                    ui.spinner();
                                    ui.label(egui::RichText::new("正在讀取附件…").small().weak());
                                });
                            }
                            self.show_composer_capsule(ui);
                        });
                        chat_layout::show_body(ui, egui::Id::new("chat_transcript"), |ui| {
                            let height = ui.available_height().max(0.0);
                            if self.chat.messages.is_empty() && !self.chat.is_streaming {
                                if height >= 62.0 {
                                    ui.add_space(((height - 62.0) * 0.5).max(0.0));
                                    ui.label(
                                        egui::RichText::new("有什麼想聊的？").size(25.0).strong(),
                                    );
                                    ui.add_space(5.0);
                                    ui.label(
                                        egui::RichText::new("一個問題、一點靈感，隨時開始。")
                                            .size(13.0)
                                            .weak(),
                                    );
                                }
                            } else {
                                self.show_list(ui, height);
                            }
                        });
                    });
                });
            });
    }

    fn send_options(&self) -> SendOptions {
        SendOptions {
            web_search_enabled: self.settings_ui.settings.web_search_enabled,
            system_prompt: self.settings_ui.settings.system_prompt.clone(),
            search_engine: map_search_engine(&self.settings_ui.settings.search_engine),
            search_engine_key: self
                .settings_ui
                .search_key_for_send()
                .filter(|key| !key.trim().is_empty()),
            chatgpt_signed_in: chatgpt_auth::NativeTokenStore::default().load().is_some(),
        }
    }

    fn attachment_warning(&self) -> Option<String> {
        draft_attachment_warning(
            &self.providers.current_model(),
            self.composer.attachments(),
            &self.providers.current_attachment_capabilities(),
        )
    }

    /// 送出：走 [`ChatStore::send`] 驗證鏈（slash 展開、無 key 警告列皆已是
    /// transcript 內的 error role）；`Ready` 即經 tokio 啟動真實串流，
    /// persist 由 store 內部寫 `conversations/*.json`。
    fn send_current(&mut self) {
        if self.chat.is_streaming || self.loading_conversation {
            return;
        }
        let text = std::mem::take(&mut self.input);
        let text = text.trim().to_owned();
        let attachments = self.composer.take_attachments();
        if text.is_empty() && attachments.is_empty() {
            return;
        }
        self.draft_generation += 1;
        self.attachment_tasks = 0;
        self.send_payload(text, attachments);
    }

    fn send_payload(&mut self, text: String, attachments: Vec<Attachment>) {
        let options = self.send_options();
        let outcome = self.chat.send(
            &text,
            attachments,
            &mut self.providers,
            &self.shortcuts,
            &self.secrets,
            &options,
        );
        match outcome {
            SendOutcome::Aborted => {}
            SendOutcome::Ready(turn) => self.start_stream(turn),
        }
    }

    /// `Ready{history,config}` → tokio spawn 跑 [`client::run`]，
    /// 事件經 std mpsc channel 回 UI 執行緒（見 [`Self::poll_stream`]）。
    fn start_stream(&mut self, turn: OutgoingTurn) {
        self.waiting.start_turn();
        self.backend
            .set_session_id(self.chat.conversation_id.to_string());
        let OutgoingTurn {
            history,
            config,
            web_access,
            turn,
        } = turn;
        let (tx, rx) = std::sync::mpsc::channel::<ChatStreamEvent>();
        let handle = self.runtime.spawn(async move {
            let mut stream = client::run(history, config, web_access);
            while let Some(ev) = stream.recv().await {
                if tx.send(ev).is_err() {
                    break;
                }
            }
        });
        self.active_stream = Some(ActiveStream { rx, turn, handle });
    }

    /// Stop 按鈕：abort 背景 task 並以已到達的 partial/reasoning 收尾 persist。
    fn stop_active_stream(&mut self) {
        if let Some(active) = self.active_stream.take() {
            active.handle.abort();
            self.chat.stop();
            self.chat.finish_turn(&active.turn);
        }
    }
}

fn optional_path(raw: &str) -> Option<PathBuf> {
    (!raw.trim().is_empty()).then(|| PathBuf::from(raw.trim()))
}

fn draft_attachment_warning(
    model: &str,
    attachments: &[Attachment],
    capabilities: &popchat_windows::chat::providers::AttachmentCapabilities,
) -> Option<String> {
    use popchat_windows::chat::store::AttachmentContent;
    if capabilities.images == Some(false)
        && attachments
            .iter()
            .any(|attachment| matches!(attachment.content, AttachmentContent::Image { .. }))
    {
        return Some(format!(
            "{model} 不接受圖片；仍會送出原圖，請考慮切換模型，否則可能被拒絕。"
        ));
    }
    if capabilities.files != Some(true) && attachments.iter().any(|attachment| matches!(&attachment.content, AttachmentContent::Pdf { extracted_text, .. } if extracted_text.is_empty())) {
        return Some(format!("掃描 PDF 沒有可抽取文字，而 {model} 不接收 PDF 原檔，因此無法傳送其內容。"));
    }
    None
}

fn load_attachment_paths(paths: Vec<PathBuf>) -> (Vec<Attachment>, Vec<String>) {
    let mut attachments = Vec::new();
    let mut errors = Vec::new();
    for path in paths {
        match attach::load_store_attachment(&path) {
            Ok(attachment) => attachments.push(attachment),
            Err(error) => errors.push(format!(
                "{}：{error:#}",
                path.file_name().unwrap_or_default().to_string_lossy()
            )),
        }
    }
    (attachments, errors)
}

fn load_dropped_attachments(files: Vec<egui::DroppedFile>) -> (Vec<Attachment>, Vec<String>) {
    let mut attachments = Vec::new();
    let mut errors = Vec::new();
    for file in files {
        let result = if let Some(path) = file.path.as_ref() {
            attach::load_store_attachment(path)
        } else if let Some(bytes) = file.bytes.as_deref() {
            let name = if !file.name.is_empty() {
                file.name.clone()
            } else if let Some(kind) = infer::get(bytes) {
                format!("拖入檔案.{}", kind.extension())
            } else {
                "拖入檔案.txt".to_owned()
            };
            attach::load_bytes(bytes, &name)
                .map(attach::into_store_attachment)
                .map_err(anyhow::Error::from)
        } else {
            Err(anyhow::anyhow!("拖入的項目沒有可讀取的檔案路徑或內容"))
        };
        match result {
            Ok(attachment) => attachments.push(attachment),
            Err(error) => errors.push(format!("無法加入附件：{error:#}")),
        }
    }
    (attachments, errors)
}

#[cfg(test)]
mod attachment_drop_tests {
    use super::*;

    #[test]
    fn actual_drop_loader_accepts_paths_and_byte_files_and_reports_each_failure() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("實際路徑.txt");
        std::fs::write(&path, "path payload").unwrap();
        let files = vec![
            egui::DroppedFile {
                path: Some(path),
                ..Default::default()
            },
            egui::DroppedFile {
                name: "直接拖入.txt".to_owned(),
                bytes: Some(std::sync::Arc::from(b"byte payload".as_slice())),
                ..Default::default()
            },
            egui::DroppedFile {
                path: Some(dir.path().join("missing.txt")),
                ..Default::default()
            },
            egui::DroppedFile::default(),
        ];
        let (attachments, errors) = load_dropped_attachments(files);
        assert_eq!(attachments.len(), 2);
        assert_eq!(errors.len(), 2);
        assert_eq!(attachments[0].filename, "實際路徑.txt");
        assert_eq!(attachments[1].filename, "直接拖入.txt");
        assert!(
            matches!(&attachments[0].content, popchat_windows::chat::store::AttachmentContent::Text(text) if text.contains("path payload"))
        );
        assert!(
            matches!(&attachments[1].content, popchat_windows::chat::store::AttachmentContent::Text(text) if text.contains("byte payload"))
        );
    }

    #[test]
    fn nameless_image_drop_uses_real_image_decoder_not_text_or_temporary_files() {
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(4, 3)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        let files = vec![egui::DroppedFile {
            bytes: Some(std::sync::Arc::from(bytes.into_inner())),
            ..Default::default()
        }];
        let (attachments, errors) = load_dropped_attachments(files);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(attachments.len(), 1);
        assert!(
            matches!(&attachments[0].content, popchat_windows::chat::store::AttachmentContent::Image { data_url } if data_url.starts_with("data:image/jpeg;base64,"))
        );
    }
}

/// Fold case/diacritics while retaining a mapping back to UTF-8 byte ranges.
/// Lowercasing can expand one scalar, so offsets into the folded string must
/// never be used to slice the user's original text.
fn folded_with_ranges(text: &str) -> (String, Vec<std::ops::Range<usize>>) {
    use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};
    let mut folded = String::new();
    let mut ranges = Vec::new();
    for (offset, character) in text.char_indices() {
        for decomposed in character
            .to_string()
            .nfd()
            .filter(|c| !is_combining_mark(*c))
        {
            for lower in decomposed.to_lowercase() {
                folded.push(lower);
                ranges.extend(std::iter::repeat_n(
                    offset..offset + character.len_utf8(),
                    lower.len_utf8(),
                ));
            }
        }
    }
    (folded, ranges)
}

fn find_ranges(text: &str, query: &str) -> Vec<std::ops::Range<usize>> {
    let (needle, _) = folded_with_ranges(query);
    if needle.is_empty() {
        return Vec::new();
    }
    let (haystack, mapping) = folded_with_ranges(text);
    haystack
        .match_indices(&needle)
        .filter_map(|(start, matched)| {
            Some(mapping.get(start)?.start..mapping.get(start + matched.len() - 1)?.end)
        })
        .collect()
}

fn markdown_display_text(markdown: &str) -> String {
    let mut result = String::new();
    for event in pulldown_cmark::Parser::new_ext(markdown, pulldown_cmark::Options::all()) {
        match event {
            pulldown_cmark::Event::Text(text) | pulldown_cmark::Event::Code(text) => {
                result.push_str(&text)
            }
            pulldown_cmark::Event::InlineMath(_) | pulldown_cmark::Event::DisplayMath(_) => {
                result.push('\u{fffc}')
            }
            pulldown_cmark::Event::SoftBreak | pulldown_cmark::Event::HardBreak => {
                result.push('\n')
            }
            pulldown_cmark::Event::End(
                pulldown_cmark::TagEnd::Paragraph
                | pulldown_cmark::TagEnd::Heading(_)
                | pulldown_cmark::TagEnd::Item
                | pulldown_cmark::TagEnd::CodeBlock
                | pulldown_cmark::TagEnd::TableRow,
            ) => result.push('\n'),
            pulldown_cmark::Event::End(pulldown_cmark::TagEnd::TableCell) => result.push(' '),
            _ => {}
        }
    }
    result
}

/// Same segment order as the original MarkdownRenderer.searchableStrings:
/// pasteable/code bodies are verbatim, cells are searched independently,
/// and bitmap display math has no searchable text.
fn searchable_message_strings(message: &ChatMessage, full_text: &str) -> Vec<String> {
    if message.role != ChatRole::Assistant {
        return vec![full_text.to_owned()];
    }
    let mut strings = Vec::new();
    for segment in panel::split_math_segments(full_text) {
        match segment {
            panel::MessageSegment::Prose(text) | panel::MessageSegment::Quote(text) => {
                strings.push(markdown_display_text(&text))
            }
            panel::MessageSegment::Code { content, .. }
            | panel::MessageSegment::Pasteable { content, .. } => strings.push(content),
            panel::MessageSegment::Table { header, rows } => {
                strings.extend(
                    header
                        .into_iter()
                        .chain(rows.into_iter().flatten())
                        .map(|cell| markdown_display_text(&cell)),
                );
            }
            panel::MessageSegment::DisplayMath(_) => {}
        }
    }
    strings
}

#[cfg(test)]
fn displayed_message_text(message: &ChatMessage, full_text: &str) -> String {
    searchable_message_strings(message, full_text).join("\n")
}

fn history_results(
    recent: &[ConversationMeta],
    corpus: &HashMap<Uuid, String>,
    query: &str,
) -> Vec<(ConversationMeta, String)> {
    let query = query.trim();
    recent
        .iter()
        .filter_map(|meta| {
            if query.is_empty()
                || !find_ranges(&meta.title, query).is_empty()
                || !find_ranges(&meta.snippet, query).is_empty()
            {
                return Some((meta.clone(), meta.snippet.clone()));
            }
            let body = corpus.get(&meta.id)?;
            let first = find_ranges(body, query).into_iter().next()?;
            let chars: Vec<_> = body
                .char_indices()
                .map(|(index, _)| index)
                .chain(std::iter::once(body.len()))
                .collect();
            let start_index = chars
                .partition_point(|index| *index < first.start)
                .saturating_sub(45);
            let end_index =
                (chars.partition_point(|index| *index < first.end) + 70).min(chars.len() - 1);
            let excerpt = format!(
                "{}{}{}",
                if start_index > 0 { "…" } else { "" },
                body[chars[start_index]..chars[end_index]].replace(['\r', '\n'], " "),
                if end_index < chars.len() - 1 {
                    "…"
                } else {
                    ""
                }
            );
            Some((meta.clone(), excerpt))
        })
        .take(40)
        .collect()
}

fn highlight_job(
    text: &str,
    query: &str,
    active: Option<usize>,
    ui: &egui::Ui,
) -> egui::text::LayoutJob {
    let matches = find_ranges(text, query);
    let accent = theme::current_palette(ui.ctx()).accent;
    let mut job = egui::text::LayoutJob::default();
    let mut cursor = 0;
    for (index, range) in matches.iter().enumerate() {
        for (part, background) in [
            (&text[cursor..range.start], egui::Color32::TRANSPARENT),
            (
                &text[range.clone()],
                accent.gamma_multiply(if active == Some(index) { 0.42 } else { 0.18 }),
            ),
        ] {
            job.append(
                part,
                0.0,
                egui::TextFormat {
                    font_id: egui::FontId::proportional(11.0),
                    color: ui.visuals().text_color(),
                    background,
                    ..Default::default()
                },
            );
        }
        cursor = range.end;
    }
    job.append(
        &text[cursor..],
        0.0,
        egui::TextFormat {
            font_id: egui::FontId::proportional(11.0),
            color: ui.visuals().weak_text_color(),
            ..Default::default()
        },
    );
    job
}

/// Add backgrounds to the renderer's text shapes, preserving all rich-text
/// metrics, selection, code controls, tables, and the original galleys.
fn paint_find_matches(
    ui: &egui::Ui,
    start: usize,
    end: Option<usize>,
    query: &str,
    active: Option<usize>,
) -> Option<egui::Rect> {
    let layer = ui.layer_id();
    let accent = theme::current_palette(ui.ctx()).accent;
    let mut occurrence = 0;
    let mut active_rect = None;
    ui.ctx().graphics_mut(|graphics| {
        let Some(list) = graphics.get_mut(layer) else {
            return;
        };
        let end = end
            .unwrap_or_else(|| list.next_idx().0)
            .min(list.next_idx().0);
        for index in start..end {
            list.mutate_shape(egui::layers::ShapeIdx(index), |clipped| {
                let egui::Shape::Text(text) = &clipped.shape else {
                    return;
                };
                // The renderer's code-language / pasteable-title / Copy
                // controls use small text; they are not transcript contents.
                if text
                    .galley
                    .job
                    .sections
                    .iter()
                    .all(|section| section.format.font_id.size < 12.0)
                {
                    return;
                }
                let matches = find_ranges(&text.galley.job.text, query);
                if matches.is_empty() {
                    return;
                }
                let mut backgrounds = Vec::new();
                for range in matches {
                    let is_active = active == Some(occurrence);
                    occurrence += 1;
                    let char_start = text.galley.job.text[..range.start].chars().count();
                    let char_end = text.galley.job.text[..range.end].chars().count();
                    let mut char_index = 0;
                    for row in &text.galley.rows {
                        for glyph in &row.glyphs {
                            if (char_start..char_end).contains(&char_index) {
                                let rect = glyph
                                    .logical_rect()
                                    .translate(text.pos.to_vec2() + row.pos.to_vec2());
                                if is_active {
                                    active_rect =
                                        Some(active_rect.map_or(rect, |previous: egui::Rect| {
                                            previous.union(rect)
                                        }));
                                }
                                backgrounds.push(egui::Shape::rect_filled(
                                    rect,
                                    1.0,
                                    accent.gamma_multiply(if is_active { 0.42 } else { 0.18 }),
                                ));
                            }
                            char_index += 1;
                        }
                        if row.ends_with_newline {
                            char_index += 1;
                        }
                    }
                }
                backgrounds.push(clipped.shape.clone());
                clipped.shape = egui::Shape::Vec(backgrounds);
            });
        }
    });
    active_rect
}

fn validate_api_endpoint(raw: &str) -> Result<(), String> {
    let url = reqwest::Url::parse(raw.trim()).map_err(|_| "API 端點格式無效".to_owned())?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("API 端點必須使用 http:// 或 https://".to_owned());
    }
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("API 端點不可包含帳密、查詢參數或 #fragment".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod endpoint_tests {
    use super::validate_api_endpoint;

    #[test]
    fn api_endpoints_are_base_urls_not_credential_or_query_carriers() {
        for valid in ["https://api.openai.com/v1", "http://localhost:11434/v1/"] {
            assert!(validate_api_endpoint(valid).is_ok());
        }
        for invalid in [
            "file:///C:/secret",
            "https://user:password@example.com/v1",
            "https://example.com/v1?key=secret",
            "https://example.com/#fragment",
            "not-a-url",
        ] {
            assert!(
                validate_api_endpoint(invalid).is_err(),
                "accepted {invalid}"
            );
        }
    }
}

#[cfg(test)]
mod action_tests {
    fn update_dialog_frame(
        ctx: &egui::Context,
        time: f64,
        events: Vec<egui::Event>,
    ) -> (Option<bool>, egui::FullOutput) {
        let mut decision = None;
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(520.0, 500.0),
                )),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ctx| {
                decision = super::update_confirmation_dialog(ctx, "0.3.0", true);
            },
        );
        assert!(
            !output.viewport_output[&egui::ViewportId::ROOT]
                .commands
                .iter()
                .any(|command| matches!(command, egui::ViewportCommand::Close)),
            "confirmation rendering never closes the app by itself"
        );
        (decision, output)
    }

    #[test]
    fn update_confirmation_requires_an_explicit_install_click_and_escape_cancels() {
        let ctx = egui::Context::default();
        assert_eq!(update_dialog_frame(&ctx, 0.0, vec![]).0, None);
        let _ = update_dialog_frame(&ctx, 0.1, vec![]);
        let key = |key| egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        assert_eq!(
            update_dialog_frame(&ctx, 0.2, vec![key(egui::Key::Enter)]).0,
            None,
            "Enter while opening must not approve installation"
        );
        assert_eq!(
            update_dialog_frame(&ctx, 0.3, vec![key(egui::Key::Escape)]).0,
            Some(false)
        );
    }

    #[test]
    fn real_update_confirmation_buttons_return_only_the_selected_decision() {
        for (label, expected) in [("返回編輯", false), ("安裝並重新啟動", true)] {
            let ctx = egui::Context::default();
            let _ = update_dialog_frame(&ctx, 0.0, vec![]);
            let (_, output) = update_dialog_frame(&ctx, 0.2, vec![]);
            let pos = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.job.text == label => {
                        Some(text.pos + text.galley.size() / 2.0)
                    }
                    _ => None,
                })
                .expect("explicit confirmation button must be visible");
            let button = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            assert_eq!(
                update_dialog_frame(
                    &ctx,
                    0.3,
                    vec![egui::Event::PointerMoved(pos), button(true)]
                )
                .0,
                None
            );
            assert_eq!(
                update_dialog_frame(&ctx, 0.4, vec![button(false)]).0,
                Some(expected)
            );
        }
    }

    #[test]
    fn user_bubble_chat_size_changes_text_not_toolbar_targets_or_panel_width() {
        use popchat_windows::ui::{controls, theme, typography};
        for width in [320.0, 520.0] {
            for size in [12, 15, 24] {
                let ctx = egui::Context::default();
                let pal = theme::apply(&ctx, "Light", theme::DEFAULT_ACCENT_HEX);
                let source = "A long user message with a URL https://example.com/very/long/path and repeated words ".repeat(4);
                let mut toolbar_height = 0.0;
                let output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 1400.0),
                        )),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            typography::apply(ui, size);
                            let w = ui.available_width();
                            ui.with_layout(theme::bubble_layout(true), |ui| {
                                ui.set_max_width(w * 0.85);
                                theme::bubble_frame(true, pal, false).show(ui, |ui| {
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(&source).color(pal.user_text),
                                        )
                                        .wrap(),
                                    );
                                });
                            });
                            let before = ui.cursor().min.y;
                            controls::message_actions_row(ui, true, false, false, false);
                            toolbar_height = ui.cursor().min.y - before;
                            assert!(ui.min_rect().right() <= width + 0.5);
                        });
                    },
                );
                assert!(toolbar_height >= 28.0 && toolbar_height < 45.0);
                assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
                    egui::Shape::Text(text) if text.galley.job.text == source
                        && text.galley.job.sections.iter().all(|section| (section.format.font_id.size - size as f32).abs() < 0.1))));
            }
        }
    }

    #[test]
    fn composer_capability_notice_matches_source_known_vs_unknown_policy() {
        use super::*;
        use popchat_windows::chat::providers::AttachmentCapabilities;
        use popchat_windows::chat::store::AttachmentContent;
        let image = Attachment::new(
            "image.jpg",
            AttachmentContent::Image {
                data_url: "data:image/jpeg;base64,eA==".to_owned(),
            },
        );
        assert!(draft_attachment_warning(
            "m",
            &[image.clone()],
            &AttachmentCapabilities::default()
        )
        .is_none());
        assert!(draft_attachment_warning(
            "m",
            &[image],
            &AttachmentCapabilities {
                images: Some(false),
                files: None
            }
        )
        .unwrap()
        .contains("圖片"));
        let scanned = Attachment::new(
            "scan.pdf",
            AttachmentContent::Pdf {
                data_url: "data:application/pdf;base64,eA==".to_owned(),
                extracted_text: String::new(),
            },
        );
        assert!(draft_attachment_warning(
            "m",
            &[scanned.clone()],
            &AttachmentCapabilities::default()
        )
        .is_some());
        assert!(draft_attachment_warning(
            "m",
            &[scanned],
            &AttachmentCapabilities {
                images: None,
                files: Some(true)
            }
        )
        .is_none());
    }
    use super::*;
    use popchat_windows::chat::store::{AttachmentContent, PromptShortcut, SecretBackend};

    #[test]
    fn find_uses_displayed_markdown_unicode_ranges_and_body_excerpts() {
        let text = "中文 Café cafe\u{301} CAFÉ";
        let ranges = find_ranges(text, "cafe");
        assert_eq!(ranges.len(), 3);
        assert!(ranges.iter().all(|range| text.get(range.clone()).is_some()));
        assert_eq!(&text[ranges[0].clone()], "Café");
        assert!(find_ranges("anything", "\u{301}").is_empty());
        let message = ChatMessage::new(
            ChatRole::Assistant,
            "**Hello** world and `code` [label](https://hidden.example)",
        );
        let displayed = displayed_message_text(&message, &message.text);
        assert!(displayed.contains("Hello world"));
        assert_eq!(find_ranges(&displayed, "hello world").len(), 1);
        assert!(!displayed.contains("https://hidden.example"));
        let blocks = ChatMessage::new(ChatRole::Assistant, "```rust\nliteral **raw**\n```\n<pasteable title=\"Not searched\">\nverbatim **raw**\n</pasteable>\n$$\nx^2\n$$");
        let searched = searchable_message_strings(&blocks, &blocks.text);
        assert_eq!(
            searched
                .iter()
                .map(|text| find_ranges(text, "**raw**").len())
                .sum::<usize>(),
            2
        );
        assert!(!searched.iter().any(|text| text.contains("Not searched")
            || text.contains("x^2")
            || text.contains("rust")));
        let meta = ConversationMeta {
            id: Uuid::new_v4(),
            title: "unrelated title".to_owned(),
            snippet: "unrelated snippet".to_owned(),
            updated_at: chrono::Utc::now(),
            is_fork: false,
        };
        let corpus = HashMap::from([(meta.id, "前文 ".repeat(80) + "CAFÉ hidden body content")]);
        let results = history_results(&[meta], &corpus, "cafe");
        assert_eq!(results.len(), 1);
        assert!(results[0].1.contains("CAFÉ"));
        assert!(results[0].1.starts_with('…'));
    }

    #[test]
    fn production_retry_edit_fork_and_load_preserve_payloads_and_branches() {
        // This binary test never constructs a window, auth store, clipboard,
        // real provider, or settings UI. All persistence is disposable.
        let directory = tempfile::tempdir().unwrap();
        let previous = std::env::var_os("POPCHAT_DATA_DIR");
        std::env::set_var("POPCHAT_DATA_DIR", directory.path());
        struct RestoreEnv(Option<std::ffi::OsString>);
        impl Drop for RestoreEnv {
            fn drop(&mut self) {
                match self.0.take() {
                    Some(value) => std::env::set_var("POPCHAT_DATA_DIR", value),
                    None => std::env::remove_var("POPCHAT_DATA_DIR"),
                }
            }
        }
        let _restore = RestoreEnv(previous);
        let mut chat = ChatStore::new();
        let attachment = Attachment::new(
            "notes.txt",
            AttachmentContent::Text("important attachment content".to_owned()),
        );
        chat.messages
            .push(ChatMessage::new(ChatRole::User, "first prompt"));
        let first_reply = ChatMessage::new(ChatRole::Assistant, "first response body");
        let fork_at = first_reply.id;
        chat.messages.push(first_reply);
        let mut last = ChatMessage::new(ChatRole::User, "/rewrite original");
        last.attachments.push(attachment.clone());
        last.wire_text = Some("outdated expansion".to_owned());
        chat.messages.push(last);
        chat.messages
            .push(ChatMessage::new(ChatRole::Error, "failed old turn"));
        chat.persist();
        let parent = chat.conversation_id;

        let PreparedMessageAction::Retry(text, attachments) =
            prepare_message_action(&mut chat, MessageAction::Retry)
        else {
            panic!("retry had no payload");
        };
        assert_eq!(text, "/rewrite original");
        assert_eq!(attachments, [attachment.clone()]);
        assert_eq!(chat.messages.len(), 2);
        let secrets = SecretStore::with_backend(SecretBackend::File);
        let mut providers = ProviderStore::load(&secrets);
        let provider = Provider::new(
            Uuid::new_v4(),
            "isolated local fixture",
            "http://localhost:1/v1",
            false,
            "fixture-model",
            ProviderKind::OpenAICompatible,
            None,
        );
        providers.selected_id = provider.id;
        providers.providers.push(provider);
        let shortcuts = ShortcutStore {
            shortcuts: vec![PromptShortcut {
                id: Uuid::new_v4(),
                name: "rewrite".to_owned(),
                template: "CURRENT TEMPLATE: {input}".to_owned(),
            }],
        };
        let SendOutcome::Ready(mut outgoing) = chat.send(
            &text,
            attachments,
            &mut providers,
            &shortcuts,
            &secrets,
            &SendOptions {
                web_search_enabled: false,
                ..Default::default()
            },
        ) else {
            panic!("retry failed send validation");
        };
        let wire = serde_json::to_string(&outgoing.history).unwrap();
        assert!(wire.contains("CURRENT TEMPLATE: original"));
        assert!(wire.contains("important attachment content"));
        assert!(!wire.contains("outdated expansion"));
        chat.apply_event(
            &mut outgoing.turn,
            ChatStreamEvent::Done("new response".to_owned()),
            &mut providers,
        );
        chat.finish_turn(&outgoing.turn);
        let PreparedMessageAction::Edit(edited) =
            prepare_message_action(&mut chat, MessageAction::Edit)
        else {
            panic!("edit had no draft");
        };
        let mut composer = Composer::new();
        composer.set_draft(edited.text, edited.attachments);
        assert_eq!(composer.text(), "/rewrite original");
        assert_eq!(composer.attachments(), [attachment.clone()]);
        assert_eq!(chat.messages.len(), 2);

        assert!(matches!(
            prepare_message_action(&mut chat, MessageAction::Fork(fork_at)),
            PreparedMessageAction::Forked
        ));
        let branch = chat.conversation_id;
        assert_ne!(branch, parent);
        assert_eq!(
            ConversationStore::load(branch).unwrap().parent_id,
            Some(parent)
        );
        chat.delete_conversation(parent);
        let resolved = ConversationStore::load_resolved(branch).unwrap();
        assert!(!resolved.missing_parent);
        assert_eq!(resolved.conversation.parent_id, None);
        assert!(resolved
            .messages
            .iter()
            .any(|message| message.text == "first response body"));
        chat.new_chat();
        chat.complete_conversation_load(branch, Some(resolved));
        assert_eq!(chat.conversation_id, branch);
        assert!(chat.messages.iter().any(|message| message.id == fork_at));
        chat.delete_conversation(branch);
        assert!(ConversationStore::load(branch).is_none());
    }
}

fn migrate_legacy_settings(
    settings_ui: &mut SettingsUi,
    providers: &mut ProviderStore,
    secrets: &mut SecretStore,
) -> Result<(), String> {
    if let Some(error) = providers.load_error() {
        return Err(format!("服務資料讀取失敗，未嘗試遷移或改寫憑證：{error}"));
    }
    let settings = settings_ui.settings.clone();
    if settings.api_key.trim().is_empty() {
        return Ok(());
    }
    validate_api_endpoint(&settings.base_url)?;
    let endpoint = settings.base_url.trim().trim_end_matches('/');
    let matching = providers.providers.iter().position(|p| {
        p.kind == ProviderKind::OpenAICompatible && p.base_url.trim_end_matches('/') == endpoint
    });
    let index = match matching {
        Some(index) => index,
        None => {
            providers.providers.push(Provider::new(
                uuid::Uuid::new_v4(),
                format!("{} (已匯入)", settings.provider_name),
                endpoint,
                false,
                settings.model.clone(),
                ProviderKind::OpenAICompatible,
                None,
            ));
            providers.providers.len() - 1
        }
    };
    let id = providers.providers[index].id;
    let existing = providers.key_for(id, secrets);
    if !existing.is_empty() && existing != settings.api_key.trim() {
        return Err("舊設定與服務存放區有不同金鑰；未覆寫任何金鑰".to_owned());
    }
    // Even an apparently existing value may be a legacy file fallback. A
    // checked native write is required before deleting the settings copy.
    providers
        .set_key(settings.api_key.trim(), id, secrets)
        .map_err(|e| format!("{e:#}"))?;
    if !settings.model.trim().is_empty() {
        providers
            .selected_models
            .insert(id, settings.model.trim().to_owned());
    }
    providers.selected_id = id;
    providers.try_save().map_err(|e| e.to_string())?;
    settings_ui.settings.api_key.clear();
    settings_ui.save()
}

/// 骨架 Settings 的引擎字串 → store 的 [`SearchEngine`]。
/// Settings 選項為 `DuckDuckGo / Brave / Tavily / Provider-native`，
/// 大小寫與連字號與 `from_raw` 的 `providerNative` 拼法不同，故在此映射。
fn map_search_engine(raw: &str) -> SearchEngine {
    let lower = raw.trim().to_ascii_lowercase().replace(['-', '_', ' '], "");
    match lower.as_str() {
        "brave" => SearchEngine::Brave,
        "tavily" => SearchEngine::Tavily,
        "providernative" => SearchEngine::ProviderNative,
        _ => SearchEngine::DuckDuckGo,
    }
}

/// 卡片四周透明留白（陰影空間）。視窗尺寸 = 卡片尺寸 + 2×此值。

/// Stop the driver before dropping the UI/window. A disconnected hotkey
/// receiver must not disconnect the otherwise usable tray path.
struct VisibilityDriver {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for VisibilityDriver {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Both event sources use the same controller as UI dismissal. Native focus
/// is deferred to update(), after the controller's show commands are applied.
fn spawn_visibility_driver(
    mut hotkey_rx: Option<std::sync::mpsc::Receiver<HotkeyEvent>>,
    mut tray_rx: Option<std::sync::mpsc::Receiver<TrayEvent>>,
    ctx: egui::Context,
    visibility: Arc<VisibilityController>,
    instance: Arc<InstanceGuard>,
    dismiss_hint: Arc<AtomicBool>,
    hotkey_suppressed: Arc<AtomicBool>,
) -> VisibilityDriver {
    let stop = Arc::new(AtomicBool::new(false));
    let driver_stop = stop.clone();
    let thread = std::thread::Builder::new()
        .name("popchat-visibility".to_owned())
        .spawn(move || {
            logf!("[driver] visibility-driver 啟動");
            // tray-icon may emit Left and Double for one double-click; keep
            // duplicate suppression scoped to tray events only. The global
            // hotkey uses MOD_NOREPEAT and must accept every separate press.
            let mut last_tray_toggle = std::time::Instant::now()
                .checked_sub(std::time::Duration::from_secs(1))
                .unwrap_or_else(std::time::Instant::now);
            while !driver_stop.load(Ordering::SeqCst) {
                if let Some(request) = instance.take_request() {
                    dismiss_hint.store(true, Ordering::SeqCst);
                    match request {
                        ReopenRequest::Show => visibility.show(&ctx, std::time::Instant::now()),
                        ReopenRequest::Settings => {
                            visibility.open_settings(&ctx, std::time::Instant::now())
                        }
                    }
                    logf!("[driver] instance request {request:?}");
                }
                // 熱鍵：blocking 等，100ms 超時順便輪托盤。
                match &hotkey_rx {
                    Some(rx) => match rx.recv_timeout(std::time::Duration::from_millis(100)) {
                        Ok(HotkeyEvent::Toggle) => {
                            dismiss_hint.store(true, Ordering::SeqCst);
                            if driver_stop.load(Ordering::SeqCst) {
                                break;
                            }
                            // The watcher also checks this flag, but a toggle
                            // may already be queued when recording is armed.
                            if !hotkey_suppressed.load(Ordering::SeqCst) {
                                let shown = visibility.toggle(&ctx, std::time::Instant::now());
                                logf!("[driver] hotkey Toggle -> visible={shown}");
                            }
                        }
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                            logf!("[driver] hotkey 端斷開；保留托盤");
                            hotkey_rx = None;
                        }
                    },
                    None => {
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                }
                if driver_stop.load(Ordering::SeqCst) {
                    break;
                }
                while let Some(rx) = &tray_rx {
                    match rx.try_recv() {
                        Ok(event) => match event {
                            TrayEvent::TogglePanel => {
                                dismiss_hint.store(true, Ordering::SeqCst);
                                let now = std::time::Instant::now();
                                if now.duration_since(last_tray_toggle)
                                    < std::time::Duration::from_millis(350)
                                {
                                    logf!("[driver] tray Toggle ignored (double-click debounce)");
                                    continue;
                                }
                                last_tray_toggle = now;
                                let shown = visibility.toggle(&ctx, now);
                                logf!("[driver] tray Toggle -> visible={shown}");
                            }
                            TrayEvent::OpenSettings => {
                                dismiss_hint.store(true, Ordering::SeqCst);
                                logf!("[driver] tray Settings");
                                visibility.open_settings(&ctx, std::time::Instant::now());
                            }
                            TrayEvent::Quit => {
                                logf!("[driver] tray Quit");
                                visibility.close(&ctx);
                                return;
                            }
                        },
                        Err(std::sync::mpsc::TryRecvError::Empty) => break,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            tray_rx = None;
                            break;
                        }
                    }
                }
            }
        })
        .expect("啟動 visibility-driver 執行緒失敗");
    VisibilityDriver {
        stop,
        thread: Some(thread),
    }
}

impl eframe::App for PopChatApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        surface::clear_color()
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        logf!("[lifecycle] UI shutdown");
        self.save_panel_geometry(None);
        self.stop_active_stream();
        self.chat.persist();
        if let Some(task) = self.auth_task.take() {
            task.abort();
        }
        if let Some(task) = self.codex_task.take() {
            task.abort();
        }
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // Capture before EVERY local/global-binding consumer. Acceptance and
        // cancellation frames must not become Send/NewChat/Hide or a paste.
        let recording_frame = popchat_windows::ui::hotkey_recorder::intercept(ctx);
        if !self.settings_open || !self.settings_ui.hotkey_recorder_visible() {
            popchat_windows::ui::hotkey_recorder::cancel(ctx);
        }
        // Consume even while hidden/settings are open, so an old native paste
        // cannot be replayed against a later draft or a different clipboard.
        let native_paste = self.native_paste.take_requested() && !recording_frame;
        if self.dismiss_hint.swap(false, Ordering::SeqCst) {
            self.first_run_hint_due = None;
            if let Some(tray) = &self.tray {
                tray.dismiss_first_run_hint();
            }
        }
        if let Some(due) = self.first_run_hint_due {
            if due <= std::time::Instant::now() {
                self.first_run_hint_due = None;
                if self.visibility.is_visible() {
                    if let Some(tray) = &self.tray {
                        if let Err(error) = tray.show_first_run_hint(
                            self.hotkey.as_ref().and_then(|hotkey| hotkey.label()),
                        ) {
                            logf!("[first-run] {error:#}");
                        }
                    }
                }
            } else {
                ctx.request_repaint_after(due.saturating_duration_since(std::time::Instant::now()));
            }
        }
        self.settings_ui.poll_services(ctx, self.runtime.handle());
        self.settings_actions(ctx);
        // 首幀自檢：run() 後字型系統就緒，確認空態提示解得到字形。
        if !self.font_logged {
            self.font_logged = true;
            let probe = "Alt+Down 呼叫 · 下方輸入開始聊天";
            let ok14 = ctx.fonts(|f| {
                f.has_glyphs(
                    &egui::FontId::new(14.0, egui::FontFamily::Proportional),
                    probe,
                )
            });
            let fams = ctx.fonts(|f| f.families());
            logf!("[font] 首幀自檢 has_glyphs={ok14} families={fams:?}");
        }
        self.poll_events(ctx, frame);
        self.settings_ui
            .set_current_window_position(self.visibility.shown_position());
        if self.update_confirmation.is_none()
            && self.update_install.is_none()
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::Comma))
        {
            self.settings_open = !self.settings_open;
        }
        self.poll_stream(ctx);
        self.chat
            .set_streaming_mode(self.settings_ui.settings.streaming_mode);
        if let Some(interval) = self.chat.tick_reveal(std::time::Instant::now()) {
            ctx.request_repaint_after(interval);
        }
        self.poll_background(ctx);
        self.poll_update_install(ctx);
        if !self.visibility.is_visible() {
            popchat_windows::ui::hotkey_recorder::cancel(ctx);
            return;
        }
        self.handle_attachment_input(ctx, native_paste);
        // 外觀：設定頁的 Appearance/Accent 經設計系統一次套用
        //（Visuals + accent 選取/連結染色 + palette 入 ctx）。
        let appearance = match self.settings_ui.settings.appearance {
            popchat_windows::ui::settings::Appearance::Light => "light",
            popchat_windows::ui::settings::Appearance::Dark => "dark",
            popchat_windows::ui::settings::Appearance::Auto => "auto",
        };
        theme::apply(ctx, appearance, &self.settings_ui.settings.accent_hex);
        if let Err(error) =
            fonts::apply_font_preferences(ctx, &self.settings_ui.settings.font_preferences)
        {
            if self.ui_error.is_none() {
                self.ui_error = Some(format!("字體設定無法套用：{error}"));
            }
        }
        // Position synchronization is part of the generation-checked UI snapshot
        // in poll_events, under the same controller lock as show/hide.
        if self.active_stream.is_some() {
            ctx.request_repaint();
        }
        // 卡片版面（透明視窗上的圓角浮窗）。Settings 彈窗底色跟卡片。
        let dark = ctx.style().visuals.dark_mode;
        ctx.style_mut(|st| {
            st.visuals.window_fill = if dark {
                egui::Color32::from_rgb(0x24, 0x24, 0x26)
            } else {
                theme::LIGHT_RAISED
            };
        });
        // Settings and the editor need real native-window space; an egui
        // overlay cannot extend beyond the compact chat viewport.
        self.update_window_geometry(ctx);
        self.show_card(ctx);
        self.show_update_confirmation(ctx);
        if !self.settings_open || !self.settings_ui.hotkey_recorder_visible() {
            popchat_windows::ui::hotkey_recorder::cancel(ctx);
        }
        if !self.settings_open && !self.history_open && !self.loading_conversation {
            let hovered = ctx.input(|input| input.raw.hovered_files.clone());
            let card = ctx
                .input(|input| input.screen_rect())
                .shrink(f32::from(CARD_MARGIN));
            self.file_drop_feedback
                .show(ctx, card, &hovered, self.attachment_tasks > 0);
        }
        if !ctx.input(|input| input.pointer.primary_down()) {
            self.native_resizing = false;
        }
    }
}

fn main() -> eframe::Result<()> {
    // panic 一律留痕到 log 檔；stdout 壞掉不再能炸死任何執行緒。
    popchat_windows::log::install_panic_hook();
    let instance =
        match InstanceGuard::acquire().map_err(|error| eframe::Error::AppCreation(error.into()))? {
            InstanceLaunch::Primary(guard) => Arc::new(guard),
            InstanceLaunch::Reopened => return Ok(()),
        };
    let first_launch = popchat_windows::startup::record_launch().unwrap_or_else(|error| {
        logf!("[startup] could not record launch: {error:#}");
        true
    });
    logf!(
        "[boot] PopChat 啟動 pid={} executable={} profile={} isolated={}",
        std::process::id(),
        std::env::current_exe()
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
        popchat_windows::paths::config_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
        popchat_windows::paths::isolated_profile().is_some()
    );
    let launch_visible = popchat_windows::startup::should_present_launch(
        first_launch,
        std::env::args().any(|arg| arg == "--login-item"),
        std::env::args().any(|arg| arg == "--settings"),
    ) || (std::env::var_os("POPCHAT_PINNED").is_some()
        && !std::env::args().any(|arg| arg == "--login-item"));
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("PopChat")
        .with_icon(egui::IconData {
            rgba: popchat_windows::app_icon::rgba(128),
            width: 128,
            height: 128,
        })
        .with_inner_size([
            panel::DEFAULT_WIDTH + 24.0,
            panel::DEFAULT_EXPANDED_HEIGHT + 24.0,
        ])
        .with_decorations(false)
        .with_transparent(true)
        .with_always_on_top()
        .with_taskbar(false)
        .with_active(launch_visible)
        .with_resizable(true);
    if !launch_visible {
        viewport = viewport.with_position([-32000.0, -32000.0]);
    }
    let native_paste = Arc::new(NativePaste::default());
    let mut options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    paste::install(&mut options, native_paste.clone());
    let result = eframe::run_native(
        "PopChat",
        options,
        Box::new(move |cc| {
            Ok(
                Box::new(PopChatApp::new(cc, instance, first_launch, native_paste))
                    as Box<dyn eframe::App>,
            )
        }),
    );
    match &result {
        Ok(()) => logf!("[lifecycle] native loop completed"),
        Err(error) => logf!("[lifecycle] native loop failed: {error}"),
    }
    result
}
