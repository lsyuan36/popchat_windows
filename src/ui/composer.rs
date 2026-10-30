//! Composer input ported from `ComposerView.swift` (capsule field + slash
//! popup + ↑/↓ prompt recall + editor morph).
//!
//! Windows keymap (Swift used ⌘ where noted):
//!
//! * Multiline field, 1–4 rows adaptive. `Enter` sends, `Shift+Enter` newline.
//! * Leading `/` (no space yet) opens the autocomplete popup below the field:
//!   `↑`/`↓` move, `Tab`/`Enter` apply, `Esc` dismisses until the next edit,
//!   click applies. Filter semantics mirror
//!   [`ShortcutStore::complete`](crate::chat::commands::ShortcutStore::complete).
//! * Empty field + `↑`/`↓` recalls sent prompts ([`PromptHistory`]); a
//!   non-empty field keeps the keys for the popup / caret. Recall walks a
//!   snapshot taken when the walk starts, so a send landing mid-walk can't
//!   shift the list under the user (same as Swift's `recall` tuple).
//! * `Ctrl+E` toggles the 400×300 large editor; inside it plain `Enter` is a
//!   newline and `Ctrl+Enter` (or its Send button) sends and closes it.
//!
//! Sending clears the field and records the raw prompt via
//! [`PromptHistory::record`] (dedup makes the second record inside
//! [`ChatStore::send`](crate::chat::store::ChatStore::send) a no-op).
//! Parse/expand itself stays in `commands.rs` — this module only filters.
//!
//! Host wiring (orchestrator): hold one [`Composer`] in the app struct and
//! replace the `show_composer` body with
//! `match composer.show(ui, &shortcuts, &mut chat.prompts) {
//! ComposerAction::Send(text) => { /* existing send_current path */ }
//! ComposerAction::None => {} }`.
//! NOTE: `shortcuts` here is the `commands::ShortcutStore`, while `main.rs`
//! currently holds `store::ShortcutStore` (different type, no `complete()`).
//! The orchestrator must bridge the two (convert or unify) when wiring.
//! `Esc` with no popup open is intentionally NOT consumed so the host's
//! global Esc-to-hide keeps working.

use egui::{Key, Modifiers};

use crate::chat::commands::{PromptShortcut, ShortcutStore};
use crate::chat::store::{Attachment, PromptHistory};

/// 彈窗閘門：草稿為 `/` 前綴且有匹配才彈（空草稿絕不彈）。
/// pre-widget 與 post-widget 兩處共用此判斷，避免語義漂移。
pub fn popup_should_show(draft: &str, shortcuts: &[PromptShortcut]) -> bool {
    match autocomplete_query(draft) {
        Some(q) => !filter_commands(shortcuts, q).is_empty(),
        None => false,
    }
}

/// Result of one [`Composer::show`] frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComposerAction {
    None,
    Send(String),
    PickAttachments,
    Stop,
}

/// AppKit's hasMarkedText guard: a candidate-confirming Return/Escape belongs
/// to the IME, not Send/autocomplete/dismiss. Commit may share a frame with
/// Return, so remember the pre-event state as well as the final state.
pub fn ime_blocks_shortcuts(composing: &mut bool, events: &[egui::Event]) -> bool {
    let mut blocked = *composing;
    for event in events {
        match event {
            egui::Event::Ime(egui::ImeEvent::Preedit(text)) => {
                *composing = !text.is_empty();
                blocked |= *composing;
            }
            egui::Event::Ime(egui::ImeEvent::Commit(_)) => {
                blocked = true;
                *composing = false;
            }
            egui::Event::Ime(egui::ImeEvent::Disabled) | egui::Event::WindowFocused(false) => {
                *composing = false;
            }
            _ => {}
        }
    }
    blocked
}

/// Transient typing state for the composer. Owns the draft so keystrokes
/// never reach the transcript; the host only sees [`ComposerAction::Send`].
#[derive(Debug, Clone)]
pub struct Composer {
    text: String,
    recall_cursor: Option<usize>,
    recall_snapshot: Vec<String>,
    show_large_editor: bool,
    autocomplete_index: usize,
    autocomplete_dismissed: bool,
    /// 上一幀自動完成彈窗是否可見（host 的 Esc-to-hide 讓位給它）。
    popup_visible: bool,
    focus_requested: bool,
    attachments: Vec<Attachment>,
    send_enabled: bool,
    input_focused: bool,
    ime_composing: bool,
    streaming: bool,
}

impl Composer {
    pub fn new() -> Self {
        Composer {
            text: String::new(),
            recall_cursor: None,
            recall_snapshot: Vec::new(),
            show_large_editor: false,
            autocomplete_index: 0,
            autocomplete_dismissed: false,
            popup_visible: false,
            focus_requested: true,
            attachments: Vec::new(),
            send_enabled: true,
            input_focused: false,
            ime_composing: false,
            streaming: false,
        }
    }

    /// 上一幀自動完成彈窗是否可見。
    pub fn popup_visible(&self) -> bool {
        self.popup_visible
    }

    /// Current draft (read-only; mutate via [`Composer::set_text`]).
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Whether the large editor window is open.
    pub fn is_large_editor_open(&self) -> bool {
        self.show_large_editor
    }

    pub fn request_focus(&mut self) {
        self.focus_requested = true;
    }

    pub fn attachments(&self) -> &[Attachment] {
        &self.attachments
    }

    pub fn set_send_enabled(&mut self, enabled: bool) {
        self.send_enabled = enabled;
    }
    pub fn set_streaming(&mut self, streaming: bool) {
        self.streaming = streaming;
    }
    pub fn has_input_focus(&self) -> bool {
        self.input_focused
    }

    pub fn add_attachments(&mut self, attachments: Vec<Attachment>) {
        if attachments.is_empty() {
            return;
        }
        self.attachments.extend(attachments);
        self.request_focus();
    }

    pub fn take_attachments(&mut self) -> Vec<Attachment> {
        std::mem::take(&mut self.attachments)
    }

    pub fn set_draft(&mut self, text: String, attachments: Vec<Attachment>) {
        self.set_text(text);
        self.attachments = attachments;
        self.request_focus();
    }

    pub fn remove_attachment(&mut self, id: uuid::Uuid) {
        self.attachments.retain(|attachment| attachment.id != id);
    }

    pub fn attachment_height(&self) -> f32 {
        if self.attachments.is_empty() {
            0.0
        } else {
            34.0
        }
    }

    fn show_attachments(&mut self, ui: &mut egui::Ui) {
        if self.attachments.is_empty() {
            return;
        }
        let mut remove = None;
        egui::ScrollArea::horizontal()
            .id_salt("draft_attachments")
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for attachment in &self.attachments {
                        let response =
                            crate::ui::controls::button(ui, format!("{}  ×", attachment.filename));
                        let response = response
                            .on_hover_text(attachment.note.as_deref().unwrap_or("移除附件"));
                        if response.clicked() {
                            remove = Some(attachment.id);
                        }
                    }
                });
            });
        if let Some(id) = remove {
            self.remove_attachment(id);
        }
        ui.add_space(6.0);
    }

    pub fn inline_rows(&self) -> usize {
        (self.text.matches('\n').count() + 1).clamp(1, 8)
    }

    /// External draft injection (Edit-prompt path): replaces the draft and
    /// drops any in-progress recall walk, like Swift's `injectedDraft`.
    pub fn set_text(&mut self, text: String) {
        self.text = text;
        self.recall_cursor = None;
        self.recall_snapshot.clear();
        self.autocomplete_index = 0;
        self.autocomplete_dismissed = false;
    }

    /// Clears draft + transient state without sending.
    pub fn clear(&mut self) {
        self.set_text(String::new());
        self.attachments.clear();
        self.show_large_editor = false;
    }

    /// Draws the composer (inline row + optional popup + optional large
    /// editor) and returns [`ComposerAction::Send`] on the frame the user
    /// sends.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        shortcuts: &ShortcutStore,
        history: &mut PromptHistory,
    ) -> ComposerAction {
        let mut action = ComposerAction::None;
        self.show_attachments(ui);

        // ---- Frame-global key snapshot (read before any widget consumes them).
        let ime_blocked = ui.input(|i| ime_blocks_shortcuts(&mut self.ime_composing, &i.events));
        let (enter_pressed, tab_pressed, up_pressed, down_pressed, esc_pressed) = ui.input(|i| {
            (
                !ime_blocked && i.key_pressed(Key::Enter),
                !ime_blocked && i.key_pressed(Key::Tab),
                !ime_blocked && i.key_pressed(Key::ArrowUp),
                !ime_blocked && i.key_pressed(Key::ArrowDown),
                !ime_blocked && i.key_pressed(Key::Escape),
            )
        });
        let shift_down = ui.input(|i| i.modifiers.shift);
        let ctrl_down = ui.input(|i| i.modifiers.ctrl);
        let ctrl_e = !ime_blocked && ui.input(|i| i.key_pressed(Key::E) && i.modifiers.ctrl);

        // Ctrl+E toggles the large editor regardless of focus (Swift: ⌘E).
        if ctrl_e {
            self.show_large_editor = !self.show_large_editor;
            self.focus_requested = true;
        }
        if esc_pressed && self.show_large_editor {
            self.show_large_editor = false;
            self.focus_requested = true;
            ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
        }
        if self.show_large_editor {
            self.popup_visible = false;
            return self.show_expanded(ui, history, ctrl_down && enter_pressed);
        }

        let input_id = egui::Id::new("popchat_composer_text");
        let input_focused = ui.memory(|m| m.focused() == Some(input_id));

        // ---- Pre-widget key handling against the pre-edit draft.
        // Consuming before the TextEdit sees the key keeps stray "\n"/"\t"
        // and caret jumps out of the draft.
        // 只有 `/` 前綴草稿才彈自動完成（空草稿不彈——實測踩過：
        // unwrap_or("") 讓空前綴匹配全部指令，空態也整個彈出來）。
        let query_opt = autocomplete_query(&self.text);
        let mut candidates = match query_opt {
            Some(q) => filter_commands(&shortcuts.shortcuts, q),
            None => Vec::new(),
        };
        let popup_open = !candidates.is_empty() && !self.autocomplete_dismissed;
        if popup_open {
            self.autocomplete_index = self.autocomplete_index.min(candidates.len() - 1);
        }

        if input_focused && !self.show_large_editor {
            if popup_open {
                if up_pressed {
                    let _ = ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowUp));
                    self.autocomplete_index = self.autocomplete_index.saturating_sub(1);
                } else if down_pressed {
                    let _ = ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowDown));
                    self.autocomplete_index =
                        (self.autocomplete_index + 1).min(candidates.len() - 1);
                } else if tab_pressed || (enter_pressed && !shift_down && !ctrl_down) {
                    // Tab / Enter applies (Shift+Enter stays a newline, as in Swift).
                    let key = if tab_pressed { Key::Tab } else { Key::Enter };
                    let _ = ui.input_mut(|i| i.consume_key(Modifiers::NONE, key));
                    if let Some(chosen) = candidates.get(self.autocomplete_index) {
                        let name = chosen.name.clone();
                        self.text = format!("/{name} ");
                        self.autocomplete_index = 0;
                    }
                    candidates.clear();
                } else if esc_pressed {
                    let _ = ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
                    self.autocomplete_dismissed = true;
                }
            } else if enter_pressed && !shift_down && !ctrl_down {
                // Plain Enter sends (Shift+Enter falls through to a newline).
                let _ = ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
                if let Some(payload) = self.take_send(history) {
                    action = ComposerAction::Send(payload);
                }
            } else if up_pressed && (self.recall_cursor.is_some() || self.text.is_empty()) {
                let snapshot_len = self
                    .recall_cursor
                    .map(|_| self.recall_snapshot.len())
                    .unwrap_or_else(|| history.entries().len());
                if let Some(next) = recall_older_index(snapshot_len, self.recall_cursor) {
                    let _ = ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowUp));
                    if self.recall_cursor.is_none() {
                        self.recall_snapshot = history.entries().to_vec();
                    }
                    self.recall_cursor = Some(next);
                    if let Some(entry) = self.recall_snapshot.get(next) {
                        self.text = entry.clone();
                    }
                }
            } else if down_pressed && self.recall_cursor.is_some() {
                let _ = ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowDown));
                match recall_newer_index(self.recall_cursor) {
                    RecallNewer::Inactive => {}
                    RecallNewer::Show(i) => {
                        self.recall_cursor = Some(i);
                        if let Some(entry) = self.recall_snapshot.get(i) {
                            self.text = entry.clone();
                        }
                    }
                    RecallNewer::RestoreDraft => {
                        self.recall_cursor = None;
                        self.recall_snapshot.clear();
                        self.text.clear();
                    }
                }
            }
            // Anything else (non-empty draft + ↑/↓, lone Tab, Esc with no
            // popup) keeps the TextEdit / host default behavior.
        }
        // Draft after programmatic key handling, before the user can edit.
        let after_keys = self.text.clone();

        // One TextEdit identity for inline/expanded layout preserves the
        // cursor, selection and undo state, like Swift's reparented text view.
        // ---- Inline row: multiline field (1–8 rows adaptive) + Send.
        // Host 把整行包在膠囊 Frame 裡；TextEdit 無邊框、Send 圓形 accent。
        let response = ui.horizontal(|ui| {
            let rows = self.inline_rows();
            let field_width =
                (ui.available_width() - 84.0 - 3.0 * ui.spacing().item_spacing.x).max(80.0);
            let pick = crate::ui::icons::button(
                ui,
                crate::ui::icons::Icon::Add,
                false,
                "加入附件（也可貼上圖片或拖入檔案）",
            )
            .clicked();
            let te = ui.add(
                egui::TextEdit::multiline(&mut self.text)
                    .id(input_id)
                    .hint_text("輸入訊息，或用 / 呼叫快捷指令…")
                    .desired_rows(rows)
                    .desired_width(field_width)
                    .margin(egui::Margin::ZERO)
                    .frame(false),
            );
            if self.focus_requested && !self.show_large_editor {
                te.request_focus();
                self.focus_requested = false;
            }
            if crate::ui::icons::button(
                ui,
                crate::ui::icons::Icon::Expand,
                self.show_large_editor,
                "展開編輯器 · Ctrl+E",
            )
            .clicked()
            {
                self.show_large_editor = !self.show_large_editor;
                self.focus_requested = true;
            }
            let can_send = self.streaming
                || self.send_enabled
                    && (!self.text.trim().is_empty() || !self.attachments.is_empty());
            let clicked = ui
                .add_enabled_ui(can_send, |ui| {
                    crate::ui::icons::button(
                        ui,
                        if self.streaming {
                            crate::ui::icons::Icon::Stop
                        } else {
                            crate::ui::icons::Icon::Send
                        },
                        true,
                        if self.streaming {
                            "停止回覆"
                        } else {
                            "送出 · Enter（Shift+Enter 換行）"
                        },
                    )
                    .clicked()
                })
                .inner;
            (te, clicked, pick)
        });
        let inner = response.inner.0;
        self.input_focused = inner.has_focus();
        let button_send = response.inner.1;
        if response.inner.2 {
            action = ComposerAction::PickAttachments;
        }
        if button_send {
            if self.streaming {
                action = ComposerAction::Stop;
            } else if let Some(payload) = self.take_send(history) {
                action = ComposerAction::Send(payload);
            }
        }
        if matches!(action, ComposerAction::Send(_)) && !self.show_large_editor {
            inner.request_focus();
        }

        // Editing the draft resets popup + recall (a recalled prompt you
        // type into becomes yours; further ↑ starts a fresh walk).
        if self.text != after_keys {
            self.autocomplete_index = 0;
            self.autocomplete_dismissed = false;
            if let Some(i) = self.recall_cursor {
                if self.recall_snapshot.get(i).map(String::as_str) != Some(self.text.as_str()) {
                    self.recall_cursor = None;
                    self.recall_snapshot.clear();
                }
            }
        }

        // ---- Autocomplete popup from the post-edit draft.
        // 同樣只在 `/` 前綴時彈出（見上）；`/` 開頭但無匹配（如 /zzz）不彈。
        let show_popup = popup_should_show(&self.text, &shortcuts.shortcuts);
        if show_popup && !self.autocomplete_dismissed {
            let fresh_query = autocomplete_query(&self.text).unwrap_or("").to_owned();
            let fresh = filter_commands(&shortcuts.shortcuts, &fresh_query);
            self.autocomplete_index = self.autocomplete_index.min(fresh.len() - 1);
            let selected = self.autocomplete_index;
            let mut apply: Option<usize> = None;
            // 浮在輸入框上方的 Area（composer 貼底，往下開會被視窗截斷；
            // 原版也是向上彈）。
            egui::Area::new(ui.id().with("popchat_autocomplete"))
                .order(egui::Order::Foreground)
                .fixed_pos(inner.rect.left_top())
                .pivot(egui::Align2::LEFT_BOTTOM)
                .show(ui.ctx(), |ui| {
                    // 彈窗限寬（卡片寬的八成）＋單行截斷，避免長模板撐滿整幅。
                    let card_w = ui
                        .ctx()
                        .input(|i| i.viewport().inner_rect)
                        .map(|r| r.width() * 0.8)
                        .unwrap_or(420.0);
                    ui.set_max_width(card_w);
                    egui::Frame::group(ui.style())
                        .corner_radius(egui::CornerRadius::same(10))
                        .show(ui, |ui| {
                            for (i, shortcut) in fresh.iter().enumerate() {
                                let preview: String = shortcut
                                    .template
                                    .replace('\n', " ")
                                    .chars()
                                    .take(80)
                                    .collect();
                                let row = format!("/{} — {preview}", shortcut.name);
                                // 字元數粗截 + Truncate wrap 雙保險（Area 寬有限）。
                                let row = if row.chars().count() > 70 {
                                    let cut: String = row.chars().take(69).collect();
                                    format!("{cut}…")
                                } else {
                                    row
                                };
                                if ui
                                    .add(
                                        egui::Button::selectable(
                                            i == selected,
                                            egui::RichText::new(row),
                                        )
                                        .wrap_mode(egui::TextWrapMode::Truncate)
                                        .min_size(egui::vec2(card_w - 24.0, 22.0)),
                                    )
                                    .clicked()
                                {
                                    apply = Some(i);
                                }
                            }
                        });
                });
            if let Some(i) = apply {
                if let Some(chosen) = fresh.get(i) {
                    self.text = format!("/{} ", chosen.name);
                    self.autocomplete_index = 0;
                    inner.request_focus();
                }
            }
            self.popup_visible = true;
        } else {
            self.popup_visible = false;
        }

        action
    }

    fn show_expanded(
        &mut self,
        ui: &mut egui::Ui,
        history: &mut PromptHistory,
        shortcut_send: bool,
    ) -> ComposerAction {
        let mut close = false;
        let mut pick = false;
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("撰寫訊息").size(17.0).strong());
            pick = crate::ui::controls::button(ui, "＋ 附件").clicked();
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                close = ui
                    .add(
                        egui::Button::new(egui::RichText::new("完成編輯").size(12.0).weak())
                            .frame(false),
                    )
                    .clicked();
            });
        });
        ui.add_space(12.0);
        let editor_id = egui::Id::new("popchat_composer_text");
        let focused = ui.memory(|memory| memory.focused() == Some(editor_id));
        if shortcut_send && focused {
            ui.input_mut(|input| input.consume_key(Modifiers::CTRL, Key::Enter));
        }
        let height = (ui.available_height() - 80.0).max(120.0);
        egui::ScrollArea::vertical()
            .id_salt("expanded_draft_scroll")
            .max_height(height)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let field = ui.add_sized(
                    [ui.available_width(), height],
                    egui::TextEdit::multiline(&mut self.text)
                        .id(editor_id)
                        .hint_text("把完整的想法寫下來…")
                        .frame(false)
                        .margin(egui::Margin::ZERO),
                );
                if self.focus_requested {
                    field.request_focus();
                    self.focus_requested = false;
                }
                self.input_focused = field.has_focus();
            });
        ui.add_space(12.0);
        let mut clicked = false;
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("Ctrl+Enter 傳送  ·  Esc 返回")
                    .size(11.0)
                    .weak(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                clicked = ui
                    .add_enabled_ui(
                        self.streaming
                            || (self.send_enabled
                                && (!self.text.trim().is_empty() || !self.attachments.is_empty())),
                        |ui| {
                            crate::ui::icons::button(
                                ui,
                                if self.streaming {
                                    crate::ui::icons::Icon::Stop
                                } else {
                                    crate::ui::icons::Icon::Send
                                },
                                true,
                                if self.streaming {
                                    "停止回覆"
                                } else {
                                    "傳送訊息"
                                },
                            )
                        },
                    )
                    .inner
                    .clicked();
            });
        });
        if clicked && self.streaming {
            return ComposerAction::Stop;
        }
        if clicked || (shortcut_send && focused) {
            if let Some(text) = self.take_send(history) {
                self.show_large_editor = false;
                self.focus_requested = true;
                return ComposerAction::Send(text);
            }
        }
        if close {
            self.show_large_editor = false;
            self.focus_requested = true;
        }
        if pick {
            return ComposerAction::PickAttachments;
        }
        ComposerAction::None
    }

    /// Clears the draft into a send payload: trims, records the raw prompt
    /// in history, resets popup/recall state. `None` when there is nothing
    /// to send.
    fn take_send(&mut self, history: &mut PromptHistory) -> Option<String> {
        if !self.send_enabled || self.streaming {
            return None;
        }
        let payload = self.text.trim().to_string();
        if payload.is_empty() && self.attachments.is_empty() {
            return None;
        }
        if !payload.is_empty() {
            history.record(&payload);
        }
        self.text.clear();
        self.recall_cursor = None;
        self.recall_snapshot.clear();
        self.autocomplete_index = 0;
        self.autocomplete_dismissed = false;
        Some(payload)
    }
}

impl Default for Composer {
    fn default() -> Self {
        Composer::new()
    }
}

// MARK: - Pure helpers (unit-tested below; UI-independent)

/// Returns the slash query when `draft` should show autocomplete: starts
/// with `/` and contains no whitespace yet (Swift additionally only checked
/// for `" "`; tabs/newlines are treated the same here).
pub fn autocomplete_query(draft: &str) -> Option<&str> {
    let body = draft.strip_prefix('/')?;
    if body.contains(char::is_whitespace) {
        return None;
    }
    Some(body)
}

/// Case-insensitive prefix filter over shortcuts in store order. Mirrors
/// `commands::ShortcutStore::complete` (which also tolerates a leading `/`
/// on the prefix); kept as a free function so the recall/autocomplete tests
/// run without a store.
pub fn filter_commands<'a>(
    shortcuts: &'a [PromptShortcut],
    prefix: &str,
) -> Vec<&'a PromptShortcut> {
    let needle = prefix.strip_prefix('/').unwrap_or(prefix).to_lowercase();
    shortcuts
        .iter()
        .filter(|s| s.name.to_lowercase().starts_with(&needle))
        .collect()
}

/// Next recall position for ↑: `None` when there is nothing to show
/// (empty history — the host must leave the key to the field); saturates at
/// the oldest entry so the key stays consumed there.
pub fn recall_older_index(len: usize, cursor: Option<usize>) -> Option<usize> {
    if len == 0 {
        return None;
    }
    match cursor {
        None => Some(0),
        Some(i) => Some((i + 1).min(len - 1)),
    }
}

/// Outcome of ↓ while browsing history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecallNewer {
    /// Not browsing — leave the key to the field.
    Inactive,
    /// Show the snapshot entry at this index.
    Show(usize),
    /// Walked past the newest entry — restore the empty composer.
    RestoreDraft,
}

/// Next ↓ step for the recall cursor.
pub fn recall_newer_index(cursor: Option<usize>) -> RecallNewer {
    match cursor {
        None => RecallNewer::Inactive,
        Some(0) => RecallNewer::RestoreDraft,
        Some(i) => RecallNewer::Show(i - 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streaming_keeps_editor_live_and_stop_does_not_consume_the_next_draft() {
        for expanded in [false, true] {
            let ctx = egui::Context::default();
            let mut composer = Composer::new();
            composer.show_large_editor = expanded;
            composer.set_streaming(true);
            let commands = ShortcutStore { shortcuts: vec![] };
            let dir = tempfile::tempdir().unwrap();
            let mut history = PromptHistory::with_storage_path(dir.path().join("history.json"));
            let mut draw = |events| {
                let mut action = ComposerAction::None;
                let output = ctx.run(
                    egui::RawInput {
                        events,
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(600.0, 460.0),
                        )),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            action = composer.show(ui, &commands, &mut history);
                        });
                    },
                );
                (output, action)
            };
            draw(vec![]);
            draw(vec![]);
            let (output, action) = draw(vec![egui::Event::Text("next question".to_owned())]);
            assert_eq!(action, ComposerAction::None);
            let stop = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Circle(circle)
                        if (circle.radius - 14.0).abs() < 0.1 && circle.fill.a() > 0 =>
                    {
                        Some(circle.center)
                    }
                    _ => None,
                })
                .expect("real Stop button remains rendered beside the editor");
            let press = egui::Event::PointerButton {
                pos: stop,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            };
            draw(vec![egui::Event::PointerMoved(stop), press]);
            let (_, action) = draw(vec![egui::Event::PointerButton {
                pos: stop,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            }]);
            assert_eq!(action, ComposerAction::Stop);
            assert_eq!(composer.text(), "next question");
            assert_eq!(composer.is_large_editor_open(), expanded);
            assert!(history.entries().is_empty());
            assert!(!dir.path().join("history.json").exists());
        }
    }

    #[test]
    fn inline_and_expanded_editor_share_cursor_selection_and_undo_state() {
        use egui::text::{CCursor, CCursorRange};
        let ctx = egui::Context::default();
        let mut composer = Composer::new();
        composer.set_text("first\nsecond".to_owned());
        let commands = ShortcutStore { shortcuts: vec![] };
        let dir = tempfile::tempdir().unwrap();
        let mut history = PromptHistory::with_storage_path(dir.path().join("history.json"));
        let draw = |composer: &mut Composer, history: &mut PromptHistory| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(600.0, 460.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        assert_eq!(composer.show(ui, &commands, history), ComposerAction::None);
                    });
                },
            );
        };
        draw(&mut composer, &mut history);
        draw(&mut composer, &mut history);
        let id = egui::Id::new("popchat_composer_text");
        let range = CCursorRange::two(CCursor::new(1), CCursor::new(4));
        let mut state = egui::TextEdit::load_state(&ctx, id).unwrap();
        state.cursor.set_char_range(Some(range));
        let mut undo = state.undoer();
        undo.add_undo(&(range, "before".to_owned()));
        state.set_undoer(undo);
        state.store(&ctx, id);
        for expanded in [true, false] {
            composer.show_large_editor = expanded;
            composer.request_focus();
            draw(&mut composer, &mut history);
            let state = egui::TextEdit::load_state(&ctx, id).unwrap();
            assert_eq!(state.cursor.char_range(), Some(range));
            assert!(state
                .undoer()
                .has_undo(&(range, composer.text().to_owned())));
            assert_eq!(composer.text(), "first\nsecond");
        }
        composer.set_text("x\n".repeat(20));
        assert_eq!(composer.inline_rows(), 8);
    }

    #[test]
    fn ime_candidate_keys_and_commit_frame_do_not_trigger_send() {
        let ctx = egui::Context::default();
        let mut composer = Composer::new();
        composer.set_text("draft ".to_owned());
        let commands = ShortcutStore { shortcuts: vec![] };
        let dir = tempfile::tempdir().unwrap();
        let mut history = PromptHistory::with_storage_path(dir.path().join("history.json"));
        let mut draw = |events| {
            let mut action = ComposerAction::None;
            let _ = ctx.run(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        action = composer.show(ui, &commands, &mut history);
                    });
                },
            );
            action
        };
        assert_eq!(draw(vec![]), ComposerAction::None);
        let enter = || egui::Event::Key {
            key: Key::Enter,
            physical_key: Some(Key::Enter),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        assert_eq!(
            draw(vec![
                egui::Event::Ime(egui::ImeEvent::Enabled),
                egui::Event::Ime(egui::ImeEvent::Preedit("ㄅ".to_owned())),
                enter()
            ]),
            ComposerAction::None
        );
        assert_eq!(
            draw(vec![
                egui::Event::Ime(egui::ImeEvent::Commit("你".to_owned())),
                enter()
            ]),
            ComposerAction::None
        );
        assert!(history.entries().is_empty());
        assert!(!dir.path().join("history.json").exists());
        let mut composing = true;
        assert!(ime_blocks_shortcuts(
            &mut composing,
            &[egui::Event::Ime(egui::ImeEvent::Disabled)]
        ));
        assert!(!ime_blocks_shortcuts(&mut composing, &[]));
    }

    #[test]
    fn attachment_only_send_and_edit_draft_removal_use_production_state() {
        use crate::chat::store::AttachmentContent;
        let attachment =
            Attachment::new("test.txt", AttachmentContent::Text("file text".to_owned()));
        let mut composer = Composer::new();
        composer.set_draft(String::new(), vec![attachment.clone()]);
        let mut history = PromptHistory::default();
        composer.set_send_enabled(false);
        assert!(composer.take_send(&mut history).is_none());
        assert_eq!(composer.attachments(), [attachment.clone()]);
        composer.set_send_enabled(true);
        assert_eq!(composer.take_send(&mut history), Some(String::new()));
        assert_eq!(composer.take_attachments(), [attachment.clone()]);
        assert!(history.entries().is_empty());
        composer.set_draft("edit me".to_owned(), vec![attachment.clone()]);
        composer.remove_attachment(attachment.id);
        assert_eq!(composer.text(), "edit me");
        assert!(composer.attachments().is_empty());
        composer.add_attachments(vec![attachment]);
        composer.clear();
        assert!(composer.text().is_empty());
        assert!(composer.attachments().is_empty());
    }

    #[test]
    fn inline_expanded_editor_fits_and_keeps_the_draft() {
        let ctx = egui::Context::default();
        crate::ui::theme::apply(&ctx, "light", crate::ui::theme::DEFAULT_ACCENT_HEX);
        let mut composer = Composer::new();
        composer.set_text("A longer draft\nwith a second line".to_owned());
        composer.show_large_editor = true;
        let commands = ShortcutStore {
            shortcuts: Vec::new(),
        };
        let mut history = PromptHistory::default();
        for _ in 0..2 {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(600.0, 430.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let available = ui.available_size();
                        let rendered = ui.scope(|ui| composer.show(ui, &commands, &mut history));
                        assert!(rendered.response.rect.width() <= available.x + 1.0);
                        assert!(
                            rendered.response.rect.height() <= available.y + 1.0,
                            "editor exceeds viewport: {:?}",
                            rendered.response.rect
                        );
                        assert!(matches!(rendered.inner, ComposerAction::None));
                    });
                },
            );
        }
        assert_eq!(composer.text(), "A longer draft\nwith a second line");
    }

    #[test]
    fn rendered_composer_fits_narrow_panel_and_editor_escape_keeps_draft() {
        let ctx = egui::Context::default();
        let mut composer = Composer::new();
        composer.set_text("a draft to keep".to_owned());
        let commands = ShortcutStore {
            shortcuts: Vec::new(),
        };
        let mut history = PromptHistory::default();
        let input = || egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(480.0, 420.0),
            )),
            ..Default::default()
        };
        for _ in 0..2 {
            let _ = ctx.run(input(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let width = ui.available_width();
                    let response = ui.scope(|ui| {
                        composer.show(ui, &commands, &mut history);
                    });
                    assert!(
                        response.response.rect.width() <= width + 1.0,
                        "composer={} available={width}",
                        response.response.rect.width()
                    );
                });
            });
        }
        composer.show_large_editor = true;
        let mut escape = input();
        escape.events.push(egui::Event::Key {
            key: Key::Escape,
            physical_key: Some(Key::Escape),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        });
        let _ = ctx.run(escape, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                assert!(matches!(
                    composer.show(ui, &commands, &mut history),
                    ComposerAction::None
                ));
            });
        });
        assert!(!composer.is_large_editor_open());
        assert_eq!(composer.text(), "a draft to keep");
    }

    fn shortcuts() -> Vec<PromptShortcut> {
        vec![
            PromptShortcut {
                id: "1".to_string(),
                name: "translate".to_string(),
                template: "Translate:\n\n{input}".to_string(),
            },
            PromptShortcut {
                id: "2".to_string(),
                name: "explain".to_string(),
                template: "Explain {input}".to_string(),
            },
            PromptShortcut {
                id: "3".to_string(),
                name: "test".to_string(),
                template: "Test".to_string(),
            },
        ]
    }

    fn names<'a>(v: Vec<&'a PromptShortcut>) -> Vec<&'a str> {
        v.into_iter().map(|s| s.name.as_str()).collect()
    }

    #[test]
    fn query_only_for_bare_slash_prefix() {
        assert_eq!(autocomplete_query("/"), Some(""));
        assert_eq!(autocomplete_query("/tr"), Some("tr"));
        assert_eq!(autocomplete_query("/translate hello"), None);
        assert_eq!(autocomplete_query("/a\tb"), None);
        assert_eq!(autocomplete_query("plain"), None);
        assert_eq!(autocomplete_query(""), None);
    }

    #[test]
    fn empty_draft_never_opens_popup() {
        // 曾因 unwrap_or("") + 空前綴匹配全部，空態也彈窗（截圖實抓）。
        let all = shortcuts();
        assert!(!popup_should_show("", &all));
        assert!(!popup_should_show("  ", &all));
        assert!(!popup_should_show("plain", &all));
        assert!(popup_should_show("/", &all));
        assert!(popup_should_show("/tr", &all));
        assert!(!popup_should_show("/zzz", &all));
        // 已帶參數的 slash 呼叫不再彈（與 autocomplete_query 語義一致）。
        assert!(!popup_should_show("/translate hello", &all));
    }

    #[test]
    fn filter_is_case_insensitive_prefix_in_store_order() {
        let all = shortcuts();
        assert_eq!(
            names(filter_commands(&all, "")),
            ["translate", "explain", "test"]
        );
        assert_eq!(
            names(filter_commands(&all, "/")),
            ["translate", "explain", "test"]
        );
        assert_eq!(names(filter_commands(&all, "t")), ["translate", "test"]);
        assert_eq!(names(filter_commands(&all, "T")), ["translate", "test"]);
        assert_eq!(names(filter_commands(&all, "/exp")), ["explain"]);
        // Substring but not prefix: no match.
        assert!(filter_commands(&all, "xplain").is_empty());
        assert!(filter_commands(&all, "zzz").is_empty());
    }

    #[test]
    fn recall_older_starts_walks_and_saturates() {
        assert_eq!(recall_older_index(0, None), None);
        assert_eq!(recall_older_index(0, Some(0)), None);
        assert_eq!(recall_older_index(3, None), Some(0));
        assert_eq!(recall_older_index(3, Some(0)), Some(1));
        assert_eq!(recall_older_index(3, Some(1)), Some(2));
        // At the oldest entry the key stays consumed (no caret move).
        assert_eq!(recall_older_index(3, Some(2)), Some(2));
        // Out-of-range cursor (history shrank mid-walk) clamps, never panics.
        assert_eq!(recall_older_index(3, Some(99)), Some(2));
    }

    #[test]
    fn recall_newer_walks_back_to_draft() {
        assert_eq!(recall_newer_index(None), RecallNewer::Inactive);
        assert_eq!(recall_newer_index(Some(2)), RecallNewer::Show(1));
        assert_eq!(recall_newer_index(Some(1)), RecallNewer::Show(0));
        assert_eq!(recall_newer_index(Some(0)), RecallNewer::RestoreDraft);
    }

    #[test]
    fn full_walk_older_then_newer() {
        // Snapshot newest-first, as PromptHistory::entries() stores them.
        let snapshot = vec![
            "third".to_string(),
            "second".to_string(),
            "first".to_string(),
        ];
        let mut cursor = None;
        let mut shown = Vec::new();
        while let Some(next) = recall_older_index(snapshot.len(), cursor) {
            if cursor == Some(next) {
                break; // saturated at oldest
            }
            cursor = Some(next);
            shown.push(snapshot[next].clone());
        }
        assert_eq!(shown, ["third", "second", "first"]);

        // ↓ walks back toward the live draft, then restores it.
        let mut back = Vec::new();
        loop {
            match recall_newer_index(cursor) {
                RecallNewer::Inactive => panic!("walk ended without restoring draft"),
                RecallNewer::Show(i) => {
                    cursor = Some(i);
                    back.push(snapshot[i].clone());
                }
                RecallNewer::RestoreDraft => {
                    cursor = None;
                    break;
                }
            }
        }
        assert_eq!(back, ["second", "third"]);
        assert_eq!(cursor, None);
    }
}
