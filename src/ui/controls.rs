//! Compact controls with real vertical padding, rather than egui's one-line
//! `small_button`, which intentionally removes the vertical button padding.

use super::icons::{self, Icon};
use eframe::egui;

pub const COMPACT_HEIGHT: f32 = 26.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatNavigation {
    None,
    NewChat,
    History,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageControl {
    None,
    Copy,
    Fork,
    Retry,
    Edit,
}

/// Icon-only message actions; callers retain all clipboard/conversation effects.
pub fn message_actions_inline(
    ui: &mut egui::Ui,
    copied: bool,
    can_fork: bool,
    rerunnable: bool,
) -> MessageControl {
    let mut action = MessageControl::None;
    if icons::button(
        ui,
        if copied { Icon::Check } else { Icon::Copy },
        copied,
        if copied { "已複製" } else { "複製訊息" },
    )
    .clicked()
    {
        action = MessageControl::Copy;
    }
    if ui
        .add_enabled_ui(can_fork, |ui| {
            icons::button(ui, Icon::Fork, false, "從這裡建立分支，不更動原聊天")
        })
        .inner
        .clicked()
    {
        action = MessageControl::Fork;
    }
    if rerunnable {
        if icons::button(ui, Icon::Retry, false, "以目前模型重試回覆").clicked() {
            action = MessageControl::Retry;
        }
        if icons::button(ui, Icon::Edit, false, "編輯提問").clicked() {
            action = MessageControl::Edit;
        }
    }
    action
}

/// Find remains a keyboard action (Ctrl+F/G), not a persistent toolbar button.
pub fn chat_navigation(ui: &mut egui::Ui, history_open: bool) -> ChatNavigation {
    ui.horizontal(|ui| chat_navigation_inline(ui, history_open))
        .inner
}

/// Draw directly into the header's existing row, beside Pin/Settings/Close.
pub fn chat_navigation_inline(ui: &mut egui::Ui, history_open: bool) -> ChatNavigation {
    let mut action = ChatNavigation::None;
    if icons::button(ui, Icon::NewChat, false, "新聊天 · Ctrl+N").clicked() {
        action = ChatNavigation::NewChat;
    }
    if icons::button(
        ui,
        Icon::History,
        history_open,
        "歷史紀錄／搜尋聊天 · Ctrl+Y",
    )
    .clicked()
    {
        action = ChatNavigation::History;
    }
    action
}

pub fn header_actions_width(ui: &egui::Ui, settings_open: bool) -> f32 {
    let count = if settings_open { 3.0 } else { 5.0 };
    count * (28.0 + ui.spacing().item_spacing.x)
}

pub fn button(ui: &mut egui::Ui, text: impl Into<egui::WidgetText>) -> egui::Response {
    toggle(ui, text, false)
}

pub fn toggle(
    ui: &mut egui::Ui,
    text: impl Into<egui::WidgetText>,
    selected: bool,
) -> egui::Response {
    let text = text.into();
    ui.scope(|ui| {
        ui.spacing_mut().button_padding = egui::vec2(9.0, 4.0);
        ui.spacing_mut().interact_size.y = COMPACT_HEIGHT;
        let mut button = egui::Button::new(if selected {
            text.color(ui.visuals().text_color())
        } else {
            text
        })
        .min_size(egui::vec2(0.0, COMPACT_HEIGHT))
        .corner_radius(6)
        .selected(selected);
        if selected {
            // A saturated accent with theme-white text is unreadable for
            // green/yellow accents. Keep selected navigation softly tinted
            // and explicitly use the normal high-contrast foreground.
            let accent = super::theme::current_palette(ui.ctx()).accent;
            button = button
                .fill(accent.gamma_multiply(if ui.visuals().dark_mode { 0.18 } else { 0.10 }))
                .stroke(egui::Stroke::new(1.0_f32, accent.gamma_multiply(0.45)));
        }
        ui.add(button)
    })
    .inner
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actual_chat_navigation_has_no_find_button() {
        let ctx = egui::Context::default();
        super::super::theme::apply(&ctx, "dark", super::super::theme::DEFAULT_ACCENT_HEX);
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                assert_eq!(chat_navigation(ui, false), ChatNavigation::None);
            });
        });
        let labels = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text.galley.text()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(labels.is_empty(), "navigation is icon-only, like Settings");
        assert!(!labels.iter().any(|text| text.contains("尋找")));
    }

    #[test]
    fn icon_navigation_keeps_both_actions_clickable() {
        for (target, expected) in [(0, ChatNavigation::NewChat), (1, ChatNavigation::History)] {
            let ctx = egui::Context::default();
            super::super::theme::apply(&ctx, "light", super::super::theme::DEFAULT_ACCENT_HEX);
            let draw = |events| {
                let mut action = ChatNavigation::None;
                let mut pos = egui::Pos2::ZERO;
                let _ = ctx.run(
                    egui::RawInput {
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            pos = ui.cursor().min
                                + egui::vec2(
                                    14.0 + target as f32 * (28.0 + ui.spacing().item_spacing.x),
                                    14.0,
                                );
                            action = chat_navigation(ui, true);
                        });
                    },
                );
                (action, pos)
            };
            let (_, pos) = draw(vec![]);
            draw(vec![]);
            for pressed in [true, false] {
                let (action, _) = draw(vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]);
                if !pressed {
                    assert_eq!(action, expected);
                }
            }
        }
    }

    #[test]
    fn inline_navigation_shares_one_row_with_pin_and_settings() {
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let reserved = header_actions_width(ui, false);
                let row = ui.horizontal(|ui| {
                    let start = ui.cursor().min.x;
                    chat_navigation_inline(ui, false);
                    let pin = icons::button(ui, Icon::Pin, false, "釘選");
                    let settings = icons::button(ui, Icon::Settings, false, "設定");
                    let close = icons::button(ui, Icon::Close, false, "關閉");
                    assert!(
                        (pin.rect.left() - start - 2.0 * (28.0 + ui.spacing().item_spacing.x))
                            .abs()
                            < 0.1
                    );
                    assert_eq!(pin.rect.center().y, settings.rect.center().y);
                    assert_eq!(pin.rect.center().y, close.rect.center().y);
                });
                assert!(row.response.rect.height() <= 32.0);
                assert!(row.response.rect.width() <= reserved);
            });
        });
    }

    #[test]
    fn message_icons_keep_actions_and_disable_fork_during_streams() {
        for (target, can_fork, rerunnable, expected) in [
            (0, true, true, MessageControl::Copy),
            (1, true, true, MessageControl::Fork),
            (1, false, true, MessageControl::None),
            (2, true, true, MessageControl::Retry),
            (3, true, true, MessageControl::Edit),
            (2, true, false, MessageControl::None),
        ] {
            let ctx = egui::Context::default();
            let draw = |events| {
                let mut action = MessageControl::None;
                let mut pos = egui::Pos2::ZERO;
                let output = ctx.run(
                    egui::RawInput {
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            ui.horizontal(|ui| {
                                pos = ui.cursor().min
                                    + egui::vec2(
                                        14.0 + target as f32 * (28.0 + ui.spacing().item_spacing.x),
                                        14.0,
                                    );
                                action = message_actions_inline(ui, true, can_fork, rerunnable);
                            });
                        });
                    },
                );
                (action, pos, output)
            };
            let (_, pos, output) = draw(vec![]);
            assert!(
                !output
                    .shapes
                    .iter()
                    .any(|s| matches!(s.shape, egui::Shape::Text(_))),
                "no inline message-action text"
            );
            draw(vec![]);
            for pressed in [true, false] {
                let (action, _, _) = draw(vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]);
                if !pressed {
                    assert_eq!(action, expected);
                }
            }
        }
    }

    #[test]
    fn selected_navigation_keeps_readable_text_for_bright_and_dark_custom_accents() {
        fn luminance(color: egui::Color32) -> f32 {
            let linear = |byte: u8| {
                let c = f32::from(byte) / 255.0;
                if c <= 0.04045 {
                    c / 12.92
                } else {
                    ((c + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * linear(color.r()) + 0.7152 * linear(color.g()) + 0.0722 * linear(color.b())
        }
        for appearance in ["dark", "light"] {
            for accent in ["#30D158", "#FFFF00", "#FFFFFF", "#000000", "#0A84FF"] {
                let ctx = egui::Context::default();
                let palette = super::super::theme::apply(&ctx, appearance, accent);
                let fill = palette
                    .accent
                    .gamma_multiply(if palette.dark { 0.18 } else { 0.10 });
                let base = super::super::theme::panel_fill(palette.dark);
                let blend = |foreground: u8, background: u8| {
                    (f32::from(foreground)
                        + f32::from(background) * (1.0 - f32::from(fill.a()) / 255.0))
                        .round() as u8
                };
                let background = egui::Color32::from_rgb(
                    blend(fill.r(), base.r()),
                    blend(fill.g(), base.g()),
                    blend(fill.b(), base.b()),
                );
                let primary = ctx.style().visuals.text_color();
                let a = luminance(primary);
                let b = luminance(background);
                assert!(
                    (a.max(b) + 0.05) / (a.min(b) + 0.05) >= 4.5,
                    "{appearance} {accent}"
                );
                let output = ctx.run(egui::RawInput::default(), |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        toggle(ui, "History", true);
                    });
                });
                let actual = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.text() == "History" => {
                            Some(text.galley.job.sections[0].format.color)
                        }
                        _ => None,
                    })
                    .expect("production selected button must paint text");
                assert_eq!(actual, primary);
            }
        }
    }

    #[test]
    fn compact_buttons_keep_vertical_padding_and_do_not_change_sibling_styles() {
        for appearance in ["dark", "light"] {
            let ctx = egui::Context::default();
            super::super::theme::apply(&ctx, appearance, super::super::theme::DEFAULT_ACCENT_HEX);
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let padding = ui.spacing().button_padding;
                    let minimum = ui.spacing().interact_size;
                    ui.horizontal(|ui| {
                        for text in ["複製", "分支", "重試", "編輯提問", "＋ 新聊天"]
                        {
                            let galley = egui::WidgetText::from(text).into_galley(
                                ui,
                                Some(egui::TextWrapMode::Extend),
                                f32::INFINITY,
                                egui::TextStyle::Button,
                            );
                            let response = button(ui, text);
                            assert!((response.rect.height() - COMPACT_HEIGHT).abs() < 0.01);
                            assert!(response.rect.height() - galley.size().y >= 7.0);
                            assert!(response.rect.width() - galley.size().x >= 17.0);
                        }
                    });
                    assert_eq!(ui.spacing().button_padding, padding);
                    assert_eq!(ui.spacing().interact_size, minimum);
                });
            });
        }
    }

    #[test]
    fn real_padded_button_accepts_clicks_in_the_vertical_padding() {
        let ctx = egui::Context::default();
        super::super::theme::apply(&ctx, "dark", super::super::theme::DEFAULT_ACCENT_HEX);
        let mut rect = egui::Rect::NOTHING;
        let mut clicked = false;
        let mut draw = |events| {
            let _ = ctx.run(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let response = button(ui, "複製");
                        rect = response.rect;
                        clicked = response.clicked();
                    });
                },
            );
            (rect, clicked)
        };
        draw(vec![]);
        let (rect, _) = draw(vec![]);
        let pos = egui::pos2(rect.center().x, rect.top() + 2.0);
        draw(vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        let (_, clicked) = draw(vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        assert!(
            clicked,
            "padding is part of the actual click target, not decoration"
        );
    }
}
