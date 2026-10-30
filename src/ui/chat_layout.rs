//! Bottom-first chat layout, without guessing the composer's height.
//!
//! Draw header/error rows first, then [`bottom_panel`], then the transcript in
//! [`show_body`] (or use [`body_rect`] for an existing transcript renderer).

use egui::{Align, Frame, Id, InnerResponse, Layout, Rect, ScrollArea, Sense, Ui, UiBuilder};

#[derive(Clone, Copy)]
struct Measurement {
    natural_height: f32,
    overflowing: bool,
    at_bottom: bool,
}

/// Reserve the measured natural height of top-down composer/footer contents.
///
/// Call immediately after header/error rows and BEFORE drawing the transcript.
/// No margins, separator, resize handle, or fixed composer-height estimate are
/// added. The remaining [`body_rect`] is the transcript's entire viewport.
///
/// Content exceeding the available height is bounded and scrollable, initially
/// aligned to its bottom so the footer stays reachable. The scroll container's
/// identity is stable, but its input is disabled unless content overflows;
/// dragging never competes with text selection in the composer.
///
/// `contents` runs exactly once per egui pass, not once per `Context::run`:
/// measurement changes request an egui discard/re-layout. Hosts must fence
/// non-idempotent frame work (file drops, paste jobs, etc.) outside this closure
/// using egui's cumulative frame number, as with other multi-pass egui UIs.
pub fn bottom_panel<R>(
    ui: &mut Ui,
    id: Id,
    contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    let measurement_id = id.with("natural_height");
    let previous = ui.data_mut(|data| data.get_temp::<Measurement>(measurement_id));
    let available_height = ui.available_height().max(0.0);
    let height = previous
        .map_or(available_height, |measurement| measurement.natural_height)
        .min(available_height);
    let overflowing = previous.is_some_and(|m| m.natural_height > available_height + 0.5);
    let outer_clip = ui.clip_rect();

    egui::TopBottomPanel::bottom(id)
        .frame(Frame::NONE)
        .show_separator_line(false)
        .resizable(false)
        .exact_height(height)
        .show_inside(ui, |ui| {
            // Keep the content's min_rect isolated from the panel's min_rect.
            // Even an oversized child widget must not enlarge the card frame.
            let rect = ui.max_rect();
            let mut bounded = ui.new_child(
                UiBuilder::new()
                    .id_salt("bounded_contents")
                    .max_rect(rect)
                    .layout(Layout::top_down(Align::Min)),
            );
            bounded.set_clip_rect(rect.intersect(outer_clip));
            // A floating overflow bar must not change the width (and hence the
            // measured wrapping) when overflow is first detected.
            bounded.spacing_mut().scroll.floating = true;
            bounded.spacing_mut().scroll.floating_allocated_width = 0.0;
            let mut scroll = ScrollArea::vertical()
                .id_salt(id.with("overflow"))
                .max_height(rect.height().max(0.0))
                .min_scrolled_height(0.0)
                .auto_shrink([false, false])
                .animated(false)
                .stick_to_bottom(true)
                .scroll_source(egui::scroll_area::ScrollSource {
                    drag: false,
                    mouse_wheel: overflowing,
                    scroll_bar: overflowing,
                })
                .scroll_bar_visibility(if overflowing {
                    egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded
                } else {
                    egui::scroll_area::ScrollBarVisibility::AlwaysHidden
                });
            if !overflowing {
                scroll = scroll.vertical_scroll_offset(0.0);
            } else if previous.is_some_and(|m| !m.overflowing || m.at_bottom) {
                scroll = scroll.vertical_scroll_offset(
                    (previous.unwrap().natural_height - rect.height()).max(0.0),
                );
            }
            let result = scroll.show(&mut bounded, contents);
            let natural_height = result.content_size.y.ceil().max(0.0);
            ui.data_mut(|data| {
                data.insert_temp(
                    measurement_id,
                    Measurement {
                        natural_height,
                        overflowing,
                        at_bottom: result.state.offset.y + 0.5
                            >= (result.content_size.y - result.inner_rect.height()).max(0.0),
                    },
                );
            });
            if previous.is_none_or(|m| (m.natural_height - natural_height).abs() > 0.5)
                || (height - natural_height.min(available_height)).abs() > 0.5
            {
                ui.ctx()
                    .request_discard("chat bottom panel measured height changed");
            }
            result.inner
        })
}

/// The non-negative transcript viewport remaining after [`bottom_panel`].
pub fn body_rect(ui: &Ui) -> Rect {
    let rect = ui.available_rect_before_wrap().intersect(ui.max_rect());
    Rect::from_min_max(rect.min, rect.max.max(rect.min))
}

/// Draw a transcript without allowing its minimum size to enlarge the card.
///
/// A transcript `ScrollArea` inside this closure should also use
/// `.min_scrolled_height(0.0)` and `.auto_shrink([false, false])`.
pub fn show_body<R>(ui: &mut Ui, id: Id, contents: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    let rect = body_rect(ui);
    let mut body = ui.new_child(
        UiBuilder::new()
            .id_salt(id)
            .max_rect(rect)
            .layout(Layout::top_down(Align::Min)),
    );
    body.set_clip_rect(rect.intersect(ui.clip_rect()));
    let inner = contents(&mut body);
    InnerResponse::new(inner, ui.allocate_rect(rect, Sense::hover()))
}

/// Keep frame strokes, negative glyph bearings and toolbar hover circles away
/// from the scrolling viewport's hard clip edge. Do not expand the clip itself.
pub fn transcript_contents<R>(
    ui: &mut Ui,
    contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    Frame::NONE
        .inner_margin(egui::Margin::symmetric(6, 2))
        .show(ui, contents)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::commands::ShortcutStore;
    use crate::chat::store::{Attachment, AttachmentContent, PromptHistory};
    use crate::ui::{composer::Composer, controls, surface, theme};

    #[derive(Clone, Copy, Debug)]
    struct Geometry {
        card: Rect,
        bottom: Rect,
        composer: Rect,
        body: Rect,
        clip: Rect,
        transcript: Rect,
        natural_height: f32,
        overflowing: bool,
    }

    fn contains(outer: Rect, inner: Rect) -> bool {
        outer.expand(0.6).contains_rect(inner)
    }

    fn draw(
        ctx: &egui::Context,
        size: egui::Vec2,
        composer: &mut Composer,
        find: bool,
        loading: bool,
    ) -> (Geometry, usize) {
        let mut geometry = None;
        let mut passes = 0;
        let mut calls = 0;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, size)),
                ..Default::default()
            },
            |ctx| {
                passes += 1;
                egui::CentralPanel::default()
                    .frame(surface::canvas_frame())
                    .show(ctx, |ui| {
                        let card = surface::card_frame(ui.visuals().dark_mode).show(ui, |ui| {
                            ui.set_min_size(ui.available_size());
                            ui.horizontal(|ui| {
                                ui.label("PopChat");
                                let _ = controls::button(ui, "History");
                            });
                            let bottom = bottom_panel(ui, Id::new("test_bottom"), |ui| {
                                calls += 1;
                                if find {
                                    ui.allocate_exact_size(
                                        egui::vec2(ui.available_width(), 38.0),
                                        Sense::hover(),
                                    );
                                }
                                ui.add_space(10.0);
                                if loading {
                                    ui.horizontal(|ui| {
                                        ui.set_min_height(24.0);
                                        ui.spinner();
                                        ui.label("Loading attachment");
                                    });
                                }
                                let composer = Frame::new()
                                    .stroke(egui::Stroke::new(1.0_f32, egui::Color32::GRAY))
                                    .inner_margin(egui::Margin::symmetric(16, 12))
                                    .show(ui, |ui| {
                                        composer.show(
                                            ui,
                                            &ShortcutStore::default(),
                                            &mut PromptHistory::default(),
                                        )
                                    })
                                    .response
                                    .rect;
                                composer
                            });
                            let body = body_rect(ui);
                            let transcript = show_body(ui, Id::new("test_body"), |ui| {
                                let clip = ui.clip_rect();
                                let scroll = ScrollArea::vertical()
                                    .id_salt("transcript")
                                    .min_scrolled_height(0.0)
                                    .max_height(body.height())
                                    .auto_shrink([false, false])
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            ui.label("Assistant / model");
                                            let response = controls::button(ui, "Copy");
                                            assert!(response.rect.height() >= 26.0);
                                            let _ = controls::button(ui, "Fork");
                                        });
                                        ui.label("Answer with a source and message controls");
                                        ui.hyperlink_to("Source", "https://example.invalid/");
                                        ui.allocate_space(egui::vec2(100.0, 900.0));
                                    });
                                (clip, scroll.inner_rect)
                            });
                            let measurement = ui.data_mut(|data| {
                                data.get_temp::<Measurement>(
                                    Id::new("test_bottom").with("natural_height"),
                                )
                                .unwrap()
                            });
                            Geometry {
                                card: Rect::NOTHING,
                                bottom: bottom.response.rect,
                                composer: bottom.inner,
                                body,
                                clip: transcript.inner.0,
                                transcript: transcript.inner.1,
                                natural_height: measurement.natural_height,
                                overflowing: measurement.overflowing,
                            }
                        });
                        let mut result = card.inner;
                        result.card = card.response.rect;
                        geometry = Some(result);
                    });
            },
        );
        assert_eq!(
            calls, passes,
            "the helper must not invoke content twice in a pass"
        );
        (geometry.unwrap(), passes)
    }

    #[test]
    fn transcript_inset_keeps_left_component_edges_away_from_scroll_clipping() {
        use crate::ui::panel::MathTextures;
        for appearance in ["light", "dark"] {
            for offset in [0.0_f32, 100000.0] {
                let ctx = egui::Context::default();
                theme::apply(&ctx, appearance, theme::DEFAULT_ACCENT_HEX);
                let mut math = MathTextures::new();
                let mut viewport = Rect::NOTHING;
                let mut maximum_offset = 0.0_f32;
                let mut output = None;
                for _ in 0..3 {
                    output = Some(ctx.run(egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(553.0, 594.0))),
                        ..Default::default()
                    }, |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            let scroll = ScrollArea::vertical().max_height(340.0).auto_shrink([false; 2]).animated(false)
                                .vertical_scroll_offset(offset.min(maximum_offset)).show(ui, |ui| {
                                    transcript_contents(ui, |ui| {
                                        let pal = theme::current_palette(ctx);
                                        let width = ui.available_width();
                                        ui.with_layout(theme::bubble_layout(true), |ui| {
                                            ui.set_max_width(width * 0.85);
                                            theme::bubble_frame(true, pal, pal.dark).show(ui, |ui| {
                                                ui.add(egui::Label::new("unbreakable_html_class".repeat(12)).wrap());
                                            });
                                        });
                                        math.show_assistant_message(ui, "Visible prose\n\n```css\n.component::before {\n  display: none;\n}\n```\n\nLast visible paragraph.");
                                        ui.horizontal(|ui| { controls::message_actions_inline(ui, true, true, true); });
                                    });
                                });
                            viewport = scroll.inner_rect;
                            maximum_offset = (scroll.content_size.y - viewport.height()).max(0.0);
                        });
                    }));
                }
                let output = output.unwrap();
                let mut found_code = false;
                let mut found_icons = false;
                fn leaves<'a>(shape: &'a egui::Shape, all: &mut Vec<&'a egui::Shape>) {
                    if let egui::Shape::Vec(children) = shape {
                        for child in children {
                            leaves(child, all);
                        }
                    } else {
                        all.push(shape);
                    }
                }
                let mut shapes = Vec::new();
                for clipped in &output.shapes {
                    leaves(&clipped.shape, &mut shapes);
                }
                for shape in &shapes {
                    match shape {
                        egui::Shape::Rect(rect)
                            if rect.fill == theme::code_bg_for(appearance == "dark")
                                && rect.corner_radius == egui::CornerRadius::same(10) =>
                        {
                            found_code = true;
                            assert!(
                                rect.rect.left() >= viewport.left() + 5.0,
                                "code frame's left edge touches scroll clip"
                            );
                            assert!(rect.rect.right() <= viewport.right() - 5.0);
                        }
                        egui::Shape::Circle(circle) if circle.radius == 14.0 => {
                            found_icons = true;
                            assert!(
                                circle.center.x - circle.radius >= viewport.left() + 5.0,
                                "toolbar circle clipped at left"
                            );
                        }
                        egui::Shape::Text(text)
                            if text.galley.text() == "css"
                                || text.galley.text().contains("Last visible paragraph") =>
                        {
                            let bounds = text.galley.mesh_bounds.translate(text.pos.to_vec2());
                            assert!(
                                bounds.left() >= viewport.left() + 4.0,
                                "first glyph bearing crosses clip"
                            );
                        }
                        _ => {}
                    }
                }
                assert!(
                    found_code,
                    "{appearance} offset={offset}: missing code frame"
                );
                // At the top the toolbar lies below this 340pt viewport; culling
                // it is correct. At the bottom it must actually be painted.
                if offset > 0.0 {
                    assert!(found_icons, "{appearance}: missing bottom toolbar");
                }
            }
        }
    }

    #[test]
    fn long_user_html_must_wrap_to_keep_transcript_glyph_meshes_visible() {
        use crate::ui::{fonts, panel::MathTextures};
        let ctx = egui::Context::default();
        theme::apply(&ctx, "Light", theme::DEFAULT_ACCENT_HEX);
        let mut composer = Composer::new();
        let mut math = MathTextures::new();
        let selected_fonts = fonts::FontPreferences {
            english: r"system-font-v1:0:\\?\C:\Windows\Fonts\TT Norms Pro Normal.ttf".into(),
            chinese: r"system-font-v1:1:\\?\C:\Windows\Fonts\YuGothR.ttc".into(),
        };
        let user = format!("User ASCII text\n使用者中文訊息\n<div class=\"{}\">\nhttps://example.invalid/{}\nA second line to wrap", "unbreakableclass".repeat(60), "pathsegment".repeat(60));
        const FIRST: &str = "let answer = 42; // 中文";
        const LAST: &str = "print('last visible code 中文')";
        let answer = format!("Assistant ASCII 中文訊息\n\n```rust\n{FIRST}\n{}\n```\n\nA paragraph between code frames.\n\n```python\n{LAST}\n```\n\nFinal assistant 中文 text.", "long_code_token".repeat(60));
        let defaults = fonts::FontPreferences::default();
        // The layout regression always runs with the normal fonts. Also exercise
        // the original reported choices when those installed faces are available,
        // without requiring a proprietary TT Norms installation on every host.
        let choices = if std::path::Path::new(r"C:\Windows\Fonts\TT Norms Pro Normal.ttf").is_file()
            && std::path::Path::new(r"C:\Windows\Fonts\YuGothR.ttc").is_file()
        {
            vec![defaults.clone(), selected_fonts, defaults]
        } else {
            vec![defaults]
        };
        let mut max_offset = 0.0_f32;
        for wrap_user in [false, true] {
            for prefs in &choices {
                fonts::apply_font_preferences(&ctx, prefs).unwrap();
                for (size, rows) in [
                    (egui::vec2(553.0, 594.0), 1),
                    (egui::vec2(553.0, 594.0), 4),
                    (egui::vec2(520.0, 344.0), 4),
                    (egui::vec2(604.0, 644.0), 1),
                ] {
                    composer.set_text(vec!["draft 中文"; rows].join("\n"));
                    for offset in [0.0_f32, 100000.0] {
                        // The first frame can reflect the preceding scroll height;
                        // inspect the settled frame as production egui does.
                        let mut final_output = None;
                        let mut viewport = Rect::NOTHING;
                        let mut observed_offset = 0.0;
                        let mut content_size = egui::Vec2::ZERO;
                        let mut user_rect = Rect::NOTHING;
                        for _ in 0..3 {
                            final_output = Some(ctx.run(egui::RawInput {
                            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, size)),
                            ..Default::default()
                        }, |ctx| {
                            egui::CentralPanel::default().frame(surface::canvas_frame()).show(ctx, |ui| {
                                surface::card_frame(false).show(ui, |ui| {
                                    ui.set_min_size(ui.available_size());
                                    ui.horizontal(|ui| { ui.label("PopChat header"); ui.label("model"); });
                                    ui.add_space(10.0);
                                    bottom_panel(ui, Id::new("diagnostic-bottom"), |ui| {
                                        ui.add_space(10.0);
                                        Frame::new().inner_margin(egui::Margin::symmetric(16, 12)).show(ui, |ui| {
                                            composer.show(ui, &ShortcutStore::default(), &mut PromptHistory::default());
                                        });
                                    });
                                    show_body(ui, Id::new("diagnostic-body"), |ui| {
                                        let height = ui.available_height().max(0.0);
                                        let scroll = ScrollArea::vertical()
                                            .max_height(height).min_scrolled_height(0.0)
                                            .auto_shrink([false; 2]).animated(false)
                                            .vertical_scroll_offset(offset.min(max_offset))
                                            .show(ui, |ui| {
                                                let pal = theme::current_palette(ui.ctx());
                                                let width = ui.available_width();
                                                ui.with_layout(theme::bubble_layout(true), |ui| {
                                                    ui.set_max_width(width * 0.85);
                                                    theme::bubble_frame(true, pal, false).show(ui, |ui| {
                                                        let label = egui::Label::new(egui::RichText::new(&user).color(pal.user_text));
                                                        user_rect = ui.add(if wrap_user { label.wrap() } else { label }).rect;
                                                    });
                                                });
                                                ui.add_space(16.0);
                                                ui.scope(|ui| {
                                                    ui.set_max_width(width - 6.0);
                                                    ui.spacing_mut().interact_size.y = 18.0;
                                                    ui.horizontal(|ui| { ui.label(egui::RichText::new("PopChat").size(12.0).weak()); });
                                                    ui.add_space(9.0);
                                                    math.show_assistant_message(ui, &answer);
                                                });
                                                ui.add_space(22.0);
                                                ui.horizontal(|ui| { let _ = controls::button(ui, "複製"); });
                                            });
                                        viewport = scroll.inner_rect;
                                        observed_offset = scroll.state.offset.y;
                                        content_size = scroll.content_size;
                                        max_offset = (scroll.content_size.y - scroll.inner_rect.height()).max(0.0);
                                    });
                                });
                            });
                        }));
                        }
                        let output = final_output.unwrap();
                        let mut seen = Vec::new();
                        let mut visible = Vec::new();
                        for clipped in &output.shapes {
                            if let egui::Shape::Text(text) = &clipped.shape {
                                let value = text.galley.text();
                                seen.push(value.to_owned());
                                // RTL galleys have negative local x coordinates;
                                // pos + size is not their actual painted rectangle.
                                let rect = text.galley.mesh_bounds.translate(text.pos.to_vec2());
                                if !rect.intersects(viewport.intersect(clipped.clip_rect)) {
                                    continue;
                                }
                                // A galley/string alone is insufficient: prove at least
                                // one nontransparent glyph triangle reaches the clip.
                                let meshes =
                                    ctx.tessellate(vec![clipped.clone()], output.pixels_per_point);
                                let has_ink = meshes.iter().any(|primitive| {
                                    let egui::epaint::Primitive::Mesh(mesh) = &primitive.primitive
                                    else {
                                        return false;
                                    };
                                    mesh.indices.chunks_exact(3).any(|triangle| {
                                        let vertices = [triangle[0], triangle[1], triangle[2]]
                                            .map(|index| mesh.vertices[index as usize]);
                                        let bounds =
                                            Rect::from_points(&vertices.map(|vertex| vertex.pos));
                                        vertices.iter().all(|vertex| vertex.color.a() > 0)
                                            && vertices
                                                .iter()
                                                .any(|vertex| vertex.uv != egui::epaint::WHITE_UV)
                                            && bounds
                                                .intersects(primitive.clip_rect.intersect(viewport))
                                            && bounds.area() > 0.0
                                    })
                                });
                                if has_ink {
                                    visible.push(value.to_owned());
                                }
                            }
                        }
                        assert!(viewport.height() > 0.0);
                        if !wrap_user {
                            assert!(content_size.x > viewport.width() * 4.0);
                            assert!(user_rect.left() < viewport.left() - 1000.0);
                            if offset != 0.0 {
                                assert!(!visible
                                    .iter()
                                    .any(|value| value.contains("Final assistant")));
                            }
                            continue;
                        }
                        assert!(content_size.x <= viewport.width() + 0.6, "wrapped transcript still expands horizontally: content={content_size:?} viewport={viewport:?} offset={observed_offset}");
                        assert!(user_rect.left() >= viewport.left() - 0.6);
                        if offset == 0.0 {
                            assert!(
                                visible.iter().any(|value| value == &user),
                                "missing visible user: {seen:?}"
                            );
                        } else {
                            assert!(
                                visible
                                    .iter()
                                    .any(|value| value.contains("Final assistant")),
                                "missing bottom text: {seen:?}"
                            );
                            if viewport.height() > 200.0 {
                                assert!(visible.iter().any(|value| value == LAST));
                                assert!(visible.iter().any(|value| value == "python"));
                                assert!(visible.iter().any(|value| value == "Copy"));
                            }
                            if viewport.height() > 350.0 {
                                assert!(visible
                                    .iter()
                                    .any(|value| value.contains("Assistant ASCII")));
                                assert!(visible.iter().any(|value| value.starts_with(FIRST)));
                                assert!(visible.iter().any(|value| value == "rust"));
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn production_composer_and_transcript_stay_within_native_bounds_without_footer() {
        for dark in [false, true] {
            for large_fonts in [false, true] {
                for width in [520.0, 604.0] {
                    for height in [272.0, 344.0, 644.0] {
                        for rows in 1..=8 {
                            for extras in [false, true] {
                                let ctx = egui::Context::default();
                                theme::apply(
                                    &ctx,
                                    if dark { "dark" } else { "light" },
                                    theme::DEFAULT_ACCENT_HEX,
                                );
                                if large_fonts {
                                    ctx.style_mut(|style| {
                                        for font in style.text_styles.values_mut() {
                                            font.size *= 1.5;
                                        }
                                    });
                                }
                                let mut composer = Composer::new();
                                composer.set_text(vec!["draft"; rows].join("\n"));
                                if extras {
                                    composer.add_attachments(vec![Attachment::new(
                                        "sample.txt",
                                        AttachmentContent::Text("attachment".into()),
                                    )]);
                                    assert_eq!(composer.attachment_height(), 34.0);
                                }
                                let size = egui::vec2(width, height);
                                let (g, _) = draw(&ctx, size, &mut composer, extras, extras);
                                let viewport = Rect::from_min_size(egui::Pos2::ZERO, size);
                                assert!(contains(viewport, g.card), "{size:?} rows={rows}: {g:?}");
                                assert!(contains(g.card, g.bottom), "{g:?}");
                                assert!(contains(g.body, g.transcript), "{g:?}");
                                assert!(contains(g.body, g.clip), "{g:?}");
                                assert!(g.body.bottom() <= g.bottom.top() + 0.6, "{g:?}");
                                if !g.overflowing {
                                    assert!(contains(g.bottom, g.composer), "{g:?}");
                                    assert!(
                                        (g.bottom.height() - g.natural_height).abs() < 0.6,
                                        "{g:?}"
                                    );
                                } else {
                                    // Too-tall composers are clipped/scrollable, not allowed
                                    // to enlarge the card. Their lower edge still stays onscreen.
                                    assert!(g.composer.bottom() <= g.bottom.bottom(), "{g:?}");
                                    assert!(g.body.height() <= 0.6, "{g:?}");
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn changing_rows_find_loading_and_native_size_remeasures_without_fixed_reserve() {
        let ctx = egui::Context::default();
        theme::apply(&ctx, "dark", theme::DEFAULT_ACCENT_HEX);
        let mut composer = Composer::new();
        for (height, rows, extras) in [
            (344.0, 1, false),
            (344.0, 8, true),
            (272.0, 8, true),
            (644.0, 8, true),
            (644.0, 1, false),
            (272.0, 1, false),
        ] {
            composer.set_text(vec!["draft"; rows].join("\n"));
            let (g, passes) = draw(
                &ctx,
                egui::vec2(520.0, height),
                &mut composer,
                extras,
                extras,
            );
            assert!(passes <= 2);
            assert!(contains(g.card, g.bottom), "{g:?}");
            assert!(g.body.bottom() <= g.bottom.top() + 0.6, "{g:?}");
        }
    }
}
