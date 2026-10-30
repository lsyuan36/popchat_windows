//! Viewport-bounded contents for an existing provider `ui.menu_button` submenu.
//! The caller applies the returned choice and closes the menu. Neither model nor
//! effort identifiers are normalized: provider-native strings survive verbatim.

use eframe::egui;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct ModelRow {
    pub model: String,
    pub selected: bool,
    /// (Provider-native effort identifier, currently selected.)
    pub efforts: Vec<(String, bool)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelChoice {
    pub model: String,
    /// `None` means a model click; the caller retains its usual default effort.
    pub effort: Option<String>,
}

/// Also suitable for the provider-root scroll area. Reserve room for the header,
/// popup frame and viewport edges instead of assuming a tall native window.
pub fn menu_height(ctx: &egui::Context) -> f32 {
    (ctx.screen_rect().height() - 140.0).clamp(64.0, 360.0)
}

fn scroll_salt(provider_id: Uuid) -> (&'static str, Uuid) {
    ("popchat-model-scroll", provider_id)
}

fn model_id(namespace: egui::Id, provider_id: Uuid, model: &str) -> egui::Id {
    namespace.with(("popchat-model", provider_id, model))
}

fn effort_id(namespace: egui::Id, provider_id: Uuid, model: &str, effort: &str) -> egui::Id {
    namespace.with(("popchat-effort", provider_id, model, effort))
}

/// A click-only control: dragging a model must not capture wheel/scroll gestures.
/// Explicit sizing and galley truncation keep even arbitrary native identifiers
/// inside the popup. Full text remains available on hover.
fn choice_button(
    ui: &mut egui::Ui,
    id: egui::Id,
    text: &str,
    selected: bool,
    size: egui::Vec2,
) -> egui::Response {
    let (_, rect) = ui.allocate_space(size);
    let response = ui.interact(rect, id, egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            text,
        )
    });
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        // Neutral selection surfaces deliberately do not inherit a potentially
        // bright accent paired with white text from the global selection style.
        let selected_fill = if ui.visuals().dark_mode {
            egui::Color32::from_rgb(0x36, 0x3A, 0x44)
        } else {
            super::theme::LIGHT_HOVER
        };
        let fill = if selected {
            selected_fill
        } else if response.hovered() || response.has_focus() {
            visuals.weak_bg_fill
        } else {
            egui::Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, 5, fill);
        if response.has_focus() {
            ui.painter()
                .rect_stroke(rect, 5, visuals.fg_stroke, egui::StrokeKind::Inside);
        }
        let text_color = ui.visuals().text_color();
        let prefix_width = 16.0;
        if selected {
            let center = egui::pos2(rect.left() + 11.0, rect.center().y);
            let stroke = egui::Stroke::new(1.5_f32, text_color);
            ui.painter().line_segment(
                [
                    center + egui::vec2(-4.0, 0.0),
                    center + egui::vec2(-1.0, 3.0),
                ],
                stroke,
            );
            ui.painter().line_segment(
                [
                    center + egui::vec2(-1.0, 3.0),
                    center + egui::vec2(4.0, -3.0),
                ],
                stroke,
            );
        }
        let mut job = egui::text::LayoutJob::simple_singleline(
            text.to_owned(),
            egui::FontId::proportional(13.0),
            text_color,
        );
        job.wrap.max_width = (rect.width() - prefix_width - 14.0).max(1.0);
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        let galley = ui.fonts(|fonts| fonts.layout_job(job));
        let pos = egui::pos2(
            rect.left() + prefix_width + 7.0,
            rect.center().y - galley.size().y / 2.0,
        );
        ui.painter()
            .with_clip_rect(rect.intersect(ui.clip_rect()))
            .galley(pos, galley, text_color);
    }
    response.on_hover_text(text)
}

/// Draw inside the existing provider submenu, not inside another popup/menu.
/// Scroll state is provider-specific and retained by egui between frames/openings.
/// Efforts are secondary 26-point chips; models are 28-point, 13-point type rows.
pub fn model_list(ui: &mut egui::Ui, provider_id: Uuid, rows: &[ModelRow]) -> Option<ModelChoice> {
    let namespace = ui.id();
    let width = (ui.ctx().screen_rect().width() - 32.0).clamp(32.0, 320.0);
    let height = menu_height(ui.ctx());
    ui.set_width(width);
    egui::ScrollArea::vertical()
        .id_salt(scroll_salt(provider_id))
        .max_height(height)
        .auto_shrink([false, true])
        .scroll_source(
            egui::scroll_area::ScrollSource::SCROLL_BAR
                | egui::scroll_area::ScrollSource::MOUSE_WHEEL,
        )
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(4.0, 3.0);
            let mut choice = None;
            for row in rows {
                let width = ui.available_width().min(width);
                let id = model_id(namespace, provider_id, &row.model);
                if choice_button(ui, id, &row.model, row.selected, egui::vec2(width, 28.0))
                    .clicked()
                {
                    choice = Some(ModelChoice {
                        model: row.model.clone(),
                        effort: None,
                    });
                }
                if !row.efforts.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(4.0, 3.0);
                        for (effort, selected) in &row.efforts {
                            let id = effort_id(namespace, provider_id, &row.model, effort);
                            let text_width = ui.fonts(|fonts| {
                                fonts
                                    .layout_no_wrap(
                                        effort.clone(),
                                        egui::FontId::proportional(13.0),
                                        ui.visuals().text_color(),
                                    )
                                    .size()
                                    .x
                            });
                            let chip_width = (text_width + 30.0).min(width);
                            if choice_button(
                                ui,
                                id,
                                effort,
                                row.selected && *selected,
                                egui::vec2(chip_width, 26.0),
                            )
                            .clicked()
                            {
                                choice = Some(ModelChoice {
                                    model: row.model.clone(),
                                    effort: Some(effort.clone()),
                                });
                            }
                        }
                    });
                }
            }
            choice
        })
        .inner
}

#[cfg(test)]
mod tests {
    use super::*;

    fn models(count: usize) -> Vec<ModelRow> {
        (0..count)
            .map(|i| ModelRow {
                model: format!("provider/native-model-{i:03}"),
                selected: i == 0,
                efforts: Vec::new(),
            })
            .collect()
    }

    struct Harness {
        ctx: egui::Context,
        namespace: egui::Id,
        bounds: egui::Rect,
        shapes: Vec<egui::epaint::ClippedShape>,
        time: f64,
    }

    impl Harness {
        fn new() -> Self {
            let ctx = egui::Context::default();
            ctx.style_mut(|style| style.animation_time = 0.0);
            Self {
                ctx,
                namespace: egui::Id::NULL,
                bounds: egui::Rect::NOTHING,
                shapes: Vec::new(),
                time: 0.0,
            }
        }

        fn frame(
            &mut self,
            provider: Uuid,
            rows: &[ModelRow],
            events: Vec<egui::Event>,
        ) -> Option<ModelChoice> {
            self.time += 1.0 / 60.0;
            let mut choice = None;
            let output = self.ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(460.0, 344.0),
                    )),
                    time: Some(self.time),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    let output = egui::Area::new(egui::Id::new("production-list-test"))
                        .fixed_pos(egui::pos2(20.0, 50.0))
                        .show(ctx, |ui| {
                            self.namespace = ui.id();
                            choice = model_list(ui, provider, rows);
                        });
                    self.bounds = output.response.rect;
                },
            );
            self.shapes = output.shapes;
            choice
        }

        fn scroll(&self, provider: Uuid) -> egui::scroll_area::State {
            egui::scroll_area::State::load(
                &self.ctx,
                self.namespace.with(egui::Id::new(scroll_salt(provider))),
            )
            .unwrap()
        }

        fn row(&self, provider: Uuid, model: &str) -> egui::Response {
            self.response(model_id(self.namespace, provider, model))
        }

        fn response(&self, id: egui::Id) -> egui::Response {
            let mut response = self.ctx.read_response(id).unwrap();
            if let Some(transform) = self.ctx.layer_transform_to_global(response.layer_id) {
                response.rect = transform * response.rect;
                response.interact_rect = transform * response.interact_rect;
            }
            response
        }

        fn wheel(&mut self, provider: Uuid, rows: &[ModelRow], pos: egui::Pos2, delta: f32) {
            self.frame(
                provider,
                rows,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0.0, delta),
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }

        fn click(
            &mut self,
            provider: Uuid,
            rows: &[ModelRow],
            pos: egui::Pos2,
        ) -> Option<ModelChoice> {
            self.frame(provider, rows, vec![egui::Event::PointerMoved(pos)]);
            self.frame(
                provider,
                rows,
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            self.frame(
                provider,
                rows,
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            )
        }
    }

    #[test]
    fn production_rows_accept_wheel_and_last_model_is_reachable_in_compact_viewport() {
        let provider = Uuid::from_u128(1);
        let rows = models(80);
        let mut h = Harness::new();
        h.frame(provider, &rows, vec![]);
        h.frame(provider, &rows, vec![]);
        h.frame(provider, &rows, vec![]);
        assert_eq!(menu_height(&h.ctx), 204.0);
        assert!(h.bounds.height() <= 204.1, "{:?}", h.bounds);
        assert!(h.bounds.bottom() <= 344.0);
        let first = h.row(provider, &rows[0].model);
        assert_eq!(first.rect.height(), 28.0);
        let pointer = first.rect.center();
        assert!(first.interact_rect.contains(pointer));
        let initial = h.scroll(provider).offset.y;
        h.wheel(provider, &rows, pointer, -100.0);
        assert!(
            h.scroll(provider).offset.y > initial,
            "wheel over a real model row must scroll"
        );
        for _ in 0..50 {
            h.wheel(provider, &rows, pointer, -240.0);
        }
        for _ in 0..40 {
            h.frame(provider, &rows, vec![]);
        }
        let last = h.row(provider, &rows.last().unwrap().model);
        assert!(
            h.bounds.contains(last.rect.center()),
            "last row {:?}, bounds {:?}",
            last.rect,
            h.bounds
        );
        assert!(last.interact_rect.contains(last.rect.center()));
        assert_eq!(
            h.click(provider, &rows, last.rect.center()),
            Some(ModelChoice {
                model: rows.last().unwrap().model.clone(),
                effort: None
            })
        );
    }

    #[test]
    fn production_long_identifier_is_bounded_and_click_returns_unmodified_model() {
        let provider = Uuid::from_u128(2);
        let mut rows = models(1);
        rows[0].model = "native/Extremely_Long_Model_Identifier-".repeat(30);
        let mut h = Harness::new();
        h.frame(provider, &rows, vec![]);
        h.frame(provider, &rows, vec![]);
        h.frame(provider, &rows, vec![]);
        let row = h.row(provider, &rows[0].model);
        assert!(row.rect.width() <= 320.0);
        assert!(h.bounds.width() <= 320.1);
        let text = h
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text == rows[0].model => Some(text),
                _ => None,
            })
            .expect("production model row paints its text");
        assert!(
            text.galley.elided,
            "long model names really render with ellipsis"
        );
        assert!(text.pos.x + text.galley.size().x <= row.rect.right());
        assert_eq!(text.galley.job.sections[0].format.font_id.size, 13.0);
        assert_eq!(
            h.click(provider, &rows, row.rect.center()),
            Some(ModelChoice {
                model: rows[0].model.clone(),
                effort: None
            })
        );
    }

    #[test]
    fn production_effort_chip_returns_exact_native_effort_and_model() {
        let provider = Uuid::from_u128(3);
        let mut rows = models(1);
        rows[0].efforts = vec![
            ("native:Adaptive MAX / 32768".into(), true),
            ("xhigh".into(), false),
        ];
        let mut h = Harness::new();
        h.frame(provider, &rows, vec![]);
        h.frame(provider, &rows, vec![]);
        h.frame(provider, &rows, vec![]);
        for (effort, _) in &rows[0].efforts {
            let id = effort_id(h.namespace, provider, &rows[0].model, effort);
            let rect = h.response(id).rect;
            assert_eq!(rect.height(), 26.0);
            assert_eq!(
                h.click(provider, &rows, rect.center()),
                Some(ModelChoice {
                    model: rows[0].model.clone(),
                    effort: Some(effort.clone())
                })
            );
        }
    }

    #[test]
    fn provider_scroll_namespaces_are_separate_and_reopening_preserves_offset() {
        let first = Uuid::from_u128(4);
        let second = Uuid::from_u128(5);
        let rows = models(40);
        let mut h = Harness::new();
        h.frame(first, &rows, vec![]);
        h.frame(first, &rows, vec![]);
        h.frame(first, &rows, vec![]);
        let pointer = h.row(first, &rows[0].model).rect.center();
        h.wheel(first, &rows, pointer, -100.0);
        for _ in 0..60 {
            h.frame(first, &rows, vec![]);
        }
        let offset = h.scroll(first).offset.y;
        assert!(offset > 0.0);
        h.frame(second, &rows, vec![]);
        assert_eq!(h.scroll(second).offset.y, 0.0);
        assert_ne!(
            model_id(h.namespace, first, &rows[0].model),
            model_id(h.namespace, second, &rows[0].model)
        );
        h.frame(first, &rows, vec![]);
        assert!((h.scroll(first).offset.y - offset).abs() < 0.01);
        h.frame(first, &rows, vec![]);
        assert!(
            (h.scroll(first).offset.y - offset).abs() < 0.01,
            "selected first row must not auto-scroll every frame"
        );
    }

    #[test]
    fn real_nested_provider_popup_fits_and_wheel_over_model_scrolls_at_272_and_344() {
        struct Snapshot {
            provider_button: egui::Rect,
            contents: Option<egui::Rect>,
            namespace: Option<egui::Id>,
        }
        for height in [272.0, 344.0] {
            let ctx = egui::Context::default();
            ctx.style_mut(|style| style.animation_time = 0.0);
            let provider = Uuid::from_u128(6);
            let rows = models(60);
            let mut time = 0.0;
            let mut frame = |events| {
                time += 1.0 / 60.0;
                let mut snapshot = Snapshot {
                    provider_button: egui::Rect::NOTHING,
                    contents: None,
                    namespace: None,
                };
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(460.0, height),
                        )),
                        time: Some(time),
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            let anchor = ui.button("Models");
                            egui::Popup::menu(&anchor)
                                .open(true)
                                .width(156.4)
                                .show(|ui| {
                                    ui.set_max_width(156.4);
                                    egui::ScrollArea::vertical()
                                        .id_salt("provider_menu_scroll")
                                        .max_height((height - 110.0).clamp(64.0, 300.0))
                                        .show(ui, |ui| {
                                            let response = ui.menu_button("Provider", |ui| {
                                                snapshot.namespace = Some(ui.id());
                                                let _ = model_list(ui, provider, &rows);
                                                snapshot.contents = Some(ui.min_rect());
                                            });
                                            snapshot.provider_button = response.response.rect;
                                        });
                                });
                        });
                    },
                );
                snapshot
            };
            frame(vec![]);
            frame(vec![]);
            let provider_pos = frame(vec![]).provider_button.center();
            for _ in 0..4 {
                frame(vec![egui::Event::PointerMoved(provider_pos)]);
            }
            let snapshot = frame(vec![]);
            let contents = snapshot
                .contents
                .expect("hover opens the actual nested provider menu");
            assert!(
                contents.height() <= height - 140.0 + 0.1,
                "{height}: {contents:?}"
            );
            assert!(
                contents.top() >= 0.0 && contents.bottom() <= height,
                "{height}: {contents:?}"
            );
            assert!(
                contents.left() >= 0.0 && contents.right() <= 460.0,
                "{height}: {contents:?}"
            );
            let namespace = snapshot.namespace.unwrap();
            let response = ctx
                .read_response(model_id(namespace, provider, &rows[0].model))
                .unwrap();
            let pointer = response.rect.center();
            let scroll_id = namespace.with(egui::Id::new(scroll_salt(provider)));
            let before = egui::scroll_area::State::load(&ctx, scroll_id)
                .unwrap()
                .offset
                .y;
            frame(vec![
                egui::Event::PointerMoved(pointer),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, -100.0),
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
            let after = egui::scroll_area::State::load(&ctx, scroll_id)
                .unwrap()
                .offset
                .y;
            assert!(
                after > before,
                "wheel over a real nested-popup row must scroll at {height}"
            );
        }
    }
}
