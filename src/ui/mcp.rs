//! Human-owned review UI. Rendering a proposal never saves/connects anything.
use super::theme;
use crate::chat::{
    mcp::{
        ApprovalAction, ApprovalRequest, Draft, LocalSetup, McpStore, Service, Session, Transport,
    },
    store::SecretStore,
};
use eframe::egui;
use uuid::Uuid;

#[cfg(test)]
#[path = "mcp_preview.rs"]
pub(super) mod preview;

#[derive(Clone, Copy)]
struct DialogStyle {
    palette: theme::AccentPalette,
    secondary: egui::Color32,
    border: egui::Color32,
    surface: egui::Color32,
}
impl DialogStyle {
    fn new(ctx: &egui::Context) -> Self {
        let palette = theme::current_palette(ctx);
        Self {
            palette,
            secondary: if palette.dark {
                egui::Color32::from_rgb(0xAA, 0xAD, 0xB7)
            } else {
                theme::LIGHT_SECONDARY
            },
            border: if palette.dark {
                egui::Color32::from_white_alpha(22)
            } else {
                egui::Color32::from_black_alpha(18)
            },
            surface: if palette.dark {
                egui::Color32::from_rgb(0x23, 0x26, 0x30)
            } else {
                theme::LIGHT_RAISED
            },
        }
    }
    fn modal(self, ctx: &egui::Context, id: &str) -> egui::Modal {
        egui::Modal::new(egui::Id::new(id))
            .backdrop_color(egui::Color32::from_black_alpha(180))
            .frame(
                egui::Frame::popup(&ctx.style())
                    .fill(theme::panel_fill(self.palette.dark))
                    .stroke(egui::Stroke::new(1.0_f32, self.border))
                    .corner_radius(14)
                    .inner_margin(22),
            )
    }
    fn apply(self, ui: &mut egui::Ui, max_width: f32) {
        ui.set_width((ui.ctx().screen_rect().width() - 88.0).clamp(232.0, max_width));
        let style = ui.style_mut();
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.button_padding = egui::vec2(12.0, 8.0);
        style.spacing.interact_size.y = 34.0;
        style.visuals.extreme_bg_color = if self.palette.dark {
            theme::code_bg_for(true)
        } else {
            theme::LIGHT_CONTROL
        };
        for visual in [
            &mut style.visuals.widgets.inactive,
            &mut style.visuals.widgets.hovered,
            &mut style.visuals.widgets.active,
        ] {
            visual.corner_radius = egui::CornerRadius::same(7);
        }
    }
    fn card(self) -> egui::Frame {
        egui::Frame::new()
            .fill(self.surface)
            .corner_radius(10)
            .inner_margin(14)
    }
    fn panel<R>(self, ui: &mut egui::Ui, body: impl FnOnce(&mut egui::Ui) -> R) -> R {
        self.card()
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                body(ui)
            })
            .inner
    }
    fn scroll_hint(self, ui: &mut egui::Ui, content_height: f32, view_height: f32, offset: f32) {
        if content_height > view_height + 1.0 {
            self.muted(
                ui,
                if offset + view_height < content_height - 1.0 {
                    "向下捲動以檢視更多內容"
                } else {
                    "已捲動至底部"
                },
            );
        }
    }
    fn muted(self, ui: &mut egui::Ui, text: &str) {
        ui.add(egui::Label::new(egui::RichText::new(text).size(12.0).color(self.secondary)).wrap());
    }
    fn heading(self, ui: &mut egui::Ui, title: &str, subtitle: &str) {
        ui.heading(egui::RichText::new(title).size(20.0));
        self.muted(ui, subtitle);
        ui.add_space(6.0);
    }
    fn section(self, ui: &mut egui::Ui, title: &str) {
        ui.label(egui::RichText::new(title).size(13.0).strong());
    }
    fn field(self, ui: &mut egui::Ui, label: &str) {
        ui.label(egui::RichText::new(label).size(12.0).color(self.secondary));
    }
    fn note(self, ui: &mut egui::Ui, title: &str, text: &str, warning: bool) {
        let color = if warning {
            ui.visuals().warn_fg_color
        } else {
            self.secondary
        };
        egui::Frame::new()
            .fill(if warning {
                color.gamma_multiply(0.07)
            } else {
                self.surface
            })
            .stroke(egui::Stroke::new(
                1.0_f32,
                if warning {
                    color.gamma_multiply(0.22)
                } else {
                    self.border
                },
            ))
            .corner_radius(9)
            .inner_margin(12)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(egui::RichText::new(title).size(12.0).strong().color(color));
                self.muted(ui, text);
            });
    }
    fn primary(self, label: &str) -> egui::Button<'_> {
        egui::Button::new(
            egui::RichText::new(label)
                .size(13.0)
                .color(self.palette.accent_text),
        )
        .fill(self.palette.accent)
        .stroke(egui::Stroke::NONE)
        .corner_radius(7)
        .min_size(egui::vec2(144.0, 36.0))
    }
    fn secondary(self, label: &str) -> egui::Button<'_> {
        egui::Button::new(egui::RichText::new(label).size(13.0))
            .fill(egui::Color32::TRANSPARENT)
            .stroke(egui::Stroke::new(1.0_f32, self.border))
            .corner_radius(7)
            .min_size(egui::vec2(72.0, 36.0))
    }
    fn actions(
        self,
        ui: &mut egui::Ui,
        cancel: &str,
        confirm: &str,
        enabled: bool,
    ) -> (egui::Response, egui::Response) {
        ui.add_space(6.0);
        if ui.available_width() < 260.0 {
            let width = ui.available_width();
            let allow = ui
                .add_enabled_ui(enabled, |ui| {
                    ui.add_sized(egui::vec2(width, 36.0), self.primary(confirm))
                })
                .inner;
            let deny = ui.add_sized(egui::vec2(width, 36.0), self.secondary(cancel));
            (deny, allow)
        } else {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let allow = ui.add_enabled(enabled, self.primary(confirm));
                let deny = ui.add(self.secondary(cancel));
                (deny, allow)
            })
            .inner
        }
    }
}

fn text_field(ui: &mut egui::Ui, value: &mut String, hint: &str, password: bool) -> egui::Response {
    ui.add(
        egui::TextEdit::singleline(value)
            .font(egui::FontId::proportional(13.0))
            .desired_width(f32::INFINITY)
            .margin(egui::vec2(10.0, 9.0))
            .hint_text(hint)
            .password(password),
    )
}

fn detail(ui: &mut egui::Ui, style: DialogStyle, label: &str, value: &str, code: bool) {
    style.field(ui, label);
    let text = egui::RichText::new(value).size(13.0);
    ui.add(
        egui::Label::new(if code { text.monospace() } else { text })
            .wrap()
            .selectable(true),
    );
}

struct TestJob {
    service: Service,
    rx: std::sync::mpsc::Receiver<Result<Vec<String>, String>>,
    handle: tokio::task::JoinHandle<()>,
}
struct LocalReview {
    conversation_id: Uuid,
    request: String,
}
impl Drop for TestJob {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

pub struct McpUi {
    #[cfg(test)]
    save_rect: Option<egui::Rect>,
    #[cfg(test)]
    save_clip_rect: Option<egui::Rect>,
    #[cfg(test)]
    private_http_rect: Option<(egui::Rect, egui::Rect)>,
    pub store: McpStore,
    pub open: bool,
    id: Option<Uuid>,
    name: String,
    http: bool,
    url: String,
    command: String,
    args: String,
    token_env: String,
    token: String,
    clear_token: bool,
    enabled: bool,
    allow_private_http: bool,
    local_review: Option<LocalReview>,
    confirmed_request: Option<(Uuid, String)>,
    has_draft: bool,
    pending: std::collections::VecDeque<Draft>,
    feedback: Option<String>,
    remove: Option<Uuid>,
    test: Option<TestJob>,
}
impl McpUi {
    pub fn new() -> Self {
        Self::with_store(McpStore::load())
    }
    pub fn with_store(store: McpStore) -> Self {
        Self {
            #[cfg(test)]
            save_rect: None,
            #[cfg(test)]
            save_clip_rect: None,
            #[cfg(test)]
            private_http_rect: None,
            store,
            open: false,
            id: None,
            name: String::new(),
            http: true,
            url: String::new(),
            command: String::new(),
            args: "[]".into(),
            token_env: String::new(),
            token: String::new(),
            clear_token: false,
            enabled: false,
            allow_private_http: false,
            local_review: None,
            confirmed_request: None,
            has_draft: false,
            pending: Default::default(),
            feedback: None,
            remove: None,
            test: None,
        }
    }
    fn fill(&mut self, id: Option<Uuid>, draft: Draft, enabled: bool) {
        self.id = id;
        self.name = draft.name;
        self.token_env = draft.token_env.unwrap_or_default();
        self.token.clear();
        self.clear_token = false;
        self.allow_private_http = id
            .and_then(|id| self.store.servers.iter().find(|s| s.id == id))
            .is_some_and(|s| s.allow_private_http);
        self.local_review = None;
        self.enabled = enabled;
        self.has_draft = true;
        match draft.transport {
            Transport::Http { url } => {
                self.http = true;
                self.url = url;
                self.command.clear();
                self.args = "[]".into();
            }
            Transport::Stdio { command, args } => {
                self.http = false;
                self.command = command;
                self.args = serde_json::to_string(&args).unwrap_or_else(|_| "[]".into());
                self.url.clear();
            }
        }
    }
    pub fn review(&mut self, draft: Draft) {
        if self.has_draft || self.open {
            if self.pending.len() < 4 {
                self.pending.push_back(draft);
                self.feedback = Some("有新的對話 MCP 草稿等待檢視；目前編輯未被覆寫。".into());
            } else {
                self.feedback=Some("MCP 待確認草稿佇列已滿，最新提案未加入。請先處理現有草稿後再提出；目前編輯未被覆寫。".into());
            }
        } else {
            self.fill(None, draft, false);
            self.feedback = Some("草稿尚未保存或連線，請核對上方資訊。".into());
        }
        self.open = true;
    }
    pub fn review_local_setup(
        &mut self,
        setup: LocalSetup,
        conversation_id: Uuid,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.has_draft && !self.open,
            "MCP 已有尚未處理的草稿；請先完成或放棄目前草稿，再重新送出連線資訊。憑證未送給模型。"
        );
        self.fill(None, setup.draft, false);
        self.token = setup.token;
        self.local_review = Some(LocalReview {
            conversation_id,
            request: setup.request,
        });
        self.feedback = Some("連線資訊已在本機填入，尚未保存或連線。".into());
        self.open = true;
        Ok(())
    }
    pub fn take_confirmed_request(&mut self) -> Option<(Uuid, String)> {
        self.confirmed_request.take()
    }
    pub fn open_settings(&mut self, id: Option<Uuid>) {
        if let Some(service) = id
            .and_then(|id| self.store.servers.iter().find(|s| s.id == id))
            .cloned()
        {
            if !self.has_draft {
                self.fill(Some(service.id), service.draft, service.enabled);
            } else {
                self.feedback = Some(
                    "已開啟 MCP 設定；保留目前尚未儲存的草稿，請自行選取要編輯的服務。".into(),
                );
            }
        }
        self.open = true;
    }
    fn draft(&self) -> anyhow::Result<Draft> {
        let transport = if self.http {
            Transport::Http {
                url: self.url.trim().into(),
            }
        } else {
            Transport::Stdio {
                command: self.command.trim().into(),
                args: serde_json::from_str(&self.args).map_err(|_| {
                    anyhow::anyhow!("args 必須是 JSON 字串陣列，例如 [\"server.js\"]。")
                })?,
            }
        };
        let draft = Draft {
            name: self.name.trim().into(),
            transport,
            token_env: (!self.http && !self.token_env.trim().is_empty())
                .then(|| self.token_env.trim().to_string()),
        };
        draft.validate()?;
        Ok(draft)
    }
    fn cancel_test(&mut self) {
        self.test = None;
    }
    fn start_test(
        &mut self,
        service: &Service,
        runtime: &tokio::runtime::Handle,
        secrets: &SecretStore,
    ) {
        match service.token(secrets) {
            Ok(token) => {
                let (tx, rx) = std::sync::mpsc::channel();
                let target = service.clone();
                let handle = runtime.spawn(async move {
                    let result = async {
                        let hidden = token.clone();
                        let mut session = Session::connect(&target, token).await?;
                        let tools = session.tools().await?;
                        Ok::<_, anyhow::Error>(
                            tools
                                .iter()
                                .filter_map(|t| t["name"].as_str())
                                .map(|name| {
                                    if hidden.is_empty() {
                                        name.to_owned()
                                    } else {
                                        name.replace(&hidden, "[REDACTED MCP TOKEN]")
                                    }
                                })
                                .collect::<Vec<_>>(),
                        )
                    }
                    .await
                    .map_err(|e| e.to_string());
                    let _ = tx.send(result);
                });
                self.test = Some(TestJob {
                    service: service.clone(),
                    rx,
                    handle,
                });
                self.feedback = Some("正在測試連線，只探索工具，不執行工具。".into());
            }
            Err(_) => self.feedback = Some("MCP 憑證無法讀取，請重新設定。".into()),
        }
    }

    fn editor(&mut self, ui: &mut egui::Ui, style: DialogStyle) {
        style.panel(ui, |ui| {
            style.section(ui, "連線資訊");
            style.field(ui, "服務名稱");
            self.has_draft |= text_field(ui, &mut self.name, "例如：我的工具服務", false).changed();
            style.field(ui, "傳輸方式");
            ui.horizontal(|ui| {
                let width = ((ui.available_width() - 8.0) / 2.0).max(60.0);
                for (value, label) in [(true, "遠端 HTTP"), (false, "本機 stdio")] {
                    let selected = self.http == value;
                    if ui
                        .add_sized(
                            egui::vec2(width, 34.0),
                            egui::Button::new(egui::RichText::new(label).size(12.0))
                                .fill(if selected {
                                    style.palette.accent.gamma_multiply(0.12)
                                } else {
                                    egui::Color32::TRANSPARENT
                                })
                                .stroke(egui::Stroke::new(
                                    1.0_f32,
                                    if selected {
                                        style.palette.accent.gamma_multiply(0.55)
                                    } else {
                                        style.border
                                    },
                                )),
                        )
                        .clicked()
                        && !selected
                    {
                        self.http = value;
                        self.has_draft = true;
                        self.allow_private_http = false;
                    }
                }
            });
            if self.http {
                style.field(ui, "服務位址");
                if text_field(ui, &mut self.url, "https://example.com/mcp", false).changed() {
                    self.has_draft = true;
                    self.allow_private_http = false;
                }
                style.muted(ui, "Streamable HTTP · 優先使用 HTTPS");
            } else {
                style.field(ui, "執行程式");
                self.has_draft |= text_field(
                    ui,
                    &mut self.command,
                    "node.exe、python.exe 或完整程式路徑",
                    false,
                )
                .changed();
                style.field(ui, "啟動參數 · JSON 陣列");
                self.has_draft |= ui
                    .add(
                        egui::TextEdit::multiline(&mut self.args)
                            .font(egui::FontId::monospace(12.0))
                            .desired_width(f32::INFINITY)
                            .desired_rows(3)
                            .margin(egui::vec2(10.0, 9.0)),
                    )
                    .changed();
                style.field(ui, "憑證環境變數 · 選填");
                self.has_draft |=
                    text_field(ui, &mut self.token_env, "例如：API_KEY", false).changed();
                style.muted(ui, "直接啟動程式，不經 shell；不會自動安裝套件。");
            }
        });
        style.panel(ui, |ui| {
            style.section(ui, "驗證");
            style.field(
                ui,
                if self.http {
                    "Bearer token · 選填"
                } else {
                    "憑證 token · 選填"
                },
            );
            self.has_draft |= text_field(ui, &mut self.token, "僅保存在本機憑證庫", true).changed();
            style.muted(
                ui,
                if self.id.is_some() {
                    "留空會保留已儲存的憑證，不會交給模型。"
                } else {
                    "憑證不寫入設定 JSON，也不會交給模型。"
                },
            );
            if self.id.is_some() {
                self.has_draft |= ui
                    .checkbox(&mut self.clear_token, "移除已儲存的憑證")
                    .changed();
            }
        });
        let private_http = self
            .draft()
            .is_ok_and(|draft| draft.requires_private_http_approval());
        if private_http {
            style.note(ui,"此連線未加密","內網 HTTP 會以明文傳送憑證與工具資料，可能遭攔截。只在可信任網路使用，優先改用 HTTPS。",true);
            let permission = ui.checkbox(
                &mut self.allow_private_http,
                "我了解風險，允許向此內網 HTTP 位址傳送憑證與資料",
            );
            #[cfg(test)]
            {
                self.private_http_rect = Some((permission.rect, ui.clip_rect()));
            }
            self.has_draft |= permission.changed();
        }
        if self.local_review.is_some() {
            style.note(
                ui,
                "確認後才會啟用",
                "按下確認後，將保存並啟用服務，接續原本的連線要求。每次工具執行仍須你確認。",
                false,
            );
            style.panel(ui, |ui| {
                style.section(ui, "本機儲存位置");
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(self.store.settings_path().display().to_string())
                            .monospace()
                            .size(11.0)
                            .color(style.secondary),
                    )
                    .wrap(),
                );
                style.muted(ui, "此 JSON 只存服務設定，憑證另存於本機憑證庫。");
            });
        } else {
            style.panel(ui, |ui| {
                self.has_draft |= ui.checkbox(&mut self.enabled, "啟用此服務").changed();
                style.muted(
                    ui,
                    "儲存不會立即連線；下一次送出聊天時才載入。每次工具執行仍須確認。",
                );
            });
        }
        style.note(
            ui,
            "只使用你信任的服務",
            if self.http {
                "服務可能接收你的對話資料。確認目的地與工具參數後再授權，不要因唯讀標示而略過確認。"
            } else {
                "本機程式不是沙箱，啟動後即可依你的權限讀寫檔案或連網；npx 等啟動器可能下載套件。"
            },
            !self.http,
        );
    }

    fn saved_services(
        &mut self,
        ui: &mut egui::Ui,
        style: DialogStyle,
        runtime: &tokio::runtime::Handle,
        secrets: &mut SecretStore,
    ) -> bool {
        let mut changed = false;
        if self.store.servers.is_empty() {
            return false;
        }
        egui::CollapsingHeader::new(format!("已儲存的服務 · {}", self.store.servers.len()))
            .default_open(self.local_review.is_none())
            .show(ui, |ui| {
                style.muted(ui, "連線測試會啟動程式／連線，只探索工具，不執行工具。");
                for service in self.store.servers.clone() {
                    style.panel(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(egui::RichText::new(&service.draft.name).size(14.0).strong());
                            ui.label(
                                egui::RichText::new(if service.enabled {
                                    "已啟用"
                                } else {
                                    "停用"
                                })
                                .size(11.0)
                                .color(if service.enabled {
                                    style.palette.accent
                                } else {
                                    style.secondary
                                }),
                            );
                        });
                        style.muted(
                            ui,
                            match &service.draft.transport {
                                Transport::Http { .. } => "Streamable HTTP",
                                Transport::Stdio { .. } => "本機 stdio",
                            },
                        );
                        ui.horizontal_wrapped(|ui| {
                            if ui.button("編輯").clicked() {
                                self.fill(Some(service.id), service.draft.clone(), service.enabled);
                                self.feedback = None;
                            }
                            if ui
                                .add_enabled(self.test.is_none(), egui::Button::new("連線測試"))
                                .clicked()
                            {
                                self.start_test(&service, runtime, secrets);
                            }
                            if ui.button("刪除").clicked() {
                                self.remove = Some(service.id);
                            }
                        });
                    });
                }
                if self.test.is_some() && ui.button("取消連線測試").clicked() {
                    self.cancel_test();
                    self.feedback = Some("已取消連線測試。".into());
                }
                if let Some(id) = self.remove {
                    style.note(
                        ui,
                        "確認刪除服務？",
                        "這會移除此服務的設定與已儲存憑證。",
                        true,
                    );
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("確認刪除").clicked() {
                            match self.store.remove(id, secrets) {
                                Ok(()) => {
                                    changed = true;
                                    self.cancel_test();
                                    self.feedback = Some("MCP 服務已移除。".into());
                                }
                                Err(_) => self.feedback = Some("無法移除服務，原設定保留。".into()),
                            }
                            self.remove = None;
                        }
                        if ui.button("返回").clicked() {
                            self.remove = None;
                        }
                    });
                }
            });
        changed
    }

    fn save_review(&mut self, secrets: &mut SecretStore) -> bool {
        let result = self.draft().and_then(|draft| {
            self.store.save_with_http_permission(
                self.id,
                draft,
                self.enabled || self.local_review.is_some(),
                (!self.token.is_empty()).then_some(self.token.as_str()),
                self.clear_token,
                self.allow_private_http,
                secrets,
            )
        });
        match result {
            Ok(id) => {
                self.id = Some(id);
                self.token.clear();
                self.clear_token = false;
                self.has_draft = false;
                self.cancel_test();
                if let Some(mut review) = self.local_review.take() {
                    review.request.push_str(&format!("\n已確認的 MCP 服務 ID：{id}；請只測試／使用此服務，不要使用先前訊息中的其他目的地。"));
                    self.confirmed_request = Some((review.conversation_id, review.request));
                }
                self.feedback = Some("MCP 服務已儲存；每次工具執行仍須確認。".into());
                true
            }
            Err(error) => {
                self.feedback = Some(format!("未能儲存 MCP：{error}"));
                false
            }
        }
    }
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        runtime: &tokio::runtime::Handle,
        secrets: &mut SecretStore,
    ) -> bool {
        let completed = self.test.as_ref().and_then(|job| {
            job.rx
                .try_recv()
                .ok()
                .map(|result| (job.service.clone(), result))
        });
        if let Some((service, result)) = completed {
            self.test = None;
            if self.store.servers.iter().any(|s| s == &service) {
                self.feedback = Some(match result {
                    Ok(names) => format!("連線成功：{} 個工具\n{}", names.len(), names.join("、")),
                    Err(error) => format!("連線測試失敗：{error}"),
                });
            }
        }
        if self.test.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        if !self.open {
            return false;
        }
        let mut changed = false;
        let mut close = false;
        let style = DialogStyle::new(ctx);
        let response=style.modal(ctx,"mcp-manager").show(ctx,|ui| {
            style.apply(ui,560.0);
            style.heading(ui,if self.local_review.is_some(){"確認新增 MCP 服務"}else if self.id.is_some(){"編輯 MCP 服務"}else{"新增 MCP 服務"},
                if self.local_review.is_some(){"資訊已在本機整理，確認後才會保存與啟用。"}else{"設定連線、驗證方式與工具使用權限。"});
            let body=egui::ScrollArea::vertical().id_salt("mcp-manager-content")
                .max_height((ctx.screen_rect().height()-258.0).max(72.0)).show(ui,|ui| {
                    if let Some(error)=&self.store.load_error {style.note(ui,"設定暫時無法使用",error,true);}
                    if !self.pending.is_empty() {
                        style.muted(ui,&format!("另有 {} 份新增草稿等待檢視。",self.pending.len()));
                        if ui.button("放棄目前草稿並檢視下一份").clicked(){if let Some(draft)=self.pending.pop_front(){self.fill(None,draft,false);}}
                    }
                    self.editor(ui,style);
                    if let Some(text)=&self.feedback {style.note(ui,"狀態",text,false);}
                    changed|=self.saved_services(ui,style,runtime,secrets);
                    egui::CollapsingHeader::new("進階選項").show(ui,|ui| {
                        let mut allow=self.store.allow_chat_proposals;
                        if ui.add_enabled(self.store.load_error.is_none(),egui::Checkbox::new(&mut allow,"允許模型提出新增草稿")).changed() {
                            match self.store.set_chat_proposals(allow){Ok(())=>{changed=true;self.feedback=Some("草稿選項已儲存，不代表允許自動新增或執行工具。".into());},Err(_)=>self.feedback=Some("無法保存 MCP 選項，原設定保留。".into())}
                        }
                        style.muted(ui,"需要支援工具呼叫的模型。也可用 /mcp add {JSON} 開啟草稿；一般聊天請勿貼上憑證。");
                        style.muted(ui,"HTTP 使用手動 Bearer token，不提供自動 OAuth 登入。儲存設定會停止舊工具回合。");
                        if ui.button("新增服務／清空目前草稿").clicked(){self.fill(None,Draft{name:String::new(),transport:Transport::Http{url:String::new()},token_env:None},false);}
                    });
                });
            style.scroll_hint(ui,body.content_size.y,body.inner_rect.height(),body.state.offset.y);
            let allow_save=self.store.load_error.is_none() && self.draft().is_ok_and(|draft|!draft.requires_private_http_approval() || self.allow_private_http);
            let local=self.local_review.is_some();
            let (cancel,save)=style.actions(ui,"關閉",if local{"確認新增、啟用並繼續"}else{"確認並儲存 MCP 服務"},allow_save);
            #[cfg(test)] {self.save_rect=Some(save.rect);self.save_clip_rect=Some(ui.clip_rect());}
            if cancel.clicked(){close=true;}
            if save.clicked(){let saved=self.save_review(secrets);changed|=saved;if saved && local {close=true;}}
        });
        if close || response.should_close() {
            self.open = false;
            self.cancel_test();
        }
        changed
    }
}

pub fn approval_dialog(ctx: &egui::Context, request: &ApprovalRequest) -> Option<bool> {
    let testing = request.action == ApprovalAction::ConnectionTest;
    let style = DialogStyle::new(ctx);
    let response=style.modal(ctx,"mcp-tool-approval").show(ctx,|ui|{
        style.apply(ui,480.0);
        style.heading(ui,if testing {"允許測試連線？"}else{"允許執行這個工具？"},
            if matches!(request.service.draft.transport,Transport::Stdio{..}) {"本機程式不是沙箱，可能讀寫檔案或連網。"}
            else if testing {"會啟動／連線並探索工具，這次不執行工具。"}
            else {"工具可能讀寫檔案或傳送資料，請先檢查。"});
        let body=egui::ScrollArea::vertical().id_salt("mcp-approval-content")
            .max_height((ctx.screen_rect().height()-258.0).max(72.0)).show(ui,|ui| {
                style.panel(ui,|ui| {
                    detail(ui,style,"服務",&request.service.draft.name,false);
                    if !testing {detail(ui,style,"工具",&request.tool,true);}
                    match &request.service.draft.transport {
                        Transport::Http{url}=>detail(ui,style,"目的地 · Streamable HTTP",url,true),
                        Transport::Stdio{command,args}=>{
                            detail(ui,style,"本機程式",command,true);
                            if !args.is_empty(){detail(ui,style,"啟動參數",&serde_json::to_string_pretty(args).unwrap_or_default(),true);}
                        }
                    }
                });
                if !testing {
                    style.panel(ui,|ui| {
                        style.section(ui,"執行參數");
                        if request.arguments.as_object().is_some_and(|args|args.is_empty()) {
                            style.muted(ui,"此工具不需要額外參數。");
                        }else{
                            ui.add(egui::Label::new(egui::RichText::new(serde_json::to_string_pretty(&request.arguments).unwrap_or_default()).monospace().size(12.0)).wrap().selectable(true));
                        }
                    });
                }
                if testing {
                    style.note(ui,"只測試連線","會啟動／連線並探索工具，不執行 tools/call，也不保存或啟用服務。拒絕不會連線。",false);
                }else{
                    style.note(ui,"確認目的地與參數後再允許","工具可能讀寫檔案、啟動程式或傳送資料。參數由模型提出，可能包含你的對話內容；拒絕不會執行。",false);
                    style.muted(ui,"服務的唯讀標示不代表已獲得你的授權。");
                }
                if matches!(request.service.draft.transport,Transport::Stdio{..}) {
                    style.note(ui,"本機程式不是沙箱","程式從啟動時即可依你的權限讀寫檔案或連網；npx 等啟動器可能下載套件。只允許你信任的來源。",true);
                }else if request.service.draft.requires_private_http_approval() {
                    style.note(ui,"此連線未加密","內網 HTTP 會以明文傳送憑證與工具資料，請只在可信任網路使用。",true);
                }
            });
        style.scroll_hint(ui,body.content_size.y,body.inner_rect.height(),body.state.offset.y);
        let (deny,allow)=style.actions(ui,"拒絕","僅允許這一次",true);
        #[cfg(test)] ctx.data_mut(|data|data.insert_temp(egui::Id::new("mcp-approval-test-allow-rect"),allow.rect));
        if deny.clicked(){Some(false)}else if allow.clicked(){Some(true)}else{None}
    });
    if response.should_close() {
        Some(false)
    } else {
        response.inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn assert_equal_card_widths(output: &egui::FullOutput) {
        let widths = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Rect(rect) if rect.corner_radius.nw == 10 => Some(rect.rect.width()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(!widths.is_empty());
        let min = widths.iter().copied().fold(f32::INFINITY, f32::min);
        let max = widths.iter().copied().fold(0.0, f32::max);
        assert!(max - min < 1.1, "section cards should align: {widths:?}");
    }
    fn assert_action_visible(output: &egui::FullOutput, size: egui::Vec2, label: &str) {
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
        let (text, clip) = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text == label => {
                    Some((text, shape.clip_rect))
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing action {label} at {size:?}"));
        let rect = egui::Rect::from_min_size(text.pos, text.galley.size());
        assert!(
            viewport.contains_rect(rect),
            "action outside viewport: {label} {rect:?} {size:?}"
        );
        assert!(
            clip.contains_rect(rect),
            "action clipped: {label} {rect:?} {clip:?}"
        );
        assert!(!output.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .iter()
            .any(|command| matches!(command, egui::ViewportCommand::Close)));
    }

    #[test]
    fn restyled_approval_has_readable_empty_arguments_and_visible_explicit_actions() {
        for dark in [false, true] {
            for size in [
                egui::vec2(320.0, 480.0),
                egui::vec2(580.0, 640.0),
                egui::vec2(900.0, 760.0),
            ] {
                for testing in [false, true] {
                    let ctx = egui::Context::default();
                    let palette = theme::apply(
                        &ctx,
                        if dark { "dark" } else { "light" },
                        theme::DEFAULT_ACCENT_HEX,
                    );
                    preview::prepare_fonts(&ctx);
                    let (respond, _decision) = tokio::sync::oneshot::channel();
                    let request = ApprovalRequest {
                        action: if testing {
                            ApprovalAction::ConnectionTest
                        } else {
                            ApprovalAction::ToolCall
                        },
                        service: Service {
                            id: Uuid::new_v4(),
                            draft: draft("資料查詢服務"),
                            enabled: true,
                            allow_private_http: false,
                            secret_ref: None,
                        },
                        tool: "inspect_status".into(),
                        arguments: serde_json::json!({}),
                        conversation_id: Uuid::new_v4(),
                        turn_id: Uuid::new_v4(),
                        respond,
                    };
                    let mut frames = vec![];
                    for index in 0..5 {
                        frames.push(ctx.run(
                            egui::RawInput {
                                screen_rect: Some(egui::Rect::from_min_size(
                                    egui::Pos2::ZERO,
                                    size,
                                )),
                                time: Some(index as f64 / 4.0),
                                ..Default::default()
                            },
                            |ctx| {
                                assert_eq!(approval_dialog(ctx, &request), None);
                            },
                        ));
                    }
                    let output = frames.last().unwrap();
                    assert_equal_card_widths(output);
                    assert_action_visible(output, size, "拒絕");
                    assert_action_visible(output, size, "僅允許這一次");
                    let rect: egui::Rect = ctx
                        .data(|data| data.get_temp(egui::Id::new("mcp-approval-test-allow-rect")))
                        .unwrap();
                    assert!(egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(rect));
                    assert!(output.shapes.iter().any(|shape|matches!(&shape.shape,egui::Shape::Rect(shape) if shape.fill==palette.accent && shape.rect.contains(rect.center()))));
                    assert!(!output.shapes.iter().any(|shape|matches!(&shape.shape,egui::Shape::Text(text) if text.galley.job.text=="{}")));
                    if !testing && size.x >= 580.0 {
                        assert!(output.shapes.iter().any(|shape|matches!(&shape.shape,egui::Shape::Text(text) if text.galley.job.text=="此工具不需要額外參數。")));
                    }
                    if !testing && (size.x == 900.0 || (dark && size.x == 320.0)) {
                        preview::export(
                            &ctx,
                            &frames,
                            size,
                            &format!(
                                "mcp-approval-{}{}.png",
                                if dark { "dark" } else { "light" },
                                if size.x == 320.0 { "-compact" } else { "" }
                            ),
                            dark,
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn restyled_editor_groups_fields_masks_password_and_keeps_save_and_close_visible() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        for dark in [false, true] {
            for http in [false, true] {
                for size in [
                    egui::vec2(320.0, 480.0),
                    egui::vec2(580.0, 640.0),
                    egui::vec2(900.0, 900.0),
                ] {
                    let dir = tempfile::tempdir().unwrap();
                    let path = dir.path().join("mcp.json");
                    let secret_path = dir.path().join("secrets.json");
                    let mut manager = McpUi::with_store(McpStore::load_at(path.clone()));
                    let mut secrets = SecretStore::isolated_file_for_tests(secret_path.clone());
                    manager.review(Draft {
                        name: "我的工具服務".into(),
                        transport: if http {
                            Transport::Http {
                                url: "https://example.com/mcp".into(),
                            }
                        } else {
                            Transport::Stdio {
                                command: "node.exe".into(),
                                args: vec!["C:\\Tools\\mcp\\server.js".into()],
                            }
                        },
                        token_env: None,
                    });
                    manager.token = "synthetic-preview-password".into();
                    let ctx = egui::Context::default();
                    theme::apply(
                        &ctx,
                        if dark { "dark" } else { "light" },
                        theme::DEFAULT_ACCENT_HEX,
                    );
                    preview::prepare_fonts(&ctx);
                    let mut frames = vec![];
                    for index in 0..5 {
                        frames.push(ctx.run(
                            egui::RawInput {
                                screen_rect: Some(egui::Rect::from_min_size(
                                    egui::Pos2::ZERO,
                                    size,
                                )),
                                time: Some(index as f64 / 4.0),
                                ..Default::default()
                            },
                            |ctx| {
                                assert!(!manager.show(ctx, runtime.handle(), &mut secrets));
                            },
                        ));
                    }
                    let output = frames.last().unwrap();
                    assert_action_visible(output, size, "確認並儲存 MCP 服務");
                    assert_action_visible(output, size, "關閉");
                    assert_equal_card_widths(output);
                    assert!(egui::Rect::from_min_size(egui::Pos2::ZERO, size)
                        .contains_rect(manager.save_rect.unwrap()));
                    assert!(!output.shapes.iter().any(|shape|matches!(&shape.shape,egui::Shape::Text(text) if text.galley.job.text.contains("synthetic-preview-password"))));
                    assert!(!path.exists());
                    assert!(!secret_path.exists());
                    assert!(manager.test.is_none());
                    assert!(manager.store.servers.is_empty());
                    if http && (size.x == 900.0 || (dark && size.x == 320.0)) {
                        preview::export(
                            &ctx,
                            &frames,
                            size,
                            &format!(
                                "mcp-settings-{}{}.png",
                                if dark { "dark" } else { "light" },
                                if size.x == 320.0 { "-compact" } else { "" }
                            ),
                            dark,
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn long_approval_arguments_are_not_truncated_and_cannot_push_actions_offscreen() {
        let ctx = egui::Context::default();
        let (respond, _decision) = tokio::sync::oneshot::channel();
        let value = format!("{}END_OF_REVIEWED_PARAMETER", "long_argument_".repeat(500));
        let request = ApprovalRequest {
            action: ApprovalAction::ToolCall,
            service: Service {
                id: Uuid::new_v4(),
                draft: draft("Fixture"),
                enabled: true,
                allow_private_http: false,
                secret_ref: None,
            },
            tool: "write_fixture".into(),
            arguments: serde_json::json!({"text":value}),
            conversation_id: Uuid::new_v4(),
            turn_id: Uuid::new_v4(),
            respond,
        };
        let size = egui::vec2(580.0, 640.0);
        let mut output = None;
        for index in 0..5 {
            output = Some(ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    time: Some(index as f64 / 4.0),
                    ..Default::default()
                },
                |ctx| {
                    assert_eq!(approval_dialog(ctx, &request), None);
                },
            ));
        }
        let output = output.unwrap();
        assert!(output.shapes.iter().any(|shape|matches!(&shape.shape,egui::Shape::Text(text) if text.galley.job.text=="向下捲動以檢視更多內容")));
        assert_action_visible(&output, size, "拒絕");
        assert_action_visible(&output, size, "僅允許這一次");
        assert!(output.shapes.iter().any(|shape|matches!(&shape.shape,egui::Shape::Text(text) if text.galley.job.text.contains("END_OF_REVIEWED_PARAMETER"))),"full arguments must remain available for review");
    }
    #[test]
    fn local_setup_requires_explicit_http_approval_and_save_before_resuming_without_the_token() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mcp.json");
        let mut manager = McpUi::with_store(McpStore::load_at(path.clone()));
        let mut secrets = SecretStore::isolated_file_for_tests(dir.path().join("secrets.json"));
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let ctx = egui::Context::default();
        let conversation_id = Uuid::new_v4();
        let setup = crate::chat::mcp::parse_chat_setup(&crate::chat::mcp::onboarding_fixture(
            "http://10.0.0.12:2918/v1/mcp",
            "synthetic-local-setup-password",
        ))
        .unwrap()
        .unwrap();
        manager.review_local_setup(setup, conversation_id).unwrap();
        assert!(!manager.enabled);
        assert!(!manager.allow_private_http);
        let mut time = 0.0;
        for _ in 0..5 {
            assert!(!frame(
                &ctx,
                &mut manager,
                runtime.handle(),
                &mut secrets,
                time,
                vec![]
            ));
            time += 0.1;
        }
        assert!(!path.exists());
        assert!(!dir.path().join("secrets.json").exists());
        assert!(manager.take_confirmed_request().is_none());
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 1400.0),
                )),
                time: Some(time),
                ..Default::default()
            },
            |ctx| {
                assert!(!manager.show(ctx, runtime.handle(), &mut secrets));
            },
        );
        time += 0.1;
        assert!(!output.shapes.iter().any(|shape|matches!(&shape.shape,egui::Shape::Text(text) if text.galley.job.text.contains("synthetic-local-setup-password"))),"password must be masked in the review screen");
        let event = |pos, pressed| {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ]
        };
        let save = manager.save_rect.unwrap().center();
        for pressed in [true, false] {
            assert!(!frame(
                &ctx,
                &mut manager,
                runtime.handle(),
                &mut secrets,
                time,
                event(save, pressed)
            ));
            time += 0.1;
        }
        assert!(!path.exists());
        for _ in 0..12 {
            let (rect, clip) = manager.private_http_rect.unwrap();
            if clip.contains_rect(rect) {
                break;
            }
            frame(
                &ctx,
                &mut manager,
                runtime.handle(),
                &mut secrets,
                time,
                vec![
                    egui::Event::PointerMoved(clip.center()),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0.0, -160.0),
                        modifiers: Default::default(),
                    },
                ],
            );
            time += 0.2;
        }
        let (rect, clip) = manager.private_http_rect.unwrap();
        assert!(
            clip.contains_rect(rect),
            "permission checkbox must be reachable by scrolling"
        );
        for pressed in [true, false] {
            assert!(!frame(
                &ctx,
                &mut manager,
                runtime.handle(),
                &mut secrets,
                time,
                event(rect.center(), pressed)
            ));
            time += 0.1;
        }
        assert!(manager.allow_private_http);
        assert!(!path.exists());
        let save = manager.save_rect.unwrap().center();
        frame(
            &ctx,
            &mut manager,
            runtime.handle(),
            &mut secrets,
            time,
            event(save, true),
        );
        time += 0.1;
        assert!(frame(
            &ctx,
            &mut manager,
            runtime.handle(),
            &mut secrets,
            time,
            event(save, false)
        ));
        assert!(!manager.open);
        let saved = McpStore::load_at(path.clone());
        assert_eq!(saved.servers.len(), 1);
        assert!(saved.servers[0].enabled);
        assert!(saved.servers[0].allow_private_http);
        assert_eq!(
            saved.servers[0].token(&secrets).unwrap(),
            "synthetic-local-setup-password"
        );
        assert!(!std::fs::read_to_string(path)
            .unwrap()
            .contains("synthetic-local-setup-password"));
        let (target, request) = manager.take_confirmed_request().unwrap();
        assert_eq!(target, conversation_id);
        assert!(!request.contains("synthetic-local-setup-password"));
        assert!(request.contains("armor_capabilities"));
        assert!(request.contains(&saved.servers[0].id.to_string()));
        assert!(manager.take_confirmed_request().is_none());
        assert!(manager.test.is_none());
    }

    #[test]
    fn local_credential_setup_never_overwrites_an_existing_unsaved_draft() {
        let dir = tempfile::tempdir().unwrap();
        let mut manager = McpUi::with_store(McpStore::load_at(dir.path().join("mcp.json")));
        manager.review(draft("My current draft"));
        manager.token = "synthetic-current-secret".into();
        let setup = crate::chat::mcp::parse_chat_setup(&crate::chat::mcp::onboarding_fixture(
            "https://example.com/mcp",
            "synthetic-new-secret",
        ))
        .unwrap()
        .unwrap();
        assert!(manager.review_local_setup(setup, Uuid::new_v4()).is_err());
        assert_eq!(manager.name, "My current draft");
        assert_eq!(manager.token, "synthetic-current-secret");
        assert!(manager.take_confirmed_request().is_none());
    }
    #[test]
    fn connection_test_modal_requires_an_actual_allow_click_and_escape_denies() {
        for approve in [false, true] {
            let ctx = egui::Context::default();
            let (respond, _decision) = tokio::sync::oneshot::channel();
            let request = ApprovalRequest {
                action: ApprovalAction::ConnectionTest,
                service: Service {
                    id: Uuid::new_v4(),
                    draft: draft("Disabled test target"),
                    enabled: false,
                    allow_private_http: false,
                    secret_ref: None,
                },
                tool: "initialize → tools/list".into(),
                arguments: serde_json::json!({}),
                conversation_id: Uuid::new_v4(),
                turn_id: Uuid::new_v4(),
                respond,
            };
            let mut result = None;
            let mut run = |time, events| {
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(600.0, 700.0),
                        )),
                        time: Some(time),
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        result = approval_dialog(ctx, &request);
                    },
                );
                result
            };
            for index in 0..3 {
                assert_eq!(run(index as f64 / 60.0, vec![]), None);
            }
            if approve {
                let rect: egui::Rect = ctx
                    .data(|data| data.get_temp(egui::Id::new("mcp-approval-test-allow-rect")))
                    .unwrap();
                let pos = rect.center();
                assert_eq!(
                    run(
                        0.1,
                        vec![
                            egui::Event::PointerMoved(pos),
                            egui::Event::PointerButton {
                                pos,
                                button: egui::PointerButton::Primary,
                                pressed: true,
                                modifiers: Default::default()
                            }
                        ]
                    ),
                    None
                );
                assert_eq!(
                    run(
                        0.2,
                        vec![
                            egui::Event::PointerMoved(pos),
                            egui::Event::PointerButton {
                                pos,
                                button: egui::PointerButton::Primary,
                                pressed: false,
                                modifiers: Default::default()
                            }
                        ]
                    ),
                    Some(true)
                );
            } else {
                assert_eq!(
                    run(
                        0.1,
                        vec![egui::Event::Key {
                            key: egui::Key::Escape,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: Default::default()
                        }]
                    ),
                    Some(false)
                );
            }
        }
    }
    #[test]
    fn model_open_settings_preserves_unsaved_draft_and_never_saves_or_enables() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mcp.json");
        let mut manager = McpUi::with_store(McpStore::load_at(path.clone()));
        let service = Service {
            id: Uuid::new_v4(),
            draft: draft("Stored disabled"),
            enabled: false,
            allow_private_http: false,
            secret_ref: None,
        };
        manager.store.servers.push(service.clone());
        manager.open_settings(Some(service.id));
        assert!(manager.open);
        assert_eq!(manager.id, Some(service.id));
        assert!(!manager.enabled);
        assert!(!path.exists());
        manager.name = "My unsaved draft".into();
        manager.token = "synthetic-unsaved-password".into();
        manager.open_settings(Some(service.id));
        assert_eq!(manager.name, "My unsaved draft");
        assert_eq!(manager.token, "synthetic-unsaved-password");
        assert!(!path.exists());
        assert!(!manager.store.servers[0].enabled);
    }
    fn draft(name: &str) -> Draft {
        Draft {
            name: name.into(),
            transport: Transport::Http {
                url: "https://fixture.invalid/mcp".into(),
            },
            token_env: None,
        }
    }
    fn frame(
        ctx: &egui::Context,
        manager: &mut McpUi,
        runtime: &tokio::runtime::Handle,
        secrets: &mut SecretStore,
        time: f64,
        events: Vec<egui::Event>,
    ) -> bool {
        frame_at_size(
            ctx,
            manager,
            runtime,
            secrets,
            time,
            events,
            egui::vec2(900.0, 1400.0),
        )
    }
    fn frame_at_size(
        ctx: &egui::Context,
        manager: &mut McpUi,
        runtime: &tokio::runtime::Handle,
        secrets: &mut SecretStore,
        time: f64,
        events: Vec<egui::Event>,
        size: egui::Vec2,
    ) -> bool {
        let mut changed = false;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ctx| {
                changed = manager.show(ctx, runtime, secrets);
            },
        );
        changed
    }
    #[test]
    fn save_footer_remains_visible_and_clickable_with_a_scrolling_service_list() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mcp.json");
        let mut manager = McpUi::with_store(McpStore::load_at(path.clone()));
        let mut secrets = SecretStore::isolated_file_for_tests(dir.path().join("secrets.json"));
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let ctx = egui::Context::default();
        let size = egui::vec2(580.0, 640.0);
        for index in 0..8 {
            manager.store.servers.push(Service {
                id: Uuid::new_v4(),
                draft: draft(&format!("Stored service {index}")),
                enabled: false,
                allow_private_http: false,
                secret_ref: None,
            });
        }
        manager.review(draft("New reviewed service"));
        for index in 0..5 {
            assert!(!frame_at_size(
                &ctx,
                &mut manager,
                runtime.handle(),
                &mut secrets,
                index as f64 / 60.0,
                vec![],
                size
            ));
        }
        let rect = manager.save_rect.unwrap();
        let pos = rect.center();
        assert!(
            egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(rect),
            "save footer outside window: {rect:?}"
        );
        assert!(manager.save_clip_rect.unwrap().contains(pos));
        assert!(!path.exists());
        frame_at_size(
            &ctx,
            &mut manager,
            runtime.handle(),
            &mut secrets,
            0.2,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ],
            size,
        );
        assert!(
            frame_at_size(
                &ctx,
                &mut manager,
                runtime.handle(),
                &mut secrets,
                0.3,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: Default::default()
                    }
                ],
                size
            ),
            "footer click failed: {:?}",
            manager.feedback
        );
        let saved = McpStore::load_at(path);
        assert_eq!(saved.servers.len(), 9);
        assert!(saved.servers.iter().all(|s| !s.enabled));
        assert!(manager.test.is_none());
    }
    #[test]
    fn dialog_proposal_is_disabled_unsaved_and_real_save_click_is_required() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mcp.json");
        let mut manager = McpUi::with_store(McpStore::load_at(path.clone()));
        let mut secrets = SecretStore::isolated_file_for_tests(dir.path().join("secrets.json"));
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let ctx = egui::Context::default();
        manager.review(draft("Proposed service"));
        assert!(!manager.enabled);
        assert!(manager.store.servers.is_empty());
        for index in 0..3 {
            assert!(!frame(
                &ctx,
                &mut manager,
                runtime.handle(),
                &mut secrets,
                index as f64 / 60.0,
                vec![]
            ));
        }
        assert!(!path.exists());
        assert!(!dir.path().join("secrets.json").exists());
        let position = manager.save_rect.unwrap().center();
        assert!(
            manager.save_clip_rect.unwrap().contains(position),
            "save button outside scroll viewport: save={:?} clip={:?}",
            manager.save_rect,
            manager.save_clip_rect
        );
        frame(
            &ctx,
            &mut manager,
            runtime.handle(),
            &mut secrets,
            0.1,
            vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ],
        );
        let saved = frame(
            &ctx,
            &mut manager,
            runtime.handle(),
            &mut secrets,
            0.2,
            vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                },
            ],
        );
        assert!(
            saved,
            "save click failed: clicked_at={position:?} save={:?} clip={:?} open={} feedback={:?}",
            manager.save_rect, manager.save_clip_rect, manager.open, manager.feedback
        );
        let reloaded = McpStore::load_at(path);
        assert_eq!(reloaded.servers.len(), 1);
        assert!(!reloaded.servers[0].enabled);
        assert!(reloaded.servers[0].secret_ref.is_none());
        assert!(manager.test.is_none());
    }
    #[test]
    fn incoming_model_proposal_does_not_overwrite_a_users_unsaved_draft() {
        let dir = tempfile::tempdir().unwrap();
        let mut manager = McpUi::with_store(McpStore::load_at(dir.path().join("mcp.json")));
        manager.review(draft("My unsaved service"));
        manager.token = "synthetic-unsaved-password".into();
        manager.review(draft("Model proposal"));
        assert_eq!(manager.name, "My unsaved service");
        assert_eq!(manager.token, "synthetic-unsaved-password");
        assert_eq!(manager.pending.len(), 1);
        assert!(manager.store.servers.is_empty());
        for _ in 0..10 {
            manager.review(draft("Another model proposal"));
        }
        assert_eq!(manager.pending.len(), 4);
        assert!(manager.feedback.as_ref().unwrap().contains("已滿"));
        assert_eq!(manager.name, "My unsaved service");
    }
    #[test]
    fn tool_approval_neutral_frame_does_not_grant_permission_and_escape_denies() {
        let ctx = egui::Context::default();
        let (respond, mut decision) = tokio::sync::oneshot::channel();
        let request = ApprovalRequest {
            action: ApprovalAction::ToolCall,
            service: Service {
                id: Uuid::new_v4(),
                draft: draft("Approval fixture"),
                enabled: true,
                allow_private_http: false,
                secret_ref: None,
            },
            tool: "write_something".into(),
            arguments: serde_json::json!({"path":"synthetic.txt"}),
            conversation_id: Uuid::new_v4(),
            turn_id: Uuid::new_v4(),
            respond,
        };
        let mut result = None;
        for index in 0..2 {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(580.0, 440.0),
                    )),
                    time: Some(index as f64 / 60.0),
                    ..Default::default()
                },
                |ctx| {
                    result = approval_dialog(ctx, &request);
                },
            );
            assert!(result.is_none());
        }
        assert!(decision.try_recv().is_err());
        let _ = ctx.run(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Default::default(),
                }],
                ..Default::default()
            },
            |ctx| {
                result = approval_dialog(ctx, &request);
            },
        );
        assert_eq!(result, Some(false));
        request.respond.send(false).unwrap();
        assert_eq!(decision.try_recv().unwrap(), false);
    }
}
