//! Update checking, ported from `UpdateChecker.swift`.
//!
//! "Is there a newer PopChat?" — nothing more. This never downloads,
//! replaces, or relaunches anything; it reports a version and links the
//! release page, and the user decides.
//!
//! - Checked at most once every [`CHECK_INTERVAL_SECS`] (24 h); a menu-bar
//!   app can run for weeks, so this is a real interval, not launch-only.
//! - Failures from a scheduled check are silent (`Ok(None)`); the manual
//!   "Check now" path propagates the error so Settings can show it.
//! - Version comparison is numeric per component, so `0.1.10` beats `0.1.9`.
//!
//! The `self_update` crate is a dependency but intentionally unwired: when
//! auto-update is ever wanted, implement [`apply_self_update`] with it. The
//! stub below keeps that seam visible.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// The releases API for this app's own repository.
pub const LATEST_RELEASE_URL: &str = "https://api.github.com/repos/lec77/PopChat/releases/latest";
/// How long a check stays good.
pub const CHECK_INTERVAL_SECS: i64 = 24 * 60 * 60;

/// The update to point at, if any — `{version, page_url}`, no download.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseInfo {
    pub version: String,
    pub page_url: String,
}

/// What this build calls itself (the `Cargo.toml` version, matching the
/// `v…` release tag the same way `CFBundleShortVersionString` did).
pub fn current_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

// ---------------------------------------------------------------------------
// Version comparison
// ---------------------------------------------------------------------------

/// Numeric component comparison: `0.1.10` beats `0.1.9`. A leading `v` is
/// stripped. Everything from the first non-numeric (non-`.`) character is
/// dropped BEFORE splitting, so `0.2.0-beta.1` reduces to `[0, 2, 0]` and
/// ties with `0.2.0` — a prerelease never reads as newer than its release.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    let left = version_components(candidate);
    let right = version_components(current);
    for index in 0..left.len().max(right.len()) {
        let a = left.get(index).copied().unwrap_or(0);
        let b = right.get(index).copied().unwrap_or(0);
        if a != b {
            return a > b;
        }
    }
    false
}

fn version_components(version: &str) -> Vec<u64> {
    let stripped = version.strip_prefix('v').unwrap_or(version);
    let numeric_prefix: String = stripped
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    numeric_prefix
        .split('.')
        .filter(|s| !s.is_empty())
        .map(|s| s.parse::<u64>().unwrap_or(0))
        .collect()
}

// ---------------------------------------------------------------------------
// Payload decoding
// ---------------------------------------------------------------------------

/// Decode the `/releases/latest` payload. That endpoint already excludes
/// drafts and prereleases, but the flags are checked anyway — a repo can
/// serve a prerelease as "latest" if it is the only release.
pub fn decode_release(json: &serde_json::Value) -> Option<ReleaseInfo> {
    if json.get("draft").and_then(|v| v.as_bool()) == Some(true) {
        return None;
    }
    if json.get("prerelease").and_then(|v| v.as_bool()) == Some(true) {
        return None;
    }
    let tag = json.get("tag_name")?.as_str()?;
    let page = json.get("html_url")?.as_str()?;
    let version = tag.strip_prefix('v').unwrap_or(tag);
    if version.is_empty() || page.is_empty() {
        return None;
    }
    Some(ReleaseInfo {
        version: version.to_string(),
        page_url: page.to_string(),
    })
}

// ---------------------------------------------------------------------------
// Fetching
// ---------------------------------------------------------------------------

fn client() -> Result<reqwest::Client, anyhow::Error> {
    Ok(reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .user_agent(format!("PopChat/{}", current_version()))
        .build()?)
}

/// Fetch the latest release. `Ok(None)` means "no releases published yet"
/// (HTTP 404) — not an update, not an error.
pub async fn fetch_latest() -> anyhow::Result<Option<ReleaseInfo>> {
    let response = client()?
        .get(LATEST_RELEASE_URL)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await?;
    match response.status().as_u16() {
        200 => {
            let json: serde_json::Value = response.json().await?;
            Ok(decode_release(&json))
        }
        403 | 429 => anyhow::bail!("GitHub is rate-limiting update checks — try again later."),
        404 => Ok(None),
        code => anyhow::bail!("GitHub returned HTTP {code}."),
    }
}

/// The manual "Check now" path: always runs, reports up-to-date vs.
/// available, and propagates failures so the caller can show them.
pub async fn check_for_update(current: &str) -> anyhow::Result<Option<ReleaseInfo>> {
    let release = fetch_latest().await?;
    // Both manual and scheduled successful checks reset the same clock.
    write_last_check(&Utc::now());
    match release {
        Some(release) if is_newer(&release.version, current) => Ok(Some(release)),
        _ => Ok(None),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum CheckState {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available(ReleaseInfo),
    Failed(String),
}

/// Application-owned scheduler matching UpdateChecker.swift's state machine.
/// Polling is cheap; HTTP work runs off the UI thread and is cancelled on drop.
pub struct UpdateChecker {
    pub state: CheckState,
    next_poll: std::time::Instant,
    pending: Option<(
        std::sync::mpsc::Receiver<Result<Option<ReleaseInfo>, String>>,
        tokio::task::AbortHandle,
        bool,
    )>,
}

impl Default for UpdateChecker {
    fn default() -> Self {
        let now = std::time::Instant::now();
        Self {
            state: CheckState::Idle,
            next_poll: now + std::time::Duration::from_secs(5),
            pending: None,
        }
    }
}

impl Drop for UpdateChecker {
    fn drop(&mut self) {
        if let Some((_, task, _)) = &self.pending {
            task.abort();
        }
    }
}

impl UpdateChecker {
    pub fn poll(&mut self, ctx: &egui::Context, runtime: &tokio::runtime::Handle, automatic: bool) {
        if let Some((rx, _, user_initiated)) = &self.pending {
            let result = match rx.try_recv() {
                Ok(result) => Some(result),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    Some(Err("更新檢查已中斷，請重試。".to_owned()))
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
            };
            if let Some(result) = result {
                let user_initiated = *user_initiated;
                self.pending = None;
                self.complete(result, user_initiated);
            }
        }
        let now = std::time::Instant::now();
        if now >= self.next_poll {
            self.next_poll = now + std::time::Duration::from_secs(CHECK_INTERVAL_SECS as u64);
            let due = read_last_check()
                .is_none_or(|last| (Utc::now() - last).num_seconds() >= CHECK_INTERVAL_SECS);
            if automatic && due {
                self.check(ctx, runtime, false);
            }
        }
        if self.pending.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        // Keep the app's scheduling alive even when the panel is offscreen.
        ctx.request_repaint_after(self.next_poll.saturating_duration_since(now));
    }

    pub fn check(
        &mut self,
        ctx: &egui::Context,
        runtime: &tokio::runtime::Handle,
        user_initiated: bool,
    ) {
        if self.pending.is_some() {
            return;
        }
        self.state = CheckState::Checking;
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        let task = runtime.spawn(async move {
            let result = check_for_update(env!("CARGO_PKG_VERSION"))
                .await
                .map_err(|error| format!("{error:#}"));
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        self.pending = Some((rx, task.abort_handle(), user_initiated));
    }

    fn complete(&mut self, result: Result<Option<ReleaseInfo>, String>, user_initiated: bool) {
        self.state = match result {
            Ok(Some(release)) => CheckState::Available(release),
            Ok(None) => CheckState::UpToDate,
            Err(error) if user_initiated => CheckState::Failed(error),
            Err(_) => CheckState::Idle,
        };
    }
}

// ---------------------------------------------------------------------------
// Scheduling (once a day)
// ---------------------------------------------------------------------------

fn last_check_path() -> PathBuf {
    let dir = crate::persistence::app_dir();
    std::fs::create_dir_all(&dir).ok();
    dir.join("last_update_check.txt")
}

fn read_last_check() -> Option<DateTime<Utc>> {
    let text = std::fs::read_to_string(last_check_path()).ok()?;
    text.trim().parse::<DateTime<Utc>>().ok()
}

fn write_last_check(now: &DateTime<Utc>) {
    std::fs::write(last_check_path(), now.to_rfc3339()).ok();
}

/// Background check: runs at most once a day, silent about failures (an
/// offline laptop must not produce anything the user has to dismiss).
/// Returns the available release, if any.
pub async fn check_if_due(current: &str) -> Option<ReleaseInfo> {
    let now = Utc::now();
    if let Some(last) = read_last_check() {
        if (now - last).num_seconds() < CHECK_INTERVAL_SECS {
            return None;
        }
    }
    match check_for_update(current).await {
        Ok(release) => {
            // Any successful check (even "up to date") resets the clock.
            write_last_check(&now);
            release
        }
        Err(_) => None,
    }
}

// ---------------------------------------------------------------------------
// Self-update seam (reserved, not implemented)
// ---------------------------------------------------------------------------

/// Where `self_update` would run if auto-update is ever wanted. Today it
/// only points at the release page: PopChat ships as an installer-less
/// build and the user decides.
pub fn apply_self_update(release: &ReleaseInfo) -> anyhow::Result<()> {
    // Reserved for `self_update::update(...)`; deliberately not wired:
    // this port only reports and links, like the macOS original.
    anyhow::bail!(
        "automatic update is not implemented — download {} from {}",
        release.version,
        release.page_url
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_version_comparison() {
        assert!(is_newer("0.1.10", "0.1.9"));
        assert!(!is_newer("0.1.9", "0.1.10"));
        assert!(!is_newer("0.1.9", "0.1.9"));
        assert!(is_newer("0.2.0", "0.1.99"));
        assert!(is_newer("v0.2.0", "0.1.0"));
        assert!(is_newer("1.0", "0.9.9"));
        assert!(!is_newer("1.0", "1.0.0"));
        // Trailing components default to zero.
        assert!(!is_newer("1.0.0", "1.0"));
        assert!(is_newer("1.0.1", "1.0"));
    }

    #[test]
    fn scheduled_errors_are_silent_but_manual_errors_are_visible() {
        let mut checker = UpdateChecker::default();
        checker.complete(Err("offline".to_owned()), false);
        assert_eq!(checker.state, CheckState::Idle);
        checker.complete(Err("offline".to_owned()), true);
        assert_eq!(checker.state, CheckState::Failed("offline".to_owned()));
        checker.complete(Ok(None), true);
        assert_eq!(checker.state, CheckState::UpToDate);
        let release = ReleaseInfo {
            version: "1.0.0".to_owned(),
            page_url: "https://example.com/release".to_owned(),
        };
        checker.complete(Ok(Some(release.clone())), false);
        assert_eq!(checker.state, CheckState::Available(release));
    }

    #[test]
    fn launch_check_is_deferred_five_seconds() {
        let before = std::time::Instant::now();
        let checker = UpdateChecker::default();
        let delay = checker.next_poll.saturating_duration_since(before);
        assert!(delay >= std::time::Duration::from_secs(5));
        assert!(delay < std::time::Duration::from_secs(6));
        assert!(checker.pending.is_none());
    }

    #[test]
    fn prerelease_suffix_never_beats_release() {
        // "0.2.0-beta.1" reduces to [0,2,0] and ties with 0.2.0.
        assert!(!is_newer("0.2.0-beta.1", "0.2.0"));
        assert!(!is_newer("0.2.0", "0.2.0-beta.1"));
        assert!(is_newer("0.2.1-beta.1", "0.2.0"));
    }

    #[test]
    fn release_payload_decoding() {
        let ok = serde_json::json!({
            "tag_name": "v0.2.0",
            "html_url": "https://github.com/lec77/PopChat/releases/tag/v0.2.0",
            "draft": false,
            "prerelease": false,
        });
        assert_eq!(
            decode_release(&ok),
            Some(ReleaseInfo {
                version: "0.2.0".to_string(),
                page_url: "https://github.com/lec77/PopChat/releases/tag/v0.2.0".to_string(),
            })
        );
        // Tag without a `v` prefix is kept as-is.
        let bare = serde_json::json!({"tag_name": "0.3.0", "html_url": "https://example.com/r"});
        assert_eq!(decode_release(&bare).unwrap().version, "0.3.0");
        // Drafts / prereleases are never offered.
        let draft = serde_json::json!({"tag_name": "v9.9.9", "html_url": "https://example.com", "draft": true});
        assert_eq!(decode_release(&draft), None);
        let pre = serde_json::json!({"tag_name": "v9.9.9", "html_url": "https://example.com", "prerelease": true});
        assert_eq!(decode_release(&pre), None);
        // Missing fields → None, not a panic.
        assert_eq!(
            decode_release(&serde_json::json!({"html_url": "https://example.com"})),
            None
        );
        assert_eq!(
            decode_release(
                &serde_json::json!({"tag_name": "v", "html_url": "https://example.com"})
            ),
            None
        );
    }

    #[test]
    fn self_update_seam_is_explicit() {
        let release = ReleaseInfo {
            version: "9.9.9".to_string(),
            page_url: "https://example.com".to_string(),
        };
        let err = apply_self_update(&release).unwrap_err();
        assert!(err.to_string().contains("not implemented"));
    }
}
