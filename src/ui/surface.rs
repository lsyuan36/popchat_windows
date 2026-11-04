//! The native transparent canvas and its one, bounded card shadow.

use eframe::egui;

pub const CARD_MARGIN: i8 = 12;
pub const CARD_RADIUS: u8 = 24;
/// Root-card margin plus breathing room for a modal's soft shadow.
pub const DIALOG_CLEARANCE: f32 = 26.0;

pub fn dialog_content_width(ctx: &egui::Context, max_width: f32, inner_margin: f32) -> f32 {
    (ctx.screen_rect().width() - 2.0 * (DIALOG_CLEARANCE + inner_margin + 1.0))
        .clamp(160.0, max_width)
}

/// Popups need a much softer, smaller shadow than egui's opaque stock shadow.
/// Keep its reach inside the space reserved by the transparent native canvas.
pub fn popup_shadow() -> egui::Shadow {
    egui::Shadow {
        offset: [0, 2],
        blur: 12,
        spread: 0,
        color: egui::Color32::from_black_alpha(40),
    }
}

/// Preserve egui's modal input/escape/outside-click handling, but never paint a
/// square backdrop across our transparent native window (including its corners).
pub fn modal(ctx: &egui::Context, id: egui::Id, frame: egui::Frame) -> egui::Modal {
    let area = egui::Modal::default_area(id);
    let card = ctx.screen_rect().shrink(f32::from(CARD_MARGIN));
    ctx.layer_painter(area.layer()).rect_filled(
        card,
        egui::CornerRadius::same(CARD_RADIUS),
        egui::Color32::from_black_alpha(180),
    );
    egui::Modal::new(id)
        .area(area)
        .backdrop_color(egui::Color32::TRANSPARENT)
        .frame(frame.shadow(popup_shadow()))
}

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
        .corner_radius(egui::CornerRadius::same(CARD_RADIUS))
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

    fn scene(dark: bool, size: egui::Vec2, legacy: bool) -> (egui::Context, Vec<egui::FullOutput>) {
        let ctx = egui::Context::default();
        super::super::theme::apply(
            &ctx,
            if dark { "dark" } else { "light" },
            super::super::theme::DEFAULT_ACCENT_HEX,
        );
        let mut frames = vec![];
        for index in 0..5 {
            frames.push(ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    time: Some(index as f64 / 4.0),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default()
                        .frame(canvas_frame())
                        .show(ctx, |ui| {
                            card_frame(dark).show(ui, |ui| {
                                ui.set_min_size(ui.available_size());
                                ui.label("Underlying settings card");
                            });
                        });
                    let frame = egui::Frame::popup(&ctx.style())
                        .corner_radius(14)
                        .inner_margin(18);
                    let id = egui::Id::new("rounded-native-modal-test");
                    let dialog = if legacy {
                        egui::Modal::new(id)
                            .backdrop_color(egui::Color32::from_black_alpha(180))
                            .frame(frame)
                    } else {
                        modal(ctx, id, frame)
                    };
                    dialog.show(ctx, |ui| {
                        ui.set_width((size.x - 110.0).clamp(100.0, 240.0));
                        ui.label("Human confirmation only");
                        let _ = ui.button("Close");
                    });
                },
            ));
        }
        (ctx, frames)
    }

    #[test]
    fn modal_backdrop_does_not_reintroduce_the_native_square_rectangle() {
        use super::super::mcp::preview;
        for dark in [false, true] {
            for size in [egui::vec2(320.0, 480.0), egui::vec2(720.0, 650.0)] {
                let (ctx, frames) = scene(dark, size, false);
                let image = preview::render(&ctx, &frames, size, [0; 4]);
                for (x, y) in [
                    (0, 0),
                    (size.x as u32 - 1, 0),
                    (0, size.y as u32 - 1),
                    (size.x as u32 - 1, size.y as u32 - 1),
                    (0, size.y as u32 / 2),
                    (size.x as u32 / 2, 0),
                ] {
                    assert_eq!(
                        image.get_pixel(x, y).0[3],
                        0,
                        "native transparent margin must stay clear: {dark} {size:?} {x},{y}"
                    );
                }
                let backdrop = frames
                    .last()
                    .unwrap()
                    .shapes
                    .iter()
                    .filter_map(|shape| match &shape.shape {
                        egui::Shape::Rect(rect)
                            if rect.fill == egui::Color32::from_black_alpha(180) =>
                        {
                            Some(rect)
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                assert_eq!(backdrop.len(), 1);
                assert_eq!(
                    backdrop[0].corner_radius,
                    egui::CornerRadius::same(CARD_RADIUS)
                );
                assert_eq!(
                    backdrop[0].rect,
                    egui::Rect::from_min_size(egui::Pos2::ZERO, size)
                        .shrink(f32::from(CARD_MARGIN))
                );
                let (old_ctx, old_frames) = scene(dark, size, true);
                let legacy = preview::render(&old_ctx, &old_frames, size, [0; 4]);
                assert_eq!(
                    legacy.get_pixel(0, 0).0[3],
                    180,
                    "fixture must reproduce the old square dimming"
                );
                preview::export(
                    &ctx,
                    &frames,
                    size,
                    &format!(
                        "modal-fixed-{}-{}.png",
                        if dark { "dark" } else { "light" },
                        size.x as u32
                    ),
                    dark,
                );
                preview::export(
                    &old_ctx,
                    &old_frames,
                    size,
                    &format!(
                        "modal-before-{}-{}.png",
                        if dark { "dark" } else { "light" },
                        size.x as u32
                    ),
                    dark,
                );
            }
        }
    }

    #[test]
    fn transparent_modal_still_blocks_underlying_clicks_and_keeps_escape_and_outside_close() {
        let ctx = egui::Context::default();
        let size = egui::vec2(600.0, 450.0);
        let mut underlying_clicked = false;
        let mut draw = |events| {
            let mut close = false;
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        underlying_clicked |= ui.button("Must not be activated").clicked();
                    });
                    close = modal(
                        ctx,
                        egui::Id::new("modal-input-test"),
                        egui::Frame::popup(&ctx.style()),
                    )
                    .show(ctx, |ui| {
                        ui.label("Confirmation required");
                    })
                    .should_close();
                },
            );
            (output, close)
        };
        for _ in 0..5 {
            assert!(!draw(vec![]).1);
        }
        let position = egui::pos2(20.0, 15.0);
        draw(vec![
            egui::Event::PointerMoved(position),
            egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        assert!(
            draw(vec![egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE
            }])
            .1
        );
        assert!(
            draw(vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE
            }])
            .1
        );
        assert!(!underlying_clicked);
    }

    #[test]
    fn all_stock_popup_surfaces_use_the_same_soft_bounded_shadow() {
        for dark in [false, true] {
            let ctx = egui::Context::default();
            super::super::theme::apply(
                &ctx,
                if dark { "dark" } else { "light" },
                super::super::theme::DEFAULT_ACCENT_HEX,
            );
            let frame = egui::Frame::popup(&ctx.style());
            assert_eq!(frame.shadow, popup_shadow());
            assert_eq!(frame.stroke.width, 1.0);
            assert!(frame.stroke.color.a() <= 23);
            let shadow = frame.shadow;
            assert!(
                f32::from(shadow.blur) * 0.5
                    + f32::from(shadow.offset[1].abs())
                    + frame.stroke.width
                    < f32::from(CARD_MARGIN)
            );
        }
    }

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
