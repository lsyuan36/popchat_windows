# Review and UI validation — 2026-10-28

> This is a record of local fixes and UI checks, **not proof of a complete
> function-by-function port**. The later [static parity audit](PARITY_AUDIT.md)
> identifies substantial missing implementations and disconnected runtime paths.

## Correctness fixes

- Visibility changes and viewport commands now share one serialized controller.
  A stale UI dismissal cannot hide a newer hotkey activation.
- Every show resets focus history; the 600 ms dismissal timer starts on the
  actual focus-loss transition. Hidden frames do not arm dismissal.
- Windows activation uses `SetForegroundWindow` directly, avoiding winit's
  synthetic Alt release. Only the requested Alt+Down is registered.
- Logging no longer writes to stdout: an undrained pipe could block the UI or
  visibility driver even when writes did not panic.
- Windows-native keyring is enabled. Migration verifies persistent writes,
  retains original data on failure, and never overwrites a newer native key.
- Settings edit real provider UUIDs rather than relabeling the active backend.
  Credentials are scoped to provider identity; endpoint-origin changes clear
  inherited credentials. Explicit key deletion and persistence errors are handled.
- Model requests are cancellable and generation-scoped, so stale responses cannot
  populate a different service's settings.
- Prompt-history and logger tests use isolated destinations rather than changing
  the user's real files.

## UI changes

- Microsoft JhengHei CJK fallback with runtime glyph checks.
- Font-independent vector buttons with hover labels.
- Truncated, clickable provider pill and a drag region that excludes buttons.
- Settings occupy an expanded native viewport, with scrollable grouped sections
  and a persistent Save/status row.
- Adaptive multiline composer, visible long-editor control, and Esc returning
  from the editor without hiding the panel or losing the draft.
- Custom accent parsing rejects invalid Unicode input safely.

## Reproduce safely

Use a fresh directory for `POPCHAT_DATA_DIR` to isolate settings, logs, provider
state and conversations. Do not copy real keys into a test profile. Credential
Manager still uses the provider UUID as its account; fresh profiles have fresh
UUIDs. Do not run two app instances concurrently when testing Alt+Down.

```powershell
cargo test --locked --all-targets
cargo test --locked --lib chat::store::secret_tests::windows_native_credential_round_trip -- --ignored --exact
cargo check --locked --all-targets
cargo build --locked --release

$env:POPCHAT_DATA_DIR = Join-Path $env:LOCALAPPDATA 'Temp\opencode\popchat-ui-review-profile'
.\target\release\popchat_windows.exe
```

The ignored native-credential test uses a unique disposable account and deletes
it afterward. Other credential tests inject an isolated in-memory backend.

Desktop checks performed without pinning: quick repeated Alt+Down; holding left
Alt while tapping Down eight times; 200 ms/1100 ms focus-loss observations;
switching and saving OpenAI while verifying `openAICompatible` routing; editor
Esc; and screenshots of the compact, settings, and expanded-editor views.

Final results: 132 library tests and 9 binary tests passed; the one normally
ignored Windows-native credential round-trip was also run separately and passed.
The release executable received 20/20 toggles at 100 ms intervals and 8/8 toggles
while left Alt remained down. Light/dark screenshots were checked. The isolated
desktop test instances were closed gracefully after validation.

## Remaining limitations

Real API chat was not exercised without user-supplied credentials. ChatGPT/Codex
login controls, PDF extraction, attachment-selection UI and installer packaging
are not completed by this review; settings identifies unavailable login controls
instead of presenting them as working API-key configurations.
