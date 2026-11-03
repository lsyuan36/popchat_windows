//! 系統匣（tray）：對應原版 AppDelegate 的 NSStatusItem。
//!
//! - tooltip `PopChat`
//! - 左鍵：顯示/隱藏浮窗（對應 `statusItemClicked` 的左鍵分支 → toggle）
//! - 右鍵：tray-icon 自動彈出選單 → Settings / Quit PopChat
//!
//! 事件經 std MPSC channel 轉發給 UI 執行緒（tray-icon 0.2 的 receiver
//! 是全域 crossbeam channel，這裡用背景執行緒 `try_recv` 輪詢後轉發，
//! 與 [`super::hotkey`] 的消費方式一致）。

use anyhow::{Context, Result};
use std::sync::mpsc::{self, Receiver};
use tray_icon::icon::Icon;
use tray_icon::menu::{menu_event_receiver, Menu, MenuItem};
use tray_icon::{tray_event_receiver, ClickEvent, TrayIcon, TrayIconBuilder};

/// UI 執行緒收到的匣事件。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    /// 左鍵：顯示/隱藏浮窗。
    TogglePanel,
    /// 右鍵選單 → Settings。
    OpenSettings,
    /// 右鍵選單 → Quit。
    Quit,
}

/// 持有 TrayIcon（drop 即移除圖示）與事件接收端。
///
/// 事件同時廣播給 UI 端與 visibility-driver 端（理由同 hotkey：
/// 隱藏時 `update()` 停擺，Toggle 必須由 driver 執行緒處理）。
pub struct PopchatTray {
    /// 必須留存：drop 會移除匣圖示。
    _tray: TrayIcon,
    /// MenuItem 是 id  handle，留存避免未來 muda 語義變化時被回收。
    _settings_item: MenuItem,
    _toggle_item: MenuItem,
    _quit_item: MenuItem,
    rx: Receiver<TrayEvent>,
    #[cfg(windows)]
    native_window: isize,
}

impl PopchatTray {
    /// 回傳 Self 與 driver 端接收器。
    pub fn new() -> Result<(Self, Receiver<TrayEvent>)> {
        let menu = Menu::new();
        let settings_item = MenuItem::new("Settings…", true, None);
        let toggle_item = MenuItem::new("Show / Hide PopChat    Alt+Down", true, None);
        let quit_item = MenuItem::new("Quit PopChat", true, None);
        let settings_id = settings_item.id();
        let quit_id = quit_item.id();
        let toggle_id = toggle_item.id();
        menu.append(&toggle_item);
        menu.append(&settings_item);
        menu.append(&quit_item);

        let icon = Icon::from_rgba(tray_pixels(), 32, 32).context("建立匣圖示失敗")?;
        #[cfg(windows)]
        let previous_windows = native_hint::tray_windows();

        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("PopChat")
            .with_icon(icon)
            // 左鍵留給我們自己處理（Toggle），選單只在右鍵出現，
            // 對應原版「左鍵 toggle、右/ctrl 鍵才 attach menu」。
            .with_menu_on_left_click(false)
            .build()
            .context("建立系統匣圖示失敗")?;
        #[cfg(windows)]
        let native_window = native_hint::tray_windows()
            .into_iter()
            .find(|window| !previous_windows.contains(window))
            .unwrap_or(0);

        let (tx_ui, rx_ui) = mpsc::channel::<TrayEvent>();
        let (tx_drv, rx_drv) = mpsc::channel::<TrayEvent>();
        std::thread::Builder::new()
            .name("popchat-tray".to_owned())
            .spawn(move || watch_loop(settings_id, quit_id, toggle_id, tx_ui, tx_drv))
            .context("啟動匣事件監聽執行緒失敗")?;

        Ok((
            Self {
                _tray: tray,
                _settings_item: settings_item,
                _toggle_item: toggle_item,
                _quit_item: quit_item,
                rx: rx_ui,
                #[cfg(windows)]
                native_window,
            },
            rx_drv,
        ))
    }

    /// UI 執行緒每幀 `try_recv` 的接收端。
    pub fn events(&self) -> &Receiver<TrayEvent> {
        &self.rx
    }

    /// FirstRunHint's native Windows equivalent: a notification belonging to
    /// this exact tray icon, not a floating banner guessed at screen corners.
    pub fn show_first_run_hint(&self, hotkey: Option<&str>) -> Result<()> {
        #[cfg(windows)]
        native_hint::notify(
            self.native_window,
            self._tray.id(),
            "PopChat 已準備好",
            &first_run_message(hotkey),
        )?;
        Ok(())
    }

    pub fn dismiss_first_run_hint(&self) {
        #[cfg(windows)]
        let _ = native_hint::notify(self.native_window, self._tray.id(), "", "");
    }
}

fn first_run_message(hotkey: Option<&str>) -> String {
    match hotkey {
        Some(shortcut) => format!(
            "按 {shortcut} 隨時顯示或隱藏 PopChat。它會留在右下角系統匣；按一下圖示也能開啟。"
        ),
        None => {
            "PopChat 會留在右下角系統匣，按一下圖示即可開啟。全域快捷鍵尚未註冊成功。".to_owned()
        }
    }
}

#[cfg(windows)]
mod native_hint {
    use super::Result;

    #[link(name = "user32")]
    extern "system" {
        fn EnumThreadWindows(
            thread: u32,
            callback: unsafe extern "system" fn(isize, isize) -> i32,
            parameter: isize,
        ) -> i32;
        fn GetClassNameW(window: isize, class: *mut u16, capacity: i32) -> i32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentThreadId() -> u32;
    }

    pub fn tray_windows() -> Vec<isize> {
        unsafe extern "system" fn collect(window: isize, parameter: isize) -> i32 {
            let mut class = [0u16; 64];
            let length = GetClassNameW(window, class.as_mut_ptr(), class.len() as i32);
            if length > 0 && String::from_utf16_lossy(&class[..length as usize]) == "tray_icon_app"
            {
                (*(parameter as *mut Vec<isize>)).push(window);
            }
            1
        }
        let mut windows = Vec::new();
        // Enumeration is synchronous and limited to this UI thread. The
        // before/after difference identifies our new tray helper precisely.
        unsafe {
            EnumThreadWindows(
                GetCurrentThreadId(),
                collect,
                &mut windows as *mut _ as isize,
            );
        }
        windows
    }

    #[repr(C)]
    struct NotifyIconData {
        size: u32,
        window: isize,
        id: u32,
        flags: u32,
        callback: u32,
        icon: isize,
        tip: [u16; 128],
        state: u32,
        state_mask: u32,
        info: [u16; 256],
        timeout: u32,
        title: [u16; 64],
        info_flags: u32,
        guid: [u8; 16],
        balloon_icon: isize,
    }
    #[link(name = "shell32")]
    extern "system" {
        fn Shell_NotifyIconW(message: u32, data: *const NotifyIconData) -> i32;
    }

    fn wide<const N: usize>(text: &str) -> [u16; N] {
        let mut result = [0; N];
        let mut offset = 0;
        for character in text.chars() {
            if offset + character.len_utf16() >= N {
                break;
            }
            let mut encoded = [0; 2];
            let units = character.encode_utf16(&mut encoded);
            result[offset..offset + units.len()].copy_from_slice(units);
            offset += units.len();
        }
        result
    }

    pub fn notify(window: isize, id: u32, title: &str, message: &str) -> Result<()> {
        anyhow::ensure!(window != 0, "找不到 PopChat 自己的系統匣視窗");
        let data = NotifyIconData {
            size: std::mem::size_of::<NotifyIconData>() as u32,
            window,
            id,
            flags: 0x10 | 0x40,
            callback: 0,
            icon: 0,
            tip: [0; 128],
            state: 0,
            state_mask: 0,
            info: wide(message),
            timeout: 10_000,
            title: wide(title),
            info_flags: 1 | 0x80,
            guid: [0; 16],
            balloon_icon: 0,
        };
        let result = unsafe { Shell_NotifyIconW(1, &data) }; // NIM_MODIFY, NIF_INFO
        anyhow::ensure!(result != 0, "Windows 無法顯示 PopChat 的系統匣提示");
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn native_layout_and_unicode_buffers_match_win32() {
            assert_eq!(
                std::mem::size_of::<super::NotifyIconData>(),
                if cfg!(target_pointer_width = "64") {
                    976
                } else {
                    956
                }
            );
            let buffer = super::wide::<4>("😀😀");
            assert_eq!(String::from_utf16(&buffer[..2]).unwrap(), "😀");
            assert_eq!(buffer[2], 0);
        }
    }
}

/// 背景執行緒：輪詢 tray-icon 0.2 的全域 receiver，轉發為 [`TrayEvent`]。
/// 右鍵選單由 tray-icon 自動彈出，這裡只處理選單項事件。
/// 廣播給 UI 與 driver 兩端；兩端都斷開才結束。
fn watch_loop(
    settings_id: u32,
    quit_id: u32,
    toggle_id: u32,
    tx_ui: std::sync::mpsc::Sender<TrayEvent>,
    tx_drv: std::sync::mpsc::Sender<TrayEvent>,
) {
    let tray_rx = tray_event_receiver();
    let menu_rx = menu_event_receiver();
    loop {
        let mut ui_gone = false;
        let mut drv_gone = false;
        while let Ok(event) = tray_rx.try_recv() {
            // ClickEvent::Right 由 tray-icon 自動開選單，無需處理；
            // Double 視為 Toggle（與左鍵同義）。
            let out = match event.event {
                ClickEvent::Left | ClickEvent::Double => Some(TrayEvent::TogglePanel),
                ClickEvent::Right => None,
            };
            if let Some(e) = out {
                ui_gone |= tx_ui.send(e).is_err();
                drv_gone |= tx_drv.send(e).is_err();
            }
        }
        while let Ok(event) = menu_rx.try_recv() {
            let out = if event.id == settings_id {
                Some(TrayEvent::OpenSettings)
            } else if event.id == quit_id {
                Some(TrayEvent::Quit)
            } else if event.id == toggle_id {
                Some(TrayEvent::TogglePanel)
            } else {
                None
            };
            if let Some(e) = out {
                ui_gone |= tx_ui.send(e).is_err();
                drv_gone |= tx_drv.send(e).is_err();
            }
        }
        if ui_gone && drv_gone {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// Two chat bubbles, rendered with 4× coverage sampling for small tray sizes.
/// No external asset lookup or single-pixel placeholder at runtime.
fn tray_pixels() -> Vec<u8> {
    crate::app_icon::rgba(32)
}

#[cfg(test)]
mod tests {
    #[test]
    fn first_run_hint_uses_actual_registered_shortcut_or_truthful_fallback() {
        assert!(super::first_run_message(Some("Alt+Down")).contains("Alt+Down"));
        let unavailable = super::first_run_message(None);
        assert!(unavailable.contains("尚未註冊"));
        assert!(!unavailable.contains("Alt+Space"));
        assert!(!unavailable.contains("Alt+Down"));
    }
    #[test]
    fn tray_has_visible_bubbles_and_transparent_background() {
        let rgba = super::tray_pixels();
        assert_eq!(rgba.len(), 32 * 32 * 4);
        assert_eq!(rgba[3], 0);
        assert!(rgba.chunks_exact(4).filter(|px| px[3] == 255).count() > 250);
        assert!(rgba.chunks_exact(4).any(|px| px[3] > 0 && px[3] < 255));
    }
}
