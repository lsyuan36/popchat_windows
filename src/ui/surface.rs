//! The native transparent canvas and its one, bounded card shadow.

use eframe::egui;

pub const CARD_MARGIN: i8 = 12;

/// eframe's default is a dark rectangle with alpha 180, even when the native
/// window and CentralPanel are transparent. Clear every frame to actual zero.
pub fn clear_color() -> [f32; 4] {
    [0.0; 4]
}

pub fn canvas_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(egui::Color32::TRANSPARENT)
        .inner_margin(egui::Margin::same(CARD_MARGIN))
}

pub fn card_frame(dark: bool) -> egui::Frame {
    egui::Frame::new()
        .fill(super::theme::panel_fill(dark))
        .stroke(if dark {
            egui::Stroke::new(1.0_f32, egui::Color32::from_white_alpha(23))
        } else {
            egui::Stroke::new(1.0_f32, egui::Color32::from_black_alpha(16))
        })
        .corner_radius(egui::CornerRadius::same(24))
        .inner_margin(egui::Margin::symmetric(22, 18))
        .shadow(egui::Shadow {
            offset: [0, 2],
            blur: 16,
            spread: 0,
            color: egui::Color32::from_black_alpha(48),
        })
}

/// Only the backdrop/shadow changes opacity; child text remains fully opaque.
pub fn card_frame_with_opacity(dark: bool, opacity: f32) -> egui::Frame {
    let opacity = if opacity.is_finite() {
        opacity.clamp(0.35, 1.0)
    } else {
        1.0
    };
    let mut frame = card_frame(dark);
    frame.fill = frame.fill.gamma_multiply(opacity);
    frame.stroke.color = frame.stroke.color.gamma_multiply(opacity);
    frame.shadow.color = frame.shadow.color.gamma_multiply(opacity);
    frame
}

/// Header-only drag target. Never cover transcript selection, input or buttons.
pub fn drag_window(ui: &mut egui::Ui, rect: egui::Rect, id: egui::Id) -> egui::Response {
    let response = ui
        .interact(rect, id, egui::Sense::drag())
        .on_hover_cursor(egui::CursorIcon::Grab)
        .on_hover_text("拖曳移動視窗");
    if response.drag_started() {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opacity_changes_backdrop_not_text_and_never_restores_the_dark_clear_rectangle() {
        assert_eq!(clear_color(), [0.0; 4]);
        for dark in [false, true] {
            let ctx = egui::Context::default();
            super::super::theme::apply(
                &ctx,
                if dark { "dark" } else { "light" },
                super::super::theme::DEFAULT_ACCENT_HEX,
            );
            let output = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default()
                    .frame(canvas_frame())
                    .show(ctx, |ui| {
                        card_frame_with_opacity(dark, 0.5).show(ui, |ui| {
                            ui.label("Opaque text");
                        });
                    });
            });
            let text = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == "Opaque text" => Some(text),
                    _ => None,
                })
                .unwrap();
            let format_color = text.galley.job.sections[0].format.color;
            let painted_color = text.override_text_color.unwrap_or_else(|| {
                if format_color == egui::Color32::PLACEHOLDER {
                    text.fallback_color
                } else {
                    format_color
                }
            });
            assert_eq!(painted_color.a(), 255);
            assert_eq!(card_frame_with_opacity(dark, 0.5).fill.a(), 128);
            assert_eq!(card_frame_with_opacity(dark, f32::NAN).fill.a(), 255);
            assert_eq!(
                card_frame_with_opacity(dark, 0.0).fill,
                card_frame_with_opacity(dark, 0.35).fill
            );
        }
    }

    #[test]
    fn header_drag_sends_native_drag_but_leaves_adjacent_buttons_clickable() {
        let ctx = egui::Context::default();
        let drag = egui::Rect::from_min_size(egui::pos2(10.0, 10.0), egui::vec2(100.0, 28.0));
        let mut click = egui::Rect::NOTHING;
        let mut clicked = false;
        let mut draw = |events| {
            let output = ctx.run(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        drag_window(ui, drag, egui::Id::new("header-drag-test"));
                        let rect = egui::Rect::from_min_size(
                            egui::pos2(125.0, 10.0),
                            egui::vec2(28.0, 28.0),
                        );
                        let response = ui.interact(
                            rect,
                            egui::Id::new("adjacent-control"),
                            egui::Sense::click(),
                        );
                        click = response.rect;
                        clicked = response.clicked();
                    });
                },
            );
            (output, click, clicked)
        };
        draw(vec![]);
        draw(vec![]);
        let pos = drag.center();
        let (pressed, _, _) = draw(vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        let (output, _, _) = draw(vec![egui::Event::PointerMoved(pos + egui::vec2(25.0, 6.0))]);
        assert!([&pressed, &output].iter().any(|frame| frame.viewport_output
            [&egui::ViewportId::ROOT]
            .commands
            .iter()
            .any(|cmd| matches!(cmd, egui::ViewportCommand::StartDrag))));
        draw(vec![egui::Event::PointerButton {
            pos: pos + egui::vec2(25.0, 6.0),
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        let (_, rect, _) = draw(vec![]);
        let pos = rect.center();
        draw(vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        let (_, _, clicked) = draw(vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        assert!(clicked);
    }

    #[test]
    fn native_clear_and_canvas_never_dim_the_rectangular_window_background() {
        assert_eq!(clear_color(), [0.0, 0.0, 0.0, 0.0]);
        let canvas = canvas_frame();
        assert_eq!(canvas.fill, egui::Color32::TRANSPARENT);
        assert_eq!(canvas.shadow, egui::Shadow::NONE);
        assert_eq!(canvas.stroke, egui::Stroke::NONE);
    }

    #[test]
    fn card_shadow_fits_inside_canvas_without_a_clipped_rectangle_in_both_themes() {
        for dark in [false, true] {
            let frame = card_frame(dark);
            let shadow = frame.shadow;
            // Epaint expands a shadow by spread + blur/2 on every side.
            let reach = f32::from(shadow.spread) + f32::from(shadow.blur) * 0.5;
            for offset in shadow.offset {
                assert!(
                    reach + f32::from(offset).abs() + frame.stroke.width <= f32::from(CARD_MARGIN)
                );
            }
            let ctx = egui::Context::default();
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(604.0, 344.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default()
                        .frame(canvas_frame())
                        .show(ctx, |ui| {
                            card_frame(dark).show(ui, |ui| {
                                ui.set_min_size(ui.available_size());
                            });
                        });
                },
            );
            let meshes = ctx.tessellate(output.shapes, output.pixels_per_point);
            let mut painted = 0;
            for clipped in meshes {
                if let egui::epaint::Primitive::Mesh(mesh) = clipped.primitive {
                    for vertex in mesh.vertices.iter().filter(|vertex| vertex.color.a() != 0) {
                        painted += 1;
                        assert!(vertex.pos.x > 0.0 && vertex.pos.x < 604.0);
                        assert!(vertex.pos.y > 0.0 && vertex.pos.y < 344.0);
                    }
                }
            }
            assert!(painted > 0, "the actual card must still be painted");
        }
    }
}
