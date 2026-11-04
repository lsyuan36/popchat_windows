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
    ui.add_space(4.0);
    for (index, (heading, shortcuts)) in GROUPS.iter().enumerate() {
        if index > 0 {
            ui.add_space(4.0);
        }
        ui.label(RichText::new(*heading).size(13.0).strong());
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
            .corner_radius(10)
            .inner_margin(egui::Margin::symmetric(10, 4))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                // Read-only keycaps do not need the settings controls' 32pt hit target.
                ui.spacing_mut().interact_size.y = 18.0;
                ui.spacing_mut().item_spacing = egui::vec2(10.0, 1.0);
                let columns = if ui.available_width() >= 420.0 { 2 } else { 1 };
                for (index, shortcuts) in shortcuts.chunks(columns).enumerate() {
                    if index > 0 {
                        ui.add_space(3.0);
                    }
                    ui.columns(columns, |uis| {
                        for (ui, shortcut) in uis.iter_mut().zip(shortcuts) {
                            row(ui, shortcut);
                        }
                    });
                }
            });
    }
}

fn row(ui: &mut Ui, shortcut: &Shortcut) {
    let width = ui.available_width();
    let keys_width = shortcut
        .bindings
        .iter()
        .enumerate()
        .map(|(index, binding)| binding_width(ui, binding, index))
        .fold(0.0_f32, f32::max);
    let gap = ui.spacing().item_spacing.x;
    let label_width = width - gap - keys_width;
    if label_width < 72.0 {
        action(ui, shortcut);
        bindings(ui, shortcut);
    } else {
        ui.horizontal(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(label_width, 0.0),
                Layout::top_down(Align::LEFT),
                |ui| {
                    ui.set_width(label_width);
                    action(ui, shortcut);
                },
            );
            ui.allocate_ui_with_layout(
                egui::vec2(keys_width, 0.0),
                Layout::top_down(Align::RIGHT),
                |ui| {
                    ui.set_width(keys_width);
                    bindings(ui, shortcut);
                },
            );
        });
    }
    if !shortcut.note.is_empty() {
        ui.add(egui::Label::new(RichText::new(shortcut.note).size(11.0).weak()).wrap());
    }
}

fn action(ui: &mut Ui, shortcut: &Shortcut) {
    ui.add(egui::Label::new(RichText::new(shortcut.action).size(13.0).strong()).wrap());
}

fn binding_width(ui: &Ui, binding: &[&str], index: usize) -> f32 {
    let text_width = |text: &str, font: egui::FontId| {
        ui.fonts(|fonts| {
            fonts
                .layout_no_wrap(text.to_owned(), font, ui.visuals().text_color())
                .size()
                .x
        })
    };
    let mut width = binding
        .iter()
        .map(|key| text_width(key, egui::FontId::monospace(11.0)) + 10.0)
        .sum::<f32>();
    width += binding.len().saturating_sub(1) as f32
        * (8.0 + text_width("+", egui::FontId::proportional(10.0)));
    if index > 0 {
        width += text_width("或", egui::FontId::proportional(11.0)) + 4.0;
    }
    width
}

fn bindings(ui: &mut Ui, shortcut: &Shortcut) {
    // Separate alternatives instead of cramming a slash into one long string.
    // Each combination fits in the narrow layout, while alternatives can stack.
    for (index, binding) in shortcut.bindings.iter().enumerate() {
        let width = binding_width(ui, binding, index);
        // Explicit LTR prevents the enclosing right-aligned column from
        // reversing key order or making a Frame paint past its allocation.
        ui.allocate_ui_with_layout(
            egui::vec2(width, 20.0),
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
        .inner_margin(egui::Margin::symmetric(4, 2))
        .show(ui, |ui| {
            ui.add(egui::Label::new(RichText::new(key).monospace().size(11.0)).selectable(false));
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_reference_pairs_shortcuts_on_wide_surfaces_without_shrinking_controls() {
        for width in [320.0, 520.0, 982.0] {
            for dark in [false, true] {
                let ctx = egui::Context::default();
                super::super::theme::apply(
                    &ctx,
                    if dark { "dark" } else { "light" },
                    super::super::theme::DEFAULT_ACCENT_HEX,
                );
                super::super::mcp::preview::prepare_fonts(&ctx);
                let mut bounds = egui::Rect::NOTHING;
                let output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 1200.0),
                        )),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            ui.spacing_mut().interact_size.y = 32.0;
                            bounds = ui.scope(show).response.rect;
                            assert_eq!(
                                ui.spacing().interact_size.y,
                                32.0,
                                "read-only compact spacing must not shrink real controls"
                            );
                        });
                    },
                );
                let label = |expected: &str| {
                    output
                        .shapes
                        .iter()
                        .find_map(|shape| match &shape.shape {
                            egui::Shape::Text(text) if text.galley.job.text == expected => {
                                Some(egui::Rect::from_min_size(text.pos, text.galley.size()))
                            }
                            _ => None,
                        })
                        .unwrap_or_else(|| panic!("missing {expected}"))
                };
                let first = label("新聊天");
                let second = label("聊天歷史");
                super::super::mcp::preview::export(
                    &ctx,
                    std::slice::from_ref(&output),
                    egui::vec2(width, 1200.0),
                    &format!(
                        "shortcuts-compact-{}-{}.png",
                        if dark { "dark" } else { "light" },
                        width as u32
                    ),
                    dark,
                );
                if width >= 520.0 {
                    assert!((first.top() - second.top()).abs() < 0.5);
                    assert!(second.left() > first.right());
                } else {
                    assert!(second.top() > first.bottom());
                }
                assert!(
                    bounds.height() < if width >= 520.0 { 700.0 } else { 900.0 },
                    "shortcut reference is too tall: {width} {bounds:?}"
                );
                for (_, shortcuts) in GROUPS {
                    for shortcut in *shortcuts {
                        assert!(label(shortcut.action).right() <= width - 7.0);
                        if !shortcut.note.is_empty() {
                            assert!(label(shortcut.note).right() <= width - 7.0);
                        }
                    }
                }
            }
        }
    }

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
