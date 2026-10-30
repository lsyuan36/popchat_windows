//! PanelController geometry and assistant-message rendering.
//!
//! Geometry uses top-left desktop coordinates in egui logical points. Native
//! monitor work areas exclude the taskbar. Shadow padding is NOT panel content:
//! callers must add/subtract their canvas padding exactly once at the UI boundary.
//! Windows foreground activation is not AppKit's nonactivating-panel behavior;
//! see `visibility::activate_window` for the deliberately keyboard-safe substitute.

use std::collections::{HashMap, VecDeque};

use eframe::egui;
use serde::{Deserialize, Serialize};

/// 預設寬（對應 PanelController.defaultWidth）。
pub const DEFAULT_WIDTH: f32 = 580.0;
/// 展開態預設高（對應 defaultExpandedHeight）。
pub const DEFAULT_EXPANDED_HEIGHT: f32 = 440.0;
/// 展開態最小高（對應 expandedMinHeight）。
pub const EXPANDED_MIN_HEIGHT: f32 = 320.0;
/// 空聊天 compact 高（對應 compactMinHeight）。
pub const COMPACT_HEIGHT: f32 = 120.0;
/// 最小寬（對應 minWidth：header pill 與長 model 名不互撞）。
pub const MIN_WIDTH: f32 = 520.0;

/// 記住的使用者幾何（對應 panelWidth / panelExpandedHeight / savedTopLeft）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PanelGeometry {
    pub width: f32,
    pub expanded_height: f32,
    /// Last viewport top-left in desktop logical points; validated at every show.
    #[serde(default)]
    pub last_x: Option<f32>,
    #[serde(default)]
    pub last_y: Option<f32>,
}

impl Default for PanelGeometry {
    fn default() -> Self {
        Self {
            width: DEFAULT_WIDTH,
            expanded_height: DEFAULT_EXPANDED_HEIGHT,
            last_x: None,
            last_y: None,
        }
    }
}

impl PanelGeometry {
    fn config_path() -> Option<std::path::PathBuf> {
        crate::paths::config_dir().map(|dir| dir.join("panel.json"))
    }

    /// 讀 persistence config；無檔或損壞時回預設（對應 UserDefaults 讀不到走 default）。
    pub fn load() -> Self {
        let path = match Self::config_path() {
            Some(p) => p,
            None => return Self::default(),
        };
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(_) => return Self::default(),
        };
        let mut geometry = serde_json::from_slice::<Self>(&bytes).unwrap_or_default();
        geometry.width = preferred_dimension(geometry.width, MIN_WIDTH, DEFAULT_WIDTH);
        geometry.expanded_height = geometry.expanded_height_clamped();
        geometry
    }

    /// 寫回（hide / resize 結束時呼叫），保留原檔直到原子替換成功。
    pub fn save(&self) {
        let path = match Self::config_path() {
            Some(p) => p,
            None => return,
        };
        if let Some(parent) = path.parent() {
            if std::fs::create_dir_all(parent).is_err() {
                return;
            }
        }
        if let Ok(bytes) = serde_json::to_vec_pretty(self) {
            if let Err(error) = crate::paths::atomic_write(&path, &bytes) {
                crate::logf!("[geometry] could not save panel geometry: {error}");
            }
        }
    }

    /// 展開高度下限 clamp（舊配置殘留過小值時修正，對應 expandedHeight getter 的 max）。
    pub fn expanded_height_clamped(&self) -> f32 {
        preferred_dimension(
            self.expanded_height,
            EXPANDED_MIN_HEIGHT,
            DEFAULT_EXPANDED_HEIGHT,
        )
    }

    /// 目標內容高度：有訊息 → 記住的展開高；空 → compact（對應 emptinessChanged）。
    pub fn target_height(&self, has_messages: bool) -> f32 {
        if has_messages {
            self.expanded_height_clamped()
        } else {
            COMPACT_HEIGHT
        }
    }

    /// Source `position()`: accept a saved restore only with a >=120×80 work-area
    /// intersection, otherwise use the cursor's screen (primary is first).
    /// Like AppKit's additional frame constraint, clamp the ENTIRE window into
    /// that work area. Screen-fitting never changes the stored size preference.
    /// `canvas_padding` is TOTAL transparent padding, e.g. vec2(24,24) for 12/side.
    /// Returned rect is the VIEWPORT rect, suitable for OuterPosition + InnerSize
    /// on our undecorated window, not the inset shell rect.
    pub fn placement(
        &self,
        has_messages: bool,
        compact_height: f32,
        canvas_padding: egui::Vec2,
        work_areas: &[egui::Rect],
        cursor: Option<egui::Pos2>,
    ) -> Option<egui::Rect> {
        let padding = valid_padding(canvas_padding);
        let size = egui::vec2(
            preferred_dimension(self.width, MIN_WIDTH, DEFAULT_WIDTH),
            if has_messages {
                self.expanded_height_clamped()
            } else {
                preferred_dimension(compact_height, COMPACT_HEIGHT, COMPACT_HEIGHT)
            },
        ) + padding;
        let saved = self.last_x.zip(self.last_y).map(|(x, y)| egui::pos2(x, y));
        let restored_host = saved.filter(|pos| pos.is_finite()).and_then(|pos| {
            let frame = egui::Rect::from_min_size(pos, size);
            work_areas.iter().copied().find(|area| {
                let overlap = frame.intersect(*area);
                valid_work_area(*area) && overlap.width() >= 120.0 && overlap.height() >= 80.0
            })
        });
        let host = restored_host.or_else(|| cursor_work_area(work_areas, cursor))?;
        let size = size.min(host.size());
        let pos = if restored_host.is_some() {
            saved.unwrap()
        } else {
            // Swift's y-up visible.minY + height*.45 -> y-down center at 55%.
            egui::pos2(
                host.center().x - size.x / 2.0,
                host.top() + host.height() * 0.55 - size.y / 2.0,
            )
        };
        Some(egui::Rect::from_min_size(
            clamp_top_left(pos, size, host),
            size,
        ))
    }

    /// `setContentHeight`: content-driven compact/expanded growth is anchored to
    /// the viewport's top-left and never writes the saved expanded preference.
    /// Clamp the bottom at the work-area edge rather than moving the header.
    pub fn height_change_rect(
        &self,
        current_viewport: egui::Rect,
        has_messages: bool,
        compact_height: f32,
        canvas_padding: egui::Vec2,
        work_areas: &[egui::Rect],
    ) -> egui::Rect {
        let padding = valid_padding(canvas_padding);
        let height = if has_messages {
            self.expanded_height_clamped()
        } else {
            preferred_dimension(compact_height, COMPACT_HEIGHT, COMPACT_HEIGHT)
        } + padding.y;
        let host = work_areas
            .iter()
            .copied()
            .find(|area| valid_work_area(*area) && area.contains(current_viewport.min));
        let height = host.map_or(height, |area| {
            height.min(area.bottom() - current_viewport.top())
        });
        egui::Rect::from_min_size(
            current_viewport.min,
            egui::vec2(current_viewport.width(), height),
        )
    }

    /// Source `windowWillResize`: compact height is content-owned; only expanded
    /// height is user-resizable. Both arguments and result are CONTENT sizes.
    pub fn constrain_user_resize(
        &self,
        proposed_content: egui::Vec2,
        has_messages: bool,
        compact_height: f32,
        available_content: egui::Vec2,
    ) -> egui::Vec2 {
        let width = preferred_dimension(proposed_content.x, MIN_WIDTH, DEFAULT_WIDTH);
        let height = if has_messages {
            preferred_dimension(
                proposed_content.y,
                EXPANDED_MIN_HEIGHT,
                DEFAULT_EXPANDED_HEIGHT,
            )
        } else {
            preferred_dimension(compact_height, COMPACT_HEIGHT, COMPACT_HEIGHT)
        };
        egui::vec2(width, height).min(egui::vec2(
            preferred_dimension(available_content.x, 1.0, DEFAULT_WIDTH),
            preferred_dimension(available_content.y, 1.0, DEFAULT_EXPANDED_HEIGHT),
        ))
    }

    /// Call ONLY for a completed USER resize, never for automatic height changes
    /// or monitor fitting. Use viewport INNER size minus TOTAL canvas padding;
    /// outer/native shadow extents must never be fed back into the saved width.
    pub fn record_user_resize(
        &mut self,
        viewport_inner_size: egui::Vec2,
        canvas_padding: egui::Vec2,
        has_messages: bool,
    ) {
        let content = viewport_inner_size - valid_padding(canvas_padding);
        if !content.is_finite() || content.x <= 0.0 || content.y <= 0.0 {
            return;
        }
        self.width = content.x.max(MIN_WIDTH);
        if has_messages {
            self.expanded_height = content.y.max(EXPANDED_MIN_HEIGHT);
        }
    }

    /// Persist actual viewport origin, not the shell origin inset by its shadow.
    pub fn remember_top_left(&mut self, pos: egui::Pos2) {
        if pos.is_finite() && pos != egui::pos2(-32000.0, -32000.0) {
            self.last_x = Some(pos.x);
            self.last_y = Some(pos.y);
        }
    }
}

fn preferred_dimension(value: f32, minimum: f32, default: f32) -> f32 {
    if value.is_finite() && value > 0.0 {
        value.max(minimum)
    } else {
        default
    }
}

fn valid_padding(padding: egui::Vec2) -> egui::Vec2 {
    egui::vec2(
        if padding.x.is_finite() {
            padding.x.max(0.0)
        } else {
            0.0
        },
        if padding.y.is_finite() {
            padding.y.max(0.0)
        } else {
            0.0
        },
    )
}

fn valid_work_area(area: egui::Rect) -> bool {
    area.is_finite() && area.width() > 0.0 && area.height() > 0.0
}

fn clamp_top_left(pos: egui::Pos2, size: egui::Vec2, area: egui::Rect) -> egui::Pos2 {
    egui::pos2(
        pos.x
            .clamp(area.left(), (area.right() - size.x).max(area.left())),
        pos.y
            .clamp(area.top(), (area.bottom() - size.y).max(area.top())),
    )
}

fn cursor_work_area(areas: &[egui::Rect], cursor: Option<egui::Pos2>) -> Option<egui::Rect> {
    let mut valid = areas.iter().copied().filter(|area| valid_work_area(*area));
    let first = valid.next()?;
    let Some(cursor) = cursor.filter(|pos| pos.is_finite()) else {
        return Some(first);
    };
    // A cursor over the taskbar is outside rcWork: nearest work area still picks
    // the correct display instead of spuriously falling back to the primary.
    std::iter::once(first).chain(valid).min_by(|a, b| {
        a.distance_sq_to_pos(cursor)
            .total_cmp(&b.distance_sq_to_pos(cursor))
    })
}

/// Enumerate Windows rcWork (not rcMonitor), primary first, in egui desktop
/// logical coordinates. Divide ALL physical coordinates by the CURRENT viewport's
/// `ctx.pixels_per_point()`; per-monitor division creates overlaps/gaps on mixed
/// DPI desktops. Re-enumerate after a DPI/display change. No synthetic fallback
/// screen is returned when enumeration fails.
pub fn available_monitor_rects(pixels_per_point: f32) -> Vec<egui::Rect> {
    native_monitors::work_areas(pixels_per_point)
}

/// Same coordinate conversion as `available_monitor_rects`, for default placement.
pub fn desktop_cursor_position(pixels_per_point: f32) -> Option<egui::Pos2> {
    native_monitors::cursor(pixels_per_point)
}

#[cfg(windows)]
mod native_monitors {
    use super::egui;
    use std::ffi::c_void;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct NativeRect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }
    #[repr(C)]
    struct MonitorInfo {
        size: u32,
        monitor: NativeRect,
        work: NativeRect,
        flags: u32,
    }
    #[repr(C)]
    struct Point {
        x: i32,
        y: i32,
    }
    #[link(name = "user32")]
    extern "system" {
        fn EnumDisplayMonitors(
            hdc: *mut c_void,
            clip: *const NativeRect,
            callback: unsafe extern "system" fn(
                *mut c_void,
                *mut c_void,
                *mut NativeRect,
                isize,
            ) -> i32,
            data: isize,
        ) -> i32;
        fn GetMonitorInfoW(monitor: *mut c_void, info: *mut MonitorInfo) -> i32;
        fn GetCursorPos(point: *mut Point) -> i32;
    }

    unsafe extern "system" fn collect(
        monitor: *mut c_void,
        _: *mut c_void,
        _: *mut NativeRect,
        data: isize,
    ) -> i32 {
        let zero = NativeRect {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        let mut info = MonitorInfo {
            size: std::mem::size_of::<MonitorInfo>() as u32,
            monitor: zero,
            work: zero,
            flags: 0,
        };
        // SAFETY: cbSize matches MONITORINFO; EnumDisplayMonitors supplies a live
        // HMONITOR and synchronously retains the Vec pointer for this call only.
        if unsafe { GetMonitorInfoW(monitor, &mut info) } != 0 {
            let output = unsafe { &mut *(data as *mut Vec<(bool, NativeRect)>) };
            output.push((info.flags & 1 != 0, info.work));
        }
        1
    }

    pub fn work_areas(scale: f32) -> Vec<egui::Rect> {
        if !scale.is_finite() || scale <= 0.0 {
            return Vec::new();
        }
        let mut areas: Vec<(bool, NativeRect)> = Vec::new();
        // SAFETY: callback is synchronous; its Vec remains alive and uniquely
        // borrowed until enumeration returns. Null DC/clip enumerate all monitors.
        if unsafe {
            EnumDisplayMonitors(
                std::ptr::null_mut(),
                std::ptr::null(),
                collect,
                &mut areas as *mut _ as isize,
            )
        } == 0
        {
            return Vec::new();
        }
        areas.sort_by_key(|(primary, _)| !primary);
        areas
            .into_iter()
            .map(|(_, rect)| {
                egui::Rect::from_min_max(
                    egui::pos2(rect.left as f32 / scale, rect.top as f32 / scale),
                    egui::pos2(rect.right as f32 / scale, rect.bottom as f32 / scale),
                )
            })
            .filter(|area| super::valid_work_area(*area))
            .collect()
    }

    pub fn cursor(scale: f32) -> Option<egui::Pos2> {
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        let mut point = Point { x: 0, y: 0 };
        // SAFETY: pointer addresses an initialized, writable Win32 POINT.
        (unsafe { GetCursorPos(&mut point) } != 0)
            .then(|| egui::pos2(point.x as f32 / scale, point.y as f32 / scale))
    }
}

#[cfg(not(windows))]
mod native_monitors {
    use super::egui;
    pub fn work_areas(_: f32) -> Vec<egui::Rect> {
        Vec::new()
    }
    pub fn cursor(_: f32) -> Option<egui::Pos2> {
        None
    }
}

/// Borderless native resize hit targets, confined to the shell's 4-point rim.
/// Call before the header drag strip; skip header dragging when this returns true.
/// Shell rect is UI-local (without its transparent shadow). No whole-window drag
/// overlay: composer, text selection, scroll areas and header icons stay untouched.
/// Compact mode exposes horizontal resizing only; also enforce native viewport
/// min/max heights to the current content-driven compact height in the caller.
pub fn resize_edges(ui: &mut egui::Ui, shell_rect: egui::Rect, has_messages: bool) -> bool {
    use egui::{CursorIcon, ResizeDirection as D};
    let edge = 4.0;
    let corner = 9.0;
    let l = shell_rect.left();
    let r = shell_rect.right();
    let t = shell_rect.top();
    let b = shell_rect.bottom();
    let rect = |x1, y1, x2, y2| egui::Rect::from_min_max(egui::pos2(x1, y1), egui::pos2(x2, y2));
    let targets = if has_messages {
        vec![
            (rect(l, t, l + corner, t + edge), D::NorthWest),
            (rect(l, t + edge, l + edge, t + corner), D::NorthWest),
            (rect(r - corner, t, r, t + edge), D::NorthEast),
            (rect(r - edge, t + edge, r, t + corner), D::NorthEast),
            (rect(l, b - edge, l + corner, b), D::SouthWest),
            (rect(l, b - corner, l + edge, b - edge), D::SouthWest),
            (rect(r - corner, b - edge, r, b), D::SouthEast),
            (rect(r - edge, b - corner, r, b - edge), D::SouthEast),
            (rect(l + corner, t, r - corner, t + edge), D::North),
            (rect(l + corner, b - edge, r - corner, b), D::South),
            (rect(l, t + corner, l + edge, b - corner), D::West),
            (rect(r - edge, t + corner, r, b - corner), D::East),
        ]
    } else {
        vec![
            (rect(l, t, l + edge, b), D::West),
            (rect(r - edge, t, r, b), D::East),
        ]
    };
    let mut active = false;
    for (index, (rect, direction)) in targets.into_iter().enumerate() {
        let response = ui.interact(
            rect,
            ui.id().with(("native-resize", index)),
            egui::Sense::drag(),
        );
        if response.hovered() || response.dragged() {
            active = true;
            ui.ctx().set_cursor_icon(match direction {
                D::North | D::South => CursorIcon::ResizeVertical,
                D::East | D::West => CursorIcon::ResizeHorizontal,
                D::NorthWest | D::SouthEast => CursorIcon::ResizeNwSe,
                D::NorthEast | D::SouthWest => CursorIcon::ResizeNeSw,
            });
            if response.is_pointer_button_down_on()
                && ui.input(|input| input.pointer.button_pressed(egui::PointerButton::Primary))
            {
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::BeginResize(direction));
            }
        }
    }
    active
}

/// 失焦自動隱藏判斷（對應 `windowDidResignKey` + 非 pinned 分支）。
/// 呼叫方需自行做「至少聚焦過一次才開始判斷」的門檻（見 main.rs `was_focused`），
/// 否則 show 後焦點尚未到達的第一幀會立刻再隱藏。
/// 實際 pin / modal / generation 判斷由 VisibilityController 的 caller 處理。
pub fn lost_focus(ctx: &eframe::egui::Context) -> bool {
    ctx.input(|i| i.viewport().focused) == Some(false)
}

#[cfg(test)]
mod geometry_tests {
    use super::*;
    use egui::{pos2, vec2, Rect, Vec2};

    fn area(x: f32, y: f32, width: f32, height: f32) -> Rect {
        Rect::from_min_size(pos2(x, y), vec2(width, height))
    }

    fn place(geometry: &PanelGeometry, screens: &[Rect], cursor: egui::Pos2) -> Rect {
        geometry
            .placement(true, COMPACT_HEIGHT, Vec2::ZERO, screens, Some(cursor))
            .unwrap()
    }

    #[test]
    fn source_defaults_and_low_end_validation_match_panel_controller() {
        let geometry = PanelGeometry::default();
        assert_eq!(geometry.width, 580.0);
        assert_eq!(geometry.expanded_height_clamped(), 440.0);
        let screen = area(0.0, 0.0, 1920.0, 1040.0);
        let mut stored = PanelGeometry {
            width: 1.0,
            expanded_height: 1.0,
            ..geometry
        };
        assert_eq!(
            place(&stored, &[screen], pos2(0.0, 0.0)).size(),
            vec2(520.0, 320.0)
        );
        stored.width = 0.0;
        stored.expanded_height = -10.0;
        assert_eq!(
            place(&stored, &[screen], pos2(0.0, 0.0)).size(),
            vec2(580.0, 440.0)
        );
    }

    #[test]
    fn default_is_below_center_on_cursor_monitor_including_negative_coordinates() {
        let screens = [
            area(0.0, 0.0, 1920.0, 1040.0),
            area(-2560.0, -1440.0, 2560.0, 1400.0),
        ];
        let rect = place(&PanelGeometry::default(), &screens, pos2(-1300.0, -500.0));
        assert_eq!(rect.center().x, -1280.0);
        assert_eq!(rect.center().y, -1440.0 + 1400.0 * 0.55);
        assert!(screens[1].contains_rect(rect));
    }

    #[test]
    fn saved_top_left_wins_over_cursor_and_supports_far_negative_monitors() {
        let screens = [
            area(0.0, 0.0, 1920.0, 1040.0),
            area(-15000.0, -2000.0, 2000.0, 1400.0),
        ];
        let geometry = PanelGeometry {
            last_x: Some(-14500.0),
            last_y: Some(-1800.0),
            ..Default::default()
        };
        assert_eq!(
            place(&geometry, &screens, pos2(900.0, 500.0)).min,
            pos2(-14500.0, -1800.0)
        );
    }

    #[test]
    fn source_intersection_threshold_is_inclusive_and_tiny_slivers_are_rejected() {
        let screens = [
            area(0.0, 0.0, 1920.0, 1040.0),
            area(1920.0, 0.0, 1920.0, 1040.0),
        ];
        let mut geometry = PanelGeometry {
            last_x: Some(-460.0),
            last_y: Some(-360.0),
            ..Default::default()
        };
        // Exactly 120×80 qualifies; borderless Windows then needs the explicit
        // full-frame clamp supplied automatically by the original titled panel.
        assert_eq!(
            place(&geometry, &screens, pos2(2500.0, 200.0)).min,
            pos2(0.0, 0.0)
        );
        geometry.last_x = Some(-461.0);
        assert!(screens[1].contains_rect(place(&geometry, &screens, pos2(2500.0, 200.0))));
        geometry.last_x = Some(-460.0);
        geometry.last_y = Some(-361.0);
        assert!(screens[1].contains_rect(place(&geometry, &screens, pos2(2500.0, 200.0))));
    }

    #[test]
    fn unplugged_screen_and_taskbar_only_restore_fall_back_to_available_screen() {
        let screens = [area(0.0, 0.0, 1920.0, 1040.0)];
        let mut geometry = PanelGeometry {
            last_x: Some(-1800.0),
            last_y: Some(100.0),
            ..Default::default()
        };
        assert!(screens[0].contains_rect(place(&geometry, &screens, pos2(900.0, 500.0))));
        geometry.last_x = Some(300.0);
        geometry.last_y = Some(1050.0); // rcMonitor includes this; rcWork does not.
        let rect = place(&geometry, &screens, pos2(900.0, 1070.0));
        assert!(screens[0].contains_rect(rect));
        assert_eq!(rect.center().y, 572.0);
    }

    #[test]
    fn cursor_on_secondary_taskbar_still_selects_secondary_screen() {
        let screens = [
            area(0.0, 0.0, 1920.0, 1040.0),
            area(1920.0, 0.0, 1920.0, 1040.0),
        ];
        let rect = place(&PanelGeometry::default(), &screens, pos2(2900.0, 1075.0));
        assert!(screens[1].contains_rect(rect));
    }

    #[test]
    fn oversized_and_nonfinite_persisted_dimensions_are_safe_without_losing_preferences() {
        let screen = area(-1920.0, 0.0, 1920.0, 1040.0);
        let mut geometry = PanelGeometry {
            width: 9000.0,
            expanded_height: 9000.0,
            last_x: Some(-1800.0),
            last_y: Some(100.0),
        };
        assert_eq!(place(&geometry, &[screen], pos2(-900.0, 500.0)), screen);
        assert_eq!(geometry.width, 9000.0);
        assert_eq!(geometry.expanded_height, 9000.0);
        geometry.width = f32::NAN;
        geometry.expanded_height = f32::INFINITY;
        geometry.last_x = Some(f32::NAN);
        geometry.last_y = Some(f32::NEG_INFINITY);
        let rect = place(&geometry, &[screen], pos2(-900.0, 500.0));
        assert_eq!(rect.size(), vec2(580.0, 440.0));
        assert!(screen.contains_rect(rect));
    }

    #[test]
    fn entirely_clamps_partial_restore_and_even_screens_smaller_than_minimum() {
        let mut geometry = PanelGeometry {
            last_x: Some(1700.0),
            last_y: Some(900.0),
            ..Default::default()
        };
        let screen = area(0.0, 0.0, 1920.0, 1040.0);
        let rect = place(&geometry, &[screen], pos2(0.0, 0.0));
        assert_eq!(rect.min, pos2(1340.0, 600.0));
        geometry.width = 9000.0;
        let tiny = area(-400.0, -200.0, 400.0, 200.0);
        assert_eq!(place(&geometry, &[tiny], pos2(-100.0, -100.0)), tiny);
    }

    #[test]
    fn top_anchored_height_changes_preserve_width_and_expanded_preference() {
        let geometry = PanelGeometry {
            expanded_height: 700.0,
            ..Default::default()
        };
        let screen = area(0.0, 0.0, 1920.0, 1040.0);
        let padding = vec2(24.0, 24.0);
        let current = area(300.0, 100.0, 624.0, 144.0);
        let expanded = geometry.height_change_rect(current, true, 120.0, padding, &[screen]);
        assert_eq!(expanded, area(300.0, 100.0, 624.0, 724.0));
        let compact = geometry.height_change_rect(expanded, false, 178.0, padding, &[screen]);
        assert_eq!(compact, area(300.0, 100.0, 624.0, 202.0));
        assert_eq!(geometry.expanded_height, 700.0);
        let low = area(300.0, 800.0, 624.0, 144.0);
        let expanded_low = geometry.height_change_rect(low, true, 120.0, padding, &[screen]);
        assert_eq!(expanded_low.min, low.min);
        assert_eq!(expanded_low.bottom(), screen.bottom());
    }

    #[test]
    fn compact_user_resize_locks_content_height_and_preserves_saved_expanded_height() {
        let mut geometry = PanelGeometry {
            expanded_height: 650.0,
            ..Default::default()
        };
        let size =
            geometry.constrain_user_resize(vec2(100.0, 900.0), false, 170.0, vec2(1920.0, 1040.0));
        assert_eq!(size, vec2(520.0, 170.0));
        geometry.record_user_resize(vec2(744.0, 194.0), vec2(24.0, 24.0), false);
        assert_eq!(geometry.width, 720.0);
        assert_eq!(geometry.expanded_height, 650.0);
        assert_eq!(
            geometry.constrain_user_resize(vec2(20.0, 10.0), true, 120.0, vec2(1920.0, 1040.0)),
            vec2(520.0, 320.0)
        );
    }

    #[test]
    fn repeated_resize_persistence_never_grows_width_from_shadow_margins() {
        let mut geometry = PanelGeometry::default();
        let screen = area(0.0, 0.0, 1920.0, 1040.0);
        let padding = vec2(24.0, 24.0);
        for _ in 0..10 {
            let rect = geometry
                .placement(true, 120.0, padding, &[screen], None)
                .unwrap();
            geometry.record_user_resize(rect.size(), padding, true);
            assert_eq!(geometry.width, 580.0);
            assert_eq!(geometry.expanded_height, 440.0);
        }
        geometry.record_user_resize(vec2(f32::NAN, 400.0), padding, true);
        assert_eq!(geometry.width, 580.0);
    }

    #[test]
    fn missing_or_invalid_monitor_information_never_invents_an_offscreen_restore() {
        let geometry = PanelGeometry::default();
        assert!(geometry
            .placement(true, 120.0, Vec2::ZERO, &[], None)
            .is_none());
        assert!(geometry
            .placement(true, 120.0, Vec2::ZERO, &[Rect::NOTHING], None)
            .is_none());
    }

    #[test]
    fn resize_rim_starts_native_resize_but_does_not_capture_composer_or_header_icons() {
        fn click_at(pos: egui::Pos2, expanded: bool) -> Vec<egui::ViewportCommand> {
            let ctx = egui::Context::default();
            let shell = area(12.0, 12.0, 580.0, 440.0);
            let frame = |events: Vec<egui::Event>| {
                ctx.run(
                    egui::RawInput {
                        screen_rect: Some(area(0.0, 0.0, 604.0, 464.0)),
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default()
                            .frame(egui::Frame::NONE)
                            .show(ctx, |ui| {
                                resize_edges(ui, shell, expanded);
                            });
                    },
                )
            };
            frame(vec![egui::Event::PointerMoved(pos)]);
            let output = frame(vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }]);
            output.viewport_output[&egui::ViewportId::ROOT]
                .commands
                .clone()
        }
        assert!(click_at(pos2(13.0, 220.0), true).iter().any(|command| {
            matches!(
                command,
                egui::ViewportCommand::BeginResize(egui::ResizeDirection::West)
            )
        }));
        assert!(click_at(pos2(300.0, 13.0), true).iter().any(|command| {
            matches!(
                command,
                egui::ViewportCommand::BeginResize(egui::ResizeDirection::North)
            )
        }));
        for (pos, expanded) in [
            (pos2(300.0, 13.0), false), // compact: no vertical drag
            (pos2(300.0, 410.0), true), // composer interior
            (pos2(40.0, 35.0), true),   // header icon interior
            (pos2(300.0, 220.0), true), // transcript / scroll interior
        ] {
            assert!(!click_at(pos, expanded)
                .iter()
                .any(|command| { matches!(command, egui::ViewportCommand::BeginResize(_)) }));
        }
    }
}

// MARK: - Assistant 訊息內的 LaTeX（對應原版 MarkdownRenderer 數學分支）
//
// 原版切分：`segments()` 先剝 ` ``` ` fence / `$$` 整行展示數學 / quote / table，
// 剩下 prose 再用 `extractInlineMath`（正則 `\$([^\s$](?:[^$\n]*[^\s$])?)\$` +
// 「含 `\^_{}=` 或完全無空白才算數學」）換成行內圖；行內字級 `NSFont.systemFontSize`，
// 展示字級 `17`，失敗一律退回等寬原文（行內 `$latex$`、展示 `$$latex$$`）。
// 這裡鏡像同一語義：
// - [`split_math_segments`]：依原版順序切 fence / pasteable / math / quote / table / prose。
// - [`split_inline_math`]：行內切分（同正則 + 同 mathy 判斷，fence 外才呼叫）。
// - [`MathTextures::show_assistant_message`]：行內公式縮小嵌入同行、展示公式置中，
//   任何 `render_latex` 失敗退回等寬原文，不擋主流程。
// - prose、引用、表格與可複製卡片由 markdown 層排版。

/// 行內公式字級：對應原版 `mathImage(…fontSize: NSFont.systemFontSize…)`。
pub const INLINE_MATH_FONT_SIZE: f32 = 13.0;
/// 展示公式字級：對應原版 `MathBlockView` 的 `fontSize: 17`。
pub const DISPLAY_MATH_FONT_SIZE: f32 = 17.0;

/// 塊級片段。`Prose` 保留原文 `$..$`（顯示時才做行內切分，以保持同行混排）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageSegment {
    Prose(String),
    DisplayMath(String),
    Quote(String),
    Table {
        header: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    Pasteable {
        title: Option<String>,
        content: String,
    },
    Code {
        language: Option<String>,
        content: String,
    },
}

/// 行內片段（借用原文；呼叫方轉成擁有字串）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InlinePart<'a> {
    Text(&'a str),
    Math(&'a str),
}

/// Direct port of MarkdownRenderer.segments(_:), including its permissive tag /
/// table recognition and consuming unclosed blocks through EOF. Do not pre-split
/// fences or math: a pasteable owns its body verbatim, even when it contains both.
pub fn split_math_segments(text: &str) -> Vec<MessageSegment> {
    let mut segments = Vec::new();
    let mut prose = Vec::new();
    let flush_prose = |prose: &mut Vec<&str>, segments: &mut Vec<MessageSegment>| {
        let joined = prose.join("\n");
        let trimmed = joined.trim();
        if !trimmed.is_empty() {
            segments.push(MessageSegment::Prose(trimmed.to_string()));
        }
        prose.clear();
    };

    // Swift components(separatedBy: "\n") preserves a trailing empty line.
    let lines: Vec<&str> = text.split('\n').collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = trim_horizontal(line);
        if trimmed.starts_with("```") {
            flush_prose(&mut prose, &mut segments);
            let hint = trim_horizontal(&trimmed[3..]);
            let mut code = Vec::new();
            i += 1;
            while i < lines.len() && !trim_horizontal(lines[i]).starts_with("```") {
                code.push(lines[i]);
                i += 1;
            }
            segments.push(MessageSegment::Code {
                language: (!hint.is_empty()).then(|| hint.to_owned()),
                content: code.join("\n"),
            });
            i += 1;
            continue;
        }
        if trimmed.starts_with("<pasteable") && trimmed.ends_with('>') {
            flush_prose(&mut prose, &mut segments);
            let title = pasteable_title(trimmed);
            let mut content = Vec::new();
            i += 1;
            while i < lines.len() && trim_horizontal(lines[i]) != "</pasteable>" {
                content.push(lines[i]);
                i += 1;
            }
            segments.push(MessageSegment::Pasteable {
                title,
                content: content.join("\n").trim().to_owned(),
            });
            i += 1;
            continue;
        }
        if trimmed == "$$"
            || (trimmed.starts_with("$$") && trimmed.ends_with("$$") && trimmed.len() > 4)
        {
            flush_prose(&mut prose, &mut segments);
            if trimmed.len() > 4 {
                segments.push(MessageSegment::DisplayMath(
                    trimmed[2..trimmed.len() - 2].to_string(),
                ));
                i += 1;
            } else {
                let mut math = Vec::new();
                i += 1;
                while i < lines.len() && trim_horizontal(lines[i]) != "$$" {
                    math.push(lines[i]);
                    i += 1;
                }
                segments.push(MessageSegment::DisplayMath(math.join("\n")));
                i += 1; // 跳過結尾 `$$`（若已到文末則自然結束）
            }
            continue;
        }
        if trimmed.starts_with('>') {
            flush_prose(&mut prose, &mut segments);
            let mut quote = Vec::new();
            while i < lines.len() {
                let quote_line = trim_horizontal(lines[i]);
                let Some(rest) = quote_line.strip_prefix('>') else {
                    break;
                };
                quote.push(rest.trim_start_matches(' '));
                i += 1;
            }
            segments.push(MessageSegment::Quote(quote.join("\n")));
            continue;
        }
        if trimmed.starts_with('|') && i + 1 < lines.len() && is_table_separator(lines[i + 1]) {
            flush_prose(&mut prose, &mut segments);
            let header = table_cells(trimmed);
            let mut rows = Vec::new();
            i += 2;
            while i < lines.len() && trim_horizontal(lines[i]).starts_with('|') {
                rows.push(table_cells(trim_horizontal(lines[i])));
                i += 1;
            }
            segments.push(MessageSegment::Table { header, rows });
            continue;
        }
        prose.push(line);
        i += 1;
    }
    flush_prose(&mut prose, &mut segments);
    segments
}

// Foundation .whitespaces, not .whitespacesAndNewlines (not str::lines()).
fn trim_horizontal(text: &str) -> &str {
    text.trim_matches(|c: char| {
        c.is_whitespace()
            && !matches!(
                c,
                '\n' | '\r' | '\u{b}' | '\u{c}' | '\u{85}' | '\u{2028}' | '\u{2029}'
            )
    })
}

fn is_table_separator(line: &str) -> bool {
    let line = trim_horizontal(line);
    line.starts_with('|') && line.contains('-') && line.chars().all(|c| "|-: ".contains(c))
}

fn table_cells(line: &str) -> Vec<String> {
    let mut cells: Vec<_> = line
        .split('|')
        .map(|cell| trim_horizontal(cell).to_owned())
        .collect();
    if cells.first().is_some_and(String::is_empty) {
        cells.remove(0);
    }
    if cells.last().is_some_and(String::is_empty) {
        cells.pop();
    }
    cells
}

/// Equivalent to Swift's first regex match: title\s*=\s*"([^"]*)".
/// In particular single quotes and empty titles do not produce a title.
fn pasteable_title(tag: &str) -> Option<String> {
    for (offset, _) in tag.match_indices("title") {
        let rest = tag[offset + 5..].trim_start_matches(char::is_whitespace);
        let Some(rest) = rest.strip_prefix('=') else {
            continue;
        };
        let rest = rest.trim_start_matches(char::is_whitespace);
        let Some(rest) = rest.strip_prefix('"') else {
            continue;
        };
        let Some(end) = rest.find('"') else { continue };
        return (end > 0).then(|| rest[..end].to_owned());
    }
    None
}

/// 文本是否含公式（fence 外切得出數學片段；供呼叫方決定是否走數學路徑）。
pub fn contains_math(text: &str) -> bool {
    split_math_segments(text).iter().any(|s| {
        matches!(s, MessageSegment::DisplayMath(_))
            || matches!(s, MessageSegment::Prose(p) | MessageSegment::Quote(p) if split_inline_math(p).iter().any(|q| matches!(q, InlinePart::Math(_))))
    })
}

/// 行內切分：镜像原版正則 `\$([^\s$](?:[^$\n]*[^\s$])?)\$`。
/// Like NSRegularExpression, candidates are consumed non-overlapping even when
/// the later mathy check rejects them. Display delimiters are handled at block
/// level; embedded `$$x$$` still matches its inner `$x$`, as the Swift regex does.
pub fn split_inline_math(line: &str) -> Vec<InlinePart<'_>> {
    let bytes = line.as_bytes();
    let mut parts = Vec::new();
    let mut i = 0usize;
    let mut text_start = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'$' {
            i += 1;
            continue;
        }
        if line[i + 1..]
            .chars()
            .next()
            .is_none_or(|c| c == '$' || c.is_whitespace())
        {
            i += 1;
            continue;
        }
        let mut close: Option<usize> = None;
        let mut j = i + 1;
        while j < bytes.len() {
            if bytes[j] == b'\n' {
                break;
            }
            if bytes[j] == b'$' {
                close = Some(j);
                break;
            }
            j += 1;
        }
        let Some(k) = close else {
            i += 1;
            continue;
        };
        let inner = &line[i + 1..k];
        // A candidate with trailing whitespace does not match the regex at all.
        if inner.chars().last().is_none_or(char::is_whitespace) {
            i += 1;
            continue;
        }
        if is_inline_math_candidate(inner) {
            if text_start < i {
                parts.push(InlinePart::Text(&line[text_start..i]));
            }
            parts.push(InlinePart::Math(inner));
            text_start = k + 1;
        }
        // A regex-matched span is consumed even if the mathy check rejects it.
        i = k + 1;
    }
    if text_start < line.len() {
        parts.push(InlinePart::Text(&line[text_start..]));
    } else if text_start == 0 && line.is_empty() {
        parts.push(InlinePart::Text(line));
    }
    parts
}

/// 對應原版 `extractInlineMath` 的 mathy 判斷：首尾非空白且
/// 「含 `\ ^ _ { } =` 任一，或完全不含空白」。`$` / `\n` 由掃描保證不出現，雙重保險。
fn is_inline_math_candidate(inner: &str) -> bool {
    if inner.is_empty() || inner.contains('$') || inner.contains('\n') {
        return false;
    }
    let first_ok = inner.chars().next().is_some_and(|c| !c.is_whitespace());
    let last_ok = inner.chars().last().is_some_and(|c| !c.is_whitespace());
    if !first_ok || !last_ok {
        return false;
    }
    inner.contains(['\\', '^', '_', '{', '}', '=']) || !inner.contains(' ')
}

/// egui 紋理層快取：`math::render_latex` 點陣 → `TextureHandle`，
/// key 用 [`super::math::math_cache_key`]。`render_latex` 失敗不寫快取、顯示端退回原文。
/// 紋理上限與像素快取同為 [`super::math::MAX_CACHED_FORMULAS`]（drop handle 即釋放 GPU 紋理）。
#[derive(Default)]
pub struct MathTextures {
    textures: HashMap<String, egui::TextureHandle>,
    order: VecDeque<String>,
}

impl MathTextures {
    pub fn new() -> Self {
        Self::default()
    }

    /// 拿（或建）公式紋理；`render_latex` 失敗回 `None`（呼叫方退回 code block）。
    pub fn texture_for(
        &mut self,
        ctx: &egui::Context,
        latex: &str,
        font_size: f32,
        display: bool,
    ) -> Option<egui::TextureHandle> {
        let key = super::math::math_cache_key(latex.trim(), font_size, display);
        if let Some(handle) = self.textures.get(&key) {
            return Some(handle.clone());
        }
        let image = super::math::render_latex(latex, font_size, display).ok()?;
        let handle = ctx.load_texture(key.clone(), image, egui::TextureOptions::LINEAR);
        while self.textures.len() >= super::math::MAX_CACHED_FORMULAS {
            match self.order.pop_front() {
                Some(old) => {
                    self.textures.remove(&old);
                }
                None => break,
            }
        }
        self.order.push_back(key.clone());
        self.textures.insert(key, handle.clone());
        Some(handle)
    }

    /// 渲染一則 assistant 訊息：外層 assistant 氣泡，內部按段渲染；
    /// Prose 走 Markdown（粗斜/標題/列表/連結/行內碼/行內公式混排），
    /// DisplayMath 置中大圖，Code 走等寬高亮塊。任何公式失敗退回原文。
    pub fn show_assistant_message(&mut self, ui: &mut egui::Ui, text: &str) {
        let dark = ui.visuals().dark_mode;
        let pal = super::theme::current_palette(ui.ctx());
        super::theme::bubble_frame(false, pal, dark).show(ui, |ui| {
            for segment in split_math_segments(text) {
                match segment {
                    MessageSegment::Prose(prose) => super::markdown::show_prose(ui, &prose, self),
                    MessageSegment::DisplayMath(latex) => {
                        let tint = ui.visuals().text_color();
                        self.show_display_math(ui, &latex, tint);
                    }
                    MessageSegment::Code { language, content } => {
                        self.show_code_block(ui, language.as_deref(), &content, dark);
                    }
                    MessageSegment::Quote(quote) => super::markdown::show_quote(ui, &quote, self),
                    MessageSegment::Table { header, rows } => {
                        super::markdown::show_table(ui, &header, &rows);
                    }
                    MessageSegment::Pasteable { title, content } => {
                        super::markdown::show_pasteable(ui, title.as_deref(), &content);
                    }
                }
            }
        });
    }

    /// code fence 塊：等寬＋底色＋語言標籤（高亮本體由 markdown 層處理
    /// 行內 Prose 內的 fence；此處處理 math 切分器抽出的頂層 fence）。
    fn show_code_block(
        &mut self,
        ui: &mut egui::Ui,
        language: Option<&str>,
        content: &str,
        _dark: bool,
    ) {
        super::markdown::show_code_block(ui, language, content);
        let _ = self;
    }

    #[allow(dead_code)]
    fn show_prose(&mut self, ui: &mut egui::Ui, prose: &str) {
        let mut paragraph: Vec<&str> = Vec::new();
        for line in prose.lines() {
            if line.trim().is_empty() {
                if !paragraph.is_empty() {
                    self.show_paragraph(ui, &paragraph);
                    paragraph.clear();
                }
                ui.add_space(4.0);
            } else {
                paragraph.push(line);
            }
        }
        if !paragraph.is_empty() {
            self.show_paragraph(ui, &paragraph);
        }
    }

    /// 一段內行內混排：文字與小圖同行 `horizontal_wrapped`。
    /// （Prose 已改走 markdown 層；此為舊路徑保留。）
    #[allow(dead_code)]
    fn show_paragraph(&mut self, ui: &mut egui::Ui, lines: &[&str]) {
        let tint = ui.visuals().text_color();
        ui.horizontal_wrapped(|ui| {
            for (index, line) in lines.iter().enumerate() {
                if index > 0 {
                    ui.label(" ");
                }
                for part in split_inline_math(line) {
                    match part {
                        InlinePart::Text(text) => {
                            ui.label(text);
                        }
                        InlinePart::Math(latex) => {
                            self.show_inline_math(ui, latex, tint);
                        }
                    }
                }
            }
        });
    }

    /// 行內公式：縮小嵌入同行（對應原版行內 `fontSize` 較展示小一級）；
    /// 失敗退回 `$latex$` 等寬原文。
    fn show_inline_math(&mut self, ui: &mut egui::Ui, latex: &str, tint: egui::Color32) {
        let Some(handle) = self.texture_for(ui.ctx(), latex, INLINE_MATH_FONT_SIZE, false) else {
            ui.monospace(format!("${latex}$"));
            return;
        };
        let size = handle.size_vec2();
        if size.x <= 0.0 || size.y <= 0.0 {
            ui.monospace(format!("${latex}$"));
            return;
        }
        let height = (INLINE_MATH_FONT_SIZE * 1.35).ceil();
        let width = (height * size.x / size.y).min(ui.available_width().max(16.0));
        ui.add(
            egui::Image::from_texture(&handle)
                .fit_to_exact_size(egui::vec2(width.max(4.0), height))
                .tint(tint),
        );
    }

    /// 展示公式：置中（對應原版 `MathBlockView` 的全寬置中 + 上下 padding）；
    /// 失敗退回 `$$latex$$` 等寬原文。
    fn show_display_math(&mut self, ui: &mut egui::Ui, latex: &str, tint: egui::Color32) {
        ui.add_space(4.0);
        let handle = self.texture_for(ui.ctx(), latex, DISPLAY_MATH_FONT_SIZE, true);
        let size = handle.as_ref().map(|h| h.size_vec2());
        match (handle, size) {
            (Some(handle), Some(size)) if size.x > 0.0 && size.y > 0.0 => {
                let scale = (ui.available_width().max(16.0) / size.x).min(1.0);
                let target = egui::vec2((size.x * scale).max(4.0), (size.y * scale).max(4.0));
                ui.vertical_centered(|ui| {
                    ui.add(
                        egui::Image::from_texture(&handle)
                            .fit_to_exact_size(target)
                            .tint(tint),
                    );
                });
            }
            _ => {
                ui.vertical_centered(|ui| {
                    ui.add(
                        egui::Label::new(egui::RichText::new(format!("$${latex}$$")).monospace())
                            .selectable(true),
                    );
                });
            }
        }
        ui.add_space(4.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Golden fixtures transcribed against MarkdownText.swift lines 30–151,
    // rather than CommonMark/GFM (which differs on these permissive cases).
    #[test]
    fn swift_segments_golden_all_blocks_in_render_order() {
        let source = "# Intro\n\n```swift full hint\n<pasteable>\n$$x$$\n| raw |\n``` trailing\n<pasteable title = \"Reply\">\n  **literal**\n```py\n$x$\n```\n</pasteable>\n$$x_1$$\n> # Quote\n> - item\n| **Name** | Value |\n| :- | -: |\n| one | `two` |\nTail";
        assert_eq!(
            split_math_segments(source),
            vec![
                MessageSegment::Prose("# Intro".into()),
                MessageSegment::Code {
                    language: Some("swift full hint".into()),
                    content: "<pasteable>\n$$x$$\n| raw |".into()
                },
                MessageSegment::Pasteable {
                    title: Some("Reply".into()),
                    content: "**literal**\n```py\n$x$\n```".into()
                },
                MessageSegment::DisplayMath("x_1".into()),
                MessageSegment::Quote("# Quote\n- item".into()),
                MessageSegment::Table {
                    header: vec!["**Name**".into(), "Value".into()],
                    rows: vec![vec!["one".into(), "`two`".into()]]
                },
                MessageSegment::Prose("Tail".into()),
            ]
        );
    }

    #[test]
    fn swift_segments_golden_unclosed_blocks_and_trailing_newlines() {
        for (source, expected) in [
            (
                "before\n```rust info\nlast\n",
                MessageSegment::Code {
                    language: Some("rust info".into()),
                    content: "last\n".into(),
                },
            ),
            (
                "before\n<pasteable>\n\n  last  \n",
                MessageSegment::Pasteable {
                    title: None,
                    content: "last".into(),
                },
            ),
            (
                "before\n$$\nlast\n",
                MessageSegment::DisplayMath("last\n".into()),
            ),
        ] {
            assert_eq!(
                split_math_segments(source),
                vec![MessageSegment::Prose("before".into()), expected],
                "{source}"
            );
        }
        assert_eq!(
            split_math_segments("```"),
            vec![MessageSegment::Code {
                language: None,
                content: String::new()
            }]
        );
        assert_eq!(
            split_math_segments("<pasteable>"),
            vec![MessageSegment::Pasteable {
                title: None,
                content: String::new()
            }]
        );
        assert_eq!(
            split_math_segments("$$"),
            vec![MessageSegment::DisplayMath(String::new())]
        );
    }

    #[test]
    fn swift_segments_golden_malformed_pasteable_rules() {
        // Recognition is prefix/suffix based, NOT an XML parser. Closing tags
        // must be the whole trimmed line; same-line bodies are NOT supported.
        assert_eq!(
            split_math_segments(
                "<pasteable-ish title=\"\" data-title=\"ignored\">\nx\n</pasteable> extra\ny"
            ),
            vec![MessageSegment::Pasteable {
                title: None,
                content: "x\n</pasteable> extra\ny".into()
            }]
        );
        assert_eq!(
            split_math_segments("<pasteable title='single'>\nx\n</pasteable>"),
            vec![MessageSegment::Pasteable {
                title: None,
                content: "x".into()
            }]
        );
        assert_eq!(
            split_math_segments("<pasteable>inline</pasteable>\nafter"),
            vec![MessageSegment::Pasteable {
                title: None,
                content: "after".into()
            }]
        );
        assert_eq!(
            split_math_segments("<pasteable title=\"cut"),
            vec![MessageSegment::Prose("<pasteable title=\"cut".into())]
        );
        assert_eq!(
            pasteable_title("<pasteable subtitle = \"first\" title=\"second\">"),
            Some("first".into())
        );
        assert!(!contains_math("<pasteable>\n$x$\n$$x$$\n</pasteable>"));
    }

    #[test]
    fn swift_segments_golden_table_rules_and_ragged_cells() {
        assert_eq!(
            split_math_segments("| A | B |\n|-|\n| x || z |\n| tail\nend"),
            vec![
                MessageSegment::Table {
                    header: vec!["A".into(), "B".into()],
                    rows: vec![vec!["x".into(), "".into(), "z".into()], vec!["tail".into()]]
                },
                MessageSegment::Prose("end".into()),
            ]
        );
        // Swift splits even escaped pipes, and does not require three dashes or
        // matching column counts. Tabs INSIDE separator lines are invalid.
        assert_eq!(table_cells("| a\\|b |"), vec!["a\\", "b"]);
        for source in [
            "A | B\n|-|-|",
            "| A |\n| : |",
            "| A |\n|-\t|",
            "| A |\n|--x|",
        ] {
            assert_eq!(
                split_math_segments(source),
                vec![MessageSegment::Prose(source.into())]
            );
        }
    }

    #[test]
    fn swift_segments_golden_quotes_and_math_own_their_bodies() {
        assert_eq!(
            split_math_segments(
                ">   # heading\n>> nested\n>\tkeep tab\n> $$x$$\n> | A |\n> | - |\nplain"
            ),
            vec![
                MessageSegment::Quote(
                    "# heading\n> nested\n\tkeep tab\n$$x$$\n| A |\n| - |".into()
                ),
                MessageSegment::Prose("plain".into()),
            ]
        );
        assert_eq!(
            split_math_segments("$$\n```\n<pasteable>\n$$\nafter"),
            vec![
                MessageSegment::DisplayMath("```\n<pasteable>".into()),
                MessageSegment::Prose("after".into()),
            ]
        );
        assert_eq!(
            split_math_segments("$$x\nnot closed"),
            vec![MessageSegment::Prose("$$x\nnot closed".into())]
        );
        assert_eq!(
            split_math_segments("$$$$"),
            vec![MessageSegment::Prose("$$$$".into())]
        );
    }

    #[test]
    fn swift_segments_does_not_normalize_crlf_or_newline_whitespace() {
        // .components(separatedBy: "\n") + .whitespaces is deliberately NOT
        // Rust str::lines() + trim(). Preserve the original malformed behavior.
        assert_eq!(
            split_math_segments("```rust\r\nbody\r\n```\r\n"),
            vec![MessageSegment::Code {
                language: Some("rust\r".into()),
                content: "body\r".into()
            }]
        );
        assert_eq!(
            split_math_segments("<pasteable>\r\nx\r\n</pasteable>\r\n"),
            vec![MessageSegment::Prose(
                "<pasteable>\r\nx\r\n</pasteable>".into()
            )]
        );
        assert!(!is_table_separator("|-|\r"));
        assert_eq!(trim_horizontal("\t  x \t"), "x");
        assert_eq!(trim_horizontal("\u{b}x\u{c}"), "\u{b}x\u{c}");
    }

    #[test]
    fn display_single_line() {
        assert_eq!(
            split_math_segments("hi\n$$x^2$$\nbye"),
            vec![
                MessageSegment::Prose("hi".to_string()),
                MessageSegment::DisplayMath("x^2".to_string()),
                MessageSegment::Prose("bye".to_string()),
            ]
        );
    }

    #[test]
    fn display_fenced_block() {
        assert_eq!(
            split_math_segments("$$\n\\frac{1}{2}\n$$"),
            vec![MessageSegment::DisplayMath("\\frac{1}{2}".to_string())]
        );
    }

    #[test]
    fn inline_math_split() {
        assert_eq!(
            split_inline_math("a $x^2$ b"),
            vec![
                InlinePart::Text("a "),
                InlinePart::Math("x^2"),
                InlinePart::Text(" b"),
            ]
        );
    }

    #[test]
    fn swift_inline_math_regex_golden_malformed_and_unicode() {
        assert_eq!(
            split_inline_math("$$x$$"),
            vec![
                InlinePart::Text("$"),
                InlinePart::Math("x"),
                InlinePart::Text("$")
            ]
        );
        assert_eq!(
            split_inline_math("$b c$x$"),
            vec![InlinePart::Text("$b c$x$")]
        );
        assert_eq!(
            split_inline_math("$a\nb$ then $z$"),
            vec![InlinePart::Text("$a\nb$ then "), InlinePart::Math("z")]
        );
        assert_eq!(
            split_inline_math("中文 $α_β$ 結束"),
            vec![
                InlinePart::Text("中文 "),
                InlinePart::Math("α_β"),
                InlinePart::Text(" 結束")
            ]
        );
        assert_eq!(
            split_inline_math("$ spaced $ and $x$"),
            vec![InlinePart::Text("$ spaced $ and "), InlinePart::Math("x")]
        );
        assert_eq!(
            split_inline_math("$a b$ $a=b$"),
            vec![InlinePart::Text("$a b$ "), InlinePart::Math("a=b")]
        );
    }

    #[test]
    fn dollar_price_not_math() {
        // 有空白又無 mathy 字元 → 字面；不成對 `$` 也按字面。
        let segments = split_math_segments("apple is $3.");
        assert!(segments
            .iter()
            .all(|s| matches!(s, MessageSegment::Prose(_))));
        assert!(!contains_math("apple is $3."));
        assert!(!contains_math("a $b c$ d"));
        assert!(contains_math("a $x^2$ b"));
        assert!(contains_math("$$\nx\n$$"));
    }

    #[test]
    fn code_fence_shields_math() {
        let segments = split_math_segments("```\n$x^2$\n```");
        assert_eq!(
            segments,
            vec![MessageSegment::Code {
                language: None,
                content: "$x^2$".to_string(),
            }]
        );
        assert!(!contains_math("```\n$x^2$\n```"));
    }

    #[test]
    fn unclosed_fence_is_code() {
        let segments = split_math_segments("```py\n$x^2$");
        assert!(matches!(segments[0], MessageSegment::Code { .. }));
        assert!(!contains_math("```py\n$x^2$"));
    }
}

/// Provider pill 文字（對應 ChatView.modelPill）：`name · model`，
/// model 為空時只顯示 name。main.rs TopBar 由 ProviderStore 讀值後經此格式化。
pub fn provider_pill_text(provider_name: &str, model: &str) -> String {
    let name = provider_name.trim();
    let display_name = if name.is_empty() { "—" } else { name };
    let model = model.trim();
    if model.is_empty() {
        display_name.to_owned()
    } else {
        format!("{display_name} · {model}")
    }
}

/// Waiting row 文案：串流尚未吐出正文時的 transient 狀態列。
/// 有 status 顯示 status，否則回預設「等待回覆…」（對應 pending_status 語義）。
pub fn waiting_label(status: Option<&str>) -> String {
    match status.map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => s.to_owned(),
        None => "等待回覆…".to_owned(),
    }
}
