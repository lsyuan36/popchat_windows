//! Distinguish native user resizes from our own layout/monitor commands.
//! Feeding programmatic sizes back into preferences caused cumulative growth
//! and destroyed the user's expanded-height preference after compact mode.

use egui::Vec2;
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct ViewportSizing {
    observed: Option<Vec2>,
    mode: Option<u8>,
    scale: Option<f32>,
    pending: Option<(Vec2, Instant)>,
}

impl ViewportSizing {
    pub fn requested(&mut self, size: Vec2, now: Instant) {
        self.pending = Some((size, now));
    }

    pub fn observe(
        &mut self,
        size: Vec2,
        mode: u8,
        scale: f32,
        monitors_changed: bool,
        explicit_resize: bool,
        now: Instant,
    ) -> bool {
        let stable = self.mode == Some(mode) && self.scale == Some(scale) && !monitors_changed;
        let changed = self
            .observed
            .is_some_and(|previous| (previous - size).length() > 1.0);
        let mut commanded = false;
        if let Some((expected, requested)) = self.pending {
            commanded = now.saturating_duration_since(requested) < Duration::from_secs(1);
            if (expected - size).length() <= 1.0 || !commanded {
                self.pending = None;
            }
        }
        self.observed = Some(size);
        self.mode = Some(mode);
        self.scale = Some(scale);
        let user_resize = stable && changed && (explicit_resize || !commanded);
        if user_resize {
            self.pending = None;
        }
        user_resize
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn command_ack_and_mode_changes_do_not_become_user_preferences() {
        let mut tracker = ViewportSizing::default();
        let now = Instant::now();
        assert!(!tracker.observe(Vec2::new(604.0, 464.0), 1, 1.0, false, false, now));
        tracker.requested(Vec2::new(604.0, 272.0), now);
        assert!(!tracker.observe(Vec2::new(604.0, 272.0), 0, 1.0, false, false, now));
        assert!(!tracker.observe(Vec2::new(604.0, 464.0), 1, 1.0, false, false, now));
        assert!(tracker.observe(Vec2::new(704.0, 564.0), 1, 1.0, false, false, now));
    }
    #[test]
    fn dpi_and_monitor_fit_never_overwrite_saved_geometry() {
        let mut tracker = ViewportSizing::default();
        let now = Instant::now();
        tracker.observe(Vec2::new(704.0, 564.0), 1, 1.0, false, false, now);
        assert!(!tracker.observe(Vec2::new(564.0, 451.0), 1, 1.25, false, false, now));
        assert!(!tracker.observe(Vec2::new(520.0, 400.0), 1, 1.25, true, false, now));
    }
    #[test]
    fn an_explicit_edge_resize_can_override_an_in_flight_size_request() {
        let mut tracker = ViewportSizing::default();
        let now = Instant::now();
        tracker.observe(Vec2::new(604.0, 464.0), 1, 1.0, false, false, now);
        tracker.requested(Vec2::new(604.0, 500.0), now);
        assert!(tracker.observe(Vec2::new(640.0, 490.0), 1, 1.0, false, true, now));
    }

    #[test]
    fn animated_native_size_limits_never_become_saved_user_geometry() {
        let mut tracker = ViewportSizing::default();
        let mut animation = crate::ui::transition::WindowHeightTransition::default();
        let start = Instant::now();
        let mut current = Vec2::new(724.0, 272.0);
        tracker.observe(current, 0, 1.25, false, false, start);
        animation.update(0, current.y, Some(current.y), 0.0, true);
        for step in 0..=15 {
            let elapsed = step as f64 * 0.016;
            let now = start + Duration::from_secs_f64(elapsed);
            assert!(!tracker.observe(current, 1, 1.25, false, false, now));
            let frame = animation.update(1, 644.0, Some(current.y), elapsed, true);
            // Native min/max size changes also need this declaration even when
            // an explicit InnerSize command would be below its 2pt threshold.
            let next = Vec2::new(current.x, frame.height);
            tracker.requested(next, now);
            current = Vec2::new(next.x, (next.y * 1.25).round() / 1.25);
        }
        assert!(!tracker.observe(
            current,
            1,
            1.25,
            false,
            false,
            start + Duration::from_millis(300)
        ));
        assert!(
            tracker.observe(
                current + Vec2::new(40.0, 0.0),
                1,
                1.25,
                false,
                true,
                start + Duration::from_millis(350)
            ),
            "an actual later edge resize must still be saved"
        );
    }
}
