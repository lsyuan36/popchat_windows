//! Public-release checks and opt-in, bounded update downloads.
//!
//! Checking never downloads automatically. A download is staged and verified;
//! only a separate, explicitly confirmed helper handoff may replace files.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// The releases API for this app's own repository.
pub const LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/lsyuan36/popchat_windows/releases/latest";
pub const RELEASE_REPOSITORY: &str = "https://github.com/lsyuan36/popchat_windows";
pub const UPDATE_ASSET_NAME: &str = "PopChat-Windows-x64.zip";
pub use crate::updater::{begin_install, InstallJob, InstallSession, InstallState, PreparedUpdate};
/// How long a check stays good.
pub const CHECK_INTERVAL_SECS: i64 = 24 * 60 * 60;

/// Deliberately remains a two-field type for existing callers. Asset metadata
/// is fetched afresh and validated when the user requests a download.
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

/// Strict stable SemVer used at the release trust boundary (no suffixes,
/// missing components, leading zeroes, overflow, or whitespace).
pub fn stable_version(version: &str) -> Option<[u64; 3]> {
    let mut result = [0; 3];
    let parts: Vec<_> = version.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    for (slot, part) in result.iter_mut().zip(parts) {
        if part.is_empty()
            || !part.bytes().all(|b| b.is_ascii_digit())
            || (part.len() > 1 && part.starts_with('0'))
        {
            return None;
        }
        *slot = part.parse().ok()?;
    }
    Some(result)
}

// ---------------------------------------------------------------------------
// Payload decoding
// ---------------------------------------------------------------------------

/// Decode the `/releases/latest` payload. That endpoint already excludes
/// drafts and prereleases, but the flags are checked anyway — a repo can
/// serve a prerelease as "latest" if it is the only release.
pub fn decode_release(json: &serde_json::Value) -> Option<ReleaseInfo> {
    if json.get("draft").and_then(|v| v.as_bool()) != Some(false) {
        return None;
    }
    if json.get("prerelease").and_then(|v| v.as_bool()) != Some(false) {
        return None;
    }
    let tag = json.get("tag_name")?.as_str()?;
    let page = json.get("html_url")?.as_str()?;
    let version = tag.strip_prefix('v')?;
    if stable_version(version).is_none()
        || page != format!("{RELEASE_REPOSITORY}/releases/tag/{tag}")
    {
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
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(10 * 60))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 || !trusted_download_origin(attempt.url()) {
                attempt.error("Untrusted update redirect")
            } else {
                attempt.follow()
            }
        }))
        .user_agent(format!("PopChat/{}", current_version()))
        .build()?)
}

fn trusted_download_origin(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(443)
        && url.host_str().is_some_and(|host| {
            host == "github.com"
                || host == "release-assets.githubusercontent.com"
                || host == "objects.githubusercontent.com"
        })
}

async fn bounded_body(mut response: reqwest::Response, max: usize) -> anyhow::Result<Vec<u8>> {
    anyhow::ensure!(
        response
            .content_length()
            .is_none_or(|size| size <= max as u64),
        "Update response exceeds its size limit"
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        anyhow::ensure!(
            chunk.len() <= max.saturating_sub(bytes.len()),
            "Update response exceeds its size limit"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

async fn fetch_payload() -> anyhow::Result<Option<serde_json::Value>> {
    fetch_payload_from(&client()?, LATEST_RELEASE_URL).await
}

async fn fetch_payload_from(
    client: &reqwest::Client,
    url: &str,
) -> anyhow::Result<Option<serde_json::Value>> {
    let response = client
        .get(url)
        .timeout(std::time::Duration::from_secs(25))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await?;
    anyhow::ensure!(
        response.url().as_str() == url,
        "Unexpected GitHub API redirect"
    );
    match response.status().as_u16() {
        200 => Ok(Some(serde_json::from_slice(
            &bounded_body(response, 2 * 1024 * 1024).await?,
        )?)),
        403 | 429 => anyhow::bail!("GitHub is rate-limiting update checks — try again later."),
        404 => Ok(None),
        code => anyhow::bail!("GitHub returned HTTP {code}."),
    }
}

/// Fetch the latest release. `Ok(None)` means "no releases published yet"
/// (HTTP 404) — not an update, not an error.
pub async fn fetch_latest() -> anyhow::Result<Option<ReleaseInfo>> {
    match fetch_payload().await? {
        None => Ok(None),
        Some(json) => decode_release(&json)
            .map(Some)
            .ok_or_else(|| anyhow::anyhow!("GitHub returned an invalid or non-stable release")),
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
// Explicit downloads (never started by the scheduler)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct DownloadAsset {
    url: String,
    sha256: String,
    size: u64,
}

fn decode_asset(json: &serde_json::Value, release: &ReleaseInfo) -> anyhow::Result<DownloadAsset> {
    anyhow::ensure!(
        decode_release(json).as_ref() == Some(release),
        "Release changed; check for updates again"
    );
    let assets = json
        .get("assets")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow::anyhow!("Release has no downloadable assets"))?;
    let matches: Vec<_> = assets
        .iter()
        .filter(|asset| asset.get("name").and_then(|v| v.as_str()) == Some(UPDATE_ASSET_NAME))
        .collect();
    anyhow::ensure!(
        matches.len() == 1,
        "Release must contain exactly one {UPDATE_ASSET_NAME}"
    );
    let asset = matches[0];
    let url = asset
        .get("browser_download_url")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    anyhow::ensure!(
        url == format!(
            "{RELEASE_REPOSITORY}/releases/download/v{}/{UPDATE_ASSET_NAME}",
            release.version
        ),
        "Release archive URL is not trusted"
    );
    let digest = asset
        .get("digest")
        .and_then(|v| v.as_str())
        .and_then(|v| v.strip_prefix("sha256:"))
        .ok_or_else(|| {
            anyhow::anyhow!("GitHub has not supplied a SHA-256 digest for this archive")
        })?;
    anyhow::ensure!(
        crate::updater::valid_hash(digest),
        "Invalid GitHub SHA-256 digest"
    );
    let size = asset.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
    anyhow::ensure!(
        size > 0 && size <= crate::updater::MAX_ARCHIVE_BYTES,
        "Invalid update archive size"
    );
    Ok(DownloadAsset {
        url: url.to_owned(),
        sha256: digest.to_ascii_lowercase(),
        size,
    })
}

#[derive(Debug, Clone, Default)]
pub enum DownloadState {
    #[default]
    Idle,
    Downloading {
        received: u64,
        total: Option<u64>,
    },
    Ready(PreparedUpdate),
    Failed(String),
}

/// Own this alongside `UpdateChecker`. Call `poll` every frame, and `start`
/// only after the user clicks Download. `Ready` is NOT permission to install.
#[derive(Default)]
pub struct UpdateDownload {
    pub state: DownloadState,
    pending: Option<(
        tokio::sync::mpsc::Receiver<DownloadState>,
        tokio::task::AbortHandle,
    )>,
}

impl Drop for UpdateDownload {
    fn drop(&mut self) {
        if let Some((_, task)) = &self.pending {
            task.abort();
        }
    }
}

impl UpdateDownload {
    pub fn start(
        &mut self,
        ctx: &egui::Context,
        runtime: &tokio::runtime::Handle,
        release: &ReleaseInfo,
    ) {
        if self.pending.is_some() {
            return;
        }
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        self.state = DownloadState::Downloading {
            received: 0,
            total: None,
        };
        let ctx = ctx.clone();
        let release = release.clone();
        let task = runtime.spawn(async move {
            let result = download_release(&release, &tx, &ctx).await;
            let state = match result {
                Ok(ready) => DownloadState::Ready(ready),
                Err(error) => DownloadState::Failed(format!("{error:#}")),
            };
            let _ = tx.send(state).await;
            ctx.request_repaint();
        });
        self.pending = Some((rx, task.abort_handle()));
    }

    pub fn poll(&mut self, ctx: &egui::Context) {
        let mut finished = false;
        if let Some((rx, _)) = &mut self.pending {
            loop {
                match rx.try_recv() {
                    Ok(state) => {
                        finished =
                            matches!(state, DownloadState::Ready(_) | DownloadState::Failed(_));
                        self.state = state;
                        if finished {
                            break;
                        }
                    }
                    Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                    Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                        self.state = DownloadState::Failed("更新下載已中斷，請重試。".into());
                        finished = true;
                        break;
                    }
                }
            }
        }
        if finished {
            self.pending = None;
        }
        if self.pending.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
}

async fn download_release(
    release: &ReleaseInfo,
    tx: &tokio::sync::mpsc::Sender<DownloadState>,
    ctx: &egui::Context,
) -> anyhow::Result<PreparedUpdate> {
    use sha2::{Digest, Sha256};
    use tokio::io::AsyncWriteExt;
    let candidate = stable_version(&release.version)
        .ok_or_else(|| anyhow::anyhow!("Invalid stable release version"))?;
    anyhow::ensure!(
        candidate
            > stable_version(&current_version())
                .ok_or_else(|| anyhow::anyhow!("This build is not a stable version"))?,
        "The update is not newer than this build"
    );
    let json = fetch_payload()
        .await?
        .ok_or_else(|| anyhow::anyhow!("Release is no longer available"))?;
    let asset = decode_asset(&json, release)?;
    let workspace = crate::updater::create_workspace()?;
    let archive = workspace.join("download.zip");
    let mut output = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&archive)
        .await?;
    let mut response = client()?.get(&asset.url).send().await?.error_for_status()?;
    anyhow::ensure!(
        trusted_download_origin(response.url()),
        "Untrusted download origin"
    );
    anyhow::ensure!(
        response.content_length().is_none_or(|n| n == asset.size),
        "Archive size does not match GitHub metadata"
    );
    let mut hasher = Sha256::new();
    let mut received = 0_u64;
    let mut reported = 0_u64;
    while let Some(chunk) = response.chunk().await? {
        anyhow::ensure!(
            (chunk.len() as u64) <= asset.size.saturating_sub(received),
            "Archive exceeds its expected size"
        );
        output.write_all(&chunk).await?;
        hasher.update(&chunk);
        received += chunk.len() as u64;
        if received.saturating_sub(reported) >= 1024 * 1024 || received == asset.size {
            match tx.try_send(DownloadState::Downloading {
                received,
                total: Some(asset.size),
            }) {
                Ok(()) | Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {}
                Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                    anyhow::bail!("Update download cancelled")
                }
            }
            ctx.request_repaint();
            reported = received;
        }
    }
    output.flush().await?;
    output.sync_all().await?;
    drop(output);
    anyhow::ensure!(received == asset.size, "Incomplete update archive");
    anyhow::ensure!(
        format!("{:x}", hasher.finalize()) == asset.sha256,
        "Update archive SHA-256 mismatch"
    );
    tx.send(DownloadState::Downloading {
        received,
        total: Some(asset.size),
    })
    .await?;
    ctx.request_repaint();
    let version = release.version.clone();
    tokio::task::spawn_blocking(move || {
        crate::updater::stage_archive(&archive, &workspace, &version, &asset.sha256)
    })
    .await?
}

/// Kept for source compatibility. Installation requires a verified download
/// and explicit confirmation; a release-page object can never authorize it.
pub fn apply_self_update(_release: &ReleaseInfo) -> anyhow::Result<()> {
    anyhow::bail!("A release page cannot authorize file replacement; use UpdateDownload, then explicitly confirm begin_install with its PreparedUpdate")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_version_comparison() {
        assert!(is_newer("0.2.2", "0.2.1"));
        assert!(!is_newer("0.2.1", "0.2.2"));
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
            "html_url": "https://github.com/lsyuan36/popchat_windows/releases/tag/v0.2.0",
            "draft": false,
            "prerelease": false,
        });
        assert_eq!(
            decode_release(&ok),
            Some(ReleaseInfo {
                version: "0.2.0".to_string(),
                page_url: "https://github.com/lsyuan36/popchat_windows/releases/tag/v0.2.0"
                    .to_string(),
            })
        );
        // Public releases require stable vMAJOR.MINOR.PATCH tags and this repo.
        let bare = serde_json::json!({"tag_name": "0.3.0", "html_url": "https://example.com/r"});
        assert_eq!(decode_release(&bare), None);
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
    fn release_page_alone_cannot_authorize_installation() {
        let release = ReleaseInfo {
            version: "9.9.9".to_string(),
            page_url: "https://example.com".to_string(),
        };
        let err = apply_self_update(&release).unwrap_err();
        assert!(err.to_string().contains("cannot authorize"));
    }

    fn release_payload() -> serde_json::Value {
        serde_json::json!({
            "tag_name": "v0.3.0", "html_url": format!("{RELEASE_REPOSITORY}/releases/tag/v0.3.0"),
            "draft": false, "prerelease": false,
            "assets": [{"name": UPDATE_ASSET_NAME, "size": 2048,
                "browser_download_url": format!("{RELEASE_REPOSITORY}/releases/download/v0.3.0/{UPDATE_ASSET_NAME}"),
                "digest": format!("sha256:{}", "a".repeat(64))}]
        })
    }

    #[test]
    fn stable_release_boundary_is_strict() {
        for version in [
            "",
            "1",
            "1.0",
            "01.2.3",
            "1.02.3",
            "1.2.03",
            "1.2.3-beta",
            "1.2.3+build",
            "1.2.3.4",
            " 1.2.3",
            "1..3",
            "1.2.18446744073709551616",
        ] {
            assert!(stable_version(version).is_none(), "{version}");
        }
        assert_eq!(stable_version("0.2.0"), Some([0, 2, 0]));
        let good = release_payload();
        for tag in ["v1.0", "v01.2.3", "v1.2.3-beta", "1.2.3"] {
            let mut json = good.clone();
            json["tag_name"] = tag.into();
            assert!(decode_release(&json).is_none());
        }
        for page in [
            "https://github.com/lec77/PopChat/releases/tag/v0.3.0",
            "https://github.com/lsyuan36/popchat_windows.evil/releases/tag/v0.3.0",
            "http://github.com/lsyuan36/popchat_windows/releases/tag/v0.3.0",
            "https://github.com/lsyuan36/popchat_windows/releases/tag/v0.3.0?x",
        ] {
            let mut json = good.clone();
            json["html_url"] = page.into();
            assert!(decode_release(&json).is_none());
        }
        for flag in ["draft", "prerelease"] {
            let mut json = good.clone();
            json[flag] = true.into();
            assert!(decode_release(&json).is_none());
            json.as_object_mut().unwrap().remove(flag);
            assert!(decode_release(&json).is_none());
        }
    }

    #[test]
    fn exact_asset_url_and_api_digest_are_required() {
        let good = release_payload();
        let release = decode_release(&good).unwrap();
        assert_eq!(
            decode_asset(&good, &release).unwrap().sha256,
            "a".repeat(64)
        );
        for field in ["digest", "name", "browser_download_url", "size"] {
            let mut json = good.clone();
            json["assets"][0].as_object_mut().unwrap().remove(field);
            assert!(decode_asset(&json, &release).is_err());
        }
        for digest in [
            "sha256:abc",
            "sha512:abcd",
            "",
            &format!("sha256:{}", "z".repeat(64)),
        ] {
            let mut json = good.clone();
            json["assets"][0]["digest"] = digest.into();
            assert!(decode_asset(&json, &release).is_err());
        }
        for url in ["https://evil.test/file.zip", "https://github.com/lec77/PopChat/releases/download/v0.3.0/PopChat-Windows-x64.zip", "https://github.com/lsyuan36/popchat_windows/releases/download/v0.3.0/PopChat-Windows-x64.zip?download=1", "https://github.com/lsyuan36/popchat_windows/releases/download/v0.2.0/PopChat-Windows-x64.zip"] {
            let mut json = good.clone();
            json["assets"][0]["browser_download_url"] = url.into();
            assert!(decode_asset(&json, &release).is_err());
        }
        let mut duplicate = good.clone();
        duplicate["assets"]
            .as_array_mut()
            .unwrap()
            .push(good["assets"][0].clone());
        assert!(decode_asset(&duplicate, &release).is_err());
    }

    #[test]
    fn redirect_policy_rejects_untrusted_origins() {
        for url in [
            "https://github.com/x",
            "https://release-assets.githubusercontent.com/x?token=x",
            "https://objects.githubusercontent.com/x",
        ] {
            assert!(trusted_download_origin(&reqwest::Url::parse(url).unwrap()));
        }
        for url in [
            "http://github.com/x",
            "https://github.com.evil.test/x",
            "https://evil.test/x",
            "https://github.com:444/x",
            "https://user@github.com/x",
            "file:///C:/file",
        ] {
            assert!(!trusted_download_origin(&reqwest::Url::parse(url).unwrap()));
        }
    }

    #[tokio::test]
    async fn api_errors_missing_release_and_body_limit() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};
        for code in [403, 429, 404, 500] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(code))
                .mount(&server)
                .await;
            let result =
                fetch_payload_from(&client().unwrap(), &format!("{}/", server.uri())).await;
            match code {
                404 => assert!(result.unwrap().is_none()),
                403 | 429 => assert!(result.unwrap_err().to_string().contains("rate-limiting")),
                _ => assert!(result.unwrap_err().to_string().contains("500")),
            }
        }
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200).set_body_bytes(vec![b' '; 2 * 1024 * 1024 + 1]),
            )
            .mount(&server)
            .await;
        assert!(
            fetch_payload_from(&client().unwrap(), &format!("{}/", server.uri()))
                .await
                .unwrap_err()
                .to_string()
                .contains("size limit")
        );
    }

    #[tokio::test]
    async fn downloader_drop_aborts_pending_network_work() {
        let (started, started_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let _ = started.send(());
            std::future::pending::<()>().await;
        });
        let (_tx, receiver) = tokio::sync::mpsc::channel(8);
        let download = UpdateDownload {
            state: DownloadState::Downloading {
                received: 0,
                total: None,
            },
            pending: Some((receiver, task.abort_handle())),
        };
        started_rx.await.unwrap();
        drop(download);
        assert!(task.await.unwrap_err().is_cancelled());
    }

    #[tokio::test]
    async fn progress_is_bounded_and_terminal_failure_is_observed() {
        let (tx, receiver) = tokio::sync::mpsc::channel(8);
        let task = tokio::spawn(std::future::pending::<()>());
        let mut download = UpdateDownload {
            state: DownloadState::Idle,
            pending: Some((receiver, task.abort_handle())),
        };
        for received in 0..8 {
            tx.try_send(DownloadState::Downloading {
                received,
                total: Some(100),
            })
            .unwrap();
        }
        assert!(matches!(
            tx.try_send(DownloadState::Downloading {
                received: 9,
                total: Some(100)
            }),
            Err(tokio::sync::mpsc::error::TrySendError::Full(_))
        ));
        let ctx = egui::Context::default();
        download.poll(&ctx);
        assert!(matches!(
            download.state,
            DownloadState::Downloading { received: 7, .. }
        ));
        tx.send(DownloadState::Failed("test error".into()))
            .await
            .unwrap();
        download.poll(&ctx);
        assert!(matches!(download.state, DownloadState::Failed(_)));
        assert!(download.pending.is_none());
        task.abort();
    }
}
