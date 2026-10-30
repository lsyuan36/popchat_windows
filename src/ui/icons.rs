//! Small, font-independent controls for the floating panel.

use eframe::egui;

#[derive(Clone, Copy)]
pub enum Icon {
    Add,
    NewChat,
    History,
    Copy,
    Check,
    Fork,
    Retry,
    Edit,
    Close,
    Settings,
    Pin,
    Send,
    Stop,
    Expand,
}

pub fn button(ui: &mut egui::Ui, icon: Icon, active: bool, label: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });
    if ui.is_rect_visible(rect) {
        let palette = super::theme::current_palette(ui.ctx());
        let visuals = ui.style().interact(&response);
        let primary_action = matches!(icon, Icon::Send | Icon::Stop);
        let fill = if active && matches!(icon, Icon::Stop) {
            egui::Color32::from_rgb(0xCB, 0x42, 0x48)
        } else if active && primary_action {
            palette.accent
        } else if active {
            palette
                .accent
                .gamma_multiply(if ui.visuals().dark_mode { 0.22 } else { 0.09 })
        } else if response.hovered() || response.has_focus() {
            visuals.bg_fill
        } else {
            egui::Color32::TRANSPARENT
        };
        let color = if active && primary_action {
            palette.accent_text
        } else if active {
            palette.accent
        } else {
            ui.visuals().weak_text_color()
        };
        let painter = ui.painter();
        let center = rect.center();
        painter.circle_filled(center, 14.0, fill);
        if response.has_focus() {
            painter.circle_stroke(center, 13.0, egui::Stroke::new(1.5_f32, palette.accent));
        }
        let stroke = egui::Stroke::new(1.6_f32, color);
        let point = |x, y| center + egui::vec2(x, y);
        let line = |a, b| painter.line_segment([a, b], stroke);
        match icon {
            Icon::Add => {
                line(point(-5.0, 0.0), point(5.0, 0.0));
                line(point(0.0, -5.0), point(0.0, 5.0));
            }
            Icon::NewChat => {
                painter.rect_stroke(
                    egui::Rect::from_min_max(point(-6.0, -6.0), point(6.0, 4.0)),
                    3,
                    stroke,
                    egui::StrokeKind::Inside,
                );
                line(point(-3.0, 4.0), point(-5.0, 7.0));
                line(point(-5.0, 7.0), point(1.0, 4.0));
                line(point(-2.5, -1.0), point(2.5, -1.0));
                line(point(0.0, -3.5), point(0.0, 1.5));
            }
            Icon::History => {
                painter.circle_stroke(center, 6.0, stroke);
                line(point(0.0, -3.5), point(0.0, 0.0));
                line(point(0.0, 0.0), point(3.0, 2.0));
            }
            Icon::Copy => {
                line(point(-6.0, 2.0), point(-6.0, -6.0));
                line(point(-6.0, -6.0), point(2.0, -6.0));
                painter.rect_stroke(
                    egui::Rect::from_min_max(point(-2.0, -2.0), point(6.0, 6.0)),
                    2,
                    stroke,
                    egui::StrokeKind::Inside,
                );
            }
            Icon::Check => {
                line(point(-5.0, 0.0), point(-1.0, 4.0));
                line(point(-1.0, 4.0), point(6.0, -4.0));
            }
            Icon::Fork => {
                line(point(-3.0, -3.5), point(-3.0, 4.5));
                line(point(-3.0, 1.0), point(2.0, 1.0));
                line(point(2.0, 1.0), point(5.0, -2.5));
                for (x, y) in [(-3.0, -5.0), (-3.0, 6.0), (5.0, -4.0)] {
                    painter.circle_stroke(point(x, y), 1.6, stroke);
                }
            }
            Icon::Retry => {
                let arc = (0..=20)
                    .map(|step| {
                        let angle = -std::f32::consts::FRAC_PI_2
                            + step as f32 * std::f32::consts::TAU * 0.875 / 20.0;
                        center + egui::vec2(angle.cos(), angle.sin()) * 6.0
                    })
                    .collect::<Vec<_>>();
                painter.add(egui::Shape::line(arc, stroke));
                line(point(-6.0, -1.0), point(-6.0, -6.0));
                line(point(-6.0, -6.0), point(-1.0, -6.0));
            }
            Icon::Edit => {
                painter.add(egui::Shape::closed_line(
                    vec![
                        point(-6.0, 6.0),
                        point(-5.0, 2.0),
                        point(3.0, -6.0),
                        point(6.0, -3.0),
                        point(-2.0, 5.0),
                    ],
                    stroke,
                ));
                line(point(1.0, -4.0), point(4.0, -1.0));
            }
            Icon::Close => {
                line(point(-4.0, -4.0), point(4.0, 4.0));
                line(point(-4.0, 4.0), point(4.0, -4.0));
            }
            Icon::Settings => {
                painter.circle_stroke(center, 4.5, stroke);
                painter.circle_stroke(center, 1.5, stroke);
                for step in 0..8 {
                    let angle = step as f32 * std::f32::consts::TAU / 8.0;
                    let direction = egui::vec2(angle.cos(), angle.sin());
                    line(center + direction * 5.0, center + direction * 6.5);
                }
            }
            Icon::Pin => {
                line(point(-3.0, -6.0), point(3.0, -6.0));
                line(point(-2.0, -6.0), point(-2.0, -1.0));
                line(point(2.0, -6.0), point(2.0, -1.0));
                line(point(-2.0, -1.0), point(-4.5, 2.0));
                line(point(2.0, -1.0), point(4.5, 2.0));
                line(point(-4.5, 2.0), point(4.5, 2.0));
                line(point(0.0, 2.0), point(0.0, 7.0));
            }
            Icon::Send => {
                line(point(0.0, 6.0), point(0.0, -5.0));
                line(point(-4.5, -0.5), point(0.0, -5.0));
                line(point(4.5, -0.5), point(0.0, -5.0));
            }
            Icon::Stop => {
                painter.rect_filled(
                    egui::Rect::from_center_size(center, egui::vec2(9.0, 9.0)),
                    2.0,
                    color,
                );
            }
            Icon::Expand => {
                line(point(-5.0, 5.0), point(5.0, -5.0));
                line(point(0.0, -5.0), point(5.0, -5.0));
                line(point(5.0, -5.0), point(5.0, 0.0));
                line(point(-5.0, 0.0), point(-5.0, 5.0));
                line(point(-5.0, 5.0), point(0.0, 5.0));
            }
        }
    }
    response.on_hover_text(label)
}

/// A small drawn brand mark, independent of emoji or system symbol fonts.
pub fn mark(ui: &mut egui::Ui, size: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    let palette = super::theme::current_palette(ui.ctx());
    let fill = if ui.visuals().dark_mode {
        egui::Color32::from_rgb(0x28, 0x35, 0x50)
    } else {
        egui::Color32::from_rgb(0xE9, 0xF1, 0xFF)
    };
    ui.painter().rect_filled(rect, (size * 0.3) as u8, fill);
    let c = rect.center();
    let radius = size * 0.25;
    let mut points = Vec::new();
    for step in 0..8 {
        let angle = step as f32 * std::f32::consts::TAU / 8.0;
        let r = if step % 2 == 0 { radius } else { radius * 0.30 };
        points.push(c + egui::vec2(angle.cos(), angle.sin()) * r);
    }
    ui.painter().add(egui::Shape::convex_polygon(
        points,
        palette.accent,
        egui::Stroke::NONE,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composer_and_header_icons_have_equal_square_hit_targets() {
        for appearance in ["dark", "light"] {
            let ctx = egui::Context::default();
            super::super::theme::apply(&ctx, appearance, super::super::theme::DEFAULT_ACCENT_HEX);
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        for icon in [
                            Icon::Add,
                            Icon::NewChat,
                            Icon::History,
                            Icon::Copy,
                            Icon::Check,
                            Icon::Fork,
                            Icon::Retry,
                            Icon::Edit,
                            Icon::Expand,
                            Icon::Send,
                            Icon::Stop,
                            Icon::Pin,
                            Icon::Settings,
                            Icon::Close,
                        ] {
                            let response = button(ui, icon, false, "test control");
                            assert_eq!(response.rect.size(), egui::vec2(28.0, 28.0));
                        }
                    });
                });
            });
        }
    }
}
