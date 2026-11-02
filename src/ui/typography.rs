//! Chat-only typography. Apply to a dedicated child Ui, never the Context style.

use eframe::egui::{FontId, TextStyle, Ui};

pub const DEFAULT_SIZE: u16 = 15;
pub const MIN_SIZE: u16 = 12;
pub const MAX_SIZE: u16 = 24;

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
