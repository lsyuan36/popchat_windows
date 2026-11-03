//! Human-owned review UI. Rendering a proposal never saves/connects anything.
use crate::chat::{
    mcp::{
        ApprovalAction, ApprovalRequest, Draft, LocalSetup, McpStore, Service, Session, Transport,
    },
    store::SecretStore,
};
use eframe::egui;
use uuid::Uuid;

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
            self.feedback = Some("由對話提出的草稿，尚未保存、啟用或連線。請逐項檢查。".into());
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
        self.feedback=Some("已在本機擷取連線資訊，憑證只顯示在密碼欄，尚未儲存或送給模型。請核對目的地；確認後會保存、啟用並接續原要求，每次工具仍須確認。".into());
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
        let response=egui::Modal::new(egui::Id::new("mcp-manager")).show(ctx,|ui| {
            ui.set_width((ctx.screen_rect().width()-70.0).clamp(230.0,530.0));
            ui.horizontal(|ui|{ui.heading("MCP 服務");if ui.button("關閉").clicked(){close=true;}});
            egui::ScrollArea::vertical().id_salt("mcp-manager-content").max_height((ctx.screen_rect().height()-220.0).max(80.0)).show(ui,|ui| {
                ui.add(egui::Label::new("MCP 服務是外部程式／網站，不是沙箱。stdio 程式以你的使用者權限執行，啟動本身即可讀寫檔案或連網；npx 等啟動器可能自行下載套件。只啟用你信任的來源。每次工具呼叫仍需確認。").wrap());
                if let Some(error)=&self.store.load_error {ui.colored_label(ui.visuals().error_fg_color,error);}
                let mut allow=self.store.allow_chat_proposals;
                if ui.add_enabled(self.store.load_error.is_none(),egui::Checkbox::new(&mut allow,"允許模型提出 MCP 新增草稿（須支援工具呼叫）")).changed() {
                    match self.store.set_chat_proposals(allow){Ok(())=>{changed=true;self.feedback=Some("對話草稿選項已儲存；不代表允許自動新增或執行工具。".into());},Err(_)=>self.feedback=Some("無法保存 MCP 選項，原設定保留。".into())}
                }
                ui.label("不開啟此選項也可直接輸入 /mcp add {JSON}；請勿在聊天貼上金鑰。");
                ui.add(egui::Label::new("支援工具呼叫的聊天可用 mcp_status 查詢狀態、open_mcp_settings 開啟設定、test_mcp_service 請求連線測試；測試含停用服務，但每次必須確認，且不會啟用服務。設定保存後請再送出一則訊息，才會載入已啟用的工具。").wrap());
                ui.separator();
                for service in self.store.servers.clone() {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(format!("{} {}",if service.enabled{"已啟用"}else{"停用"},service.draft.name));
                        if ui.button("編輯").clicked(){self.fill(Some(service.id),service.draft.clone(),service.enabled);self.feedback=None;}
                        if ui.add_enabled(self.test.is_none(),egui::Button::new("測試（會啟動／連線）")).clicked() {
                            match service.token(secrets) {
                                Ok(token)=>{
                                    let (tx,rx)=std::sync::mpsc::channel();let target=service.clone();
                                    let handle=runtime.spawn(async move {let result=async {let hidden=token.clone();let mut session=Session::connect(&target,token).await?;let tools=session.tools().await?;Ok::<_,anyhow::Error>(tools.iter().filter_map(|t|t["name"].as_str()).map(|name|if hidden.is_empty(){name.to_owned()}else{name.replace(&hidden,"[REDACTED MCP TOKEN]")}).collect::<Vec<_>>())}.await.map_err(|e|e.to_string());let _=tx.send(result);});
                                    self.test=Some(TestJob {service:service.clone(),rx,handle});self.feedback=Some("正在測試 MCP；只探索工具，不執行 tools/call。".into());
                                }
                                Err(_)=>self.feedback=Some("MCP 憑證無法讀取，請重新設定。".into()),
                            }
                        }
                        if ui.button("刪除").clicked(){self.remove=Some(service.id);}
                    });
                }
                if self.test.is_some() && ui.button("取消連線測試").clicked(){self.cancel_test();self.feedback=Some("已取消連線測試。".into());}
                if let Some(id)=self.remove {
                    ui.colored_label(ui.visuals().warn_fg_color,"確認刪除此 MCP 服務與其憑證？");
                    ui.horizontal(|ui|{
                        if ui.button("確認刪除").clicked(){match self.store.remove(id,secrets){Ok(())=>{changed=true;self.cancel_test();self.feedback=Some("MCP 服務已移除。".into());},Err(_)=>self.feedback=Some("無法移除服務，原設定保留。".into())}self.remove=None;}
                        if ui.button("返回").clicked(){self.remove=None;}
                    });
                }
                ui.separator();
                if self.local_review.is_some(){
                    ui.add(egui::Label::new(format!("確認後將寫入：{}\n憑證存入本機憑證庫，不寫入這份 JSON。",self.store.settings_path().display())).wrap());
                }
                ui.horizontal_wrapped(|ui| {
                    if ui.button("新增／清空草稿").clicked(){self.fill(None,Draft {name:String::new(),transport:Transport::Http {url:String::new()},token_env:None},false);}
                    if !self.pending.is_empty() && ui.button(format!("放棄目前草稿並檢視下一個（{}）",self.pending.len())).clicked(){if let Some(draft)=self.pending.pop_front(){self.fill(None,draft,false);}}
                });
                ui.label(if self.id.is_some(){"編輯服務"}else{"新增草稿（不會覆寫同名服務）"});
                ui.label("名稱");self.has_draft|=ui.text_edit_singleline(&mut self.name).changed();
                ui.horizontal(|ui| {if ui.selectable_value(&mut self.http,true,"Streamable HTTP").clicked(){self.has_draft=true;}if ui.selectable_value(&mut self.http,false,"stdio").clicked(){self.has_draft=true;}});
                if self.http {ui.label("MCP URL（HTTPS；內網 HTTP 須另外確認；不接受 URL 金鑰）");if ui.text_edit_singleline(&mut self.url).changed(){self.has_draft=true;self.allow_private_http=false;}}
                else {ui.label("程式 command（直接啟動，不經 shell）");self.has_draft|=ui.text_edit_singleline(&mut self.command).changed();ui.label("args（JSON 字串陣列）");self.has_draft|=ui.text_edit_multiline(&mut self.args).changed();ui.label("將 token 傳給此環境變數（選填，例如 API_KEY）");self.has_draft|=ui.text_edit_singleline(&mut self.token_env).changed();}
                ui.label(if self.http{"Bearer token（選填；留空保留已存憑證）"}else{"憑證 token（僅供上方環境變數；留空保留已存憑證）"});
                self.has_draft|=ui.add(egui::TextEdit::singleline(&mut self.token).password(true)).changed();
                if self.id.is_some(){self.has_draft|=ui.checkbox(&mut self.clear_token,"移除原先 token").changed();}
                let private_http=self.draft().is_ok_and(|draft|draft.requires_private_http_approval());
                if private_http {
                    ui.add(egui::Label::new(egui::RichText::new("此位址使用未加密的內網 HTTP，Bearer token 與工具資料可能被網路上的其他人攔截。只在可信任網路使用；優先改用 HTTPS。").color(ui.visuals().warn_fg_color)).wrap());
                    let permission=ui.checkbox(&mut self.allow_private_http,"我了解風險，允許向這個內網 HTTP 位址傳送憑證與資料");
                    #[cfg(test)] {self.private_http_rect=Some((permission.rect,ui.clip_rect()));}
                    self.has_draft|=permission.changed();
                }
                if self.local_review.is_some(){ui.add(egui::Label::new("下方確認按鈕會保存並啟用此服務，接續原本的連線測試要求；憑證不會交給模型。").wrap());}
                else {self.has_draft|=ui.checkbox(&mut self.enabled,"啟用：允許在下一次送出聊天時連線／啟動此服務").changed();}
                ui.label("儲存不會立刻啟動；若本輪有進行中的聊天工具，變更後會停止該輪。HTTP 首版只支援手動 Bearer token，不會自動 OAuth 登入。");
                if let Some(text)=&self.feedback {ui.separator();ui.add(egui::Label::new(text).wrap());}
            });
            // Keep the explicit save action reachable even when the service
            // list, help text or an error makes the editor scroll.
            ui.separator();
                let allow_save=self.store.load_error.is_none() && self.draft().is_ok_and(|draft|!draft.requires_private_http_approval() || self.allow_private_http);
                let save=ui.add_enabled(allow_save,egui::Button::new(if self.local_review.is_some(){"確認新增、啟用並繼續"}else{"確認並儲存 MCP 服務"}));
                #[cfg(test)] {self.save_rect=Some(save.rect);self.save_clip_rect=Some(ui.clip_rect());}
                if save.clicked() {
                    let result=self.draft().and_then(|draft|self.store.save_with_http_permission(self.id,draft,self.enabled || self.local_review.is_some(),(!self.token.is_empty()).then_some(self.token.as_str()),self.clear_token,self.allow_private_http,secrets));
                    match result {
                        Ok(id)=>{
                            changed=true;self.id=Some(id);self.token.clear();self.clear_token=false;self.has_draft=false;self.cancel_test();
                            if let Some(mut review)=self.local_review.take(){
                                review.request.push_str(&format!("\n已確認的 MCP 服務 ID：{id}；請只測試／使用此服務，不要使用先前訊息中的其他目的地。"));
                                self.confirmed_request=Some((review.conversation_id,review.request));
                                close=true;
                            }
                            self.feedback=Some("MCP 服務已儲存；每次工具執行仍須確認。".into());
                        }
                        Err(error)=>self.feedback=Some(format!("未能儲存 MCP：{error}")),
                    }
                }
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
    let response=egui::Modal::new(egui::Id::new("mcp-tool-approval")).show(ctx,|ui|{
        ui.set_width((ctx.screen_rect().width()-70.0).clamp(220.0,500.0));
        ui.heading(if testing {"允許這次 MCP 連線測試？"} else {"允許這次 MCP 工具執行？"});
        ui.label(format!("服務：{}\n工具：{}",request.service.draft.name,request.tool));
        match &request.service.draft.transport {
            Transport::Http {url}=>{ui.add(egui::Label::new(format!("目的地：{url}")).wrap());}
            Transport::Stdio {command,args}=>{ui.add(egui::Label::new(format!("程式：{command}\nargs：{}",serde_json::to_string(args).unwrap_or_default())).wrap());}
        }
        ui.add(egui::Label::new(if testing {"此操作會連線／啟動服務，只做初始化與工具探索，不執行 tools/call，也不保存或啟用服務。stdio 不是沙箱，程式從啟動時即可讀寫或連網，npx 等啟動器可能下載套件。只允許你信任的來源；拒絕不會連線。"} else {"以下參數由模型提出，可能包含你的對話資料。工具可能寫檔、執行程式或傳送資料；服務的唯讀標示不會自動授權。拒絕不會執行。"}).wrap());
        egui::ScrollArea::vertical().max_height((ctx.screen_rect().height()-300.0).clamp(60.0,220.0)).show(ui,|ui|{ui.add(egui::Label::new(egui::RichText::new(serde_json::to_string_pretty(&request.arguments).unwrap_or_default()).monospace()).wrap());});
        ui.horizontal(|ui|{
            let deny=ui.button("拒絕");
            let allow=ui.button("僅允許這一次");
            #[cfg(test)] ctx.data_mut(|data|data.insert_temp(egui::Id::new("mcp-approval-test-allow-rect"),allow.rect));
            if deny.clicked(){Some(false)}else if allow.clicked(){Some(true)}else{None}
        }).inner
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
