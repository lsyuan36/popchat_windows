//! Shared, responsive layout for the appearance preference groups.

use egui::{self, Align, InnerResponse, Layout, Ui};

pub fn caption(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(egui::RichText::new(text).size(12.0).weak()).wrap());
}

pub fn heading(ui: &mut Ui, title: &str) {
    ui.label(egui::RichText::new(title).size(15.0).strong());
    ui.add_space(8.0);
}

pub fn card<R>(ui: &mut Ui, contents: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    let dark = ui.visuals().dark_mode;
    egui::Frame::new()
        .fill(if dark {
            egui::Color32::from_white_alpha(7)
        } else {
            super::theme::LIGHT_RAISED
        })
        .stroke(egui::Stroke::new(
            1.0_f32,
            if dark {
                egui::Color32::from_white_alpha(16)
            } else {
                egui::Color32::from_black_alpha(18)
            },
        ))
        .corner_radius(12)
        .inner_margin(16)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing = egui::vec2(10.0, 6.0);
            ui.spacing_mut().interact_size.y = 32.0;
            let widgets = &mut ui.visuals_mut().widgets;
            let control_fill = if dark {
                egui::Color32::from_rgb(44, 47, 56)
            } else {
                super::theme::LIGHT_CONTROL
            };
            widgets.inactive.bg_fill = control_fill;
            widgets.inactive.weak_bg_fill = control_fill;
            widgets.inactive.bg_stroke = egui::Stroke::new(
                1.0_f32,
                if dark {
                    egui::Color32::from_white_alpha(18)
                } else {
                    egui::Color32::from_black_alpha(22)
                },
            );
            contents(ui)
        })
}

/// Align labels and controls in two columns, stacking only on narrow surfaces.
pub fn row<R>(ui: &mut Ui, label: &str, control: impl FnOnce(&mut Ui) -> R) -> R {
    let width = ui.available_width();
    if width < 350.0 {
        ui.label(egui::RichText::new(label).size(13.0));
        ui.add_space(2.0);
        ui.horizontal(control).inner
    } else {
        let control_width = (width * 0.56).min(260.0);
        let label_width = width - control_width - ui.spacing().item_spacing.x;
        ui.horizontal(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(label_width, 32.0),
                Layout::left_to_right(Align::Center),
                |ui| {
                    // allocate_ui's requested size is not a minimum: a short
                    // label would otherwise collapse this column to its text.
                    ui.set_min_size(egui::vec2(label_width, 32.0));
                    ui.label(egui::RichText::new(label).size(13.0));
                },
            );
            ui.allocate_ui_with_layout(
                egui::vec2(control_width, 32.0),
                Layout::left_to_right(Align::Center),
                |ui| {
                    ui.set_min_size(egui::vec2(control_width, 32.0));
                    control(ui)
                },
            )
            .inner
        })
        .inner
    }
}

pub fn divider(ui: &mut Ui) {
    ui.add_space(10.0);
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter().hline(
        rect.x_range(),
        rect.center().y,
        egui::Stroke::new(1.0_f32, ui.visuals().text_color().gamma_multiply(0.09)),
    );
    ui.add_space(10.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn light_preference_card_and_controls_use_shared_cream_surfaces() {
        let ctx = egui::Context::default();
        super::super::theme::apply(&ctx, "Light", super::super::theme::DEFAULT_ACCENT_HEX);
        let mut bounds = egui::Rect::NOTHING;
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                bounds = card(ui, |ui| {
                    assert_eq!(
                        ui.visuals().widgets.inactive.bg_fill,
                        super::super::theme::LIGHT_CONTROL
                    );
                    assert_eq!(
                        ui.visuals().widgets.inactive.weak_bg_fill,
                        super::super::theme::LIGHT_CONTROL
                    );
                    ui.label("Appearance");
                })
                .response
                .rect;
            });
        });
        assert!(output.shapes.iter().any(|shape| matches!(
            &shape.shape,
            egui::Shape::Rect(rect) if rect.rect == bounds && rect.fill == super::super::theme::LIGHT_RAISED
        )));
    }

    #[test]
    fn preference_rows_align_and_stay_within_narrow_and_wide_cards() {
        for width in [300.0, 380.0, 520.0, 604.0] {
            for dark in [false, true] {
                let ctx = egui::Context::default();
                ctx.set_visuals(if dark {
                    egui::Visuals::dark()
                } else {
                    egui::Visuals::light()
                });
                let mut controls = Vec::new();
                let mut bounds = egui::Rect::NOTHING;
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 700.0),
                        )),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            bounds = card(ui, |ui| {
                                for title in ["中文字體", "預設跳出位置"] {
                                    row(ui, title, |ui| {
                                        let response = egui::ComboBox::from_id_salt(title)
                                            .width(ui.available_width())
                                            .selected_text("系統預設")
                                            .show_ui(ui, |_| {});
                                        controls.push(response.response.rect);
                                    });
                                }
                            })
                            .response
                            .rect;
                        });
                    },
                );
                assert!(bounds.right() <= width - 7.0, "{width}: {bounds:?}");
                assert!((controls[0].left() - controls[1].left()).abs() < 0.5);
                assert!((controls[0].width() - controls[1].width()).abs() < 0.5);
                for rect in controls {
                    assert!(
                        bounds.shrink(14.0).contains_rect(rect),
                        "{width}: {rect:?} / {bounds:?}"
                    );
                    assert!(rect.height() >= 32.0);
                    assert!((rect.right() - (bounds.right() - 17.0)).abs() < 1.5,
                        "control must align to the card's right inset: {width}: {rect:?} / {bounds:?}");
                }
            }
        }
    }
}
