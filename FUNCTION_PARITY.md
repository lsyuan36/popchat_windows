# Swift function-by-function parity inventory

Static source audit, 2026-10-30. **Not a declaration of behavioral equivalence.**

- Original: `D:\playground\popchat_tmp\Sources\PopChat` (32 Swift files).
- Port: `D:\playground\popchat_windows\src`. This refresh edits only `FUNCTION_PARITY.md` and `PORT_PROGRESS.md`.
- **513 explicit definitions: 450 outside `main.swift`, 63 harness helpers in `main.swift`.** Includes private/nested functions, explicit initializers, and `MessageRow.==`. Excludes synthesized initializers, closures, computed properties, call expressions, and `main.swift:1016`'s `func fib`, which is text inside a Markdown fixture, not executable Swift. UI properties/actions are inventoried separately below.
- Source line numbers are 1-based declaration starts. Overloads and repeated helper names have separate rows. Rust symbols, rather than unstable line anchors, identify counterparts. `←` names an actual caller or an explicit absence of one. Local helper caller chains inherit the production entry identified in their section.
- **Post-integration refresh:** lifecycle, geometry, glyph fade, streaming composer, selection, independent Settings autosave, HSB accent, bubble styles, reasoning, Unicode slash matching, file paste/drop feedback and the six latest requests have production callers identified below. **Alt+Down is now the configurable default**, superseding the previous fixed-key requirement; Alt+Space remains Windows-reserved and rejected. `PARITY_AUDIT.md` and earlier progress snapshots are historical, not current caller authority.
- The static inventory itself did not open/execute the app, providers, browser, credential store or user data. Reading source/test definitions establishes reachability only; test names below are references. Parent-session test results and isolated native screenshots are recorded separately in `PORT_PROGRESS.md`.
- Latest integration: **v0.2.0 — LOCAL BUILD / Evis Lai**, fmt/check/all-target tests without warnings (**536 library, 26 binary, 15 importer example; 3 ignored**). Installed sole chat app **PID 324**, formal profile (`isolated=false`), app/helper hashes and preserved data verified. Hover help, installed TT Norms Pro→Arial defaults and explicit SHA-verified in-app update pipeline wired. Actual package gate separately passed for 877 files; no native live next-version upgrade claim. README/original Le Chen attribution/MIT and dependency asset notices supplied. GitHub publication is a separate remote step, not inferred from tests. Final raw window off-screen; no forced input/Send/API Save/clipboard/startup changes.

## Status vocabulary

| Status | Meaning |
|---|---|
| WIRED | Concrete counterpart has a production caller; does **not** mean every original edge case or live provider is verified. |
| PARTIAL | Reachable subset/replacement with a named remaining behavioral gap; native/live verification limits are recorded separately from reachability. |
| CORE_ONLY | Counterpart exists, but production caller is absent/unproven; tests do not promote it. |
| MISSING | No identified counterpart for the original behavior. |
| PLATFORM_REPLACED | Explicit macOS/framework mechanism replaced by a named Windows/egui mechanism; not an equivalence claim. |
| TEST_HARNESS | Test/debug reference, not a user feature or a missing product function. |

`main::x` means `src/main.rs::PopChatApp::x` unless explicitly labeled a free function. Other abbreviated module names below resolve to `src/chat/<module>.rs` or `src/ui/<module>.rs` as stated in each section.

## Refreshed integration evidence (Rust symbols and callers)

These are production callers, not inferred module/test coverage. Symbols are the stable lookup key while integration edits shift line numbers. See `PORT_PROGRESS.md` for parent-session command results, isolated native screenshots, failed/incomplete checks, and the single-UI-instance testing constraint; no aggregate/native acceptance is inferred from this table.

| Path | Implementation → actual application caller | Classification / remaining boundary |
|---|---|---|
| Single instance/reopen | `src/instance.rs::InstanceGuard::acquire/take_request` ← free `main` **before** eframe; retained `Arc<InstanceGuard>` in app/`spawn_visibility_driver` | PLATFORM_REPLACED; driver polls while hidden, `Show` calls `show`, never toggle; `--settings` forwarded to `open_settings`. Actual multi-process native validation not run by this audit. |
| Launch policy | `src/startup.rs::record_launch/should_present_launch` ← primary-only free `main` / app `new` | WIRED; first non-login opens, login/repeat launch stays hidden, explicit Settings wins. Windows Run-entry `startup::set_enabled` is the SMAppService replacement. |
| First-run hint | `src/ui/tray.rs::PopchatTray::show_first_run_hint/dismiss_first_run_hint` exact tray HWND + own icon ID → native notification ← app `new/update/hide` | PLATFORM_REPLACED; 350ms scheduled caller and dismissal exist; **Windows shell balloon, not macOS anchored popover**. |
| Geometry/resize | `src/ui/panel.rs::PanelGeometry`, `src/ui/layout.rs::ViewportSizing` ← app `new/update_window_geometry`; `resize_edges` ← `show_card`; `.with_resizable(true)` ← free `main` | WIRED / PLATFORM_REPLACED; monitor work areas, position restore, explicit user-resize handshake and debounced preference save. Natural compact height/animations remain PARTIAL; mixed-DPI native matrix not exercised here. |
| Navigation / mode height motion | `transition::PageTransition::update/show` ← `main::show_card`, `SettingsUi::show`; `WindowHeightTransition::update` ← `main::update_window_geometry`; `motion_enabled` ← all three callers | WIRED user request; incoming-only 180ms fade/±6pt real-layout slide, stable widget IDs and original clip, no outgoing-page re-render. Mode-only height tween; composer growth/native resize snap. Read-only cached Windows animation policy honored for these transitions, not all existing animations. Actual translated click/IME/cursor/idle/clip and programmatic-size regression tests passed; native motion acceptance pending. |
| Fade | `src/ui/reveal.rs::paint_reveal` ← `main::show_list` immediately after assistant body with its exclusive shape range, before `paint_find_matches`; clock ← app `update` | WIRED; final prose/code segment only, grapheme bands/sentence ages, controls/math mesh untouched; no automatic claim of streaming-caret or attachment fade equivalence. |
| Streaming editor/state | `src/ui/composer.rs::Composer::show/show_expanded/take_send` ← `main::show_composer_capsule` | WIRED; editor stays editable, Stop replaces Send, draft not consumed during stream, inline/expanded share `popchat_composer_text` TextEdit ID and up to 8 inline rows. Native IME/full-app performance validation remains separate. |
| Bopomofo Enter / preedit override | `ime_blocks_shortcuts/ime_blocks_return → ime_text_edit/paint_preedit_underline` ← both composer editors | WIRED; candidate Return disables TextEdit newline binding as well as composer shortcuts, repeat blocked until release; locally transparent preedit selection plus underline, ordinary selection preserved. Focused/event-order/cancel/discard regressions passed, real native Bopomofo acceptance still pending. |
| Selection | `src/ui/markdown.rs::prose_selection_island/selection_island` ← `MathTextures::show_assistant_message` ← `main::show_list` | WIRED selection islands; shared selection inside prose paragraphs/formatting/links; code/quote/table-cell/pasteable islands intentionally separate, as source. Bitmap attachment/rich clipboard behavior still PARTIAL. |
| Independent preference saves | `src/ui/settings.rs::persist_general_preferences/commands_section` ← `SettingsUi::show`; app live `CommandsChanged` consumer ← `main::settings_actions` | WIRED; general prefs (including font/window), search keys and Commands/system prompt save independently, preserving unsaved API endpoint/key/model draft and unknown JSON fields. Unapplied hotkey draft is excluded; API catalog editing still requires explicit Save. |
| Templates/Codex path | template menu → `main::settings_actions`; native picker in `SettingsUi::codex_controls` → `poll_services` → `CodexPathChanged` → app `settings_actions` → `persist_codex_path` | WIRED; actual template-add caller; path typing/picker immediately updates backend/invalidation/persistence without committing API draft. No live Codex/account claim. |
| Custom accent and bubble style | `src/ui/accent.rs::AccentPicker::show` ← `SettingsUi::appearance_section` ← `show` → independent preference autosave; `AccentPalette::for_bubble_style` ← user row in `main::show_list` | WIRED; rainbow swatch/popover, live HSB gradient sliders, local hue/saturation retention, dedup, reject/reseed Hex and Reset; 15% light/19% dark tint or accent fill with CIE L* contrast. Retired `quietGray`/unknown style falls back to tint on deserialize. Native visual/accessibility acceptance remains unverified. |
| Waiting/live reasoning | `src/ui/reasoning.rs::WaitingView::start_turn` ← `main::start_stream`; `WaitingView::show` ← `main::show_list` while streaming before body arrives | WIRED behavior; stable per-turn clock, fixed 3/12-line selectable scroll, suffix-only marker stripping, pre-append bottom-follow, 8s timer, pulse and top fade. Accumulated prefix comparisons/galley work remain O(n); pulse repaints UI rather than native CA leaf animation. OS Reduce Motion is not automatically bridged. |
| Settled reasoning | `src/ui/reasoning.rs::show_disclosure` → `markdown::show_prose` ← assistant row in `main::show_list`, keyed by message ID | WIRED; initially collapsed local disclosure, formatted selectable Markdown at normal renderer font sizes, secondary-color text/math and leading rule. Native attributed-object/clipboard, animation and layout-cost equivalence not claimed. |
| Footer clipping | `src/ui/chat_layout.rs::bottom_panel/show_body` ← `main::show_card`; composer rect ← `main::show_composer_capsule` | WIRED; measured bottom stack replaces fixed 92pt reserve, includes Find/loading/composer/footer, bounds transcript and scrolls overflowing bottom contents without selection-drag capture. Native 604×344 single/three-line and 604×644 attachment/three-line/Find footer screenshots checked. Estimated native compact height still differs from original natural sizing. |
| Separate English/CJK fonts | `src/ui/fonts.rs::show_font_preferences/font_picker` → `font_catalog::catalog` / `preference_layout` ← Settings `appearance_section`; `apply_font_preferences → font_cmap::subset` ← app `new/update` | WIRED; full supported installed system/user/registered font catalog replaces curated list; searchable metadata/virtualized rows/refresh, file+TTC face persistence and legacy filename compatibility. Independent script cmap prevents multilingual English fonts stealing CJK; always retain fallback/code fonts. Native 337-face catalog/cream surface proof and retained TT Norms/Yu Gothic UI after relaunch recorded. Blank transcript was reproduced as RTL-label overflow and fixed with explicit wrap; see progress. Not arbitrary import, bitmap-only/oversize fonts, IVS/all-glyph/performance acceptance. |
| Bundled Chinese default / adjustable chat type | `fonts::bundled_chinese/build_definitions` ← `apply_font_preferences`; `typography::apply/scaled` ← `main::show_list/show_composer_capsule`, Markdown/math/reasoning renderers; `SettingsUi::chat_font_size_preferences → general_preferences` | WIRED / installed new requests. Official pinned OFL Swei Gothic TC Regular embedded; empty Chinese choice uses bundle, manual choices retained, cached CJK-only fallback independent of English/code. Chat size 12–24pt (default15), local style marker not global zoom, proportional explicit renderer/math sizes and real composer layout; independent preference save excludes API/hotkey drafts. Glyph/picker, renderer/IME/cursor and actual slider/persistence tests passed; native multi-DPI/type acceptance not implied. |
| User light-mode override | `theme::LIGHT_PANEL/LIGHT_CONTROL/LIGHT_RAISED` → `theme::apply/panel_fill/code_bg_for` and Settings/composer/popup/Markdown callers | WIRED; light neutral surfaces intentionally use cream/beige instead of original white/cool grays per the user's request. Dark mode, accent semantics, clear alpha and actual image/math texture pixels remain unchanged. Native settings and original code-response screenshot checked; not a macOS color-equivalence claim. |
| Icon-only chat navigation | `controls::header_actions_width/chat_navigation_inline → icons::button(Icon::NewChat/History)` ← `main::header_row` | WIRED user override; same 28×28 font-independent vector/hover/active rendering as Settings, accessible button labels and Ctrl+N/Y tooltips; same-row Pin/Settings/Close reservation. Latest same-row production screenshot checked. Action callbacks tested without a real profile; no native NewChat click that would reset the user's draft. |
| Official control assets | `icons::button → resvg white-mask texture cache` ← composer/header/message controls | WIRED user override; 14 official Free Solid SVGs from Font Awesome 6.7.2, pinned upstream/hash/license, no icon font or handdrawn control paths. 28pt targets, per-context 42-entry bounded scale cache; bespoke PopChat brand separate. Asset/cache/raster/UI tests and bounded native appearance proof, not full native hover/focus/DPI acceptance. |
| Icon functional hover help | `icons::button → hover_help` ← header/navigation/message/composer/table; `hover_help` ← find/history/attachment/reasoning icon controls | WIRED new request; 180ms movement-tolerant, context-bounded timer, disabled explanations, state-aware labels/shortcuts, popup outside page clipping and native viewport edge fitting. Actual hover text, jitter, disabled/click gating and corner clip tests passed; full native hover acceptance is separate from reachability. |
| Installed English default | `font_catalog::default_english_candidates/default_english_rank` → cached `fonts::default_english/resolve_default_english` → `build_definitions` ← startup/preferences apply | WIRED user override; empty English prefers metadata-validated normal regular TT Norms Pro then Arial, egui last resort if both missing/unreadable. Neither English font bundled/downloaded/copied; Latin-only cmap, installed manual choices/CJK/code preserved. Candidate-only font-byte reads and immutable process cache; restart after installed-default changes. Pure priority/fallback and actual installed-path/font-metric tests passed; not a redistribution of proprietary font files. |
| Repository release / opt-in update installation | `update::UpdateDownload::start/poll` ← Settings; `SettingsAction::InstallUpdate` → `main::update_confirmation_dialog/begin_install/poll_update_install` → `popchat_updater --request` | WIRED user override beyond original check/link behavior; own repo stable release and mandatory API SHA, bounded safe ZIP/PE staging, explicit draft-loss confirmation, READY-before-exit, verified PID wait/zero same-path app check, retained backups/rollback/manifest preservation, restart guard. Public package contract exercised on actual ZIP; transaction mocked in isolated directories. No real next-version/native upgrade acceptance claim or codesign guarantee. Older helper-less builds need one manual upgrade. |
| Provider loss / recovery protection | `ProviderStore::load_at/load_error/try_save` ← startup `load`; error guard ← `main::save_settings/migrate_legacy_settings/settings_actions/show_card`; directory label ← `SettingsUi::show` | WIRED; missing file distinct from unreadable/invalid schema, failed load preserves original and blocks subsequent saves/credential edits; bound storage path, stale UUID recovery. Explicit saved `settings_models` separate from live choice, consumed by Settings reopen. Offline roundtrip/corruption tests; cross-profile recovery is user-directed, not an implicit merge. |
| Per-user installation / explicit custom import | `scripts/install.ps1 → examples/import_custom_providers.rs` → verified installed exe / default-profile app startup | USER-AUTHORIZED platform deployment; only 2 custom providers + UUID key copies, no general/history/OAuth/search migration, source intact. Native WM_CLOSE/zero-instance handoff, hash-preserved default data, installed shortcuts and registration checked. Importer conflict/rollback/mock-credential tests; no claim cross-resource crash atomicity, no uninstall/live provider/IME matrix executed. |
| Current subscription model snapshot | `chatgpt_auth::{MODEL_CATALOG,DEFAULT_MODEL,supported_reasoning_efforts}` → `providers::chatgpt_catalog/apply_chatgpt_catalog` ← startup/auth events | WIRED per latest user request; 2026-11-02 official Codex docs list Sol 6.1 / Terra 6.1 / Astra 6 / Luna 6. Only known retired subscription selections migrate; newer/manual IDs and API/custom catalogs preserved. Offline snapshot, not account discovery; no claim all plans or the unofficial transport were live-tested. |
| Message actions / scroll edge / footer overrides | `controls::message_actions_inline` + `chat_layout::transcript_contents` ← `main::show_list`; footer removed from `main::show_card` | WIRED latest user requests; Copy/Check/Fork/Retry/Edit retain real callers/disabled conditions; inset protects left-edge strokes/glyph bearings without expanding clip. Bottom hints removed, not keyboard/paste/drop. Same production build visually checked; exact test boundaries and incomplete native action/DPI matrices remain in progress. |
| Message-side toolbar alignment | `controls::message_actions_row → message_actions_inline` ← role-aware `main::show_list` | WIRED latest user requests; user group shares right edge with bubble, assistant/error left; inner LTR group preserves order and hit targets. User extra 16pt/8pt removed, gap ≤6.5pt asserted across widths/themes/counts, user bubble padding unchanged. Assistant follow-up is separately recorded below. Callback tests passed; installed update includes source, no current-chat native screenshot claim. |
| Assistant compact reply end | `panel::MathTextures::show_assistant_message → markdown::show_prose_compact_end / terminal segment spacing flags` ← `main::show_list`; assistant trailing 22pt/8pt removed | WIRED follow-up request; only terminal flow/segment whitespace and transparent reply-frame bottom inset removed. Internal paragraph/heading/list text coordinates preserved; quote/table/pasteable/math keep inter-segment gaps and code/card/cell internal padding. Real prose/list/quote/table renderer-to-toolbar gap tests passed; user right alignment/Error spacing unchanged. No native current-conversation screenshot claim. |
| Table layout / whole-table Copy | `markdown::table_column_widths/table_cell_job/show_table → show_text_label`; `table_copy_button → table_as_markdown → ctx.copy_text` ← `MathTextures::show_assistant_message` | WIRED latest user requests; shared responsive columns, measured maximum row heights, top-aligned bounded cells, wide-table scrolling; cell selection/link behavior retained. Viewport-right official Copy/Check outside scroller copies full Markdown only, content-bound independent 1.5s feedback. Actual PDF/wrap/row geometry and clipboard-command/feedback fixtures passed; no native clipboard overwrite performed for tests. |
| No startup command window | `main.rs` non-test Windows GUI subsystem attribute → installed PE Subsystem 2; existing `codex::AppServerSession::spawn` CREATE_NO_WINDOW | PLATFORM_REPLACED; desktop/Start menu invoke GUI exe directly, installer rejects Console app builds. PE/installed path/boot/absence of app child consoles checked; not a real Codex launch matrix. Test harness/importer retain intended CLI output. |
| Configurable global hotkey | Settings `hotkey_section` draft/Apply → `SettingsAction::ChangeHotkey` → `main::settings_actions → HotkeyManager::set_binding` → successful registration only → `SettingsUi::commit_hotkey_binding`; initialization ← app `new` | PLATFORM_REPLACED / WIRED; default Alt+Down, Alt+Space rejected. New registration precedes old unregister; failure retains old active binding and does not commit draft. Native Ctrl+Alt+P hide/show/restart, old-key unregister probe and controlled Ctrl+Alt+O conflict passed; defaults restored. Persistence errors remain separately reported. |
| Recorded hotkey / chat shortcut reference | `hotkey_recorder::intercept/show` ← first action of `main::update` / Settings `hotkey_section`; shared `HotkeyManager::suppression_flag` ← recorder + watcher + visibility-driver; `shortcut_list::show` ← hotkey section | WIRED user requests, source validation/deployment tracked in progress. Physical-key capture with modifier preview, invalid/repeat rejection, Esc/Tab/focus/pane/hide cancellation; pending draft only, explicit Apply, no capture-triggered app action or queued global toggle. Grouped read-only keycap cards, responsive 320/520pt renderer/palette proof. Offline recorder/registry tests; no new native hotkey registration or live recorder acceptance claim. |
| Backdrop opacity | Settings `appearance_section → window_prefs::show_preferences`; `surface::card_frame_with_opacity(window_prefs::opacity(...))` ← `main::show_card` | WIRED; 35–100% multiplies backdrop fill/border/shadow only, never viewport/UI/text opacity; native clear stays zero. Native 55% checked, restored 100%; independent autosave, not native blur/glass. |
| Window dragging | `surface::drag_window` ← brand in `main::header_row` / top band in `main::show_card`; existing header gap also emits `StartDrag` | PLATFORM_REPLACED / WIRED; header-only hit regions exclude controls, transcript selection and composer, suppressed during resize. Both brand/top band moved the actual native window; controls remained usable. |
| Default show position | `window_prefs::placement_rect` ← app `new/update_window_geometry` | WIRED; Remember / cursor-screen center / near cursor / top-right / bottom-right / fixed coordinates; applies startup/new SHOW only. Native Fixed hide/show returned to saved coordinates, dragged OLE drop did not snap back, Remember preserved relaunch position. Remaining four-mode/mixed-DPI native matrix pending; monitor-only change clamps current position. |
| Native file paste/drop feedback | `ui/paste.rs` root-window observer → `main::handle_attachment_input → clipboard::file_paths/load_attachments`; dropped paths/bytes → canonical attachment parser; `FileDropFeedback::show` ← app `update`, `files_added` ← actual append result | WIRED for CF_HDROP/path/in-memory bytes; paint-only hover/loading/success feedback and Ctrl+O picker. Earlier CF_HDROP native proof remains version-bounded; latest real OLE Copy confirms badge above composer and no Fixed-position reset via `activate_visible`. Virtual-file promises remain missing. |

Targeted test references for the refreshed paths: `settings::tests::general_preference_autosave_never_commits_provider_drafts_or_secrets`, `command_autosave_changes_only_its_own_preference_and_preserves_unknown_fields`; `layout::tests::command_ack_and_mode_changes_do_not_become_user_preferences`, `dpi_and_monitor_fit_never_overwrite_saved_geometry`, `an_explicit_edge_resize_can_override_an_in_flight_size_request`; composer streaming/shared-state and Markdown real-drag tests are named in their inventory sections. These complement the concrete callers above; their existence is not a substitute for native/live acceptance.


## Application and UI definitions

### AccentPicker.swift — 4 definitions

Rust: `src/ui/accent.rs`, `src/ui/settings.rs`, `src/ui/theme.rs`. Actual entry: `SettingsUi::show → appearance_section → AccentPicker::show`; changed general preferences independently autosave in `show`, then app theme/rendering consumes the current settings. Opening an unset picker only seeds local state; it never publishes a custom accent.

| Swift line / definition | Rust counterpart ← caller | Status / boundary |
|---|---|---|
| `176 labeled` | labeled rows in `accent::AccentPicker::popover_body` ← `show` ← `SettingsUi::appearance_section` | PLATFORM_REPLACED; egui Hue/Saturation/Brightness gradient sliders and Hex field, not native SwiftUI controls. |
| `188 seed(from:)` | `AccentPicker::seed/sync_custom` ← swatch open / popup external-custom synchronization | WIRED; seed is local-only, preserves hue for gray; slider writes do not reseed themselves, preserving saturation at black. Explicit external/Hex seeding can reset black saturation, as source. |
| `198 apply` | `AccentPicker::apply` ← gradient-slider edits / `commit_hex_draft` | WIRED; live HSB → quantized uppercase hex, deduplicated accent/custom writes; same-color slider edits keep local state and can select/create custom. Settings autosave only on changed persisted values. |
| `209 commitHexDraft` | `AccentPicker::commit_hex_draft` ← Hex Enter; `reset` ← popup Reset | WIRED; optional `#`, six ASCII hex digits, whitespace-only trim excluding newlines; valid uppercase/reseed/apply, invalid draft restores custom/current without guessing; Reset clears custom and restores Blue without seed-publishing. |

### AppDelegate.swift — 13 definitions

Rust: `src/main.rs`, `src/startup.rs`, `src/instance.rs`, `src/ui/{tray,hotkey,visibility}.rs`.

| Swift line / definition | Rust counterpart ← caller | Status / boundary |
|---|---|---|
| `16 hidePanel` | `main::hide` is analogous; original method only used by first-run smoke harness | TEST_HARNESS; do not count this wrapper as another user feature. |
| `18 applicationDidFinishLaunching` | Rust free `main`, `PopChatApp::new/update`, `spawn_visibility_driver` | PLATFORM_REPLACED; real boot/tray/hotkey/update/launch/instance callers; no complete macOS lifecycle equivalence implied. |
| `89 presentLaunchUI(loginItem:)` | `startup::record_launch/should_present_launch` ← free `main`, app `new`; delayed hint ← `update` | WIRED launch policy; first/non-login/repeat/Settings branches present; hint uses platform balloon, native sequencing not exercised here. |
| `111 applicationShouldHandleReopen` | `InstanceGuard::acquire/take_request` ← free `main` / hidden-capable `spawn_visibility_driver` → visibility `show/open_settings` | PLATFORM_REPLACED; pre-eframe guard retained by Arc, SHOW never toggles, `--settings` forwarding real. |
| `117 statusItemClicked` | `tray::watch_loop` ← `PopchatTray::new` → visibility driver | PLATFORM_REPLACED; Windows left click/menu events. |
| `129 showMenu` | `PopchatTray::new` constructs native menu ← app `new` | PARTIAL; Toggle/Settings/Quit wired; no conditional update-available menu item. |
| `164 togglePanel` | `VisibilityController::toggle` ← visibility driver hotkey/tray | WIRED; off-screen hiding is a deliberate native-window strategy. |
| `170 openReleasePage` | Settings release hyperlink ← `SettingsUi::hotkey_section` | PARTIAL; release page handoff exists, original tray notification/action absent. |
| `184 applicationDidResignActive` | `main::poll_events` → `VisibilityController::ui_frame` | PARTIAL; auto-hide deliberately disabled while Settings/editor/picker open; original app-resign closes Settings too. |
| `194 installMainMenu` | egui TextEdit shortcuts + app `poll_events` | PLATFORM_REPLACED; no invisible AppKit edit menu; composer shared-ID undo/selection state present, native key/IME matrix not fully verified. |
| `230 closeKeyWindow` | Ctrl+W handling ← `main::poll_events` | PARTIAL; one embedded Settings surface, not a separate closable key window. |
| `239 openSettings` | `settings_open` ← tray driver / header / Ctrl+, → `show_card` | PARTIAL; embedded view rather than retained independent floating window. |
| `269 DismissOnEscWindow.sendEvent` | IME guard/Escape priority ← `main::poll_events` | PLATFORM_REPLACED; native marked text replaced by egui IME state. |

### ChatView.swift — 25 definitions

Rust: `src/main.rs`, `src/ui/{composer,panel,markdown,reveal,reasoning}.rs`, `src/chat/store.rs`. App `start_stream → WaitingView::start_turn` resets the retained wait state; `show_list → WaitingView::show` renders it only while streaming before body arrival. Settled message reasoning uses `show_list → reasoning::show_disclosure → markdown::show_prose`.

| Swift line / definition | Rust counterpart ← caller | Status / boundary |
|---|---|---|
| `107 revealRestoredTranscript` | `ChatStore::restore_tick/restored_into_empty_panel` exist; no app consumer found | CORE_ONLY; no group restore opacity/offset/delayed reveal. |
| `454 editLastPrompt` | `main::message_action(Edit)` → `take_last_user_message_for_editing`, `Composer::set_draft` ← row button | WIRED; attachments restored with text. |
| `479 toggleFind` | `main::toggle_find` ← `poll_events` Ctrl+F | WIRED; per explicit user request, persistent Find button removed; Ctrl+F/G and Find bar remain. |
| `490 closeFind` | reset `FindState` + `Composer::request_focus` ← `show_find/poll_events` | WIRED. |
| `497 stepMatch` | `main::step_find` ← find buttons / Ctrl+G / field keys | WIRED; wraps. |
| `506 scrollToMatch` | `paint_find_matches` + `scroll_to_rect` ← `main::show_list` | WIRED; row fallback when precise glyph rectangle absent. |
| `513 findBar` | `main::show_find` ← `show_card` | WIRED. |
| `563 matchLabel` | count label inline ← `main::show_find` | PARTIAL; always `0 / 0`, unlike blank/no-matches labels. |
| `569 findStepButton` | ↑/↓ buttons inline ← `main::show_find` | PLATFORM_REPLACED; egui controls. |
| `597 rowTransition(for:)` | No equivalent per-role row insertion transition | MISSING; user slide/opacity and Reduce Motion handling absent. |
| `607 trackingBottomPin` | egui `ScrollArea::stick_to_bottom` ← `main::show_list` | PARTIAL; explicit 40pt/4pt threshold contract not reproduced. |
| `626 newMessagesPill` | No new-message jump pill in `show_list` | MISSING. |
| `666 ContentHeightKey.reduce` | draft row/chip height formula ← `main::update_window_geometry`; measured `chat_layout::bottom_panel` ← `show_card` | PARTIAL; bottom stack is now measured/bounded, but native compact-window growth still uses estimates rather than natural total content height. |
| `673 TranscriptWidthKey.reduce` | `ui.available_width` ← `main::show_list` | PLATFORM_REPLACED; egui layout, no preference key. |
| `680 HeaderWidthKey.reduce` | available-width cap ← `main::header_row` | PLATFORM_REPLACED; cap differs from original 46%. |
| `742 MessageRow.==` | Immediate-mode `main::show_list`; no row equivalence gate | MISSING; original unchanged-row performance contract not demonstrated. |
| `915 rerunRow` | row action group ← `main::show_list` | WIRED; calls `message_action`. |
| `1101 ThinkingScroller.height(lines:)` | `reasoning::WaitingView::show_scroller`, `COLLAPSED_LINES/EXPANDED_LINES` ← `WaitingView::show` ← `main::show_list` | WIRED; ceil egui line height at 11.5pt × fixed 3/12 lines + 4pt, independent of accumulated text; native font metrics differ. |
| `1109 ThinkingScroller.makeCoordinator` | retained `WaitingView::text` (`LiveText`) ← app `new`; `start_turn` ← `main::start_stream` | PLATFORM_REPLACED; persistent raw/stripped state and per-turn scroll ID; not NSTextStorage incremental-layout performance. |
| `1111 ThinkingScroller.makeNSView` | `WaitingView::show_scroller` selectable label inside independent egui ScrollArea ← `show` ← `main::show_list` | PLATFORM_REPLACED; fixed-height wheel/bar scrolling, selection owns drags, Model thinking accessibility label; native drop/accessibility behavior not validated. |
| `1134 ThinkingScroller.updateNSView` | `LiveText::update` + `WaitingView::show_scroller` ← `WaitingView::show` | PARTIAL; suffix-only stripping/append and replacement reset, follow decided from pre-append dimensions then unanimated bottom scroll. Equality/prefix comparison and full accumulated galley cloning/hash/layout remain O(n), not native incremental text-storage parity. |
| `1153 ThinkingScroller.isAtBottom` | `reasoning::is_at_bottom` ← `WaitingView::show_scroller` | WIRED; first text follows; old content/viewport and current offset use source 8pt tolerance, scrolled-up reader is not yanked on append. |
| `1170 ThinkingScroller.stripEmphasis` | `reasoning::strip_emphasis` and identical suffix filter in `LiveText::update` ← `WaitingView::show` | WIRED via active filter; drops `*`, backticks and `#` only on appended suffix, preserving underscores; standalone helper is not the production call. |
| `1174 ThinkingScroller.rendered` | `LiveText::stripped` → 11.5pt secondary-color selectable galley ← `WaitingView::show_scroller` | PLATFORM_REPLACED; stripped text/Copy and top-22% vertex-alpha fade wired; accumulated layout is not attributed suffix append. |
| `1181 ThinkingScroller.sizeThatFits` | fixed ScrollArea min/max height + available width ← `WaitingView::show_scroller` | PLATFORM_REPLACED; fixed 3/12-line viewport prevents token-driven transcript height growth; native metric/performance acceptance separate. |

### ComposerView.swift — 24 definitions

Rust: `src/main.rs`, `src/ui/composer.rs`, active attachment loader `src/chat/attach.rs`. `ui/attachments.rs::AttachmentDraft` is **not** the active app draft; `Composer` owns active attachments.

| Swift line / definition | Rust counterpart ← caller | Status / boundary |
|---|---|---|
| `16 ComposerModel.handleFiles` | `main::load_attachments/load_attachment_paths` ← drop / picker | WIRED; background generation guards. |
| `37 ComposerModel.handlePasteboard` | root-window paste observer → `main::handle_attachment_input → clipboard::file_paths/load_attachments` → clipboard image/file loader | WIRED for CF_HDROP file lists and image paste; parent native file-only Ctrl+V proof recorded separately. Panel-wide AppKit interception/format precedence is not equivalent. |
| `53 ComposerModel.handleImage` | `attach::load_image_bytes/encode_image` ← clipboard loader | WIRED; JPEG and 2048-long-edge path. |
| `64 ComposerModel.remove` | `Composer::remove_attachment` ← `show_attachments` | WIRED. |
| `69 ComposerModel.clear` | `Composer::clear` ← `main::reset_draft` | WIRED. |
| `74 updateSizeWarning` | `main::draft_attachment_warning` ← `attachment_warning/show_composer_capsule` | PARTIAL; capability notice exists; original aggregate payload-size warning not identified. |
| `104 CompositionAwareTextView.setMarkedText` | `composer::ime_blocks_shortcuts` ← `Composer::show/show_expanded`, app `poll_events` | PLATFORM_REPLACED; egui IME lifecycle, not shared native editor. |
| `109 CompositionAwareTextView.unmarkText` | same IME state reducer ← composer/app input handling | PLATFORM_REPLACED; commit-frame guard present, native IME behavior not live-tested. |
| `133 ComposerTextView.makeCoordinator` | `Composer::new` ← app `new` | PLATFORM_REPLACED; persistent egui editor state. |
| `135 ComposerTextView.makeNSView` | `Composer::show/show_expanded` TextEdit ← `show_composer_capsule` | PLATFORM_REPLACED; one shared TextEdit ID preserves cursor/selection/undo across modes; native IME matrix unverified. |
| `172 ComposerTextView.updateNSView` | `Composer::set_draft/set_text/show` ← app editing / render | PLATFORM_REPLACED; inline/expanded use `popchat_composer_text` ID, not two independent editor states. |
| `221 Coordinator.init` | `Composer::new` ← app `new` | PLATFORM_REPLACED. |
| `231 observeWidth(of:)` | TextEdit width derived in `Composer::show` | PLATFORM_REPLACED; no NSScrollView bounds observer. |
| `242 textDidChange` | TextEdit response/state ← `Composer::show` | PLATFORM_REPLACED. |
| `250 syncFromTextView` | TextEdit directly mutates draft ← `Composer::show` | PLATFORM_REPLACED. |
| `258 remeasure` | `Composer::inline_rows` (1–8), egui layout + `chat_layout::bottom_panel` ← `main::update_window_geometry/show_card` | PARTIAL; footer reservation now measures rendered bottom contents, native compact-window height still estimates explicit newlines instead of native used-text-rect measurement. |
| `274 textView(doCommandBy:)` | composer key/IME handling ← `Composer::show/show_expanded` | PARTIAL; Enter/Shift+Enter/recall/slash/editor reachable; native command/selection/undo matrix unverified. |
| `565 sendOrStopButton` | `Composer::show/show_expanded` Send/Stop → `ComposerAction` ← `main::show_composer_capsule` | WIRED; editor remains editable during stream, Stop replaces Send without consuming next draft. |
| `602 recallOlder` | `composer::recall_older_index` + entry snapshot ← `Composer::show` | WIRED; separate store `PromptHistory::recall_older` is not called by composer. |
| `619 recallNewer` | `composer::recall_newer_index` + snapshot ← `Composer::show` | WIRED; source also starts only on empty draft and returns empty after newest (`ComposerView.swift:597–628`); no arbitrary-draft recall requirement. |
| `667 complete(with:)` | autocomplete selection in `Composer::show` ← slash popup keys/click | WIRED; execution still expanded by active store. |
| `755 attachViaPicker` | `main::pick_attachments` ← `ComposerAction::PickAttachments` | WIRED. |
| `772 submitFromButton` | `Composer::take_send` ← composer button → app `send_current` | WIRED; attachments-only send allowed. |
| `780 submit` | `Composer::take_send` ← composer Enter → app `send_current` | WIRED; sends through store validation. |

### Find.swift — 8 definitions

Rust: free helpers in `src/main.rs`.

| Swift line / definition | Rust counterpart ← caller | Status / boundary |
|---|---|---|
| `19 FindHighlight.ranges(in:query:)` | `find_ranges/folded_with_ranges` ← find hits, painting, history highlight | PARTIAL; Unicode/diacritic folding implemented; exact Foundation folding across all scripts unverified. |
| `37 FindHighlight.count` | `find_ranges(...).len()` ← `main::find_hits` | WIRED. |
| `41 FindHighlight.fill(active:)` | active/passive color inline ← `paint_find_matches/highlight_job` | PLATFORM_REPLACED. |
| `54 FindHighlight.paint(NSAttributedString)` | `paint_find_matches` ← `main::show_list` | PLATFORM_REPLACED; glyph-shape painting, not attributed background mutation. |
| `65 FindHighlight.paint(String)` | `highlight_job` / glyph painting ← history / transcript | PLATFORM_REPLACED. |
| `89 FindCursor.init?` | occurrence indexing in `main::find_hits/show_list` | PARTIAL; no direct cursor type; segment/display ordering must stay aligned. |
| `95 FindCursor.next(text)` | rendered-shape occurrence scan ← `paint_find_matches` | PARTIAL; not a shared source-to-render cursor. |
| `104 FindCursor.next(all:)` | segment strings and flattened hit enumeration ← `searchable_message_strings/find_hits` | PARTIAL; glyph grouping across adjacent widgets remains unverified. |

### FirstRunHint.swift — 2 definitions

Rust: `src/ui/tray.rs`, app `src/main.rs`.

| Swift line / definition | Rust counterpart ← caller | Status / boundary |
|---|---|---|
| `28 show(from:)` | `PopchatTray::show_first_run_hint` ← `main::update` delayed 350ms | PLATFORM_REPLACED; notification uses exact own tray HWND/icon ID and actual shortcut/fallback; Windows balloon, not macOS popover. |
| `45 dismiss` | `PopchatTray::dismiss_first_run_hint` ← `main::hide/update`, driver dismissal flag | PLATFORM_REPLACED; same own-icon native notification cleared; shell lifetime not exercised here. |

### FloatingPanel.swift — 9 definitions

Rust: `src/main.rs`, `src/ui/visibility.rs` and egui/winit.

| Swift line / definition | Rust counterpart ← caller | Status / boundary |
|---|---|---|
| `11 FloatingPanel.init(contentRect:)` | `ViewportBuilder` ← free `main`; `surface::clear_color/canvas_frame/card_frame_with_opacity` ← `eframe::App::clear_color/main::show_card` | PARTIAL; topmost/taskbarless with fully transparent native clear (not eframe's default dark rectangle) and one bounded card shadow; background-only opacity now configurable, not AppKit nonactivating panel or native glass equivalence. |
| `46 cancelOperation` | Escape → `main::hide` ← `poll_events` | WIRED; editor/find/history priority is explicit. |
| `53 performKeyEquivalent(with:)` | root-HWND observer in `ui/paste.rs` + egui paste → `main::handle_attachment_input` ← `update` | PARTIAL; Windows Ctrl+V/Shift+Insert/WM_PASTE and CF_HDROP/image routing wired without global hook or swallowing ordinary text; not identical panel-wide AppKit interception. |
| `64 pasteboardHasAttachable` | `clipboard::file_paths` + clipboard image loader ← `main::handle_attachment_input/load_attachments` | PLATFORM_REPLACED; bounded native CF_HDROP probe, not guessed path text; file-only Ctrl+V native proof exists, full raw-image/format matrix unverified. |
| `79 WindowDragView.mouseDown` | `ViewportCommand::StartDrag` ← `surface::drag_window` ← brand/top band in `main::header_row/show_card`, plus header gap | PLATFORM_REPLACED; controls/transcript/composer excluded, resize guard; actual native brand/top-band drags verified, not a full DPI/focus matrix. |
| `85 WindowDragView.scrollWheel` | no equivalent forwarding from header drag strip into transcript | MISSING. |
| `93 findScrollView` | no recursive scroll-view lookup/forwarder | MISSING. |
| `104 WindowDragStrip.makeNSView` | `surface::drag_window` ← header brand/top band; header-gap drag response ← `main::header_row` | PLATFORM_REPLACED; no full-card drag capture. |
| `105 WindowDragStrip.updateNSView` | immediate header render ← `main::header_row` | PLATFORM_REPLACED; original method is a no-op. |

### HistoryPopover.swift — 12 definitions

Rust: `src/main.rs`, `src/ui/history.rs`; active `src/chat/store.rs::ConversationStore`.

| Swift line / definition | Rust counterpart ← caller | Status / boundary |
|---|---|---|
| `155 itemView` | day/title/snippet rows ← `main::show_history` | WIRED; embedded history surface. |
| `178 cascade` | no per-row cascade animation | MISSING. |
| `186 close(afterFlash:then:)` | `history_open = false` / `load_chat` ← history action | PARTIAL; no flash/out-animation-before-load ordering. |
| `203 open(meta)` | `main::load_chat` ← `show_history` click/Enter | WIRED; async resolved load with generation check. |
| `211 moveSelection` | Up/Down clamp + scroll ← `main::show_history` | WIRED. |
| `219 openSelected` | selected-row Enter ← `main::show_history` → `load_chat` | WIRED. |
| `225 row(result)` | `history::row` ← `main::show_history`; left title/time/Fork/current/delete/snippet | WIRED; subtle selection tint/readable foreground, whole-row open and independent 26×26 delete target; native appearance/motion still differs. |
| `319 loadFullText` | `main::refresh_history_index` → `ConversationStore::full_text_index` ← `open_history/delete_chat` | WIRED; background index, no reasoning. |
| `330 recompute` | free `history_results` ← `main::show_history` | WIRED; title/body search and excerpt. |
| `388 rowHeight` | explicit 42/62pt row geometry in `history::row` ← `main::show_history` | PLATFORM_REPLACED; accommodates padded delete target, not the original 30/46pt metrics. |
| `407 groupTitle(for:calendar:)` | local `%Y-%m-%d` heading ← `main::show_history` | PARTIAL; no Today/Yesterday/relative grouping labels. |
| `417 timestamp` | local `%H:%M` ← `main::show_history` | PARTIAL; date-dependent original formatting not fully matched. |

### MarkdownText.swift — 26 definitions

Rust: `src/ui/{panel,markdown,math,reveal,reasoning}.rs`; free search helpers in `src/main.rs`. Production renderer is `main::show_list → MathTextures::show_assistant_message → panel::split_math_segments`, then prose/code/table/pasteable/math renderers. There is no current `markdown::segments` symbol; `PORT_PROGRESS.md` now names the active segmentation route.

Selection correction: original `SelectableText` is one text-view island. Paragraphs, headings, formatting and links **within prose share selection**; fenced code, quotes, each table cell and pasteable bodies are intentionally separate islands. Current `markdown::prose_selection_island/selection_island` reproduces that boundary, including normalized plain Copy separators. Source references `actual_selection_copies_rich_prose_and_inline_links_without_inserted_spaces`, `actual_selection_spans_prose_paragraphs_and_headings`, `actual_selection_stops_at_swift_segment_and_table_cell_boundaries`, and `actual_selection_copies_code_and_pasteable_bodies_not_their_decoration` drive real renderer pointer-drag/Copy commands, not just parser output. Do not list intentional cross-island separation as a defect. Inline bitmap math remains non-selectable as an object/rich clipboard attachment (`actual_selection_crosses_inline_bitmap_but_copies_only_neighboring_text`). This audit read these tests, not reran them.

| Swift line / definition | Rust counterpart ← caller | Status / boundary |
|---|---|---|
| `30 MarkdownRenderer.segments` | `panel::split_math_segments` ← `MathTextures::show_assistant_message` | WIRED; golden references include malformed/unclosed blocks. |
| `34 flushProse` | segmentation prose-flush closure ← `split_math_segments` | WIRED. |
| `140 isTableSeparator` | `panel::is_table_separator` ← segmentation | WIRED. |
| `146 tableCells` | `panel::table_cells` ← segmentation | WIRED. |
| `163 searchableStrings(segment)` | segment match in free `searchable_message_strings` ← `main::find_hits` | WIRED; displayed strings, not raw tags. |
| `176 tableCell` | `markdown::table_cell_inlines/table_cell_job → show_text_label` ← measured cells in `show_table` | WIRED; inline-only text/links, fixed-width selectable galley; no math substitution like source. |
| `190 searchableStrings(for:message:)` | free `searchable_message_strings` ← `find_hits` / `displayed_message_text` | WIRED; reasoning excluded. |
| `214 attributedProse` | `markdown::parse_prose/show_prose` ← assistant renderer | PARTIAL; caret and complete attributed-string semantics absent. |
| `247 attributedReasoning` | `reasoning::show_disclosure → markdown::show_prose` ← `main::show_list` assistant row | WIRED formatted settled Markdown, local initially collapsed state, normal renderer font metrics with secondary color only; native attributed attachment/clipboard/cache equivalence remains partial. |
| `262 cached` | egui layout internals / separate math cache; no matching bounded attributed-prose cache | PARTIAL; exact parse/measurement invalidation behavior not ported. |
| `274 buildProse` | `markdown::parse_prose` ← `show_prose` | PARTIAL; pulldown-cmark rendering rather than original attributed layout. |
| `401 plain` | egui text labels / `markdown::selection_island` ← message/code/card renderers | PLATFORM_REPLACED; real drag/Copy within source text islands; no rich-clipboard/bitmap attachment equivalence. |
| `456 highlightedCode` | `markdown::highlight_line` ← `show_code_block` | PARTIAL; handwritten lexer/theme/cache differ from Splash. Original calls Splash's Swift highlighter without passing the fence language; a missing multi-language Splash catalog is not a source requirement. Exact Swift token/color/spacing equivalence remains unverified. |
| `479 mathImage` | `math::render_latex/render_uncached/math_svg` ← `MathTextures::texture_for` | PARTIAL; real latex-rust/OpenType MATH layout, not text screenshot; full SwiftMath compatibility unverified. |
| `497 extractInlineMath` | `markdown::protect_inline_math`, `panel::split_inline_math` ← prose parser/renderer | WIRED; regex-order golden references. |
| `525 substituteInlineMath` | `markdown::restore_inline_math/show_inline_math` ← prose renderer | PLATFORM_REPLACED; textures rather than attributed attachments/baselines. |
| `582 SelectableText.makeCoordinator` | widget state / egui text layout ← renderer | PLATFORM_REPLACED; no identical measurement/paint cache coordinator. |
| `591 SelectableText.makeNSView` | `markdown::selection_island/prose_selection_island/show_text_label` ← active renderer | PLATFORM_REPLACED; real selectable text across prose formatting/paragraphs/links, intentionally separate source block/cell islands; bitmap/rich clipboard gap remains. |
| `603 SelectableText.updateNSView` | immediate selectable renderer ← `main::show_list` | PARTIAL; source selection-island behavior wired, but native attributed-object/measurement cache contracts not identical. |
| `611 revealActiveMatch` | glyph match rectangle → `scroll_to_rect` ← `main::show_list` | WIRED; fine positioning when matching shapes identified. |
| `632 SelectableText.reveal(range:)` | free `paint_find_matches` ← `main::show_list` | PLATFORM_REPLACED; egui glyph coordinates, not AppKit container traversal. |
| `669 SelectableText.sizeThatFits` | egui text layout ← assistant renderers | PARTIAL; no original width/string measurement-cache contract. |
| `763 AssistantMessageView.fade` | `ui::reveal::paint_reveal` ← `main::show_list` after assistant body, before find | WIRED; final prose/code source budget and grapheme bands projected over actual glyph shapes; other source segments stay solid. |
| `780 segmentFinds` | free `searchable_message_strings` + `paint_find_matches` ← `main::show_list` | PARTIAL; separate count/paint passes rather than shared cursor. |
| `906 TableBlockView.cellView` | `markdown::show_table` measured row/cell rectangles → `show_text_label` ← assistant renderer | WIRED; maximum row height and top-aligned bounded cells replace auto Grid; independent selection/link behavior retained. |
| `914 TableBlockView.cellFind` | flattened occurrence painter ← `main::show_list` | PARTIAL; no explicit per-cell cursor; ragged/empty-cell highlight alignment unverified. |

### PanelController.swift — 17 definitions

Rust: `src/ui/{panel,visibility,layout,window_prefs,chat_layout}.rs`, `src/main.rs`. Integration callers are present: startup/new SHOW use `window_prefs::placement_rect/restore_position`, monitor-only changes validate current placement; `ViewportSizing::observe/requested` distinguishes user resize from programmatic/mode/DPI changes; user `constrain_user_resize/record_user_resize`, `height_change_rect` in `main::update_window_geometry`; rim `resize_edges` in `show_card`; `.with_resizable(true)` in free `main`; 180ms debounce plus hide/exit persistence. Measured bottom-stack layout prevents the old fixed-reserve clipping but does not replace natural compact-window sizing. Latest native placement/drag/mixed-DPI acceptance is separate from wiring.

| Swift line / definition | Rust counterpart ← caller | Status / boundary |
|---|---|---|
| `59 init(providerStore:shortcutStore:)` | `PopChatApp::new` + free `main` viewport builder | PLATFORM_REPLACED; real placement/resize/lifecycle dependencies held; not nonactivating AppKit equivalence. |
| `132 dismiss` | `main::hide` / visibility `dismiss` ← Esc/Ctrl+W/close/driver | WIRED; explicit user-dismiss path. |
| `137 toggle` | visibility `toggle` ← `spawn_visibility_driver` | WIRED. |
| `161 prewarm` | `PopChatApp::new` creates composer/store/renderer | PLATFORM_REPLACED; eager egui construction, not hidden AppKit hierarchy prewarm. |
| `172 show` | visibility `show/ui_frame` ← driver/reopen | PARTIAL; delayed native activation + best-effort focus, not nonactivating NSPanel. |
| `199 hide` | visibility `dismiss/hide` ← app/driver/blur | PLATFORM_REPLACED; off-screen move and best-effort previous-window restore. |
| `230 scaledTransform` | no show/hide scale/rise layer transform | MISSING. |
| `239 transformAnimation` | no CoreAnimation-equivalent show/hide transform/opacity transition | MISSING. |
| `272 position` | `window_prefs::placement_rect` + `PanelGeometry::placement` + visibility `new_at/restore_position` ← app `new/update_window_geometry` | WIRED; six configured modes apply startup/new SHOW only; monitor-only changes clamp current dragged position. Native six-mode/mixed-DPI matrix unverified. |
| `312 shrinkToFit` | `PanelGeometry::placement/height_change_rect` ← app geometry update | WIRED; work-area size/bottom clamp actually consumed; no live monitor-unplug/DPI acceptance claim. |
| `327 emptinessChanged` | `main::update_window_geometry` mode/height selection ← app `update` | PARTIAL; compact/expanded transitions wired, no original animated growth/restore sequencing. |
| `349 compactHeightChanged` | draft/chip row estimate ← `main::update_window_geometry` | PARTIAL; no natural-height report including wrapped notices. |
| `358 setContentHeight` | `height_change_rect` + `InnerSize` ← `main::update_window_geometry` | PARTIAL; top-anchored/work-area path wired, immediate commands rather than original animation. |
| `386 windowWillResize` | `constrain_user_resize/record_user_resize` + `ViewportSizing` ← `update_window_geometry`; `resize_edges` ← `show_card` | PLATFORM_REPLACED; true native resize, compact vertical lock, shadow padding and min/max limits wired; native user-drag matrix not run here. |
| `403 windowDidResignKey` | visibility `ui_frame` ← `main::poll_events` | PARTIAL; grace/continuous blur/pin guards wired, Settings exemption differs. |
| `423 windowDidMove` | visibility `remember_position` ← `ui_frame`; `geometry.remember_top_left` ← app `save_panel_geometry` | WIRED; shown position persisted on hide/exit, not hidden sentinel; native move/monitor sequence unverified here. |
| `429 windowDidEndLiveResize` | `ViewportSizing::observe/requested`, `record_user_resize` + 180ms debounce/`save_panel_geometry` ← `main::update_window_geometry` | PLATFORM_REPLACED; actual resize handshake/persistence caller, mode/DPI/programmatic resize must not overwrite user preference. |

### PanelDrop.swift — 12 definitions

Rust: `src/main.rs::handle_attachment_input/load_attachments`, `src/ui/file_drop.rs`, winit/egui drop events. Parent's real visible OLE source achieved Copy, hover animation and an attachment chip on the earlier build; hidden-source None was a failed attempt, not a pass. Neither proves virtual-file promises or the latest badge placement.

| Swift line / definition | Rust counterpart ← caller | Status / boundary |
|---|---|---|
| `42 PanelDropView.init(frame:)` | winit drop target initialization ← eframe free `main`; `FileDropFeedback` retained by app | PLATFORM_REPLACED; path and in-memory byte events consumed; virtual-file promises not implemented. |
| `48 PanelDropView.init?(coder:)` | no NSCoder construction needed (original fatalError) | PLATFORM_REPLACED. |
| `50 draggingEntered` | hovered-file events → `FileDropFeedback::show` ← app `update` | WIRED; paint-only fade/scale/glow/bounce, no click/wheel/selection capture; earlier native OLE hover proof exists, full acceptance matrix not implied. |
| `56 draggingExited` | empty hover state → `FileDropFeedback::show` ← app `update` | WIRED; feedback fades out; not an AppKit delegate. |
| `57 draggingEnded` | hover/loading/result state → `FileDropFeedback::show/files_added/failed` ← app update/attachment completion | WIRED; success count only after actual append, zero successes never show success. New composer-rect badge anchor is not yet native-verified. |
| `59 performDragOperation` | dropped paths/bytes → `main::handle_attachment_input/load_attachments` → canonical parser | WIRED; earlier real visible-source OLE Copy/chip proof; not virtual-file promises. |
| `64 accepts(pasteboard)` | dropped path/byte intake ← `main::handle_attachment_input` | PARTIAL; supports delivered egui paths/bytes, not Windows FileDescriptor/file-promise acceptance matrix. |
| `70 handleDrop(from:)` | paths/bytes → `main::handle_attachment_input/load_attachments` → `attach::load_bytes`/path loader | PARTIAL; canonical parsing and mixed-result feedback wired; promise/URL-first/raw OS image conversion matrix still absent or unverified. |
| `91 receive(receivers)` | no file-promise receiver equivalent | MISSING; screenshot/virtual-file drag must be separately implemented or explicitly validated via OS conversion. |
| `120 PanelDropCatcher.makeNSView` | eframe window input ← free `main` | PLATFORM_REPLACED. |
| `126 PanelDropCatcher.updateNSView` | per-frame drop event read ← app `update` | PLATFORM_REPLACED. |
| `130 PanelDropCatcher.wire` | dropped paths → loader → `Composer::add_attachments` via background event | WIRED for path drops; not proof of file promises. |

### PanelState.swift — 0 definitions

No explicit `func` or `init`. Original `pinned:7` maps to `PopChatApp::pinned` (header/Ctrl+P and blur gate); `focusBump:11` maps to visibility `FrameUpdate::request_composer_focus → Composer::request_focus` in `main::poll_events`. Both are WIRED state/action families, not counted functions.

### ProviderSwitcher.swift — 18 definitions

Rust: `src/main.rs::header_row`, `src/chat/providers.rs`. A real configured-provider/model/effort popup exists; it is still a **nested egui menu**, not the original three-lane preview switcher.

| Swift line / definition | Rust counterpart ← caller | Status / boundary |
|---|---|---|
| `33 init(store:)` | popup catalog/remembered model setup ← `main::header_row` | PARTIAL; no separate preview ID/model/effort indices. |
| `153 railRow` | provider submenu ← `main::header_row` | PARTIAL; configured predicate wired, original preview rail absent. |
| `244 modelRow` | `switcher::model_list` within provider submenu ← `main::header_row` | WIRED; viewport-bounded independent model ScrollArea with wheel input; click → `ProviderStore::select`. Original keyboard preview still PARTIAL. |
| `338 effortRow` | bounded model-list effort chips ← `main::header_row → switcher::model_list` | WIRED; metadata-gated exact native effort → `select(...Some(effort))`. |
| `382 effortLabel` | raw effort string ← `main::header_row` | PARTIAL; no friendly effort-label mapping. |
| `391 columnHeader` | submenu label/group rendering ← `main::header_row` | PARTIAL; original columns/captions not reproduced. |
| `401 dimRow` | manage-settings footer ← `main::header_row` | PARTIAL; not original shortcut/action row family. |
| `475 commitModel` | `ProviderStore::select` ← model click in `main::header_row` | WIRED; chosen/default effort resolved by store. |
| `480 commitEffort` | `ProviderStore::select` ← effort click in `main::header_row` | WIRED. |
| `485 commitProvider` | no direct provider-row commit; only model/effort clicks call `select` | PARTIAL; Enter on original provider lane semantics absent. |
| `493 close(then:)` | `ui.close()` ← popup row/footer | PARTIAL; no 130ms close animation/action ordering. |
| `545 move(delta)` | egui menu navigation; no explicit three-lane preview reducer | MISSING original preview behavior; framework navigation is not equivalence proof. |
| `562 syncModelIndex` | remembered model read ← `main::header_row` | PARTIAL; no keyboard preview cursor. |
| `567 syncEffortIndex` | `chosen_effort` read ← `main::header_row` | PARTIAL; no keyboard preview cursor/fallback lane. |
| `587 KeyCaptureView.makeNSView` | egui popup input routing ← `main::header_row` | PLATFORM_REPLACED; no hidden native first responder. |
| `593 KeyCaptureView.updateNSView` | egui popup input routing ← `main::header_row` | PLATFORM_REPLACED. |
| `603 CaptureView.viewDidMoveToWindow` | egui popup focus management | PLATFORM_REPLACED; original asynchronous one-time first-responder claim not replicated. |
| `612 CaptureView.keyDown` | egui menu keys ← `main::header_row` popup | PARTIAL; custom ↑↓/←→/Enter preview/commit law not ported. |

### Reveal.swift — 4 definitions

Rust: `src/chat/store.rs::TextReveal`, `src/ui/reveal.rs`. Pacing is wired via `main::update → tick_reveal → display_text`; alpha painting is now wired via `main::show_list → reveal_for → paint_reveal` immediately after assistant-body rendering, before find painting. The exclusive shape range excludes reasoning/actions. Bitmap math intentionally remains untouched, not a claim of NSTextAttachment fade parity.

| Swift line / definition | Rust counterpart ← caller | Status / boundary |
|---|---|---|
| `37 TextReveal.eased` | `TextReveal::eased` ← `ramp_with_bands/groups` ← store reveal clock → actual app painter | WIRED; easing feeds real glyph alpha. |
| `52 TextReveal.ramp` | `TextReveal::ramp/ramp_with_bands` ← reveal clock → `paint_reveal` ← `main::show_list` | WIRED; grapheme bands painted, not just paced prefix. |
| `71 TextReveal.groups` | `TextReveal::groups` ← reveal clock → actual app painter | WIRED; overlapping sentence ages/alphas drive real glyph shapes. |
| `84 RevealFade.paint` | `ui::reveal::paint_reveal/paint_galley` ← `main::show_list` | WIRED; post-body/pre-find range, final prose/code segment projection; bitmap attachment fade/caret behavior separately limited. |

### SearchField.swift — 8 definitions

Rust: `src/main.rs::show_find/show_history/poll_events` and egui TextEdit.

| Swift line / definition | Rust counterpart ← caller | Status / boundary |
|---|---|---|
| `27 KeyRoutingTextField.makeNSView` | TextEdit creation ← `show_find/show_history` | PLATFORM_REPLACED. |
| `41 updateNSView` | direct query binding / focus flag ← `show_find/show_history` | PLATFORM_REPLACED. |
| `62 makeCoordinator` | `FindState` / history app state ← app `new` | PLATFORM_REPLACED. |
| `68 Coordinator.init` | same retained query/focus state | PLATFORM_REPLACED. |
| `73 controlTextDidChange` | field `changed()` ← `show_find/show_history` | PLATFORM_REPLACED. |
| `78 control(doCommandBy:)` | arrows/Enter/Escape ← `show_find/show_history/poll_events` | PARTIAL; IME/key routing for these search fields not independently proven. |
| `112 AutoFocusingTextField.viewDidMoveToWindow` | TextEdit `request_focus` ← `show_find/show_history` | PLATFORM_REPLACED. |
| `124 View.keyCommand` | Ctrl shortcuts consumed ← `main::poll_events` | PLATFORM_REPLACED; no hidden SwiftUI button. |

### SettingsView.swift — 19 definitions

Rust: `src/ui/settings.rs::SettingsUi`; actions consumed by `main::settings_actions`; API catalog Save by `main::save_settings`. Catalog editing stays separate from live model selection. General preferences (including `font_preferences/window_preferences`), search keys, Commands/system prompt and Codex path autosave independently, preserving API draft fields/unknown JSON. Hotkey has its own draft/Apply → OS transaction → success-only commit; it is excluded from general autosave. Provider endpoint/API key/model catalog editing still uses explicit Save.

| Swift line / definition | Rust counterpart ← caller | Status / boundary |
|---|---|---|
| `79 init` | `SettingsUi::load/from_settings` ← app `new` | PARTIAL; embedded/tab presentation and explicit API-catalog Save differ; general/Commands/search/Codex-path live persistence is wired. |
| `271 accentSwatch` | preset buttons + `AccentPicker::show` ← `appearance_section` ← `show` | WIRED; presets/custom rainbow swatch/HSB popup immediately preview and independently autosave accent/custom changes, without committing provider drafts. |
| `435 providerRow` | `providers_section/provider_picker` ← `show` | PARTIAL; CRUD/actions real, different accordion presentation. |
| `482 providerBadge` | provider labels/status ← `providers_section` | PARTIAL; original free/local/custom badges not all reproduced. |
| `528 providerIsReady` | `ProviderStore::is_configured` ← popup `configured_providers`; no Settings readiness-dot caller found | PARTIAL; ready predicate exists, Settings green-dot/rail consistency UI not reproduced. |
| `533 providerSubtitle` | connection/subscription guidance ← `connection_fields/chatgpt_controls/codex_controls` | PARTIAL; status UI not identical. |
| `572 toggleEditor` | `SettingsUi::select_provider/load_provider` ← provider picker | PLATFORM_REPLACED; selector not expandable row. |
| `587 collapseEditor` | no matching collapse action; provider editor stays as selected pane | MISSING original accordion behavior. |
| `595 editorBand` | `connection_fields` ← `providers_section` | WIRED; endpoint/key/model and subscription-specific controls reachable. |
| `659 freeTierNotice` | template note/get-key hyperlink ← `providers_section` | WIRED; current source includes template caveat and key URL. |
| `671 pdfPassThroughRow` | PDF checkbox ← `providers_section` → `main::save_settings` | WIRED; affects active wire content. |
| `696 capabilityRows` | exception clear action ← `providers_section` → `settings_actions` | PARTIAL; per-image/file capability/source status rows not identified; clear/manual error actions real. |
| `735 capabilityNotice` | active composer capability warning ← `main::attachment_warning` | PARTIAL; Settings-specific capability notice not identified. |
| `753 exceptionLabel` | exception source stored in providers; no Settings source-label renderer found | CORE_ONLY; generic clear action is not source-specific labeling. |
| `763 editorField` | `field_label` + egui inputs ← `connection_fields` | PLATFORM_REPLACED. |
| `948 locateCodexExecutable` | native `rfd::FileDialog` worker ← `codex_controls`; result `poll_services → CodexPathChanged → main::settings_actions/persist_codex_path` | WIRED; immediate backend/path save + inspection invalidation; no API-draft Save required; picker suppresses blur dismissal. |
| `1094 editorWell` | bounded ScrollArea/TextEdit ← `commands_section` | PLATFORM_REPLACED; fixed prompt/template editors now present. |
| `1126 modelRow(for:)` | model input/catalog fetch ← `connection_fields` | WIRED; results keep identity/capabilities, Save doesn't switch live pill selection. |
| `1162 currentSearchKey` | `search_key_for_send` / engine secret load + `persist_general_preferences` ← `search_section/show`; `main::send_options` | WIRED; edited Brave/Tavily keys immediately saved in native store, never provider JSON; actual executor consumes selection. |

### Shortcuts.swift — 0 definitions

`KeyboardShortcuts.Name.togglePopChat:6` is a static value/action family, not a function. `src/ui/hotkey.rs::HotkeyManager::new_with_binding/try_register/set_binding/watch_loop` are reached from app `new`, retry, Settings `ChangeHotkey` and visibility driver. PLATFORM_REPLACED: **Alt+Down is the configurable default**, explicitly superseding the earlier fixed-key requirement; Alt+Space stays blocked. Settings now has an egui physical-key recorder producing a draft followed by explicit Apply, with app/watcher/driver suppression while capturing. OS registration succeeds before `SettingsUi::commit_hotkey_binding`; conflict/invalid input retains old registration and saved binding, while disk-write failure is separately reported. Earlier native Ctrl+Alt+P switching, relaunch persistence, old-key unregister and controlled OS conflict verified; Alt+Down restored. New recorder native key/IME/Win timing matrices remain unverified.

### Theme.swift — 30 definitions

Rust: `src/ui/theme.rs`, `src/ui/accent.rs`, `src/ui/surface.rs`, settings and immediate-mode widgets. Root theme application is wired at app `new/update`; `appearance_section` edits/autosaves HSB accent, BubbleStyle, separate fonts and window preferences; `main::show_list` selects the user-bubble palette. Backdrop opacity is wired independently of text opacity, but native glass/colored panel-tint controls remain missing; accent tint is a separate bubble style.

| Swift line / definition | Rust counterpart ← caller | Status / boundary |
|---|---|---|
| `67 AppearanceChoice.applyCurrent` | `theme::apply/resolve_dark` ← app `new/update` | WIRED; Auto/Light/Dark. |
| `89 Theme.color` | `hex_to_color32` ← settings/theme/renderers | PLATFORM_REPLACED; egui Color32. |
| `93 nsColor` | `hex_to_color32` ← palette/renderers | PLATFORM_REPLACED. |
| `101 components` | `components01/parse_rgb6` ← palette contrast | WIRED; Rust additionally trims surrounding whitespace. |
| `102 components.parse` | `parse_rgb6` ← `components01/hex_to_color32/normalize_hex` | WIRED; default-blue fallback. |
| `122 hex(hue:saturation:brightness:)` | `theme::hex_from_hsb` ← `AccentPicker::seed/apply/popover_body` | WIRED; active HSB picker/gradient/swatch uses uppercase six-digit hex conversion. |
| `127 hsb` | `theme::hsb_from_hex` ← `AccentPicker::seed` | WIRED; sRGB conversion/default-blue fallback; picker retains prior hue for gray separately. |
| `143 rgb(hue:saturation:brightness:)` | six-sector RGB arithmetic inside `hex_from_hsb` ← picker/gradient rendering | WIRED; hue wraps (including negative turns), saturation/brightness clamp; no OS/HDR conversion. |
| `162 byte` | channel clamp/round closure in `hex_from_hsb` ← picker apply/gradient rendering | WIRED; nearest-byte quantization with half-up rounding; same stored color suppresses duplicate publication. |
| `175 contrastingNSColor` | `contrasting_text_on` ← palette / accent buttons | WIRED; CIE L* threshold, not WCAG-max choice. |
| `177 contrastingNSColor.linear` | nested `linear` ← `contrasting_text_on` | WIRED. |
| `186 contrastingForeground` | `contrasting_text_on` → palette `accent_text` ← `from_hex/for_bubble_style` ← `main::show_list` | WIRED; CIE L* contrast is consumed by actual accentFill user rows as well as preset labels. |
| `191 bubbleFill` | `AccentPalette::for_bubble_style` → `bubble_frame` ← user row in `main::show_list` | WIRED; accentTint 15% light/19% dark alpha or opaque accentFill selected in Settings; retired `quietGray`/unknown persisted values deserialize as tint. Native compositing/visual equivalence unverified. |
| `201 bubbleForeground` | palette `user_text` ← `for_bubble_style` ← `main::show_list` label | WIRED; tint uses theme primary black/white, fill uses actual CIE L* accent contrast. |
| `205 bubbleForegroundNSColor` | same palette text ← user row | PLATFORM_REPLACED; Color32 rather than NSColor, with fill/tint foreground rule actively consumed. |
| `209 panelBorder` | card hairline ← `main::show_card` | PARTIAL; opacity tokens differ. |
| `218 liftedFill` | pasteable frame fill ← `markdown::show_pasteable` | PARTIAL; depth intent present, not exact theme tokens. |
| `222 liftedBorder` | pasteable stroke ← `show_pasteable` | PARTIAL; visual parity unverified. |
| `226 liftedDivider` | pasteable separator/header ← `show_pasteable` | PARTIAL; exact opacity/spacing not matched. |
| `231 recessedFill` | `code_bg_for/code_frame` ← `markdown::show_code_block` | PARTIAL; different opaque theme tokens. |
| `235 recessedHeader` | code header rendering ← `show_code_block` | PARTIAL. |
| `239 recessedBorder` | `bubble_stroke/code_frame` ← code renderer | PARTIAL. |
| `243 recessedCaption` | weak header/caption text ← code renderer | PARTIAL. |
| `253 VisualEffectBackground.makeNSView` | `surface::card_frame_with_opacity` using `theme::panel_fill` ← `main::show_card` | PARTIAL; configurable backdrop alpha, not native behind-window blur/glass. |
| `261 VisualEffectBackground.updateNSView` | theme/window preference update → `surface::card_frame_with_opacity` ← app `update/show_card` | PARTIAL; background-only opacity leaves text unchanged, no native material state. |
| `320 PanelGlassBackground.tintFill` | no glass/tint ramp | MISSING. |
| `334 PillBackground.body(content:)` | hover fills/icons ← `main::header_row` | PARTIAL; not source capsule/material treatment. |
| `350 GlassCard.body(content:)` | egui composer/card frame ← `show_composer_capsule/show_card` | PARTIAL; card backdrop alpha configurable, composer remains its own surface; neither is native glass. |
| `361 View.pillBackground` | header widget frames ← `main::header_row` | PLATFORM_REPLACED; original visual effect partial as above. |
| `365 View.glassCard` | egui `Frame` ← composer/settings rendering | PLATFORM_REPLACED; no glass implementation implied. |

### UpdateChecker.swift — 6 definitions

Rust: `src/update.rs`. Production owner is `SettingsUi::updates`; app `update → SettingsUi::poll_services → UpdateScheduler::poll` runs even with settings closed. Manual check/release link in settings `hotkey_section`.

| Swift line / definition | Rust counterpart ← caller | Status / boundary |
|---|---|---|
| `65 checkIfDue` | `UpdateScheduler::poll` ← `SettingsUi::poll_services` ← app `update` | WIRED; 5s deferred launch, daily policy; separate `update::check_if_due` helper is not the active scheduler. |
| `75 check(userInitiated:)` | `UpdateScheduler::check` ← automatic poll / settings manual button | WIRED; background errors silent, manual errors visible. |
| `105 fetchLatest` | `update::fetch_latest` ← `check_for_update` ← scheduler task | WIRED; no network performed by audit. |
| `134 decodeRelease` | `update::decode_release` ← `fetch_latest` | WIRED. |
| `148 isNewer` | `update::is_newer` ← `check_for_update` | WIRED. |
| `163 components` | `update::version_components` ← `is_newer` | WIRED; not proof of live release endpoint availability. |

## Chat, storage, authentication and transports

### Chat/Attachment.swift — 9 definitions

Rust: `src/chat/attach.rs`. Active route: picker/drop/paste → `main::load_attachment_paths/load_attachments` → `load_store_attachment/into_store_attachment` → `Composer` draft → `ChatStore::send → wire_content_for_message`. Loader fixtures are not live document-format compatibility guarantees.

| Swift line / definition | Rust counterpart ← caller | Status / boundary |
|---|---|---|
| `59 AttachmentLoader.load(url:)` | `load_store_attachment → load_file` ← app attachment loader | WIRED; caps/routing/errors, now converted into active store type. |
| `94 load(image:suggestedName:)` | `load_image_bytes` ← clipboard loader | WIRED; clipboard image decoding is a platform replacement. |
| `106 encode(image:filename:)` | `encode_image` ← file/image loader | PARTIAL; 2048/JPEG/cap implemented; HEIC/HEIF NSImage decode coverage missing. |
| `143 loadPDF` | `load_pdf_bytes` ← `load_file` | PARTIAL; real PDF text/raw retention/password/page/size policies; lopdf extraction not proven identical to PDFKit on arbitrary PDFs. |
| `194 loadWordDocument` | `load_word_bytes/extract_doc/extract_zip_document/extract_rtf/load_rtfd_bundle` ← `load_file` | PARTIAL; real DOC/DOCX/ODT/RTF/RTFD extraction; parser subset not identical to NSAttributedString document import. |
| `215 loadSpreadsheet` | `load_spreadsheet/spreadsheet_from_archive` ← `load_file` | WIRED; XLSX/XLSM row/character caps, warnings and sheet names. |
| `266 cellText` | `parse_sheet_rows/parse_shared_strings` ← spreadsheet parser | PARTIAL; shared/inline/raw numeric tested in source; all ISO date/rich-string cases not proven. |
| `281 escapeCSV` | `escape_csv` ← `parse_sheet_rows` | WIRED. |
| `290 loadPlainText` | `load_plain_bytes` ← `load_file` | WIRED; NUL refusal/lossy UTF-8/empty/truncation/grapheme policy. |

### Chat/ChatGPTAuth.swift — 24 definitions

Rust: `src/chat/chatgpt_auth.rs`. Production sign-in/out/cancel caller: `SettingsAction → main::settings_actions`. Production refresh caller: registered `ResponsesBackend`, not just test helpers. Native tokens use chunked Windows credentials with manifest/readback/cleanup; **no real credential or account accessed by this audit**.

| Swift line / definition | Rust counterpart ← caller | Status / boundary |
|---|---|---|
| `56 supportedReasoningEfforts` | `supported_reasoning_efforts` ← `providers::chatgpt_catalog` ← provider load/auth catalog refresh | WIRED. |
| `60 defaultReasoningEffort` | `default_reasoning_effort` ← `chatgpt_catalog` | WIRED. |
| `71 AuthError.init(message:statusCode:)` | `AuthError::new` ← token HTTP/decode paths | WIRED. |
| `101 loadTokens` | `NativeTokenStore::load/load_chunked_tokens` ← app init/send/poll and backend credentials | PLATFORM_REPLACED; credential chunks, not original secret-file blob. |
| `109 save(tokens)` | `NativeTokenStore::save/save_chunked_tokens` ← `sign_in/credentials_at` | PLATFORM_REPLACED; verified native persistence/rotation, local test references only. |
| `123 signOut` | `sign_out` ← app Settings signout task | WIRED; waits for cancelled login before deletion. |
| `133 signIn` | `sign_in` ← app Settings signin task | WIRED; listener binds before browser launch; real browser/provider flow unverified. |
| `169 withCallbackServer` | `bind_callback/callback_server` ← `sign_in` | WIRED; owned listener/connections cancel on future drop. |
| `255 ResumeGate.claim` | `Mutex<Option<oneshot::Sender>>::take` ← callback connection | PLATFORM_REPLACED; exactly-once claim preserved structurally. |
| `267 receiveRequestLine` | bounded fragmented-line read loop ← `callback_server` connection | WIRED. |
| `289 parseCallback` | `parse_callback` ← callback server | WIRED; route/state checked before accepting OAuth errors. |
| `316 htmlEscape` | `html_escape` ← `failure_page` | WIRED. |
| `324 respond(connection:status:body:)` | HTTP response write/shutdown inline ← callback connection | PLATFORM_REPLACED; Tokio socket. |
| `347 exchangeCode` | `exchange_code` ← `sign_in` | WIRED; PKCE verifier and token request. |
| `361 validCredentials` | `credentials_at(force_refresh=false)` ← `ResponsesBackend::stream_one_round`; public `valid_credentials` wrapper also exists | WIRED via shared implementation; wrapper existence alone is not evidence. |
| `374 refreshedCredentials` | `credentials_at(force_refresh=true)` ← Responses 401 retry; public `refreshed_credentials` wrapper also exists | WIRED; one forced refresh/retry, invalid grant vs transient failure distinguished. |
| `382 refresh(tokens)` | `refresh` ← `credentials_at` | WIRED. |
| `406 tokenRequest(form:)` | `token_request/token_request_at` ← exchange/refresh | WIRED; endpoint-injectable tests do not prove issuer availability. |
| `428 buildTokens` | `build_tokens` ← exchange/refresh | WIRED; account required, optional previous claims preserved. |
| `454 accountID(in:)` | `account_id_in` ← `build_tokens` | WIRED. |
| `465 decodeJWTClaims` | `decode_jwt_claims` ← `build_tokens` | WIRED; extraction, not signature validation. |
| `476 randomURLSafe(bytes:)` | `random_bytes + base64url_encode` ← `login_url` | PLATFORM_REPLACED; OS secure randomness via crate. |
| `485 base64URL` | `base64url_encode` ← PKCE/login URL | WIRED. |
| `492 formEncode` | `percent_encode` ← `login_url`; reqwest `.form` ← token request | PLATFORM_REPLACED; actual token form encoding delegated to reqwest. |

### Chat/ChatStore.swift — 25 definitions

Rust: active `src/chat/store.rs::ChatStore` (not the parallel `src/persistence.rs` model). Actual send entry is `main::send_current/send_payload`; event entry `main::poll_stream`; renderer reads `display_text` while wire/copy/persistence use arrived text.

| Swift line / definition | Rust counterpart ← caller | Status / boundary |
|---|---|---|
| `55 searchableText` | `ChatStore::searchable_text` has no production caller; app calls `ConversationStore::full_text_index` directly | PARTIAL; body-search feature wired through app background cache, original store-cache method CORE_ONLY. |
| `124 recordManualCapabilityException` | `record_manual_capability_exception` ← `main::show_list` error-row action | WIRED; records provider/model from failed turn, not current pill. |
| `133 init(providerStore:shortcutStore:)` | `ChatStore::new` ← app `new` | WIRED; loads recent/resumes latest, active dependencies passed at send. |
| `143 adopt(loaded)` | `adopt` ← store `new/complete_conversation_load` ← app background load | WIRED; missing-parent notice/shared prefix restored. |
| `198 send(text:attachments:)` | `send` ← `main::send_payload` ← composer/retry | WIRED; slash/provider/auth/capability/tool routing reached; live providers unverified. |
| `435 commit(reasoning:to:)` | `commit_reasoning` ← `apply_event/finish_turn` ← `main::poll_stream` | WIRED; reasoning durable but excluded from model history. |
| `448 typewriterInterval` | `typewriter_interval_ms` ← reveal clock | WIRED; pacing, not proof of glyph fade. |
| `462 typewriterStep` | `typewriter_step` ← `tick_reveal` ← app `update` | WIRED; backlog/finished-tail behavior. |
| `482 startDrain(messageID:mode:)` | `RevealDrain`, `accept_arrived_text/tick_reveal` ← events/app clock → `reveal_for/paint_reveal` ← `main::show_list` | WIRED; pacing and glyph/sentence alpha painting both have actual app callers. |
| `561 sentenceCommitLength` | `sentence_commit_length` ← `tick_reveal` | WIRED; boundary/fence/grapheme-aware drain. |
| `632 wireContent` | `wire_content_for_message` ← `history_for_wire` ← `send` | WIRED; text/images/PDF redecided at send, warning notes handled. |
| `668 resolveWebAccess` | free `resolve_web_access` ← `send` using `main::send_options` | WIRED; native Codex search, OpenRouter plugin or actual local tools. |
| `674 keyedEngine` | keyed-engine branch ← `resolve_web_access` | WIRED; active Settings key passed; fallback activity/warning rather than silent fake search. |
| `705 stop` | `ChatStore::stop` ← `main::stop_active_stream` | WIRED; forwarding task abort drops receiver, transport cancellation paths exist. |
| `718 fullText(of:)` | `full_text` ← full-message Copy / wire/persist/find code | WIRED; returns arrived target, not displayed prefix. |
| `727 flushTypewriter` | `flush_reveal` ← stop/truncate/new/fork/snapshot replacement | WIRED; stale fade/prefix cleared. |
| `742 fork(at:)` | `fork` ← `prepare_message_action(Fork)` ← row action | WIRED; branch prefix persisted, source preserved. |
| `784 retryLastTurn` | `retry_last_turn` ← `prepare_message_action(Retry)` → `main::send_payload` | WIRED; payload function actually resubmitted by caller. |
| `795 takeLastUserMessageForEditing` | `take_last_user_message_for_editing` ← Edit → `Composer::set_draft` | WIRED; attachments restored. |
| `810 truncate(to:)` | `truncate` ← retry/edit | WIRED; materializes fork children before rewrite. |
| `838 deleteConversation` | `delete_conversation` ← `main::delete_chat` ← history | WIRED; materialize children/current branch adjustment. |
| `857 newChat` | `new_chat` ← app new-chat button/Ctrl+N | WIRED; app additionally cancels stream/invalidates async draft/load. |
| `867 loadConversation` | app `load_chat → ConversationStore::load_resolved → complete_conversation_load` | WIRED; store's synchronous `load_conversation` has no direct app caller; equivalent active async route identified. |
| `882 persist` | `persist` ← send/events/turn completion/new/load/app exit | WIRED; active store JSON, write failure reporting not guaranteed. |
| `905 title(for:)` | `title_for` ← `persist` | WIRED; first user prompt title. |

### Chat/CodexAppServerClient.swift — 49 definitions

Rust: `src/chat/codex.rs`. Production chain: `main::new → client::set_delegated_backend`; `main::start_stream → client::run → backends::DelegatedBackend::start → CodexBackend::start → drive_history/drive_history_inner`. Inspection: settings/popup → `main::inspect_codex → codex::inspect`. Windows executable discovery/process supervision deliberately differs from macOS login-shell probing. Real installed Codex/account not exercised.

| Swift line / definition | Rust counterpart ← caller | Status / boundary |
|---|---|---|
| `56 executableURL` | `resolve_executable` ← `inspect/CodexBackend::start` | PLATFORM_REPLACED; configured path/PATH/Windows candidates and batch shims. |
| `111 loginShellCodexURL` | Windows `candidate_dirs/resolve_executable` ← actual inspect/turn | PLATFORM_REPLACED; no interactive login shell sourcing. |
| `167 ShellProbeCache.get` | executable resolution per inspect/turn; no matching shell-result cache | PLATFORM_REPLACED; source macOS probe optimization not needed by this mechanism. |
| `175 ShellProbeCache.set` | configured executable lock ← `DelegatedBackend::set_codex_executable` ← settings/app init | PLATFORM_REPLACED; configuration cache, not shell-discovery cache. |
| `189 ProbeBuffer.init(marker:)` | no login-shell marker transport; Windows discovery instead | PLATFORM_REPLACED. |
| `191 ProbeBuffer.append` | no login-shell marker accumulation | PLATFORM_REPLACED. |
| `199 waitForMarkerLine` | no login-shell marker wait | PLATFORM_REPLACED. |
| `205 markerPayload` | no login-shell marker payload | PLATFORM_REPLACED. |
| `217 completedMarkerLine` | no login-shell marker framing | PLATFORM_REPLACED. |
| `226 inspect(includeModels:)` | `codex::inspect` ← `main::inspect_codex` | WIRED; actual initialize/account/model list mapped, always full model inspection. |
| `269 inspectBlocking` | async `inspect/initialize/read_chatgpt_account` ← app inspection task | WIRED; fake-executable test reference, not live Codex proof. |
| `329 run` | `CodexBackend::start` ← registered delegated router | WIRED; adapters bridge into active `client::ChatStreamEvent`. |
| `371 runTurn` | `drive_history/drive_history_inner` ← `CodexBackend::start` | WIRED; prior history/current images/effort/native-search/sandbox boundaries carried. |
| `478 reasoningSnapshot` | local reasoning snapshot closure ← `drive_history_inner` | WIRED; summary preferred over raw. |
| `621 reasoningKey` | `reasoning_key` ← reasoning event handling | WIRED. |
| `650 ItemAssembly.delta` | `ItemAssembly::delta` ← body/reasoning events | WIRED. |
| `661 ItemAssembly.completed` | `ItemAssembly::completed` ← completed-agent-message event | WIRED. |
| `674 ItemAssembly.dropInFlight` | `drop_in_flight` ← retryable-error event | WIRED; replay doesn't duplicate unfinished content. |
| `685 timeoutMessage` | `timeout_message` ← read/write/turn inactivity timeouts | WIRED. |
| `704 readChatGPTAccount` | `read_chatgpt_account` ← inspect/turn handshake | WIRED; typed not-signed-in result. |
| `722 responseResult` | `response_result` ← handshake/request/turn response handling | WIRED; typed protocol errors. |
| `739 splitHistory` | `split_wire_history` ← `drive_history_inner` | WIRED; last user split and system prompt. |
| `752 textContent` | `text_content` ← `split_wire_history` | WIRED. |
| `761 responseItem` | `wire_response_item` ← prior-message injection | WIRED. |
| `786 turnInput` | `wire_parts(current_turn=true)` ← turn/start construction | WIRED; typed text/image input. |
| `809 webSearchLabel` | `web_search_label` ← web-search-completed event | WIRED. |
| `816 activityLabel` | `activity_label` ← item-started event | WIRED. |
| `830 friendlyError` | `friendly_error` ← server error/turn-failed events | WIRED. |
| `838 appServerWorkingDirectory` | `app_server_working_directory` ← turn startup | PLATFORM_REPLACED; isolated platform data path, not user's cwd. |
| `866 JSONValue.init(from:)` | `serde_json::Value` deserialization ← `AppServerSession::read_message` | PLATFORM_REPLACED; integer IDs explicitly checked by `id_as_i64`. |
| `877 JSONValue.encode(to:)` | serde JSON serialization ← `AppServerSession::send` | PLATFORM_REPLACED. |
| `910 SessionHolder.set` | owned task/session lifetime ← `CodexBackend::start` select on receiver closure | PLATFORM_REPLACED; no shared Swift cancellation holder. |
| `921 SessionHolder.stop` | receiver-close cancellation + `AppServerSession::shutdown/Drop` | PLATFORM_REPLACED; Windows JobObject supervises descendants. |
| `935 checkCancellation` | receiver `.is_closed/.closed()` guards ← backend/drive loop | PLATFORM_REPLACED; cancellation also covers handshake/backpressured write. |
| `953 InactivityWatchdog.init` | session inactivity duration + Tokio timeout ← inspect/backend setup | PLATFORM_REPLACED; no DispatchSource timer object. |
| `971 InactivityWatchdog.kick` | new read/turn deadlines ← `read_message/drive_history_inner` | PARTIAL; resets on decoded messages rather than exact Swift watchdog byte-activity callbacks; partial-line timing semantics need comparison. |
| `977 InactivityWatchdog.cancel` | owned timeout futures dropped with task/session | PLATFORM_REPLACED; cleanup via shutdown/Drop. |
| `985 InactivityWatchdog.check` | Tokio timeout branches ← session read/send/turn loops | PLATFORM_REPLACED; typed timeout output. |
| `1026 Session.init(executable:webSearch:onMessage:)` | `AppServerSession::spawn/new` ← inspect/backend | PLATFORM_REPLACED; windowless Tokio child, buffered writer/stderr task/JobObject. |
| `1092 Session.initialize` | free `initialize` ← inspect/turn handshake | WIRED. |
| `1104 Session.request` | `AppServerSession::request` ← initialize/account/model/thread/injection | WIRED; unrelated messages buffered. |
| `1118 beginRequest` | `begin_request` ← request / turn-start | WIRED; turn notifications consumed before response finishes. |
| `1125 notify` | `notify` ← initialize | WIRED. |
| `1129 nextMessage` | `next_message` ← turn event loop | WIRED. |
| `1134 Session.stop` | `shutdown/Drop` ← inspect completion / turn/cancellation | PLATFORM_REPLACED; kills/reaps child, descendant job cleanup. |
| `1146 send(object)` | `send` ← begin_request/notify | WIRED; bounded async writer channel/backpressure/cancellation. |
| `1166 readMessage` | `read_message` ← request/next_message | WIRED; JSONL/blank lines/errors, Tokio framing instead of FileHandle reads. |
| `1194 appendStderr` | `drain_stderr` ← spawned stderr task | WIRED; bounded diagnostic tail. |
| `1202 stderrSuffix` | `stderr_suffix_blocking` ← request/inspection failures | WIRED. |

### Chat/CodexResponsesClient.swift — 9 definitions

Rust: `src/chat/responses.rs`. This is the distinct HTTP Responses transport, **not** local Codex app-server. Registration/caller proven in `main::new/start_stream → client::run → DelegatedBackend::start → ResponsesBackend::start`. Session UUID set by actual `main::start_stream`.

| Swift line / definition | Rust counterpart ← caller | Status / boundary |
|---|---|---|
| `35 run` | `ResponsesBackend::start/start_with_session` ← delegated router | WIRED; real subscription service not verified. |
| `50 runLoop` | `ResponsesBackend::run_loop` ← `start_with_session` | WIRED; tool rounds/replay/final forced answer, cancellation ownership. |
| `152 inputItem(for:)` | `input_item` ← run_loop history conversion | WIRED; typed message input. |
| `198 streamOneRound` | `ResponsesBackend::stream_one_round` ← run_loop | WIRED; credentials/headers/session/HTTP and forced-refresh retry. |
| `274 credentials(forceRefresh:)` | `chatgpt_auth::credentials_at` ← stream_one_round | WIRED; native store and refresh are active. |
| `281 consumeStream` | `consume_stream`, `StreamAssembly::line` ← stream_one_round | WIRED; typed deltas/reasoning/items/errors/truncation and fragmented SSE. |
| `390 truncationMessage` | `truncation_message` ← StreamAssembly terminal event | WIRED. |
| `401 errorText` | `error_text` ← stream/HTTP error handling | WIRED. |
| `412 friendlyHTTPError` | `friendly_http_error` ← stream_one_round HTTP branch | WIRED; local SSE/HTTP test references only. |

### Chat/Conversations.swift — 16 definitions

Rust: active `src/chat/store.rs::{ConversationStore,ConversationMeta}`. The similarly named `src/persistence.rs` implementation is **not evidence for active conversation behavior**. App owns background history index and resolved loads; store owns persistence/fork deletion/pruning.

| Swift line / definition | Rust counterpart ← caller | Status / boundary |
|---|---|---|
| `24 ConversationMeta.snippet(for:)` | `ConversationMeta::snippet_for` ← `ConversationStore::list_recent` ← store new/persist/history | WIRED. |
| `53 url(for:)` | `ConversationStore::url_for` ← save/load/delete | PLATFORM_REPLACED; Windows profile data path. |
| `57 save(conversation)` | `save` ← `ChatStore::persist` / materialize | WIRED; original also silent on many failures; durability not proven by reachability. |
| `64 load(id:)` | `load` ← `load_resolved/resolve_messages` | WIRED. |
| `71 delete(id:)` | `delete` ← safe delete/pruning | WIRED. |
| `81 resolveMessages(conversation)` | `resolve_messages` ← `load_resolved` | WIRED. |
| `85 resolveMessages(recursive)` | `resolve_messages_with` ← resolver/materialize/index | WIRED; cycles/missing ancestor flags. |
| `105 loadResolved(id:)` | `load_resolved` ← store startup / `main::load_chat` task | WIRED. |
| `114 materializeChildren(of:)` | `materialize_children` ← safe delete / `ChatStore::truncate` | WIRED. |
| `123 deleteMaterializingChildren(id:)` | `delete_materializing_children` ← `ChatStore::delete_conversation` ← history delete | WIRED. |
| `128 materialize(conversation:in:)` | `materialize` ← children/pruning | WIRED. |
| `139 loadAllResolved` | `load_all_resolved` ← index/list_recent | WIRED. |
| `151 loadAll` | `load_all` ← materialize/index/list_recent | WIRED. |
| `177 fullTextIndex` | `full_text_index` ← `main::refresh_history_index` background task | WIRED; active path excludes reasoning/activity/errors. |
| `193 excerpt(of:matching:context:)` | `excerpt` ← free `history_results` ← `main::show_history` | PARTIAL; lowercase/scalar context differs from Foundation case-insensitive/Swift Character edge cases. |
| `208 listRecent` | `list_recent` ← store new/persist/delete/load | WIRED; prune/materialize and resolved snippets. |

### Chat/OpenAIChatClient.swift — 13 definitions

Rust: `src/chat/client.rs`; actual caller `main::start_stream → client::run`; local web executor is now the real `chat/web.rs` adapter. HTTP/SSE tests in source are not real provider validation.

| Swift line / definition | Rust counterpart ← caller | Status / boundary |
|---|---|---|
| `62 WireContent.encode(to:)` | untagged serde `WireContent` serialization ← HTTP request builder | PLATFORM_REPLACED; bare text string vs part array preserved. |
| `70 WireContent.init(from:)` | serde `WireContent` deserialization ← data/model decoding; no app need to decode request history identified | CORE_ONLY; encoding active, decode route not user behavior proof. |
| `105 WirePart.text` | `WirePart::text` ← active `wire_content_for_message` | WIRED. |
| `109 WirePart.imageDataURL` | `WirePart::image_data_url` ← active attachment wire encoder | WIRED. |
| `113 WirePart.fileDataURL` | `WirePart::file_data_url` ← PDF wire encoder | WIRED. |
| `130 WireMessage.init` | `WireMessage::new` / literal construction ← `history_for_wire/run_loop` | WIRED. |
| `183 run` | `client::run` ← `main::start_stream` | WIRED; delegated backend registered, normal APIs use Chat Completions. |
| `197 runLoop` | `run_loop` ← run | WIRED; actual tool output appended and sent in next round. |
| `297 StreamDelta.init(from:)` | `StreamDelta::deserialize` ← `decode_delta` | WIRED; content/reasoning spellings/tool fragments. |
| `354 decodeDelta(line:)` | `decode_delta` ← stream_one_round SSE parser | WIRED. |
| `369 streamOneRound` | `stream_one_round` ← run_loop | WIRED; DONE exits request, split UTF-8/CRLF/final line/cancellation code present. |
| `479 attributeContentRejection` | `attribute_content_rejection` ← stream_one_round HTTP-error branch | WIRED; actual sent history/part indices supplied. |
| `498 capability(forPartType:)` | part-type match inside `attribute_content_rejection` | WIRED; active store records rejection/manual fallback actions. |

### Chat/PromptHistory.swift — 1 definition

Rust: `src/chat/store.rs::PromptHistory` (active), not unrelated config history in `src/persistence.rs`.

| Swift line / definition | Rust counterpart ← caller | Status / boundary |
|---|---|---|
| `22 record(text)` | `PromptHistory::record` ← `Composer::take_send` and `ChatStore::send` | WIRED; trim/dedup/bounded persisted history; both callers use same active store. |

### Chat/ProviderStore.swift — 45 definitions

Rust: `src/chat/providers.rs::ProviderStore`, Settings fetch worker in `src/ui/settings.rs`; app `header_row/settings_actions/save_settings/lazy_fetch_models/inspect_codex`. Standalone store methods with no caller are called out rather than inferred from their tests.

| Swift line / definition | Rust counterpart ← caller | Status / boundary |
|---|---|---|
| `29 Provider.init(id:name:baseURL:isPreset:defaultModel:kind:freeTemplate:)` | `Provider::new` ← presets/add_custom/add_template | WIRED. |
| `40 Provider.init(from:)` | derived serde Provider + custom `ProviderKind::deserialize` ← `ProviderStore::load` | PLATFORM_REPLACED; unknown kind fallback. |
| `123 FreeProviderTemplate.named` | `free_template_named` ← app add-template/settings template notice | WIRED. |
| `245 ProviderStore.init` | `ProviderStore::load` ← app `new` | WIRED; presets/migrations/catalog/secrets. |
| `384 chatGPTPreset` | `chatgpt_preset` ← presets/load | WIRED. |
| `395 codexAppServerPreset` | `codex_app_server_preset` ← presets/load | WIRED. |
| `406 presets` | `presets` ← load/default | WIRED. |
| `436 isConfigured` | `is_configured/is_configured_static` ← `configured_providers` ← app model popup | PARTIAL; predicate active but Settings readiness indicator not identified. |
| `478 hasSetupEvidence` | `has_setup_evidence` ← no production caller found | CORE_ONLY; current header renders model string directly, original setup CTA stability absent. |
| `496 rememberedModel` | `remembered_model` ← popup/current_model/settings load | WIRED. |
| `500 setModel(model)` | `set_model` ← only unused `set_model_for` wrapper chain | CORE_ONLY direct API; live change uses `select`, catalog editor deliberately avoids selecting. |
| `504 setModel(model:for:)` | `set_model_for` ← `set_model` only; app equivalent is `select` | CORE_ONLY direct API; behavioral live selection route is wired. |
| `508 supportedEfforts` | `supported_efforts` ← popup / remembered effort | WIRED; explicit metadata, not guessed model names. |
| `512 rememberedEffort` | `remembered_effort` ← chosen/current effort | WIRED. |
| `529 chosenEffort` | `chosen_effort` ← app popup / select | WIRED. |
| `533 setEffort` | `set_effort` ← `select` / app SettingsAction | WIRED; persists remembered model-specific value. |
| `540 hasEffortModels` | `has_effort_models` ← no production caller found | CORE_ONLY; popup computes per-model effort availability directly. |
| `550 attachmentCapabilities(for:model:)` | `attachment_capabilities` ← current/send | WIRED. |
| `561 currentAttachmentCapabilities` | `current_attachment_capabilities` ← app draft warning / store send | WIRED. |
| `565 resolveCapabilities` | free `resolve_capabilities` ← attachment_capabilities | WIRED; metadata/default/exception precedence. |
| `582 kindDefaultCapabilities` | `kind_default_capabilities` ← attachment_capabilities | WIRED. |
| `599 recordCapabilityException` | `record_capability_exception` ← store automatic rejection / manual error-row action | WIRED. |
| `613 mergedException` | `merged_exception` ← record_capability_exception | WIRED; manual-source precedence retained. |
| `630 clearCapabilityException` | `clear_capability_exception` ← SettingsAction consumed by app | WIRED. |
| `638 select(id:model:effort:)` | `select` ← real model/effort popup / SettingsAction | WIRED; settings catalog Save doesn't redirect live selection. |
| `647 applyChatGPTCatalog` | `apply_chatgpt_catalog` ← load / app auth completion / lazy fetch | WIRED. |
| `661 refreshCodexAppServer` | `main::inspect_codex` + `codex_task` guard + provider generation | PARTIAL; one full inspection task shared by app actions, not original awaitable includeModels/coalesced-follow-up API. |
| `694 invalidateCodexAppServer` | `invalidate_codex_app_server` ← app path change in save/inspect action | WIRED; old generation results rejected. |
| `706 performCodexRefresh` | `mark_codex_checking/apply_codex_inspection` ← app inspect/background event | WIRED; typed failure/status/model/effort publication. |
| `766 currentKey` | `current_key` ← current_config ← store send | WIRED. |
| `770 key(for:)` | `key_for` ← config/settings/lazy models | WIRED; native SecretStore. |
| `774 setKey(key:for:)` | `set_key` ← `main::save_settings` | WIRED; explicit save/error result instead of Swift property write-through. |
| `781 currentConfig` | `current_config` ← `ChatStore::send` | WIRED; kind/model/effort/key reach actual router. |
| `795 addCustom` | `add_custom` ← `main::settings_actions(AddProvider)` | WIRED; catalog add does not select live provider. |
| `809 addTemplate` | `add_template` ← app template action | WIRED; unique custom name/origin marker. |
| `820 uniqueName` | `unique_name` ← add_custom/add_template | WIRED. |
| `832 remove(id)` | `remove` ← app remove action | WIRED; secret deletion errors returned; preset guard. |
| `874 decodeModelList` | `decode_model_list` ← fetch_models_list ← settings/popup fetch | WIRED; capabilities retained by production consumers. |
| `887 isFetching` | `is_fetching` ← `main::header_row` | WIRED. |
| `893 lazyFetchModels(for:)` | `lazy_fetch_plan` ← `main::lazy_fetch_models` ← popup click | WIRED; API/Codex/ChatGPT branches have actual workers. |
| `912 fetchModels()` | store `fetch_models` wrapper no app caller; active `SettingsUi::start_model_fetch` | PARTIAL; user fetch wired through scoped UI worker, not this wrapper. |
| `916 fetchModels(for:)` | store `fetch_models_for` no app caller; actual settings `fetch_catalog` + app `lazy_fetch_models` | PARTIAL; same low-level fetch/metadata/probe reached by actual workers. |
| `986 probeOllamaCapabilities` | `probe_ollama_capabilities` ← settings `fetch_catalog` / app lazy worker | WIRED; native `/api/show` capabilities preserved. |
| `1029 persistJSON` | serde + `ProviderStore::try_save/save` ← mutations/app Save | PLATFORM_REPLACED; file-based provider state, not UserDefaults per key. |
| `1035 loadJSON` | serde read ← `ProviderStore::load` ← app `new` | PLATFORM_REPLACED. |

### Chat/SecretStore.swift — 5 definitions

Rust: active `src/chat/store.rs::SecretStore`, Windows native credential backend; auth additionally uses `chatgpt_auth::NativeTokenStore` chunk protocol. No real stores read in this audit.

| Swift line / definition | Rust counterpart ← caller | Status / boundary |
|---|---|---|
| `23 load` | `SecretStore::load_file/migrate_legacy_file` ← `new` | PLATFORM_REPLACED; legacy-file migration to native Windows credentials. |
| `35 save(secrets)` | `persist_native/save_file` ← `set/delete/migration` | PLATFORM_REPLACED; native readback and atomic explicit file backend, no silent native-failure plaintext fallback. |
| `45 get(account:)` | `SecretStore::get` ← provider keys/search keys/auth chunk reader | WIRED; platform backend differs. |
| `49 set(value:account:)` | `SecretStore::set` ← provider Save/search-key Save/auth chunk writes | WIRED; explicit errors. |
| `59 delete(account:)` | `SecretStore::delete` ← provider remove/empty key/auth signout | WIRED; verified absence/legacy cleanup paths. |

### Chat/ShortcutStore.swift — 6 definitions

Rust: active disk/slash execution `src/chat/store.rs`; readonly completion mirror `src/chat/commands.rs`; settings `commands_section` edits actual UUID-backed entries and publishes `CommandsChanged` to app. Do not equate tests of the mirror with active execution.

| Swift line / definition | Rust counterpart ← caller | Status / boundary |
|---|---|---|
| `11 PromptShortcut.expand(input:)` | `store::PromptShortcut::expand` ← `ChatStore::send` | WIRED; separate `commands::PromptShortcut::expand` also exists, not active execution. |
| `27 ShortcutStore.init` | `store::ShortcutStore::load` ← app `new` / SettingsUi initialization | WIRED. |
| `48 match(name:)` | active `store::ShortcutStore::find` ← `ChatStore::send`; `composer::filter_commands` ← `Composer::show` for completion; mirror `commands::match_name` is not the app execution caller | PARTIAL; active execution and completion now use Unicode `to_lowercase`, not ASCII-only matching. Exact Foundation case-insensitive folding/locale equivalence is not established by lowercasing. |
| `52 add` | direct `ShortcutStore::add` no app caller; equivalent append ← Settings `commands_section` | WIRED behavior via real editor; new row focus and autosave. |
| `56 remove(id:)` | direct `ShortcutStore::remove` no app caller; retain/delete ← `commands_section` | WIRED behavior via real editor. |
| `60 persist` | `ShortcutStore::save_entries` ← `commands_section` on edits/retry | WIRED; current editor autosaves + live app list update, not older progress-note Save-only state. |

### Chat/WebTools.swift — 11 definitions

Rust: `src/chat/web.rs`. Production chain: `client::run_loop` or `ResponsesBackend::run_loop` → `client::WebToolExecutor::execute` → real `web::WebToolExecutor::execute`. **The old stub is no longer the active implementation.** Separate generic `web::run_with_tools` driver is not the app entry.

| Swift line / definition | Rust counterpart ← caller | Status / boundary |
|---|---|---|
| `49 activityLabel` | `web::WebToolExecutor::activity_label` ← client adapter ← both run loops | WIRED. |
| `58 execute(name:argumentsJSON:)` | `web::WebToolExecutor::execute` ← client adapter ← active tool calls | WIRED; returns real search/fetch output/errors. |
| `76 decodeArguments` | `decode_arguments` ← execute/activity_label | WIRED. |
| `91 search(query:)` | `web_search` ← execute | WIRED; actual engine dispatch. |
| `106 searchDuckDuckGo` | `search_duckduckgo/parse_ddg_results` ← web_search | WIRED; current external markup/service reliability unverified. |
| `142 searchTavily` | `search_tavily` ← web_search | WIRED; Settings secret used; no live key exercised. |
| `165 searchBrave` | `search_brave` ← web_search | WIRED; Settings secret used; no live key exercised. |
| `192 fetch(urlString:)` | `fetch_url` ← execute | WIRED; fetches URL and strips/truncates HTML. |
| `207 getString(url:headers:)` | `get_string` / engine reqwest request builders ← search/fetch | WIRED; UA/headers/query/timeout adapters. |
| `219 checkHTTP` | `check_http` ← engine/fetch response paths | WIRED; HTTP status error detail. |
| `231 stripHTML` | `strip_html` ← fetch/parser | WIRED; source regex-order references, not full browser extraction. |

## main.swift — 63 definitions, all TEST_HARNESS

The normal application bootstrap is top-level `main.swift:2988–2996`, not a `func`; it maps to Rust free `main → PopChatApp::new/update` and AppDelegate rows above. Resource bundle redirection at `:9` is PLATFORM_REPLACED by Cargo/embedded resources, not a missing chat feature. CLI smoke flows, screenshots, fake actors, failure/cleanup helpers, synthetic files and view-tree probes below are **test references, not 63 product features**. No original harness was executed.

Every row below has status **TEST_HARNESS**, including helpers whose native GUI test has not been ported. Test references mean “associated Rust test exists in source,” never “this native harness passes.” Rust file prefixes: `store` = `src/chat/store.rs`; `client` = `src/chat/client.rs`; `codex` = `src/chat/codex.rs`; `composer` = `src/ui/composer.rs`; `panel` = `src/ui/panel.rs`; `main tests` = `src/main.rs`'s test module.

| Swift line / definition | Harness caller / role | Rust test counterpart or explicit unverified reference |
|---|---|---|
| `149 isBoundary` | `--smoke-typewriter` independent sentence oracle | store `actual_sentence_reveal_waits_for_boundary_and_drains_unclosed_fence_on_finish`; independent oracle not identical. |
| `159 fencesBefore` | same sentence/fence oracle | same store sentence test; no direct Swift helper port required. |
| `245 alpha(at:)` | same fade-band oracle | store `swift_text_reveal_eased_ramp_and_groups_match_source_values`; renderer tests in `ui/reveal.rs`. |
| `297 turn(webSearch:model:)` | `--smoke-codex-app-server-search` real-turn probe | codex `production_adapter_preserves_history_images_effort_search_and_events` uses fake executable, not live search. |
| `774 payload` | `--smoke-update` JSON fixture | update `release_payload_decoding`. |
| `835 line` | `--smoke-reasoning` SSE fixture | client `decode_delta_reads_content_and_both_reasoning_spellings`, `decode_delta_handles_done_and_non_data_lines`. |
| `877 renderedHeight` | same, real fixed thinking-view measurement | reasoning `full_waiting_renderer_has_fixed_three_and_twelve_line_windows_even_at_100k`; headless renderer geometry, not native view metrics/performance. |
| `904 height(attributed:)` | same, reasoning metrics-neutral check | reasoning `settled_is_local_collapsed_formatted_secondary_and_normal_size` compares normal renderer font sizes; not native attributed-string metric equivalence. |
| `950 CheckLog.check` | shared pass/fail tally | Rust test assertions/framework, not product functionality. |
| `959 CheckLog.finish` | shared verdict/exit | Rust test runner, no CLI twin required. |
| `967 composerTextView(in:)` | native composer lookup for GUI harnesses | egui headless input in composer tests; no native view-tree probe. |
| `979 sendKey` | synthetic native keyboard dispatch | composer `ime_candidate_keys_and_commit_frame_do_not_trigger_send`; headless egui, not HWND keyboard dispatch. |
| `995 sendArrow` | native recall/navigation events | composer `recall_older_starts_walks_and_saturates`, `recall_newer_walks_back_to_draft`; reducer tests only. |
| `1005 installBenchmarkConversation` | scratch long-transcript fixture | markdown `actual_assistant_path_prose_quote_lists_and_highlighted_code`; no equivalent complete native scroll benchmark identified. |
| `1045 installFindConversation` | scratch widely separated find hits | main `find_uses_displayed_markdown_unicode_ranges_and_body_excerpts`; no complete native scroll fixture harness. |
| `1075 transcriptScrollView(in:)` | tallest native document selection | egui ScrollArea tests are not an AppKit/Win32 recursive scroller equivalent. |
| `1077 transcriptScrollView.walk` | nested traversal | same, no native harness port identified. |
| `1107 restoreGeometryDefaults` | `--smoke-minsize` cleanup | panel geometry pure tests; original defaults not touched by audit. |
| `1122 clamp(panel:size:)` | same, actual resize delegate probe | panel `compact_user_resize_locks_content_height_and_preserves_saved_expanded_height`; app resize now hooked but no native resize harness run. |
| `1126 findFieldScroll` | same, composer native scroller lookup | no matching native GUI harness. |
| `1133 findDragStrip` | same, header native view lookup | no matching native drag/resize harness. |
| `1140 report(label:)` | same geometry report/assertions | panel `source_defaults_and_low_end_validation_match_panel_controller`, `top_anchored_height_changes_preserve_width_and_expanded_preference`. |
| `1309 focusedEditor` | `--smoke-typing` native editor lookup | composer headless editor tests; no current native typing-cost harness. |
| `1312 focusedEditor.findTextView` | nested traversal | same; original unchanged-transcript measurement invariant unverified. |
| `1392 cleanup` | `--smoke-recall` scratch cleanup | composer/store recall tests use local state/fixtures; no exact CLI harness. |
| `1397 fail` | recall failure reporter | associated recall tests; test framework substitute. |
| `1402 arrow` | recall native ↑/↓ | composer `full_walk_older_then_newer`; reducer-level, not native selection/IME. |
| `1405 type` | recall native text input | composer headless events; no exact native recall harness. |
| `1504 cleanup` | `--smoke-input` scratch/provider cleanup | composer streaming-edit/IME/state-preservation tests / main message-action fixtures; no full native provider-input continuity proof. |
| `1515 fail` | input failure reporter | test framework substitute. |
| `1520 type` | real typing/IME during streaming | composer `streaming_keeps_editor_live_and_stop_does_not_consume_the_next_draft`, `inline_and_expanded_editor_share_cursor_selection_and_undo_state`, IME guard test; app now renders editor during stream, but full native/provider harness not run here. |
| `1721 filterField` | `--smoke-history` focus lookup | main `find_uses_displayed_markdown_unicode_ranges_and_body_excerpts` covers corpus logic, not native focus. |
| `1723 filterField.walk` | nested traversal | no native history-focus harness. |
| `1735 fail` | history failure reporter | no exact history keyboard/teardown end-to-end harness identified. |
| `1797 findField(in:)` | `--smoke-find` native field lookup | main find test covers strings/ranges only. |
| `1799 findField.walk` | nested traversal | no native field lookup counterpart needed for egui, equivalent end-to-end test absent. |
| `1807 fail` | find failure reporter | main find tests; not proof of scrolling/highlighting end-to-end. |
| `1816 highlights(in:)` | native painted range/rectangle inspection | no equivalent full-app find rectangle stepping harness identified. |
| `1818 highlights.walk` | nested traversal | free `paint_find_matches` production code exists; no native tree traversal. |
| `1851 typeQuery` | actual key events query text | composer egui input tests don't establish full find behavior. |
| `1978 fail` | `--smoke-accent` failure reporter | theme `source_hsb_conversion_golden_cases`, `source_bubble_styles_and_retired_style_fallback_are_preserved`; accent state/Hex/Reset and actual popup-drag tests; not original native harness execution. |
| `2077 fail` | `--smoke-attach-caps` failure reporter | store `metadata_capabilities_flow_into_wire_choice`; providers `resolve_capabilities_prefers_exception_per_field`. |
| `2229 fail` | `--smoke-drop` failure reporter | attachment loader tests exist; native file-promise/drop acceptance harness absent. |
| `2233 pump(timeout:until:what:)` | drop runloop completion wait | no promise-receive equivalent harness. |
| `2241 filePromiseProvider(fileNameForType:)` | fake promised-file provider | no Windows virtual-file drag fixture identified. |
| `2244 filePromiseProvider(writePromiseTo:)` | fake provider writes promised file | no matching receive/write drag test identified. |
| `2250 scratchPasteboard` | isolated drop pasteboards | no original isolated-pasteboard equivalent; parent separately verified native CF_HDROP Ctrl+V/real-path OLE on earlier Windows build, with guarded clipboard handling; not virtual-file promise proof. |
| `2384 finish(code:)` | `--smoke-notice` restore prefs/exit | no actual compact notice-height discrimination harness identified. |
| `2389 findDropView` | same, native target lookup | path-drop app caller exists; native target walker not needed. |
| `2404 cleanUpFiles` | same, fake binary decks cleanup | loader fixtures use temporary directories; no exact GUI notice test. |
| `2408 drop(file:)` | same, real drop route | no long-vs-short wrapped notice/window-growth test identified. |
| `2420 settledHeight` | same, native window content height | pure geometry tests are not content-measurement/window-growth proof. |
| `2488 restore` | `--shot` screenshots/defaults cleanup | headless settings/markdown rendering tests; not screenshots of the running native app. |
| `2665 CodexRefreshProbe.inspect` | fake actor for coalescing test | app `codex_task` guard exists; no direct awaitable three-waiter/coalesced-follow-up test identified. |
| `2689 restore` | `--smoke-codex-refresh-coalescing` cleanup | source generation guards/adapter tests, not exact three-caller Swift law. |
| `2740 restore` | `--smoke-providers` cleanup | providers tests + Settings scope tests; no original three-lane keyboard harness. |
| `2749 fail` | provider preview/commit failure reporter | test framework substitute; current popup is different. |
| `2783 captureView` | invisible native switcher responder lookup | egui popup has no equivalent native hidden capture view. |
| `2787 captureView.walk` | nested traversal | no native menu preview harness. |
| `2797 send(keyCode:label:)` | provider-lane keyboard navigation | no exact ↑↓/←→/Enter preview-before-commit test or implementation identified. |
| `2931 restore` | `--smoke-firstrun` marker restore | startup `launch_marker_is_persistent_and_profile_local`, `concurrent_launch_record_has_exactly_one_first_launch`. |
| `2939 check(passed:law:)` | first launch/login/reopen assertions | startup `launch_visibility_matches_first_run_login_and_settings_policy`; instance `secondary_reopens_without_toggling_and_settings_has_priority`. |
| `2943 step(delay:body:)` | first-run GUI sequencing | tray `first_run_hint_uses_actual_registered_shortcut_or_truthful_fallback`; no native balloon/launch/reopen sequence exercised here. |

### Top-level harness/action families (not counted definitions)

Many Swift tests are top-level `if` bodies with no function declaration. Inventorying only helpers would omit these laws. All entries in this table are TEST_HARNESS, not user-feature completion evidence.

| Swift entry | Rust test reference / boundary |
|---|---|
| `:14–18 --smoke / --smoke-search / --smoke-pasteable` | client `run_fetches_real_page_and_resubmits_actual_tool_output`; markdown `actual_assistant_path_renders_table_and_verbatim_card_not_raw_tags`; no real model elicitation test performed. |
| `:86 --smoke-typewriter` | store `actual_reveal_clock_paces_text_and_copy_still_sees_arrived_target`, sentence/fade tests above; actual `main::show_list → paint_reveal` now verified statically. |
| `:269 --check-codex-app-server` | codex `production_inspection_maps_server_models_and_efforts_without_guessing`; fake server. |
| `:295 --smoke-codex-app-server-search` | production adapter test; no live model/search assertion. |
| `:363 --smoke-codex-app-server-streaming` | codex production adapter verifies event delivery before terminal response; fake executable. |
| `:435 --smoke-codex-app-server-timeout` | codex `production_handshake_inactivity_and_inspection_cancellation_kill_child`, `silent_server_yields_timeout_error`. |
| `:469 --smoke-codex-app-server-backpressure` | codex `production_receiver_drop_cancels_backpressured_image_write`. |
| `:514 --chatgpt-login` | developer CLI authentication utility; Settings actual caller now exists; real login unverified. |
| `:527 --smoke-chatgpt` (`--with-search :535`) | responses request/401/tool replay/SSE/real-fetch tests; local HTTP, not subscription validation. |
| `:573 --smoke-persist` | store `reasoning_is_persisted_but_never_sent`, `fork_chain_resolves_and_missing_parent_flags`; main payload integration test. |
| `:616 --smoke-rerun` | main `production_retry_edit_fork_and_load_preserve_payloads_and_branches`. |
| `:689 --smoke-chat-search` | store `full_text_index_excludes_reasoning`, `excerpt_finds_case_insensitive_hit_with_ellipses`; main display/search test. |
| `:759 --smoke-update` | update `numeric_version_comparison`, `release_payload_decoding`, scheduler error/delay tests. |
| `:807 --smoke-bundles` | packaging/fonts/SwiftMath resources; Windows embedded resources are a platform replacement; full packaged-exe deployment not audited. |
| `:833 --smoke-reasoning` | client decode + reasoning fixed 3/12-line/100k renderer, suffix/replacement, pre-append follow, 8s timer, selectable Copy, top-fade and normal-size settled-Markdown tests exist; native metrics/CPU/OS Reduce Motion unverified. |
| `:920 --smoke-file <path>` | `attach.rs` format-specific fixture tests; no user file read. |
| `:1092 --smoke-minsize` | panel geometry pure tests listed above; app resize integration separate. |
| `:1215 --smoke-scroll` | no equivalent long-transcript native scroll/CPU benchmark identified. |
| `:1280 --smoke-typing` | no unchanged-row measurement-count benchmark equivalent; headless composer fit tests are different. |
| `:1379 --smoke-recall` | composer recall-index tests / store prompt-history test; native caret/IME dispatch unverified. |
| `:1477 --smoke-input` (`--live :1497`) | composer real headless streaming text/Stop and shared editor cursor/selection/undo tests; production stream-editor caller now present; native IME with live provider still unverified here. |
| `:1705 --smoke-history` | app real history keyboard/load/delete callers; no native focus/teardown integration harness identified. |
| `:1787 --smoke-find` | app range/hit test; actual glyph scrolling/paint matrix not end-to-end verified. |
| `:1977 --smoke-accent` | theme HSB/contrast/bubble-style fallback + accent gray-hue/black-saturation/dedup/reject-Hex/Reset and headless real-popup drag/submission tests exist; no native harness/visual acceptance claim. |
| `:2076 --smoke-attach-caps` | providers capability precedence + store wire tests; actual error-row exception action present. |
| `:2227 --smoke-drop` | path/byte drop and feedback callers; parent earlier-build native path OLE proof exists. Native promised-file/raw-image/URL-priority matrix remains absent or unverified. |
| `:2357 --smoke-notice [png]` | no matching content-measured short/long error notice height test. |
| `:2468 --shot` (`--dark :2475`, `--light :2477`, `--no-custom :2495`, `--accent :2498`, `--fill :2504`) | deterministic Swift QA rendering; Rust headless settings/render tests cover subsets only. |
| `:2681 --smoke-codex-refresh-coalescing` | app task guard/generation exists; original shared-await/follow-up contract not directly tested. |
| `:2729 --smoke-providers` | catalog mutation/settings scoping tests; no three-lane preview keyboard law. |
| `:2922 --smoke-firstrun` | startup/instance/tray core tests; native lifecycle integration still needs actual Windows validation. |

## SwiftUI properties, bindings and inline action inventory

These are intentionally **outside the 513-function count**. Closely related properties/actions are grouped with explicit source names/lines. Declarative UI behavior cannot be declared ported merely because its store method exists. Framework IDs/colors/state-only convenience properties are included as families rather than inflated into feature counts.

| Original file / properties or inline action family | Current Rust caller / status |
|---|---|
| `AccentPicker.swift` `isSet:23`, `selected:24`, `CustomAccentSwatch.body:26` (select custom/open picker); `wheel:56` | `accent::paint_swatch` ← `AccentPicker::show` ← Settings `appearance_section` WIRED; rainbow ring/custom center/selected outline, click selects existing custom and opens popup, unset open does not publish. Native visual/accessibility comparison remains separate. |
| Same `current:78`, `hueBinding:85`, `saturationBinding:89`, `brightnessBinding:93`, `AccentPickerPopover.body:97`, Reset action `:156`, `GradientSlider.body:229` | `AccentPicker::popover_body/seed/apply/commit_hex_draft/reset` + free `accent::gradient_slider` WIRED live HSB gradient controls, gray-hue/black-slider-saturation retention, dedup, rejected Hex restoration and Reset-to-unset/Blue; Settings `show` autosaves changed accent/custom independently. |
| `AppDelegate.swift` `isPanelOnScreen:15`, `launchedAsLoginItem:80`; status menu update family `:140–147` | visibility state / `--login-item` PLATFORM_REPLACED; conditional tray release item MISSING. |
| `ChatView.swift` `accent:57`, `body:59`, `mainContent:117` (glass/drop overlay/compact-height observation/editor-only content) | `main::show_card/update → chat_layout::bottom_panel/show_body` + `FileDropFeedback::show` PARTIAL; measured bottom-stack/hover feedback wired, native natural-window measurement and glass not equivalent. |
| Same `pillsRow:157`, `modelPill:175`, `actionPill:210`, `switcherLabel:260` | `main::header_row` PARTIAL: real model/effort/history/new/pin actions; no original setup-evidence CTA or three-lane switcher; 46% sizing differs. |
| Same `transcriptZone:278` (scroll pin, restore pre-scroll/group reveal, per-row equivalence, showCaret), `topFade:589` | `main::show_list` PARTIAL; basic auto-follow/find works; restore animation/per-row gate/top mask/caret absent or unproven. |
| Same `findHits:465`; query/index changes `:402–409` | `main::find_hits/show_find/show_list` WIRED; count/paint/scroll boundaries still need full UI validation. |
| Same `MessageRow.textFind:738`, `MessageRow.body:754`, Copy/Fork/context menu `:819–860`, capability buttons `:877–900`, `rerunButtons:927` | `main::show_list/message_action` PARTIAL: Copy/Fork/Retry/Edit/manual exception wired, full-message Copy has 1200ms feedback and row actions hide during active streaming. Original context menu and action-hide-during-finished-reveal semantics still differ/need comparison. |
| Same `ReasoningDisclosure.body:957` | `main::show_list → reasoning::show_disclosure → markdown::show_prose` WIRED; message-local initially collapsed Markdown, normal renderer font sizes/secondary color/leading rule, selectable plain Copy; no native attributed-object or disclosure-animation equivalence claim. |
| Same `WaitingIndicator.body:1020`, `thinking:1080`, `ElapsedLabel.body:1194` | `main::start_stream → WaitingView::start_turn`; `show_list → WaitingView::show` WIRED before body arrival: fixed 3/12-line selectable thinking window, suffix stripping, pre-append 8pt follow rule, stable per-turn 8s timer, 1.6s pulse and top-22% fade. PARTIAL performance/native accessibility: O(n) accumulated comparisons/galley, UI repaints rather than isolated CA pulse, `animation_time == 0` honored but OS Reduce Motion not auto-bridged. |
| `ComposerView.swift` `accent:333`, `body:339`, `composerCard:394`, `editorFooter:473`, `composerField:510` | `Composer::show/show_expanded` ← `main::show_composer_capsule` WIRED: editable during stream, Stop replaces Send, shared TextEdit ID retains cursor/selection/undo, 1–8 inline rows. Native IME/performance limits remain separate. |
| Same `attachButton:486`, `attachCard:675`, chip removal; `AttachmentChip.body:796`, `icon:843` | active `Composer::show_attachments/remove_attachment`, app picker/drop/load WIRED; format-specific chip styling/processing notes not exact. |
| Same `globeButton:497`, persisted web-search toggle | Settings `search_section → main::send_options` PARTIAL; global search setting works, no inline composer globe toggle. |
| Same `completionCandidates:633`, `slashCard:641`, `canSend:768`, `capabilityNotice:736` | `Composer::show`, app warning/send options WIRED for slash/attachment-only/capability warning; stream blocks sending, not typing. Active execution/completion use Unicode lowercasing; exact Foundation folding and strict-whitespace completion edge still differ/need comparison. |
| Same native text bridge callbacks `onHeightChange:125`, `onReturn:127`, `onMoveUp:128`, `onMoveDown:129`, `onTab:130`, `onEscape:131`; marked text observers `:104–111` | egui TextEdit/input dispatch PLATFORM_REPLACED; shared-mode undo/caret/selection tested in source; native IME and no-transcript-remeasure performance contracts unverified here. |
| `FirstRunHint.swift` `shortcutDescription:23`, `isShowing:42`, hint `body:55` | own-tray notification + real 350ms app caller PLATFORM_REPLACED; Windows shell balloon visibility/lifetime unlike anchored macOS transient popover. |
| `FloatingPanel.swift` `canBecomeKey:43`, `canBecomeMain:44`; panel style/focus init | free `main` + visibility native focus PARTIAL; best-effort foreground/return-to-previous-window, not nonactivating AppKit semantics. |
| `HistoryPopover.swift` `Result.id:50`, `body:69`, `filterRow:106`, `list:129` | `main::show_history` WIRED load/delete/filter/keyboard; embedded page rather than popover focus window. |
| Same `shellAnimation:171`, `visible:351`, `ListItem.id:360`, `items:368`, `listHeight:394`; close/cascade action timing | `history_results/show_history` PARTIAL; data list actual, source stagger/flash/relative grouping/height treatment differs. |
| `MarkdownText.swift` `Segment.isProse:22`, `SelectableText.displayed:584`, `AssistantMessageView.body:713` | segmentation/renderer → actual glyph fade → find painting WIRED; selection uses correct source islands, not a blanket cross-widget-selection gap. Bitmap attachment/rich clipboard/caret limits remain. |
| Same `BlinkingCaret.body:792` and steady prose caret `:725–734` | MISSING active streaming caret; no renderer showCaret input at snapshot. |
| Same `CodeBlockView.body:814` Copy feedback `:825–829`, `QuoteBlockView.body:861`, `TableBlockView.body:878` | `markdown::show_code_block/show_quote/show_table` WIRED; independent block/cell selection is intentional source behavior, covered by real drag/Copy fixture paths. Exact original Splash Swift lexer/theme/cache behavior remains partial, not an absent multi-language catalog. |
| Same `PasteableBlockView.accent:938`, `body:940`, verbatim Copy feedback `:954–958`; `MathBlockView.body:992` raw fallback | `show_pasteable/copy_button` and `MathTextures::show_display_math` WIRED; true math layout exists, full SwiftMath language coverage not claimed. |
| `PanelController.swift` `expandedHeight:31`, `panelWidth:43`, `minContentSize:52`, `isPanelOnScreen:118`, `isPinned:129`, `reduceMotion:155`, `savedTopLeft:255` | `PanelGeometry` + visibility + `ViewportSizing` + `main::update_window_geometry` WIRED geometry state; natural-content sizing/animations/Reduce Motion partial or missing; native matrix unverified here. |
| `PanelState.swift` `pinned:7`, `focusBump:11` | app pin and visibility focus-request WIRED; these aren't explicit initializer definitions. |
| `ProviderSwitcher.swift` `accent:66`, `rail:67`, `previewProvider:68`, `models:69`, `previewModel:72`, `efforts:75`, `switcherWidth:80`, `body:86` | app `header_row` PARTIAL; real configured menu/catalog/effort data, no preview-state/lane layout. |
| Same `providerRail:136`, `addProviderRow:191`, `modelColumn:199`, `modelHeaderTitle:307`, `effortColumn:315`, `footer:421`, `bodyHeight:454`, `shellAnimation:464` | nested popup + manage Settings action PARTIAL; no original add-provider/editor shortcut rows, preview metadata layout or shell animation. |
| Same `keyCapture:507`, `CaptureView.acceptsFirstResponder:601`, provider/model/effort hover/onChange preview families `:86–131` | egui menu navigation PARTIAL; no explicit inert arrow preview and Enter commit contract. |
| `Reveal.swift` `TextReveal.isEmpty:34` and `Stop` lengths/alpha/trailingSkip state | store reveal clock/`reveal_for → paint_reveal` WIRED actual app shape-range consumer; bitmap meshes remain solid, active caret not supplied by app. |
| `SearchField.swift` callbacks `onMoveUp:21`, `onMoveDown:22`, `onReturn:24`, `onEscape:25`; native focus/key-command family | app `show_find/show_history/poll_events` PLATFORM_REPLACED; search-field IME interaction requires native verification. |
| `SettingsView.swift` Tab `id:11`, `label:12`, `icon:21`; `body:90`, `tabBar:110`, `generalTab:139` | `SettingsUi::show → persist_general_preferences` WIRED immediate general-pref/font/window/search-key saves; Commands/Codex-path save independently, API catalog drafts retain explicit Save. Hotkey draft has separate Apply and success-only commit. Embedded/tab presentation remains different. |
| Same login toggle `:142–155`, `updateRow:198` / automatic checks `:164`, `hotkeyTab:1110` | `SettingsUi::hotkey_section`, `startup::set_enabled`, scheduler PLATFORM_REPLACED/WIRED; egui recorder → draft/Apply → `ChangeHotkey → main::settings_actions → set_binding → commit_hotkey_binding` only after OS success; default Alt+Down, Alt+Space rejected. New recording has offline interception/suppression proof, not a full native acceptance matrix. |
| Same `appearanceRow:236`, `accentRow:252`, `liquidGlassRow:291`, `panelTintRow:305`, `previewRow:330`, bubble-style picker `:173`, streaming picker `:181` | `appearance_section → AccentPicker::show`, `theme::apply`, `main::show_list → AccentPalette::for_bubble_style` WIRED appearance/HSB accent/bubble style/stream-mode with immediate independent autosave. PARTIAL: glass/panel tint/Reduce Transparency and dedicated live bubble preview absent. |
| Same `providersTab:360`, `formPageBackground:427`, `formCardBackground:431`, `addProviderRow:495` | Settings provider pane + app actions WIRED for custom/template CRUD; no original accordion/readiness-dot visual contract. |
| Same `chatGPTAuthRows:784`, `codexAppServerRows:876` | Settings auth/Codex controls → app actions → native token/inspection/backend WIRED; native Locate picker/path changes independently autosave/invalidate actual backend; account/provider services not live-verified. |
| Same `webSearchTab:969` engine/key changes | `search_section/show → persist_general_preferences` saves engine/toggle/native secret independently; `search_key_for_send → main::send_options → actual web executor` WIRED; no inline composer globe. |
| Same `commandsTab:999` system prompt reset/edit/explicit empty prompt; shortcut names/templates/add/delete/focus | `commands_section → save_system_prompt_only/ShortcutStore::save_entries → CommandsChanged` WIRED in current source; autosave errors exposed. |
| `Shortcuts.swift` static `.togglePopChat:6` / Settings recorder family | `HotkeyManager` + `hotkey_recorder` PLATFORM_REPLACED; physical-key recorder with local/global suppression → explicit Apply, default Alt+Down, conflict retains old binding; Alt+Space blocked. Not the macOS native recorder widget. |
| `Theme.swift` BubbleStyle `id:16/label:17`, StreamingMode `id:33/label:34`, AppearanceChoice `id:49/label:50/nsAppearance:58`, `currentAccent:85` | Rust enums/theme resolution + `appearance_section` selectors WIRED; BubbleStyle `accentTint/accentFill` persists and actual user row consumes it, retired `quietGray`/unknown style falls back to tint. Platform labels/appearance APIs differ. |
| Same `PanelGlassBackground.body:277`, `solidFill:312`, `liquidGlass/panelTint/reduceTransparency:272–275` | `theme::panel_fill → surface::card_frame_with_opacity` ← `main::show_card` PARTIAL; configurable backdrop alpha only, no native glass/colored tint/Reduce Transparency bridge. |
| `UpdateChecker.swift` `automaticChecksEnabled:29`, `availableRelease:56` | Settings automatic flag/scheduler/link WIRED; original tray available-release consumer still absent. |

### Non-UI computed accessor families (not explicit functions)

| Source accessors | Current Rust route / limit |
|---|---|
| `Chat/Attachment.swift` synthesized Attachment/Content/NoteKind/AttachError Codable/init; none declared explicitly | active `store::Attachment` serde + loader conversion; synthesized methods not added to count. |
| `Chat/ChatGPTAuth.swift` `AuthError.errorDescription:69`, `isSignedIn:116`, `accountEmail:117`, `planLabel:118` | error Display/native-token load → app state/settings; WIRED, platform token representation replaced. |
| `Chat/ChatStore.swift` `streamTargetLength:94`, `webSearchEnabled:167`, `systemPrompt:194`, `lastUserIndex:771`, `canRerunLastTurn:776` | store/app settings and rerun gates WIRED; stream length convenience method has no app consumer and counts scalars, unlike source Character count. |
| `Chat/CodexAppServerClient.swift` `ClientError.errorDescription:47`, `ItemAssembly.snapshot:680`, JSON accessors `stringValue:890/boolValue:891/objectValue:892/arrayValue:893/intValue:894`, cancellation `isStopped:930`, watchdog `didTimeOut:966` | typed Codex errors/ItemAssembly snapshot/serde Value/explicit integer check/task closure/timeouts WIRED or PLATFORM_REPLACED; no separate shared Swift timer/cancellation object. |
| `Chat/PromptHistory.swift` `entries:16`; `Chat/ProviderStore.swift` template `id:76`, `chatGPTProviderNames:379`, `selectedProvider:425`, `configuredProviders:465`, `currentModel:485`, `currentReasoningEffort:490` | active store/provider accessors → app initialization/composer/popup/send WIRED; template IDs/platform storage differ. |
| `Chat/WebTools.swift` SearchEngineChoice `id:10/label:12/apiKeyAccount:22` | `SearchEngine` + Settings engine labels/native secret account helper WIRED; actual executor consumes selected key. |

## Remaining gaps and verification boundary

### Resolved findings and corrected source assumptions

- Streaming editor is no longer hidden: actual app calls `Composer::show` with streaming state; editable draft/shared cursor-selection-undo/Stop behavior have direct headless widget tests.
- Glyph/sentence fade is no longer core-only: app paints the assistant body's exclusive shape range after renderer and before Find. Geometry/resize, hidden-instance requests and independent preference saves also have actual callers listed above.
- **Intentional selection islands are not a missing cross-block feature.** Real renderer drag/Copy tests cover prose paragraphs/formatting/links and original code/quote/table-cell/pasteable boundaries. The remaining bitmap/rich-clipboard distinction is narrower.
- **Empty-only recall is source behavior**, not a port defect: original `recallOlder/recallNewer` starts empty and restores empty. The separate Rust history API's arbitrary-draft restoration should not invent a stronger UI requirement.
- **HSB and bubble styles are no longer missing:** actual Settings popup/gradient/Hex/Reset callers preserve gray hue, black-slider saturation and deduplicated publication; independent live autosave preserves API drafts. User rows consume tint (15% light/19% dark) or contrast-aware fill; retired `quietGray` falls back to tint.
- **Reasoning is no longer a generic label:** app starts/resets `WaitingView` per turn and renders fixed selectable 3/12-line thinking with suffix-only stripping, pre-append follow, 8s timer, pulse/top fade; settled disclosure renders normal-size secondary Markdown. Headless tests exist for these contracts, but neither incremental native layout performance nor native visual/accessibility acceptance is established.
- **Slash matching is not ASCII-only:** both active store execution and completion use Unicode lowercasing. This is not proof of all Foundation case-folding/locale rules.
- **Splash is not a multi-language catalog requirement here:** Swift source calls its default Swift highlighter without a fence-language argument. Exact Swift lexing, palette, spacing and cache behavior—not an invented language catalog—is the remaining comparison.
- **File routing/hover are no longer absent:** root-window paste observation reaches native CF_HDROP probing and the active loader, while path/byte drops and paint-only hover/loading/result feedback have app callers. Parent's earlier-build native file-only Ctrl+V and real visible-source OLE Copy/chip proofs are recorded in `PORT_PROGRESS.md`; hidden-source None was not a pass, and the newer badge anchor has no native proof yet.
- **Six latest requests are wired, not yet natively accepted:** measured footer layout, separate English/CJK preferences, configurable default Alt+Down, backdrop-only opacity, header/top-band drag and six startup/new-SHOW placements. Font/window autosave preserves API drafts; hotkey draft is excluded and commits only after OS registration. Parent is running latest fmt/check/all-target validation; no new result count or native six-feature pass is asserted here.

### Remaining meaningful gaps / acceptance checks

1. **Provider preview/setup UI:** nested menu still replaces the original inert provider→model→effort keyboard preview and Enter commit. `has_setup_evidence` still has no app caller, header omits original setup CTA, and Settings readiness-dot consistency is not implemented.
2. **Reasoning performance/accessibility/animation:** suffix-only stripping and fixed viewport are wired, but equality/prefix comparisons plus accumulated galley clone/hash/layout still cost O(n) and can yield quadratic work over a growing turn. It is not NSTextStorage suffix-layout parity. Pulse uses bounded 30Hz UI repaints, not isolated native CA animation; timer repaint is not a proven leaf-only update. Zero egui animation time disables pulse, but Windows Reduce Motion is not automatically read/bridged; native disclosure/expand animation, font metrics and long-trace CPU acceptance remain unverified.
3. **Natural compact sizing and layout cost:** `chat_layout::bottom_panel/show_body` now measures/bounds the real bottom stack instead of a fixed 92pt reserve, keeping the footer reachable even when contents overflow. `Composer::inline_rows` still counts explicit newlines (clamped to 8), not visual wraps; `main::update_window_geometry` still estimates native window height. Native bounded footer checks at 604×344/644 passed, not original natural-window growth/unchanged-transcript CPU parity.
4. **Caret/restore/animation:** current `MathTextures::show_assistant_message(ui,text)` receives no active caret flag; the fade module's trailing-caret support tests do not create a production caret. `restore_tick/restored_into_empty_panel` remain unconsumed by app restore animation. Incoming page/settings-pane and mode-height motion now honor Windows animation policy; source row/show/hide/within-mode growth, remaining animation policy bridges, transcript top mask and new-messages jump pill remain missing/partial. The reasoning-scroller top fade is not the transcript mask.
5. **Bitmap/rich text clipboard:** text selection can cross neighboring labels around inline math, but copies no selectable NSTextAttachment/object/rich clipboard payload for the bitmap. Math meshes also stay solid during text fade. Full SwiftMath compatibility, arbitrary PDF/document extraction/HEIC/HEIF and exact original Splash Swift lexing/theme/cache behavior remain partial or unverified.
6. **Other source-specific affordances:** native glass/colored panel tint/Reduce Transparency and dedicated live bubble preview, header-wheel forwarding, exact panel-wide clipboard/format precedence, virtual-file promises/Windows FileDescriptor and raw OS image-drop conversion matrix, composer globe toggle, conditional tray update entry; retained separate native Settings/blur behavior also differs. Hover feedback and CF_HDROP routing are implemented, not remaining missing features. History cascade/flash/relative labels, source-specific context menus, attachment payload-size warning and Settings capability/source-label presentation remain partial/missing as recorded in their rows.
7. **Native/service acceptance, not unwired features:** latest same-build footer/font/rebinding/opacity/drag/Fixed-and-Remember checks have bounded native evidence in progress notes; the other placement modes and full work-area/DPI/persistence, tray balloon lifetime, Settings forwarding, native IME/foreground restoration and packaged resources still need their own Windows matrices. Best-effort Win32 focus is **not nonactivating NSPanel equivalence**. No real provider, subscription, browser login or installed Codex account was verified here.
8. **Semantic/data edges:** active slash execution/completion use Unicode lowercasing, but exact Foundation case folding/locale behavior needs comparison; completion rejects all whitespace whereas source checks literal space. Foundation search folding/context versus Rust Unicode/grapheme behavior, Find count-to-glyph alignment for split/table shapes, Codex partial-line watchdog timing/shared-await refresh semantics, cross-version data migration and silent conversation persistence failures remain explicit checks. The detailed PARTIAL/CORE_ONLY/MISSING rows above remain authoritative; this summary does not remove any other gap.

### Explicitly unverified modules and paths

- **Live services:** `client.rs`, `web.rs`, `responses.rs`, `chatgpt_auth.rs`, `codex.rs`, model discovery in `providers.rs/settings.rs`. Local fixtures/mock SSE/fake app-server references are not real provider/account/browser/search validation.
- **Native integration:** main/visibility/tray/hotkey/font/renderer modules plus `update.rs`, `updater.rs`, GUI `bin/popchat_updater.rs`. Sole chat app **PID 324**, installed **0.2.0 LOCAL BUILD**, formal profile with 2 custom services. Update-only data/hash/boot/GUI guards checked. Hover, English fallback and opt-in in-app installation wired; true release ZIP staged/validated without invoking installation. Real headless callback/renderer/confirmation/IME and isolated transaction tests, not complete native update/recording/type/motion/Bopomofo/live-account acceptance. Final window off-screen; no forced input, Send/API Save/native Copy; preserve data/clipboard.
- **User data/durability:** `chat/store.rs`, `paths.rs`, auth/secret migration/chunks. Only code read; no real history or preferences loaded. Silent conversation write failures and cross-version schema behavior need isolated tests and intentional user migration validation.
- **Rendering:** `ui/{markdown,math,reveal,theme,fonts,accent,reasoning}.rs`. Real renderer drag/Copy, accent popup/state and reasoning viewport/font/Copy tests exist in source; their latest execution results are not supplied by this audit. App fade/accent/bubble/reasoning callers are present. These do not establish bitmap attachment/rich clipboard, packaged fonts/all math glyphs, native metrics/accessibility, O(n)-free incremental layout or visual/CPU equivalence.
- **Parallel/non-authoritative implementations:** `persistence.rs` conversation/config types, `ui/attachments.rs::AttachmentDraft`, `commands.rs` execution helpers, `web::run_with_tools`, public auth wrapper helpers, provider store-fetch wrappers. They may have useful tests, but active callers are separately identified above. Divergent duplicate model/index/command implementations must not be silently treated as the production path.
- **Updates/packaging:** `update.rs` scheduler is actually called, but release endpoint/network notification/packaged Windows lifecycle not exercised. Original updater intentionally only opens a release page; automatic exe replacement is **not** an original requirement and its stub is not counted as a missing feature.

### Inventory reconciliation method

Declaration starts were enumerated from all `*.swift` files, including declaration modifiers/attributes and operator names, using this search pattern:

```regex
^\s*(?:(?:@\w+(?:\([^)]*\))?|private|fileprivate|public|internal|static|class|override|required|convenience|nonisolated|mutating|final)\s+)*(?:func\s+\S+|init\??\s*\()
```

Candidate lines were checked against source context: exclude the Markdown-fixture `fib` at `main.swift:1016`; include the actual operator `MessageRow.==` at `ChatView.swift:742`. A read-only PowerShell reconciliation was rerun after this refresh, comparing each file's declaration-start line set to this document's per-file function rows: **32 files, 513 declarations, 513 rows; no missing, extra, duplicate or heading-count mismatches**. The ordered file/Swift-line/definition identity checksum is unchanged from before these edits, so no original inventory row was removed or renamed. This validates inventory coverage, not semantic parity or Rust test outcomes.

Inventory reconciliation: **32 files; 513 definitions; 450 non-`main.swift` definitions + 63 TEST_HARNESS helpers.** UI/action families and top-level smoke cases are additional audit surfaces, not completion percentages. No completion percentage is asserted.
