//! Window preferences without persistence or native-window side effects.
//!
//! Apply placement only on startup / a new SHOW, not on every frame: otherwise
//! a default placement would fight the user's subsequent window dragging.
//! Opacity is a background multiplier; never apply it to the whole viewport or
//! a UI scope, since text and controls must retain their original opacity.

use super::preference_layout as form;
use eframe::egui::{self, Pos2, Rect};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowPreferences {
    pub opacity_percent: u8,
    pub placement: Placement,
    pub fixed_x: i32,
    pub fixed_y: i32,
}

impl Default for WindowPreferences {
    fn default() -> Self {
        Self {
            opacity_percent: 100,
            placement: Placement::Remember,
            fixed_x: 100,
            fixed_y: 100,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Placement {
    #[default]
    Remember,
    CursorScreenCenter,
    NearCursor,
    TopRight,
    BottomRight,
    Fixed,
}

impl Placement {
    const ALL: [Self; 6] = [
        Self::Remember,
        Self::CursorScreenCenter,
        Self::NearCursor,
        Self::TopRight,
        Self::BottomRight,
        Self::Fixed,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Remember => "記住上次位置",
            Self::CursorScreenCenter => "螢幕中央",
            Self::NearCursor => "游標附近",
            Self::TopRight => "螢幕右上角",
            Self::BottomRight => "螢幕右下角",
            Self::Fixed => "固定座標",
        }
    }
}

pub fn show_preferences(
    ui: &mut egui::Ui,
    prefs: &mut WindowPreferences,
    current_position: Option<Pos2>,
) {
    prefs.opacity_percent = prefs.opacity_percent.clamp(35, 100);
    form::heading(ui, "視窗");
    form::card(ui, |ui| {
        form::row(ui, "背景不透明度", |ui| {
            let slider_width = (ui.available_width() - 70.0).max(40.0);
            ui.scope(|ui| {
                ui.spacing_mut().slider_width = slider_width;
                ui.spacing_mut().interact_size.y = 20.0;
                ui.add(
                    egui::Slider::new(&mut prefs.opacity_percent, 35..=100)
                        .show_value(false)
                        .trailing_fill(true),
                )
                .on_hover_text("降低數值可透出背景，文字保持清晰。");
            });
            ui.add_sized(
                egui::vec2(60.0, 32.0),
                egui::DragValue::new(&mut prefs.opacity_percent)
                    .range(35..=100)
                    .suffix("%"),
            );
        });
        form::caption(ui, "僅調整背景，文字與按鈕不受影響。");
        form::divider(ui);
        form::row(ui, "預設跳出位置", |ui| {
            egui::ComboBox::from_id_salt("window-default-placement")
                .width(ui.available_width())
                .truncate()
                .selected_text(prefs.placement.label())
                .show_ui(ui, |ui| {
                    for placement in Placement::ALL {
                        ui.selectable_value(&mut prefs.placement, placement, placement.label());
                    }
                })
                .response
                .on_hover_text("螢幕位置以游標所在的螢幕為準。");
        });
        if prefs.placement == Placement::Fixed {
            ui.add_space(4.0);
            form::row(ui, "固定座標", |ui| {
                let width = ((ui.available_width() - 10.0) * 0.5).max(40.0);
                ui.add_sized(
                    egui::vec2(width, 32.0),
                    egui::DragValue::new(&mut prefs.fixed_x)
                        .prefix("X  ")
                        .speed(1.0),
                );
                ui.add_sized(
                    egui::vec2(width, 32.0),
                    egui::DragValue::new(&mut prefs.fixed_y)
                        .prefix("Y  ")
                        .speed(1.0),
                );
            });
            let position = usable_position(current_position);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(position.is_some(), egui::Button::new("使用目前位置"))
                        .clicked()
                    {
                        if let Some(position) = position {
                            prefs.fixed_x = position.x.round() as i32;
                            prefs.fixed_y = position.y.round() as i32;
                        }
                    }
                });
            });
        }
        ui.add_space(4.0);
        form::caption(ui, "下次叫出視窗時套用，仍可隨時拖曳移動。");
    });
}

/// Multiply a background's existing alpha by this value (do not replace it).
pub fn opacity(prefs: &WindowPreferences) -> f32 {
    f32::from(prefs.opacity_percent.clamp(35, 100)) / 100.0
}

/// Return a VIEWPORT rect in desktop logical points, including total padding.
/// Remember retains PanelGeometry's >=120×80 restore validation unchanged.
/// Other modes ignore remembered coordinates, but reuse its dimension clamps.
/// Fixed chooses the monitor nearest its requested origin, not the cursor, and
/// clamps even completely offscreen coordinates rather than rejecting a restore.
pub fn placement_rect(
    geometry: &super::panel::PanelGeometry,
    prefs: &WindowPreferences,
    has_messages: bool,
    compact_height: f32,
    total_padding: egui::Vec2,
    work_areas: &[Rect],
    cursor: Option<Pos2>,
) -> Option<Rect> {
    if prefs.placement == Placement::Remember {
        return geometry.placement(
            has_messages,
            compact_height,
            total_padding,
            work_areas,
            cursor,
        );
    }

    let fixed = egui::pos2(prefs.fixed_x as f32, prefs.fixed_y as f32);
    let anchor = if prefs.placement == Placement::Fixed {
        Some(fixed)
    } else {
        cursor.filter(|position| position.is_finite())
    };
    let host = nearest_work_area(work_areas, anchor)?;
    let mut dimensions = geometry.clone();
    dimensions.last_x = None;
    dimensions.last_y = None;
    let size = dimensions
        .placement(has_messages, compact_height, total_padding, &[host], None)?
        .size();

    let gap = 16.0;
    let candidate = match prefs.placement {
        Placement::Remember => unreachable!("handled by PanelGeometry above"),
        Placement::CursorScreenCenter => host.center() - size * 0.5,
        Placement::NearCursor => anchor.unwrap_or_else(|| host.center()) + egui::vec2(gap, gap),
        Placement::TopRight => egui::pos2(host.right() - size.x - gap, host.top() + gap),
        Placement::BottomRight => {
            egui::pos2(host.right() - size.x - gap, host.bottom() - size.y - gap)
        }
        Placement::Fixed => fixed,
    };
    let min = egui::pos2(
        candidate
            .x
            .clamp(host.left(), (host.right() - size.x).max(host.left())),
        candidate
            .y
            .clamp(host.top(), (host.bottom() - size.y).max(host.top())),
    );
    Some(Rect::from_min_size(min, size))
}

fn usable_position(position: Option<Pos2>) -> Option<Pos2> {
    position.filter(|pos| pos.is_finite() && *pos != egui::pos2(-32000.0, -32000.0))
}

fn nearest_work_area(areas: &[Rect], anchor: Option<Pos2>) -> Option<Rect> {
    let mut valid = areas.iter().copied().filter(|area| {
        area.is_finite() && area.size().is_finite() && area.width() > 0.0 && area.height() > 0.0
    });
    let first = valid.next()?;
    let Some(anchor) = anchor else {
        return Some(first);
    };
    std::iter::once(first).chain(valid).min_by(|a, b| {
        // f64 prevents squared distances from overflowing for extreme origins.
        let distance = |area: &Rect| {
            let x = f64::from(anchor.x);
            let y = f64::from(anchor.y);
            let dx = x - x.clamp(f64::from(area.left()), f64::from(area.right()));
            let dy = y - y.clamp(f64::from(area.top()), f64::from(area.bottom()));
            dx * dx + dy * dy
        };
        distance(a).total_cmp(&distance(b))
    })
}

#[cfg(test)]
mod tests {
    use super::super::panel::PanelGeometry;
    use super::*;
    use egui::{pos2, vec2, Vec2};

    fn area(x: f32, y: f32, width: f32, height: f32) -> Rect {
        Rect::from_min_size(pos2(x, y), vec2(width, height))
    }

    fn screens() -> [Rect; 2] {
        [
            area(0.0, 0.0, 1920.0, 1040.0),
            area(-2560.0, -1440.0, 2560.0, 1400.0),
        ]
    }

    fn prefs(placement: Placement) -> WindowPreferences {
        WindowPreferences {
            placement,
            fixed_x: -2200,
            fixed_y: -1200,
            ..Default::default()
        }
    }

    fn place(geometry: &PanelGeometry, prefs: &WindowPreferences, screens: &[Rect]) -> Rect {
        placement_rect(
            geometry,
            prefs,
            true,
            120.0,
            vec2(24.0, 24.0),
            screens,
            Some(pos2(-2000.0, -1000.0)),
        )
        .unwrap()
    }

    #[test]
    fn serde_defaults_support_missing_fields_and_round_trip_every_mode() {
        let defaults = WindowPreferences::default();
        assert_eq!(defaults.opacity_percent, 100);
        assert_eq!(defaults.placement, Placement::Remember);
        assert_eq!((defaults.fixed_x, defaults.fixed_y), (100, 100));
        assert_eq!(
            serde_json::from_str::<WindowPreferences>("{}").unwrap(),
            defaults
        );
        let partial: WindowPreferences = serde_json::from_str(r#"{"fixed_x":-400}"#).unwrap();
        assert_eq!(partial.fixed_x, -400);
        assert_eq!(partial.fixed_y, 100);
        assert_eq!(partial.opacity_percent, 100);
        assert_eq!(partial.placement, Placement::Remember);
        for mode in Placement::ALL {
            let original = prefs(mode);
            assert_eq!(
                serde_json::from_str::<WindowPreferences>(
                    &serde_json::to_string(&original).unwrap()
                )
                .unwrap(),
                original
            );
        }
    }

    #[test]
    fn opacity_is_clamped_without_touching_existing_alpha_or_preferences() {
        for percent in 0..=u8::MAX {
            let prefs = WindowPreferences {
                opacity_percent: percent,
                ..Default::default()
            };
            let multiplier = opacity(&prefs);
            assert_eq!(multiplier, f32::from(percent.clamp(35, 100)) / 100.0);
            assert_eq!(prefs.opacity_percent, percent);
            // The caller multiplies alpha, retaining its own translucent base.
            let base_alpha = 192.0;
            assert!(base_alpha * multiplier <= base_alpha);
        }
    }

    #[test]
    fn every_mode_supports_negative_monitor_origins_and_expected_anchors() {
        let screens = screens();
        let geometry = PanelGeometry::default();
        for mode in Placement::ALL {
            let rect = place(&geometry, &prefs(mode), &screens);
            assert!(screens[1].contains_rect(rect), "{mode:?}: {rect:?}");
            assert_eq!(rect.size(), vec2(604.0, 464.0));
            match mode {
                Placement::Remember => assert_eq!(rect.center(), pos2(-1280.0, -670.0)),
                Placement::CursorScreenCenter => assert_eq!(rect.center(), screens[1].center()),
                Placement::NearCursor => assert_eq!(rect.min, pos2(-1984.0, -984.0)),
                Placement::TopRight => assert_eq!(rect.min, pos2(-620.0, -1424.0)),
                Placement::BottomRight => assert_eq!(rect.min, pos2(-620.0, -520.0)),
                Placement::Fixed => assert_eq!(rect.min, pos2(-2200.0, -1200.0)),
            }
        }
    }

    #[test]
    fn remember_delegates_restore_validation_including_exact_threshold() {
        let screens = screens();
        for (x, y) in [
            (100.0, 100.0),
            (-484.0, -384.0),
            (-485.0, -384.0),
            (-9000.0, 0.0),
        ] {
            let geometry = PanelGeometry {
                last_x: Some(x),
                last_y: Some(y),
                ..Default::default()
            };
            assert_eq!(
                placement_rect(
                    &geometry,
                    &prefs(Placement::Remember),
                    true,
                    120.0,
                    vec2(24.0, 24.0),
                    &screens,
                    Some(pos2(800.0, 500.0))
                ),
                geometry.placement(
                    true,
                    120.0,
                    vec2(24.0, 24.0),
                    &screens,
                    Some(pos2(800.0, 500.0))
                )
            );
        }
    }

    #[test]
    fn explicit_modes_ignore_saved_position_but_do_not_mutate_geometry() {
        let screens = screens();
        let geometry = PanelGeometry {
            last_x: Some(100.0),
            last_y: Some(100.0),
            ..Default::default()
        };
        for mode in Placement::ALL
            .into_iter()
            .filter(|mode| *mode != Placement::Remember)
        {
            assert!(screens[1].contains_rect(place(&geometry, &prefs(mode), &screens)));
        }
        assert_eq!(geometry.last_x, Some(100.0));
        assert_eq!(geometry.last_y, Some(100.0));
        assert_eq!(geometry.width, 580.0);
    }

    #[test]
    fn all_modes_survive_unplugged_monitors_oversize_and_tiny_work_areas() {
        let geometry = PanelGeometry {
            width: 9000.0,
            expanded_height: 9000.0,
            last_x: Some(-2200.0),
            last_y: Some(-1200.0),
        };
        for host in [
            screens()[0],
            area(-400.0, -200.0, 400.0, 200.0),
            area(0.0, 0.0, 60.0, 40.0),
        ] {
            for mode in Placement::ALL {
                assert_eq!(place(&geometry, &prefs(mode), &[host]), host, "{mode:?}");
            }
        }
        assert_eq!(geometry.width, 9000.0);
        assert_eq!(geometry.expanded_height, 9000.0);
    }

    #[test]
    fn fixed_clamps_extreme_coordinates_instead_of_using_default_center() {
        let screens = screens();
        for (x, y, expected) in [
            (i32::MAX, i32::MAX, pos2(1316.0, 576.0)),
            (i32::MIN, i32::MIN, screens[1].min),
        ] {
            let prefs = WindowPreferences {
                fixed_x: x,
                fixed_y: y,
                ..prefs(Placement::Fixed)
            };
            assert_eq!(
                place(&PanelGeometry::default(), &prefs, &screens).min,
                expected
            );
        }
    }

    #[test]
    fn cursor_modes_use_containing_or_nearest_work_area_then_first_valid() {
        let screens = screens();
        for cursor in [Some(pos2(-1000.0, -20.0)), Some(pos2(-1000.0, -500.0))] {
            let rect = placement_rect(
                &PanelGeometry::default(),
                &prefs(Placement::CursorScreenCenter),
                true,
                120.0,
                Vec2::ZERO,
                &screens,
                cursor,
            )
            .unwrap();
            assert_eq!(rect.center(), screens[1].center());
        }
        for cursor in [None, Some(pos2(f32::NAN, f32::INFINITY))] {
            let rect = placement_rect(
                &PanelGeometry::default(),
                &prefs(Placement::CursorScreenCenter),
                true,
                120.0,
                Vec2::ZERO,
                &[Rect::NOTHING, screens[0], screens[1]],
                cursor,
            )
            .unwrap();
            assert_eq!(rect.center(), screens[0].center());
        }
        for mode in Placement::ALL {
            assert!(placement_rect(
                &PanelGeometry::default(),
                &prefs(mode),
                true,
                120.0,
                Vec2::ZERO,
                &[],
                None
            )
            .is_none());
            assert!(placement_rect(
                &PanelGeometry::default(),
                &prefs(mode),
                true,
                120.0,
                Vec2::ZERO,
                &[Rect::NOTHING],
                None
            )
            .is_none());
        }
    }

    #[test]
    fn near_cursor_clamps_whole_viewport_at_work_area_edges() {
        let host = screens()[1];
        for cursor in [host.min, host.max, host.center_top(), host.center_bottom()] {
            let rect = placement_rect(
                &PanelGeometry::default(),
                &prefs(Placement::NearCursor),
                true,
                120.0,
                vec2(24.0, 24.0),
                &[host],
                Some(cursor),
            )
            .unwrap();
            assert!(host.contains_rect(rect));
        }
    }

    #[test]
    fn explicit_modes_reuse_compact_and_invalid_dimension_clamps() {
        let host = screens()[0];
        let geometry = PanelGeometry {
            width: f32::NAN,
            expanded_height: f32::INFINITY,
            ..Default::default()
        };
        for mode in Placement::ALL {
            let rect = placement_rect(
                &geometry,
                &prefs(mode),
                false,
                178.0,
                vec2(24.0, 24.0),
                &[host],
                None,
            )
            .unwrap();
            assert_eq!(rect.size(), vec2(604.0, 202.0));
            let rect = placement_rect(
                &geometry,
                &prefs(mode),
                true,
                f32::NAN,
                vec2(-4.0, f32::NAN),
                &[host],
                None,
            )
            .unwrap();
            assert_eq!(rect.size(), vec2(580.0, 440.0));
        }
    }

    #[test]
    fn current_position_rejects_nonfinite_and_minimized_sentinel() {
        for position in [
            None,
            Some(pos2(f32::NAN, 0.0)),
            Some(pos2(0.0, f32::INFINITY)),
            Some(pos2(-32000.0, -32000.0)),
        ] {
            assert!(usable_position(position).is_none());
        }
        assert_eq!(
            usable_position(Some(pos2(-1400.5, -10.2))),
            Some(pos2(-1400.5, -10.2))
        );
    }

    fn ui_frame(
        ctx: &egui::Context,
        prefs: &mut WindowPreferences,
        position: Option<Pos2>,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(area(0.0, 0.0, 800.0, 600.0)),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    show_preferences(ui, prefs, position);
                });
            },
        )
    }

    fn text_position(output: &egui::FullOutput, text: &str) -> Pos2 {
        fn find(shape: &egui::epaint::Shape, text: &str) -> Option<Pos2> {
            match shape {
                egui::epaint::Shape::Text(shape) if shape.galley.job.text == text => {
                    Some(shape.pos + shape.galley.size() * 0.5)
                }
                egui::epaint::Shape::Vec(shapes) => {
                    shapes.iter().find_map(|shape| find(shape, text))
                }
                _ => None,
            }
        }
        output
            .shapes
            .iter()
            .find_map(|shape| find(&shape.shape, text))
            .unwrap_or_else(|| panic!("missing UI label: {text}"))
    }

    fn ui_click(
        ctx: &egui::Context,
        prefs: &mut WindowPreferences,
        position: Option<Pos2>,
        target: Pos2,
    ) -> egui::FullOutput {
        ui_frame(
            ctx,
            prefs,
            position,
            vec![egui::Event::PointerMoved(target)],
        );
        for pressed in [true, false] {
            ui_frame(
                ctx,
                prefs,
                position,
                vec![egui::Event::PointerButton {
                    pos: target,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        }
        ui_frame(ctx, prefs, position, vec![])
    }

    #[test]
    fn preferences_combo_really_offers_and_selects_all_six_modes() {
        for mode in Placement::ALL {
            let ctx = egui::Context::default();
            let mut prefs = WindowPreferences::default();
            let initial = ui_frame(&ctx, &mut prefs, None, vec![]);
            let combo = text_position(&initial, Placement::Remember.label());
            let opened = ui_click(&ctx, &mut prefs, None, combo);
            for offered in Placement::ALL {
                text_position(&opened, offered.label());
            }
            // Remember is also the selected text behind the menu; the other
            // five choices exercise actual selectable-value interaction.
            if mode != Placement::Remember {
                let target = text_position(&opened, mode.label());
                ui_click(&ctx, &mut prefs, None, target);
            }
            assert_eq!(prefs.placement, mode);
        }
    }

    #[test]
    fn preferences_ui_clamps_opacity_and_current_button_rounds_negative_coordinates() {
        let ctx = egui::Context::default();
        let mut prefs = WindowPreferences {
            opacity_percent: 255,
            ..prefs(Placement::Fixed)
        };
        let position = Some(pos2(-1400.6, -10.2));
        let output = ui_frame(&ctx, &mut prefs, position, vec![]);
        assert_eq!(prefs.opacity_percent, 100);
        let button = text_position(&output, "使用目前位置");
        let output = ui_click(&ctx, &mut prefs, position, button);
        assert_eq!((prefs.fixed_x, prefs.fixed_y), (-1401, -10));
        assert!(output
            .viewport_output
            .values()
            .all(|viewport| viewport.commands.is_empty()));
        prefs.opacity_percent = 0;
        ui_frame(&ctx, &mut prefs, None, vec![]);
        assert_eq!(prefs.opacity_percent, 35);
        ui_click(&ctx, &mut prefs, None, button);
        assert_eq!((prefs.fixed_x, prefs.fixed_y), (-1401, -10));
    }
}
