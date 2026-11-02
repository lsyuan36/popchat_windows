# Windows build and per-user installation

The Rust application installs to `%LOCALAPPDATA%\Programs\PopChat`, without
administrator access. Installation creates desktop and Start menu shortcuts and
an entry in Windows Installed apps. It does **not** enable login startup.

## Install an existing custom-service profile

This workflow explicitly imports **only custom providers** into the normal
`%APPDATA%\PopChat\PopChat\config` profile. It does not replace general
preferences, history, the live provider selection, OAuth credentials, or search
credentials. Source files and source credentials remain intact.

```powershell
# Read-only installation plan (no build, credential access, shutdown, or install):
.\scripts\install.ps1 -SourceProfile 'C:\absolute\existing-profile' -ExpectedPid 1234

# Validate, build, close the sole expected app normally, import, install and open:
.\scripts\install.ps1 -SourceProfile 'C:\absolute\existing-profile' -ExpectedPid 1234 -Apply
```

Use the actual sole running PopChat PID, or `-ExpectedPid 0` when no PopChat is
running. A changed PID, multiple instances, conflicting shortcut/install path,
malformed provider file, or conflicting provider UUID stops installation.
The current recovery workflow expects two custom providers in the source.
No unsaved API or chat draft is automatically saved: finish your edits first.

Builds use the existing `target/button-review` staging directory so the running
development executable is not overwritten. `-SkipBuild` is for an already
validated application **and** importer release build.

## Update an installed app without importing data again

```powershell
.\scripts\install.ps1 -UpdateOnly -ExpectedPid 1234 -Apply
```

Use `-ExpectedPid 0` if the app is already closed. This mode does not run the
importer, read/copy credentials, or access an old source profile. It verifies
that settings, provider data, commands and history remain unchanged before
launching the updated app. No `-SourceProfile` is accepted in update-only mode.
The installer also checks the application PE header for Windows GUI subsystem
`2`, preventing an accidental Console build from opening a command window on
normal desktop/Start menu launches. Test binaries and the installation helper
remain console programs for their intended command-line use.

The source-tree installer also installs `popchat_updater.exe`, checks both GUI
executables before closing the current app, and records their hashes in the
installation manifest. Release documentation and third-party notices are copied
with the binaries. It does not run the in-app updater during development updates.

## Public ZIP and in-app updates

New users can download `PopChat-Windows-x64.zip` from this repository's Releases,
extract all files to a writable directory, and run `popchat_windows.exe`.
Keep the adjacent `popchat_updater.exe` and `licenses` directory. No developer
profile, custom-provider import or administrator elevation is required.

The application checks **`lsyuan36/popchat_windows`**, not `lec77/PopChat`, for
newer stable releases. A download must match the GitHub asset SHA-256 and the
bounded Windows x64 package contract. Installing requires a separate user
confirmation; the Rust helper validates the staged files and target before the
app closes, waits for the old process to exit, then installs and restarts it.
Settings, providers, credentials and histories are not migrated or replaced.
Backups are retained; integrity hashes are not Authenticode signatures.

## Import safety

`examples/import_custom_providers.rs` is a Rust command-line helper, not a UI.
It has a read-only default mode. The installer invokes its `--apply` mode only
after the old UI exits and a zero-instance check succeeds.

- Preserve the destination JSON's unrelated fields and provider selection.
- Merge custom-provider metadata and that provider's model/capability maps.
- Copy only those provider UUID credentials between the old profile namespace
  and the default Windows Credential Manager namespace, inside the process.
- Never print/export plaintext keys, delete source keys, or overwrite different
  existing target keys. A different target value is a conflict, not permission
  to replace it.
- Keep a uniquely named, same-directory backup of the destination provider
  JSON before commit; reject concurrent source/destination changes.
- Verify credential writes; on failure, attempt rollback of only newly created
  destination credentials. This is not a cross-resource filesystem/keyring
  transaction. Original data and the JSON backup remain available for recovery.

After installation, start PopChat using the installed shortcut or executable,
not the old temporary-profile launch scripts. The installed process uses the
normal profile without `POPCHAT_DATA_DIR`; do not set that variable globally.

## Validation and removal

```powershell
cargo fmt --all -- --check
cargo check --locked --all-targets
cargo test --locked --all-targets --quiet
```

Font Awesome assets and attribution are included under `licenses/font-awesome`.
Swei Gothic TC Regular 2.143 is embedded in the executable as the Chinese default;
it requires no font installation or runtime download. Its original OFL 1.1,
copyright and pinned-source notices are installed under `licenses/swei-gothic`
(source asset provenance: `assets/fonts/swei-gothic/SOURCE.md`). Existing manual
English/Chinese font selections are retained. The English default prefers installed
upright regular TT Norms Pro, then Arial; neither font is bundled or downloaded.
If both are unavailable, egui's embedded fonts remain the last-resort fallback.
Restart PopChat after installing a new default font. The application is not code-signed.

Appearance settings include a 12–24 pt chat-text size control (default 15 pt),
applied to messages, Markdown/code/tables/math, reasoning and the composer rather
than the entire interface. Size saves independently of unsaved API drafts.
The hotkey page records a key combination, then requires explicit Apply; recording
does not change the registered hotkey. Escape cancels recording, and registration
conflicts leave the previous binding active. Chat shortcuts are a read-only list.

Uninstall through Windows Installed apps after closing PopChat. Uninstall removes
the recognized application executable, shortcuts and registration, while retaining
all profiles, Windows credentials, license files and any executable backups.
