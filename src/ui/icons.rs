//! Font-independent controls using vendored Font Awesome Free SVGs.
//! Attribution, upstream pin, and icon mapping: `assets/font-awesome/README.md`.

use eframe::egui;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
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

const GLYPH_SIZE: f32 = 16.0;
const RASTER_SIZES: [u32; 3] = [32, 64, 128];
const HOVER_HELP_DELAY: f64 = 0.18;

#[derive(Clone, Copy)]
struct HoverHelp {
    widget: egui::Id,
    entered: f64,
    frame: u64,
}

/// Short, movement-tolerant hints for icon-only controls, including disabled
/// controls. Keep one timer per context, not one retained entry per message.
/// Tooltip's own popup handles viewport fitting; it does not inherit the chat
/// body's clipping, translation, text scaling or opacity.
pub fn hover_help(response: egui::Response, label: &str) -> egui::Response {
    let ctx = &response.ctx;
    let key = egui::Id::new("popchat-icon-hover-help");
    let (now, pressed, scrolling) = ctx.input(|input| {
        (
            input.time,
            input.pointer.any_down(),
            input.smooth_scroll_delta != egui::Vec2::ZERO,
        )
    });
    // The popup's initial sizing/fitting pass can overlap a control near the
    // native viewport edge. Its own hint must not break the owner's hover timer.
    let over_own_hint = response.is_tooltip_open()
        && ctx.input(|input| {
            input
                .pointer
                .hover_pos()
                .is_some_and(|pos| response.interact_rect.contains(pos))
        });
    let hovered = (response.contains_pointer() || over_own_hint)
        && !pressed
        && !scrolling
        && !egui::Popup::is_any_open(ctx)
        && !label.trim().is_empty()
        && now.is_finite();
    if !hovered {
        ctx.data_mut(|data| {
            if data
                .get_temp::<HoverHelp>(key)
                .is_some_and(|state| state.widget == response.id)
            {
                data.remove::<HoverHelp>(key);
            }
        });
        return response;
    }
    let frame = ctx.cumulative_frame_nr();
    let state = ctx.data_mut(|data| {
        let state = data
            .get_temp::<HoverHelp>(key)
            .filter(|state| {
                state.widget == response.id
                    && now >= state.entered
                    && frame.saturating_sub(state.frame) <= 1
            })
            .map_or(
                HoverHelp {
                    widget: response.id,
                    entered: now,
                    frame,
                },
                |mut state| {
                    state.frame = frame;
                    state
                },
            );
        data.insert_temp(key, state);
        state
    });
    let remaining = HOVER_HELP_DELAY - (now - state.entered);
    if remaining > 0.0 {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(remaining));
    } else {
        egui::Tooltip::for_widget(&response)
            .width((ctx.screen_rect().width() - 24.0).clamp(80.0, 280.0))
            .show(|ui| {
                ui.add(
                    egui::Label::new(egui::RichText::new(label).size(13.0))
                        .wrap()
                        .selectable(false),
                );
            });
    }
    response
}

impl Icon {
    fn svg(self) -> &'static str {
        match self {
            Self::Add => include_str!("../../assets/font-awesome/solid/plus.svg"),
            Self::NewChat => include_str!("../../assets/font-awesome/solid/comment-medical.svg"),
            Self::History => include_str!("../../assets/font-awesome/solid/clock-rotate-left.svg"),
            Self::Copy => include_str!("../../assets/font-awesome/solid/copy.svg"),
            Self::Check => include_str!("../../assets/font-awesome/solid/check.svg"),
            Self::Fork => include_str!("../../assets/font-awesome/solid/code-branch.svg"),
            Self::Retry => include_str!("../../assets/font-awesome/solid/rotate-right.svg"),
            Self::Edit => include_str!("../../assets/font-awesome/solid/pen.svg"),
            Self::Close => include_str!("../../assets/font-awesome/solid/xmark.svg"),
            Self::Settings => include_str!("../../assets/font-awesome/solid/gear.svg"),
            Self::Pin => include_str!("../../assets/font-awesome/solid/thumbtack.svg"),
            Self::Send => include_str!("../../assets/font-awesome/solid/arrow-up.svg"),
            Self::Stop => include_str!("../../assets/font-awesome/solid/stop.svg"),
            Self::Expand => include_str!(
                "../../assets/font-awesome/solid/up-right-and-down-left-from-center.svg"
            ),
        }
    }
}

fn raster_size(pixels_per_point: f32) -> u32 {
    let requested = GLYPH_SIZE * pixels_per_point;
    RASTER_SIZES
        .into_iter()
        .find(|&size| size as f32 >= requested)
        .unwrap_or(128)
}

fn rasterize(icon: Icon, size: u32) -> egui::ColorImage {
    // Keep official geometry and attribution untouched. Upstream's default black
    // fill becomes a white mask; egui supplies palette/disabled tint at paint time.
    let svg = icon.svg().replacen("<svg ", "<svg fill=\"#fff\" ", 1);
    let tree = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default())
        .expect("vendored Font Awesome SVG must be valid");
    let dimensions = tree.size();
    let scale = size as f32 / dimensions.width().max(dimensions.height());
    let x = (size as f32 - dimensions.width() * scale) / 2.0;
    let y = (size as f32 - dimensions.height() * scale) / 2.0;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size).expect("bounded icon raster");
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_row(scale, 0.0, 0.0, scale, x, y),
        &mut pixmap.as_mut(),
    );
    egui::ColorImage::from_rgba_premultiplied([size as usize, size as usize], pixmap.data())
}

#[derive(Default)]
struct IconCache {
    // Only 14 icons x 3 resolutions; colors and arbitrary monitor scales never
    // create entries. Max RGBA storage is 1,204,224 bytes per CPU/GPU copy.
    textures: HashMap<(Icon, u32), egui::TextureHandle>,
}

fn cache(ctx: &egui::Context) -> Arc<Mutex<IconCache>> {
    ctx.data_mut(|data| {
        data.get_temp_mut_or_default::<Arc<Mutex<IconCache>>>(egui::Id::new(
            "font-awesome-icons-v1",
        ))
        .clone()
    })
}

fn texture(ctx: &egui::Context, icon: Icon, pixels_per_point: f32) -> egui::TextureHandle {
    let cache = cache(ctx);
    let mut cache = cache.lock().expect("icon cache lock");
    let size = raster_size(pixels_per_point);
    cache
        .textures
        .entry((icon, size))
        .or_insert_with(|| {
            ctx.load_texture(
                format!("font-awesome-{icon:?}-{size}"),
                rasterize(icon, size),
                egui::TextureOptions::LINEAR,
            )
        })
        .clone()
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
        let texture = texture(ui.ctx(), icon, ui.ctx().pixels_per_point());
        painter.image(
            texture.id(),
            egui::Rect::from_center_size(center, egui::Vec2::splat(GLYPH_SIZE)),
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            color,
        );
    }
    hover_help(response, label)
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
    use sha2::{Digest, Sha256};

    const ALL: [Icon; 14] = [
        Icon::Add,
        Icon::NewChat,
        Icon::History,
        Icon::Copy,
        Icon::Check,
        Icon::Fork,
        Icon::Retry,
        Icon::Edit,
        Icon::Close,
        Icon::Settings,
        Icon::Pin,
        Icon::Send,
        Icon::Stop,
        Icon::Expand,
    ];

    fn hint_frame(
        ctx: &egui::Context,
        time: f64,
        events: Vec<egui::Event>,
        enabled: bool,
        anchor: egui::Pos2,
        icon: Icon,
        label: &str,
    ) -> (egui::FullOutput, egui::Rect, bool) {
        let mut rect = egui::Rect::NOTHING;
        let mut clicked = false;
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(320.0, 220.0),
                )),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let bounds = egui::Rect::from_min_size(anchor, egui::vec2(28.0, 28.0));
                    let mut child = ui.new_child(
                        egui::UiBuilder::new()
                            .id_salt("hint-fixture")
                            .max_rect(bounds),
                    );
                    child.set_clip_rect(bounds);
                    let response = child
                        .add_enabled_ui(enabled, |ui| button(ui, icon, false, label))
                        .inner;
                    rect = response.rect;
                    clicked = response.clicked();
                });
            },
        );
        (output, rect, clicked)
    }

    fn has_help(output: &egui::FullOutput, label: &str) -> bool {
        output.shapes.iter().any(|shape| {
            matches!(&shape.shape,
            egui::Shape::Text(text) if text.galley.job.text == label)
        })
    }

    #[test]
    fn every_official_control_shows_hover_help_even_when_disabled_and_pointer_jitters() {
        let label = "功能說明 · Ctrl+N";
        for appearance in ["light", "dark"] {
            for enabled in [true, false] {
                for icon in ALL {
                    let ctx = egui::Context::default();
                    super::super::theme::apply(
                        &ctx,
                        appearance,
                        super::super::theme::DEFAULT_ACCENT_HEX,
                    );
                    let anchor = egui::pos2(80.0, 70.0);
                    let (_, rect, _) = hint_frame(&ctx, 0.0, vec![], enabled, anchor, icon, label);
                    let pos = rect.center();
                    let (early, _, _) = hint_frame(
                        &ctx,
                        0.1,
                        vec![egui::Event::PointerMoved(pos)],
                        enabled,
                        anchor,
                        icon,
                        label,
                    );
                    assert!(!has_help(&early, label));
                    let _ = hint_frame(
                        &ctx,
                        0.2,
                        vec![egui::Event::PointerMoved(pos + egui::vec2(1.0, 1.0))],
                        enabled,
                        anchor,
                        icon,
                        label,
                    );
                    let _ = hint_frame(
                        &ctx,
                        0.31,
                        vec![egui::Event::PointerMoved(pos - egui::vec2(1.0, 1.0))],
                        enabled,
                        anchor,
                        icon,
                        label,
                    );
                    let (shown, after, clicked) =
                        hint_frame(&ctx, 0.35, vec![], enabled, anchor, icon, label);
                    assert!(
                        has_help(&shown, label),
                        "missing {appearance}/{enabled}/{icon:?} hint"
                    );
                    assert_eq!(
                        after, rect,
                        "tooltip must not affect the real hit target/layout"
                    );
                    assert!(!clicked);
                    assert!(
                        shown.platform_output.commands.is_empty(),
                        "hover is not an action"
                    );
                    let (left, _, _) = hint_frame(
                        &ctx,
                        0.4,
                        vec![egui::Event::PointerMoved(egui::pos2(5.0, 5.0))],
                        enabled,
                        anchor,
                        icon,
                        label,
                    );
                    assert!(!has_help(&left, label));
                }
            }
        }
    }

    #[test]
    fn help_escapes_local_body_clip_but_fits_the_native_viewport_at_every_corner() {
        let label = "複製整張表格（Markdown） · Ctrl+Shift+G";
        for anchor in [
            egui::pos2(8.0, 8.0),
            egui::pos2(284.0, 8.0),
            egui::pos2(8.0, 184.0),
            egui::pos2(284.0, 184.0),
        ] {
            let ctx = egui::Context::default();
            let (_, rect, _) = hint_frame(&ctx, 0.0, vec![], true, anchor, Icon::Copy, label);
            let _ = hint_frame(
                &ctx,
                0.1,
                vec![egui::Event::PointerMoved(rect.center())],
                true,
                anchor,
                Icon::Copy,
                label,
            );
            let _ = hint_frame(&ctx, 0.31, vec![], true, anchor, Icon::Copy, label);
            let (output, _, _) = hint_frame(&ctx, 0.36, vec![], true, anchor, Icon::Copy, label);
            let (shape, text) = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.job.text == label => Some((shape, text)),
                    _ => None,
                })
                .expect("tooltip should paint at all native viewport corners");
            let bounds = egui::Rect::from_min_size(text.pos, text.galley.size());
            assert!(
                shape.clip_rect.contains_rect(bounds),
                "tooltip clipped by its old 28pt body: {bounds:?}"
            );
            assert!(
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(320.5, 220.5))
                    .contains_rect(bounds),
                "tooltip outside native viewport: {bounds:?}"
            );
        }
    }

    #[test]
    fn press_hides_help_clicks_once_and_disabled_controls_remain_inert() {
        let label = "設定 · Ctrl+,";
        for enabled in [true, false] {
            let ctx = egui::Context::default();
            let anchor = egui::pos2(80.0, 70.0);
            let (_, rect, _) =
                hint_frame(&ctx, 0.0, vec![], enabled, anchor, Icon::Settings, label);
            let pos = rect.center();
            let _ = hint_frame(
                &ctx,
                0.1,
                vec![egui::Event::PointerMoved(pos)],
                enabled,
                anchor,
                Icon::Settings,
                label,
            );
            let _ = hint_frame(&ctx, 0.31, vec![], enabled, anchor, Icon::Settings, label);
            let (shown, _, _) =
                hint_frame(&ctx, 0.35, vec![], enabled, anchor, Icon::Settings, label);
            assert!(has_help(&shown, label));
            let pointer = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            let (pressed, _, down_click) = hint_frame(
                &ctx,
                0.4,
                vec![pointer(true)],
                enabled,
                anchor,
                Icon::Settings,
                label,
            );
            assert!(!has_help(&pressed, label));
            let (_, _, up_click) = hint_frame(
                &ctx,
                0.5,
                vec![pointer(false)],
                enabled,
                anchor,
                Icon::Settings,
                label,
            );
            assert_eq!(
                usize::from(down_click) + usize::from(up_click),
                usize::from(enabled)
            );
            let (left, _, _) = hint_frame(
                &ctx,
                0.6,
                vec![egui::Event::PointerGone],
                enabled,
                anchor,
                Icon::Settings,
                label,
            );
            assert!(!has_help(&left, label));
        }
    }

    #[test]
    fn stable_tooltip_does_not_keep_repainting_when_nothing_changes() {
        let ctx = egui::Context::default();
        let anchor = egui::pos2(80.0, 70.0);
        let label = "複製訊息";
        let (_, rect, _) = hint_frame(&ctx, 0.0, vec![], true, anchor, Icon::Copy, label);
        let _ = hint_frame(
            &ctx,
            0.1,
            vec![egui::Event::PointerMoved(rect.center())],
            true,
            anchor,
            Icon::Copy,
            label,
        );
        for time in [0.31, 0.4, 0.8, 1.0] {
            let _ = hint_frame(&ctx, time, vec![], true, anchor, Icon::Copy, label);
        }
        let (output, _, _) = hint_frame(&ctx, 1.1, vec![], true, anchor, Icon::Copy, label);
        assert!(has_help(&output, label));
        assert!(
            output.viewport_output[&egui::ViewportId::ROOT].repaint_delay
                > std::time::Duration::from_millis(100),
            "a visible, settled help popup must not start an idle render loop"
        );
    }

    #[test]
    fn official_assets_and_license_match_pinned_release() {
        // SHA-256 of upstream bytes at the documented commit (normalize only
        // checkout CRLF in the license, for Windows core.autocrlf worktrees).
        let expected = [
            "8c1c05a38a072f42156542c6f3c6f91f0188e7bf27990f81ff42f42e0719c7e5",
            "ae484ba5087f08ba85b894407e07673d16b8631be22fcc4deffa3818c330b07d",
            "fd68973d039b960835d942bf7f43b3f509ee4daf5d4067d8d9ddead8efd8716b",
            "d983dbfe6de7e2c4750267fb35feb0c843b74b3fa2e8e3f8d3a2dce944bd2b85",
            "238257f08a1167f608209d7ef44fa3eaa59859811a2cedeab34cd9b63667e410",
            "80afc17c28937946484aa92a94c7e045a8333d5d4b4658e0c93d88fec616d496",
            "4d323caa7b0621d7d9096e2b994f5b93ae29a377635fca0742630e046401bc50",
            "e5f608b0a297d4b113dbbe846450a0e5ace73643ee564bdaed8e5ae7305a247d",
            "591fea2ae19d968cac870da11a403068f083a84bb909ee248e11353ed319ec8d",
            "b9a38444d7af356e89bb9b2d11852847f969e8b9a45981ca0a1387477247ae81",
            "088d40be7f75784a596ad63d78ca5f1d3ae5ea1e866b533320389599de37a7ba",
            "7520187e5d6be7145b9a45b776b4beb4a061c96dbac77397dbc858182f1a76be",
            "d4a65fb72daa0ccb69bc2c639ac526319a57467d9405666da92a2927b079d00a",
            "f42ecc9d1935979b16001e95400302543a96617099314d1fe1e68c6bcd3757db",
        ];
        for (icon, hash) in ALL.into_iter().zip(expected) {
            let svg = icon.svg();
            assert_eq!(
                format!("{:x}", Sha256::digest(svg.as_bytes())),
                hash,
                "{icon:?}"
            );
            assert!(svg.contains("Font Awesome Free 6.7.2 by @fontawesome"));
            assert!(svg.contains("Icons: CC BY 4.0"));
            assert!(!svg.contains("<text"));
            assert!(!svg.contains("<image"));
            assert!(!svg.contains("<use"));
        }
        let license = include_str!("../../assets/font-awesome/LICENSE.txt").replace("\r\n", "\n");
        assert_eq!(
            format!("{:x}", Sha256::digest(license.as_bytes())),
            "9b914eae88817d63b576eab5aafde7068c7a1abae125d7cdfb034f1da43a9afc"
        );
        let readme = include_str!("../../assets/font-awesome/README.md");
        assert!(readme.contains("af620534bfc3c2d4cbefcfeec29603bbe7809e64"));
        assert!(include_str!("../../assets/font-awesome/NOTICE.md").contains("CC BY 4.0"));
    }

    #[test]
    fn every_icon_rasterizes_as_nonempty_white_mask_at_every_resolution() {
        for icon in ALL {
            for size in RASTER_SIZES {
                let image = rasterize(icon, size);
                assert_eq!(image.size, [size as usize; 2]);
                assert!(
                    image.pixels.iter().any(|pixel| pixel.a() != 0),
                    "{icon:?}/{size}"
                );
                for pixel in image.pixels {
                    assert_eq!(pixel.to_array(), [pixel.a(); 4], "white premultiplied mask");
                }
            }
        }
    }

    #[test]
    fn texture_cache_reuses_masks_and_is_bounded_across_palettes_and_scales() {
        let ctx = egui::Context::default();
        for appearance in ["light", "dark"] {
            for accent in ["#0055FF", "#DD2233", "#00CC88"] {
                super::super::theme::apply(&ctx, appearance, accent);
                for step in 1..=1024 {
                    let scale = step as f32 / 100.0;
                    for icon in ALL {
                        let first = texture(&ctx, icon, scale);
                        assert_eq!(first.id(), texture(&ctx, icon, scale).id());
                        assert_eq!(first.size(), [raster_size(scale) as usize; 2]);
                    }
                }
            }
        }
        let cache = cache(&ctx);
        let cache = cache.lock().unwrap();
        assert_eq!(cache.textures.len(), ALL.len() * RASTER_SIZES.len());
        let bytes: usize = cache
            .textures
            .values()
            .map(|texture| {
                let [width, height] = texture.size();
                width * height * 4
            })
            .sum();
        assert_eq!(bytes, 1_204_224);
        let other_context = egui::Context::default();
        assert!(super::cache(&other_context)
            .lock()
            .unwrap()
            .textures
            .is_empty());
    }

    #[test]
    fn composer_and_header_icons_have_equal_square_hit_targets() {
        for appearance in ["dark", "light"] {
            let ctx = egui::Context::default();
            ctx.set_fonts(egui::FontDefinitions::empty());
            super::super::theme::apply(&ctx, appearance, super::super::theme::DEFAULT_ACCENT_HEX);
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1000.0, 600.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        for enabled in [true, false] {
                            for active in [true, false] {
                                ui.add_enabled_ui(enabled, |ui| {
                                    ui.horizontal(|ui| {
                                        for icon in ALL {
                                            let response = button(ui, icon, active, "test control");
                                            assert_eq!(
                                                response.rect.size(),
                                                egui::vec2(28.0, 28.0)
                                            );
                                            assert_eq!(response.enabled(), enabled);
                                        }
                                    });
                                });
                            }
                        }
                    });
                },
            );
            assert!(!output
                .shapes
                .iter()
                .any(|shape| matches!(shape.shape, egui::Shape::Text(_))));
            let glyphs = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Mesh(mesh) => Some(mesh),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(
                glyphs.len(),
                ALL.len() * 4,
                "every button paints a font-free image"
            );
            for glyph in glyphs {
                assert_eq!(glyph.calc_bounds().size(), egui::Vec2::splat(GLYPH_SIZE));
            }
        }
    }
}
