//! Human-owned skills controls. Rendering is inert; only explicit clicks save
//! settings or scan sources. No real user folder is read by headless tests.
use super::theme;
use crate::chat::skills::{Scan, SkillStore, SkillTurn};
use eframe::egui;
use std::path::PathBuf;

pub struct SkillsUi {
    pub store: SkillStore,
    pub open: bool,
    scan: Scan,
    scanned: bool,
    path: String,
    project: bool,
    filter: String,
    feedback: Option<String>,
    remove: Option<usize>,
    scan_job: Option<std::sync::mpsc::Receiver<Scan>>,
}
impl Default for SkillsUi {
    fn default() -> Self {
        Self::new(SkillStore::load())
    }
}
impl SkillsUi {
    pub fn new(store: SkillStore) -> Self {
        Self {
            store,
            open: false,
            scan: Scan::default(),
            scanned: false,
            path: String::new(),
            project: false,
            filter: String::new(),
            feedback: None,
            remove: None,
            scan_job: None,
        }
    }
    /// Per-turn discovery runs in the existing background chat task. The UI
    /// never caches a read permission beyond its content-bound store grant.
    pub fn snapshot_store(&self) -> SkillStore {
        self.store.clone()
    }
    pub fn snapshot(&self) -> SkillTurn {
        self.store.snapshot(&self.scan)
    }
    fn rescan(&mut self) {
        // Replacing the receiver invalidates an older source snapshot. A late
        // scan can never reintroduce a disabled/removed source into this UI.
        let (tx, rx) = std::sync::mpsc::channel();
        let store = self.store.clone();
        self.scan = Scan::default();
        self.scanned = false;
        match std::thread::Builder::new()
            .name("popchat-skills-scan".into())
            .spawn(move || {
                let _ = tx.send(store.scan());
            }) {
            Ok(_) => self.scan_job = Some(rx),
            Err(_) => {
                self.scan_job = None;
                self.feedback = Some("無法開始技能掃描，請稍後再試。".into());
            }
        }
    }
    fn report(&mut self, result: anyhow::Result<()>) -> bool {
        match result {
            Ok(()) => {
                self.feedback = Some("設定已保存；下一次送出聊天起生效。".into());
                true
            }
            Err(_) => {
                self.feedback = Some(
                    "無法保存：請檢查資料夾、技能是否變更、來源數量或設定檔權限；原設定保留。"
                        .into(),
                );
                false
            }
        }
    }
    /// Returns true on persisted changes so the owner cancels an old tool turn.
    pub fn show(&mut self, ctx: &egui::Context) -> bool {
        if !self.open {
            return false;
        }
        if let Some(job) = &self.scan_job {
            match job.try_recv() {
                Ok(scan) => {
                    self.scan = scan;
                    self.scanned = true;
                    self.scan_job = None;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.scan_job = None;
                    self.feedback = Some("技能掃描未完成。".into());
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(60))
                }
            }
        }
        let palette = theme::current_palette(ctx);
        let border = if palette.dark {
            egui::Color32::from_white_alpha(22)
        } else {
            theme::LIGHT_BORDER
        };
        let mut changed = false;
        let mut close = false;
        let modal=super::surface::modal(ctx,egui::Id::new("skills-manager"),
                egui::Frame::popup(&ctx.style()).fill(theme::panel_fill(palette.dark))
                .corner_radius(14).inner_margin(20).stroke(egui::Stroke::new(1.0_f32,border)))
            .show(ctx,|ui| {
                ui.set_width(super::surface::dialog_content_width(ctx,640.0,20.0));
                ui.spacing_mut().item_spacing=egui::vec2(8.0,8.0);
                ui.heading("Skills 技能設定");
                ui.add(egui::Label::new("啟用前請確認來源可信任。啟用後，模型可讀取並傳送該技能的全文及目錄內文字參考檔；不會執行腳本，也不會取得額外 MCP 權限。").wrap());
                // Frame padding, footer/hint/gaps and 26pt native clearance on
                // both sides: the modal must not overhang the parent card.
                let body_height=(ctx.screen_rect().height()-ui.min_size().y-170.0).max(40.0);
                let scroll=egui::ScrollArea::vertical().id_salt("skills-content")
                    .max_height(body_height).show(ui,|ui| {
                        if let Some(error)=&self.store.load_error {ui.colored_label(ui.visuals().error_fg_color,error);}
                        ui.add_enabled_ui(self.store.load_error.is_none(),|ui| {
                            ui.strong("技能來源");
                            for (index,source) in self.store.sources().to_vec().iter().enumerate() {
                                card(ui,palette.dark,|ui| {
                                    let mut enabled=source.enabled;
                                    if ui.checkbox(&mut enabled,&source.name).changed() {
                                        let result=self.store.set_source_enabled(index,enabled);
                                        if self.report(result) {changed=true;self.rescan();}
                                    }
                                    ui.add(egui::Label::new(egui::RichText::new(source.path.display().to_string()).monospace().size(11.0)).wrap());
                                    if ui.small_button("移除此來源").clicked() {self.remove=Some(index);}
                                });
                            }
                            if let Some(index)=self.remove {
                                ui.add(egui::Label::new("確認移除此來源？只移除 PopChat 設定，不刪除檔案；所有技能需重新啟用。").wrap());
                                ui.horizontal_wrapped(|ui| {
                                    if ui.button("確認移除").clicked() {
                                        let result=self.store.remove_source(index);
                                        if self.report(result) {changed=true;self.rescan();}
                                        self.remove=None;
                                    }
                                    if ui.button("返回").clicked() {self.remove=None;}
                                });
                            }
                            egui::CollapsingHeader::new("加入專案／自訂來源").show(ui,|ui| {
                                ui.add(egui::TextEdit::singleline(&mut self.path).desired_width(f32::INFINITY).hint_text("絕對資料夾路徑"));
                                ui.checkbox(&mut self.project,"這是專案根目錄（只讀取 .opencode／.claude／.agents 下的 skills）");
                                if ui.button("確認加入來源").clicked() {
                                    let result=self.store.add_source(PathBuf::from(self.path.trim()),self.project);
                                    if self.report(result) {changed=true;self.path.clear();self.rescan();}
                                }
                            });
                            ui.add_space(4.0);
                            let scan=ui.add_enabled(self.scan_job.is_none(),egui::Button::new("掃描／重新整理技能"));
                            #[cfg(test)] ctx.data_mut(|data|data.insert_temp(egui::Id::new("skills-scan-test-rect"),scan.rect));
                            if scan.clicked() {self.rescan();}
                            ui.small("未存在的路徑會略過；不搜尋任意專案、不修改其他工具設定。");
                            if self.scan_job.is_some() {ui.label("正在本機掃描技能…");}
                            else if !self.scanned {ui.label("請按「掃描／重新整理技能」讀取來源。新技能一律預設停用。");}
                            ui.separator();
                            let active=self.scan.skills.iter().filter(|s|self.store.enabled(s)).count();
                            ui.strong(format!("已找到 {} 個技能 · 已啟用 {active}",self.scan.skills.len()));
                            if active>self.snapshot().len() {ui.colored_label(ui.visuals().warn_fg_color,"已啟用數量／說明超過單輪目錄上限；請停用部分技能，避免有技能無法提供給模型。");}
                            ui.add(egui::TextEdit::singleline(&mut self.filter).desired_width(f32::INFINITY).hint_text("搜尋名稱或說明"));
                            let filter=self.filter.to_lowercase();
                            for skill in self.scan.skills.clone().iter().filter(|s|filter.is_empty() || s.name.to_lowercase().contains(&filter) || s.description.to_lowercase().contains(&filter)) {
                                card(ui,palette.dark,|ui| {
                                    let mut enabled=self.store.enabled(skill);
                                    let toggle=ui.add_enabled(skill.model_invocable,egui::Checkbox::new(&mut enabled,&skill.name));
                                    #[cfg(test)] ctx.data_mut(|data|data.insert_temp(egui::Id::new(("skill-enable-test",&skill.id)),toggle.rect));
                                    if toggle.changed() {
                                        let result=self.store.set_enabled(skill,enabled);changed|=self.report(result);
                                    }
                                    ui.add(egui::Label::new(&skill.description).wrap());
                                    ui.small(&skill.source_name);
                                    ui.add(egui::Label::new(egui::RichText::new(skill.file.display().to_string()).monospace().size(11.0)).wrap());
                                    if !skill.model_invocable {ui.small("技能宣告 disable-model-invocation；此版不提供手動技能指令，因此不會加入模型目錄。");}
                                });
                            }
                            for note in &self.scan.notices {ui.add(egui::Label::new(egui::RichText::new(note).small().color(ui.visuals().warn_fg_color)).wrap());}
                            if self.scanned && self.scan.skills.is_empty() {ui.label("沒有可用技能。請確認路徑及 SKILL.md 的 name／description frontmatter。");}
                            egui::CollapsingHeader::new("格式與相容範圍").show(ui,|ui| {
                                ui.add(egui::Label::new("支援最深 6 層 SKILL.md；OpenCode／自訂來源另支援根目錄 *.md。使用真正 YAML 解析器；hooks、allowed-tools、context:fork、shell 動態替換不會執行。每檔最多 32 KiB，不接受符號連結或 junction。技能修改後須重新掃描並啟用；同名技能依完整路徑區分，不互相覆寫。").wrap());
                                ui.add(egui::Label::new("只支援 PopChat 的工具呼叫回合，不是完整 OpenCode／Codex／Claude 執行環境。Codex app-server 不注入這些工具。指引若需要 shell、寫檔等未提供能力，模型應說明限制。").wrap());
                            });
                        });
                        if let Some(feedback)=&self.feedback {ui.add(egui::Label::new(feedback).wrap());}
                    });
                #[cfg(test)] ctx.data_mut(|data|data.insert_temp(egui::Id::new("skills-scroll-test-offset"),scroll.state.offset.y));
                if scroll.content_size.y>scroll.inner_rect.height()+1.0 {ui.small("捲動以檢視更多來源與技能");}
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui| {
                    if ui.add_sized([90.0,34.0],egui::Button::new("關閉")).clicked() {close=true;}
                });
            });
        if close || modal.should_close() {
            self.open = false;
        }
        changed
    }
}
fn card(ui: &mut egui::Ui, dark: bool, body: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(if dark {
            egui::Color32::from_rgb(35, 38, 48)
        } else {
            theme::LIGHT_RAISED
        })
        .inner_margin(12)
        .corner_radius(9)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            body(ui);
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::skills::Source;
    fn fixture() -> (tempfile::TempDir, SkillsUi) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("skills");
        std::fs::create_dir_all(root.join("test")).unwrap();
        std::fs::write(
            root.join("test/SKILL.md"),
            "---\nname: Fixture Skill\ndescription: Fake skill instructions\n---\nINERT_BODY",
        )
        .unwrap();
        let store = SkillStore::load_at(
            dir.path().join("skills.json"),
            vec![Source {
                name: "Fixture source".into(),
                path: root,
                enabled: true,
                root_markdown: false,
            }],
        );
        let mut manager = SkillsUi::new(store);
        manager.open = true;
        (dir, manager)
    }
    fn frame(
        ctx: &egui::Context,
        manager: &mut SkillsUi,
        size: egui::Vec2,
        events: Vec<egui::Event>,
    ) -> (egui::FullOutput, bool) {
        // Geometry/color snapshots should not depend on Area's fade-in clock.
        ctx.style_mut(|style| style.animation_time = 0.0);
        let mut changed = false;
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .frame(super::super::surface::canvas_frame())
                    .show(ctx, |ui| {
                        super::super::surface::card_frame(ui.visuals().dark_mode).show(ui, |ui| {
                            ui.set_min_size(ui.available_size());
                            ui.label("PopChat settings - offline fixture");
                        });
                    });
                changed = manager.show(ctx);
            },
        );
        (output, changed)
    }
    fn click(
        ctx: &egui::Context,
        manager: &mut SkillsUi,
        size: egui::Vec2,
        position: egui::Pos2,
    ) -> bool {
        frame(
            ctx,
            manager,
            size,
            vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        frame(
            ctx,
            manager,
            size,
            vec![egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        )
        .1
    }
    #[test]
    fn neutral_render_never_scans_saves_or_enables_and_close_remains_visible() {
        for dark in [false, true] {
            for size in [egui::vec2(320.0, 480.0), egui::vec2(760.0, 820.0)] {
                let (dir, mut manager) = fixture();
                let ctx = egui::Context::default();
                theme::apply(
                    &ctx,
                    if dark { "dark" } else { "light" },
                    theme::DEFAULT_ACCENT_HEX,
                );
                crate::ui::mcp::preview::prepare_fonts(&ctx);
                let mut frames = vec![];
                for _ in 0..5 {
                    let (output, changed) = frame(&ctx, &mut manager, size, vec![]);
                    assert!(!changed);
                    frames.push(output);
                }
                assert!(!manager.scanned);
                assert!(manager.scan_job.is_none());
                assert!(manager.scan.skills.is_empty());
                assert!(!dir.path().join("skills.json").exists());
                let output = frames.last().unwrap();
                let close = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.job.text == "關閉" => Some((
                            egui::Rect::from_min_size(text.pos, text.galley.size()),
                            shape.clip_rect,
                        )),
                        _ => None,
                    })
                    .unwrap();
                assert!(egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(close.0));
                assert!(close.1.contains_rect(close.0));
                let native_safe = egui::Rect::from_min_size(egui::Pos2::ZERO, size).shrink(24.0);
                let outer = crate::ui::mcp::preview::rect_shapes(output)
                    .into_iter()
                    .find(|rect| {
                        rect.corner_radius == egui::CornerRadius::same(14)
                            && rect.fill == theme::panel_fill(dark)
                    })
                    .map(|rect| rect.rect)
                    .unwrap_or_else(|| {
                        panic!(
                            "missing stable modal frame: {dark} {size:?}; rects={:?}",
                            crate::ui::mcp::preview::rect_shapes(output)
                        )
                    });
                assert!(
                    native_safe.contains_rect(outer),
                    "modal/shadow need breathing room inside the parent card: {size:?} {outer:?}"
                );
                crate::ui::mcp::preview::export(
                    &ctx,
                    &frames,
                    size,
                    &format!(
                        "skills-{}{}.png",
                        if dark { "dark" } else { "light" },
                        if size.x == 320.0 { "-compact" } else { "" }
                    ),
                    dark,
                );
            }
        }
    }
    #[test]
    fn explicit_enable_click_persists_while_scan_itself_is_read_only() {
        let (dir, mut manager) = fixture();
        manager.scan = manager.store.scan();
        manager.scanned = true;
        let skill = manager.scan.skills[0].clone();
        let ctx = egui::Context::default();
        crate::ui::mcp::preview::prepare_fonts(&ctx);
        let size = egui::vec2(900.0, 1100.0);
        for _ in 0..5 {
            assert!(!frame(&ctx, &mut manager, size, vec![]).1);
        }
        assert!(!manager.store.enabled(&skill));
        assert!(!dir.path().join("skills.json").exists());
        let rect: egui::Rect = ctx
            .data(|data| data.get_temp(egui::Id::new(("skill-enable-test", &skill.id))))
            .unwrap();
        assert!(click(&ctx, &mut manager, size, rect.center()));
        assert!(manager.store.enabled(&skill));
        let reloaded = SkillStore::load_at(dir.path().join("skills.json"), vec![]);
        assert!(reloaded.enabled(&skill));
        assert!(manager.feedback.as_ref().unwrap().contains("下一次"));
    }
    #[test]
    fn late_scan_cannot_restore_removed_sources() {
        let (_dir, mut manager) = fixture();
        let old_scan = manager.store.scan();
        let (old_tx, old_rx) = std::sync::mpsc::channel();
        manager.scan_job = Some(old_rx);
        manager.store.set_source_enabled(0, false).unwrap();
        manager.rescan();
        assert!(old_tx.send(old_scan).is_err());
        let scan = manager
            .scan_job
            .take()
            .unwrap()
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        assert!(scan.skills.is_empty());
        assert!(manager.snapshot().is_empty());
    }

    #[test]
    fn long_scrolled_skill_lists_keep_the_native_margin_transparent_and_the_footer_visible() {
        use super::super::mcp::preview;
        for dark in [false, true] {
            for size in [egui::vec2(320.0, 480.0), egui::vec2(720.0, 650.0)] {
                let (dir, mut manager) = fixture();
                let root = manager.store.sources()[0].path.clone();
                for index in 0..24 {
                    let folder = root.join(format!("synthetic-{index:02}"));
                    std::fs::create_dir(&folder).unwrap();
                    std::fs::write(folder.join("SKILL.md"),format!("---\nname: Synthetic Skill {index:02}\ndescription: >\n  {}\n---\nNEVER_PRELOADED_BODY", "Use this synthetic instruction pack for offline layout verification. It has long English descriptions, nested reference paths, and deliberately repeated information to exercise wrapping and scrolling without reading any real user data. ".repeat(2))).unwrap();
                }
                manager.scan = manager.store.scan();
                manager.scanned = true;
                assert_eq!(manager.scan.skills.len(), 25);
                let ctx = egui::Context::default();
                theme::apply(
                    &ctx,
                    if dark { "dark" } else { "light" },
                    theme::DEFAULT_ACCENT_HEX,
                );
                preview::prepare_fonts(&ctx);
                let mut frames = vec![];
                for _ in 0..5 {
                    let (output, changed) = frame(&ctx, &mut manager, size, vec![]);
                    assert!(!changed);
                    frames.push(output);
                }
                let pointer = egui::pos2(size.x * 0.5, size.y * 0.5);
                let (output, changed) = frame(
                    &ctx,
                    &mut manager,
                    size,
                    vec![
                        egui::Event::PointerMoved(pointer),
                        egui::Event::MouseWheel {
                            unit: egui::MouseWheelUnit::Point,
                            delta: egui::vec2(0.0, -1200.0),
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
                assert!(!changed);
                frames.push(output);
                for _ in 0..8 {
                    let (output, changed) = frame(&ctx, &mut manager, size, vec![]);
                    assert!(!changed);
                    frames.push(output);
                }
                let output = frames.last().unwrap();
                let close = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.job.text == "關閉" => Some((
                            egui::Rect::from_min_size(text.pos, text.galley.size()),
                            shape.clip_rect,
                        )),
                        _ => None,
                    })
                    .unwrap();
                assert!(egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(close.0));
                assert!(close.1.contains_rect(close.0));
                let native_safe = egui::Rect::from_min_size(egui::Pos2::ZERO, size).shrink(24.0);
                let outer = preview::rect_shapes(output)
                    .into_iter()
                    .find(|rect| {
                        rect.corner_radius == egui::CornerRadius::same(14)
                            && rect.fill == theme::panel_fill(dark)
                    })
                    .map(|rect| rect.rect)
                    .unwrap();
                assert!(
                    native_safe.contains_rect(outer),
                    "scrolled modal must not overhang the root card: {size:?} {outer:?}"
                );
                let offset: f32 = ctx
                    .data(|data| data.get_temp(egui::Id::new("skills-scroll-test-offset")))
                    .unwrap();
                assert!(
                    offset > 400.0,
                    "long list must actually be scrolled into view: {offset}"
                );
                let image = preview::render(&ctx, &frames, size, [0; 4]);
                for (x, y) in [
                    (0, 0),
                    (size.x as u32 - 1, 0),
                    (0, size.y as u32 - 1),
                    (size.x as u32 - 1, size.y as u32 - 1),
                ] {
                    assert_eq!(image.get_pixel(x, y).0[3], 0);
                }
                assert!(!dir.path().join("skills.json").exists());
                assert!(manager.snapshot().is_empty());
                preview::export(
                    &ctx,
                    &frames,
                    size,
                    &format!(
                        "skills-scrolled-{}-{}.png",
                        if dark { "dark" } else { "light" },
                        size.x as u32
                    ),
                    dark,
                );
            }
        }
    }
}
