//! Design system（設計系統）：accent palette + bubble/code/quote frames + one-shot apply。
//!
//! 對應原版 `Theme.swift` / `AccentPicker.swift` 的語義，移植到 egui：
//! - 四 accent 預設 hex 照抄 `Theme.accentOptions`，另支援自訂 hex。
//! - 對比色規則照抄 `contrastingNSColor`：CIE L* ≥ 60 給黑字，否則白字。
//!   注意：不是「WCAG contrast 誰大用誰」—— system blue `#0A84FF` 按 L* 是白字。
//! - 外觀 Light / Dark / Auto；Auto 跟系統。
//!
//! # Orchestrator 接線方式（main / panel 如何取用 palette）
//!
//! 1. 在 `mod.rs` 加一行 `pub mod theme;`（本檔目前是孤立新檔，需掛上模組）。
//! 2. 每幀（或設定變更時）呼叫一次（取代 `settings_ui.apply_visuals` 內的純 Visuals 切換）：
//!    ```ignore
//!    let palette = crate::ui::theme::apply(ctx, "Auto", &settings.accent_hex);
//!    // appearance: "Light" | "Dark" | "Auto"（大小寫不限，前後空白不限；
//!    // 無法識別的字串視同 "Auto"）；accent_hex: "#RRGGBB"（`#` 可省略，大小寫不限）
//!    ```
//! 3. render 端讀取（transcript 氣泡）：
//!    ```ignore
//!    let palette = crate::ui::theme::current_palette(ctx); // apply 存的；沒有就按 ctx.theme() 補預設
//!    let dark = ctx.theme() == egui::Theme::Dark;
//!    egui::Frame::... // 或直接：
//!    crate::ui::theme::bubble_frame(true, palette, dark).show(ui, |ui| {
//!        ui.label(egui::RichText::new("你：…").color(palette.user_text));
//!    });
//!    crate::ui::theme::bubble_frame(false, palette, dark).show(ui, |ui| {
//!        ui.label(egui::RichText::new(text).color(palette.assistant_text));
//!    });
//!    ```
//!    user 氣泡靠右：外層用 `ui.with_layout(bubble_layout(true), …)`；
//!    assistant 靠左：`ui.with_layout(bubble_layout(false), …)`。
//! 4. code 塊 / 引用條：
//!    ```ignore
//!    code_frame(dark).show(ui, |ui| { … });
//!    ui.label(…); // 引用條左側色帶用 quote_accent_stroke(&palette) 或 quote_stroke()
//!    ```
//!
//! 只用 egui，不加依賴。

use eframe::egui;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub enum BubbleStyle {
    #[default]
    #[serde(rename = "accentTint")]
    AccentTint,
    #[serde(rename = "accentFill")]
    AccentFill,
}

impl<'de> serde::Deserialize<'de> for BubbleStyle {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = <serde_json::Value as serde::Deserialize>::deserialize(deserializer)?;
        Ok(if value.as_str() == Some("accentFill") {
            Self::AccentFill
        } else {
            Self::AccentTint
        })
    }
}

// ---------------------------------------------------------------------------
// Accent presets（照抄 Theme.accentOptions，含順序）
// ---------------------------------------------------------------------------

/// 四 accent 預設（原版 `Theme.accentOptions`，順序照抄：藍 / 紫 / 橘 / 綠）。
pub const ACCENT_PRESETS: [&str; 4] = ["#0A84FF", "#BF5AF2", "#FF9F0A", "#30D158"];

/// 預設 accent（原版 `Theme.defaultAccentHex`）。
pub const DEFAULT_ACCENT_HEX: &str = "#0A84FF";

/// 氣泡圓角（user / assistant 共用）。
pub const BUBBLE_RADIUS: u8 = 16;
/// code 塊圓角。
pub const CODE_RADIUS: u8 = 8;
/// 引用條左側色帶寬度。
pub const QUOTE_WIDTH: f32 = 3.0;

/// palette 在 `ctx.data` 的 temp key（固定 `"popchat-palette"`）。
pub const PALETTE_ID: &str = "popchat-palette";

/// [`PALETTE_ID`] 對應的 [`egui::Id`]。
pub fn palette_id() -> egui::Id {
    egui::Id::new(PALETTE_ID)
}

// ---------------------------------------------------------------------------
// Hex parsing（照抄 Theme.components 語義：非六位 hex → 預設藍，不猜黑）
// ---------------------------------------------------------------------------

/// 解析 `#RRGGBB`（`#` 可省略、大小寫不限、前後空白容錯）。
/// 失敗回傳 `None`；呼叫端決定 fallback（語義上等同原版： fallback 預設 accent）。
fn parse_rgb6(hex: &str) -> Option<(u8, u8, u8)> {
    let h = hex.trim().strip_prefix('#').unwrap_or(hex.trim());
    if h.len() != 6 || !h.is_ascii() {
        return None;
    }
    let r = u8::from_str_radix(&h[0..2], 16).ok()?;
    let g = u8::from_str_radix(&h[2..4], 16).ok()?;
    let b = u8::from_str_radix(&h[4..6], 16).ok()?;
    Some((r, g, b))
}

/// `#RRGGBB` → [`egui::Color32`]；解析失敗回傳預設藍（對應 settings.rs 既有行為）。
pub fn hex_to_color32(hex: &str) -> egui::Color32 {
    match parse_rgb6(hex) {
        Some((r, g, b)) => egui::Color32::from_rgb(r, g, b),
        None => egui::Color32::from_rgb(0x0A, 0x84, 0xFF),
    }
}

/// 正規化為 `#RRGGBB`（大寫）；解析失敗回傳預設藍 hex。供設定存檔 round-trip 用。
pub fn normalize_hex(hex: &str) -> String {
    match parse_rgb6(hex) {
        Some((r, g, b)) => format!("#{r:02X}{g:02X}{b:02X}"),
        None => DEFAULT_ACCENT_HEX.to_owned(),
    }
}

/// 0…1 sRGB 分量；非法 hex fallback 預設 accent（照抄 `Theme.components`）。
fn components01(hex: &str) -> (f64, f64, f64) {
    let (r, g, b) = parse_rgb6(hex).unwrap_or((0x0A, 0x84, 0xFF));
    (
        f64::from(r) / 255.0,
        f64::from(g) / 255.0,
        f64::from(b) / 255.0,
    )
}

/// sRGB hex → (hue, saturation, brightness), each in 0…1. Grays have hue 0;
/// the picker, not this conversion, remembers their previous hue. Invalid hex
/// uses the existing default-accent fallback, just like `Theme.hsb`.
pub fn hsb_from_hex(hex: &str) -> (f64, f64, f64) {
    let (red, green, blue) = components01(hex);
    let high = red.max(green).max(blue);
    let low = red.min(green).min(blue);
    let delta = high - low;
    let mut hue = 0.0;
    if delta > 0.0 {
        hue = if high == red {
            (green - blue) / delta + if green < blue { 6.0 } else { 0.0 }
        } else if high == green {
            (blue - red) / delta + 2.0
        } else {
            (red - green) / delta + 4.0
        } / 6.0;
    }
    (hue, if high > 0.0 { delta / high } else { 0.0 }, high)
}

/// HSB → uppercase sRGB `#RRGGBB`, using the arithmetic from `Theme.rgb/byte`.
/// Hue wraps (including negative turns); saturation/brightness clamp to 0…1.
/// Channels round to the nearest byte, with half values rounded up. Non-finite
/// hue/NaN components defensively become zero; no OS color space or HDR is used.
pub fn hex_from_hsb(hue: f64, saturation: f64, brightness: f64) -> String {
    let wrapped = if hue.is_finite() {
        ((hue % 1.0 + 1.0) % 1.0) * 6.0
    } else {
        0.0
    };
    let unit = |value: f64| {
        if value.is_nan() {
            0.0
        } else {
            value.clamp(0.0, 1.0)
        }
    };
    let saturation = unit(saturation);
    let value = unit(brightness);
    let sector = wrapped.floor();
    let fraction = wrapped - sector;
    let p = value * (1.0 - saturation);
    let q = value * (1.0 - saturation * fraction);
    let t = value * (1.0 - saturation * (1.0 - fraction));
    let (red, green, blue) = match sector as u8 % 6 {
        0 => (value, t, p),
        1 => (q, value, p),
        2 => (p, value, t),
        3 => (p, q, value),
        4 => (t, p, value),
        _ => (value, p, q),
    };
    let byte = |channel: f64| (channel.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02X}{:02X}{:02X}", byte(red), byte(green), byte(blue))
}

// ---------------------------------------------------------------------------
// Contrasting text（照抄 contrastingNSColor：CIE L* at 60）
// ---------------------------------------------------------------------------

/// 原版 `contrastingNSColor` 語義：CIE L* ≥ 60 → 黑字，否則白字。
/// 不是 WCAG ratio 最大者（system blue 按 ratio 該是黑字，但設計上是白字）。
pub fn contrasting_text_on(hex: &str) -> egui::Color32 {
    fn linear(c: f64) -> f64 {
        if c <= 0.03928 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }
    let (r, g, b) = components01(hex);
    let luminance = 0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b);
    let lightness = if luminance <= 0.008856 {
        903.3 * luminance
    } else {
        116.0 * luminance.powf(1.0 / 3.0) - 16.0
    };
    if lightness >= 60.0 {
        egui::Color32::BLACK
    } else {
        egui::Color32::WHITE
    }
}

// ---------------------------------------------------------------------------
// Mode-dependent surfaces（對應 Theme lifted/recessed 語義：三層深度）
// ---------------------------------------------------------------------------

/// Custom soft-beige light appearance. Keep neutral surfaces separate from
/// accent colors, white foregrounds, and image/formula texture pixels.
pub const LIGHT_PANEL: egui::Color32 = egui::Color32::from_rgb(0xF5, 0xED, 0xDD);
pub const LIGHT_CONTROL: egui::Color32 = egui::Color32::from_rgb(0xED, 0xE1, 0xCA);
pub const LIGHT_RAISED: egui::Color32 = egui::Color32::from_rgb(0xFA, 0xF2, 0xE3);
pub const LIGHT_HOVER: egui::Color32 = egui::Color32::from_rgb(0xE5, 0xD6, 0xBA);
pub const LIGHT_BORDER: egui::Color32 = egui::Color32::from_rgb(0xD3, 0xC3, 0xA5);
pub const LIGHT_SECONDARY: egui::Color32 = egui::Color32::from_rgb(0x62, 0x5B, 0x50);

/// Assistant surface: warm cream in light mode, unchanged deep gray in dark.
pub fn assistant_bg_for(dark: bool) -> egui::Color32 {
    if dark {
        egui::Color32::from_rgb(0x25, 0x28, 0x32)
    } else {
        LIGHT_RAISED
    }
}

/// assistant 氣泡字：亮底黑字 / 暗底白字（`.primary` 語義）。
pub fn assistant_text_for(dark: bool) -> egui::Color32 {
    if dark {
        egui::Color32::WHITE
    } else {
        egui::Color32::BLACK
    }
}

/// Code blocks use a recessed cream surface in light mode / near-black in dark.
pub fn code_bg_for(dark: bool) -> egui::Color32 {
    if dark {
        egui::Color32::from_rgb(0x20, 0x23, 0x2C)
    } else {
        LIGHT_CONTROL
    }
}

/// 氣泡描邊（hairline）：亮底黑 6% / 暗底白 9%。
fn bubble_stroke(dark: bool) -> egui::Stroke {
    if dark {
        egui::Stroke::new(1.0_f32, egui::Color32::from_white_alpha(24))
    } else {
        egui::Stroke::new(1.0_f32, egui::Color32::from_black_alpha(16))
    }
}

// ---------------------------------------------------------------------------
// AccentPalette
// ---------------------------------------------------------------------------

/// 一次算好的設計 token：`apply` 存進 `ctx.data`，render 端用
/// [`current_palette`] 讀回。全部 `Copy`，跨幀傳遞零成本。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AccentPalette {
    /// accent 本體（自訂 hex 或四預設之一）。
    pub accent: egui::Color32,
    /// accent 底上的字（對比色規則：亮底黑字 / 暗底白字）。
    pub accent_text: egui::Color32,
    /// User bubble surface, default accentTint; for_bubble_style selects fill.
    pub user_bubble: egui::Color32,
    /// Primary text for tint; contrast text for a filled accent surface.
    pub user_text: egui::Color32,
    /// assistant 氣泡底（微灰，與 user 底永遠不同）。
    pub assistant_bg: egui::Color32,
    /// assistant 氣泡字（亮黑 / 暗白）。
    pub assistant_text: egui::Color32,
    /// code 塊底（recessed）。
    pub code_bg: egui::Color32,
    /// 建 palette 時的深淺模式。
    pub dark: bool,
}

impl AccentPalette {
    pub fn for_bubble_style(mut self, style: BubbleStyle) -> Self {
        match style {
            BubbleStyle::AccentTint => {
                self.user_bubble = self
                    .accent
                    .gamma_multiply(if self.dark { 0.19 } else { 0.15 });
                self.user_text = self.assistant_text;
            }
            BubbleStyle::AccentFill => {
                self.user_bubble = self.accent;
                self.user_text = self.accent_text;
            }
        }
        self
    }
    /// 由 hex 建 palette。hex 容錯同 [`hex_to_color32`]（非法 → 預設藍）。
    pub fn from_hex(hex: &str, dark: bool) -> Self {
        let accent = hex_to_color32(hex);
        let accent_text = contrasting_text_on(hex);
        Self {
            accent,
            accent_text,
            user_bubble: accent.gamma_multiply(if dark { 0.19 } else { 0.15 }),
            user_text: assistant_text_for(dark),
            assistant_bg: assistant_bg_for(dark),
            assistant_text: assistant_text_for(dark),
            code_bg: code_bg_for(dark),
            dark,
        }
    }

    /// 預設藍 palette。
    pub fn default_for(dark: bool) -> Self {
        Self::from_hex(DEFAULT_ACCENT_HEX, dark)
    }
}

impl Default for AccentPalette {
    fn default() -> Self {
        Self::default_for(false)
    }
}

// ---------------------------------------------------------------------------
// Frames
// ---------------------------------------------------------------------------

/// 訊息氣泡框：圓角 12。user = accent 底 + 對比字；assistant = 微灰底。
///
/// 對齊（靠右 / 靠左）是 layout 的事，不是 Frame 的事，外層包：
/// `ui.with_layout(bubble_layout(user), |ui| bubble_frame(…).show(ui, …))`。
pub fn bubble_frame(user: bool, palette: AccentPalette, _dark: bool) -> egui::Frame {
    let fill = if user {
        palette.user_bubble
    } else {
        egui::Color32::TRANSPARENT
    };
    egui::Frame::new()
        .fill(fill)
        .stroke(egui::Stroke::NONE)
        .corner_radius(egui::CornerRadius::same(BUBBLE_RADIUS))
        .inner_margin(if user {
            egui::Margin::symmetric(16, 11)
        } else {
            egui::Margin::symmetric(0, 3)
        })
        .outer_margin(egui::Margin::symmetric(0, 4))
}

/// 氣泡用的左右對齊 layout：user 靠右、assistant 靠左。
pub fn bubble_layout(user: bool) -> egui::Layout {
    if user {
        egui::Layout::right_to_left(egui::Align::Min)
    } else {
        egui::Layout::left_to_right(egui::Align::Min)
    }
}

/// code 塊框：recessed 下沉表面，圓角 8 + hairline。
pub fn code_frame(dark: bool) -> egui::Frame {
    egui::Frame::new()
        .fill(code_bg_for(dark))
        .stroke(bubble_stroke(dark))
        .corner_radius(egui::CornerRadius::same(CODE_RADIUS))
        .inner_margin(egui::Margin::symmetric(8, 6))
}

/// 引用條左側色帶（中性灰，寬 [`QUOTE_WIDTH`]）。
pub fn quote_stroke() -> egui::Stroke {
    egui::Stroke::new(QUOTE_WIDTH, egui::Color32::GRAY)
}

/// 引用條左側色帶（accent 色，寬 [`QUOTE_WIDTH`]）。
pub fn quote_accent_stroke(palette: AccentPalette) -> egui::Stroke {
    egui::Stroke::new(QUOTE_WIDTH, palette.accent)
}

// ---------------------------------------------------------------------------
// apply / read-back
// ---------------------------------------------------------------------------

/// 系統是否深色。注意：egui 0.32 沒有 `ctx.os_default_theme`，
/// 等價寫法是 `ctx.system_theme()`（`None` 時退回目前 `ctx.theme()`）。
pub fn system_dark(ctx: &egui::Context) -> bool {
    match ctx.system_theme() {
        Some(theme) => theme == egui::Theme::Dark,
        None => ctx.theme() == egui::Theme::Dark,
    }
}

/// appearance 字串 → 深淺。`"light"` / `"dark"` 精確指定（大小寫、空白容錯），
/// `"auto"` 與其他一切字串都跟系統。
pub fn resolve_dark(ctx: &egui::Context, appearance: &str) -> bool {
    match appearance.trim().to_ascii_lowercase().as_str() {
        "light" => false,
        "dark" => true,
        _ => system_dark(ctx),
    }
}

/// 一次套好：Visuals（淺 / 深 / 跟系統）+ selection/hyperlink 吃 accent +
/// palette 存進 `ctx.data`（key `"popchat-palette"`）。
///
/// 回傳算好的 [`AccentPalette`]（render 端也可稍後用 [`current_palette`] 讀回）。
pub fn apply(ctx: &egui::Context, appearance: &str, accent_hex: &str) -> AccentPalette {
    let dark = resolve_dark(ctx, appearance);
    let palette = AccentPalette::from_hex(accent_hex, dark);
    let mut visuals = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    // accent 出沒之處：選取底 + 連結色（對應「custom accent 用在所有 accent 位置」）。
    // 注意：egui 的 selected label 文字色取 selection.stroke.color——
    // 若也設成 accent 就會「藍底藍字」看不見（實測踩過），用對比色。
    visuals.selection.bg_fill = palette.accent;
    visuals.selection.stroke.color = palette.accent_text;
    visuals.hyperlink_color = palette.accent;
    // Keep small CJK copy and placeholder text readable on the actual card
    // surfaces, rather than relying on egui's faint default weak-text blend.
    visuals.override_text_color = Some(if dark {
        egui::Color32::from_rgb(0xE5, 0xE5, 0xE8)
    } else {
        egui::Color32::from_rgb(0x29, 0x29, 0x30)
    });
    visuals.weak_text_color = Some(if dark {
        egui::Color32::from_rgb(0xAA, 0xB1, 0xBF)
    } else {
        LIGHT_SECONDARY
    });
    visuals.extreme_bg_color = if dark {
        egui::Color32::from_rgb(0x22, 0x25, 0x2F)
    } else {
        LIGHT_CONTROL
    };
    visuals.faint_bg_color = if dark {
        egui::Color32::from_rgb(0x25, 0x28, 0x32)
    } else {
        LIGHT_CONTROL
    };
    visuals.code_bg_color = code_bg_for(dark);
    visuals.window_fill = panel_fill(dark);
    visuals.window_corner_radius = egui::CornerRadius::same(18);
    visuals.popup_shadow = super::surface::popup_shadow();
    visuals.window_shadow = super::surface::popup_shadow();
    visuals.window_stroke = egui::Stroke::new(
        1.0_f32,
        if dark {
            egui::Color32::from_white_alpha(23)
        } else {
            egui::Color32::from_black_alpha(16)
        },
    );
    let primary = visuals.override_text_color.unwrap();
    let border = if dark {
        egui::Color32::from_rgb(0x39, 0x3D, 0x49)
    } else {
        LIGHT_BORDER
    };
    for widget in [
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
    ] {
        widget.corner_radius = egui::CornerRadius::same(7);
        widget.fg_stroke.color = primary;
        widget.bg_stroke = egui::Stroke::new(1.0_f32, border);
    }
    visuals.widgets.inactive.bg_fill = visuals.faint_bg_color;
    visuals.widgets.inactive.weak_bg_fill = if dark {
        visuals.extreme_bg_color
    } else {
        LIGHT_RAISED
    };
    visuals.widgets.hovered.bg_fill = if dark {
        egui::Color32::from_rgb(0x33, 0x37, 0x43)
    } else {
        LIGHT_HOVER
    };
    visuals.widgets.hovered.weak_bg_fill = visuals.widgets.hovered.bg_fill;
    visuals.widgets.active.bg_fill = visuals.widgets.hovered.bg_fill;
    visuals.widgets.active.weak_bg_fill = visuals.widgets.hovered.bg_fill;
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, border);
    if !dark {
        // Central panels, popup-open buttons, and passive widget frames must
        // not fall back to egui's stock white/cool-gray light surfaces.
        visuals.panel_fill = LIGHT_PANEL;
        visuals.widgets.noninteractive.bg_fill = LIGHT_PANEL;
        visuals.widgets.noninteractive.weak_bg_fill = LIGHT_PANEL;
        visuals.widgets.open.bg_fill = LIGHT_HOVER;
        visuals.widgets.open.weak_bg_fill = LIGHT_HOVER;
        visuals.widgets.open.bg_stroke = egui::Stroke::new(1.0_f32, border);
        visuals.widgets.open.fg_stroke.color = primary;
    }
    ctx.set_visuals(visuals);
    ctx.style_mut(|style| {
        style
            .text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(15.0));
        style
            .text_styles
            .insert(egui::TextStyle::Small, egui::FontId::proportional(12.0));
        style
            .text_styles
            .insert(egui::TextStyle::Button, egui::FontId::proportional(13.0));
        style
            .text_styles
            .insert(egui::TextStyle::Monospace, egui::FontId::monospace(13.0));
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(12.0, 7.0);
        style.spacing.interact_size.y = 30.0;
    });
    ctx.data_mut(|d| d.insert_temp(palette_id(), palette));
    palette
}

pub fn panel_fill(dark: bool) -> egui::Color32 {
    if dark {
        egui::Color32::from_rgb(0x1B, 0x1D, 0x25)
    } else {
        LIGHT_PANEL
    }
}

/// 讀回 [`apply`] 存的 palette；沒存過（首幀前）就按目前 theme 補預設藍。
pub fn get_palette(ctx: &egui::Context) -> Option<AccentPalette> {
    ctx.data(|d| d.get_temp(palette_id()))
}

/// render 端入口：有存就用存的，沒有就補預設（永不回 `None`）。
pub fn current_palette(ctx: &egui::Context) -> AccentPalette {
    get_palette(ctx).unwrap_or_else(|| AccentPalette::default_for(ctx.theme() == egui::Theme::Dark))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn luminance(color: egui::Color32) -> f64 {
        let linear = |value: u8| {
            let v = f64::from(value) / 255.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(color.r()) + 0.7152 * linear(color.g()) + 0.0722 * linear(color.b())
    }

    #[test]
    fn light_neutral_surfaces_are_opaque_beige_not_white() {
        let ctx = egui::Context::default();
        let palette = apply(&ctx, "Light", DEFAULT_ACCENT_HEX);
        let style = ctx.style();
        let visuals = &style.visuals;
        let mut surfaces = vec![
            LIGHT_PANEL,
            LIGHT_CONTROL,
            LIGHT_RAISED,
            LIGHT_HOVER,
            LIGHT_BORDER,
            panel_fill(false),
            palette.assistant_bg,
            palette.code_bg,
            code_frame(false).fill,
            visuals.panel_fill,
            visuals.window_fill,
            visuals.extreme_bg_color,
            visuals.faint_bg_color,
            visuals.code_bg_color,
        ];
        for widget in [
            &visuals.widgets.noninteractive,
            &visuals.widgets.inactive,
            &visuals.widgets.hovered,
            &visuals.widgets.active,
            &visuals.widgets.open,
        ] {
            surfaces.extend([widget.bg_fill, widget.weak_bg_fill]);
        }
        for surface in surfaces {
            assert!(surface.is_opaque(), "neutral surface: {surface:?}");
            assert_ne!(surface, egui::Color32::WHITE);
            assert!(
                surface.r() >= surface.g() && surface.g() > surface.b(),
                "neutral surface must stay warm: {surface:?}"
            );
        }
        assert!(luminance(LIGHT_RAISED) > luminance(LIGHT_PANEL));
        assert!(luminance(LIGHT_PANEL) > luminance(LIGHT_CONTROL));
        assert!(luminance(LIGHT_CONTROL) > luminance(LIGHT_HOVER));
        assert_eq!(palette.accent, hex_to_color32(DEFAULT_ACCENT_HEX));
        assert_eq!(palette.accent_text, egui::Color32::WHITE);
        assert_eq!(
            bubble_frame(false, palette, false).fill,
            egui::Color32::TRANSPARENT
        );
    }

    #[test]
    fn beige_override_does_not_change_dark_surfaces() {
        let ctx = egui::Context::default();
        apply(&ctx, "Light", DEFAULT_ACCENT_HEX);
        let palette = apply(&ctx, "Dark", DEFAULT_ACCENT_HEX);
        let style = ctx.style();
        let visuals = &style.visuals;
        let stock = egui::Visuals::dark();
        assert_eq!(panel_fill(true), egui::Color32::from_rgb(0x1B, 0x1D, 0x25));
        assert_eq!(
            palette.assistant_bg,
            egui::Color32::from_rgb(0x25, 0x28, 0x32)
        );
        assert_eq!(palette.code_bg, egui::Color32::from_rgb(0x20, 0x23, 0x2C));
        assert_eq!(
            visuals.extreme_bg_color,
            egui::Color32::from_rgb(0x22, 0x25, 0x2F)
        );
        assert_eq!(
            visuals.widgets.inactive.weak_bg_fill,
            visuals.extreme_bg_color
        );
        assert_eq!(
            visuals.widgets.hovered.bg_fill,
            egui::Color32::from_rgb(0x33, 0x37, 0x43)
        );
        assert_eq!(visuals.panel_fill, stock.panel_fill);
        assert_eq!(
            visuals.widgets.noninteractive.bg_fill,
            stock.widgets.noninteractive.bg_fill
        );
        assert_eq!(visuals.widgets.open.bg_fill, stock.widgets.open.bg_fill);
        assert_eq!(
            visuals.widgets.open.weak_bg_fill,
            stock.widgets.open.weak_bg_fill
        );
        assert_eq!(visuals.window_shadow, super::super::surface::popup_shadow());
        assert_eq!(visuals.popup_shadow, super::super::surface::popup_shadow());
    }

    #[test]
    fn source_hsb_conversion_golden_cases() {
        // Literal results of Theme.swift's six-sector sRGB arithmetic. These
        // intentionally are not egui Hsva (which is not the source contract).
        for (h, s, b, expected) in [
            (0.0, 1.0, 1.0, "#FF0000"),
            (1.0 / 6.0, 1.0, 1.0, "#FFFF00"),
            (2.0 / 6.0, 1.0, 1.0, "#00FF00"),
            (3.0 / 6.0, 1.0, 1.0, "#00FFFF"),
            (4.0 / 6.0, 1.0, 1.0, "#0000FF"),
            (5.0 / 6.0, 1.0, 1.0, "#FF00FF"),
            (1.0, 1.0, 1.0, "#FF0000"),
            (-0.25, 1.0, 1.0, "#8000FF"),
            (2.5, 1.0, 1.0, "#00FFFF"),
            (0.0, 0.5, 0.5, "#804040"),
            (0.5, 0.5, 0.5, "#408080"),
            (0.75, 0.0, 0.5, "#808080"),
            (0.75, 1.0, 0.0, "#000000"),
            (0.0, 4.0, 4.0, "#FF0000"),
            (0.3, -1.0, 2.0, "#FFFFFF"),
            (0.3, 1.0, -1.0, "#000000"),
        ] {
            assert_eq!(hex_from_hsb(h, s, b), expected, "HSB {h}/{s}/{b}");
        }
        for (hex, expected) in [
            ("#000000", (0.0, 0.0, 0.0)),
            ("#808080", (0.0, 0.0, 128.0 / 255.0)),
            ("#FFFFFF", (0.0, 0.0, 1.0)),
            ("#FF0080", ((6.0 - 128.0 / 255.0) / 6.0, 1.0, 1.0)),
            ("#0A84FF", ((4.0 - 122.0 / 245.0) / 6.0, 245.0 / 255.0, 1.0)),
            (
                "#BF5AF2",
                ((4.0 + 101.0 / 152.0) / 6.0, 152.0 / 242.0, 242.0 / 255.0),
            ),
            ("#FF9F0A", (149.0 / 245.0 / 6.0, 245.0 / 255.0, 1.0)),
            (
                "#30D158",
                ((2.0 + 40.0 / 161.0) / 6.0, 161.0 / 209.0, 209.0 / 255.0),
            ),
        ] {
            let actual = hsb_from_hex(hex);
            for (a, b) in [
                (actual.0, expected.0),
                (actual.1, expected.1),
                (actual.2, expected.2),
            ] {
                assert!((a - b).abs() < 1e-12, "{hex}: {actual:?} != {expected:?}");
            }
            assert_eq!(hex_from_hsb(actual.0, actual.1, actual.2), hex);
        }
        assert_eq!(hsb_from_hex("bad"), hsb_from_hex(DEFAULT_ACCENT_HEX));
    }

    #[test]
    fn hsb_round_trips_all_grays_and_a_full_rgb_grid_without_hdr() {
        for red in (0..=255).step_by(17) {
            for green in (0..=255).step_by(17) {
                for blue in (0..=255).step_by(17) {
                    let hex = format!("#{red:02X}{green:02X}{blue:02X}");
                    let (h, s, b) = hsb_from_hex(&hex);
                    assert_eq!(hex_from_hsb(h, s, b), hex);
                }
            }
        }
        for gray in 0..=255 {
            let hex = format!("#{gray:02X}{gray:02X}{gray:02X}");
            let (_, s, b) = hsb_from_hex(&hex);
            assert_eq!(hex_from_hsb(0.73, s, b), hex);
        }
        assert_eq!(
            hex_from_hsb(f64::NAN, f64::INFINITY, f64::INFINITY),
            "#FF0000"
        );
        assert_eq!(hex_from_hsb(0.5, f64::NAN, f64::NAN), "#000000");
    }

    #[test]
    fn source_bubble_styles_and_retired_style_fallback_are_preserved() {
        assert_eq!(
            serde_json::from_str::<BubbleStyle>("\"quietGray\"").unwrap(),
            BubbleStyle::AccentTint
        );
        for dark in [false, true] {
            let tint =
                AccentPalette::from_hex("#0A84FF", dark).for_bubble_style(BubbleStyle::AccentTint);
            assert_eq!(
                tint.user_bubble,
                tint.accent.gamma_multiply(if dark { 0.19 } else { 0.15 })
            );
            assert_eq!(tint.user_text, tint.assistant_text);
            let filled = tint.for_bubble_style(BubbleStyle::AccentFill);
            assert_eq!(filled.user_bubble, filled.accent);
            assert_eq!(filled.user_text, egui::Color32::WHITE);
            assert_eq!(
                AccentPalette::from_hex("#FFFF99", dark)
                    .for_bubble_style(BubbleStyle::AccentFill)
                    .user_text,
                egui::Color32::BLACK
            );
        }
    }

    #[test]
    fn secondary_copy_is_readable_on_chat_surfaces_in_both_modes() {
        let ctx = egui::Context::default();
        for (mode, dark) in [("light", false), ("dark", true)] {
            apply(&ctx, mode, DEFAULT_ACCENT_HEX);
            let style = ctx.style();
            let visuals = &style.visuals;
            for text in [visuals.text_color(), visuals.weak_text_color()] {
                for background in [
                    panel_fill(dark),
                    assistant_bg_for(dark),
                    code_bg_for(dark),
                    visuals.extreme_bg_color,
                    visuals.widgets.inactive.weak_bg_fill,
                    visuals.widgets.hovered.bg_fill,
                    visuals.window_fill,
                ] {
                    let a = luminance(text);
                    let b = luminance(background);
                    let ratio = (a.max(b) + 0.05) / (a.min(b) + 0.05);
                    assert!(
                        ratio >= 4.5,
                        "{mode}: copy {text:?} on {background:?} contrast is {ratio}"
                    );
                }
            }
            assert_eq!(ctx.style().text_styles[&egui::TextStyle::Body].size, 15.0);
        }
    }

    #[test]
    fn contrast_black_and_white_verdicts() {
        // 亮底黑字 / 暗底白字（contrastingNSColor 語義）。
        assert_eq!(contrasting_text_on("#FFFFFF"), egui::Color32::BLACK);
        assert_eq!(contrasting_text_on("#000000"), egui::Color32::WHITE);
        // 關鍵：system blue 按 L*（~56 < 60）是白字，不是 WCAG-ratio 勝出的黑字。
        assert_eq!(contrasting_text_on("#0A84FF"), egui::Color32::WHITE);
        // 淺黃 / 亮綠按 L*（≥ 60）是黑字。
        assert_eq!(contrasting_text_on("#FFFF99"), egui::Color32::BLACK);
        assert_eq!(contrasting_text_on("#30D158"), egui::Color32::BLACK);
    }

    #[test]
    fn hex_parse_case_and_hash_tolerant() {
        let a = hex_to_color32("#0A84FF");
        for variant in ["#0a84ff", "0A84FF", "0a84ff", "  #0a84ff  ", "#0A84Ff"] {
            assert_eq!(hex_to_color32(variant), a, "variant: {variant}");
        }
        // 非法 hex → 預設藍（原版 components fallback 語義），不是黑。
        let fallback = hex_to_color32(DEFAULT_ACCENT_HEX);
        for bad in [
            "",
            "#",
            "#ZZZZZZ",
            "#12345",
            "#1234567",
            "not-a-color",
            "你好",
            "#你好",
            "aé€",
        ] {
            assert_eq!(hex_to_color32(bad), fallback, "bad: {bad}");
        }
        assert_eq!(normalize_hex("0a84ff"), "#0A84FF");
        assert_eq!(normalize_hex("garbage"), DEFAULT_ACCENT_HEX);
    }

    #[test]
    fn user_and_assistant_backgrounds_differ() {
        for hex in ACCENT_PRESETS
            .iter()
            .copied()
            .chain(std::iter::once("#FFFF99"))
        {
            for dark in [false, true] {
                let p = AccentPalette::from_hex(hex, dark);
                assert_ne!(
                    p.user_bubble, p.assistant_bg,
                    "hex={hex} dark={dark}: user/assistant 底色必須不同"
                );
                assert_ne!(
                    p.user_bubble, p.accent,
                    "user bubble should be a subtle accent tint"
                );
                let uf = bubble_frame(true, p, dark);
                let af = bubble_frame(false, p, dark);
                assert_eq!(uf.fill, p.user_bubble);
                assert_eq!(af.fill, egui::Color32::TRANSPARENT);
                assert_eq!(af.stroke, egui::Stroke::NONE);
                assert_ne!(uf.fill, af.fill);
                assert_eq!(uf.corner_radius, egui::CornerRadius::same(BUBBLE_RADIUS));
            }
        }
    }

    #[test]
    fn apply_stores_palette_for_render() {
        let ctx = egui::Context::default();
        let p = apply(&ctx, "dark", "#0A84FF");
        assert!(p.dark);
        assert_eq!(p.accent, hex_to_color32("#0A84FF"));
        assert_eq!(get_palette(&ctx), Some(p));
        assert_eq!(current_palette(&ctx), p);

        let p2 = apply(&ctx, "Light", "#30d158");
        assert!(!p2.dark);
        assert_eq!(get_palette(&ctx), Some(p2));

        // Auto 跟系統：headless 下不斷言深淺，只斷言有存且可讀。
        let p3 = apply(&ctx, "Auto", "#BF5AF2");
        assert_eq!(get_palette(&ctx), Some(p3));
    }
}
