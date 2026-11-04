//! ChatView.swift's waiting row and settled reasoning disclosure.
//!
//! Live text is stripped only on the appended suffix. egui still lays out (and
//! compares/hashes) the accumulated text: this is NOT NSTextView's incremental
//! text-storage performance. The fixed viewport does prevent transcript reflow.
//! egui 0.32 exposes no native accessibilityReduceMotion flag. We honor its
//! existing `Style::animation_time == 0` convention, without inventing a setting.

use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

use eframe::egui;

use super::{markdown, panel::MathTextures, theme};

pub const COLLAPSED_LINES: usize = 3;
pub const EXPANDED_LINES: usize = 12;
pub const THINKING_FONT_SIZE: f32 = 11.5;
const BOTTOM_TOLERANCE: f32 = 8.0;
static NEXT_VIEW: AtomicU64 = AtomicU64::new(0);

/// Character-wise removal intentionally preserves underscores (`snake_case`).
pub fn strip_emphasis(chunk: &str) -> String {
    chunk
        .chars()
        .filter(|c| !matches!(c, '*' | '`' | '#'))
        .collect()
}

/// Use the PRE-append document dimensions, never the new, longer document.
pub fn is_at_bottom(offset: f32, content_height: f32, viewport_height: f32) -> bool {
    content_height <= viewport_height
        || offset + viewport_height >= content_height - BOTTOM_TOLERANCE
}

pub fn elapsed_seconds(elapsed: Duration) -> Option<u64> {
    (elapsed.as_secs() >= 8).then_some(elapsed.as_secs())
}

fn next_second(elapsed: Duration) -> Duration {
    Duration::from_secs(1) - Duration::from_nanos(u64::from(elapsed.subsec_nanos()))
}

#[derive(Default)]
struct LiveText {
    raw: String,
    stripped: String,
    #[cfg(test)]
    processed_bytes: usize,
}

impl LiveText {
    fn update(&mut self, text: &str) -> bool {
        if text == self.raw {
            return false;
        }
        let suffix = if let Some(suffix) = text.strip_prefix(self.raw.as_str()) {
            suffix
        } else {
            // Some providers replace their snapshot instead of appending.
            self.raw.clear();
            self.stripped.clear();
            text
        };
        #[cfg(test)]
        {
            self.processed_bytes += suffix.len();
        }
        self.stripped
            .extend(suffix.chars().filter(|c| !matches!(c, '*' | '`' | '#')));
        self.raw.push_str(suffix);
        true
    }
}

/// Retain one instance for the active turn; call `start_turn` even when the
/// previous turn never displayed a waiting row (e.g. an immediate first token).
pub struct WaitingView {
    started_at: Instant,
    expanded: bool,
    text: LiveText,
    instance_id: egui::Id,
    turn: u64,
    scroll_id: egui::Id,
    previous_content_height: f32,
    previous_viewport_height: f32,
}

impl Default for WaitingView {
    fn default() -> Self {
        let instance_id = egui::Id::new((
            "waiting-reasoning",
            NEXT_VIEW.fetch_add(1, Ordering::Relaxed),
        ));
        Self {
            started_at: Instant::now(),
            expanded: false,
            text: LiveText::default(),
            instance_id,
            turn: 0,
            scroll_id: instance_id.with(0_u64),
            previous_content_height: 0.0,
            previous_viewport_height: 0.0,
        }
    }
}

impl WaitingView {
    pub fn start_turn(&mut self) {
        self.started_at = Instant::now();
        self.expanded = false;
        self.text = LiveText::default();
        self.turn = self.turn.wrapping_add(1);
        self.scroll_id = self.instance_id.with(self.turn);
        self.previous_content_height = 0.0;
        self.previous_viewport_height = 0.0;
    }

    pub fn reset(&mut self) {
        self.start_turn();
    }

    pub fn show(&mut self, ui: &mut egui::Ui, status: &str, reasoning: Option<&str>) {
        let reasoning = reasoning.unwrap_or("");
        let first_text = self.text.raw.is_empty();
        let changed = self.text.update(reasoning);
        let thinking = !reasoning.is_empty();
        let elapsed = self.started_at.elapsed();
        let moving = ui.style().animation_time > 0.0;
        // A smooth 1.6-second pulse, bounded at 30 Hz rather than an unbounded
        // request_repaint loop. The clock is per turn, not reset per UI pass.
        let pulse = if moving {
            (1.0 - (elapsed.as_secs_f64() * std::f64::consts::TAU / 1.6).cos()) as f32 * 0.5
        } else {
            0.0
        };
        let secondary = ui.visuals().weak_text_color();
        ui.push_id(self.instance_id, |ui| {
            ui.add_space(4.0);
            let header = ui
                .horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let (dot, _) =
                        ui.allocate_exact_size(egui::vec2(7.0, 18.0), egui::Sense::hover());
                    ui.painter().circle_filled(
                        dot.center(),
                        3.5,
                        theme::current_palette(ui.ctx())
                            .accent
                            .gamma_multiply(1.0 - pulse * 0.75),
                    );
                    // Reserve timer and chevron space first, so status always stays
                    // one line, even in a narrow panel.
                    let timer = elapsed_seconds(elapsed).map(|n| format!("{n}s"));
                    let timer_width = timer.as_ref().map_or(0.0, |s| {
                        ui.fonts(|f| {
                            f.layout_no_wrap(s.clone(), egui::FontId::monospace(11.0), secondary)
                                .size()
                                .x
                                + 8.0
                        })
                    });
                    let width =
                        (ui.available_width() - timer_width - if thinking { 26.0 } else { 0.0 })
                            .max(1.0);
                    // add_sized centers its child UI even with Label::halign.
                    // Reserve the full status width, but lay its text beside
                    // the dot rather than halfway across the panel.
                    ui.allocate_ui_with_layout(
                        egui::vec2(width, 18.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.set_min_size(egui::vec2(width, 18.0));
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(if thinking { "Thinking" } else { status })
                                        .size(if thinking {
                                            13.0
                                        } else {
                                            super::typography::ACTIVITY_SIZE
                                        })
                                        .color(secondary.gamma_multiply(1.0 - pulse * 0.45)),
                                )
                                .halign(egui::Align::Min)
                                .truncate()
                                .selectable(false),
                            );
                        },
                    );
                    if let Some(timer) = timer {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(timer)
                                    .monospace()
                                    .size(11.0)
                                    .color(secondary.gamma_multiply(0.7)),
                            )
                            .selectable(false),
                        );
                    }
                    if thinking {
                        let help = if self.expanded {
                            "收合思考內容"
                        } else {
                            "展開思考內容"
                        };
                        let (rect, response) =
                            ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::click());
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::WidgetType::Button,
                                ui.is_enabled(),
                                help,
                            )
                        });
                        chevron(
                            ui,
                            rect.center(),
                            if self.expanded { -1.0 } else { 1.0 },
                            secondary,
                        );
                        if super::icons::hover_help(response, help).clicked() {
                            self.expanded = !self.expanded;
                        }
                    }
                })
                .response;
            let mut visible = ui.is_rect_visible(header.rect);
            if thinking {
                ui.add_space(5.0 - ui.spacing().item_spacing.y);
                let output = self.show_scroller(ui, changed, first_text);
                visible |= ui.is_rect_visible(output.inner_rect);
            }
            ui.add_space(4.0);
            if visible {
                ui.ctx().request_repaint_after(next_second(elapsed));
                if moving {
                    ui.ctx()
                        .request_repaint_after(Duration::from_secs_f64(1.0 / 30.0));
                }
            }
        });
    }

    fn show_scroller(
        &mut self,
        ui: &mut egui::Ui,
        changed: bool,
        first_text: bool,
    ) -> egui::scroll_area::ScrollAreaOutput<()> {
        let font = egui::FontId::proportional(super::typography::scaled(ui, THINKING_FONT_SIZE));
        let line_height = ui.fonts(|f| f.row_height(&font)).ceil();
        let lines = if self.expanded {
            EXPANDED_LINES
        } else {
            COLLAPSED_LINES
        };
        let height = line_height * lines as f32 + 4.0;
        let old_height = self.previous_content_height;
        let old_viewport = self.previous_viewport_height;
        let output = egui::ScrollArea::vertical()
            .id_salt(self.scroll_id)
            .max_height(height)
            .min_scrolled_height(height)
            .auto_shrink([false, false])
            .animated(false)
            // Selection owns drags; scrolling still works via wheel and bar.
            .scroll_source(egui::scroll_area::ScrollSource {
                drag: false,
                ..Default::default()
            })
            .show_viewport(ui, |ui, viewport| {
                let following =
                    first_text || is_at_bottom(viewport.min.y, old_height, old_viewport);
                let top = ui.cursor().top() + viewport.min.y;
                ui.set_clip_rect(ui.clip_rect().intersect(egui::Rect::from_min_size(
                    egui::pos2(ui.clip_rect().left(), top),
                    egui::vec2(ui.clip_rect().width(), height),
                )));
                ui.add_space(2.0);
                let color = ui.visuals().weak_text_color();
                let mut job = egui::text::LayoutJob::simple(
                    self.text.stripped.clone(),
                    font,
                    color,
                    ui.available_width(),
                );
                job.sections[0].format.line_height = Some(line_height);
                let (pos, galley, response) = egui::Label::new(job)
                    .wrap()
                    .selectable(true)
                    .sense(egui::Sense::click_and_drag() - egui::Sense::FOCUSABLE)
                    .layout_in_ui(ui);
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::Label,
                        ui.is_enabled(),
                        "Model thinking",
                    )
                });
                if ui.is_rect_visible(response.rect) {
                    let galley = top_fade(galley, pos.y, top, height * 0.22);
                    egui::text_selection::LabelSelectionState::label_text_selection(
                        ui,
                        &response,
                        pos,
                        galley,
                        color,
                        egui::Stroke::NONE,
                    );
                }
                ui.add_space(2.0);
                if changed && following {
                    ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
                }
            });
        self.previous_content_height = output.content_size.y;
        self.previous_viewport_height = output.inner_rect.height();
        output
    }
}

/// Vertex alpha only: no overpainted background, changed glyph metrics, or
/// invisible selection widgets. Rows outside the fade keep their shared meshes.
fn top_fade(mut galley: Arc<egui::Galley>, text_y: f32, top: f32, fade: f32) -> Arc<egui::Galley> {
    for placed in &mut Arc::make_mut(&mut galley).rows {
        let y = text_y + placed.pos.y;
        if y >= top + fade || y + placed.size.y < top {
            continue;
        }
        for vertex in &mut Arc::make_mut(&mut placed.row).visuals.mesh.vertices {
            let alpha = ((y + vertex.pos.y - top) / fade).clamp(0.0, 1.0);
            vertex.color = vertex.color.gamma_multiply(alpha);
        }
    }
    galley
}

fn chevron(ui: &egui::Ui, center: egui::Pos2, direction: f32, color: egui::Color32) {
    ui.painter().add(egui::Shape::line(
        vec![
            center + egui::vec2(-3.0, -1.5 * direction),
            center + egui::vec2(0.0, 1.5 * direction),
            center + egui::vec2(3.0, -1.5 * direction),
        ],
        egui::Stroke::new(1.0_f32, color),
    ));
}

fn recolor_text(shape: &mut egui::Shape, color: egui::Color32) {
    match shape {
        egui::Shape::Text(text) => text.override_text_color = Some(color),
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                recolor_text(shape, color);
            }
        }
        _ => {}
    }
}

/// Settled reasoning is collapsed per message ID. The existing prose renderer
/// supplies formatted selection/copy and math; subordination changes color ONLY.
pub fn show_disclosure(ui: &mut egui::Ui, id: egui::Id, text: &str, math: &mut MathTextures) {
    if text.is_empty() {
        return;
    }
    ui.push_id(id, |ui| {
        let key = id.with("reasoning-expanded");
        let mut expanded = ui.ctx().data(|d| d.get_temp::<bool>(key)).unwrap_or(false);
        let secondary = ui.visuals().weak_text_color();
        let label = ui.fonts(|f| {
            f.layout_no_wrap(
                "Reasoning".into(),
                egui::FontId::proportional(11.0),
                secondary,
            )
        });
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(label.size().x + 30.0, 18.0),
            egui::Sense::click(),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), "Reasoning")
        });
        if expanded {
            chevron(
                ui,
                rect.left_center() + egui::vec2(4.0, 0.0),
                1.0,
                secondary,
            );
        } else {
            let c = rect.left_center() + egui::vec2(4.0, 0.0);
            ui.painter().add(egui::Shape::line(
                vec![
                    c + egui::vec2(-1.5, -3.0),
                    c + egui::vec2(1.5, 0.0),
                    c + egui::vec2(-1.5, 3.0),
                ],
                egui::Stroke::new(1.0_f32, secondary),
            ));
        }
        // A tiny, font-independent two-lobed brain mark.
        let brain = rect.left_center() + egui::vec2(17.0, 0.0);
        for side in [-1.0, 1.0] {
            ui.painter().circle_stroke(
                brain + egui::vec2(side * 2.0, -1.0),
                3.0,
                egui::Stroke::new(0.8_f32, secondary),
            );
        }
        ui.painter().line_segment(
            [brain + egui::vec2(0.0, -4.0), brain + egui::vec2(0.0, 4.0)],
            egui::Stroke::new(0.8_f32, secondary),
        );
        ui.painter().galley(
            rect.left_center() + egui::vec2(30.0, -label.size().y / 2.0),
            label,
            secondary,
        );
        let help = if expanded {
            "收合模型思考內容"
        } else {
            "展開模型思考內容"
        };
        if super::icons::hover_help(response, help).clicked() {
            expanded = !expanded;
            ui.ctx().data_mut(|d| d.insert_temp(key, expanded));
        }
        if expanded {
            ui.add_space(6.0 - ui.spacing().item_spacing.y);
            let body = ui
                .horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.add_space(10.0);
                    ui.vertical(|ui| {
                        ui.visuals_mut().override_text_color = Some(secondary);
                        ui.visuals_mut().weak_text_color = Some(secondary);
                        ui.visuals_mut().widgets.noninteractive.fg_stroke.color = secondary;
                        ui.visuals_mut().hyperlink_color = secondary;
                        // Explicit syntax-highlighting/strong colors bypass the
                        // Visuals override. Recolor ONLY this renderer's text paint
                        // range, leaving its shared galleys, layout and hit testing
                        // untouched. Math receives the scoped secondary tint above.
                        let layer = ui.layer_id();
                        let start = ui.ctx().graphics_mut(|g| g.entry(layer).next_idx());
                        markdown::show_prose(ui, text, math);
                        ui.ctx().graphics_mut(|g| {
                            let list = g.entry(layer);
                            for index in start.0..list.next_idx().0 {
                                list.mutate_shape(egui::layers::ShapeIdx(index), |shape| {
                                    recolor_text(&mut shape.shape, secondary)
                                });
                            }
                        });
                    });
                })
                .response;
            ui.painter().line_segment(
                [body.rect.left_top(), body.rect.left_bottom()],
                egui::Stroke::new(1.0_f32, ui.visuals().widgets.noninteractive.bg_stroke.color),
            );
            ui.add_space(6.0);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_sizes_scale_thinking_glyphs_and_three_or_twelve_line_viewports_together() {
        for appearance in ["light", "dark"] {
            for width in [320.0, 520.0] {
                for size in [12, 14, 15, 20, 24] {
                    let ctx = context();
                    super::super::theme::apply(
                        &ctx,
                        appearance,
                        super::super::theme::DEFAULT_ACCENT_HEX,
                    );
                    let mut waiting = WaitingView::default();
                    let text = "Thinking text with wrapping and multiple rows. ".repeat(30);
                    for expanded in [false, true] {
                        waiting.expanded = expanded;
                        let mut expected_height = 0.0;
                        let mut raw = input(0.0, vec![]);
                        raw.screen_rect = Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 1600.0),
                        ));
                        let output = ctx.run(raw, |ctx| {
                            egui::CentralPanel::default().show(ctx, |ui| {
                                super::super::typography::apply(ui, size);
                                let font = egui::FontId::proportional(
                                    super::super::typography::scaled(ui, THINKING_FONT_SIZE),
                                );
                                expected_height = ui.fonts(|fonts| fonts.row_height(&font)).ceil()
                                    * if expanded { 12.0 } else { 3.0 }
                                    + 4.0;
                                waiting.show(ui, "Connecting", Some(&text));
                            });
                        });
                        assert_eq!(waiting.previous_viewport_height, expected_height);
                        let shapes = texts(&output);
                        let body = shapes
                            .iter()
                            .find(|shape| shape.galley.text() == text)
                            .unwrap();
                        assert!(
                            (body.galley.job.sections[0].format.font_id.size
                                - THINKING_FONT_SIZE * super::super::typography::scale(size))
                            .abs()
                                < 0.001
                        );
                        let header = shapes
                            .iter()
                            .find(|shape| shape.galley.text() == "Thinking")
                            .unwrap();
                        assert_eq!(header.galley.job.sections[0].format.font_id.size, 13.0);
                    }
                }
            }
        }
    }

    fn input(time: f64, events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(420.0, 1000.0),
            )),
            time: Some(time),
            events,
            ..Default::default()
        }
    }

    fn context() -> egui::Context {
        let ctx = egui::Context::default();
        ctx.style_mut(|style| style.animation_time = 0.0);
        ctx
    }

    #[test]
    fn waiting_tool_search_status_uses_small_type_without_shrinking_thinking_text() {
        for size in [12, 24] {
            for status in ["Searching the web", "Reading a page", "MCP tool discovery"] {
                let ctx = context();
                let mut waiting = WaitingView::default();
                let output = ctx.run(input(0.0, vec![]), |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        super::super::typography::apply(ui, size);
                        waiting.show(ui, status, None);
                    });
                });
                let status_shape = texts(&output)
                    .into_iter()
                    .find(|t| t.galley.job.text == status)
                    .unwrap();
                assert!(status_shape
                    .galley
                    .job
                    .sections
                    .iter()
                    .all(|s| s.format.font_id.size == super::super::typography::ACTIVITY_SIZE));
                let (thinking, _) = waiting_frame(&ctx, &mut waiting, "Actual reasoning content");
                let header = texts(&thinking)
                    .into_iter()
                    .find(|t| t.galley.job.text == "Thinking")
                    .unwrap();
                assert!(header
                    .galley
                    .job
                    .sections
                    .iter()
                    .all(|s| s.format.font_id.size == 13.0));
            }
        }
    }

    fn waiting_frame(
        ctx: &egui::Context,
        waiting: &mut WaitingView,
        text: &str,
    ) -> (egui::FullOutput, egui::Rect) {
        waiting_events_frame(ctx, waiting, text, 0.0, vec![])
    }

    fn waiting_events_frame(
        ctx: &egui::Context,
        waiting: &mut WaitingView,
        text: &str,
        time: f64,
        events: Vec<egui::Event>,
    ) -> (egui::FullOutput, egui::Rect) {
        let mut rect = egui::Rect::NOTHING;
        let output = ctx.run(input(time, events), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                rect = ui
                    .scope(|ui| waiting.show(ui, "Connecting", Some(text)))
                    .response
                    .rect;
            });
        });
        (output, rect)
    }

    fn scroller_frame(
        ctx: &egui::Context,
        waiting: &mut WaitingView,
        text: &str,
    ) -> egui::scroll_area::ScrollAreaOutput<()> {
        let first = waiting.text.raw.is_empty();
        let changed = waiting.text.update(text);
        let mut result = None;
        let _ = ctx.run(input(0.0, vec![]), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                result = Some(waiting.show_scroller(ui, changed, first));
            });
        });
        result.unwrap()
    }

    fn texts(output: &egui::FullOutput) -> Vec<&egui::epaint::TextShape> {
        fn collect<'a>(shape: &'a egui::Shape, out: &mut Vec<&'a egui::epaint::TextShape>) {
            match shape {
                egui::Shape::Text(shape) => out.push(shape),
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        collect(shape, out);
                    }
                }
                _ => {}
            }
        }
        let mut result = Vec::new();
        for shape in &output.shapes {
            collect(&shape.shape, &mut result);
        }
        result
    }

    #[test]
    fn waiting_status_stays_beside_the_dot_instead_of_centering_in_reserved_width() {
        let ctx = context();
        let mut waiting = WaitingView::default();
        for text in ["", "reasoning"] {
            let (output, rect) = waiting_frame(&ctx, &mut waiting, text);
            let label = texts(&output)
                .into_iter()
                .find(|shape| {
                    shape.galley.text()
                        == if text.is_empty() {
                            "Connecting"
                        } else {
                            "Thinking"
                        }
                })
                .unwrap();
            assert!(
                label.pos.x - rect.left() < 32.0,
                "status unexpectedly centered: {}",
                label.pos.x
            );
        }
    }

    #[test]
    fn appended_chunks_only_are_stripped_and_replacements_reset() {
        let mut cache = LiveText::default();
        let mut source = String::new();
        for _ in 0..1000 {
            source.push_str("**snake_case** `字` #\n");
            assert!(cache.update(&source));
        }
        assert_eq!(cache.processed_bytes, source.len());
        assert_eq!(cache.stripped, "snake_case 字 \n".repeat(1000));
        assert!(!cache.update(&source));
        assert_eq!(cache.processed_bytes, source.len());
        assert!(cache.update("# new_snapshot"));
        assert_eq!(cache.raw, "# new_snapshot");
        assert_eq!(cache.stripped, " new_snapshot");
        assert_eq!(cache.processed_bytes, source.len() + "# new_snapshot".len());
        assert!(cache.update(""));
        assert!(cache.stripped.is_empty());
        assert_eq!(strip_emphasis("**a_b** `c` #d"), "a_b c d");
    }

    #[test]
    fn exact_elapsed_threshold_and_repaint_boundary() {
        assert_eq!(elapsed_seconds(Duration::from_millis(7999)), None);
        assert_eq!(elapsed_seconds(Duration::from_secs(8)), Some(8));
        assert_eq!(elapsed_seconds(Duration::from_millis(8999)), Some(8));
        assert_eq!(elapsed_seconds(Duration::from_secs(9)), Some(9));
        assert_eq!(
            next_second(Duration::from_millis(8250)),
            Duration::from_millis(750)
        );
        let ctx = context();
        let mut waiting = WaitingView::default();
        waiting.started_at = Instant::now() - Duration::from_millis(7500);
        let (output, _) = waiting_frame(&ctx, &mut waiting, "");
        assert!(texts(&output)
            .iter()
            .any(|s| s.galley.text() == "Connecting"));
        assert!(!texts(&output).iter().any(|s| s.galley.text() == "7s"));
        waiting.started_at = Instant::now() - Duration::from_millis(8250);
        let (output, _) = waiting_frame(&ctx, &mut waiting, "");
        assert!(texts(&output).iter().any(|s| s.galley.text() == "8s"));
        let status = texts(&output)
            .into_iter()
            .find(|s| s.galley.text() == "Connecting")
            .unwrap();
        assert_eq!(
            status.galley.job.sections[0].format.color,
            ctx.style().visuals.weak_text_color()
        );
    }

    #[test]
    fn full_waiting_renderer_has_fixed_three_and_twelve_line_windows_even_at_100k() {
        let ctx = context();
        let mut waiting = WaitingView::default();
        let (_, short) = waiting_frame(&ctx, &mut waiting, "short");
        let line_height = ctx
            .fonts(|f| f.row_height(&egui::FontId::proportional(THINKING_FONT_SIZE)))
            .ceil();
        assert_eq!(waiting.previous_viewport_height, line_height * 3.0 + 4.0);
        let long = "word ".repeat(20_000);
        let (_, long_rect) = waiting_frame(&ctx, &mut waiting, &long);
        assert_eq!(short.height(), long_rect.height());
        assert_eq!(waiting.previous_viewport_height, line_height * 3.0 + 4.0);
        waiting.expanded = true;
        let (_, expanded_long) = waiting_frame(&ctx, &mut waiting, &long);
        assert_eq!(waiting.previous_viewport_height, line_height * 12.0 + 4.0);
        let (_, expanded_short) = waiting_frame(&ctx, &mut waiting, "short");
        assert_eq!(expanded_long.height(), expanded_short.height());
        assert_eq!(expanded_short.height() - short.height(), line_height * 9.0);
        assert_eq!(waiting.text.stripped, "short");
        // Marker-only chunks have an empty rendered string but still occupy the
        // same thinking window, rather than crashing or collapsing the row.
        waiting.expanded = false;
        let (_, markers) = waiting_frame(&ctx, &mut waiting, "**`#");
        assert_eq!(markers.height(), short.height());
        assert!(waiting.text.stripped.is_empty());
    }

    #[test]
    fn live_chevron_expands_instead_of_hiding_all_thinking() {
        let ctx = context();
        let mut waiting = WaitingView::default();
        let _ = waiting_frame(&ctx, &mut waiting, "thinking");
        let (output, collapsed) = waiting_frame(&ctx, &mut waiting, "thinking");
        let status = texts(&output)
            .into_iter()
            .find(|s| s.galley.text() == "Thinking")
            .unwrap();
        let pos = egui::pos2(
            collapsed.right() - 9.0,
            status.pos.y + status.galley.size().y / 2.0,
        );
        let _ = waiting_events_frame(
            &ctx,
            &mut waiting,
            "thinking",
            0.1,
            vec![egui::Event::PointerMoved(pos), pointer(pos, true)],
        );
        let (output, expanded) = waiting_events_frame(
            &ctx,
            &mut waiting,
            "thinking",
            0.2,
            vec![pointer(pos, false)],
        );
        assert!(waiting.expanded);
        assert!(expanded.height() > collapsed.height());
        assert!(texts(&output).iter().any(|s| s.galley.text() == "thinking"));
    }

    #[test]
    fn live_stripped_text_is_selectable_and_copyable() {
        let ctx = context();
        let mut waiting = WaitingView::default();
        let source = "**snake_case** and `text`";
        let expected = "snake_case and text";
        let _ = waiting_frame(&ctx, &mut waiting, source);
        let (output, _) = waiting_frame(&ctx, &mut waiting, source);
        let painted = texts(&output);
        let shape = painted
            .iter()
            .find(|s| s.galley.text() == expected)
            .unwrap();
        let cursor = |index| {
            shape.pos
                + shape
                    .galley
                    .pos_from_cursor(egui::text::CCursor::new(index))
                    .center()
                    .to_vec2()
        };
        let start = cursor(0) + egui::vec2(0.1, 0.0);
        let end = cursor(expected.chars().count()) - egui::vec2(0.1, 0.0);
        let mut commands = Vec::new();
        for (time, events) in [
            (
                0.2,
                vec![egui::Event::PointerMoved(start), pointer(start, true)],
            ),
            (0.3, vec![egui::Event::PointerMoved(end)]),
            (0.4, vec![egui::Event::PointerMoved(end)]),
            (0.5, vec![pointer(end, false)]),
            (0.6, vec![egui::Event::Copy]),
        ] {
            commands.extend(
                waiting_events_frame(&ctx, &mut waiting, source, time, events)
                    .0
                    .platform_output
                    .commands,
            );
        }
        let copies: Vec<_> = commands
            .iter()
            .filter_map(|c| match c {
                egui::OutputCommand::CopyText(text) => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(copies, [expected]);
    }

    #[test]
    fn production_scroll_follows_only_the_preappend_bottom() {
        assert!(is_at_bottom(0.0, 20.0, 50.0));
        assert!(is_at_bottom(42.0, 100.0, 50.0));
        assert!(!is_at_bottom(41.9, 100.0, 50.0));
        let ctx = context();
        let mut waiting = WaitingView::default();
        let source = "line\n".repeat(100);
        let initial = scroller_frame(&ctx, &mut waiting, &source);
        let max = initial.content_size.y - initial.inner_rect.height();
        assert!((initial.state.offset.y - max).abs() < 1.0);
        let mut state = initial.state;
        state.offset.y = 50.0;
        state.store(&ctx, initial.id);
        let appended = format!("{source}another line\n");
        let reading = scroller_frame(&ctx, &mut waiting, &appended);
        assert!(
            (reading.state.offset.y - 50.0).abs() < 1.0,
            "do not yank the reader"
        );
        let mut state = reading.state;
        state.offset.y = reading.content_size.y - reading.inner_rect.height() - 8.0;
        state.store(&ctx, reading.id);
        let following = scroller_frame(&ctx, &mut waiting, &format!("{appended}last\n"));
        assert!(
            (following.state.offset.y - (following.content_size.y - following.inner_rect.height()))
                .abs()
                < 1.0
        );
    }

    #[test]
    fn start_turn_resets_clock_expansion_caches_and_scroll_identity() {
        let mut waiting = WaitingView::default();
        let old = waiting.scroll_id;
        waiting.started_at = Instant::now() - Duration::from_secs(60);
        waiting.expanded = true;
        waiting.text.update("**old**");
        waiting.previous_content_height = 900.0;
        waiting.start_turn();
        assert_ne!(waiting.scroll_id, old);
        assert!(waiting.started_at.elapsed() < Duration::from_secs(1));
        assert!(!waiting.expanded);
        assert!(waiting.text.raw.is_empty() && waiting.text.stripped.is_empty());
        assert_eq!(waiting.text.processed_bytes, 0);
        assert_eq!(waiting.previous_content_height, 0.0);
    }

    #[test]
    fn fade_preserves_layout_and_changes_only_vertex_alpha() {
        let ctx = context();
        let _ = ctx.run(input(0.0, vec![]), |ctx| {
            let original = ctx.fonts(|f| {
                f.layout_no_wrap(
                    "reasoning".into(),
                    egui::FontId::proportional(11.5),
                    egui::Color32::GRAY,
                )
            });
            let faded = top_fade(original.clone(), 0.0, 0.0, 20.0);
            assert_eq!(original.rect, faded.rect);
            assert_eq!(original.job, faded.job);
            assert_eq!(original.rows[0].glyphs, faded.rows[0].glyphs);
            assert_ne!(
                original.rows[0].visuals.mesh.vertices,
                faded.rows[0].visuals.mesh.vertices
            );
        });
    }

    fn disclosure_frame(
        ctx: &egui::Context,
        math: &mut MathTextures,
        text: &str,
        time: f64,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        ctx.run(input(time, events), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                show_disclosure(ui, egui::Id::new("first"), text, math);
                show_disclosure(ui, egui::Id::new("second"), "other reasoning", math);
            });
        })
    }

    fn pointer(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn settled_is_local_collapsed_formatted_secondary_and_normal_size() {
        let ctx = context();
        let mut math = MathTextures::default();
        let source = "# Plan\n\n**bold** and *em* and `snake_case`\n\n```rust\nlet x = 1;\n```";
        let output = disclosure_frame(&ctx, &mut math, source, 0.0, vec![]);
        assert_eq!(
            texts(&output)
                .iter()
                .filter(|s| s.galley.text() == "Reasoning")
                .count(),
            2
        );
        assert!(!texts(&output).iter().any(|s| s.galley.text() == "Plan"));
        let header = texts(&output)
            .into_iter()
            .find(|s| s.galley.text() == "Reasoning")
            .unwrap();
        let pos = header.pos + header.galley.size() / 2.0;
        let _ = disclosure_frame(
            &ctx,
            &mut math,
            source,
            0.1,
            vec![egui::Event::PointerMoved(pos), pointer(pos, true)],
        );
        let output = disclosure_frame(&ctx, &mut math, source, 0.2, vec![pointer(pos, false)]);
        let painted = texts(&output);
        assert!(painted.iter().any(|s| s.galley.text() == "Plan"));
        assert!(painted
            .iter()
            .any(|s| s.galley.text() == "bold and em and snake_case"));
        assert!(!painted.iter().any(|s| s.galley.text() == "other reasoning"));
        let prose = painted
            .iter()
            .find(|s| s.galley.text() == "bold and em and snake_case")
            .unwrap();
        assert_eq!(
            prose.override_text_color,
            Some(ctx.style().visuals.weak_text_color())
        );
        assert!(painted
            .iter()
            .filter(|s| s.galley.text() != "Reasoning")
            .all(|s| s.override_text_color == Some(ctx.style().visuals.weak_text_color())));
        let baseline_ctx = context();
        let baseline = baseline_ctx.run(input(0.0, vec![]), |ctx| {
            egui::CentralPanel::default()
                .show(ctx, |ui| markdown::show_prose(ui, source, &mut math));
        });
        for actual in painted.iter().filter(|s| s.galley.text() != "Reasoning") {
            let baseline_texts = texts(&baseline);
            let normal = baseline_texts
                .iter()
                .find(|s| s.galley.text() == actual.galley.text())
                .unwrap();
            let sizes = |s: &egui::epaint::TextShape| {
                s.galley
                    .job
                    .sections
                    .iter()
                    .map(|s| s.format.font_id.size)
                    .collect::<Vec<_>>()
            };
            assert_eq!(sizes(actual), sizes(normal));
        }
    }

    #[test]
    fn settled_formatted_selection_emits_plain_copy_without_system_clipboard() {
        let ctx = context();
        let mut math = MathTextures::default();
        let source = "**bold** and *em* and `snake_case`";
        ctx.data_mut(|d| d.insert_temp(egui::Id::new("first").with("reasoning-expanded"), true));
        let _ = disclosure_frame(&ctx, &mut math, source, 0.0, vec![]);
        let output = disclosure_frame(&ctx, &mut math, source, 0.1, vec![]);
        let painted = texts(&output);
        let shape = painted
            .iter()
            .find(|s| s.galley.text() == "bold and em and snake_case")
            .unwrap();
        let cursor = |index| {
            shape.pos
                + shape
                    .galley
                    .pos_from_cursor(egui::text::CCursor::new(index))
                    .center()
                    .to_vec2()
        };
        let start = cursor(0) + egui::vec2(0.1, 0.0);
        let end = cursor(shape.galley.text().chars().count()) - egui::vec2(0.1, 0.0);
        let mut commands = Vec::new();
        for (time, events) in [
            (
                0.2,
                vec![egui::Event::PointerMoved(start), pointer(start, true)],
            ),
            (0.3, vec![egui::Event::PointerMoved(end)]),
            (0.4, vec![egui::Event::PointerMoved(end)]),
            (0.5, vec![pointer(end, false)]),
            (0.6, vec![egui::Event::Copy]),
        ] {
            commands.extend(
                disclosure_frame(&ctx, &mut math, source, time, events)
                    .platform_output
                    .commands,
            );
        }
        let copies: Vec<_> = commands
            .iter()
            .filter_map(|c| match c {
                egui::OutputCommand::CopyText(text) => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(copies, ["bold and em and snake_case"]);
    }
}
