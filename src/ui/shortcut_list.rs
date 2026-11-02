//! Read-only, responsive reference for shortcuts implemented by main/composer.

use egui::{Align, Layout, RichText, Ui};

struct Shortcut {
    action: &'static str,
    bindings: &'static [&'static [&'static str]],
    note: &'static str,
}

const GROUPS: &[(&str, &[Shortcut])] = &[
    (
        "對話",
        &[
            Shortcut {
                action: "新聊天",
                bindings: &[&["Ctrl", "N"]],
                note: "",
            },
            Shortcut {
                action: "聊天歷史",
                bindings: &[&["Ctrl", "Y"]],
                note: "",
            },
        ],
    ),
    (
        "尋找",
        &[
            Shortcut {
                action: "尋找目前聊天",
                bindings: &[&["Ctrl", "F"]],
                note: "在歷史頁則聚焦搜尋欄",
            },
            Shortcut {
                action: "下一個結果",
                bindings: &[&["Ctrl", "G"]],
                note: "開啟聊天搜尋時",
            },
            Shortcut {
                action: "上一個結果",
                bindings: &[&["Ctrl", "Shift", "G"]],
                note: "開啟聊天搜尋時",
            },
        ],
    ),
    (
        "輸入與附件",
        &[
            Shortcut {
                action: "加入檔案",
                bindings: &[&["Ctrl", "O"]],
                note: "",
            },
            Shortcut {
                action: "貼上",
                bindings: &[&["Ctrl", "V"], &["Shift", "Insert"]],
                note: "文字、圖片或已複製的檔案",
            },
            Shortcut {
                action: "展開／收合輸入框",
                bindings: &[&["Ctrl", "E"]],
                note: "",
            },
            Shortcut {
                action: "傳送訊息",
                bindings: &[&["Enter"]],
                note: "一般輸入框；指令建議開啟時改為選取",
            },
            Shortcut {
                action: "換行",
                bindings: &[&["Shift", "Enter"]],
                note: "一般輸入框",
            },
            Shortcut {
                action: "大編輯器傳送",
                bindings: &[&["Ctrl", "Enter"]],
                note: "大編輯器內 Enter 為換行",
            },
        ],
    ),
    (
        "視窗與設定",
        &[
            Shortcut {
                action: "開啟／關閉設定",
                bindings: &[&["Ctrl", ","]],
                note: "",
            },
            Shortcut {
                action: "釘選／取消釘選",
                bindings: &[&["Ctrl", "P"]],
                note: "釘選後失焦不隱藏",
            },
            Shortcut {
                action: "關閉目前面板",
                bindings: &[&["Escape"]],
                note: "優先關閉歷史、搜尋、設定或編輯器",
            },
            Shortcut {
                action: "隱藏視窗",
                bindings: &[&["Ctrl", "W"]],
                note: "",
            },
        ],
    ),
];

/// No interactions or callbacks: all keycaps are labels, not buttons.
pub fn show(ui: &mut Ui) {
    super::preference_layout::caption(ui, "聊天視窗中的快捷操作；全域喚出快捷鍵可在上方另外設定。");
    ui.add_space(10.0);
    for (index, (heading, shortcuts)) in GROUPS.iter().enumerate() {
        if index > 0 {
            ui.add_space(14.0);
        }
        super::preference_layout::heading(ui, heading);
        super::preference_layout::card(ui, |ui| {
            for (index, shortcut) in shortcuts.iter().enumerate() {
                if index > 0 {
                    super::preference_layout::divider(ui);
                }
                row(ui, shortcut);
            }
        });
    }
}

fn row(ui: &mut Ui, shortcut: &Shortcut) {
    let width = ui.available_width();
    if width < 400.0 {
        action(ui, shortcut);
        ui.add_space(4.0);
        ui.with_layout(Layout::top_down(Align::RIGHT), |ui| bindings(ui, shortcut));
    } else {
        let gap = ui.spacing().item_spacing.x;
        let label_width = (width - gap) * 0.47;
        let keys_width = width - gap - label_width;
        ui.horizontal(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(label_width, 0.0),
                Layout::top_down(Align::LEFT),
                |ui| {
                    ui.set_min_width(label_width);
                    action(ui, shortcut);
                },
            );
            ui.allocate_ui_with_layout(
                egui::vec2(keys_width, 0.0),
                Layout::top_down(Align::RIGHT),
                |ui| {
                    ui.set_min_width(keys_width);
                    bindings(ui, shortcut);
                },
            );
        });
    }
}

fn action(ui: &mut Ui, shortcut: &Shortcut) {
    ui.add(egui::Label::new(RichText::new(shortcut.action).size(13.0).strong()).wrap());
    if !shortcut.note.is_empty() {
        ui.add(egui::Label::new(RichText::new(shortcut.note).size(11.0).weak()).wrap());
    }
}

fn bindings(ui: &mut Ui, shortcut: &Shortcut) {
    // Separate alternatives instead of cramming a slash into one long string.
    // Each combination fits in the narrow layout, while alternatives can stack.
    for (index, binding) in shortcut.bindings.iter().enumerate() {
        let text_width = |text: &str, font: egui::FontId| {
            ui.fonts(|fonts| {
                fonts
                    .layout_no_wrap(text.to_owned(), font, ui.visuals().text_color())
                    .size()
                    .x
            })
        };
        let mut width: f32 = binding
            .iter()
            .map(|key| text_width(key, egui::FontId::monospace(11.0)) + 16.0)
            .sum();
        width += binding.len().saturating_sub(1) as f32
            * (8.0 + text_width("+", egui::FontId::proportional(10.0)));
        if index > 0 {
            width += text_width("或", egui::FontId::proportional(11.0)) + 4.0;
        }
        // Explicit LTR prevents the enclosing right-aligned column from
        // reversing key order or making a Frame paint past its allocation.
        ui.allocate_ui_with_layout(
            egui::vec2(width, 26.0),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.set_min_width(width);
                ui.spacing_mut().item_spacing.x = 4.0;
                if index > 0 {
                    ui.label(RichText::new("或").size(11.0).weak());
                }
                for (index, key) in binding.iter().enumerate() {
                    if index > 0 {
                        ui.label(RichText::new("+").size(10.0).weak());
                    }
                    keycap(ui, key);
                }
            },
        );
    }
}

fn keycap(ui: &mut Ui, key: &str) {
    let dark = ui.visuals().dark_mode;
    egui::Frame::new()
        .fill(if dark {
            egui::Color32::from_rgb(44, 47, 56)
        } else {
            super::theme::LIGHT_CONTROL
        })
        .stroke(egui::Stroke::new(
            1.0_f32,
            if dark {
                egui::Color32::from_white_alpha(30)
            } else {
                egui::Color32::from_black_alpha(26)
            },
        ))
        .corner_radius(5)
        .inner_margin(egui::Margin::symmetric(7, 4))
        .show(ui, |ui| {
            ui.add(egui::Label::new(RichText::new(key).monospace().size(11.0)).selectable(false));
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actual_render_stays_inside_320_and_520_in_both_palettes() {
        for width in [320.0, 520.0] {
            for dark in [false, true] {
                let ctx = egui::Context::default();
                super::super::theme::apply(
                    &ctx,
                    if dark { "dark" } else { "light" },
                    super::super::theme::DEFAULT_ACCENT_HEX,
                );
                let mut bounds = egui::Rect::NOTHING;
                let output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 2400.0),
                        )),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            show(ui);
                            bounds = ui.min_rect();
                        });
                    },
                );
                assert!(bounds.right() <= width - 7.0, "{width}/{dark}: {bounds:?}");
                let mut labels = Vec::new();
                let mut caps = 0;
                for clipped in &output.shapes {
                    match &clipped.shape {
                        egui::Shape::Text(text) => {
                            let rect = egui::Rect::from_min_size(text.pos, text.galley.size());
                            assert!(
                                rect.left() >= 7.0 && rect.right() <= width - 7.0,
                                "clipped text {width}/{dark}: {} {rect:?}",
                                text.galley.text()
                            );
                            labels.push(text.galley.text().to_owned());
                        }
                        egui::Shape::Rect(rect)
                            if rect.corner_radius == egui::CornerRadius::same(5) =>
                        {
                            assert!(
                                rect.rect.left() >= 7.0 && rect.rect.right() <= width - 7.0,
                                "keycap overflow {width}/{dark}: {:?}",
                                rect.rect
                            );
                            assert_eq!(
                                rect.fill,
                                if dark {
                                    egui::Color32::from_rgb(44, 47, 56)
                                } else {
                                    super::super::theme::LIGHT_CONTROL
                                }
                            );
                            caps += 1;
                        }
                        _ => {}
                    }
                }
                for expected in [
                    "對話",
                    "尋找",
                    "輸入與附件",
                    "視窗與設定",
                    "新聊天",
                    "聊天歷史",
                    "上一個結果",
                    "下一個結果",
                    "貼上",
                    "Enter",
                    "Escape",
                    "Shift",
                    "Insert",
                    ",",
                    "或",
                ] {
                    assert!(
                        labels.iter().any(|label| label == expected),
                        "missing {expected}"
                    );
                }
                assert!(caps >= 30, "all combinations should have keycaps");
            }
        }
    }
}
