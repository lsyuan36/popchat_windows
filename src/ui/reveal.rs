//! Metrics-neutral counterpart of Reveal.swift/RevealFade.paint.
//!
//! Call once, AFTER MathTextures::show_assistant_message and BEFORE find-match
//! painting, with that message body's exclusive graphics shape range. No layout,
//! font-atlas mutation, global tint, or texture modification happens here.
//!
//! Like AssistantMessageView.fade(over:), only the FINAL prose/code segment is
//! eligible; quote/table/pasteable/display-math segments intentionally stay solid.
//! Prose may be several galleys (paragraphs, links, inline-code/math boundaries).
//! We project their rendered strings into one stream, then map each galley's
//! scalar offsets into its grapheme alpha bands. This prevents restarting the
//! gradient at each widget. Source-length bands are clamped to that segment,
//! matching Swift's deliberately approximate source/rendered-text fitting.
//!
//! Bitmap math is NOT faded: an inline attachment occupies one projected slot,
//! but its texture/mesh is untouched. Text fallback math can fade as text. As in
//! the source, markdown syntax removal can slightly shift the soft gradient.
//! Decorations/backgrounds are not glyphs and retain their original colors.

use std::sync::Arc;

use egui::{epaint::TextShape, Shape};
use unicode_segmentation::UnicodeSegmentation;

pub use crate::chat::store::{RevealState, RevealStop, TextReveal};
use crate::ui::markdown::{parse_prose, FlowBlock, Inline};
use crate::ui::panel::{split_math_segments, MessageSegment};

/// Paint the body shapes on ui.layer_id(), in [start, end). `None` means the
/// current list end. The renderer's small language/title and proportional
/// Copy/“Copied” controls are excluded (code with that literal text is not).
/// If fade.trailing_skip is nonzero, those renderer-appended glyphs are read
/// from the actual trailing text shape(s); state.text still excludes the caret.
/// Paint fresh shapes once per frame; multiplying an already-painted range
/// twice would, intentionally, multiply alpha twice.
pub fn paint_reveal(ui: &egui::Ui, state: &RevealState, start: usize, end: Option<usize>) {
    if state.fade.is_empty() {
        return;
    }
    let segments = split_math_segments(&state.text);
    let Some(segment) = segments.last() else {
        return;
    };
    let source = match segment {
        MessageSegment::Prose(source)
        | MessageSegment::Code {
            content: source, ..
        } => source,
        _ => return,
    };
    let budget = source.graphemes(true).count();
    if budget == 0 {
        return;
    }
    ui.ctx().graphics_mut(|graphics| {
        let Some(list) = graphics.get_mut(ui.layer_id()) else {
            return;
        };
        let end = end.unwrap_or(list.next_idx().0).min(list.next_idx().0);
        let mut actual_strings = Vec::new();
        for clipped in list.all_entries().take(end).skip(start) {
            collect_body_strings(&clipped.shape, &mut actual_strings);
        }
        let mut projected = match segment {
            MessageSegment::Prose(_) => project_prose(source, &actual_strings),
            _ => source.clone(),
        };
        if state.fade.trailing_skip > 0 {
            let trailing: Vec<&str> = actual_strings
                .iter()
                .rev()
                .flat_map(|text| text.graphemes(true).rev())
                .take(state.fade.trailing_skip)
                .collect();
            for glyph in trailing.into_iter().rev() {
                projected.push_str(glyph);
            }
        }
        let alphas = scalar_alphas(&projected, &state.fade, budget);
        let mut cursor = projected.len();
        let mut scalar_cursor = alphas.len();
        let mut stopped = false;
        for index in (start..end).rev() {
            list.mutate_shape(egui::layers::ShapeIdx(index), |clipped| {
                visit_text_reverse(&mut clipped.shape, &mut |text| {
                    if stopped || !is_body(text) {
                        return;
                    }
                    // Only unseen whitespace/attachments may occur between
                    // adjacent rendered text shapes. Fail closed on an unknown
                    // rendering rather than fading unrelated earlier blocks.
                    if projected[..cursor].chars().all(invisible_slot) {
                        stopped = true;
                        return;
                    }
                    let visible = text.galley.text();
                    let Some(offset) = projected[..cursor].rfind(visible) else {
                        stopped = true;
                        return;
                    };
                    let next = offset + visible.len();
                    if !projected[next..cursor].chars().all(invisible_slot) {
                        stopped = true;
                        return;
                    }
                    let scalar_end = scalar_cursor - projected[next..cursor].chars().count();
                    let scalar_start = scalar_end - visible.chars().count();
                    paint_galley(text, &alphas[scalar_start..scalar_end]);
                    cursor = offset;
                    scalar_cursor = scalar_start;
                });
            });
        }
    });
}

fn invisible_slot(chr: char) -> bool {
    chr.is_whitespace() || chr == '\u{fffc}'
}

fn is_body(text: &TextShape) -> bool {
    // PopChat uses 15pt prose, >=13.65pt headings and 13pt code; headers
    // are 11.5/12pt. Keep code literally spelling “Copy” eligible, unlike
    // the proportional 13pt button beside it.
    !text.galley.is_empty()
        && text
            .galley
            .job
            .sections
            .iter()
            .any(|s| s.format.font_id.size >= 13.0)
        && !(matches!(text.galley.text(), "Copy" | "Copied")
            && text.galley.job.sections.iter().all(|s| {
                s.format.font_id.family == egui::FontFamily::Proportional
                    && s.format.font_id.size <= 13.0
            }))
}

fn collect_body_strings(shape: &Shape, strings: &mut Vec<String>) {
    match shape {
        Shape::Text(text) if is_body(text) => strings.push(text.galley.text().to_owned()),
        Shape::Vec(shapes) => {
            for shape in shapes {
                collect_body_strings(shape, strings);
            }
        }
        _ => {}
    }
}

fn visit_text_reverse(shape: &mut Shape, visit: &mut impl FnMut(&mut TextShape)) {
    match shape {
        Shape::Text(text) => visit(text),
        Shape::Vec(shapes) => {
            for shape in shapes.iter_mut().rev() {
                visit_text_reverse(shape, visit);
            }
        }
        _ => {}
    }
}

/// Same parsing as the renderer, not a second layout. Phantom separators retain
/// paragraph/hard-break distances even though egui makes separate widgets.
fn project_prose(source: &str, actual: &[String]) -> String {
    fn inlines(out: &mut String, items: &[Inline], actual: &[String]) {
        for item in items {
            match item {
                Inline::Text(text, _) | Inline::Code(text) => out.push_str(text),
                Inline::Link { label, .. } => out.push_str(label),
                Inline::Space => out.push(' '),
                Inline::HardBreak => out.push('\n'),
                Inline::Math(latex) => {
                    let fallback = format!("${latex}$");
                    if actual.contains(&fallback) {
                        out.push_str(&fallback);
                    } else {
                        out.push('\u{fffc}');
                    }
                }
            }
        }
    }
    let mut result = String::new();
    for block in parse_prose(source) {
        if !result.is_empty() {
            result.push('\n');
        }
        match block {
            FlowBlock::Heading { inlines: items, .. }
            | FlowBlock::Paragraph(items)
            | FlowBlock::Quote(items) => inlines(&mut result, &items, actual),
            FlowBlock::List(items) => {
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        result.push('\n');
                    }
                    if item.ordered {
                        result.push_str(&format!("{}. ", item.number));
                    } else {
                        result.push_str("• ");
                    }
                    inlines(&mut result, &item.inlines, actual);
                }
            }
            FlowBlock::Code { content, .. } => result.push_str(&content),
            FlowBlock::Rule => {}
        }
    }
    result
}

/// Expand one alpha per extended grapheme to egui's one-glyph-per-scalar model.
/// All combining marks / ZWJ scalars of one source character share one alpha.
fn scalar_alphas(text: &str, fade: &TextReveal, source_budget: usize) -> Vec<f32> {
    let graphemes: Vec<&str> = text.graphemes(true).collect();
    let mut alpha = vec![1.0_f32; graphemes.len()];
    let mut end = graphemes.len().saturating_sub(fade.trailing_skip);
    let mut budget = source_budget;
    for stop in &fade.stops {
        if end == 0 || budget == 0 || stop.length == 0 {
            break;
        }
        let length = stop.length.min(budget);
        budget -= length;
        let start = end.saturating_sub(length);
        alpha[start..end].fill(stop.alpha.clamp(0.0, 1.0) as f32);
        end = start;
    }
    graphemes
        .iter()
        .zip(alpha)
        .flat_map(|(g, a)| std::iter::repeat_n(a, g.chars().count()))
        .collect()
}

/// epaint 0.32 emits four consecutive vertices for every glyph whose uv_rect
/// is not empty (including italics), within glyph_vertex_range. Spaces/newlines
/// have no quad. Never infer the glyph from an overlapping logical rectangle.
fn paint_galley(text: &mut TextShape, alphas: &[f32]) {
    if alphas.iter().all(|alpha| *alpha >= 1.0) {
        return;
    }
    let override_color = text.override_text_color.take();
    let fallback = text.fallback_color;
    let galley = Arc::make_mut(&mut text.galley);
    let mut scalar = 0;
    for placed in &mut galley.rows {
        let count = placed.glyphs.len();
        let needs_edit = override_color.is_some()
            || alphas
                .get(scalar..scalar + count)
                .is_some_and(|row| row.iter().any(|a| *a < 1.0));
        if needs_edit {
            let row = Arc::make_mut(&mut placed.row);
            let mut vertex = row.visuals.glyph_vertex_range.start;
            for glyph in &row.glyphs {
                let alpha = alphas.get(scalar).copied().unwrap_or(1.0);
                scalar += 1;
                if glyph.uv_rect.is_nothing() {
                    continue;
                }
                let end = (vertex + 4).min(row.visuals.glyph_vertex_range.end);
                for v in &mut row.visuals.mesh.vertices[vertex..end] {
                    if alpha < 1.0 || override_color.is_some() {
                        // Bake overrides/placeholders BEFORE multiplying, or the
                        // tessellator would replace the faded vertex color.
                        let base = override_color.unwrap_or_else(|| {
                            if v.color == egui::Color32::PLACEHOLDER {
                                fallback
                            } else {
                                v.color
                            }
                        });
                        v.color = base.gamma_multiply(alpha);
                    }
                }
                vertex = end;
            }
        } else {
            scalar += count;
        }
        scalar += usize::from(placed.ends_with_newline);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Color32, FontId, Pos2, Rect};

    fn state(source: &str, fade: TextReveal) -> RevealState {
        RevealState {
            message_id: uuid::Uuid::new_v4(),
            text: source.to_owned(),
            fade,
        }
    }

    fn band(length: usize, alpha: f64) -> TextReveal {
        TextReveal {
            stops: vec![RevealStop { length, alpha }],
            trailing_skip: 0,
        }
    }

    fn text_shapes(shape: &Shape, out: &mut Vec<TextShape>) {
        match shape {
            Shape::Text(text) => out.push(text.clone()),
            Shape::Vec(shapes) => {
                for shape in shapes {
                    text_shapes(shape, out);
                }
            }
            _ => {}
        }
    }

    fn glyph_colors(text: &TextShape) -> Vec<Color32> {
        text.galley
            .rows
            .iter()
            .flat_map(|row| {
                row.visuals.mesh.vertices[row.visuals.glyph_vertex_range.clone()]
                    .chunks_exact(4)
                    .map(|quad| {
                        text.override_text_color.unwrap_or_else(|| {
                            if quad[0].color == Color32::PLACEHOLDER {
                                text.fallback_color
                            } else {
                                quad[0].color
                            }
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    #[test]
    fn headless_paint_changes_real_glyph_quads_not_metrics_or_shared_cache() {
        let ctx = egui::Context::default();
        let mut original = None;
        let mut original_copy = None;
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let galley = ui.fonts(|fonts| {
                    fonts.layout(
                        "ab cd\nef".to_owned(),
                        FontId::proportional(13.0),
                        Color32::from_rgba_premultiplied(80, 60, 40, 160),
                        25.0,
                    )
                });
                original = Some(galley.clone());
                original_copy = Some((*galley).clone());
                let start = ui
                    .ctx()
                    .graphics(|g| g.get(ui.layer_id()).map_or(0, |l| l.next_idx().0));
                ui.painter()
                    .galley(Pos2::new(10.0, 10.0), galley, Color32::WHITE);
                paint_reveal(
                    ui,
                    &state("```\nab cd\nef\n```", band(3, 0.25)),
                    start,
                    None,
                );
            });
        });
        let original = original.unwrap();
        let original_copy = original_copy.unwrap();
        let mut shapes = Vec::new();
        for clipped in &output.shapes {
            text_shapes(&clipped.shape, &mut shapes);
        }
        let painted = shapes
            .iter()
            .find(|s| s.galley.text() == "ab cd\nef")
            .unwrap();
        assert!(!Arc::ptr_eq(&original, &painted.galley));
        assert!(Arc::ptr_eq(&original.job, &painted.galley.job));
        assert_eq!(*original, original_copy);
        assert_eq!(original.rect, painted.galley.rect);
        assert_eq!(original.mesh_bounds, painted.galley.mesh_bounds);
        assert_eq!(original.intrinsic_size(), painted.galley.intrinsic_size());
        assert_eq!(original.num_vertices, painted.galley.num_vertices);
        assert_eq!(original.num_indices, painted.galley.num_indices);
        for (before, after) in original.rows.iter().zip(&painted.galley.rows) {
            assert_eq!(before.pos, after.pos);
            assert_eq!(before.glyphs, after.glyphs);
            assert_eq!(before.size, after.size);
            assert_eq!(before.visuals.mesh.indices, after.visuals.mesh.indices);
            for (a, b) in before
                .visuals
                .mesh
                .vertices
                .iter()
                .zip(&after.visuals.mesh.vertices)
            {
                assert_eq!(a.pos, b.pos);
                assert_eq!(a.uv, b.uv);
            }
        }
        let colors = glyph_colors(painted);
        assert!(colors.iter().any(|c| c.a() == 40));
        assert!(colors.iter().any(|c| c.a() == 160));
        let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
        assert!(primitives.iter().any(|p| match &p.primitive {
            egui::epaint::Primitive::Mesh(mesh) => mesh.vertices.iter().any(|v| v.color.a() == 40),
            _ => false,
        }));
    }

    #[test]
    fn fallback_and_override_are_resolved_before_alpha_and_caret_stays_solid() {
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                for override_color in [None, Some(Color32::from_rgb(20, 80, 120))] {
                    let galley = ui.fonts(|fonts| {
                        fonts.layout_no_wrap(
                            "abc|".into(),
                            FontId::proportional(13.0),
                            Color32::PLACEHOLDER,
                        )
                    });
                    let pristine = galley.clone();
                    let mut shape =
                        TextShape::new(Pos2::ZERO, galley, Color32::from_rgb(100, 120, 140));
                    shape.override_text_color = override_color;
                    let mut fade = band(2, 0.5);
                    fade.trailing_skip = 1;
                    paint_galley(&mut shape, &scalar_alphas("abc|", &fade, 4));
                    assert!(shape.override_text_color.is_none());
                    let colors = glyph_colors(&shape);
                    let base = override_color.unwrap_or(shape.fallback_color);
                    assert_eq!(colors[1], base.gamma_multiply(0.5));
                    assert_eq!(colors[2], base.gamma_multiply(0.5));
                    assert_eq!(colors[3], base);
                    assert!(pristine.rows.iter().all(|row| row.visuals.mesh.vertices
                        [row.visuals.glyph_vertex_range.clone()]
                    .iter()
                    .all(|v| v.color == Color32::PLACEHOLDER)));
                }
            });
        });
    }

    #[test]
    fn public_paint_excludes_renderer_appended_caret_from_source_bands() {
        let ctx = egui::Context::default();
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let start = ui
                    .ctx()
                    .graphics(|g| g.get(ui.layer_id()).map_or(0, |l| l.next_idx().0));
                ui.label(egui::RichText::new("abc▍").size(13.0));
                let mut fade = band(2, 0.5);
                fade.trailing_skip = 1;
                paint_reveal(ui, &state("abc", fade), start, None);
            });
        });
        let mut shapes = Vec::new();
        for clipped in &output.shapes {
            text_shapes(&clipped.shape, &mut shapes);
        }
        let painted = shapes.iter().find(|s| s.galley.text() == "abc▍").unwrap();
        assert_eq!(
            glyph_colors(painted)
                .iter()
                .map(|c| c.a())
                .collect::<Vec<_>>(),
            vec![255, 128, 128, 255]
        );
    }

    #[test]
    fn backward_runs_are_contiguous_across_galleys_and_never_touch_controls() {
        let ctx = egui::Context::default();
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let start = ui
                    .ctx()
                    .graphics(|g| g.get(ui.layer_id()).map_or(0, |l| l.next_idx().0));
                ui.label(egui::RichText::new("Copy").size(11.0));
                ui.label(egui::RichText::new("old").size(13.0));
                ui.label(egui::RichText::new("link").size(13.0));
                ui.label(egui::RichText::new("tail").size(13.0));
                paint_reveal(
                    ui,
                    &state(
                        "old[link](https://example.test)tail",
                        TextReveal {
                            stops: vec![
                                RevealStop {
                                    length: 4,
                                    alpha: 0.25,
                                },
                                RevealStop {
                                    length: 4,
                                    alpha: 0.75,
                                },
                            ],
                            trailing_skip: 0,
                        },
                    ),
                    start,
                    None,
                );
            });
        });
        let mut shapes = Vec::new();
        for clipped in &output.shapes {
            text_shapes(&clipped.shape, &mut shapes);
        }
        let tail = shapes.iter().find(|s| s.galley.text() == "tail").unwrap();
        let link = shapes.iter().find(|s| s.galley.text() == "link").unwrap();
        assert!(glyph_colors(tail).iter().all(|c| c.a() == 64));
        assert!(glyph_colors(link).iter().all(|c| c.a() == 191));
        for name in ["old", "Copy"] {
            assert!(
                glyph_colors(shapes.iter().find(|s| s.galley.text() == name).unwrap())
                    .iter()
                    .all(|c| c.a() == 255)
            );
        }
    }

    #[test]
    fn renderer_final_segment_scope_matches_swift_and_preserves_table_text() {
        for source in [
            "before\n\n```rust\nlet answer = 42;\n```",
            "before\n\n| Name | Value |\n| - | - |\n| alpha | beta |",
            "> quote",
            "<pasteable title=\"Copy\">\nbody\n</pasteable>",
        ] {
            let ctx = egui::Context::default();
            let mut before = Vec::new();
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0))),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let start = ui
                            .ctx()
                            .graphics(|g| g.get(ui.layer_id()).map_or(0, |l| l.next_idx().0));
                        crate::ui::panel::MathTextures::new().show_assistant_message(ui, source);
                        ui.ctx().graphics(|graphics| {
                            for clipped in graphics
                                .get(ui.layer_id())
                                .unwrap()
                                .all_entries()
                                .skip(start)
                            {
                                text_shapes(&clipped.shape, &mut before);
                            }
                        });
                        paint_reveal(ui, &state(source, band(1000, 0.5)), start, None);
                    });
                },
            );
            let mut shapes = Vec::new();
            for clipped in &output.shapes {
                text_shapes(&clipped.shape, &mut shapes);
            }
            for shape in &shapes {
                if shape.galley.text() == "let answer = 42;" {
                    let original = before
                        .iter()
                        .find(|s| s.galley.text() == shape.galley.text())
                        .unwrap();
                    assert_eq!(
                        glyph_colors(shape),
                        glyph_colors(original)
                            .iter()
                            .map(|c| c.gamma_multiply(0.5))
                            .collect::<Vec<_>>()
                    );
                } else {
                    let original = before
                        .iter()
                        .find(|s| s.pos == shape.pos && s.galley.text() == shape.galley.text())
                        .unwrap();
                    assert_eq!(
                        original.galley,
                        shape.galley,
                        "{source}: {}",
                        shape.galley.text()
                    );
                }
            }
            assert!(!shapes.is_empty());
            if source.contains("```rust") {
                assert!(shapes.iter().any(|s| s.galley.text() == "let answer = 42;"));
            }
        }
    }

    #[test]
    fn graphemes_share_alpha_and_source_budget_prevents_previous_segment_bleed() {
        let text = "old e\u{301}👩‍👩‍👧‍👦|";
        let mut fade = band(999, 0.2);
        fade.trailing_skip = 1;
        let alphas = scalar_alphas(text, &fade, 2);
        assert_eq!(&alphas[..4], &[1.0; 4]);
        assert!(alphas[4..alphas.len() - 1].iter().all(|a| *a == 0.2));
        assert_eq!(*alphas.last().unwrap(), 1.0);
    }

    #[test]
    fn real_glyphs_receive_ramp_and_sentence_group_alphas_newest_first() {
        for fade in [
            TextReveal::ramp(8),
            TextReveal::groups(&[(3, 0), (2, 175)], 350),
        ] {
            let ctx = egui::Context::default();
            let source = "abcdefgh";
            let expected = scalar_alphas(source, &fade, 8);
            let color = Color32::from_rgba_unmultiplied(100, 160, 240, 200);
            let output = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let start = ui
                        .ctx()
                        .graphics(|g| g.get(ui.layer_id()).map_or(0, |l| l.next_idx().0));
                    let galley = ui.fonts(|f| {
                        f.layout_no_wrap(source.to_owned(), FontId::proportional(13.0), color)
                    });
                    ui.painter()
                        .galley(Pos2::new(10.0, 10.0), galley, Color32::WHITE);
                    paint_reveal(ui, &state(source, fade.clone()), start, None);
                });
            });
            let mut shapes = Vec::new();
            for clipped in &output.shapes {
                text_shapes(&clipped.shape, &mut shapes);
            }
            let shape = shapes.iter().find(|s| s.galley.text() == source).unwrap();
            assert_eq!(
                glyph_colors(shape),
                expected
                    .into_iter()
                    .map(|a| color.gamma_multiply(a))
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn bitmap_math_mesh_is_untouched_between_fading_prose_galleys() {
        let ctx = egui::Context::default();
        let mut bitmap = egui::Mesh::with_texture(egui::TextureId::User(42));
        bitmap.add_rect_with_uv(
            Rect::from_min_size(Pos2::new(60.0, 10.0), egui::vec2(12.0, 12.0)),
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::from_rgb(80, 120, 160),
        );
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let start = ui
                    .ctx()
                    .graphics(|g| g.get(ui.layer_id()).map_or(0, |l| l.next_idx().0));
                for (index, word) in ["before ", " after"].into_iter().enumerate() {
                    if index == 1 {
                        ui.painter().add(Shape::mesh(bitmap.clone()));
                    }
                    let galley = ui.fonts(|f| {
                        f.layout_no_wrap(
                            word.to_owned(),
                            FontId::proportional(13.0),
                            Color32::WHITE,
                        )
                    });
                    ui.painter().galley(
                        Pos2::new(10.0 + index as f32 * 80.0, 10.0),
                        galley,
                        Color32::WHITE,
                    );
                }
                paint_reveal(ui, &state("before $x$ after", band(1000, 0.5)), start, None);
            });
        });
        assert!(output.shapes.iter().any(|clipped| match &clipped.shape {
            Shape::Mesh(mesh) => mesh.as_ref() == &bitmap,
            _ => false,
        }));
        let mut shapes = Vec::new();
        for clipped in &output.shapes {
            text_shapes(&clipped.shape, &mut shapes);
        }
        assert_eq!(shapes.len(), 2);
        assert!(shapes
            .iter()
            .all(|s| glyph_colors(s).iter().all(|c| c.a() == 128)));
    }

    #[test]
    fn actual_markdown_widgets_share_one_backward_sentence_fade() {
        let source = "first paragraph\n\nsecond [link](https://example.test) `code` tail";
        let ctx = egui::Context::default();
        crate::ui::theme::apply(&ctx, "light", crate::ui::theme::DEFAULT_ACCENT_HEX);
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let start = ui
                    .ctx()
                    .graphics(|g| g.get(ui.layer_id()).map_or(0, |l| l.next_idx().0));
                crate::ui::panel::MathTextures::new().show_assistant_message(ui, source);
                paint_reveal(
                    ui,
                    &state(source, TextReveal::groups(&[(5, 0), (4, 175)], 350)),
                    start,
                    None,
                );
            });
        });
        let mut shapes = Vec::new();
        for clipped in &output.shapes {
            text_shapes(&clipped.shape, &mut shapes);
        }
        let tail = shapes
            .iter()
            .find(|s| s.galley.text().ends_with("code tail"))
            .unwrap();
        let colors = glyph_colors(tail);
        assert!(colors[colors.len() - 4..].iter().all(|c| c.a() == 0));
        assert!(colors[colors.len() - 8..colors.len() - 4]
            .iter()
            .all(|c| c.a() == 191));
        assert!(colors[..colors.len() - 8].iter().all(|c| c.a() == 255));
        let settled = shapes
            .iter()
            .find(|s| s.galley.text() == "first paragraph")
            .unwrap();
        assert!(glyph_colors(settled).iter().all(|c| c.a() == 255));
    }
}
