//! History-list rows. The caller owns filtering and fork title markers,
//! arrow-key selection, Enter and Ctrl+Backspace routing, and actual mutations.

use eframe::egui;

use super::theme;

const PADDING: f32 = 8.0;
const DELETE_SIZE: f32 = 26.0;
const PLAIN_HEIGHT: f32 = 42.0;
const PREVIEW_HEIGHT: f32 = 62.0;
const SELECTED_TINT: f32 = 0.12;

pub struct HistoryRowResponse {
    /// Full-row geometry, including the preview and the nested delete target.
    /// Use this for hovering and scrolling; use `open`/`delete` for actions.
    pub response: egui::Response,
    pub open: bool,
    pub delete: bool,
}

/// Draw a left-aligned, one- or two-line history row with a separate delete
/// button. `id` must be stable and unique per conversation. Pass the complete
/// display title (including any fork marker) and the existing search
/// highlighter's job; explicit highlight colors/backgrounds are preserved.
///
/// The whole row opens, except its 26×26 delete square. Both controls support
/// normal egui keyboard focus/activation; list-level shortcuts stay with the
/// caller. The action booleans are mutually exclusive, even on a double click.
pub fn row(
    ui: &mut egui::Ui,
    id: egui::Id,
    title: &str,
    subtitle_job: Option<egui::text::LayoutJob>,
    time: &str,
    selected: bool,
    current: bool,
) -> HistoryRowResponse {
    let subtitle_job = subtitle_job.filter(|job| !job.text.is_empty());
    let height = if subtitle_job.is_some() {
        PREVIEW_HEIGHT
    } else {
        PLAIN_HEIGHT
    };
    let (_, rect) = ui.allocate_space(egui::vec2(ui.available_width().max(1.0), height));
    let response = ui.interact(rect, id, egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, ui.is_enabled(), selected, title)
    });

    // Register the child last so egui gives it priority over the full-row hit
    // region. Absolute rectangles do not advance the list cursor a second time.
    let line_center = rect.top() + PLAIN_HEIGHT / 2.0;
    let delete_rect = egui::Rect::from_center_size(
        egui::pos2(rect.right() - PADDING - DELETE_SIZE / 2.0, line_center),
        egui::vec2(DELETE_SIZE, DELETE_SIZE),
    );
    let delete_response = ui.interact(delete_rect, id.with("delete"), egui::Sense::click());
    delete_response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), "刪除聊天")
    });

    let delete = delete_response.clicked();
    let open = response.clicked()
        && !delete
        && response
            .interact_pointer_pos()
            .is_none_or(|pos| !delete_rect.contains(pos));

    if ui.is_rect_visible(rect) {
        let palette = theme::current_palette(ui.ctx());
        let hovered = response.contains_pointer() || delete_response.contains_pointer();
        let base = ui.visuals().window_fill;
        // Opaque composition makes contrast independent of an underlying
        // widget's selection fill. Never use selection.bg_fill as a solid fill.
        let background = if selected || current {
            let tint = if selected { SELECTED_TINT } else { 0.08 };
            mix(
                base,
                palette.accent,
                tint + if hovered { 0.03 } else { 0.0 },
            )
        } else if hovered {
            mix(base, palette.assistant_text, 0.06)
        } else {
            base
        };
        let primary = readable(
            ui.visuals().text_color(),
            background,
            palette.assistant_text,
        );
        let weak = readable(ui.visuals().weak_text_color(), background, primary);
        let painter = ui.painter().with_clip_rect(ui.clip_rect().intersect(rect));
        painter.rect_filled(rect, 8, background);
        if selected || response.has_focus() {
            painter.rect_stroke(
                rect.shrink(0.5),
                8,
                egui::Stroke::new(1.0_f32, mix(background, palette.accent, 0.55)),
                egui::StrokeKind::Inside,
            );
        }

        let time_galley = ui.fonts(|fonts| {
            fonts.layout_no_wrap(time.to_owned(), egui::FontId::proportional(10.0), weak)
        });
        let time_right = delete_rect.left() - 6.0;
        let time_left = (time_right - time_galley.size().x).max(rect.left() + PADDING);
        let text_left = rect.left() + PADDING;
        let marker_width = if current { 16.0 } else { 0.0 };
        let title_width = (time_left - 8.0 - marker_width - text_left).max(0.0);
        if title_width > 0.0 {
            let mut title_job = egui::text::LayoutJob::simple_singleline(
                title.to_owned(),
                egui::FontId::proportional(12.5),
                primary,
            );
            truncate_left(&mut title_job, title_width);
            let title_galley = ui.fonts(|fonts| fonts.layout_job(title_job));
            let title_pos = egui::pos2(text_left, line_center - title_galley.size().y / 2.0);
            let title_clip = egui::Rect::from_min_max(
                egui::pos2(text_left, rect.top()),
                egui::pos2(text_left + title_width, rect.top() + PLAIN_HEIGHT),
            );
            let title_painter = painter.with_clip_rect(painter.clip_rect().intersect(title_clip));
            title_painter.galley(title_pos, title_galley.clone(), primary);
            if current {
                // No semibold font family is configured in the app. A small
                // physical-pixel overstrike adds weight without changing the
                // metrics, text color or Latin/CJK fallback font selection.
                title_painter.galley(
                    title_pos + egui::vec2(0.35 / ui.ctx().pixels_per_point(), 0.0),
                    title_galley,
                    primary,
                );
            }
        }
        if current && title_width > 0.0 {
            let center = egui::pos2(time_left - 16.0, line_center);
            let stroke = egui::Stroke::new(1.5_f32, primary);
            painter.line_segment(
                [
                    center + egui::vec2(-4.0, 0.0),
                    center + egui::vec2(-1.0, 3.0),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    center + egui::vec2(-1.0, 3.0),
                    center + egui::vec2(4.0, -3.0),
                ],
                stroke,
            );
        }
        if time_right > time_left {
            let time_clip = egui::Rect::from_min_max(
                egui::pos2(time_left, rect.top()),
                egui::pos2(time_right, rect.top() + PLAIN_HEIGHT),
            );
            painter
                .with_clip_rect(painter.clip_rect().intersect(time_clip))
                .galley(
                    egui::pos2(time_left, line_center - time_galley.size().y / 2.0),
                    time_galley,
                    weak,
                );
        }

        if let Some(mut job) = subtitle_job {
            // Normalize only the highlighter's ordinary foregrounds. A search
            // match or any explicit custom color must survive painter.galley
            // (not galley_with_override_text_color).
            for section in &mut job.sections {
                let format = &mut section.format;
                if format.background == egui::Color32::TRANSPARENT {
                    if format.color == ui.visuals().weak_text_color()
                        || format.color == egui::Color32::PLACEHOLDER
                    {
                        format.color = weak;
                    } else if format.color == ui.visuals().text_color() {
                        format.color = primary;
                    }
                }
            }
            truncate_left(&mut job, (rect.width() - 2.0 * PADDING).max(0.0));
            let galley = ui.fonts(|fonts| fonts.layout_job(job));
            painter.galley(
                egui::pos2(text_left, rect.top() + 44.0 - galley.size().y / 2.0),
                galley,
                weak,
            );
        }

        let delete_fill = if delete_response.hovered()
            || delete_response.has_focus()
            || delete_response.is_pointer_button_down_on()
        {
            mix(background, primary, 0.08)
        } else {
            background
        };
        painter.rect_filled(delete_rect, 5, delete_fill);
        if delete_response.has_focus() {
            painter.rect_stroke(
                delete_rect.shrink(0.5),
                5,
                egui::Stroke::new(1.0_f32, palette.accent),
                egui::StrokeKind::Inside,
            );
        }
        let stroke = egui::Stroke::new(1.5_f32, readable(weak, delete_fill, primary));
        let center = delete_rect.center();
        for direction in [-1.0, 1.0] {
            painter.line_segment(
                [
                    center + egui::vec2(-4.0, -4.0 * direction),
                    center + egui::vec2(4.0, 4.0 * direction),
                ],
                stroke,
            );
        }
    }
    delete_response.on_hover_text("刪除聊天 · Ctrl+Backspace（或聚焦後按 Enter / Space）");
    HistoryRowResponse {
        response,
        open,
        delete,
    }
}

fn truncate_left(job: &mut egui::text::LayoutJob, width: f32) {
    job.halign = egui::Align::Min;
    job.justify = false;
    job.break_on_newline = false;
    job.wrap.max_width = width;
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
}

/// Deliberate sRGB compositing, matching the color values that are painted.
fn mix(base: egui::Color32, tint: egui::Color32, amount: f32) -> egui::Color32 {
    let channel =
        |a: u8, b: u8| (f32::from(a) * (1.0 - amount) + f32::from(b) * amount).round() as u8;
    egui::Color32::from_rgb(
        channel(base.r(), tint.r()),
        channel(base.g(), tint.g()),
        channel(base.b(), tint.b()),
    )
}

fn luminance(color: egui::Color32) -> f64 {
    let linear = |value: u8| {
        let value = f64::from(value) / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(color.r()) + 0.7152 * linear(color.g()) + 0.0722 * linear(color.b())
}

fn contrast(a: egui::Color32, b: egui::Color32) -> f64 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// Keep semantic primary/secondary colors when possible; make only the small
/// adjustment needed on tinted surfaces, including a custom very bright accent.
fn readable(
    foreground: egui::Color32,
    background: egui::Color32,
    preferred: egui::Color32,
) -> egui::Color32 {
    if foreground.is_opaque() && contrast(foreground, background) >= 4.5 {
        return foreground;
    }
    let target = if contrast(preferred, background) >= 4.5 {
        preferred
    } else if contrast(egui::Color32::BLACK, background)
        > contrast(egui::Color32::WHITE, background)
    {
        egui::Color32::BLACK
    } else {
        egui::Color32::WHITE
    };
    for step in 1..=20 {
        let candidate = mix(foreground, target, step as f32 / 20.0);
        if contrast(candidate, background) >= 4.5 {
            return candidate;
        }
    }
    target
}

#[cfg(test)]
mod tests {
    use super::*;

    const TITLE: &str = "Left aligned history";
    const PREVIEW: &str = "Preview with matching words";

    fn draw(
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        width: f32,
        preview: bool,
        selected: bool,
        current: bool,
    ) -> (egui::FullOutput, HistoryRowResponse) {
        let mut result = None;
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(640.0, 480.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.set_width(width);
                    let job = preview.then(|| {
                        egui::text::LayoutJob::simple_singleline(
                            PREVIEW.to_owned(),
                            egui::FontId::proportional(11.0),
                            ui.visuals().weak_text_color(),
                        )
                    });
                    result = Some(row(
                        ui,
                        egui::Id::new("history-test"),
                        TITLE,
                        job,
                        "12:34",
                        selected,
                        current,
                    ));
                });
            },
        );
        (output, result.unwrap())
    }

    fn texts(output: &egui::FullOutput) -> Vec<&egui::epaint::TextShape> {
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text),
                _ => None,
            })
            .collect()
    }

    fn delete_rect(row: egui::Rect) -> egui::Rect {
        egui::Rect::from_center_size(
            egui::pos2(row.right() - PADDING - 13.0, row.top() + 21.0),
            egui::vec2(26.0, 26.0),
        )
    }

    fn click_event(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn production_row_has_left_aligned_galleys_padding_and_square_delete_target() {
        for appearance in ["light", "dark"] {
            for width in [180.0, 300.0, 500.0] {
                for preview in [false, true] {
                    for current in [false, true] {
                        let ctx = egui::Context::default();
                        theme::apply(&ctx, appearance, "#30D158");
                        let (output, result) = draw(&ctx, vec![], width, preview, true, current);
                        let rect = result.response.rect;
                        assert_eq!(rect.height(), if preview { 62.0 } else { 42.0 });
                        let text_shapes = texts(&output);
                        let title = text_shapes
                            .iter()
                            .find(|text| text.galley.job.text == TITLE)
                            .unwrap();
                        assert_eq!(title.galley.job.halign, egui::Align::Min);
                        assert_eq!(title.galley.rows.len(), 1);
                        assert!((title.pos.x - rect.left() - 8.0).abs() < 0.01);
                        assert!(title.pos.y - rect.top() >= 8.0);
                        assert!(title.galley.rows[0].rect().left().abs() < 0.01);
                        assert!(
                            title.galley.rows[0].rect().width()
                                <= title.galley.job.wrap.max_width + 1.0
                        );
                        let time = text_shapes
                            .iter()
                            .find(|text| text.galley.job.text == "12:34")
                            .unwrap();
                        assert!(title.pos.x + title.galley.size().x <= time.pos.x - 6.0);
                        if preview {
                            let subtitle = text_shapes
                                .iter()
                                .find(|text| text.galley.job.text == PREVIEW)
                                .unwrap();
                            assert_eq!(subtitle.galley.job.halign, egui::Align::Min);
                            assert_eq!(subtitle.pos.x, title.pos.x);
                            assert_eq!(subtitle.galley.rows.len(), 1);
                            assert!(subtitle.pos.y > title.pos.y + title.galley.size().y);
                            assert!(
                                subtitle.pos.y + subtitle.galley.size().y <= rect.bottom() - 8.0
                            );
                        }
                        let delete = output
                            .shapes
                            .iter()
                            .find_map(|shape| match &shape.shape {
                                egui::Shape::Rect(shape) if shape.rect == delete_rect(rect) => {
                                    Some(shape.rect)
                                }
                                _ => None,
                            })
                            .unwrap();
                        assert_eq!(delete.size(), egui::vec2(26.0, 26.0));
                        assert_eq!(delete.top() - rect.top(), 8.0);
                        assert_eq!(rect.right() - delete.right(), 8.0);
                        let strokes = output
                            .shapes
                            .iter()
                            .filter(|shape| matches!(&shape.shape, egui::Shape::LineSegment { points, .. } if points.iter().all(|point| delete.contains(*point))))
                            .count();
                        assert_eq!(strokes, 2, "delete is a drawn X, not a font glyph");
                        let check_strokes = output.shapes.iter().filter(|shape| {
                            matches!(&shape.shape, egui::Shape::LineSegment { points, .. } if points.iter().all(|point| !delete.contains(*point)))
                        }).count();
                        assert_eq!(
                            check_strokes,
                            if current { 2 } else { 0 },
                            "current chat uses a vector check, not a missing font glyph"
                        );
                        let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
                        assert!(primitives
                            .iter()
                            .any(|primitive| matches!(&primitive.primitive,
                            egui::epaint::Primitive::Mesh(mesh) if !mesh.vertices.is_empty())));
                    }
                }
            }
        }
    }

    #[test]
    fn all_production_nonhighlight_text_exceeds_wcag_aa_on_every_row_state() {
        for appearance in ["light", "dark"] {
            for accent in theme::ACCENT_PRESETS
                .into_iter()
                .chain(["#00FF00", "#FFFFFF", "#000000"])
            {
                for selected in [false, true] {
                    for current in [false, true] {
                        for hovered in [false, true] {
                            let ctx = egui::Context::default();
                            let palette = theme::apply(&ctx, appearance, accent);
                            draw(&ctx, vec![], 300.0, true, selected, current);
                            let (_, first) = draw(&ctx, vec![], 300.0, true, selected, current);
                            let events = if hovered {
                                vec![egui::Event::PointerMoved(first.response.rect.center())]
                            } else {
                                vec![]
                            };
                            let (output, result) =
                                draw(&ctx, events, 300.0, true, selected, current);
                            let fill = output
                                .shapes
                                .iter()
                                .find_map(|shape| match &shape.shape {
                                    egui::Shape::Rect(shape)
                                        if shape.rect == result.response.rect =>
                                    {
                                        Some(shape.fill)
                                    }
                                    _ => None,
                                })
                                .unwrap();
                            for text in texts(&output) {
                                for section in &text.galley.job.sections {
                                    let ratio = contrast(section.format.color, fill);
                                    assert!(
                                        ratio >= 4.5,
                                        "{appearance}/{accent}: {} = {ratio}",
                                        text.galley.job.text
                                    );
                                }
                            }
                            if selected {
                                let max_tint =
                                    mix(ctx.style().visuals.window_fill, palette.accent, 0.15);
                                for (actual, base, bound) in [
                                    (fill.r(), ctx.style().visuals.window_fill.r(), max_tint.r()),
                                    (fill.g(), ctx.style().visuals.window_fill.g(), max_tint.g()),
                                    (fill.b(), ctx.style().visuals.window_fill.b(), max_tint.b()),
                                ] {
                                    assert!(actual.abs_diff(base) <= bound.abs_diff(base));
                                }
                                assert_ne!(fill, palette.accent);
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn production_preview_preserves_custom_foregrounds_and_search_highlights() {
        let ctx = egui::Context::default();
        theme::apply(&ctx, "dark", "#30D158");
        let custom = egui::Color32::from_rgb(241, 220, 90);
        let highlight = egui::Color32::from_rgba_unmultiplied(48, 209, 88, 46);
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let mut job = egui::text::LayoutJob::default();
                for (text, color, background) in [
                    ("custom ", custom, egui::Color32::TRANSPARENT),
                    ("match", egui::Color32::WHITE, highlight),
                    (
                        " ordinary",
                        ui.visuals().weak_text_color(),
                        egui::Color32::TRANSPARENT,
                    ),
                ] {
                    job.append(
                        text,
                        0.0,
                        egui::TextFormat {
                            font_id: egui::FontId::proportional(11.0),
                            color,
                            background,
                            ..Default::default()
                        },
                    );
                }
                job.halign = egui::Align::Center;
                row(
                    ui,
                    egui::Id::new("highlight"),
                    TITLE,
                    Some(job),
                    "12:34",
                    true,
                    false,
                );
            });
        });
        let text_shapes = texts(&output);
        let preview = text_shapes
            .iter()
            .find(|text| text.galley.job.text == "custom match ordinary")
            .unwrap();
        let sections = &preview.galley.job.sections;
        assert_eq!(sections[0].format.color, custom);
        assert_eq!(sections[1].format.color, egui::Color32::WHITE);
        assert_eq!(sections[1].format.background, highlight);
        assert_eq!(preview.galley.job.halign, egui::Align::Min);
        assert_eq!(preview.override_text_color, None);
    }

    #[test]
    fn pointer_clicks_open_entire_row_except_the_independent_delete_square() {
        for preview in [false, true] {
            for target in [
                "title",
                "preview",
                "padding",
                "time",
                "delete",
                "delete-padding",
                "beside-delete",
            ] {
                let ctx = egui::Context::default();
                theme::apply(&ctx, "light", "#30D158");
                draw(&ctx, vec![], 300.0, preview, true, false);
                let (_, initial) = draw(&ctx, vec![], 300.0, preview, true, false);
                let rect = initial.response.rect;
                let delete = delete_rect(rect);
                let pos = match target {
                    "title" => rect.min + egui::vec2(24.0, 21.0),
                    "preview" => rect.min + egui::vec2(30.0, if preview { 44.0 } else { 32.0 }),
                    "padding" => rect.min + egui::vec2(3.0, 3.0),
                    "time" => egui::pos2(delete.left() - 15.0, delete.center().y),
                    "delete" => delete.center(),
                    "delete-padding" => delete.min + egui::vec2(1.0, 1.0),
                    "beside-delete" => egui::pos2(delete.right() + 3.0, delete.center().y),
                    _ => unreachable!(),
                };
                draw(
                    &ctx,
                    vec![egui::Event::PointerMoved(pos), click_event(pos, true)],
                    300.0,
                    preview,
                    true,
                    false,
                );
                let (_, result) = draw(
                    &ctx,
                    vec![click_event(pos, false)],
                    300.0,
                    preview,
                    true,
                    false,
                );
                let deleting = target.starts_with("delete");
                assert_eq!(
                    (result.open, result.delete),
                    (!deleting, deleting),
                    "{target}, preview={preview}"
                );
                assert!(!(result.open && result.delete));
                // A second (double) click still emits exactly one action.
                draw(
                    &ctx,
                    vec![click_event(pos, true)],
                    300.0,
                    preview,
                    true,
                    false,
                );
                let (_, again) = draw(
                    &ctx,
                    vec![click_event(pos, false)],
                    300.0,
                    preview,
                    true,
                    false,
                );
                assert_eq!((again.open, again.delete), (!deleting, deleting));
            }
        }
    }

    #[test]
    fn focused_delete_keyboard_activation_never_opens_the_row() {
        let ctx = egui::Context::default();
        theme::apply(&ctx, "dark", "#30D158");
        draw(&ctx, vec![], 300.0, true, false, false);
        draw(&ctx, vec![], 300.0, true, false, false);
        let id = egui::Id::new("history-test").with("delete");
        ctx.memory_mut(|memory| memory.request_focus(id));
        let (_, result) = draw(
            &ctx,
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            300.0,
            true,
            false,
            false,
        );
        assert!(result.delete);
        assert!(!result.open);
    }
}
