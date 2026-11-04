//! Context-local shortcut capture. Call `intercept` BEFORE every app shortcut.
//!
//! Recording edits only the caller's draft. Registration and persistence remain
//! explicit Apply actions. Install the manager's suppression flag once with
//! `set_suppression_flag`; arming publishes suppression immediately (not next
//! frame), and keeps it through the final captured frame and key release.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use egui::{Context, Event, Id, Key, Modifiers, Ui};

#[derive(Clone, Default)]
struct Recording {
    active: bool,
    owner: Option<Id>,
    pending: Option<String>,
    error: Option<String>,
    preview: String,
    blocked_frame: Option<u64>,
    // Keep the OS guard armed until the accepted key is released. Other keys
    // and ordinary text input resume on the very next frame.
    release_key: Option<Key>,
    global_release_frame: Option<u64>,
    intercepted_frame: Option<u64>,
    suppression: Option<Arc<AtomicBool>>,
}

fn state_id() -> Id {
    Id::new("popchat_hotkey_recorder")
}

fn state(ctx: &Context) -> Recording {
    ctx.data_mut(|data| data.get_temp::<Recording>(state_id()).unwrap_or_default())
}

fn store(ctx: &Context, state: Recording) {
    if let Some(flag) = &state.suppression {
        flag.store(
            state.active
                || state.release_key.is_some()
                || state.blocked_frame == Some(ctx.cumulative_frame_nr())
                || state.global_release_frame == Some(ctx.cumulative_frame_nr()),
            Ordering::SeqCst,
        );
    }
    ctx.data_mut(|data| data.insert_temp(state_id(), state));
}

/// Connect the SAME flag to `HotkeyManager` and the visibility-driver consumer.
/// This does not construct a manager or make any OS registration calls.
pub fn set_suppression_flag(ctx: &Context, flag: Arc<AtomicBool>) {
    let mut recording = state(ctx);
    if let Some(previous) = &recording.suppression {
        if !Arc::ptr_eq(previous, &flag) {
            previous.store(false, Ordering::SeqCst);
        }
    }
    recording.suppression = Some(flag);
    store(ctx, recording);
}

pub fn is_recording(ctx: &Context) -> bool {
    state(ctx).active
}

/// True also on the accepted/cancelled frame, for native paste/other callbacks
/// that do not come from egui's event queue.
pub fn is_suppressing(ctx: &Context) -> bool {
    let recording = state(ctx);
    recording.active || recording.blocked_frame == Some(ctx.cumulative_frame_nr())
}

/// Call when leaving the hotkey tab, closing settings, hiding, or shutting down.
/// The draft is untouched; this frame's captured keys remain consumed.
pub fn cancel(ctx: &Context) {
    let mut recording = state(ctx);
    if !recording.active
        && recording.pending.is_none()
        && recording.error.is_none()
        && recording.owner.is_none()
    {
        return;
    }
    if recording.active {
        recording.blocked_frame = Some(ctx.cumulative_frame_nr());
    }
    recording.active = false;
    recording.pending = None;
    recording.error = None;
    surrender_focus(ctx, recording.owner);
    recording.owner = None;
    store(ctx, recording);
    ctx.request_repaint();
}

fn surrender_focus(ctx: &Context, owner: Option<Id>) {
    if let Some(owner) = owner {
        ctx.memory_mut(|memory| memory.surrender_focus(owner));
    }
}

fn modifiers(mut mods: Modifiers) -> Modifiers {
    // egui-winit drops Super on Windows. Use the same native read as local
    // hotkey consumption; this is a read only, never a synthetic keypress.
    mods.mac_cmd |= super::hotkey::windows_key_down();
    mods
}

fn preview(mods: Modifiers) -> String {
    let mut names = Vec::new();
    for (down, label) in [
        (mods.ctrl, "Ctrl"),
        (mods.alt, "Alt"),
        (mods.shift, "Shift"),
        (mods.mac_cmd, "Win"),
    ] {
        if down {
            names.push(label);
        }
    }
    names.push("…");
    names.join(" + ")
}

/// Must run at the START of `App::update`, before `consume_local_binding`,
/// `poll_events`, composer handling, and all other keyboard shortcuts.
/// Returns whether this frame belongs to the recorder. Repeated calls/passes
/// are safe; the entire acceptance frame remains stripped of keyboard input.
pub fn intercept(ctx: &Context) -> bool {
    let frame = ctx.cumulative_frame_nr();
    let mut recording = state(ctx);
    if let Some(held) = recording.release_key {
        let released = ctx.input(|input| {
            !input.focused
                || input.events.iter().any(
                    |event| matches!(event, Event::Key { key, pressed: false, .. } if *key == held),
                )
        });
        // A held capture must not become a local repeated Ctrl+N/etc after
        // acceptance, but must not disable unrelated ordinary typing either.
        ctx.input_mut(|input| {
            let retain = |event: &Event| !matches!(event, Event::Key { key, .. } if *key == held);
            input.events.retain(retain);
            input.raw.events.retain(retain);
        });
        if released {
            recording.release_key = None;
            recording.global_release_frame = Some(frame);
            ctx.request_repaint();
        }
    }
    let blocked = recording.active || recording.blocked_frame == Some(frame);
    if !blocked {
        // Release suppression on the next ordinary frame, not during show().
        recording.blocked_frame = None;
        store(ctx, recording);
        return false;
    }

    let mut allow_tab = false;
    if recording.active && recording.intercepted_frame != Some(frame) {
        let (focused, events, current_mods) =
            ctx.input(|input| (input.focused, input.events.clone(), input.modifiers));
        recording.preview = preview(modifiers(current_mods));
        allow_tab = events.iter().any(|event| {
            matches!(
                event,
                Event::Key {
                    key: Key::Tab,
                    pressed: true,
                    ..
                }
            )
        });
        // egui clears focus at begin_pass on Escape, even Ctrl+Escape, before
        // we can inspect it (a filter cannot be installed on the first focused
        // frame). Do not mistake that automatic clearing for user focus loss.
        let escape_cleared_focus = ctx.memory(|memory| memory.focused().is_none())
            && events.iter().any(|event| {
                matches!(
                    event,
                    Event::Key {
                        key: Key::Escape,
                        pressed: true,
                        ..
                    }
                )
            });
        let lost_widget_focus = !escape_cleared_focus
            && recording
                .owner
                .is_some_and(|owner| ctx.memory(|memory| memory.focused() != Some(owner)));
        if !focused || lost_widget_focus || allow_tab {
            recording.active = false;
        } else {
            for event in events {
                let Event::Key {
                    key,
                    physical_key,
                    pressed: true,
                    repeat: false,
                    modifiers: mods,
                } = event
                else {
                    continue;
                };
                let mods = modifiers(mods);
                if key == Key::Escape && !mods.ctrl && !mods.alt && !mods.shift && !mods.mac_cmd {
                    recording.active = false;
                    break;
                }
                match super::hotkey::binding_from_key(physical_key.unwrap_or(key), mods) {
                    Ok(binding) => {
                        recording.pending = Some(binding);
                        let already_released = ctx.input(|input| input.events.iter().any(|event| {
                            matches!(event, Event::Key { key: released, pressed: false, .. } if *released == key)
                        }));
                        if already_released {
                            recording.release_key = None;
                            recording.global_release_frame = Some(frame);
                        } else {
                            recording.release_key = Some(key);
                        }
                        recording.error = None;
                        recording.active = false;
                        break;
                    }
                    Err(error) => recording.error = Some(error.to_string()),
                }
            }
        }
        if !recording.active {
            surrender_focus(ctx, recording.owner);
            recording.owner = None;
        }
    }
    recording.intercepted_frame = Some(frame);
    recording.blocked_frame = Some(frame);
    // Keep pointer events so Cancel, tabs and other controls stay usable.
    // Tab cancels and still participates in normal focus navigation.
    ctx.input_mut(|input| {
        let retain = |event: &Event| match event {
            Event::Key { key: Key::Tab, .. } if allow_tab => true,
            Event::Key { .. }
            | Event::Text(_)
            | Event::Ime(_)
            | Event::Paste(_)
            | Event::Copy
            | Event::Cut => false,
            _ => true,
        };
        input.events.retain(retain);
        input.raw.events.retain(retain);
    });
    store(ctx, recording);
    ctx.request_repaint();
    true
}

/// Record button + modifier preview/error. Returns true only if capture changed
/// `draft`; never registers, saves preferences, or triggers an app action.
pub fn show(ui: &mut Ui, draft: &mut String) -> bool {
    let ctx = ui.ctx().clone();
    intercept(&ctx);
    let mut recording = state(&ctx);
    let mut changed = if let Some(binding) = recording.pending.take() {
        let changed = *draft != binding;
        *draft = binding;
        changed
    } else {
        false
    };
    let mut cancel_clicked = false;
    ui.horizontal_wrapped(|ui| {
        let response = ui.button(if recording.active {
            "重新錄製"
        } else {
            "錄製快捷鍵"
        });
        if response.clicked() {
            recording.active = true;
            recording.owner = Some(response.id);
            recording.pending = None;
            recording.error = None;
            recording.preview = preview(modifiers(ui.input(|input| input.modifiers)));
            // Ignore keys from the click/activation frame. Only a subsequent
            // physical key press can become the candidate.
            recording.intercepted_frame = Some(ctx.cumulative_frame_nr());
            recording.blocked_frame = Some(ctx.cumulative_frame_nr());
            response.request_focus();
            store(&ctx, recording.clone());
            ctx.request_repaint();
        }
        if recording.active {
            cancel_clicked = ui.button("取消錄製").clicked();
            ui.label(egui::RichText::new(&recording.preview).monospace().strong());
        } else {
            // Some Windows/system shortcuts never reach the window as a key
            // event. Keep an explicit draft editor as a fallback; Apply still
            // owns validation, OS registration and persistence.
            changed |= ui
                .add(
                    egui::TextEdit::singleline(draft)
                        .id_salt("manual_hotkey_draft")
                        .font(egui::TextStyle::Monospace)
                        .hint_text("Alt+Left")
                        .desired_width((ui.available_size_before_wrap().x - 8.0).max(64.0)),
                )
                .on_hover_text("待套用快捷鍵：可錄製或直接輸入，例如 Alt+Left；仍需按「套用」。")
                .changed();
        }
    });
    if recording.active {
        if let Some(owner) = recording.owner {
            ui.memory_mut(|memory| {
                memory.set_focus_lock_filter(
                    owner,
                    egui::EventFilter {
                        tab: false,
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        escape: true,
                    },
                )
            });
        }
        super::preference_layout::caption(
            ui,
            "請按下快捷鍵組合；Esc 取消，Tab 離開。錄製後仍需按「套用」。",
        );
        if let Some(error) = &recording.error {
            ui.add(
                egui::Label::new(egui::RichText::new(error).color(ui.visuals().error_fg_color))
                    .wrap(),
            );
        }
    }
    store(&ctx, recording);
    if cancel_clicked {
        cancel(&ctx);
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Harness {
        ctx: Context,
        draft: String,
        flag: Arc<AtomicBool>,
        leaked: usize,
    }

    impl Harness {
        fn new() -> Self {
            let ctx = Context::default();
            let flag = Arc::new(AtomicBool::new(false));
            set_suppression_flag(&ctx, flag.clone());
            Self {
                ctx,
                draft: "Alt+Down".into(),
                flag,
                leaked: 0,
            }
        }

        fn frame(
            &mut self,
            events: Vec<Event>,
            mods: Modifiers,
            focused: bool,
        ) -> (egui::FullOutput, bool, bool) {
            let mut changed = false;
            let mut blocked = false;
            let output = self.ctx.run(
                egui::RawInput {
                    events,
                    modifiers: mods,
                    focused,
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(520.0, 400.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    blocked = intercept(ctx);
                    // Stand-ins for main.poll_events and composer callbacks, run
                    // BEFORE show just like production. No application/OS actions.
                    for key in [Key::N, Key::Y, Key::W, Key::Enter] {
                        if ctx.input(|input| input.key_pressed(key)) {
                            self.leaked += 1;
                        }
                    }
                    if blocked {
                        assert!(!ctx.input(|input| input.events.iter().any(|event| matches!(
                            event,
                            Event::Text(_)
                                | Event::Ime(_)
                                | Event::Paste(_)
                                | Event::Copy
                                | Event::Cut
                        ))));
                    }
                    egui::CentralPanel::default().show(ctx, |ui| {
                        changed = show(ui, &mut self.draft);
                        let _ = ui.button("下一個控制項");
                    });
                },
            );
            (output, changed, blocked)
        }

        fn idle(&mut self) -> egui::FullOutput {
            self.frame(vec![], Modifiers::NONE, true).0
        }

        fn click(&mut self, label: &str) {
            let output = self.idle();
            let pos = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == label => {
                        Some(text.pos + text.galley.size() * 0.5)
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("missing button {label}"));
            for pressed in [true, false] {
                self.frame(
                    vec![
                        Event::PointerMoved(pos),
                        Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Modifiers::NONE,
                        },
                    ],
                    Modifiers::NONE,
                    true,
                );
            }
        }

        fn arm(&mut self) {
            self.idle();
            self.click("錄製快捷鍵");
            assert!(is_recording(&self.ctx));
            assert!(
                self.flag.load(Ordering::SeqCst),
                "suppressed immediately on click"
            );
        }
    }

    fn key(key: Key, mods: Modifiers) -> Event {
        Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers: mods,
        }
    }

    fn release(key: Key) -> Event {
        Event::Key {
            key,
            physical_key: Some(key),
            pressed: false,
            repeat: false,
            modifiers: Modifiers::NONE,
        }
    }

    #[test]
    fn real_click_then_capture_changes_only_draft_and_swallows_entire_final_frame() {
        let mut h = Harness::new();
        h.arm();
        let (_, changed, blocked) = h.frame(
            vec![
                key(Key::N, Modifiers::CTRL),
                key(Key::Enter, Modifiers::NONE),
                Event::Text("must not insert".into()),
                Event::Ime(egui::ImeEvent::Commit("輸入".into())),
                Event::Paste("must not paste".into()),
                Event::Copy,
                Event::Cut,
            ],
            Modifiers::CTRL,
            true,
        );
        assert!(changed && blocked);
        assert_eq!(h.draft, "Ctrl+N");
        assert!(!is_recording(&h.ctx));
        assert_eq!(
            h.leaked, 0,
            "neither main's Ctrl+N nor composer Enter callback may fire"
        );
        assert!(
            h.flag.load(Ordering::SeqCst),
            "acceptance frame still suppresses OS driver"
        );
        let (_, changed, blocked) = h.frame(
            vec![Event::Text("ordinary input".into())],
            Modifiers::NONE,
            true,
        );
        assert!(!blocked && !changed);
        assert!(h
            .ctx
            .input(|input| input.events.contains(&Event::Text("ordinary input".into()))));
        assert!(
            h.flag.load(Ordering::SeqCst),
            "OS stays suppressed while accepted N is held"
        );
        h.frame(vec![key(Key::N, Modifiers::CTRL)], Modifiers::CTRL, true);
        assert_eq!(h.leaked, 0, "held captured key cannot leak as autorepeat");
        h.frame(vec![release(Key::N)], Modifiers::NONE, true);
        assert!(
            h.flag.load(Ordering::SeqCst),
            "release frame is guarded too"
        );
        h.idle();
        assert!(!h.flag.load(Ordering::SeqCst));
    }

    #[test]
    fn physical_key_and_all_modifiers_are_canonical_and_command_is_not_win() {
        let mut h = Harness::new();
        h.arm();
        let mods = Modifiers {
            ctrl: true,
            alt: true,
            shift: true,
            mac_cmd: true,
            command: true,
        };
        h.frame(
            vec![Event::Key {
                key: Key::O,
                physical_key: Some(Key::P),
                pressed: true,
                repeat: false,
                modifiers: mods,
            }],
            mods,
            true,
        );
        assert_eq!(h.draft, "Ctrl+Alt+Shift+Win+P");
        h.arm();
        h.frame(
            vec![Event::Key {
                key: Key::Comma,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers {
                    ctrl: true,
                    command: true,
                    ..Default::default()
                },
            }],
            Modifiers::CTRL,
            true,
        );
        assert_eq!(h.draft, "Ctrl+Comma");
    }

    #[test]
    fn alt_arrows_are_captured_immediately_after_arming_without_focus_navigation() {
        for (key_code, expected) in [
            (Key::ArrowLeft, "Alt+Left"),
            (Key::ArrowRight, "Alt+Right"),
            (Key::ArrowUp, "Alt+Up"),
            (Key::ArrowDown, "Alt+Down"),
        ] {
            let mut h = Harness::new();
            h.arm();
            let (_, changed, blocked) =
                h.frame(vec![key(key_code, Modifiers::ALT)], Modifiers::ALT, true);
            assert!(blocked);
            assert_eq!(changed, expected != "Alt+Down");
            assert_eq!(h.draft, expected);
            assert!(!is_recording(&h.ctx));
            assert_eq!(h.leaked, 0);
            h.frame(vec![release(key_code)], Modifiers::NONE, true);
            h.idle();
            assert!(!h.flag.load(Ordering::SeqCst));
        }
    }

    #[test]
    fn direct_text_input_can_set_alt_left_when_native_capture_is_unavailable() {
        let mut h = Harness::new();
        h.click("Alt+Down");
        // egui-winit aliases Windows Ctrl to `command` for text editing.
        let select_all = Modifiers {
            ctrl: true,
            command: true,
            ..Modifiers::NONE
        };
        let (_, changed, blocked) = h.frame(
            vec![key(Key::A, select_all), Event::Paste("Alt+Left".into())],
            select_all,
            true,
        );
        assert!(changed && !blocked);
        assert_eq!(h.draft, "Alt+Left");
        assert!(!is_recording(&h.ctx));
        assert!(!h.flag.load(Ordering::SeqCst));
        assert_eq!(
            super::super::hotkey::normalize_binding(&h.draft).unwrap(),
            "Alt+Left"
        );
    }

    #[test]
    fn record_button_and_manual_draft_share_one_row_inside_narrow_settings() {
        for width in [250.0, 450.0] {
            let ctx = Context::default();
            let mut draft = "Alt+Down".to_owned();
            let mut rect = egui::Rect::NOTHING;
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width + 16.0, 160.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        ui.spacing_mut().interact_size.y = 32.0;
                        rect = ui
                            .scope(|ui| {
                                assert!(!show(ui, &mut draft));
                            })
                            .response
                            .rect;
                    });
                },
            );
            assert!(
                rect.height() <= 36.0,
                "recorder should stay on one row: {rect:?}"
            );
            for shape in &output.shapes {
                if let egui::Shape::Text(text) = &shape.shape {
                    assert!(
                        text.pos.x >= 8.0 && text.pos.x + text.galley.size().x <= width + 8.1,
                        "manual draft must not overflow: {}",
                        text.galley.job.text
                    );
                }
            }
        }
    }

    #[test]
    fn invalid_reserved_unsupported_and_held_repeat_keep_recording_and_old_draft() {
        let mut h = Harness::new();
        h.arm();
        for (key_code, mods) in [
            (Key::A, Modifiers::NONE),
            (Key::Space, Modifiers::ALT),
            (Key::F25, Modifiers::CTRL),
        ] {
            let (_, changed, blocked) = h.frame(vec![key(key_code, mods)], mods, true);
            assert!(!changed && blocked && is_recording(&h.ctx));
            assert_eq!(h.draft, "Alt+Down");
            assert!(state(&h.ctx).error.is_some());
        }
        // A remains down from the invalid initial press: egui marks this as
        // autorepeat even with new modifiers. It must NOT suddenly bind Ctrl+A.
        h.frame(vec![key(Key::A, Modifiers::CTRL)], Modifiers::CTRL, true);
        assert!(is_recording(&h.ctx));
        assert_eq!(h.draft, "Alt+Down");
        h.frame(
            vec![Event::Key {
                key: Key::A,
                physical_key: Some(Key::A),
                pressed: false,
                repeat: false,
                modifiers: Modifiers::CTRL,
            }],
            Modifiers::CTRL,
            true,
        );
        h.frame(vec![key(Key::A, Modifiers::CTRL)], Modifiers::CTRL, true);
        assert_eq!(h.draft, "Ctrl+A");
    }

    #[test]
    fn modifier_preview_escape_cancel_and_modified_escape_binding() {
        let mut h = Harness::new();
        h.arm();
        h.frame(vec![], Modifiers::CTRL | Modifiers::SHIFT, true);
        assert_eq!(state(&h.ctx).preview, "Ctrl + Shift + …");
        assert_eq!(h.draft, "Alt+Down");
        h.frame(
            vec![key(Key::Escape, Modifiers::NONE)],
            Modifiers::NONE,
            true,
        );
        assert!(!is_recording(&h.ctx));
        assert_eq!(h.draft, "Alt+Down");
        assert!(!h.ctx.input(|input| input.key_pressed(Key::Escape)));
        h.arm();
        h.frame(
            vec![
                Event::Key {
                    key: Key::Escape,
                    physical_key: Some(Key::Escape),
                    pressed: false,
                    repeat: false,
                    modifiers: Modifiers::CTRL,
                },
                key(Key::Escape, Modifiers::CTRL),
            ],
            Modifiers::CTRL,
            true,
        );
        assert_eq!(h.draft, "Ctrl+Escape");
    }

    #[test]
    fn tab_focus_loss_cancel_button_and_host_cancel_preserve_draft() {
        for cause in 0..5 {
            let mut h = Harness::new();
            h.arm();
            match cause {
                0 => {
                    h.frame(vec![key(Key::Tab, Modifiers::NONE)], Modifiers::NONE, true);
                }
                1 => {
                    h.frame(vec![Event::WindowFocused(false)], Modifiers::NONE, false);
                }
                2 => {
                    h.ctx
                        .memory_mut(|memory| memory.request_focus(Id::new("other-field")));
                    h.idle();
                }
                3 => h.click("取消錄製"),
                _ => cancel(&h.ctx),
            }
            assert!(!is_recording(&h.ctx), "cause {cause}");
            assert_eq!(h.draft, "Alt+Down");
            h.idle();
            if cause == 4 {
                // Host cancellation outside ctx.run targets the upcoming frame.
                h.idle();
            }
            assert!(!h.flag.load(Ordering::SeqCst), "cause {cause}");
        }
    }

    #[test]
    fn state_is_context_local_and_existing_global_binding_is_capturable() {
        let mut h = Harness::new();
        let other = Context::default();
        h.arm();
        assert!(!is_recording(&other));
        let (_, changed, blocked) = h.frame(
            vec![key(Key::ArrowDown, Modifiers::ALT)],
            Modifiers::ALT,
            true,
        );
        assert!(!changed && blocked);
        assert_eq!(h.draft, "Alt+Down");
        assert!(!is_recording(&h.ctx));
        assert!(h.flag.load(Ordering::SeqCst));
        h.frame(vec![release(Key::ArrowDown)], Modifiers::NONE, true);
        assert!(h.flag.load(Ordering::SeqCst));
        h.idle();
        assert!(!h.flag.load(Ordering::SeqCst));
    }

    #[test]
    fn re_record_discards_error_and_same_frame_release_does_not_stick_suppression() {
        let mut h = Harness::new();
        h.arm();
        h.frame(vec![key(Key::Space, Modifiers::ALT)], Modifiers::ALT, true);
        assert!(state(&h.ctx).error.is_some());
        h.click("重新錄製");
        assert!(is_recording(&h.ctx));
        assert!(state(&h.ctx).error.is_none());
        let (_, changed, blocked) = h.frame(
            vec![
                key(Key::Y, Modifiers::CTRL),
                release(Key::Y),
                key(Key::Enter, Modifiers::NONE),
            ],
            Modifiers::CTRL,
            true,
        );
        assert!(changed && blocked);
        assert_eq!(h.draft, "Ctrl+Y");
        assert_eq!(h.leaked, 0);
        assert!(h.flag.load(Ordering::SeqCst));
        h.idle();
        assert!(!h.flag.load(Ordering::SeqCst));
    }
}
