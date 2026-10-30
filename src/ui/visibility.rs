//! Serialized panel transitions. Never call this controller from inside an
//! egui input/output closure: the lock order is controller -> egui, not vice versa.

use egui::{Context, Pos2, Rect, ViewportCommand, ViewportId};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const HIDDEN_POS: Pos2 = egui::pos2(-32000.0, -32000.0);
const SHOW_GRACE: Duration = Duration::from_millis(1200);
const FOCUS_LOSS_DELAY: Duration = Duration::from_millis(600);

/// Read this from egui before entering the controller.
#[derive(Clone, Copy, Default)]
pub struct ViewportSnapshot {
    pub outer_rect: Option<Rect>,
    pub inner_rect: Option<Rect>,
    pub focused: Option<bool>,
    pub minimized: Option<bool>,
}

impl ViewportSnapshot {
    pub fn read(ctx: &Context) -> Self {
        ctx.input(|input| {
            let viewport = input.viewport();
            Self {
                outer_rect: viewport.outer_rect,
                inner_rect: viewport.inner_rect,
                focused: viewport.focused,
                minimized: viewport.minimized,
            }
        })
    }
}

#[derive(Default)]
pub struct UiUpdate {
    pub hidden: bool,
    pub request_composer_focus: bool,
}

struct Activation {
    /// Wait for a complete UI frame, not an egui pass (layout may discard a pass).
    observed_frame: Option<u64>,
}

struct State {
    visible: bool,
    closing: bool,
    generation: u64,
    shown_pos: Pos2,
    /// Ignore the snapshot from the frame which enqueued a native move. Don't
    /// require exact coordinates: changing monitors may also change the DPI.
    position_requested_frame: Option<u64>,
    shown_at: Instant,
    focused_ever: bool,
    unfocused_since: Option<Instant>,
    activation: Option<Activation>,
    settings_requested: bool,
}

pub struct VisibilityController {
    state: Mutex<State>,
}

impl VisibilityController {
    #[allow(dead_code)] // Compatibility constructor used by controller tests/library callers.
    pub fn new(now: Instant) -> Self {
        Self::new_at(now, egui::pos2(100.0, 100.0))
    }

    /// Supply the monitor-validated viewport top-left used by ViewportBuilder.
    /// Unlike the compatibility constructor, the first hide/show restores the
    /// actual launch position even if no native geometry snapshot has arrived.
    pub fn new_at(now: Instant, validated_pos: Pos2) -> Self {
        let shown_pos = if usable_position(validated_pos) {
            validated_pos
        } else {
            egui::pos2(100.0, 100.0)
        };
        Self {
            state: Mutex::new(State {
                visible: true,
                closing: false,
                generation: 1,
                shown_pos,
                position_requested_frame: None,
                shown_at: now,
                focused_ever: false,
                unfocused_since: None,
                activation: Some(Activation {
                    observed_frame: None,
                }),
                settings_requested: false,
            }),
        }
    }

    /// Revalidate against current work areas before show / after monitor changes.
    /// The caller supplies a pure `PanelGeometry::placement` result. Hidden
    /// panels stay hidden; visible panels move without reactivating or resetting
    /// focus-loss safeguards. Generation rejects an obsolete UI frame's restore.
    pub fn restore_position(&self, ctx: &Context, generation: u64, validated_pos: Pos2) -> bool {
        let mut state = self.state.lock().unwrap();
        if state.closing || state.generation != generation || !usable_position(validated_pos) {
            return false;
        }
        state.shown_pos = validated_pos;
        state.position_requested_frame = Some(ctx.cumulative_frame_nr());
        if state.visible {
            ctx.send_viewport_cmd_to(
                ViewportId::ROOT,
                ViewportCommand::OuterPosition(validated_pos),
            );
            ctx.request_repaint_of(ViewportId::ROOT);
        }
        true
    }

    /// Last non-hidden viewport top-left, for persistence without racing a hotkey.
    pub fn shown_position(&self) -> Pos2 {
        self.state.lock().unwrap().shown_pos
    }

    pub fn generation(&self) -> u64 {
        self.state.lock().unwrap().generation
    }

    pub fn is_visible(&self) -> bool {
        self.state.lock().unwrap().visible
    }

    pub fn toggle(&self, ctx: &Context, now: Instant) -> bool {
        let mut state = self.state.lock().unwrap();
        if !state.closing {
            if state.visible {
                state.hide(ctx);
            } else {
                state.show(ctx, now);
            }
        }
        state.visible
    }

    /// A second-instance/reopen request means SHOW, never toggle. A visible
    /// panel receives a fresh serialized activation and composer-focus request.
    pub fn show(&self, ctx: &Context, now: Instant) {
        let mut state = self.state.lock().unwrap();
        if !state.closing {
            state.show(ctx, now);
        }
    }

    /// A drop/paste may focus an already visible draft without becoming a new
    /// SHOW or reapplying its default placement. Hidden panels stay hidden.
    pub fn activate_visible(&self, ctx: &Context, now: Instant) {
        let mut state = self.state.lock().unwrap();
        if state.visible && !state.closing {
            state.activation = Some(Activation {
                observed_frame: None,
            });
            state.shown_at = now;
            state.unfocused_since = None;
            ctx.request_repaint_of(ViewportId::ROOT);
        }
    }

    pub fn open_settings(&self, ctx: &Context, now: Instant) {
        let mut state = self.state.lock().unwrap();
        if !state.closing {
            state.settings_requested = true;
            state.show(ctx, now);
        }
    }

    pub fn take_settings_request(&self) -> bool {
        std::mem::take(&mut self.state.lock().unwrap().settings_requested)
    }

    /// Dismiss only the activation which produced this UI action. A hotkey
    /// show during the frame must not be overwritten by an older Esc/click.
    pub fn dismiss(&self, ctx: &Context, generation: u64, view: ViewportSnapshot) -> bool {
        let mut state = self.state.lock().unwrap();
        if state.closing || !state.visible || state.generation != generation {
            return false;
        }
        state.remember_position(view.outer_rect, ctx.cumulative_frame_nr());
        state.hide(ctx);
        true
    }

    pub fn close(&self, ctx: &Context) {
        let mut state = self.state.lock().unwrap();
        if !state.closing {
            state.closing = true;
            state.activation = None;
            state.generation = state.generation.wrapping_add(1);
            restore_previous_window();
            ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Close);
            ctx.request_repaint_of(ViewportId::ROOT);
        }
    }

    /// Run on the UI thread. The native activation callback runs under the
    /// same lock as hide/show, so an obsolete activation cannot steal focus.
    /// `frame_nr` must be Context::cumulative_frame_nr(), not the pass count.
    pub fn ui_frame(
        &self,
        ctx: &Context,
        generation: u64,
        frame_nr: u64,
        view: ViewportSnapshot,
        now: Instant,
        dismiss_allowed: bool,
        activate: impl FnOnce() -> bool,
    ) -> UiUpdate {
        let mut state = self.state.lock().unwrap();
        if state.closing || !state.visible || state.generation != generation {
            return UiUpdate::default();
        }

        if let Some(activation) = state.activation.as_mut() {
            match activation.observed_frame {
                None => {
                    activation.observed_frame = Some(frame_nr);
                    ctx.request_repaint_of(ViewportId::ROOT);
                }
                Some(frame)
                    if frame_nr > frame
                        && view.minimized != Some(true)
                        && view
                            .outer_rect
                            .is_some_and(|rect| usable_position(rect.min)) =>
                {
                    // The preceding UI frame drained the show commands. Do
                    // not feed its pre-activation focus snapshot into policy.
                    let _ = activate();
                    state.activation = None;
                    state.remember_position(view.outer_rect, frame_nr);
                    ctx.request_repaint_of(ViewportId::ROOT);
                    return UiUpdate {
                        request_composer_focus: true,
                        hidden: false,
                    };
                }
                Some(_) => ctx.request_repaint_after(Duration::from_millis(16)),
            }
            return UiUpdate::default();
        }

        state.remember_position(view.outer_rect, frame_nr);
        match view.focused {
            Some(true) => {
                state.focused_ever = true;
                state.unfocused_since = None;
            }
            Some(false) if state.focused_ever => {
                let since = *state.unfocused_since.get_or_insert(now);
                if dismiss_allowed {
                    if now.saturating_duration_since(since) >= FOCUS_LOSS_DELAY
                        && now.saturating_duration_since(state.shown_at) >= SHOW_GRACE
                    {
                        state.hide(ctx);
                        return UiUpdate {
                            hidden: true,
                            request_composer_focus: false,
                        };
                    }
                    ctx.request_repaint_after(Duration::from_millis(200));
                }
            }
            None => state.unfocused_since = None,
            _ => {}
        }
        UiUpdate::default()
    }
}

impl State {
    fn show(&mut self, ctx: &Context, now: Instant) {
        self.visible = true;
        self.generation = self.generation.wrapping_add(1);
        self.shown_at = now;
        self.focused_ever = false;
        self.unfocused_since = None;
        self.activation = Some(Activation {
            observed_frame: None,
        });
        self.position_requested_frame = Some(ctx.cumulative_frame_nr());
        ctx.send_viewport_cmd_to(
            ViewportId::ROOT,
            ViewportCommand::OuterPosition(self.shown_pos),
        );
        ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Minimized(false));
        // Never use ViewportCommand::Focus: winit 0.30 injects Alt down/up.
        ctx.request_repaint_of(ViewportId::ROOT);
    }

    fn hide(&mut self, ctx: &Context) {
        self.visible = false;
        self.generation = self.generation.wrapping_add(1);
        self.focused_ever = false;
        self.unfocused_since = None;
        self.activation = None;
        ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::OuterPosition(HIDDEN_POS));
        // Never steal focus back after a click into another app: the native
        // helper restores only when our own panel is still foreground.
        restore_previous_window();
        ctx.request_repaint_of(ViewportId::ROOT);
    }

    fn remember_position(&mut self, rect: Option<Rect>, frame_nr: u64) {
        if self
            .position_requested_frame
            .is_some_and(|requested| frame_nr <= requested)
        {
            return;
        }
        if let Some(rect) = rect.filter(|rect| usable_position(rect.min)) {
            self.shown_pos = rect.min;
            self.position_requested_frame = None;
        }
    }
}

fn usable_position(pos: Pos2) -> bool {
    // Negative virtual-desktop coordinates are valid, even beyond -10000 on a
    // many-monitor layout. Reject only our actual hidden/minimized sentinel.
    pos.is_finite() && pos.distance(HIDDEN_POS) > 2.0
}

/// Must be called on the eframe UI thread, after the show commands have been
/// applied. Denied foreground activation leaves the panel visible and does not
/// arm focus-loss dismissal. This function never synthesizes keyboard input.
pub fn activate_window(frame: &eframe::Frame) -> bool {
    #[cfg(windows)]
    {
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let Ok(handle) = frame.window_handle() else {
            return false;
        };
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            return false;
        };
        // SAFETY: eframe owns the live HWND for the duration of this UI call.
        unsafe { native_focus::activate(handle.hwnd.get()) }
    }
    #[cfg(not(windows))]
    {
        let _ = frame;
        false
    }
}

fn restore_previous_window() {
    #[cfg(windows)]
    native_focus::restore();
}

/// A best-effort Windows substitute, NOT a nonactivating NSPanel equivalent.
/// WS_EX_NOACTIVATE/AttachThreadInput/SendInput would compromise reliable typing
/// or Alt+Down state. Use immediate SetForegroundWindow alone in both directions.
#[cfg(windows)]
mod native_focus {
    use std::ffi::c_void;
    use std::sync::Mutex;

    struct Previous {
        panel: isize,
        window: isize,
        process: u32,
    }
    static PREVIOUS: Mutex<Option<Previous>> = Mutex::new(None);

    #[link(name = "user32")]
    extern "system" {
        fn GetForegroundWindow() -> *mut c_void;
        fn SetForegroundWindow(hwnd: *mut c_void) -> i32;
        fn GetWindowThreadProcessId(hwnd: *mut c_void, process: *mut u32) -> u32;
        fn IsWindow(hwnd: *mut c_void) -> i32;
        fn IsWindowVisible(hwnd: *mut c_void) -> i32;
        fn IsIconic(hwnd: *mut c_void) -> i32;
    }

    pub unsafe fn activate(panel: isize) -> bool {
        let mut previous = PREVIOUS.lock().unwrap();
        // SAFETY: getters accept null/invalid HWNDs and report failure. `panel`
        // is the live eframe HWND guaranteed by the caller.
        let foreground = unsafe { GetForegroundWindow() };
        let mut process = 0;
        unsafe { GetWindowThreadProcessId(foreground, &mut process) };
        let success = unsafe { SetForegroundWindow(panel as *mut c_void) } != 0;
        if success && foreground as isize != panel {
            // Don't return to another PopChat/settings window or resurrect an
            // unrelated older capture when native activation was denied.
            *previous = (process != 0 && process != std::process::id()).then_some(Previous {
                panel,
                window: foreground as isize,
                process,
            });
        }
        success
    }

    pub fn restore() {
        let Some(previous) = PREVIOUS.lock().unwrap().take() else {
            return;
        };
        let window = previous.window as *mut c_void;
        let mut process = 0;
        // SAFETY: all HWNDs are revalidated; process comparison reduces handle
        // reuse risk. No focus restoration if the user already selected another
        // app, or closed/minimized the previously foreground window.
        unsafe {
            if GetForegroundWindow() as isize != previous.panel
                || IsWindow(window) == 0
                || IsWindowVisible(window) == 0
                || IsIconic(window) != 0
            {
                return;
            }
            GetWindowThreadProcessId(window, &mut process);
            if process == previous.process {
                let _ = SetForegroundWindow(window);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{mpsc, Arc};

    #[test]
    fn focusing_a_visible_draft_does_not_start_a_new_show_or_move_it() {
        let mut h = Harness::new();
        h.ready();
        let generation = h.controller.generation();
        let position = h.controller.shown_position();
        let activations = h.activations;
        h.controller
            .activate_visible(&h.ctx, h.now + Duration::from_millis(4));
        let commands = h.frame(5, false);
        assert!(!commands
            .iter()
            .any(|cmd| matches!(cmd, ViewportCommand::OuterPosition(_))));
        h.frame(6, true);
        assert_eq!(h.controller.generation(), generation);
        assert_eq!(h.controller.shown_position(), position);
        assert_eq!(h.activations, activations + 1);
        h.controller.dismiss(&h.ctx, generation, h.view(false));
        let hidden_generation = h.controller.generation();
        h.controller
            .activate_visible(&h.ctx, h.now + Duration::from_millis(7));
        assert!(!h.controller.is_visible());
        assert_eq!(h.controller.generation(), hidden_generation);
    }

    struct Harness {
        ctx: Context,
        controller: Arc<VisibilityController>,
        now: Instant,
        position: Pos2,
        activations: usize,
    }

    impl Harness {
        fn new() -> Self {
            let now = Instant::now();
            Self {
                ctx: Context::default(),
                controller: Arc::new(VisibilityController::new(now)),
                now,
                position: egui::pos2(100.0, 100.0),
                activations: 0,
            }
        }

        fn view(&self, focused: bool) -> ViewportSnapshot {
            ViewportSnapshot {
                outer_rect: Some(Rect::from_min_size(self.position, egui::vec2(600.0, 200.0))),
                focused: Some(focused),
                minimized: Some(false),
                ..Default::default()
            }
        }

        fn frame(&mut self, elapsed_ms: u64, focused: bool) -> Vec<ViewportCommand> {
            let view = self.view(focused);
            let generation = self.controller.generation();
            let output = self.ctx.run(egui::RawInput::default(), |ctx| {
                self.controller.ui_frame(
                    ctx,
                    generation,
                    ctx.cumulative_frame_nr(),
                    view,
                    self.now + Duration::from_millis(elapsed_ms),
                    true,
                    || {
                        self.activations += 1;
                        false
                    },
                );
            });
            let commands = output.viewport_output[&ViewportId::ROOT].commands.clone();
            for command in &commands {
                match command {
                    ViewportCommand::OuterPosition(pos) => self.position = *pos,
                    ViewportCommand::Focus => panic!("activation must not synthesize Alt-up"),
                    _ => {}
                }
            }
            commands
        }

        fn ready(&mut self) {
            self.frame(0, true);
            self.frame(1, true);
            self.frame(2, true);
        }
    }

    #[test]
    fn dismissal_and_background_show_keep_state_and_command_order_consistent() {
        let mut h = Harness::new();
        h.ready();
        let old_generation = h.controller.generation();
        assert!(h.controller.dismiss(&h.ctx, old_generation, h.view(false)));
        let (done_tx, done_rx) = mpsc::channel();
        let controller = h.controller.clone();
        let ctx = h.ctx.clone();
        let now = h.now;
        let driver = std::thread::spawn(move || {
            assert!(controller.toggle(&ctx, now));
            done_tx.send(()).unwrap();
        });
        done_rx.recv().unwrap();
        // The old frame resumes after a new show; it must not append a hide.
        assert!(!h.controller.dismiss(&h.ctx, old_generation, h.view(false)));
        let commands = h.frame(10, false);
        driver.join().unwrap();
        let positions: Vec<_> = commands
            .iter()
            .filter_map(|command| match command {
                ViewportCommand::OuterPosition(pos) => Some(*pos),
                _ => None,
            })
            .collect();
        assert_eq!(positions, [HIDDEN_POS, egui::pos2(100.0, 100.0)]);
        assert!(h.controller.is_visible());
        assert_eq!(h.position, egui::pos2(100.0, 100.0));
        assert!(!h.controller.toggle(&h.ctx, h.now));
        h.frame(11, false);
        assert_eq!(h.position, HIDDEN_POS);
    }

    #[test]
    fn transition_lock_covers_the_gap_before_egui_enqueues_the_position() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let mut h = Harness::new();
        h.ready();
        h.frame(3, true); // Let egui's outstanding repaint settle.
        let generation = h.controller.generation();
        let view = h.view(false);
        let (paused_tx, paused_rx) = mpsc::channel();
        let (resume_tx, resume_rx) = mpsc::channel();
        let resume_rx = Mutex::new(resume_rx);
        let first_wake = AtomicBool::new(true);
        h.ctx.set_request_repaint_callback(move |_| {
            if first_wake.swap(false, Ordering::SeqCst) {
                // egui calls this before pushing the command into its queue.
                paused_tx.send(()).unwrap();
                resume_rx.lock().unwrap().recv().unwrap();
            }
        });
        let controller = h.controller.clone();
        let ctx = h.ctx.clone();
        let ui = std::thread::spawn(move || controller.dismiss(&ctx, generation, view));
        paused_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        // This is the old race window: flag updated, command not yet queued.
        assert!(h.controller.state.try_lock().is_err());
        let (attempt_tx, attempt_rx) = mpsc::channel();
        let controller = h.controller.clone();
        let ctx = h.ctx.clone();
        let now = h.now;
        let driver = std::thread::spawn(move || {
            attempt_tx.send(()).unwrap();
            controller.toggle(&ctx, now)
        });
        attempt_rx.recv().unwrap();
        resume_tx.send(()).unwrap();
        assert!(ui.join().unwrap());
        assert!(driver.join().unwrap());
        let commands = h.frame(10, false);
        let positions: Vec<_> = commands
            .iter()
            .filter_map(|command| match command {
                ViewportCommand::OuterPosition(pos) => Some(*pos),
                _ => None,
            })
            .collect();
        assert_eq!(positions, [HIDDEN_POS, egui::pos2(100.0, 100.0)]);
        assert!(h.controller.is_visible());
        assert_ne!(h.position, HIDDEN_POS);
    }

    #[test]
    fn every_show_resets_focus_history_and_denied_activation_stays_visible() {
        let mut h = Harness::new();
        h.ready();
        assert!(!h.controller.toggle(&h.ctx, h.now));
        h.frame(100, true); // Off-screen HWND may still retain focus.
        h.frame(2000, false);
        assert!(h
            .controller
            .toggle(&h.ctx, h.now + Duration::from_millis(2100)));
        h.frame(2100, false);
        h.frame(2101, false); // Native activation callback returns false.
        h.frame(10000, false);
        assert!(h.controller.is_visible());
        assert_ne!(h.position, HIDDEN_POS);
        h.frame(10001, true); // This activation now actually acquires focus.
        h.frame(11000, false);
        h.frame(11599, false);
        assert!(h.controller.is_visible());
        h.frame(11600, false);
        assert!(!h.controller.is_visible());
        assert_eq!(h.position, HIDDEN_POS);
    }

    #[test]
    fn focus_loss_starts_at_the_transition_and_focus_gain_resets_it() {
        let mut h = Harness::new();
        h.ready();
        h.frame(10000, false); // Last focused frame was ~10s ago, not 600ms of blur.
        h.frame(10599, false);
        assert!(h.controller.is_visible());
        h.frame(10599, true);
        h.frame(10600, false);
        h.frame(11199, false);
        assert!(h.controller.is_visible());
        h.frame(11200, false);
        assert!(!h.controller.is_visible());
        assert_eq!(h.position, HIDDEN_POS);
    }

    #[test]
    fn focus_dismissal_waits_for_both_the_show_grace_and_continuous_loss() {
        let mut h = Harness::new();
        h.ready();
        h.frame(10, false);
        h.frame(610, false);
        h.frame(1199, false);
        assert!(h.controller.is_visible());
        h.frame(1200, false);
        assert_eq!(h.position, HIDDEN_POS);
        assert!(!h.controller.is_visible());
    }

    #[test]
    fn discarded_layout_pass_does_not_activate_before_show_commands_are_applied() {
        let mut h = Harness::new();
        h.ready();
        h.controller.toggle(&h.ctx, h.now);
        h.frame(10, false);
        h.controller.toggle(&h.ctx, h.now);
        let generation = h.controller.generation();
        let view = h.view(false);
        let mut passes = 0;
        let activations = h.activations;
        let output = h.ctx.run(egui::RawInput::default(), |ctx| {
            passes += 1;
            h.controller.ui_frame(
                ctx,
                generation,
                ctx.cumulative_frame_nr(),
                view,
                h.now,
                true,
                || {
                    h.activations += 1;
                    false
                },
            );
            if ctx.current_pass_index() == 0 {
                ctx.request_discard("exercise a second layout pass");
            }
        });
        assert_eq!(passes, 2);
        assert_eq!(h.activations, activations);
        for command in &output.viewport_output[&ViewportId::ROOT].commands {
            if let ViewportCommand::OuterPosition(pos) = command {
                h.position = *pos;
            }
        }
        assert_ne!(h.position, HIDDEN_POS);
        h.frame(11, false);
        assert_eq!(h.activations, activations + 1);
    }

    #[test]
    fn activation_waits_for_a_completed_show_frame_and_is_cancelled_by_hide() {
        let mut h = Harness::new();
        h.ready();
        let activations = h.activations;
        h.controller.toggle(&h.ctx, h.now);
        h.frame(10, false);
        h.controller.toggle(&h.ctx, h.now);
        h.frame(11, false); // Drain show commands, don't activate an off-screen HWND.
        assert_eq!(h.activations, activations);
        h.controller.toggle(&h.ctx, h.now); // Cancel before deferred activation.
        h.frame(12, false);
        assert_eq!(h.activations, activations);
        assert_eq!(h.position, HIDDEN_POS);
        h.controller.toggle(&h.ctx, h.now);
        h.frame(13, false);
        assert_eq!(h.activations, activations);
        h.frame(14, false);
        assert_eq!(h.activations, activations + 1);
        assert!(h.controller.is_visible());
    }

    #[test]
    fn rapid_toggles_keep_every_press_and_never_emit_focus_commands() {
        let mut h = Harness::new();
        h.ready();
        for _ in 0..8 {
            h.controller.toggle(&h.ctx, h.now);
        }
        let commands = h.frame(120, false);
        assert_eq!(
            commands
                .iter()
                .filter(|c| matches!(c, ViewportCommand::OuterPosition(_)))
                .count(),
            8
        );
        assert!(h.controller.is_visible());
        assert_ne!(h.position, HIDDEN_POS);
        h.controller.close(&h.ctx);
        h.controller.toggle(&h.ctx, h.now);
        let commands = h.frame(121, false);
        assert!(commands.iter().any(|c| matches!(c, ViewportCommand::Close)));
        assert!(!commands
            .iter()
            .any(|c| matches!(c, ViewportCommand::OuterPosition(_))));
    }

    #[test]
    fn initial_validated_negative_position_survives_hide_before_first_snapshot() {
        let mut h = Harness::new();
        let pos = egui::pos2(-14500.0, -1800.0);
        h.controller = Arc::new(VisibilityController::new_at(h.now, pos));
        h.position = pos;
        h.controller.toggle(&h.ctx, h.now);
        h.controller.toggle(&h.ctx, h.now);
        h.frame(0, false);
        h.frame(1, false);
        assert_eq!(h.position, pos);
        assert_eq!(h.controller.shown_position(), pos);
        assert_eq!(h.activations, 1);
    }

    #[test]
    fn monitor_restore_rejects_stale_generation_and_never_unhides_a_hidden_panel() {
        let mut h = Harness::new();
        h.ready();
        let stale = h.controller.generation();
        h.controller.toggle(&h.ctx, h.now);
        let pos = egui::pos2(-1800.0, 40.0);
        assert!(!h.controller.restore_position(&h.ctx, stale, pos));
        assert!(h
            .controller
            .restore_position(&h.ctx, h.controller.generation(), pos));
        h.frame(10, false);
        assert!(!h.controller.is_visible());
        assert_eq!(h.position, HIDDEN_POS);
        assert_eq!(h.controller.shown_position(), pos);
        h.controller.toggle(&h.ctx, h.now);
        h.frame(20, false);
        assert_eq!(h.position, pos);
    }

    #[test]
    fn pre_restore_snapshot_does_not_overwrite_position_or_activate_in_same_frame() {
        let mut h = Harness::new();
        let pos = egui::pos2(1200.0, 400.0);
        assert!(h
            .controller
            .restore_position(&h.ctx, h.controller.generation(), pos));
        // The first frame observes native coordinates from before restore.
        h.frame(0, false);
        assert_eq!(h.activations, 0);
        assert_eq!(h.controller.shown_position(), pos);
        h.frame(1, false);
        assert_eq!(h.activations, 1);
        assert_eq!(h.controller.shown_position(), pos);
    }

    #[test]
    fn cross_monitor_dpi_change_does_not_block_deferred_native_activation() {
        let mut h = Harness::new();
        h.ready();
        h.controller.toggle(&h.ctx, h.now);
        h.frame(10, false);
        let target = egui::pos2(2400.0, 200.0);
        h.controller
            .restore_position(&h.ctx, h.controller.generation(), target);
        h.controller.show(&h.ctx, h.now);
        let activations = h.activations;
        h.frame(20, false);
        // Same physical origin, now reported in the destination monitor's points.
        h.position = target / 2.0;
        h.frame(21, false);
        assert_eq!(h.activations, activations + 1);
        assert_eq!(h.controller.shown_position(), target / 2.0);
    }

    #[test]
    fn unknown_focus_breaks_the_uninterrupted_blur_interval() {
        let mut h = Harness::new();
        h.ready();
        h.frame(2000, false);
        let generation = h.controller.generation();
        let mut view = h.view(false);
        view.focused = None;
        h.controller.ui_frame(
            &h.ctx,
            generation,
            100,
            view,
            h.now + Duration::from_millis(2500),
            true,
            || false,
        );
        h.frame(2600, false);
        h.frame(3199, false);
        assert!(h.controller.is_visible());
        h.frame(3200, false);
        assert!(!h.controller.is_visible());
    }

    #[test]
    fn monitor_restore_during_blur_does_not_restart_focus_loss_grace() {
        let mut h = Harness::new();
        h.ready();
        h.frame(2000, false);
        let pos = egui::pos2(-1200.0, 100.0);
        assert!(h
            .controller
            .restore_position(&h.ctx, h.controller.generation(), pos));
        h.frame(2500, false);
        h.frame(2599, false);
        assert!(h.controller.is_visible());
        h.frame(2600, false);
        assert!(!h.controller.is_visible());
        assert_eq!(h.controller.shown_position(), pos);
    }

    #[test]
    fn reopen_is_show_not_toggle_and_refocuses_an_already_visible_panel() {
        let mut h = Harness::new();
        h.ready();
        let generation = h.controller.generation();
        let activations = h.activations;
        h.frame(2000, false);
        h.controller
            .show(&h.ctx, h.now + Duration::from_millis(2500));
        assert!(h.controller.is_visible());
        assert_ne!(h.controller.generation(), generation);
        assert!(!h.controller.take_settings_request());
        assert!(!h.controller.dismiss(&h.ctx, generation, h.view(false)));
        h.frame(2500, false);
        h.frame(2501, false);
        assert_eq!(h.activations, activations + 1);
        h.frame(10000, false); // A denied refocus never arms automatic hiding.
        assert!(h.controller.is_visible());
    }

    #[test]
    fn reopen_shows_hidden_panel_without_requesting_settings() {
        let mut h = Harness::new();
        h.ready();
        h.controller.toggle(&h.ctx, h.now);
        h.frame(10, false);
        h.controller.show(&h.ctx, h.now + Duration::from_millis(20));
        h.frame(20, false);
        h.frame(21, false);
        assert!(h.controller.is_visible());
        assert_eq!(h.position, egui::pos2(100.0, 100.0));
        assert!(!h.controller.take_settings_request());
    }
}
