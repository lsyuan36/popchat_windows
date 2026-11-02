//! 可設定的全域熱鍵（預設 Alt+Down）；衝突時保留原本註冊並提供重試。
//!
//! Windows 的 `RegisterHotKey` 使用 MOD_NOREPEAT，按住時不會重複觸發；
//! 因此不需要會吞掉快速連按的時間 debounce。Alt+Space 保留 Windows 用途；
//! 不在設定的快捷鍵被佔用時偷偷改綁。
//! 對應原版 `KeyboardShortcuts.onKeyUp(for: .togglePopChat)`。
//! `global-hotkey` 在背景執行緒收 OS 事件，經 std MPSC channel
//! 轉發 [`HotkeyEvent::Toggle`] 給 UI 執行緒（每幀 `try_recv`）。
//! 註冊失敗由呼叫方顯示在 UI 上並提供重試。

use crate::logf;
use anyhow::{Context, Result};
use global_hotkey::{
    hotkey::{Code, HotKey, Modifiers},
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
};
use std::sync::{
    atomic::{AtomicBool, AtomicU32, Ordering},
    mpsc::{self, Receiver, Sender},
    Arc,
};

/// 預設快捷鍵（不是唯一支援的快捷鍵）。
pub const HOTKEY_LABEL: &str = "Alt+Down";

#[derive(Clone, Debug)]
struct Binding {
    hotkey: HotKey,
    label: String,
    local_key: egui::Key,
}

/// 驗證並正規化快捷鍵，不呼叫 OS。
///
/// 不分大小寫、忽略各 token 兩側空白；修飾鍵必須在主鍵之前，且不可重複。
/// 輸出順序固定為 Ctrl+Alt+Shift+Win+主鍵。接受 Control、Super、Windows、
/// ArrowDown/Up/Left/Right、Esc、Return、Del、Ins、PgUp/PgDn 等別名。
/// 主鍵支援 A-Z、0-9、方向鍵、F1-F24、導覽/編輯鍵及常用標點（以名稱輸入）。
/// 除 F1-F24 外必須有 Ctrl/Alt/Win，避免攔截一般打字；Alt+Space 不允許。
pub fn normalize_binding(binding: &str) -> Result<String> {
    Ok(parse_binding(binding)?.label)
}

/// Convert a recorded egui key using the parser's exact allowlist and validation.
/// `command` is deliberately ignored: on Windows it aliases Ctrl, not Win.
/// The caller can augment `mac_cmd` with the native Windows-key state.
pub fn binding_from_key(key: egui::Key, mods: egui::Modifiers) -> Result<String> {
    let name = main_keys()
        .iter()
        .find(|(_, _, local)| *local == key)
        .map(|(name, _, _)| *name)
        .context("此按鍵不支援作為全域快捷鍵")?;
    let mut parts = Vec::new();
    for (down, label) in [
        (mods.ctrl, "Ctrl"),
        (mods.alt, "Alt"),
        (mods.shift, "Shift"),
        (mods.mac_cmd, "Win"),
    ] {
        if down {
            parts.push(label);
        }
    }
    parts.push(name);
    normalize_binding(&parts.join("+"))
}

fn parse_binding(binding: &str) -> Result<Binding> {
    let mut tokens = binding.split('+').map(str::trim).peekable();
    let mut mods = Modifiers::empty();
    let main = loop {
        let token = tokens.next().context("快捷鍵不可為空")?;
        anyhow::ensure!(!token.is_empty(), "快捷鍵不可包含空白鍵名");
        if tokens.peek().is_none() {
            break token.to_ascii_uppercase();
        }
        let modifier = match token.to_ascii_uppercase().as_str() {
            "CTRL" | "CONTROL" => Modifiers::CONTROL,
            "ALT" => Modifiers::ALT,
            "SHIFT" => Modifiers::SHIFT,
            "WIN" | "WINDOWS" | "SUPER" => Modifiers::SUPER,
            _ => anyhow::bail!("未知修飾鍵 {token}；請將 Ctrl/Alt/Shift/Win 放在主鍵前"),
        };
        anyhow::ensure!(!mods.contains(modifier), "修飾鍵不可重複：{token}");
        mods |= modifier;
    };
    let key_name = match main.as_str() {
        "ARROWDOWN" => "Down",
        "ARROWUP" => "Up",
        "ARROWLEFT" => "Left",
        "ARROWRIGHT" => "Right",
        "ESC" => "Escape",
        "RETURN" => "Enter",
        "DEL" => "Delete",
        "INS" => "Insert",
        "PGUP" => "PageUp",
        "PGDN" | "PGDOWN" => "PageDown",
        _ => main.as_str(),
    };
    let (name, code, local_key) = main_keys()
        .iter()
        .find(|(name, _, _)| name.eq_ignore_ascii_case(key_name))
        .copied()
        .with_context(|| format!("未知主鍵：{main}"))?;
    let function_key = name.starts_with('F') && name[1..].parse::<u8>().is_ok();
    anyhow::ensure!(
        function_key || mods.intersects(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SUPER),
        "除 F1-F24 外，快捷鍵需包含 Ctrl、Alt 或 Win，以免攔截一般輸入"
    );
    anyhow::ensure!(
        !(code == Code::Space && mods == Modifiers::ALT),
        "Alt+Space 為 Windows 保留快捷鍵"
    );
    let mut parts = Vec::new();
    for (flag, name) in [
        (Modifiers::CONTROL, "Ctrl"),
        (Modifiers::ALT, "Alt"),
        (Modifiers::SHIFT, "Shift"),
        (Modifiers::SUPER, "Win"),
    ] {
        if mods.contains(flag) {
            parts.push(name);
        }
    }
    parts.push(name);
    Ok(Binding {
        hotkey: HotKey::new(Some(mods), code),
        label: parts.join("+"),
        local_key,
    })
}

// A shared allowlist keeps parser support and local key consumption in sync.
fn main_keys() -> &'static [(&'static str, Code, egui::Key)] {
    macro_rules! keys {
        ($($name:literal => $code:ident / $local:ident),* $(,)?) => {
            &[$(($name, Code::$code, egui::Key::$local)),*]
        };
    }
    keys![
        "A" => KeyA / A, "B" => KeyB / B, "C" => KeyC / C,
        "D" => KeyD / D, "E" => KeyE / E, "F" => KeyF / F,
        "G" => KeyG / G, "H" => KeyH / H, "I" => KeyI / I,
        "J" => KeyJ / J, "K" => KeyK / K, "L" => KeyL / L,
        "M" => KeyM / M, "N" => KeyN / N, "O" => KeyO / O,
        "P" => KeyP / P, "Q" => KeyQ / Q, "R" => KeyR / R,
        "S" => KeyS / S, "T" => KeyT / T, "U" => KeyU / U,
        "V" => KeyV / V, "W" => KeyW / W, "X" => KeyX / X,
        "Y" => KeyY / Y, "Z" => KeyZ / Z,
        "0" => Digit0 / Num0, "1" => Digit1 / Num1, "2" => Digit2 / Num2,
        "3" => Digit3 / Num3, "4" => Digit4 / Num4, "5" => Digit5 / Num5,
        "6" => Digit6 / Num6, "7" => Digit7 / Num7, "8" => Digit8 / Num8,
        "9" => Digit9 / Num9,
        "Down" => ArrowDown / ArrowDown, "Up" => ArrowUp / ArrowUp,
        "Left" => ArrowLeft / ArrowLeft, "Right" => ArrowRight / ArrowRight,
        "F1" => F1 / F1, "F2" => F2 / F2, "F3" => F3 / F3,
        "F4" => F4 / F4, "F5" => F5 / F5, "F6" => F6 / F6,
        "F7" => F7 / F7, "F8" => F8 / F8, "F9" => F9 / F9,
        "F10" => F10 / F10, "F11" => F11 / F11, "F12" => F12 / F12,
        "F13" => F13 / F13, "F14" => F14 / F14, "F15" => F15 / F15,
        "F16" => F16 / F16, "F17" => F17 / F17, "F18" => F18 / F18,
        "F19" => F19 / F19, "F20" => F20 / F20, "F21" => F21 / F21,
        "F22" => F22 / F22, "F23" => F23 / F23, "F24" => F24 / F24,
        "Space" => Space / Space, "Enter" => Enter / Enter, "Tab" => Tab / Tab,
        "Escape" => Escape / Escape, "Backspace" => Backspace / Backspace,
        "Delete" => Delete / Delete, "Insert" => Insert / Insert,
        "Home" => Home / Home, "End" => End / End,
        "PageUp" => PageUp / PageUp, "PageDown" => PageDown / PageDown,
        "Comma" => Comma / Comma, "Period" => Period / Period,
        "Slash" => Slash / Slash, "Backslash" => Backslash / Backslash,
        "Semicolon" => Semicolon / Semicolon, "Quote" => Quote / Quote,
        "Minus" => Minus / Minus, "Equal" => Equal / Equals,
        "BracketLeft" => BracketLeft / OpenBracket,
        "BracketRight" => BracketRight / CloseBracket,
        "Backquote" => Backquote / Backtick,
    ]
}

/// Small registry boundary: transaction tests never call RegisterHotKey.
trait Registry {
    fn register(&self, hotkey: HotKey) -> Result<()>;
    fn unregister(&self, hotkey: HotKey) -> Result<()>;
}

impl Registry for GlobalHotKeyManager {
    fn register(&self, hotkey: HotKey) -> Result<()> {
        Ok(GlobalHotKeyManager::register(self, hotkey)?)
    }

    fn unregister(&self, hotkey: HotKey) -> Result<()> {
        Ok(GlobalHotKeyManager::unregister(self, hotkey)?)
    }
}

struct Registration {
    configured: Binding,
    active: Option<Binding>,
    last_error: Option<String>,
}

impl Registration {
    fn new(configured: Binding) -> Self {
        Self {
            configured,
            active: None,
            last_error: None,
        }
    }

    fn apply(
        &mut self,
        registry: &impl Registry,
        current: &AtomicU32,
        next: Binding,
    ) -> Result<&str> {
        if self
            .active
            .as_ref()
            .is_some_and(|old| old.hotkey == next.hotkey)
        {
            self.last_error = None;
            return Ok(&self.active.as_ref().unwrap().label);
        }
        let result = (|| {
            // Leave the old registration and id intact until BOTH OS calls succeed.
            registry
                .register(next.hotkey)
                .with_context(|| format!("{} 註冊失敗", next.label))?;
            if let Some(old) = &self.active {
                if let Err(error) = registry.unregister(old.hotkey) {
                    let rollback = registry.unregister(next.hotkey);
                    anyhow::bail!(
                        "無法取消舊快捷鍵 {}：{error:#}；{}",
                        old.label,
                        match rollback {
                            Ok(()) => "已取消新註冊，保留原快捷鍵".to_owned(),
                            Err(error) =>
                                format!("取消新註冊也失敗：{error:#}；仍只接受原快捷鍵事件"),
                        }
                    );
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            let msg = format!("{error:#}。請釋放衝突快捷鍵後重試；也可從托盤開啟面板。");
            self.last_error = Some(msg.clone());
            logf!("[hotkey] {msg}");
            anyhow::bail!("{msg}");
        }
        self.configured = next.clone();
        self.active = Some(next);
        self.last_error = None;
        current.store(self.configured.hotkey.id(), Ordering::SeqCst);
        logf!("[hotkey] 已註冊全域熱鍵 {}", self.configured.label);
        Ok(&self.configured.label)
    }
}

/// UI 執行緒收到的熱鍵事件。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyEvent {
    /// 切換浮窗顯示/隱藏（對應 PanelController.toggle）。
    Toggle,
}

/// 持有 `GlobalHotKeyManager`（drop 即反註冊）與事件接收端。
///
/// 註冊與監聽分離：`new()` 永遠建好監聽執行緒（不過早失敗），
/// `try_register()` 可重複呼叫（佔用者釋放後重試）。watch 執行緒經共享
/// `current_id` 認目前生效的熱鍵，註冊成功前收到的事件一律忽略。
/// 事件同時廣播給 UI 端與 visibility-driver 端各一個 channel：
/// driver 可在 UI 尚未重繪時接收快捷鍵（見 main.rs）。
pub struct HotkeyManager {
    /// 必須留存：drop 會取消註冊。
    _manager: GlobalHotKeyManager,
    current_id: Arc<AtomicU32>,
    suppressed: Arc<AtomicBool>,
    registration: Registration,
    rx: Receiver<HotkeyEvent>,
    stop: Arc<AtomicBool>,
    watcher: Option<std::thread::JoinHandle<()>>,
}

impl HotkeyManager {
    /// 建管理器 + 監聽執行緒，並做第一次註冊嘗試。
    /// 回傳 Self 與 driver 端接收器。監聽執行緒永遠存在，
    /// 即使第一次註冊失敗，重試成功後事件照樣送達兩端。
    pub fn new() -> Result<(Self, Receiver<HotkeyEvent>)> {
        Self::new_with_binding(HOTKEY_LABEL)
    }

    /// 語法錯誤回傳 Err；OS 首次註冊失敗仍回傳管理器，label 為 None，可重試。
    pub fn new_with_binding(binding: &str) -> Result<(Self, Receiver<HotkeyEvent>)> {
        let configured = parse_binding(binding)?;
        let manager = GlobalHotKeyManager::new().context("建立全域熱鍵管理器失敗")?;
        let current_id = Arc::new(AtomicU32::new(0));
        let (tx_ui, rx_ui) = mpsc::channel::<HotkeyEvent>();
        let (tx_drv, rx_drv) = mpsc::channel::<HotkeyEvent>();
        let current = current_id.clone();
        let suppressed = Arc::new(AtomicBool::new(false));
        let watcher_suppressed = suppressed.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let watcher_stop = stop.clone();
        let watcher = std::thread::Builder::new()
            .name("popchat-hotkey".to_owned())
            .spawn(move || watch_loop(current, watcher_stop, watcher_suppressed, tx_ui, tx_drv))
            .context("啟動熱鍵監聽執行緒失敗")?;
        let mut this = Self {
            _manager: manager,
            current_id,
            suppressed,
            registration: Registration::new(configured),
            rx: rx_ui,
            stop,
            watcher: Some(watcher),
        };
        // 第一次嘗試：失敗也不回 Err，由 label()/register_error() 呈現，
        // UI 顯示紅字 + 重試按鈕，App 仍可從托盤開啟。
        let _ = this.try_register();
        Ok((this, rx_drv))
    }

    /// 重試設定的快捷鍵；已註冊時不重新註冊，不引入收不到按鍵的空窗。
    pub fn try_register(&mut self) -> Result<&str> {
        self.registration.apply(
            &self._manager,
            &self.current_id,
            self.registration.configured.clone(),
        )
    }

    /// 先註冊新鍵，再取消舊鍵；失敗時保留舊鍵、標籤、current_id 與兩端 channel。
    pub fn set_binding(&mut self, binding: &str) -> Result<&str> {
        let next = parse_binding(binding)?;
        self.registration
            .apply(&self._manager, &self.current_id, next)
    }

    /// 實際生效的熱鍵名；尚無成功註冊時為 None，換鍵失敗則保留舊名。
    pub fn label(&self) -> Option<&str> {
        self.registration
            .active
            .as_ref()
            .map(|binding| binding.label.as_str())
    }

    /// 最近一次註冊失敗的原因（換鍵失敗時仍可能有生效的舊鍵）。
    pub fn register_error(&self) -> Option<&str> {
        self.registration.last_error.as_deref()
    }

    /// Share with the recorder AND visibility-driver. The recorder sets this
    /// immediately when armed, before the next OS hotkey can be forwarded.
    /// The driver must also check it when consuming already queued toggles.
    /// Suppression never unregisters a binding or changes registration state.
    pub fn suppression_flag(&self) -> Arc<AtomicBool> {
        self.suppressed.clone()
    }

    /// 在 UI 本地快捷鍵處理之前呼叫，避免同一個 press 又觸發面板內動作。
    /// 只移除目前生效組合的主鍵 press；不修改 release、modifier 或其他輸入。
    pub fn consume_local_binding(&self, ctx: &egui::Context) {
        if let Some(binding) = &self.registration.active {
            let win_down = windows_key_down();
            ctx.input_mut(|input| consume_binding_events(&mut input.events, binding, win_down));
        }
    }

    /// UI 執行緒的接收端（顯示時每幀排空；Toggle 由 driver 處理，此處只排空丟棄）。
    pub fn events(&self) -> &Receiver<HotkeyEvent> {
        &self.rx
    }
}

fn consume_binding_events(events: &mut Vec<egui::Event>, binding: &Binding, win_down: bool) {
    events.retain(|event| {
        !matches!(event,
            egui::Event::Key { key, pressed: true, modifiers, .. }
                if *key == binding.local_key
                    && modifiers.ctrl == binding.hotkey.mods.contains(Modifiers::CONTROL)
                    && modifiers.alt == binding.hotkey.mods.contains(Modifiers::ALT)
                    && modifiers.shift == binding.hotkey.mods.contains(Modifiers::SHIFT)
                    && win_down == binding.hotkey.mods.contains(Modifiers::SUPER)
        )
    });
}

// egui-winit 0.32 discards Super on Windows (mac_cmd is always false), so it
// cannot distinguish Ctrl+P from Win+Ctrl+P. Read the UI thread's key state;
// never consume a Win binding using a guessed/aliased egui command modifier.
#[cfg(windows)]
pub(super) fn windows_key_down() -> bool {
    #[link(name = "user32")]
    extern "system" {
        fn GetKeyState(virtual_key: i32) -> i16;
    }
    // SAFETY: GetKeyState has no pointer arguments; VK_LWIN/VK_RWIN are valid.
    unsafe { GetKeyState(0x5B) < 0 || GetKeyState(0x5C) < 0 }
}

#[cfg(not(windows))]
pub(super) fn windows_key_down() -> bool {
    false
}

impl Drop for HotkeyManager {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(watcher) = self.watcher.take() {
            let _ = watcher.join();
        }
    }
}

/// 背景執行緒：阻塞收 global-hotkey 事件，只轉發按下（Pressed）事件。
/// 經共享 `current` 認目前生效鍵，廣播給 UI 與 driver 兩端；
/// 管理器釋放或兩端皆斷開時結束。
fn watch_loop(
    current: Arc<AtomicU32>,
    stop: Arc<AtomicBool>,
    suppressed: Arc<AtomicBool>,
    tx_ui: Sender<HotkeyEvent>,
    tx_drv: Sender<HotkeyEvent>,
) {
    let receiver = GlobalHotKeyEvent::receiver();
    while !stop.load(Ordering::SeqCst) {
        let Ok(event) = receiver.recv_timeout(std::time::Duration::from_millis(100)) else {
            continue;
        };
        let id = current.load(Ordering::SeqCst);
        if should_forward(id, event.id(), event.state(), &suppressed) {
            let ui_gone = tx_ui.send(HotkeyEvent::Toggle).is_err();
            let drv_gone = tx_drv.send(HotkeyEvent::Toggle).is_err();
            if ui_gone && drv_gone {
                break;
            }
        }
    }
}

fn should_forward(
    current_id: u32,
    event_id: u32,
    state: HotKeyState,
    suppressed: &AtomicBool,
) -> bool {
    !suppressed.load(Ordering::SeqCst) && is_active_press(current_id, event_id, state)
}

fn is_active_press(current_id: u32, event_id: u32, state: HotKeyState) -> bool {
    current_id != 0 && event_id == current_id && state == HotKeyState::Pressed
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct MockRegistry<'a> {
        current: &'a AtomicU32,
        calls: RefCell<Vec<(bool, u32, u32)>>,
        fail_register: bool,
        fail_unregister: Vec<u32>,
    }

    impl<'a> MockRegistry<'a> {
        fn new(current: &'a AtomicU32) -> Self {
            Self {
                current,
                calls: RefCell::new(Vec::new()),
                fail_register: false,
                fail_unregister: Vec::new(),
            }
        }
    }

    impl Registry for MockRegistry<'_> {
        fn register(&self, hotkey: HotKey) -> Result<()> {
            self.calls
                .borrow_mut()
                .push((true, hotkey.id(), self.current.load(Ordering::SeqCst)));
            anyhow::ensure!(!self.fail_register, "mock registration conflict");
            Ok(())
        }

        fn unregister(&self, hotkey: HotKey) -> Result<()> {
            self.calls
                .borrow_mut()
                .push((false, hotkey.id(), self.current.load(Ordering::SeqCst)));
            anyhow::ensure!(
                !self.fail_unregister.contains(&hotkey.id()),
                "mock unregister failure"
            );
            Ok(())
        }
    }

    #[test]
    fn public_api_signatures_remain_compatible_without_registering_any_keys() {
        let _: fn() -> Result<(HotkeyManager, Receiver<HotkeyEvent>)> = HotkeyManager::new;
        let _: fn(&str) -> Result<(HotkeyManager, Receiver<HotkeyEvent>)> =
            HotkeyManager::new_with_binding;
        let _: for<'a, 'b> fn(&'a mut HotkeyManager, &'b str) -> Result<&'a str> =
            HotkeyManager::set_binding;
        let _: fn(&mut HotkeyManager) -> Result<&str> = HotkeyManager::try_register;
        let _: fn(&HotkeyManager, &egui::Context) = HotkeyManager::consume_local_binding;
        assert_eq!(normalize_binding(HOTKEY_LABEL).unwrap(), "Alt+Down");
    }

    #[test]
    fn normalizes_modifier_order_case_whitespace_and_aliases() {
        for (input, expected) in [
            (" alt + arrowdown ", "Alt+Down"),
            ("shift + control + p", "Ctrl+Shift+P"),
            ("Windows+ALT+ctrl+shift+9", "Ctrl+Alt+Shift+Win+9"),
            ("super+esc", "Win+Escape"),
            ("ctrl+return", "Ctrl+Enter"),
            ("alt+pgdn", "Alt+PageDown"),
            ("f24", "F24"),
            ("ctrl+comma", "Ctrl+Comma"),
        ] {
            let normalized = normalize_binding(input).unwrap();
            assert_eq!(normalized, expected);
            assert_eq!(normalize_binding(&normalized).unwrap(), normalized);
        }
    }

    #[test]
    fn rejects_reserved_unsafe_unknown_and_malformed_shortcuts() {
        for input in [
            "",
            " ",
            "Alt+Space",
            "aLt + sPaCe",
            "A",
            "9",
            "Space",
            "Down",
            "Shift+A",
            "Shift+9",
            "Shift+Enter",
            "Ctrl",
            "Alt+",
            "+P",
            "Ctrl++P",
            "Ctrl+Ctrl+P",
            "Win+Super+P",
            "Ctrl+P+Shift",
            "Ctrl+A+B",
            "Ctrl+Unknown",
            "Ctrl+F25",
            "Ctrl+F0",
            "Hyper+P",
        ] {
            assert!(normalize_binding(input).is_err(), "should reject {input:?}");
        }
    }

    #[test]
    fn every_supported_main_key_normalizes_and_maps_to_egui() {
        for (name, code, local) in main_keys() {
            let binding = parse_binding(&format!("Ctrl+{name}")).unwrap();
            assert_eq!(binding.hotkey.key, *code);
            assert_eq!(binding.local_key, *local);
            assert_eq!(binding.label, format!("Ctrl+{name}"));
        }
        for n in 1..=24 {
            assert_eq!(
                normalize_binding(&format!("f{n}")).unwrap(),
                format!("F{n}")
            );
        }
    }

    #[test]
    fn recorder_conversion_uses_same_allowlist_and_canonical_modifiers() {
        for (name, _, key) in main_keys() {
            assert_eq!(
                binding_from_key(*key, egui::Modifiers::CTRL).unwrap(),
                format!("Ctrl+{name}")
            );
        }
        let all = egui::Modifiers {
            ctrl: true,
            alt: true,
            shift: true,
            mac_cmd: true,
            command: true,
        };
        assert_eq!(
            binding_from_key(egui::Key::ArrowDown, all).unwrap(),
            "Ctrl+Alt+Shift+Win+Down"
        );
        assert!(binding_from_key(egui::Key::A, egui::Modifiers::NONE).is_err());
        assert!(binding_from_key(egui::Key::Space, egui::Modifiers::ALT).is_err());
        assert!(binding_from_key(egui::Key::F25, egui::Modifiers::CTRL).is_err());
        assert!(binding_from_key(
            egui::Key::P,
            egui::Modifiers {
                command: true,
                ..Default::default()
            }
        )
        .is_err());
    }

    #[test]
    fn recording_suppresses_active_driver_events_without_touching_registration() {
        let flag = Arc::new(AtomicBool::new(false));
        let watcher_flag = flag.clone();
        assert!(should_forward(42, 42, HotKeyState::Pressed, &watcher_flag));
        flag.store(true, Ordering::SeqCst);
        for _ in 0..5 {
            assert!(!should_forward(42, 42, HotKeyState::Pressed, &watcher_flag));
        }
        flag.store(false, Ordering::SeqCst);
        assert!(should_forward(42, 42, HotKeyState::Pressed, &watcher_flag));
        assert!(!should_forward(42, 41, HotKeyState::Pressed, &watcher_flag));
    }

    #[test]
    fn failed_initial_registration_retains_configured_binding_for_retry() {
        let current = AtomicU32::new(0);
        let mut registry = MockRegistry::new(&current);
        let mut state = Registration::new(parse_binding("Ctrl+Shift+P").unwrap());
        registry.fail_register = true;
        assert!(state
            .apply(&registry, &current, state.configured.clone())
            .is_err());
        assert!(state.active.is_none());
        assert!(state
            .last_error
            .as_deref()
            .unwrap()
            .contains("Ctrl+Shift+P"));
        assert_eq!(current.load(Ordering::SeqCst), 0);
        registry.fail_register = false;
        assert_eq!(
            state
                .apply(&registry, &current, state.configured.clone())
                .unwrap(),
            "Ctrl+Shift+P"
        );
        assert!(state.last_error.is_none());
        let expected_id = parse_binding("Ctrl+Shift+P").unwrap().hotkey.id();
        assert_eq!(current.load(Ordering::SeqCst), expected_id);
        assert_eq!(
            *registry.calls.borrow(),
            vec![(true, expected_id, 0), (true, expected_id, 0)]
        );
    }

    #[test]
    fn rebind_registers_first_and_publishes_id_only_after_old_unregistered() {
        let current = AtomicU32::new(0);
        let registry = MockRegistry::new(&current);
        let old = parse_binding(HOTKEY_LABEL).unwrap();
        let next = parse_binding("Ctrl+P").unwrap();
        let (old_id, new_id) = (old.hotkey.id(), next.hotkey.id());
        let mut state = Registration::new(old.clone());
        state.apply(&registry, &current, old).unwrap();
        registry.calls.borrow_mut().clear();
        assert_eq!(state.apply(&registry, &current, next).unwrap(), "Ctrl+P");
        assert_eq!(
            *registry.calls.borrow(),
            vec![(true, new_id, old_id), (false, old_id, old_id)]
        );
        assert_eq!(current.load(Ordering::SeqCst), new_id);
        assert!(!is_active_press(new_id, old_id, HotKeyState::Pressed));
        assert!(is_active_press(new_id, new_id, HotKeyState::Pressed));
        assert_eq!(state.active.as_ref().unwrap().label, "Ctrl+P");
        assert_eq!(state.configured.label, "Ctrl+P");
        registry.calls.borrow_mut().clear();
        state
            .apply(&registry, &current, parse_binding("control+p").unwrap())
            .unwrap();
        assert!(
            registry.calls.borrow().is_empty(),
            "same shortcut must not re-register"
        );
    }

    #[test]
    fn new_registration_conflict_preserves_old_binding_and_events() {
        let current = AtomicU32::new(0);
        let mut registry = MockRegistry::new(&current);
        let old = parse_binding(HOTKEY_LABEL).unwrap();
        let old_id = old.hotkey.id();
        let next = parse_binding("Ctrl+N").unwrap();
        let new_id = next.hotkey.id();
        let mut state = Registration::new(old.clone());
        state.apply(&registry, &current, old).unwrap();
        registry.calls.borrow_mut().clear();
        registry.fail_register = true;
        assert!(state.apply(&registry, &current, next).is_err());
        assert_eq!(*registry.calls.borrow(), vec![(true, new_id, old_id)]);
        assert_eq!(state.active.as_ref().unwrap().label, HOTKEY_LABEL);
        assert_eq!(state.configured.label, HOTKEY_LABEL);
        assert_eq!(current.load(Ordering::SeqCst), old_id);
        assert!(is_active_press(old_id, old_id, HotKeyState::Pressed));
        assert!(!is_active_press(old_id, new_id, HotKeyState::Pressed));
    }

    #[test]
    fn failed_initial_registration_can_be_replaced_without_unregistering_anything() {
        let current = AtomicU32::new(0);
        let mut registry = MockRegistry::new(&current);
        let original = parse_binding("Ctrl+N").unwrap();
        let original_id = original.hotkey.id();
        let replacement = parse_binding("Ctrl+F").unwrap();
        let replacement_id = replacement.hotkey.id();
        let mut state = Registration::new(original.clone());
        registry.fail_register = true;
        assert!(state.apply(&registry, &current, original).is_err());
        registry.fail_register = false;
        assert_eq!(
            state.apply(&registry, &current, replacement).unwrap(),
            "Ctrl+F"
        );
        assert_eq!(
            *registry.calls.borrow(),
            vec![(true, original_id, 0), (true, replacement_id, 0),]
        );
        assert_eq!(state.configured.label, "Ctrl+F");
        assert_eq!(current.load(Ordering::SeqCst), replacement_id);
    }

    #[test]
    fn old_unregister_failure_rolls_back_new_and_never_claims_success() {
        for rollback_fails in [false, true] {
            let current = AtomicU32::new(0);
            let mut registry = MockRegistry::new(&current);
            let old = parse_binding(HOTKEY_LABEL).unwrap();
            let next = parse_binding("Ctrl+F").unwrap();
            let (old_id, new_id) = (old.hotkey.id(), next.hotkey.id());
            let mut state = Registration::new(old.clone());
            state.apply(&registry, &current, old).unwrap();
            registry.calls.borrow_mut().clear();
            registry.fail_unregister.push(old_id);
            if rollback_fails {
                registry.fail_unregister.push(new_id);
            }
            let error = state
                .apply(&registry, &current, next)
                .unwrap_err()
                .to_string();
            assert_eq!(
                *registry.calls.borrow(),
                vec![
                    (true, new_id, old_id),
                    (false, old_id, old_id),
                    (false, new_id, old_id),
                ]
            );
            assert_eq!(current.load(Ordering::SeqCst), old_id);
            assert_eq!(state.active.as_ref().unwrap().label, HOTKEY_LABEL);
            assert_eq!(state.configured.label, HOTKEY_LABEL);
            assert!(state.last_error.is_some());
            assert_eq!(error.contains("取消新註冊也失敗"), rollback_fails);
            assert!(!is_active_press(old_id, new_id, HotKeyState::Pressed));
        }
    }

    fn key_event(key: egui::Key, modifiers: egui::Modifiers, pressed: bool) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed,
            repeat: false,
            modifiers,
        }
    }

    #[test]
    fn local_consumption_removes_only_exact_main_key_presses() {
        for name in ["P", "N", "F"] {
            let binding = parse_binding(&format!("Ctrl+{name}")).unwrap();
            let key = binding.local_key;
            let release = key_event(key, egui::Modifiers::CTRL, false);
            let other = key_event(egui::Key::A, egui::Modifiers::CTRL, true);
            let extra_shift = key_event(key, egui::Modifiers::CTRL | egui::Modifiers::SHIFT, true);
            let extra_alt = key_event(key, egui::Modifiers::CTRL | egui::Modifiers::ALT, true);
            let plain = key_event(key, egui::Modifiers::NONE, true);
            let text = egui::Event::Text("text".to_owned());
            let retained = vec![release, other, extra_shift, extra_alt, plain, text];
            let mut events = retained.clone();
            events.push(key_event(key, egui::Modifiers::CTRL, true));
            consume_binding_events(&mut events, &binding, false);
            assert_eq!(events, retained);
        }
    }

    #[test]
    fn local_consumption_preserves_modifier_release_and_requires_exact_win_state() {
        let binding = parse_binding("Alt+Down").unwrap();
        let alt_release = key_event(egui::Key::ArrowDown, egui::Modifiers::NONE, false);
        let mut events = vec![
            alt_release.clone(),
            key_event(egui::Key::ArrowDown, egui::Modifiers::ALT, true),
        ];
        consume_binding_events(&mut events, &binding, false);
        assert_eq!(events, vec![alt_release]);
        let win_binding = parse_binding("Ctrl+Win+P").unwrap();
        let event = key_event(egui::Key::P, egui::Modifiers::CTRL, true);
        let mut events = vec![event.clone()];
        consume_binding_events(&mut events, &win_binding, false);
        assert_eq!(events, vec![event.clone()]);
        consume_binding_events(&mut events, &win_binding, true);
        assert!(events.is_empty());
        let ctrl_binding = parse_binding("Ctrl+P").unwrap();
        let mut events = vec![event.clone()];
        consume_binding_events(&mut events, &ctrl_binding, true);
        assert_eq!(events, vec![event]);
    }

    #[test]
    fn local_consumption_matches_logical_windows_virtual_key_not_us_physical_position() {
        let binding = parse_binding("Ctrl+P").unwrap();
        let physical_match = egui::Event::Key {
            key: egui::Key::O,
            physical_key: Some(egui::Key::P),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::CTRL,
        };
        let logical_only_match = egui::Event::Key {
            key: egui::Key::P,
            physical_key: Some(egui::Key::O),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::CTRL,
        };
        let mut events = vec![physical_match.clone(), logical_only_match];
        consume_binding_events(&mut events, &binding, false);
        // RegisterHotKey uses VK_P, not the US keyboard's P scan-code position.
        assert_eq!(events, vec![physical_match]);
    }

    #[test]
    fn each_active_press_is_forwarded_including_quick_repeated_presses() {
        // MOD_NOREPEAT suppresses key-hold autorepeat at the OS registration;
        // independent key-downs are always accepted, regardless of spacing.
        for _ in 0..5 {
            assert!(is_active_press(42, 42, HotKeyState::Pressed));
        }
    }

    #[test]
    fn released_or_stale_hotkeys_are_ignored() {
        assert!(!is_active_press(42, 42, HotKeyState::Released));
        assert!(!is_active_press(42, 41, HotKeyState::Pressed));
        assert!(!is_active_press(0, 42, HotKeyState::Pressed));
    }
}
