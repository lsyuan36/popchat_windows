//! Chat-only typography. Apply to a dedicated child Ui, never the Context style.

use eframe::egui::{FontId, TextStyle, Ui};

pub const DEFAULT_SIZE: u16 = 15;
pub const MIN_SIZE: u16 = 12;
pub const MAX_SIZE: u16 = 24;
/// Tool/search activity is metadata, not transcript prose. Keep it small even
/// when the user chooses a larger chat size; never scale the actual answer down.
pub const ACTIVITY_SIZE: f32 = 12.0;

pub fn activity_label(ui: &mut Ui, text: &str) -> eframe::egui::Response {
    ui.add(
        eframe::egui::Label::new(
            eframe::egui::RichText::new(format!("… {text}"))
                .size(ACTIVITY_SIZE)
                .color(ui.visuals().weak_text_color()),
        )
        .wrap()
        .selectable(true),
    )
}

pub fn normalize(size: u16) -> u16 {
    size.clamp(MIN_SIZE, MAX_SIZE)
}

pub fn scale(size: u16) -> f32 {
    f32::from(normalize(size)) / f32::from(DEFAULT_SIZE)
}

/// Approximate inline composer row budget; TextEdit still measures its own font.
pub fn line_height(size: u16) -> f32 {
    19.0 * scale(size)
}

fn scale_style() -> TextStyle {
    TextStyle::Name("popchat-chat-scale".into())
}

/// Inherited by child Uis, without changing chrome, font families, or global UI
/// zoom. Absolute baseline sizes make repeated/nested applications idempotent.
pub fn apply(ui: &mut Ui, size: u16) {
    let ratio = scale(size);
    let style = ui.style_mut();
    style
        .text_styles
        .entry(TextStyle::Body)
        .or_insert_with(|| FontId::proportional(f32::from(DEFAULT_SIZE)))
        .size = f32::from(normalize(size));
    style
        .text_styles
        .entry(TextStyle::Monospace)
        .or_insert_with(|| FontId::monospace(13.0))
        .size = 13.0 * ratio;
    style
        .text_styles
        .insert(scale_style(), FontId::proportional(ratio));
}

/// Scale an explicit renderer font from its original default-size baseline.
/// An ordinary Ui has no marker and therefore retains its original metrics.
pub fn scaled(ui: &Ui, base: f32) -> f32 {
    base * ui
        .style()
        .text_styles
        .get(&scale_style())
        .map_or(1.0, |font| font.size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui;

    #[test]
    fn tool_and_search_activity_stays_small_without_resizing_normal_chat() {
        for appearance in ["light", "dark"] {
            for size in [12, 15, 24] {
                let ctx = egui::Context::default();
                super::super::theme::apply(
                    &ctx,
                    appearance,
                    super::super::theme::DEFAULT_ACCENT_HEX,
                );
                let samples = [
                    "MCP connection / tool discovery: Synthetic service",
                    "Searching: a synthetic query",
                    "Reading: https://example.com/reference",
                    "Skills instructions loaded on demand",
                ];
                let mut responses = vec![];
                let output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(320.0, 600.0),
                        )),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            apply(ui, size);
                            for sample in samples {
                                responses.push(activity_label(ui, sample));
                            }
                            ui.label("NORMAL_ANSWER_BODY");
                            assert_eq!(
                                ui.style().text_styles[&TextStyle::Body].size,
                                f32::from(size)
                            );
                        });
                    },
                );
                for sample in samples {
                    let expected = format!("… {sample}");
                    let text = output
                        .shapes
                        .iter()
                        .find_map(|shape| match &shape.shape {
                            egui::Shape::Text(text) if text.galley.job.text == expected => {
                                Some(text)
                            }
                            _ => None,
                        })
                        .unwrap();
                    assert!(text
                        .galley
                        .job
                        .sections
                        .iter()
                        .all(|s| s.format.font_id.size == ACTIVITY_SIZE));
                }
                let normal = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.job.text == "NORMAL_ANSWER_BODY" => {
                            Some(text)
                        }
                        _ => None,
                    })
                    .unwrap();
                assert!(normal
                    .galley
                    .job
                    .sections
                    .iter()
                    .all(|s| s.format.font_id.size == f32::from(size)));
                assert!(responses
                    .iter()
                    .all(|response| response.rect.width() <= 304.1));
            }
        }
    }

    #[test]
    fn chat_styles_are_local_inherited_idempotent_and_leave_chrome_alone() {
        for appearance in ["light", "dark"] {
            let ctx = egui::Context::default();
            super::super::theme::apply(&ctx, appearance, super::super::theme::DEFAULT_ACCENT_HEX);
            let original = ctx.style();
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    assert_eq!(scaled(ui, 13.0), 13.0);
                    for size in [12, 14, 15, 20, 24] {
                        ui.scope(|ui| {
                            apply(ui, size);
                            apply(ui, size);
                            assert_eq!(
                                ui.style().text_styles[&TextStyle::Body].size,
                                f32::from(size)
                            );
                            assert_eq!(
                                ui.style().text_styles[&TextStyle::Monospace].size,
                                13.0 * scale(size)
                            );
                            for style in [TextStyle::Small, TextStyle::Button, TextStyle::Heading] {
                                assert_eq!(
                                    ui.style().text_styles[&style],
                                    original.text_styles[&style]
                                );
                            }
                            ui.scope(|ui| assert_eq!(scaled(ui, 12.0), 12.0 * scale(size)));
                        });
                        assert_eq!(scaled(ui, 13.0), 13.0);
                    }
                });
            });
            assert_eq!(ctx.style().text_styles, original.text_styles);
        }
        assert_eq!(normalize(0), MIN_SIZE);
        assert_eq!(normalize(u16::MAX), MAX_SIZE);
        assert_eq!(scale(DEFAULT_SIZE), 1.0);
        assert_eq!(line_height(DEFAULT_SIZE), 19.0);
    }
}
