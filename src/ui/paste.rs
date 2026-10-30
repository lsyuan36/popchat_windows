//! Observe native paste requests before egui-winit consumes clipboard shortcuts.
//!
//! This module never reads the clipboard or consumes a Windows message. Text
//! paste remains winit's responsibility; the app can separately handle files or
//! images when [`NativePaste::take_requested`] returns true.

#[cfg(windows)]
use eframe::egui;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

/// Shared between the event-loop hook and the app's update loop.
#[derive(Default)]
pub struct NativePaste {
    requested: AtomicBool,
    #[cfg(windows)]
    hwnd: std::sync::atomic::AtomicUsize,
    #[cfg(windows)]
    context: std::sync::Mutex<Option<egui::Context>>,
}

impl NativePaste {
    /// Bind to the exact root window during app creation, not other viewports or
    /// native dialogs on the same event-loop thread. A missing handle disables
    /// observation. This is a no-op on non-Windows platforms.
    pub fn bind(&self, creation: &eframe::CreationContext<'_>) {
        #[cfg(windows)]
        {
            use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

            self.hwnd.store(0, Ordering::Release);
            let hwnd = match creation.window_handle().map(|handle| handle.as_raw()) {
                Ok(RawWindowHandle::Win32(handle)) => handle.hwnd.get() as usize,
                _ => 0,
            };
            *self.context.lock().unwrap() = Some(creation.egui_ctx.clone());
            self.hwnd.store(hwnd, Ordering::Release);
        }
        #[cfg(not(windows))]
        let _ = creation;
    }

    /// Consume pending requests once. Multiple native messages before the next
    /// UI update coalesce into one request. Always false on non-Windows.
    pub fn take_requested(&self) -> bool {
        self.requested.swap(false, Ordering::AcqRel)
    }

    #[cfg(any(windows, test))]
    fn request(&self) {
        self.requested.store(true, Ordering::Release);
        #[cfg(windows)]
        {
            // Release our lock before invoking egui's repaint callback.
            let context = self.context.lock().unwrap().clone();
            if let Some(context) = context {
                context.request_repaint_of(egui::ViewportId::ROOT);
            }
        }
    }
}

/// Install before `eframe::run_native`, retaining any existing event-loop
/// builder customization. The observer always allows normal message dispatch.
/// This is a no-op on non-Windows platforms.
pub fn install(options: &mut eframe::NativeOptions, state: Arc<NativePaste>) {
    #[cfg(windows)]
    {
        use winit::platform::windows::EventLoopBuilderExtWindows;

        let previous = options.event_loop_builder.take();
        options.event_loop_builder = Some(Box::new(move |builder| {
            if let Some(previous) = previous {
                previous(builder);
            }
            builder.with_msg_hook(move |message| {
                if !message.is_null() {
                    // SAFETY: winit supplies a valid Win32 MSG for the duration
                    // of this callback. NativeMessage mirrors its C layout.
                    let message = unsafe { &*message.cast::<windows::NativeMessage>() };
                    windows::observe(&state, message);
                }
                false
            });
        }));
    }
    #[cfg(not(windows))]
    let _ = (options, state);
}

#[cfg(any(windows, test))]
const WM_KEYDOWN: u32 = 0x0100;
#[cfg(any(windows, test))]
const WM_PASTE: u32 = 0x0302;
#[cfg(any(windows, test))]
const VK_V: usize = 0x56;
#[cfg(any(windows, test))]
const VK_INSERT: usize = 0x2D;

/// Pure classification keeps modifier/repeat/window filtering testable without
/// installing a hook, accessing the clipboard, or generating real input.
#[cfg(any(windows, test))]
#[allow(clippy::too_many_arguments)]
fn is_paste_message(
    target_hwnd: usize,
    hwnd: usize,
    message: u32,
    wparam: usize,
    lparam: isize,
    ctrl: bool,
    shift: bool,
    alt: bool,
) -> bool {
    if target_hwnd == 0 || hwnd != target_hwnd {
        return false;
    }
    if message == WM_PASTE {
        return true;
    }
    message == WM_KEYDOWN
        && (lparam as usize & (1 << 30)) == 0
        && !alt
        && ((wparam == VK_V && ctrl) || (wparam == VK_INSERT && shift))
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::ffi::c_void;

    #[repr(C)]
    struct Point {
        x: i32,
        y: i32,
    }

    /// Win32 MSG, including its trailing private DWORD (48 bytes on x64).
    #[repr(C)]
    pub(super) struct NativeMessage {
        hwnd: *mut c_void,
        message: u32,
        wparam: usize,
        lparam: isize,
        time: u32,
        point: Point,
        private: u32,
    }

    #[link(name = "user32")]
    extern "system" {
        fn GetKeyState(virtual_key: i32) -> i16;
    }

    fn key_down(virtual_key: i32) -> bool {
        // SAFETY: GetKeyState accepts a virtual-key code and has no pointer or
        // lifetime requirements. Its high bit is the thread's key-down state.
        unsafe { GetKeyState(virtual_key) < 0 }
    }

    pub(super) fn observe(state: &NativePaste, message: &NativeMessage) {
        let target = state.hwnd.load(Ordering::Acquire);
        let hwnd = message.hwnd as usize;
        if target == 0 || hwnd != target {
            return;
        }
        // Do not query key state for unrelated messages (or WM_PASTE, whose
        // meaning is independent of whichever modifiers happen to be held).
        let (ctrl, shift, alt) = if message.message == WM_KEYDOWN
            && matches!(message.wparam, VK_V | VK_INSERT)
            && (message.lparam as usize & (1 << 30)) == 0
        {
            (key_down(0x11), key_down(0x10), key_down(0x12))
        } else {
            (false, false, false)
        };
        if is_paste_message(
            target,
            hwnd,
            message.message,
            message.wparam,
            message.lparam,
            ctrl,
            shift,
            alt,
        ) {
            state.request();
        }
    }

    #[cfg(test)]
    #[test]
    fn native_message_layout_matches_win32() {
        use std::mem::{offset_of, size_of};

        assert_eq!(
            size_of::<NativeMessage>(),
            if cfg!(target_pointer_width = "64") {
                48
            } else {
                32
            }
        );
        assert_eq!(offset_of!(NativeMessage, message), size_of::<usize>());
        assert_eq!(
            offset_of!(NativeMessage, wparam),
            if cfg!(target_pointer_width = "64") {
                16
            } else {
                8
            }
        );
        assert_eq!(
            offset_of!(NativeMessage, private),
            size_of::<NativeMessage>() - 4
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: usize = 0x1234;

    #[test]
    fn keyboard_modifier_matrix() {
        for ctrl in [false, true] {
            for shift in [false, true] {
                for alt in [false, true] {
                    for key in [VK_V, VK_INSERT, 0x41] {
                        assert_eq!(
                            is_paste_message(ROOT, ROOT, WM_KEYDOWN, key, 1, ctrl, shift, alt),
                            !alt && ((key == VK_V && ctrl) || (key == VK_INSERT && shift)),
                            "key={key:#x}, ctrl={ctrl}, shift={shift}, alt={alt}",
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn held_key_autorepeat_is_ignored_but_distinct_presses_are_not() {
        for key in [VK_V, VK_INSERT] {
            for lparam in [1, 1 | (1 << 24)] {
                assert!(is_paste_message(
                    ROOT, ROOT, WM_KEYDOWN, key, lparam, true, true, false
                ));
                assert!(!is_paste_message(
                    ROOT,
                    ROOT,
                    WM_KEYDOWN,
                    key,
                    lparam | (1 << 30),
                    true,
                    true,
                    false
                ));
            }
        }
    }

    #[test]
    fn paste_message_does_not_require_modifiers_or_key_bits() {
        for ctrl in [false, true] {
            for shift in [false, true] {
                for alt in [false, true] {
                    assert!(is_paste_message(
                        ROOT, ROOT, WM_PASTE, 0, -1, ctrl, shift, alt
                    ));
                }
            }
        }
    }

    #[test]
    fn ignores_other_windows_and_unbound_handles() {
        for message in [WM_KEYDOWN, WM_PASTE] {
            for (target, hwnd) in [(ROOT, ROOT + 1), (ROOT, 0), (0, ROOT), (0, 0)] {
                assert!(!is_paste_message(
                    target, hwnd, message, VK_V, 1, true, true, false
                ));
            }
        }
    }

    #[test]
    fn ignores_keyup_system_keys_and_unrelated_messages() {
        for message in [0, 0x0101, 0x0104, 0x0105, 0x0102, 0x0200] {
            assert!(!is_paste_message(
                ROOT, ROOT, message, VK_V, 1, true, true, false
            ));
        }
    }

    #[test]
    fn requests_coalesce_and_are_consumed_once() {
        let state = NativePaste::default();
        assert!(!state.take_requested());
        state.request();
        state.request();
        assert!(state.take_requested());
        assert!(!state.take_requested());
        state.request();
        assert!(state.take_requested());
        assert!(!state.take_requested());
    }

    #[cfg(not(windows))]
    #[test]
    fn installation_is_a_noop_off_windows() {
        let state = Arc::new(NativePaste::default());
        let mut options = eframe::NativeOptions::default();
        options.event_loop_builder = Some(Box::new(|_| {}));
        install(&mut options, Arc::clone(&state));
        assert!(options.event_loop_builder.is_some());
        assert!(!state.take_requested());
    }
}
