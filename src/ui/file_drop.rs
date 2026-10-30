//! Paint-only feedback for native file drops. Call `show` after the main card;
//! the host owns loading and calls `files_added` only for successfully appended
//! attachments. This module never reads files, accepts a drop, or captures input.

use std::time::Duration;

use egui::{Color32, Context, HoveredFile, Painter, Rect, Stroke, StrokeKind};

const FRAME: Duration = Duration::from_millis(33);
const NOTICE_SECONDS: f64 = 2.0;

#[derive(Default)]
pub struct FileDropFeedback {
    opacity: f32,
    from: f32,
    target: f32,
    changed_at: f64,
    hint: String,
    loading: bool,
    notice: Option<Notice>,
    composer_top: Option<f32>,
}

struct Notice {
    kind: NoticeKind,
    // Start on the UI clock, not wall time or the previous frame's timestamp.
    started_at: Option<f64>,
}

enum NoticeKind {
    Added(usize),
    Failed,
}

#[derive(Clone, Copy)]
struct Colors {
    accent: Color32,
    text: Color32,
    surface: Color32,
}

impl FileDropFeedback {
    /// Anchor notices above the real composer, never over its editable text.
    pub fn set_composer_rect(&mut self, rect: Rect) {
        self.composer_top = rect.is_finite().then_some(rect.top());
    }

    /// Report actual successful appends, never hovered/dropped file counts.
    /// A zero-count result deliberately does not show a success indication.
    pub fn files_added(&mut self, count: usize) {
        if count != 0 {
            self.notice = Some(Notice {
                kind: NoticeKind::Added(count),
                started_at: None,
            });
        }
    }

    /// Optional generic failure feedback. Detailed per-file errors belong to
    /// the host; no paths or unbounded error messages are painted here.
    pub fn failed(&mut self) {
        // Preserve an accurate success count for a partially successful batch.
        if !matches!(
            self.notice.as_ref().map(|n| &n.kind),
            Some(NoticeKind::Added(_))
        ) {
            self.notice = Some(Notice {
                kind: NoticeKind::Failed,
                started_at: None,
            });
        }
    }

    /// `rect` is the rounded card, normally the native window inset by 12 pt.
    /// Uses only a clipped layer painter: no Area, widgets, senses, or input
    /// mutation, so buttons, text selection, scrolling, and drops pass through.
    pub fn show(&mut self, ctx: &Context, rect: Rect, hovered: &[HoveredFile], loading: bool) {
        let now = ctx.input(|i| i.time);
        let style = ctx.style();
        let duration = f64::from(style.animation_time.max(0.0));
        let motion = duration > 0.0;
        let active = !hovered.is_empty() || loading;

        self.advance(now, duration);
        let target = if active { 1.0 } else { 0.0 };
        if self.target != target {
            self.from = self.opacity;
            self.target = target;
            self.changed_at = now;
        }
        self.advance(now, duration);
        if active {
            self.loading = loading;
            self.hint = hover_hint(hovered);
        }

        let notice_age = self.notice.as_mut().map(|notice| {
            let start = *notice.started_at.get_or_insert(now);
            (now - start).max(0.0)
        });
        if notice_age.is_some_and(|age| age >= NOTICE_SECONDS) {
            self.notice = None;
        }

        let rect = rect.intersect(ctx.input(|i| i.screen_rect()));
        if !rect.is_finite() || rect.width() < 16.0 || rect.height() < 16.0 {
            return;
        }
        // Rectangular scissoring alone cannot protect a transparent window's
        // rounded corners. All shapes also sit inside the card's 24 pt curve;
        // there are no outside shadows or outward strokes.
        let painter = ctx
            .layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("file-drop-feedback"),
            ))
            .with_clip_rect(rect);
        let colors = Colors {
            accent: style.visuals.selection.bg_fill,
            text: style.visuals.text_color(),
            surface: style.visuals.panel_fill,
        };
        if self.opacity > 0.0 {
            self.paint_hover(&painter, rect, now, motion, colors);
        }
        if let (Some(notice), Some(age)) = (&self.notice, notice_age) {
            let notice_rect = self.composer_top.map_or(rect, |top| {
                Rect::from_min_max(
                    rect.min,
                    egui::pos2(rect.right(), (top - 8.0).clamp(rect.top(), rect.bottom())),
                )
            });
            paint_notice(&painter, notice_rect, notice, age, motion, colors);
        }

        // Never request an immediate self-repaint. Reduced motion keeps hover
        // and loading static, with just one deadline to dismiss a notice.
        if motion && (active || self.opacity != self.target) {
            ctx.request_repaint_after(FRAME);
        } else if let (Some(_), Some(age)) = (&self.notice, notice_age) {
            if motion && (age < 0.7 || age >= NOTICE_SECONDS - 0.35) {
                ctx.request_repaint_after(FRAME);
            } else {
                // The badge holds still between its ring and its exit fade.
                let deadline = if motion {
                    NOTICE_SECONDS - 0.35
                } else {
                    NOTICE_SECONDS
                };
                ctx.request_repaint_after(Duration::from_secs_f64((deadline - age).max(0.001)));
            }
        }
    }

    fn advance(&mut self, now: f64, duration: f64) {
        let t = if duration > 0.0 {
            ((now - self.changed_at) / duration).clamp(0.0, 1.0) as f32
        } else {
            1.0
        };
        self.opacity = self.from + (self.target - self.from) * ease(t);
        if t >= 1.0 {
            self.opacity = self.target;
        }
    }

    fn paint_hover(&self, painter: &Painter, rect: Rect, now: f64, motion: bool, colors: Colors) {
        let Colors {
            accent,
            text,
            surface,
        } = colors;
        let alpha = self.opacity;
        let border = rect.shrink(4.0);
        let radius = 20.0_f32.min(border.width().min(border.height()) * 0.5);
        painter.rect_filled(border, radius, accent.gamma_multiply(0.08 * alpha));
        for (width, strength) in [(10.0_f32, 0.025), (6.0_f32, 0.045), (1.5_f32, 0.85)] {
            painter.rect_stroke(
                border,
                radius,
                Stroke::new(width, accent.gamma_multiply(strength * alpha)),
                StrokeKind::Inside,
            );
        }

        let safe = rect.shrink(28.0);
        if safe.width() < 100.0 || safe.height() < 76.0 {
            return;
        }
        let scale = if motion { 0.96 + 0.04 * alpha } else { 1.0 };
        let plate = Rect::from_center_size(
            safe.center(),
            egui::vec2(safe.width().min(300.0), safe.height().min(122.0)) * scale,
        );
        let painter = painter.with_clip_rect(safe);
        painter.rect_filled(plate, 16.0, surface.gamma_multiply(0.97 * alpha));
        painter.rect_stroke(
            plate,
            16.0,
            Stroke::new(1.0_f32, accent.gamma_multiply(0.30 * alpha)),
            StrokeKind::Inside,
        );
        let icon = egui::pos2(plate.center().x, plate.top() + 23.0);
        let icon_color = accent.gamma_multiply(alpha);
        if self.loading {
            paint_spinner(&painter, icon, now, motion, icon_color);
        } else {
            let bounce = if motion {
                (now * 3.0).sin() as f32 * 1.5
            } else {
                0.0
            };
            let center = icon + egui::vec2(0.0, bounce);
            painter.circle_filled(center, 15.0, accent.gamma_multiply(0.12 * alpha));
            painter.line_segment(
                [center - egui::vec2(6.0, 0.0), center + egui::vec2(6.0, 0.0)],
                Stroke::new(2.0_f32, icon_color),
            );
            painter.line_segment(
                [center - egui::vec2(0.0, 6.0), center + egui::vec2(0.0, 6.0)],
                Stroke::new(2.0_f32, icon_color),
            );
        }
        let width = plate.width() - 24.0;
        paint_text(
            &painter,
            egui::pos2(plate.center().x, plate.top() + 53.0),
            if self.loading {
                "正在加入附件…"
            } else {
                "放開以加入附件"
            },
            16.0,
            text.gamma_multiply(alpha),
            width,
        );
        if plate.height() >= 100.0 {
            paint_text(
                &painter,
                egui::pos2(plate.center().x, plate.top() + 82.0),
                &self.hint,
                12.0,
                text.gamma_multiply(0.76 * alpha),
                width,
            );
        }
    }
}

fn ease(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

fn hover_hint(hovered: &[HoveredFile]) -> String {
    if hovered.len() == 1 {
        if let Some(name) = hovered[0].path.as_deref().and_then(|p| p.file_name()) {
            // Cap before font layout, discard control/newline/bidi override
            // characters, and never expose the parent directory.
            let name = name.to_string_lossy();
            let mut chars = name.chars().filter(|c| {
                !c.is_control() && !matches!(*c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
            });
            let mut hint: String = chars.by_ref().take(72).collect();
            if chars.next().is_some() {
                hint.push('…');
            }
            if !hint.is_empty() {
                return hint;
            }
        }
    }
    if hovered.is_empty() {
        "請稍候，檔案準備中".to_owned()
    } else {
        format!("{} 個檔案", hovered.len())
    }
}

fn paint_text(
    painter: &Painter,
    center: egui::Pos2,
    text: &str,
    size: f32,
    color: Color32,
    width: f32,
) {
    let mut job = egui::text::LayoutJob::simple(
        text.to_owned(),
        egui::FontId::proportional(size),
        color,
        width.max(1.0),
    );
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.break_on_newline = false;
    let galley = painter.layout_job(job);
    let pos = center - galley.size() * 0.5;
    painter.galley(pos, galley, color);
}

fn paint_spinner(painter: &Painter, center: egui::Pos2, now: f64, motion: bool, color: Color32) {
    let phase = if motion { (now * 4.0) as f32 } else { 0.0 };
    let points: Vec<_> = (0..=24)
        .map(|i| center + egui::Vec2::angled(phase + i as f32 / 24.0 * 4.7) * 9.0)
        .collect();
    painter.add(egui::Shape::line(points, Stroke::new(2.0_f32, color)));
}

fn paint_notice(
    painter: &Painter,
    rect: Rect,
    notice: &Notice,
    age: f64,
    motion: bool,
    colors: Colors,
) {
    let Colors {
        accent,
        text,
        surface,
    } = colors;
    let safe = rect.shrink(28.0);
    if safe.width() < 120.0 || safe.height() < 48.0 {
        return;
    }
    let alpha = if motion {
        ease((age / 0.14).clamp(0.0, 1.0) as f32)
            * ease(((NOTICE_SECONDS - age) / 0.35).clamp(0.0, 1.0) as f32)
    } else {
        1.0
    };
    let badge = Rect::from_center_size(
        egui::pos2(safe.center().x, safe.bottom() - 22.0),
        egui::vec2(safe.width().min(292.0), 44.0),
    );
    let painter = painter.with_clip_rect(safe);
    painter.rect_filled(badge, 14.0, surface.gamma_multiply(alpha));
    painter.rect_stroke(
        badge,
        14.0,
        Stroke::new(1.0_f32, accent.gamma_multiply(0.45 * alpha)),
        StrokeKind::Inside,
    );
    let icon = egui::pos2(badge.left() + 22.0, badge.center().y);
    let color = accent.gamma_multiply(alpha);
    let message = match notice.kind {
        NoticeKind::Added(count) => {
            if motion {
                let pulse = (age / 0.7).clamp(0.0, 1.0) as f32;
                painter.circle_stroke(
                    icon,
                    7.0 + 8.0 * pulse,
                    Stroke::new(1.5_f32, accent.gamma_multiply((1.0 - pulse) * 0.55 * alpha)),
                );
            }
            painter.add(egui::Shape::line(
                vec![
                    icon + egui::vec2(-5.0, 0.0),
                    icon + egui::vec2(-1.0, 4.0),
                    icon + egui::vec2(6.0, -4.0),
                ],
                Stroke::new(2.0_f32, color),
            ));
            format!("已加入 {count} 個附件")
        }
        NoticeKind::Failed => {
            painter.circle_stroke(icon, 8.0, Stroke::new(1.5_f32, color));
            painter.line_segment(
                [icon - egui::vec2(0.0, 4.0), icon + egui::vec2(0.0, 1.0)],
                Stroke::new(1.5_f32, color),
            );
            painter.circle_filled(icon + egui::vec2(0.0, 4.0), 1.0, color);
            "無法加入附件，請重試".to_owned()
        }
    };
    paint_text(
        &painter,
        egui::pos2(badge.center().x + 13.0, badge.center().y),
        &message,
        13.0,
        text.gamma_multiply(alpha),
        badge.width() - 62.0,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen() -> Rect {
        Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(604.0, 344.0))
    }

    fn hovered() -> Vec<HoveredFile> {
        vec![HoveredFile {
            path: Some("C:/private/report.txt".into()),
            ..Default::default()
        }]
    }

    fn frame(
        ctx: &Context,
        feedback: &mut FileDropFeedback,
        time: f64,
        files: Vec<HoveredFile>,
        loading: bool,
    ) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(screen()),
                time: Some(time),
                hovered_files: files,
                ..Default::default()
            },
            |ctx| {
                let hovered = ctx.input(|i| i.raw.hovered_files.clone());
                feedback.show(ctx, screen().shrink(12.0), &hovered, loading);
            },
        )
    }

    fn text<'a>(output: &'a egui::FullOutput, value: &str) -> &'a egui::epaint::TextShape {
        output
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.job.text == value => Some(t),
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing painted text: {value}"))
    }

    #[test]
    fn success_badge_uses_actual_composer_position_without_covering_draft_text() {
        let ctx = Context::default();
        ctx.style_mut(|style| style.animation_time = 0.0);
        let mut feedback = FileDropFeedback::default();
        let composer = Rect::from_min_max(egui::pos2(35.0, 190.0), egui::pos2(569.0, 286.0));
        feedback.set_composer_rect(composer);
        feedback.files_added(1);
        let output = frame(&ctx, &mut feedback, 0.0, vec![], false);
        let message = text(&output, "已加入 1 個附件");
        assert!(message.pos.y + message.galley.size().y < composer.top());
        for shape in &output.shapes {
            if let egui::Shape::Rect(rect) = &shape.shape {
                assert!(rect.rect.bottom() < composer.top());
            }
        }
    }

    #[test]
    fn raw_hover_frames_paint_enter_exit_alpha_scale_and_no_fake_success() {
        let ctx = Context::default();
        ctx.style_mut(|s| s.animation_time = 0.2);
        let mut feedback = FileDropFeedback::default();
        assert!(frame(&ctx, &mut feedback, 0.0, vec![], false)
            .shapes
            .is_empty());
        let _ = frame(&ctx, &mut feedback, 1.0, hovered(), false);
        let entering = frame(&ctx, &mut feedback, 1.05, hovered(), false);
        let entered = frame(&ctx, &mut feedback, 1.3, hovered(), false);
        let early = text(&entering, "放開以加入附件");
        let full = text(&entered, "放開以加入附件");
        assert!(
            early.galley.job.sections[0].format.color.a()
                < full.galley.job.sections[0].format.color.a()
        );
        let plate_width = |output: &egui::FullOutput| {
            output
                .shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Rect(r) if r.corner_radius.nw == 16 => Some(r.rect.width()),
                    _ => None,
                })
                .next()
                .unwrap()
        };
        assert!(plate_width(&entering) < plate_width(&entered));
        text(&entered, "report.txt");
        assert!(
            feedback.notice.is_none(),
            "hover must not claim an attachment was added"
        );
        let _ = frame(&ctx, &mut feedback, 1.4, vec![], false);
        let leaving = frame(&ctx, &mut feedback, 1.5, vec![], false);
        assert!(plate_width(&leaving) < plate_width(&entered));
        assert!(
            text(&leaving, "放開以加入附件").galley.job.sections[0]
                .format
                .color
                .a()
                < full.galley.job.sections[0].format.color.a()
        );
        assert!(frame(&ctx, &mut feedback, 1.7, vec![], false)
            .shapes
            .is_empty());
    }

    #[test]
    fn only_success_callback_paints_count_then_expires_failure_is_not_success() {
        let ctx = Context::default();
        let mut feedback = FileDropFeedback::default();
        feedback.files_added(0);
        assert!(frame(&ctx, &mut feedback, 0.0, vec![], false)
            .shapes
            .is_empty());
        feedback.files_added(2);
        let _ = frame(&ctx, &mut feedback, 1.0, vec![], false);
        let success = frame(&ctx, &mut feedback, 1.2, vec![], false);
        text(&success, "已加入 2 個附件");
        assert!(success
            .shapes
            .iter()
            .any(|s| matches!(s.shape, egui::Shape::Circle(_))));
        assert!(frame(&ctx, &mut feedback, 3.01, vec![], false)
            .shapes
            .is_empty());
        feedback.failed();
        let _ = frame(&ctx, &mut feedback, 4.0, vec![], false);
        let failure = frame(&ctx, &mut feedback, 4.2, vec![], false);
        text(&failure, "無法加入附件，請重試");
        assert!(!failure.shapes.iter().any(
            |s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.job.text.contains("已加入"))
        ));
    }

    #[test]
    fn loading_spinner_is_painted_and_reduced_motion_is_static() {
        let ctx = Context::default();
        ctx.style_mut(|s| s.animation_time = 0.0);
        let mut feedback = FileDropFeedback::default();
        let first = frame(&ctx, &mut feedback, 0.0, hovered(), true);
        let second = frame(&ctx, &mut feedback, 1.0, hovered(), true);
        text(&first, "正在加入附件…");
        assert_eq!(first.shapes, second.shapes);
        assert!(first
            .shapes
            .iter()
            .any(|s| matches!(&s.shape, egui::Shape::Path(p) if p.points.len() == 25)));
        assert!(frame(&ctx, &mut feedback, 2.0, vec![], false)
            .shapes
            .is_empty());
        feedback.files_added(1);
        text(
            &frame(&ctx, &mut feedback, 3.0, vec![], false),
            "已加入 1 個附件",
        );
        assert!(frame(&ctx, &mut feedback, 5.01, vec![], false)
            .shapes
            .is_empty());
    }

    #[test]
    fn animated_loading_rotates_real_spinner_paths_and_hover_reports_count() {
        let ctx = Context::default();
        let mut feedback = FileDropFeedback::default();
        let _ = frame(&ctx, &mut feedback, 0.0, hovered(), true);
        let first = frame(&ctx, &mut feedback, 0.5, hovered(), true);
        let second = frame(&ctx, &mut feedback, 0.6, hovered(), true);
        let points = |output: &egui::FullOutput| {
            output
                .shapes
                .iter()
                .find_map(|s| match &s.shape {
                    egui::Shape::Path(path) if path.points.len() == 25 => Some(path.points.clone()),
                    _ => None,
                })
                .unwrap()
        };
        assert_ne!(points(&first), points(&second));
        let files = [hovered(), hovered(), hovered()].concat();
        let multiple = frame(&ctx, &mut feedback, 0.7, files, false);
        text(&multiple, "3 個檔案");
        assert!(feedback.notice.is_none());
    }

    #[test]
    fn reduced_motion_hover_does_not_keep_requesting_frames() {
        let ctx = Context::default();
        ctx.style_mut(|s| s.animation_time = 0.0);
        let mut feedback = FileDropFeedback::default();
        for n in 0..4 {
            let _ = frame(&ctx, &mut feedback, n as f64 * 0.1, hovered(), true);
        }
        let steady = frame(&ctx, &mut feedback, 0.5, hovered(), true);
        assert_eq!(
            steady.viewport_output[&egui::ViewportId::ROOT].repaint_delay,
            Duration::MAX
        );
        feedback.files_added(1);
        let first = frame(&ctx, &mut feedback, 1.0, vec![], false);
        text(&first, "已加入 1 個附件");
        let steady = frame(&ctx, &mut feedback, 1.2, vec![], false);
        let delay = steady.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
        assert!(delay > Duration::from_secs(1) && delay <= Duration::from_secs(2));
        assert!(frame(&ctx, &mut feedback, 3.01, vec![], false)
            .shapes
            .is_empty());
    }

    #[test]
    fn success_hold_sleeps_until_exit_fade_then_returns_to_idle() {
        let ctx = Context::default();
        let mut feedback = FileDropFeedback::default();
        feedback.files_added(1);
        for time in [0.0, 0.1, 0.2, 0.8] {
            let _ = frame(&ctx, &mut feedback, time, vec![], false);
        }
        let hold = frame(&ctx, &mut feedback, 0.9, vec![], false);
        text(&hold, "已加入 1 個附件");
        let delay = hold.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
        assert!(delay > Duration::from_millis(500));
        let fading = frame(&ctx, &mut feedback, 1.8, vec![], false);
        let delay = fading.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
        assert!(delay > Duration::ZERO && delay <= FRAME);
        for time in [2.1, 2.2, 2.3] {
            let _ = frame(&ctx, &mut feedback, time, vec![], false);
        }
        let idle = frame(&ctx, &mut feedback, 2.4, vec![], false);
        assert!(idle.shapes.is_empty());
        assert_eq!(
            idle.viewport_output[&egui::ViewportId::ROOT].repaint_delay,
            Duration::MAX
        );
    }

    #[test]
    fn filenames_and_actual_painted_meshes_are_bounded_in_both_themes() {
        for dark in [false, true] {
            let ctx = Context::default();
            ctx.set_visuals(if dark {
                egui::Visuals::dark()
            } else {
                egui::Visuals::light()
            });
            ctx.style_mut(|s| s.animation_time = 0.0);
            let expected = ctx.style().visuals.text_color();
            let mut feedback = FileDropFeedback::default();
            feedback.files_added(3);
            let files = vec![HoveredFile {
                path: Some(format!("C:/secret/{}.txt", "測試\n".repeat(300)).into()),
                ..Default::default()
            }];
            let output = frame(&ctx, &mut feedback, 0.0, files, false);
            let title = text(&output, "放開以加入附件");
            assert_eq!(title.galley.job.sections[0].format.color, expected);
            let luminance = |color: Color32| {
                let channel = |v: u8| {
                    let v = f64::from(v) / 255.0;
                    if v <= 0.04045 {
                        v / 12.92
                    } else {
                        ((v + 0.055) / 1.055).powf(2.4)
                    }
                };
                0.2126 * channel(color.r())
                    + 0.7152 * channel(color.g())
                    + 0.0722 * channel(color.b())
            };
            let fg = luminance(expected);
            let bg = luminance(ctx.style().visuals.panel_fill);
            assert!((fg.max(bg) + 0.05) / (fg.min(bg) + 0.05) >= 4.5);
            for shape in &output.shapes {
                assert!(screen().shrink(12.0).contains_rect(shape.clip_rect));
                if let egui::Shape::Text(t) = &shape.shape {
                    assert!(t.galley.job.text.chars().count() <= 73);
                    assert!(!t.galley.job.text.contains("secret"));
                    assert_eq!(t.galley.rows.len(), 1);
                    assert!(t.galley.size().x <= 277.0);
                }
            }
            let meshes = ctx.tessellate(output.shapes, output.pixels_per_point);
            let mut painted = 0;
            for clipped in meshes {
                if let egui::epaint::Primitive::Mesh(mesh) = clipped.primitive {
                    for vertex in mesh.vertices.iter().filter(|v| v.color.a() > 0) {
                        painted += 1;
                        let card = screen().shrink(12.0);
                        assert!(
                            card.contains(vertex.pos),
                            "paint escaped card: {:?}",
                            vertex.pos
                        );
                        // Distance from the nearest point of the radius-24
                        // card's central rectangle also checks rounded corners.
                        let center = card.shrink(24.0).clamp(vertex.pos);
                        assert!(center.distance(vertex.pos) <= 24.1);
                    }
                }
            }
            assert!(painted > 100);
        }
    }

    #[test]
    fn underlying_button_still_receives_clicks_while_overlay_paints() {
        let ctx = Context::default();
        ctx.style_mut(|s| s.animation_time = 0.0);
        let mut feedback = FileDropFeedback::default();
        let mut button = Rect::NOTHING;
        let mut clicked = false;
        for (time, pressed) in [(0.0, None), (0.1, Some(true)), (0.2, Some(false))] {
            let pos = button.center();
            let events = pressed
                .map(|pressed| {
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ]
                })
                .unwrap_or_default();
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(screen()),
                    time: Some(time),
                    hovered_files: hovered(),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        // Put the real button directly underneath the center plate.
                        let response = ui.put(
                            Rect::from_center_size(screen().center(), egui::vec2(120.0, 30.0)),
                            egui::Button::new("Underlying button"),
                        );
                        button = response.rect;
                        clicked |= response.clicked();
                    });
                    let files = ctx.input(|i| i.raw.hovered_files.clone());
                    feedback.show(ctx, screen().shrink(12.0), &files, false);
                },
            );
            text(&output, "放開以加入附件");
        }
        assert!(
            clicked,
            "paint-only foreground must not occlude the button's hit test"
        );
    }

    #[test]
    fn idle_stops_repainting_and_active_requests_are_delayed() {
        let ctx = Context::default();
        let mut feedback = FileDropFeedback::default();
        // Drain egui's own initial/font repaint requests before measuring.
        for n in 0..4 {
            let _ = frame(&ctx, &mut feedback, n as f64 * 0.1, vec![], false);
        }
        let idle = frame(&ctx, &mut feedback, 0.5, vec![], false);
        assert_eq!(
            idle.viewport_output[&egui::ViewportId::ROOT].repaint_delay,
            Duration::MAX
        );
        let _ = frame(&ctx, &mut feedback, 1.0, hovered(), false);
        let active = frame(&ctx, &mut feedback, 1.5, hovered(), false);
        let delay = active.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
        assert!(delay > Duration::ZERO && delay <= FRAME, "{delay:?}");
        let _ = frame(&ctx, &mut feedback, 2.0, vec![], false);
        for n in 0..4 {
            let _ = frame(&ctx, &mut feedback, 2.5 + n as f64 * 0.1, vec![], false);
        }
        let idle = frame(&ctx, &mut feedback, 3.0, vec![], false);
        assert_eq!(
            idle.viewport_output[&egui::ViewportId::ROOT].repaint_delay,
            Duration::MAX
        );
    }
}
