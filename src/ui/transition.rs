//! Incoming-only navigation motion. Layout, input and IME coordinates move
//! together; no outgoing page, captured image or persistent widget state is kept.

use std::time::Duration;

const DURATION: f64 = 0.180;
const DISTANCE: f32 = 6.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransitionFrame {
    pub opacity: f32,
    pub offset: egui::Vec2,
    pub active: bool,
}

impl TransitionFrame {
    const SETTLED: Self = Self {
        opacity: 1.0,
        offset: egui::Vec2::ZERO,
        active: false,
    };
}

#[derive(Default)]
pub struct PageTransition {
    key: Option<u64>,
    started_at: Option<f64>,
    last_time: Option<f64>,
    direction: f32,
}

impl PageTransition {
    /// First observation is fully visible. A new key starts a fresh incoming
    /// page; repeating a key/time (including egui discard passes) is idempotent.
    pub fn update(&mut self, key: u64, now: f64, enabled: bool) -> TransitionFrame {
        let previous = self.key;
        let reset_clock = !now.is_finite() || self.last_time.is_some_and(|last| now < last);
        self.key = Some(key);
        self.last_time = now.is_finite().then_some(now);

        if previous.is_none() || !enabled || reset_clock {
            self.started_at = None;
            return TransitionFrame::SETTLED;
        }

        if previous != Some(key) {
            self.direction = if key > previous.unwrap() { 1.0 } else { -1.0 };
            self.started_at = Some(now);
        }

        let Some(start) = self.started_at else {
            return TransitionFrame::SETTLED;
        };
        let progress = (now - start) / DURATION;
        if now >= start + DURATION || progress >= 1.0 {
            self.started_at = None;
            return TransitionFrame::SETTLED;
        }

        let eased = ease_out(progress);
        TransitionFrame {
            opacity: (0.2 + 0.8 * eased) as f32,
            offset: egui::vec2(self.direction * DISTANCE * (1.0 - eased) as f32, 0.0),
            active: true,
        }
    }
}

fn ease_out(progress: f64) -> f64 {
    1.0 - (1.0 - progress.clamp(0.0, 1.0)).powi(3)
}

/// Render the contents once in a stable child scope, with real translated
/// layout coordinates. Explicit widget IDs and shared application state remain
/// the caller's responsibility and are never salted with a page key or clock.
///
/// The clip stays at the original available rectangle. Reserve the *unshifted*
/// occupied space in the parent, bounded to that rectangle, so the six-point
/// entrance cannot grow a scroll area's width or the enclosing window.
pub fn show<R>(
    ui: &mut egui::Ui,
    frame: TransitionFrame,
    contents: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    show_impl(ui, frame, false, contents)
}

/// For contents INSIDE a vertical ScrollArea. Egui gives its content Ui a
/// viewport-sized max_rect, not infinite height. Preserve the full natural
/// height and use the scroll area's clip; otherwise long panes look short to
/// the scrollbar and their lower controls can never scroll into view.
/// Width stays bounded so entrance motion cannot create horizontal overflow.
pub fn show_scroll_contents<R>(
    ui: &mut egui::Ui,
    frame: TransitionFrame,
    contents: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    show_impl(ui, frame, true, contents)
}

fn show_impl<R>(
    ui: &mut egui::Ui,
    frame: TransitionFrame,
    vertical_overflow: bool,
    contents: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    let available = ui.available_rect_before_wrap();
    let original = egui::Rect::from_min_max(available.min, available.max.max(available.min));
    let clip = if vertical_overflow {
        egui::Rect::from_min_max(
            egui::pos2(original.left(), ui.clip_rect().top()),
            egui::pos2(original.right(), ui.clip_rect().bottom()),
        )
        .intersect(ui.clip_rect())
    } else {
        original.intersect(ui.clip_rect())
    };
    let offset = if frame.offset.is_finite() {
        frame.offset
    } else {
        egui::Vec2::ZERO
    };
    let opacity = if frame.opacity.is_finite() {
        frame.opacity.clamp(0.0, 1.0)
    } else {
        1.0
    };
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("page_transition")
            .max_rect(original.translate(offset)),
    );
    child.set_clip_rect(clip);
    child.multiply_opacity(opacity);
    let inner = contents(&mut child);
    let natural = child.min_rect().translate(-offset);
    let occupied = if vertical_overflow {
        egui::Rect::from_min_max(
            egui::pos2(
                natural.left().clamp(original.left(), original.right()),
                natural.top(),
            ),
            egui::pos2(
                natural.right().clamp(original.left(), original.right()),
                natural.bottom(),
            ),
        )
    } else {
        natural.intersect(original)
    };
    let id = child.unique_id();
    // Dropping registers the final translated response, including close flags.
    drop(child);
    let response = ui
        .ctx()
        .read_response(id)
        .expect("a dropped child Ui has a registered response");
    ui.advance_cursor_after_rect(occupied);

    if frame.active {
        ui.ctx().request_repaint_after(Duration::from_millis(16));
    }
    egui::InnerResponse::new(inner, response)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeightFrame {
    pub height: f32,
    pub active: bool,
}

#[derive(Default)]
pub struct WindowHeightTransition {
    mode: Option<u8>,
    target: f32,
    sample: f32,
    from: f32,
    started_at: Option<f64>,
    last_time: Option<f64>,
}

impl WindowHeightTransition {
    /// Animate only mode changes. Same-mode target changes (composer growth,
    /// user resizing) snap. On reversal use the observed native height, not an
    /// assumed viewport-command acknowledgement; fall back to the last sample.
    /// Pass `enabled = false` for native resizing, monitor/DPI changes or reduced
    /// motion. The caller must track animated viewport requests as programmatic.
    pub fn update(
        &mut self,
        mode: u8,
        target: f32,
        current: Option<f32>,
        now: f64,
        enabled: bool,
    ) -> HeightFrame {
        let observed = current
            .filter(|height| height.is_finite() && *height > 0.0)
            .map(|height| height.max(1.0));
        // Preserve every finite positive target exactly. Invalid targets fall
        // back to the previous valid target, actual height, then one point.
        let target = finite_height(target)
            .or_else(|| self.mode.and_then(|_| finite_height(self.target)))
            .or(observed)
            .unwrap_or(1.0);
        let reset_clock = !now.is_finite() || self.last_time.is_some_and(|last| now < last);
        let first = self.mode.is_none();
        let mode_changed = self.mode != Some(mode);
        let target_changed = target != self.target;
        self.mode = Some(mode);
        self.target = target;
        self.last_time = now.is_finite().then_some(now);

        if first || !enabled || reset_clock || (!mode_changed && target_changed) {
            return self.settle();
        }
        if mode_changed {
            self.from = observed
                .or_else(|| finite_height(self.sample))
                .unwrap_or(target);
            self.started_at = Some(now);
        }

        let Some(start) = self.started_at else {
            return self.settle();
        };
        let progress = (now - start) / DURATION;
        if now >= start + DURATION || progress >= 1.0 || self.from == target {
            return self.settle();
        }
        // f64 arithmetic prevents overflow even for finite f32::MAX targets.
        // Clamp roundoff to the endpoints; never overshoot the native limits.
        self.sample =
            (self.from as f64 + (target as f64 - self.from as f64) * ease_out(progress)) as f32;
        self.sample = self
            .sample
            .clamp(self.from.min(target), self.from.max(target));
        HeightFrame {
            height: self.sample,
            active: true,
        }
    }

    fn settle(&mut self) -> HeightFrame {
        self.started_at = None;
        self.sample = self.target;
        HeightFrame {
            height: self.target,
            active: false,
        }
    }
}

fn finite_height(height: f32) -> Option<f32> {
    height.is_finite().then(|| height.max(1.0))
}

/// Honor the application's animation setting and Windows' read-only client
/// area animation preference. Cache per egui context for five input-clock
/// seconds; no global state, registry writes or system setting changes.
pub fn motion_enabled(ctx: &egui::Context) -> bool {
    if ctx.style().animation_time <= 0.0 {
        return false;
    }

    #[cfg(windows)]
    {
        let now = ctx.input(|input| input.time);
        ctx.data_mut(|data| {
            let id = egui::Id::new("popchat_client_area_animation_preference");
            let cached = data.get_temp::<MotionPreference>(id);
            let preference = cached_motion_preference(cached, now, query_client_area_animation);
            data.insert_temp(id, preference);
            preference.enabled
        })
    }
    #[cfg(not(windows))]
    {
        true
    }
}

#[cfg(any(windows, test))]
#[derive(Clone, Copy)]
struct MotionPreference {
    checked_at: f64,
    enabled: bool,
}

#[cfg(any(windows, test))]
fn cached_motion_preference(
    cached: Option<MotionPreference>,
    now: f64,
    query: impl FnOnce() -> bool,
) -> MotionPreference {
    if let Some(cached) = cached {
        let age = now - cached.checked_at;
        if age.is_finite() && (0.0..5.0).contains(&age) {
            return cached;
        }
    }
    MotionPreference {
        checked_at: now,
        enabled: query(),
    }
}

#[cfg(windows)]
fn query_client_area_animation() -> bool {
    #[link(name = "user32")]
    extern "system" {
        fn SystemParametersInfoW(
            action: u32,
            parameter: u32,
            value: *mut std::ffi::c_void,
            flags: u32,
        ) -> i32;
    }

    const SPI_GETCLIENTAREAANIMATION: u32 = 0x1042;
    let mut enabled: i32 = 1;
    // SAFETY: SPI_GETCLIENTAREAANIMATION writes one BOOL to a valid, aligned
    // local i32. This GET action and flags=0 never change system settings.
    let success = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            (&mut enabled as *mut i32).cast(),
            0,
        )
    };
    success == 0 || enabled != 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{pos2, vec2, Color32, Event, Id, Modifiers, PointerButton, Pos2, Rect, Shape};

    fn close(actual: f32, expected: f32) {
        assert!((actual - expected).abs() < 0.001, "{actual} != {expected}");
    }

    #[test]
    fn page_first_observation_and_same_key_never_fade() {
        let mut transition = PageTransition::default();
        for now in [0.0, 0.04, 1.0, 10.0] {
            assert_eq!(transition.update(7, now, true), TransitionFrame::SETTLED);
        }
    }

    #[test]
    fn page_uses_cubic_incoming_trajectory_and_settles_exactly() {
        let mut transition = PageTransition::default();
        transition.update(0, 0.0, true);
        let start = transition.update(1, 1.0, true);
        assert_eq!(start.opacity, 0.2);
        assert_eq!(start.offset, vec2(6.0, 0.0));
        assert!(start.active);
        let middle = transition.update(1, 1.09, true);
        close(middle.opacity, 0.9);
        close(middle.offset.x, 0.75);
        assert!(middle.active);
        assert_eq!(transition.update(1, 1.18, true), TransitionFrame::SETTLED);
        assert_eq!(transition.update(1, 10.0, true), TransitionFrame::SETTLED);
    }

    #[test]
    fn page_rapid_navigation_reversal_and_reentrant_passes() {
        let mut transition = PageTransition::default();
        transition.update(10, 0.0, true);
        let forward = transition.update(20, 0.01, true);
        assert_eq!(transition.update(20, 0.01, true), forward);
        assert!(transition.update(20, 0.03, true).opacity > forward.opacity);
        let back = transition.update(10, 0.04, true);
        assert_eq!(back.opacity, 0.2);
        assert_eq!(back.offset, vec2(-6.0, 0.0));
        assert_eq!(transition.update(10, 0.04, true), back);
        let next = transition.update(30, 0.05, true);
        assert_eq!(next, forward);
        assert_eq!(transition.update(30, 0.05, true), next);
    }

    #[test]
    fn page_disabled_and_nonfinite_or_reset_clocks_snap_without_restarting() {
        for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0] {
            let mut transition = PageTransition::default();
            transition.update(0, 1.0, true);
            transition.update(1, 1.1, true);
            assert_eq!(
                transition.update(2, invalid, true),
                TransitionFrame::SETTLED
            );
            assert_eq!(transition.update(2, 2.0, true), TransitionFrame::SETTLED);
            assert!(transition.update(3, 2.1, true).active);
            assert_eq!(transition.update(3, 2.11, false), TransitionFrame::SETTLED);
            assert_eq!(transition.update(3, 2.12, true), TransitionFrame::SETTLED);
        }
    }

    #[test]
    fn page_trajectory_is_bounded_monotonic_and_ends_at_duration() {
        for key in [0, 2] {
            let mut transition = PageTransition::default();
            transition.update(1, 0.0, true);
            transition.update(key, 0.0, true);
            let mut last_opacity = 0.2;
            let mut last_offset = 6.0;
            for step in 0..=180 {
                let frame = transition.update(key, step as f64 / 1000.0, true);
                assert!((0.2..=1.0).contains(&frame.opacity));
                assert!(frame.opacity >= last_opacity);
                assert!(frame.offset.x.abs() <= last_offset);
                last_opacity = frame.opacity;
                last_offset = frame.offset.x.abs();
            }
            assert_eq!(
                transition.update(key, DURATION, true),
                TransitionFrame::SETTLED
            );
        }
    }

    #[test]
    fn height_first_call_and_same_mode_target_changes_snap() {
        let mut transition = WindowHeightTransition::default();
        assert_eq!(
            transition.update(0, 270.0, Some(900.0), 0.0, true),
            HeightFrame {
                height: 270.0,
                active: false
            }
        );
        assert!(!transition.update(0, 300.0, Some(270.0), 0.1, true).active);
        assert!(!transition.update(0, 320.0, Some(300.0), 0.2, true).active);
        transition.update(1, 600.0, Some(320.0), 0.3, true);
        let growth = transition.update(1, 620.0, Some(350.0), 0.31, true);
        assert_eq!(
            growth,
            HeightFrame {
                height: 620.0,
                active: false
            }
        );
        assert_eq!(transition.update(1, 620.0, None, 0.4, true), growth);
    }

    #[test]
    fn height_mode_change_uses_actual_height_and_cubic_ease() {
        let mut transition = WindowHeightTransition::default();
        transition.update(0, 270.0, None, 0.0, true);
        let start = transition.update(1, 600.0, Some(280.0), 0.0, true);
        assert_eq!(
            start,
            HeightFrame {
                height: 280.0,
                active: true
            }
        );
        assert_eq!(transition.update(1, 600.0, Some(400.0), 0.0, true), start);
        let middle = transition.update(1, 600.0, Some(300.0), 0.09, true);
        close(middle.height, 560.0);
        assert!(middle.active);
        assert_eq!(
            transition.update(1, 600.0, None, DURATION, true),
            HeightFrame {
                height: 600.0,
                active: false
            }
        );
    }

    #[test]
    fn height_reversal_uses_observation_or_previous_sample_not_old_target() {
        let mut transition = WindowHeightTransition::default();
        transition.update(0, 270.0, None, 0.0, true);
        transition.update(1, 600.0, Some(270.0), 0.0, true);
        let previous = transition.update(1, 600.0, None, 0.06, true);
        let back = transition.update(0, 270.0, Some(390.0), 0.07, true);
        assert_eq!(back.height, 390.0);
        assert!(back.active);
        assert_ne!(back.height, previous.height);
        let next = transition.update(0, 270.0, None, 0.08, true);
        let reversed = transition.update(1, 600.0, None, 0.09, true);
        assert_eq!(reversed.height, next.height);
        assert_eq!(transition.update(1, 600.0, None, 0.09, true), reversed);
    }

    #[test]
    fn height_disabled_bad_clocks_and_bad_targets_are_safe() {
        for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0] {
            let mut transition = WindowHeightTransition::default();
            transition.update(0, 270.0, None, 0.0, true);
            transition.update(1, 600.0, None, 0.1, true);
            let snap = transition.update(1, 600.0, None, invalid, true);
            assert_eq!(
                snap,
                HeightFrame {
                    height: 600.0,
                    active: false
                }
            );
            assert_eq!(transition.update(1, 600.0, None, 1.0, true), snap);
            assert!(!transition.update(0, 270.0, None, 1.1, false).active);
            assert!(!transition.update(0, 270.0, None, 1.2, true).active);
        }
        for target in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -50.0, 0.0] {
            let frame = WindowHeightTransition::default().update(0, target, None, 0.0, true);
            assert_eq!(frame.height, 1.0);
            assert!(!frame.active);
        }
    }

    #[test]
    fn height_never_overshoots_even_at_extreme_finite_targets() {
        for (from, target) in [
            (270.0, 600.0),
            (600.0, 270.0),
            (1.0, f32::MAX),
            (f32::MAX, 1.0),
        ] {
            let mut transition = WindowHeightTransition::default();
            transition.update(0, from, None, 0.0, true);
            transition.update(1, target, Some(from), 0.0, true);
            for step in 0..=180 {
                let frame = transition.update(1, target, None, step as f64 / 1000.0, true);
                assert!(frame.height.is_finite());
                assert!((from.min(target)..=from.max(target)).contains(&frame.height));
            }
            assert_eq!(
                transition.update(1, target, None, DURATION, true),
                HeightFrame {
                    height: target,
                    active: false
                }
            );
        }
    }

    fn input(time: f64, events: Vec<Event>) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0))),
            time: Some(time),
            events,
            ..Default::default()
        }
    }

    fn test_ui(ctx: &egui::Context) -> egui::Ui {
        egui::Ui::new(
            ctx.clone(),
            Id::new("transition_test_root"),
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_size(pos2(20.0, 20.0), vec2(200.0, 120.0)))
                .layout(egui::Layout::top_down(egui::Align::Min)),
        )
    }

    fn context() -> egui::Context {
        let ctx = egui::Context::default();
        ctx.options_mut(|options| options.max_passes = std::num::NonZeroUsize::new(1).unwrap());
        ctx
    }

    #[test]
    fn translated_button_shapes_hitrect_and_click_follow_real_geometry() {
        let ctx = context();
        let frame = TransitionFrame {
            opacity: 0.2,
            offset: vec2(6.0, 0.0),
            active: true,
        };
        let mut clicks = 0;
        let mut calls = 0;
        let mut draw = |ctx: &egui::Context| {
            let mut ui = test_ui(ctx);
            show(&mut ui, frame, |ui| {
                calls += 1;
                let response = ui.add_sized(vec2(80.0, 24.0), egui::Button::new("Go"));
                if response.clicked() {
                    clicks += 1;
                }
                response
            })
            .inner
        };
        let mut button = None;
        let output = ctx.run(input(0.0, vec![]), |ctx| button = Some(draw(ctx)));
        let rect = button.as_ref().unwrap().rect;
        assert_eq!(rect.min, pos2(26.0, 20.0));
        assert_eq!(button.as_ref().unwrap().interact_rect, rect);
        assert!(output
            .shapes
            .iter()
            .any(|shape| matches!(&shape.shape, Shape::Rect(painted) if painted.rect == rect)));
        // This point lies outside the unshifted button but inside its new edge.
        let point = pos2(rect.right() - 2.0, rect.center().y);
        assert!(point.x > rect.right() - frame.offset.x);
        let pointer = |pressed| Event::PointerButton {
            pos: point,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        let _ = ctx.run(
            input(0.016, vec![Event::PointerMoved(point), pointer(true)]),
            |ctx| {
                draw(ctx);
            },
        );
        let _ = ctx.run(input(0.032, vec![pointer(false)]), |ctx| {
            draw(ctx);
        });
        assert_eq!(clicks, 1);
        assert_eq!(calls, 3, "the contents closure runs once per pass");
    }

    #[test]
    fn translated_clip_preserves_available_size_and_does_not_grow_parent() {
        let ctx = context();
        for offset in [-6.0, 0.0, 6.0] {
            let _ = ctx.run(input(0.0, vec![]), |ctx| {
                let mut ui = test_ui(ctx);
                let original = ui.available_rect_before_wrap();
                let clip = original.shrink(2.0);
                ui.set_clip_rect(clip);
                let frame = TransitionFrame {
                    opacity: 1.0,
                    offset: vec2(offset, 0.0),
                    active: false,
                };
                let response = show(&mut ui, frame, |ui| {
                    assert_eq!(
                        ui.available_rect_before_wrap(),
                        original.translate(frame.offset)
                    );
                    assert_eq!(ui.available_size(), original.size());
                    assert_eq!(ui.clip_rect(), clip);
                    ui.add_sized(
                        vec2(original.width(), 24.0),
                        egui::Button::new("full width"),
                    )
                });
                assert_eq!(
                    response.inner.interact_rect,
                    response.inner.rect.intersect(clip)
                );
                assert_eq!(ui.min_rect().left(), original.left());
                assert_eq!(ui.min_rect().right(), original.right());
                assert_eq!(ui.min_rect().width(), original.width());
                close(ui.min_rect().height(), 24.0);
                assert_eq!(ui.clip_rect(), clip);
            });
        }
    }

    #[test]
    fn zero_height_page_during_native_resize_is_bounded_and_still_runs_once() {
        let ctx = context();
        let mut calls = 0;
        let _ = ctx.run(input(0.0, vec![]), |ctx| {
            let mut ui = egui::Ui::new(
                ctx.clone(),
                Id::new("empty_transition_root"),
                egui::UiBuilder::new()
                    .max_rect(Rect::from_min_size(pos2(20.0, 20.0), vec2(200.0, 0.0))),
            );
            show(
                &mut ui,
                TransitionFrame {
                    opacity: 0.2,
                    offset: vec2(6.0, 0.0),
                    active: true,
                },
                |ui| {
                    calls += 1;
                    assert!(ui.available_height() >= 0.0);
                    ui.add_sized(vec2(200.0, 28.0), egui::Button::new("clipped page"));
                },
            );
            assert!(ui.min_rect().width() <= 200.0);
            close(ui.min_rect().height(), 0.0);
        });
        assert_eq!(calls, 1);
    }

    #[test]
    fn scrolling_pane_keeps_natural_height_and_bottom_controls_reachable_by_wheel() {
        for offset in [-6.0, 0.0, 6.0] {
            let ctx = context();
            let mut time = 0.0;
            let mut draw = |events: Vec<Event>| {
                time += 0.1;
                let mut result = None;
                let output = ctx.run(input(time, events), |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let scroll = egui::ScrollArea::vertical()
                            .id_salt("natural_height_test")
                            .max_height(180.0)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.label("Settings heading fixture");
                                show_scroll_contents(
                                    ui,
                                    TransitionFrame {
                                        opacity: 1.0,
                                        offset: vec2(offset, 0.0),
                                        active: offset != 0.0,
                                    },
                                    |ui| {
                                        ui.label("First pane field");
                                        ui.add_space(700.0);
                                        ui.button("Bottom pane action")
                                    },
                                )
                                .inner
                            });
                        result = Some((
                            scroll.content_size,
                            scroll.inner_rect,
                            scroll.state.offset,
                            scroll.inner,
                        ));
                    });
                });
                (output, result.unwrap())
            };
            let _ = draw(vec![]);
            let (_, initial) = draw(vec![]);
            assert!(
                initial.0.y > 740.0,
                "pane height must include all content, not only the viewport: {:?}",
                initial.0
            );
            let pos = initial.1.center();
            let (_, mut scrolled) = draw(vec![
                Event::PointerMoved(pos),
                Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: vec2(0.0, -10_000.0),
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
            for _ in 0..8 {
                scrolled = draw(vec![]).1;
            }
            assert!(
                scrolled.2.y > 500.0,
                "wheel must move the viewport: {:?}",
                scrolled.2
            );
            assert!(
                scrolled.1.contains_rect(scrolled.3.interact_rect)
                    && scrolled.3.rect.top() >= scrolled.1.top()
                    && scrolled.3.rect.bottom() <= scrolled.1.bottom() + 0.5,
                "bottom action must become visible: {:?} {:?}",
                scrolled.1,
                scrolled.3.rect
            );
            let pos = scrolled.3.rect.center();
            for pressed in [true, false] {
                let (_, result) = draw(vec![
                    Event::PointerMoved(pos),
                    Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]);
                assert_eq!(
                    result.3.clicked(),
                    !pressed,
                    "bottom action must receive the actual click"
                );
            }
        }
    }

    #[test]
    fn scope_keeps_widget_identity_palette_parent_opacity_and_shared_galley() {
        let ctx = context();
        let global_style = ctx.style();
        let mut identities = Vec::new();
        for offset in [-6.0, 6.0, 0.0] {
            let _ = ctx.run(input(0.0, vec![]), |ctx| {
                let mut ui = test_ui(ctx);
                ui.set_opacity(0.5);
                let style = ui.style().clone();
                let galley = ui.painter().layout_no_wrap(
                    "shared".into(),
                    egui::FontId::proportional(14.0),
                    Color32::WHITE,
                );
                let pristine = (*galley).clone();
                let frame = TransitionFrame {
                    opacity: 0.2,
                    offset: vec2(offset, 0.0),
                    active: false,
                };
                let response = show(&mut ui, frame, |ui| {
                    close(ui.opacity(), 0.1);
                    assert_eq!(ui.style(), &style);
                    ui.painter()
                        .galley(ui.min_rect().min, galley.clone(), Color32::WHITE);
                    ui.add(
                        egui::TextEdit::singleline(&mut String::new())
                            .id(Id::new("shared_composer")),
                    )
                });
                identities.push((response.response.id, response.inner.id));
                assert_eq!(*galley, pristine);
                assert_eq!(ui.style(), &style);
                close(ui.opacity(), 0.5);
                assert_eq!(*ctx.style(), *global_style);
            });
        }
        assert!(identities.windows(2).all(|ids| ids[0] == ids[1]));
        assert_eq!(identities[0].1, Id::new("shared_composer"));
    }

    #[test]
    fn text_edit_preedit_commit_cursor_and_ime_geometry_survive_motion() {
        let ctx = context();
        let id = Id::new("fixed_composer_ime_test");
        let mut text = String::from("prefix ");
        ctx.memory_mut(|memory| memory.request_focus(id));
        let mut edit_frame = |time, offset, opacity, events| {
            let mut edit = None;
            let output = ctx.run(input(time, events), |ctx| {
                let frame = TransitionFrame {
                    opacity,
                    offset: vec2(offset, 0.0),
                    active: offset != 0.0,
                };
                edit = Some(
                    show(&mut test_ui(ctx), frame, |ui| {
                        egui::TextEdit::singleline(&mut text).id(id).show(ui)
                    })
                    .inner,
                );
            });
            (output, edit.unwrap(), text.clone())
        };
        let (_, mut initial, _) = edit_frame(0.0, 0.0, 1.0, vec![]);
        initial
            .state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::one(
                egui::text::CCursor::new(7),
            )));
        initial.state.store(&ctx, id);

        let (preedit_output, preedit, value) = edit_frame(
            0.016,
            6.0,
            0.2,
            vec![
                Event::Ime(egui::ImeEvent::Enabled),
                Event::Ime(egui::ImeEvent::Preedit("ni".into())),
            ],
        );
        assert_eq!(value, "prefix ni");
        assert_eq!(preedit.response.id, id);
        assert!(preedit.response.has_focus());
        assert_eq!(preedit.cursor_range.unwrap().as_sorted_char_range(), 7..9);
        let (moved_output, moved, value) = edit_frame(0.032, -6.0, 0.7, vec![]);
        assert_eq!(value, "prefix ni");
        assert_eq!(moved.response.id, id);
        assert!(moved.response.has_focus());
        assert_eq!(moved.cursor_range, preedit.cursor_range);
        assert_eq!(
            moved.response.rect,
            preedit.response.rect.translate(vec2(-12.0, 0.0))
        );
        assert_eq!(moved.galley_pos, preedit.galley_pos + vec2(-12.0, 0.0));
        let preedit_ime = preedit_output.platform_output.ime.unwrap();
        let moved_ime = moved_output.platform_output.ime.unwrap();
        assert_eq!(moved_ime.rect, preedit_ime.rect.translate(vec2(-12.0, 0.0)));
        assert_eq!(
            moved_ime.cursor_rect,
            preedit_ime.cursor_rect.translate(vec2(-12.0, 0.0))
        );

        let (_, committed, value) = edit_frame(
            0.048,
            0.0,
            1.0,
            vec![Event::Ime(egui::ImeEvent::Commit("你".into()))],
        );
        assert_eq!(
            value, "prefix 你",
            "commit must replace preedit exactly once"
        );
        assert_eq!(committed.response.id, id);
        assert!(committed.response.has_focus());
        assert_eq!(committed.cursor_range.unwrap().as_sorted_char_range(), 8..8);
        let (_, idle, value) =
            edit_frame(0.064, 0.0, 1.0, vec![Event::Ime(egui::ImeEvent::Disabled)]);
        assert_eq!(value, "prefix 你");
        assert_eq!(idle.cursor_range, committed.cursor_range);
    }

    #[test]
    fn idle_does_not_schedule_a_repaint_loop_but_active_schedules_a_tick() {
        let ctx = context();
        // egui itself requests a few initial warm-up repaints.
        for step in 0..5 {
            let _ = ctx.run(input(step as f64, vec![]), |ctx| {
                show(&mut test_ui(ctx), TransitionFrame::SETTLED, |_| {});
            });
        }
        let idle = ctx.run(input(5.0, vec![]), |ctx| {
            show(&mut test_ui(ctx), TransitionFrame::SETTLED, |_| {});
        });
        assert_eq!(
            idle.viewport_output[&egui::ViewportId::ROOT].repaint_delay,
            Duration::MAX
        );
        let active = ctx.run(input(6.0, vec![]), |ctx| {
            show(
                &mut test_ui(ctx),
                TransitionFrame {
                    active: true,
                    ..TransitionFrame::SETTLED
                },
                |_| {},
            );
        });
        assert!(
            active.viewport_output[&egui::ViewportId::ROOT].repaint_delay
                <= Duration::from_millis(16)
        );
    }

    #[test]
    fn disabled_style_short_circuits_without_any_native_preference_query() {
        let ctx = context();
        for animation_time in [0.0, -1.0] {
            ctx.style_mut(|style| style.animation_time = animation_time);
            assert!(!motion_enabled(&ctx));
        }
    }

    #[test]
    fn motion_cache_uses_input_clock_and_refreshes_after_five_seconds_or_reset() {
        let first = cached_motion_preference(None, 10.0, || false);
        assert!(!first.enabled);
        for now in [10.0, 10.0, 14.999] {
            let cached =
                cached_motion_preference(Some(first), now, || panic!("cache should be fresh"));
            assert!(!cached.enabled);
        }
        for now in [15.0, 9.0, f64::NAN, f64::INFINITY] {
            assert!(cached_motion_preference(Some(first), now, || true).enabled);
        }
    }
}
