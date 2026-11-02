//! Staging and the independent, GUI-subsystem updater process.
//!
//! SHA-256 checks integrity against GitHub's API metadata; they are NOT a
//! publisher signature or Windows Authenticode verification. No credentials,
//! registry, startup entries, shortcuts, or profile files are accessed here.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const MAX_ARCHIVE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_FILE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_EXPANDED_BYTES: u64 = 256 * 1024 * 1024;
// The locked dependency-license inventory currently has ~850 notice files.
const MAX_ENTRIES: usize = 2048;
const APP: &str = "popchat_windows.exe";
const HELPER: &str = "popchat_updater.exe";
const MANIFEST: &str = "install-manifest.json";
const MAX_JSON: u64 = 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StagedFile {
    relative: String,
    sha256: String,
    size: u64,
}

/// Only produced after a trusted archive has passed all staging checks. UI code
/// should clone it and display `version()`, not construct or modify it.
#[derive(Debug, Clone)]
pub struct PreparedUpdate {
    version: String,
    workspace: PathBuf,
    files: Vec<StagedFile>,
    nonce: String,
}

impl PreparedUpdate {
    pub fn version(&self) -> &str {
        &self.version
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallState {
    Starting,
    /// Helper validated the parent process and staged files, holds a process
    /// handle, and is waiting. NOW (not before) the app may actually terminate.
    ReadyToExit,
    Failed(String),
}

pub struct InstallSession {
    pub state: InstallState,
    receiver: std::sync::mpsc::Receiver<Result<(), String>>,
}

/// Compatibility name for the handoff job.
pub type InstallJob = InstallSession;

impl InstallSession {
    /// Nonblocking. Call every frame and arrange a repaint while Starting.
    pub fn poll(&mut self) -> Result<bool> {
        if self.state == InstallState::Starting {
            match self.receiver.try_recv() {
                Ok(Ok(())) => self.state = InstallState::ReadyToExit,
                Ok(Err(error)) => self.state = InstallState::Failed(error),
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.state = InstallState::Failed("Updater handshake interrupted".into());
                }
            }
        }
        match &self.state {
            InstallState::Starting => Ok(false),
            InstallState::ReadyToExit => Ok(true),
            InstallState::Failed(error) => anyhow::bail!("{error}"),
        }
    }
}

/// Call ONLY after explicit restart/draft-loss confirmation. All filesystem
/// validation, helper bootstrap and the handshake happen on a worker thread.
/// This never exits the app. `ReadyToExit` is the caller's permission to exit.
pub fn begin_install(prepared: &PreparedUpdate) -> Result<InstallSession> {
    let prepared = prepared.clone();
    let (tx, receiver) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("popchat-update-handoff".into())
        .spawn(move || {
            let result = bootstrap_helper(&prepared).map_err(|error| format!("{error:#}"));
            let _ = tx.send(result);
        })?;
    Ok(InstallSession {
        state: InstallState::Starting,
        receiver,
    })
}

pub(crate) fn valid_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|c| c.is_ascii_hexdigit())
}

fn hash_file(path: &Path, max: u64) -> Result<String> {
    ensure_regular_file(path)?;
    let mut file = File::open(path)?;
    anyhow::ensure!(
        file.metadata()?.len() <= max,
        "File exceeds size limit: {}",
        path.display()
    );
    let mut hasher = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        total = total
            .checked_add(count as u64)
            .context("File size overflow")?;
        anyhow::ensure!(total <= max, "File grew beyond size limit");
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn is_reparse(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn ensure_no_links(path: &Path) -> Result<()> {
    for ancestor in path.ancestors() {
        if ancestor.as_os_str().is_empty() {
            continue;
        }
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) => anyhow::ensure!(
                !is_reparse(&metadata) && !metadata.file_type().is_symlink(),
                "Symlink or reparse point is not allowed: {}",
                ancestor.display()
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn ensure_regular_file(path: &Path) -> Result<()> {
    ensure_no_links(path)?;
    anyhow::ensure!(
        fs::symlink_metadata(path)?.is_file(),
        "Not a regular file: {}",
        path.display()
    );
    Ok(())
}

pub(crate) fn create_workspace() -> Result<PathBuf> {
    let temp = fs::canonicalize(std::env::temp_dir())?;
    ensure_no_links(&temp)?;
    let workspace = temp.join(format!("PopChat-update-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&workspace)?;
    Ok(workspace)
}

fn validate_workspace(workspace: &Path) -> Result<()> {
    ensure_no_links(workspace)?;
    let temp = fs::canonicalize(std::env::temp_dir())?;
    let actual = fs::canonicalize(workspace)?;
    anyhow::ensure!(
        actual == workspace && actual.parent() == Some(temp.as_path()),
        "Update stage is not in its private temporary directory"
    );
    let name = actual.file_name().and_then(|v| v.to_str()).unwrap_or("");
    let id = name
        .strip_prefix("PopChat-update-")
        .context("Invalid stage directory name")?;
    uuid::Uuid::parse_str(id).context("Invalid stage identifier")?;
    Ok(())
}

fn safe_component(component: &str) -> bool {
    if component.is_empty()
        || component == "."
        || component == ".."
        || component.ends_with(['.', ' '])
        || component.len() > 120
        || !component
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_. +".contains(&b))
    {
        return false;
    }
    let base = component
        .split('.')
        .next()
        .unwrap_or("")
        .to_ascii_uppercase();
    !matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$")
        && !(base.len() == 4
            && (base.starts_with("COM") || base.starts_with("LPT"))
            && matches!(base.as_bytes()[3], b'0'..=b'9'))
}

fn allowed_relative(name: &str, directory: bool) -> bool {
    let name = if directory {
        name.strip_suffix('/').unwrap_or(name)
    } else {
        name
    };
    if name.len() > 220 || name.contains('\\') || !name.split('/').all(safe_component) {
        return false;
    }
    if directory {
        return name == "licenses" || name.starts_with("licenses/");
    }
    if matches!(
        name,
        APP | HELPER | "README.md" | "LICENSE" | "NOTICE.md" | "THIRD_PARTY_NOTICES.md"
    ) {
        return true;
    }
    if !name.starts_with("licenses/") {
        return false;
    }
    if name == "licenses/dependencies/index.json" {
        return true;
    }
    let basename = name.rsplit('/').next().unwrap_or("");
    // Notices and static attribution assets only. Never executable content,
    // shortcuts, scripts, or files with Windows alternate data streams.
    let extension = Path::new(basename)
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let upper = basename.to_ascii_uppercase();
    matches!(
        extension.as_str(),
        "txt"
            | "md"
            | "license"
            | "svg"
            | "apache"
            | "apache2"
            | "chromium"
            | "httprouter"
            | "mit"
            | "spdx"
    ) || (upper.starts_with("LICENSE-")
        && !extension.is_empty()
        && extension.bytes().all(|b| b.is_ascii_digit()))
        || (extension.is_empty()
            && (matches!(
                upper.as_str(),
                "LICENSE"
                    | "LICENCE"
                    | "NOTICE"
                    | "COPYING"
                    | "COPYRIGHT"
                    | "OFL"
                    | "AUTHORS"
                    | "PATENTS"
                    | "UNLICENSE"
            ) || upper.starts_with("LICENSE-")
                || upper.ends_with("_LICENSE")))
}

/// Check the central-directory attributes directly: zip's public Unix-mode
/// accessor does not expose DOS/Windows reparse flags. ZIP64 is unnecessary
/// under our strict limits and deliberately rejected.
fn validate_zip_attributes(path: &Path) -> Result<()> {
    use std::io::{Seek, SeekFrom};
    let mut file = File::open(path)?;
    let length = file.metadata()?.len();
    anyhow::ensure!(
        length <= MAX_ARCHIVE_BYTES && length >= 22,
        "Invalid archive size"
    );
    let tail_len = length.min(65557) as usize;
    file.seek(SeekFrom::End(-(tail_len as i64)))?;
    let mut tail = vec![0; tail_len];
    file.read_exact(&mut tail)?;
    let end = (0..=tail_len - 22)
        .rev()
        .find(|&i| {
            tail[i..i + 4] == [0x50, 0x4b, 5, 6]
                && i + 22 + u16::from_le_bytes([tail[i + 20], tail[i + 21]]) as usize == tail_len
        })
        .context("Invalid ZIP directory")?;
    let u16at = |i: usize| u16::from_le_bytes([tail[end + i], tail[end + i + 1]]);
    let u32at = |i: usize| u32::from_le_bytes(tail[end + i..end + i + 4].try_into().unwrap());
    let count = u16at(10) as usize;
    anyhow::ensure!(
        u16at(4) == 0
            && u16at(6) == 0
            && u16at(8) as usize == count
            && count > 0
            && count <= MAX_ENTRIES,
        "Unsupported or oversized ZIP directory"
    );
    let offset = u32at(16) as u64;
    let size = u32at(12) as u64;
    let directory_end = offset.checked_add(size).context("ZIP directory overflow")?;
    anyhow::ensure!(
        directory_end == length - tail_len as u64 + end as u64,
        "Unsupported ZIP directory layout"
    );
    file.seek(SeekFrom::Start(offset))?;
    for _ in 0..count {
        let mut header = [0_u8; 46];
        file.read_exact(&mut header)?;
        anyhow::ensure!(header[..4] == [0x50, 0x4b, 1, 2], "Invalid ZIP entry");
        let attributes = u32::from_le_bytes(header[38..42].try_into().unwrap());
        // FILE_ATTRIBUTE_REPARSE_POINT / DEVICE; Unix symlink/special modes.
        anyhow::ensure!(
            attributes & (0x400 | 0x40) == 0,
            "ZIP reparse/device entry is forbidden"
        );
        let host = header[5];
        if host == 3 {
            let kind = (attributes >> 16) & 0xf000;
            anyhow::ensure!(
                kind == 0 || kind == 0x8000 || kind == 0x4000,
                "ZIP symlink/special entry is forbidden"
            );
        }
        anyhow::ensure!(
            u16::from_le_bytes([header[8], header[9]]) & 1 == 0,
            "Encrypted updates are forbidden"
        );
        let skip: u64 = [28, 30, 32]
            .into_iter()
            .map(|i| u16::from_le_bytes([header[i], header[i + 1]]) as u64)
            .sum();
        file.seek(SeekFrom::Current(skip as i64))?;
        anyhow::ensure!(
            file.stream_position()? <= directory_end,
            "ZIP directory overflow"
        );
    }
    anyhow::ensure!(
        file.stream_position()? == directory_end,
        "Unexpected ZIP directory data"
    );
    Ok(())
}

/// PE32+ AMD64 executable, not a DLL, with the Windows GUI subsystem.
fn validate_pe(path: &Path) -> Result<()> {
    use std::io::{Seek, SeekFrom};
    ensure_regular_file(path)?;
    let mut file = File::open(path)?;
    let length = file.metadata()?.len();
    anyhow::ensure!((256..=MAX_FILE_BYTES).contains(&length), "Invalid PE size");
    let mut dos = [0_u8; 64];
    file.read_exact(&mut dos)?;
    anyhow::ensure!(&dos[..2] == b"MZ", "Missing PE DOS signature");
    let offset = u32::from_le_bytes(dos[60..64].try_into().unwrap()) as u64;
    anyhow::ensure!(
        offset >= 64 && offset.checked_add(112).is_some_and(|n| n <= length),
        "Invalid PE header offset"
    );
    file.seek(SeekFrom::Start(offset))?;
    let mut header = [0_u8; 112];
    file.read_exact(&mut header)?;
    let u16at = |i: usize| u16::from_le_bytes([header[i], header[i + 1]]);
    anyhow::ensure!(
        &header[..4] == b"PE\0\0" && u16at(4) == 0x8664,
        "Update requires an x64 PE executable"
    );
    anyhow::ensure!(
        u16at(6) > 0 && u16at(20) >= 112 && u16at(22) & 0x2 != 0 && u16at(22) & 0x2000 == 0,
        "Invalid PE executable characteristics"
    );
    anyhow::ensure!(
        u16at(24) == 0x20b && u16at(92) == 2,
        "Update executable must use the Windows GUI subsystem"
    );
    Ok(())
}

pub(crate) fn stage_archive(
    archive: &Path,
    workspace: &Path,
    version: &str,
    expected_archive_hash: &str,
) -> Result<PreparedUpdate> {
    anyhow::ensure!(
        crate::update::stable_version(version).is_some(),
        "Invalid update version"
    );
    validate_workspace(workspace)?;
    anyhow::ensure!(
        archive == workspace.join("download.zip"),
        "Archive is outside its workspace"
    );
    ensure_regular_file(archive)?;
    anyhow::ensure!(
        valid_hash(expected_archive_hash)
            && hash_file(archive, MAX_ARCHIVE_BYTES)? == expected_archive_hash,
        "Archive SHA-256 changed before staging"
    );
    validate_zip_attributes(archive)?;
    let mut zip = zip::ZipArchive::new(File::open(archive)?)?;
    anyhow::ensure!(
        zip.len() > 0 && zip.len() <= MAX_ENTRIES,
        "Too many ZIP entries"
    );
    let payload = workspace.join("payload");
    fs::create_dir(&payload)?;
    let mut seen = HashSet::new();
    let mut files = Vec::new();
    let mut total = 0_u64;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index)?;
        let directory = entry.is_dir();
        let name = entry.name().to_owned();
        anyhow::ensure!(
            allowed_relative(&name, directory),
            "Unexpected or unsafe archive path: {name}"
        );
        anyhow::ensure!(!entry.is_symlink(), "Archive symlinks are forbidden");
        let key = name.trim_end_matches('/').to_ascii_lowercase();
        anyhow::ensure!(seen.insert(key), "Duplicate/case-colliding archive entry");
        let output_path = payload.join(name.trim_end_matches('/'));
        if directory {
            ensure_no_links(&output_path)?;
            fs::create_dir_all(&output_path)?;
            continue;
        }
        anyhow::ensure!(
            entry.size() <= MAX_FILE_BYTES,
            "Expanded update file is too large"
        );
        total = total
            .checked_add(entry.size())
            .context("Expanded size overflow")?;
        anyhow::ensure!(total <= MAX_EXPANDED_BYTES, "Expanded archive is too large");
        fs::create_dir_all(output_path.parent().context("Missing output parent")?)?;
        ensure_no_links(&output_path)?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&output_path)?;
        let copied = std::io::copy(&mut (&mut entry).take(MAX_FILE_BYTES + 1), &mut output)?;
        anyhow::ensure!(
            copied == entry.size() && copied <= MAX_FILE_BYTES,
            "ZIP expanded length mismatch"
        );
        output.sync_all()?;
        drop(output);
        let sha256 = hash_file(&output_path, MAX_FILE_BYTES)?;
        if name == APP || name == HELPER {
            validate_pe(&output_path)?;
        }
        files.push(StagedFile {
            relative: name,
            sha256,
            size: copied,
        });
    }
    for required in [
        APP,
        HELPER,
        "README.md",
        "LICENSE",
        "NOTICE.md",
        "THIRD_PARTY_NOTICES.md",
    ] {
        anyhow::ensure!(
            files.iter().any(|file| file.relative == required),
            "Archive is missing {required}"
        );
    }
    for required in [
        "licenses/font-awesome/",
        "licenses/swei-gothic/",
        "licenses/dependencies/",
    ] {
        anyhow::ensure!(
            files.iter().any(|file| file.relative.starts_with(required)),
            "Archive is missing {required} notices"
        );
    }
    let ready = PreparedUpdate {
        version: version.to_owned(),
        workspace: workspace.to_owned(),
        files,
        nonce: uuid::Uuid::new_v4().to_string(),
    };
    verify_staged(&ready.workspace, &ready.files)?;
    Ok(ready)
}

fn verify_staged(workspace: &Path, files: &[StagedFile]) -> Result<()> {
    validate_workspace(workspace)?;
    anyhow::ensure!(
        !files.is_empty() && files.len() <= MAX_ENTRIES,
        "Invalid staged file list"
    );
    let mut seen = HashSet::new();
    let mut total = 0_u64;
    for file in files {
        anyhow::ensure!(
            allowed_relative(&file.relative, false)
                && valid_hash(&file.sha256)
                && seen.insert(file.relative.to_ascii_lowercase())
                && file.size <= MAX_FILE_BYTES,
            "Invalid staged file metadata"
        );
        total = total
            .checked_add(file.size)
            .context("Staged size overflow")?;
        anyhow::ensure!(total <= MAX_EXPANDED_BYTES, "Staged payload too large");
        let path = workspace.join("payload").join(&file.relative);
        anyhow::ensure!(
            fs::metadata(&path)?.len() == file.size
                && hash_file(&path, MAX_FILE_BYTES)? == file.sha256,
            "Staged file SHA-256 mismatch: {}",
            file.relative
        );
        if file.relative == APP || file.relative == HELPER {
            validate_pe(&path)?;
        }
    }
    anyhow::ensure!(
        seen.contains(APP) && seen.contains(HELPER),
        "Staged binaries missing"
    );
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HelperRequest {
    protocol: u32,
    parent_pid: u32,
    parent_exe: PathBuf,
    install_dir: PathBuf,
    launch_cwd: PathBuf,
    workspace: PathBuf,
    bootstrap_sha256: String,
    old_app_sha256: String,
    old_helper_sha256: String,
    old_manifest_sha256: Option<String>,
    old_version: String,
    version: String,
    nonce: String,
    files: Vec<StagedFile>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Status {
    nonce: String,
    state: String,
    message: String,
}

fn read_bounded(path: &Path, max: u64) -> Result<Vec<u8>> {
    ensure_regular_file(path)?;
    let mut data = Vec::new();
    File::open(path)?.take(max + 1).read_to_end(&mut data)?;
    anyhow::ensure!(data.len() as u64 <= max, "Metadata size limit exceeded");
    Ok(data)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    ensure_no_links(path)?;
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn manifest_snapshot(dir: &Path, old_version: &str, old_hash: &str) -> Result<Option<Vec<u8>>> {
    let path = dir.join(MANIFEST);
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
        Ok(_) => {}
    }
    let bytes = read_bounded(&path, MAX_JSON)?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    anyhow::ensure!(
        value.is_object() && value.get("app").and_then(|v| v.as_str()) == Some("PopChatWindows"),
        "Unrecognized install manifest"
    );
    anyhow::ensure!(
        value.get("version").and_then(|v| v.as_str()) == Some(old_version),
        "Install manifest version mismatch"
    );
    anyhow::ensure!(
        value
            .get("sha256")
            .and_then(|v| v.as_str())
            .is_some_and(|v| v.eq_ignore_ascii_case(old_hash)),
        "Install manifest app hash mismatch"
    );
    let recorded_dir = value
        .get("install_dir")
        .and_then(|v| v.as_str())
        .context("Install manifest directory missing")?;
    anyhow::ensure!(
        fs::canonicalize(recorded_dir)? == dir,
        "Install manifest directory mismatch"
    );
    let executable = value
        .get("executable")
        .and_then(|v| v.as_str())
        .context("Install manifest executable missing")?;
    anyhow::ensure!(
        executable == APP || fs::canonicalize(executable).is_ok_and(|path| path == dir.join(APP)),
        "Install manifest executable mismatch"
    );
    Ok(Some(bytes))
}

fn bootstrap_helper(prepared: &PreparedUpdate) -> Result<()> {
    anyhow::ensure!(
        cfg!(windows),
        "In-app installation is supported only on Windows"
    );
    verify_staged(&prepared.workspace, &prepared.files)?;
    anyhow::ensure!(
        crate::update::stable_version(&prepared.version)
            > crate::update::stable_version(env!("CARGO_PKG_VERSION")),
        "Update is not newer than this build"
    );
    let parent_exe = fs::canonicalize(std::env::current_exe()?)?;
    anyhow::ensure!(
        parent_exe.file_name().and_then(|v| v.to_str()) == Some(APP),
        "Run the installed or portable PopChat executable to update"
    );
    ensure_no_links(&parent_exe)?;
    let install_dir = parent_exe
        .parent()
        .context("App directory missing")?
        .to_owned();
    let helper = install_dir.join(HELPER);
    validate_pe(&parent_exe)?;
    validate_pe(&helper).context(
        "Installed updater helper missing or invalid; download the full release manually",
    )?;
    let old_app_sha256 = hash_file(&parent_exe, MAX_FILE_BYTES)?;
    let old_helper_sha256 = hash_file(&helper, MAX_FILE_BYTES)?;
    let old_version = env!("CARGO_PKG_VERSION").to_owned();
    let manifest = manifest_snapshot(&install_dir, &old_version, &old_app_sha256)?;
    if let Some(bytes) = &manifest {
        let json: serde_json::Value = serde_json::from_slice(bytes)?;
        if let Some(recorded) = json.get("updater_sha256") {
            anyhow::ensure!(
                recorded
                    .as_str()
                    .is_some_and(|v| v.eq_ignore_ascii_case(&old_helper_sha256)),
                "Installed helper SHA-256 mismatch"
            );
        }
    }
    // Run a copy of the currently installed, trusted helper, never the newly
    // downloaded helper. That permits replacing both installed executables.
    let bootstrap = prepared.workspace.join("bootstrap.exe");
    copy_verified(&helper, &bootstrap, &old_helper_sha256, MAX_FILE_BYTES)?;
    let request = HelperRequest {
        protocol: 1,
        parent_pid: std::process::id(),
        parent_exe,
        install_dir,
        launch_cwd: fs::canonicalize(std::env::current_dir()?)?,
        workspace: prepared.workspace.clone(),
        bootstrap_sha256: old_helper_sha256.clone(),
        old_app_sha256,
        old_helper_sha256,
        old_manifest_sha256: manifest
            .as_ref()
            .map(|bytes| format!("{:x}", Sha256::digest(bytes))),
        old_version,
        version: prepared.version.clone(),
        nonce: prepared.nonce.clone(),
        files: prepared.files.clone(),
    };
    let request_path = prepared.workspace.join("request.json");
    write_new(&request_path, &serde_json::to_vec(&request)?)?;
    let mut command = std::process::Command::new(&bootstrap);
    command
        .arg("--request")
        .arg(&request_path)
        .current_dir(&prepared.workspace);
    no_console(&mut command);
    let mut child = command
        .spawn()
        .context("Could not start the updater helper")?;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(status) = child.try_wait()? {
            let message = read_status(&prepared.workspace, &prepared.nonce)
                .map(|v| v.message)
                .unwrap_or_else(|_| format!("Updater exited before becoming ready ({status})"));
            anyhow::bail!("{message}");
        }
        if let Ok(status) = read_status(&prepared.workspace, &prepared.nonce) {
            if status.state == "ready" {
                return Ok(());
            }
            if status.state == "failed" {
                anyhow::bail!("{}", status.message);
            }
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("Updater readiness handshake timed out; the app has not been changed");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn no_console(command: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    #[cfg(not(windows))]
    let _ = command;
}

fn copy_verified(source: &Path, target: &Path, expected: &str, max: u64) -> Result<()> {
    anyhow::ensure!(
        hash_file(source, max)? == expected,
        "Source SHA-256 changed before copying"
    );
    ensure_no_links(target)?;
    let mut source_file = File::open(source)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)?;
    let count = std::io::copy(&mut (&mut source_file).take(max + 1), &mut output)?;
    anyhow::ensure!(count <= max, "Copy exceeded its size limit");
    output.sync_all()?;
    drop(output);
    anyhow::ensure!(
        hash_file(target, max)? == expected,
        "Copied file SHA-256 mismatch"
    );
    Ok(())
}

fn read_status(workspace: &Path, nonce: &str) -> Result<Status> {
    let status: Status =
        serde_json::from_slice(&read_bounded(&workspace.join("status.json"), 8192)?)?;
    anyhow::ensure!(status.nonce == nonce, "Updater status nonce mismatch");
    Ok(status)
}

fn write_status(request: &HelperRequest, state: &str, message: &str) -> Result<()> {
    validate_workspace(&request.workspace)?;
    uuid::Uuid::parse_str(&request.nonce).context("Invalid status nonce")?;
    anyhow::ensure!(state.len() <= 32, "Invalid status state");
    let message: String = message.chars().take(1000).collect();
    let data = serde_json::to_vec(&Status {
        nonce: request.nonce.clone(),
        state: state.to_owned(),
        message,
    })?;
    anyhow::ensure!(data.len() <= 8192, "Updater status exceeds its size limit");
    let pending = request
        .workspace
        .join(format!("status-{}.json", uuid::Uuid::new_v4()));
    write_new(&pending, &data)?;
    atomic_replace(&pending, &request.workspace.join("status.json"))
}

fn validate_request(request_path: &Path, request: &HelperRequest) -> Result<()> {
    validate_workspace(&request.workspace)?;
    anyhow::ensure!(
        request_path == request.workspace.join("request.json") && request.protocol == 1,
        "Invalid updater protocol/request location"
    );
    uuid::Uuid::parse_str(&request.nonce).context("Invalid handshake nonce")?;
    let old =
        crate::update::stable_version(&request.old_version).context("Invalid installed version")?;
    let new = crate::update::stable_version(&request.version).context("Invalid target version")?;
    anyhow::ensure!(
        new > old && request.parent_pid != 0 && request.parent_pid != std::process::id(),
        "Invalid update target or parent PID"
    );
    anyhow::ensure!(
        request.parent_exe == request.install_dir.join(APP)
            && fs::canonicalize(&request.install_dir)? == request.install_dir,
        "Invalid app directory"
    );
    ensure_no_links(&request.install_dir)?;
    anyhow::ensure!(
        request.install_dir != request.workspace
            && !request.workspace.starts_with(&request.install_dir)
            && !request.install_dir.starts_with(&request.workspace),
        "Installation and stage directories overlap"
    );
    let own_exe = fs::canonicalize(std::env::current_exe()?)?;
    anyhow::ensure!(
        own_exe == request.workspace.join("bootstrap.exe"),
        "Helper must run from its temporary bootstrap copy"
    );
    validate_pe(&own_exe)?;
    anyhow::ensure!(
        hash_file(&own_exe, MAX_FILE_BYTES)? == request.bootstrap_sha256
            && request.bootstrap_sha256 == request.old_helper_sha256,
        "Bootstrap helper hash mismatch"
    );
    verify_existing(request)?;
    verify_staged(&request.workspace, &request.files)?;
    Ok(())
}

fn verify_existing(request: &HelperRequest) -> Result<Option<Vec<u8>>> {
    anyhow::ensure!(
        hash_file(&request.parent_exe, MAX_FILE_BYTES)? == request.old_app_sha256,
        "Installed app changed since the update was prepared"
    );
    anyhow::ensure!(
        hash_file(&request.install_dir.join(HELPER), MAX_FILE_BYTES)? == request.old_helper_sha256,
        "Installed helper changed since the update was prepared"
    );
    let manifest = manifest_snapshot(
        &request.install_dir,
        &request.old_version,
        &request.old_app_sha256,
    )?;
    let hash = manifest
        .as_ref()
        .map(|bytes| format!("{:x}", Sha256::digest(bytes)));
    anyhow::ensure!(
        hash == request.old_manifest_sha256,
        "Install manifest changed since preparation"
    );
    Ok(manifest)
}

/// Entry point for the GUI-subsystem helper executable. Never call against a
/// live installation in tests; test the transaction with isolated directories.
pub fn helper_main(request_path: &Path) -> Result<()> {
    anyhow::ensure!(cfg!(windows), "Windows updater helper only");
    let request: HelperRequest = serde_json::from_slice(&read_bounded(request_path, MAX_JSON)?)?;
    // Do not write even a status file to a path supplied by an invalid request.
    validate_workspace(&request.workspace)?;
    anyhow::ensure!(
        request_path == request.workspace.join("request.json"),
        "Invalid request location"
    );
    let result = run_helper(request_path, &request);
    if let Err(error) = &result {
        let _ = write_status(&request, "failed", &format!("{error:#}"));
        let parent_gone = ParentProcess::open(request.parent_pid, &request.parent_exe)
            .map_or(true, |parent| parent.wait(Duration::ZERO).is_ok());
        if parent_gone {
            show_failure(&format!("{error:#}"), &request.workspace);
        }
    }
    result
}

fn show_failure(message: &str, workspace: &Path) {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "user32")]
        extern "system" {
            fn MessageBoxW(
                window: *mut std::ffi::c_void,
                text: *const u16,
                title: *const u16,
                flags: u32,
            ) -> i32;
        }
        let bounded: String = message.chars().take(1500).filter(|c| *c != '\0').collect();
        let status_path: String = workspace
            .join("status.json")
            .display()
            .to_string()
            .chars()
            .take(512)
            .collect();
        let text = format!("PopChat update failed. No second chat window has been opened.\n\n{bounded}\n\nUpdate status: {status_path}\nIf rollback was incomplete, keep the retained backup and reinstall the public release manually.");
        let text: Vec<u16> = std::ffi::OsStr::new(&text)
            .encode_wide()
            .chain(Some(0))
            .collect();
        let title: Vec<u16> = "PopChat update".encode_utf16().chain(Some(0)).collect();
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                text.as_ptr(),
                title.as_ptr(),
                0x10 | 0x10000,
            );
        }
    }
    #[cfg(not(windows))]
    let _ = (message, workspace);
}

fn run_helper(request_path: &Path, request: &HelperRequest) -> Result<()> {
    validate_request(request_path, request)?;
    let parent = ParentProcess::open(request.parent_pid, &request.parent_exe)?;
    ensure_no_other_app_processes(&request.parent_exe, request.parent_pid)?;
    write_status(request, "ready", "Waiting for PopChat to exit")?;
    wait_then_install(
        request,
        || parent.wait(Duration::from_secs(30)),
        || {
            let mut command = std::process::Command::new(&request.parent_exe);
            // Preserve the original working directory as well as the inherited
            // environment (including a relative POPCHAT_DATA_DIR). Never inspect
            // or rewrite the profile or credentials to accomplish a relaunch.
            command.arg("--settings").current_dir(&request.launch_cwd);
            no_console(&mut command);
            let mut child = command
                .spawn()
                .context("Updated app could not be relaunched")?;
            // A successful CreateProcess is not proof the new app started. If
            // it exits immediately (missing runtime, startup failure, another
            // instance winning the race), roll back only after that PID exits.
            let deadline = Instant::now() + Duration::from_millis(1500);
            while Instant::now() < deadline {
                if let Some(status) = child.try_wait()? {
                    anyhow::bail!("Updated app exited during startup ({status})");
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Ok(())
        },
    )?;
    // A status-write failure after launching must not roll back a running app.
    let _ = write_status(
        request,
        "installed",
        "Update installed; backup retained in the application directory",
    );
    Ok(())
}

fn wait_then_install(
    request: &HelperRequest,
    wait: impl FnOnce() -> Result<()>,
    launch: impl FnOnce() -> Result<()>,
) -> Result<()> {
    wait().context("PopChat did not exit within 30 seconds; no files have been replaced")?;
    ensure_no_other_app_processes(&request.parent_exe, request.parent_pid)?;
    // No destination writes whatsoever occur before the process handle signals.
    verify_staged(&request.workspace, &request.files)?;
    let manifest = verify_existing(request)?;
    install_transaction(request, manifest, launch)
}

struct BackupEntry {
    target: PathBuf,
    backup: Option<PathBuf>,
    old_hash: Option<String>,
    new_hash: String,
    max: u64,
}

fn install_transaction(
    request: &HelperRequest,
    manifest: Option<Vec<u8>>,
    launch: impl FnOnce() -> Result<()>,
) -> Result<()> {
    ensure_no_links(&request.install_dir)?;
    let backup_root = request
        .install_dir
        .join(format!(".popchat-update-backup-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&backup_root)
        .context("Application folder is not writable; download and install the release manually")?;
    let mut applied: Vec<BackupEntry> = Vec::new();
    let result = (|| -> Result<()> {
        for file in &request.files {
            let target = request.install_dir.join(&file.relative);
            let parent = target.parent().context("Missing destination parent")?;
            ensure_no_links(parent)?;
            fs::create_dir_all(parent)?;
            let backup = backup_target(
                &target,
                &backup_root.join(&file.relative),
                &file.sha256,
                MAX_FILE_BYTES,
            )?;
            // Backups retain all preexisting files, including notices. Newly
            // introduced files are tracked and only our own copies removed.
            let incoming = parent.join(format!(
                ".popchat-{}-{}.new",
                request.nonce,
                uuid::Uuid::new_v4()
            ));
            copy_verified(
                &request.workspace.join("payload").join(&file.relative),
                &incoming,
                &file.sha256,
                MAX_FILE_BYTES,
            )?;
            if file.relative == APP || file.relative == HELPER {
                validate_pe(&incoming)?;
            }
            verify_target_unchanged(&backup)?;
            atomic_replace(&incoming, &target)?;
            applied.push(backup);
        }
        if let Some(original) = manifest {
            let manifest_path = request.install_dir.join(MANIFEST);
            anyhow::ensure!(
                read_bounded(&manifest_path, MAX_JSON)? == original,
                "Install manifest changed during update"
            );
            let mut value: serde_json::Value = serde_json::from_slice(&original)?;
            let app_hash = &request
                .files
                .iter()
                .find(|file| file.relative == APP)
                .context("App missing")?
                .sha256;
            let helper_hash = &request
                .files
                .iter()
                .find(|file| file.relative == HELPER)
                .context("Helper missing")?
                .sha256;
            value["version"] = request.version.clone().into();
            value["sha256"] = app_hash.clone().into();
            value["updater_sha256"] = helper_hash.clone().into();
            let bytes = serde_json::to_vec_pretty(&value)?;
            anyhow::ensure!(
                bytes.len() as u64 <= MAX_JSON,
                "Updated manifest exceeds size limit"
            );
            let hash = format!("{:x}", Sha256::digest(&bytes));
            let backup =
                backup_target(&manifest_path, &backup_root.join(MANIFEST), &hash, MAX_JSON)?;
            let incoming = request
                .install_dir
                .join(format!(".popchat-manifest-{}.new", uuid::Uuid::new_v4()));
            write_new(&incoming, &bytes)?;
            verify_target_unchanged(&backup)?;
            atomic_replace(&incoming, &manifest_path)?;
            applied.push(backup);
        }
        launch()?;
        Ok(())
    })();
    if let Err(error) = result {
        let mut failures = Vec::new();
        for entry in applied.iter().rev() {
            if let Err(rollback) = rollback_entry(entry) {
                failures.push(format!("{}: {rollback:#}", entry.target.display()));
            }
        }
        if !failures.is_empty() {
            anyhow::bail!(
                "{error:#}; rollback incomplete (backups retained at {}): {}",
                backup_root.display(),
                failures.join("; ")
            );
        }
        return Err(error.context(format!(
            "Original files restored; backups retained at {}",
            backup_root.display()
        )));
    }
    Ok(())
}

fn backup_target(
    target: &Path,
    backup_path: &Path,
    new_hash: &str,
    max: u64,
) -> Result<BackupEntry> {
    ensure_no_links(target)?;
    let (backup, old_hash) = match fs::symlink_metadata(target) {
        Ok(_) => {
            let old_hash = hash_file(target, max)?;
            fs::create_dir_all(backup_path.parent().context("Backup parent missing")?)?;
            copy_verified(target, backup_path, &old_hash, max)?;
            (Some(backup_path.to_owned()), Some(old_hash))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (None, None),
        Err(error) => return Err(error.into()),
    };
    Ok(BackupEntry {
        target: target.to_owned(),
        backup,
        old_hash,
        new_hash: new_hash.to_owned(),
        max,
    })
}

fn verify_target_unchanged(entry: &BackupEntry) -> Result<()> {
    if let Some(old_hash) = &entry.old_hash {
        anyhow::ensure!(
            hash_file(&entry.target, entry.max)? == *old_hash,
            "Destination changed during update"
        );
    } else {
        anyhow::ensure!(
            fs::symlink_metadata(&entry.target)
                .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound),
            "Destination appeared during update"
        );
    }
    Ok(())
}

fn rollback_entry(entry: &BackupEntry) -> Result<()> {
    anyhow::ensure!(
        hash_file(&entry.target, entry.max)? == entry.new_hash,
        "Refusing to roll back a file changed by another process"
    );
    match (&entry.backup, &entry.old_hash) {
        (Some(backup), Some(hash)) => {
            let incoming = entry
                .target
                .parent()
                .context("Rollback parent missing")?
                .join(format!(".popchat-rollback-{}.new", uuid::Uuid::new_v4()));
            copy_verified(backup, &incoming, hash, entry.max)?;
            atomic_replace(&incoming, &entry.target)
        }
        (None, None) => {
            fs::remove_file(&entry.target)?;
            Ok(())
        }
        _ => anyhow::bail!("Invalid rollback bookkeeping"),
    }
}

fn atomic_replace(source: &Path, target: &Path) -> Result<()> {
    ensure_no_links(source)?;
    ensure_no_links(target)?;
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        extern "system" {
            fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
        }
        let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
        let target: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
        // Same-volume rename, replace existing, write-through. No shell/UAC.
        let ok = unsafe { MoveFileExW(source.as_ptr(), target.as_ptr(), 0x1 | 0x8) };
        anyhow::ensure!(
            ok != 0,
            "Atomic replacement failed: {}",
            std::io::Error::last_os_error()
        );
    }
    #[cfg(not(windows))]
    fs::rename(source, target)?;
    Ok(())
}

#[cfg(windows)]
struct ParentProcess(isize);

#[cfg(windows)]
impl ParentProcess {
    fn open(pid: u32, expected_path: &Path) -> Result<Self> {
        #[link(name = "kernel32")]
        extern "system" {
            fn OpenProcess(access: u32, inherit: i32, pid: u32) -> isize;
            fn QueryFullProcessImageNameW(
                process: isize,
                flags: u32,
                path: *mut u16,
                size: *mut u32,
            ) -> i32;
        }
        use std::os::windows::ffi::OsStringExt;
        // Hold this handle from BEFORE READY until after wait. PID reuse cannot
        // trick the helper into waiting on/replacing a different process.
        let handle = unsafe { OpenProcess(0x00100000 | 0x1000, 0, pid) }; // SYNCHRONIZE | QUERY_LIMITED_INFORMATION
        anyhow::ensure!(
            handle != 0,
            "Cannot open the PopChat process: {}",
            std::io::Error::last_os_error()
        );
        let parent = Self(handle);
        let mut path = vec![0_u16; 32768];
        let mut size = path.len() as u32;
        anyhow::ensure!(
            unsafe { QueryFullProcessImageNameW(handle, 0, path.as_mut_ptr(), &mut size) } != 0,
            "Cannot verify parent executable path"
        );
        let actual = PathBuf::from(std::ffi::OsString::from_wide(&path[..size as usize]));
        anyhow::ensure!(
            fs::canonicalize(actual)? == expected_path,
            "Parent PID is not the expected PopChat executable"
        );
        Ok(parent)
    }

    fn wait(&self, timeout: Duration) -> Result<()> {
        #[link(name = "kernel32")]
        extern "system" {
            fn WaitForSingleObject(handle: *mut std::ffi::c_void, milliseconds: u32) -> u32;
        }
        let result = unsafe {
            WaitForSingleObject(
                self.0 as *mut _,
                timeout.as_millis().min(u32::MAX as u128) as u32,
            )
        };
        anyhow::ensure!(
            result == 0,
            "Parent process is still running or the wait failed ({result})"
        );
        Ok(())
    }
}

#[cfg(windows)]
impl Drop for ParentProcess {
    fn drop(&mut self) {
        #[link(name = "kernel32")]
        extern "system" {
            fn CloseHandle(handle: isize) -> i32;
        }
        unsafe {
            CloseHandle(self.0);
        }
    }
}

#[cfg(windows)]
fn ensure_no_other_app_processes(expected_path: &Path, parent_pid: u32) -> Result<()> {
    use std::os::windows::ffi::OsStringExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn K32EnumProcesses(ids: *mut u32, bytes: u32, needed: *mut u32) -> i32;
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> isize;
        fn QueryFullProcessImageNameW(
            process: isize,
            flags: u32,
            path: *mut u16,
            size: *mut u32,
        ) -> i32;
        fn WaitForSingleObject(handle: *mut std::ffi::c_void, milliseconds: u32) -> u32;
    }
    let mut ids = vec![0_u32; 65536];
    let mut needed = 0_u32;
    let bytes = (ids.len() * std::mem::size_of::<u32>()) as u32;
    anyhow::ensure!(
        unsafe { K32EnumProcesses(ids.as_mut_ptr(), bytes, &mut needed) } != 0 && needed < bytes,
        "Could not safely check for another running copy of PopChat"
    );
    for pid in ids
        .into_iter()
        .take(needed as usize / 4)
        .filter(|pid| *pid != 0 && *pid != parent_pid && *pid != std::process::id())
    {
        let handle = unsafe { OpenProcess(0x00100000 | 0x1000, 0, pid) };
        if handle == 0 {
            continue;
        } // Protected unrelated processes are normal.
        let process = ParentProcess(handle);
        let mut buffer = vec![0_u16; 32768];
        let mut length = buffer.len() as u32;
        if unsafe { QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut length) } == 0 {
            continue;
        }
        let path = PathBuf::from(std::ffi::OsString::from_wide(&buffer[..length as usize]));
        if fs::canonicalize(path).is_ok_and(|path| path == expected_path) {
            anyhow::ensure!(unsafe { WaitForSingleObject(process.0 as *mut _, 0) } == 0,
                "Another PopChat process from the same application folder is still running; close it before updating");
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn ensure_no_other_app_processes(_expected_path: &Path, _parent_pid: u32) -> Result<()> {
    Ok(())
}

#[cfg(not(windows))]
struct ParentProcess;

#[cfg(not(windows))]
impl ParentProcess {
    fn open(_pid: u32, _expected_path: &Path) -> Result<Self> {
        anyhow::bail!("Windows updater only")
    }
    fn wait(&self, _timeout: Duration) -> Result<()> {
        anyhow::bail!("Windows updater only")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zip::write::SimpleFileOptions;

    /// Publisher gate using the *real* release archive, without running either
    /// executable or preparing a live installation. Run explicitly after ZIP
    /// creation with its independently recorded expected SHA-256.
    #[test]
    #[ignore = "requires POPCHAT_TEST_RELEASE_ARCHIVE and POPCHAT_TEST_RELEASE_SHA256"]
    fn release_zip_matches_updater_contract() {
        let source = PathBuf::from(
            std::env::var_os("POPCHAT_TEST_RELEASE_ARCHIVE").expect("release archive path"),
        );
        let expected = std::env::var("POPCHAT_TEST_RELEASE_SHA256")
            .expect("release SHA-256")
            .to_ascii_lowercase();
        assert!(valid_hash(&expected));
        let workspace = Workspace::new();
        let archive = workspace.0.join("download.zip");
        fs::copy(source, &archive).unwrap();
        let prepared =
            stage_archive(&archive, &workspace.0, env!("CARGO_PKG_VERSION"), &expected).unwrap();
        assert_eq!(prepared.version(), env!("CARGO_PKG_VERSION"));
        assert!(prepared.files.iter().any(|file| file.relative == APP));
        assert!(prepared.files.iter().any(|file| file.relative == HELPER));
        assert!(prepared
            .files
            .iter()
            .any(|file| file.relative == "licenses/dependencies/index.json"));
        assert!(
            !workspace.0.join("request.json").exists(),
            "package validation must not start an installation"
        );
        println!(
            "REAL_RELEASE_ZIP_VALIDATED files={} version={}",
            prepared.files.len(),
            prepared.version()
        );
    }

    struct Workspace(PathBuf);
    impl Workspace {
        fn new() -> Self {
            Self(create_workspace().unwrap())
        }
    }
    impl Drop for Workspace {
        fn drop(&mut self) {
            // This is only a uniquely created test-owned temporary directory.
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn fake_pe(marker: u8) -> Vec<u8> {
        let mut bytes = vec![0_u8; 512];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[60..64].copy_from_slice(&64_u32.to_le_bytes());
        bytes[64..68].copy_from_slice(b"PE\0\0");
        bytes[68..70].copy_from_slice(&0x8664_u16.to_le_bytes());
        bytes[70..72].copy_from_slice(&1_u16.to_le_bytes());
        bytes[84..86].copy_from_slice(&240_u16.to_le_bytes());
        bytes[86..88].copy_from_slice(&0x22_u16.to_le_bytes());
        bytes[88..90].copy_from_slice(&0x20b_u16.to_le_bytes());
        bytes[156..158].copy_from_slice(&2_u16.to_le_bytes());
        bytes[511] = marker;
        bytes
    }

    fn files() -> Vec<(String, Vec<u8>)> {
        vec![
            (APP.into(), fake_pe(3)),
            (HELPER.into(), fake_pe(4)),
            ("README.md".into(), b"new readme".to_vec()),
            ("LICENSE".into(), b"MIT".to_vec()),
            ("NOTICE.md".into(), b"source attribution".to_vec()),
            (
                "THIRD_PARTY_NOTICES.md".into(),
                b"dependency notices".to_vec(),
            ),
            (
                "licenses/font-awesome/LICENSE.txt".into(),
                b"font license".to_vec(),
            ),
            ("licenses/font-awesome/icon.svg".into(), b"<svg/>".to_vec()),
            (
                "licenses/swei-gothic/OFL.txt".into(),
                b"font license".to_vec(),
            ),
            (
                "licenses/dependencies/NOTICE.md".into(),
                b"dependencies".to_vec(),
            ),
        ]
    }

    fn make_zip(workspace: &Workspace, entries: &[(String, Vec<u8>)]) -> PathBuf {
        let path = workspace.0.join("download.zip");
        let mut zip = zip::ZipWriter::new(File::create(&path).unwrap());
        for (name, bytes) in entries {
            zip.start_file(
                name,
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated),
            )
            .unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
        path
    }

    fn stage(workspace: &Workspace, entries: &[(String, Vec<u8>)]) -> Result<PreparedUpdate> {
        let archive = make_zip(workspace, entries);
        let hash = hash_file(&archive, MAX_ARCHIVE_BYTES).unwrap();
        stage_archive(&archive, &workspace.0, "0.3.0", &hash)
    }

    fn request(prepared: &PreparedUpdate, installed: &Path, with_manifest: bool) -> HelperRequest {
        let installed = fs::canonicalize(installed).unwrap();
        fs::write(installed.join(APP), fake_pe(1)).unwrap();
        fs::write(installed.join(HELPER), fake_pe(2)).unwrap();
        fs::write(installed.join("README.md"), b"old readme").unwrap();
        fs::write(installed.join("user-side-file.dat"), b"never change").unwrap();
        let old_app_sha256 = hash_file(&installed.join(APP), MAX_FILE_BYTES).unwrap();
        let old_helper_sha256 = hash_file(&installed.join(HELPER), MAX_FILE_BYTES).unwrap();
        let old_manifest_sha256 = if with_manifest {
            let value = serde_json::json!({
                "app": "PopChatWindows", "version": "0.2.0", "sha256": old_app_sha256,
                "updater_sha256": old_helper_sha256,
                "install_dir": installed, "executable": installed.join(APP),
                "data_profile": "untouched-profile", "shortcuts": ["untouched.lnk"],
                "unknown_nested_field": { "keep": [1, 2, 3] },
                "build_channel": "LOCAL BUILD", "author": "Evis Lai"
            });
            let bytes = serde_json::to_vec(&value).unwrap();
            fs::write(installed.join(MANIFEST), &bytes).unwrap();
            Some(format!("{:x}", Sha256::digest(bytes)))
        } else {
            None
        };
        HelperRequest {
            protocol: 1,
            parent_pid: std::process::id(),
            parent_exe: installed.join(APP),
            install_dir: installed,
            launch_cwd: std::env::current_dir().unwrap(),
            workspace: prepared.workspace.clone(),
            bootstrap_sha256: old_helper_sha256.clone(),
            old_app_sha256,
            old_helper_sha256,
            old_manifest_sha256,
            old_version: "0.2.0".into(),
            version: prepared.version.clone(),
            nonce: prepared.nonce.clone(),
            files: prepared.files.clone(),
        }
    }

    fn directory_names(path: &Path) -> Vec<String> {
        let mut result: Vec<_> = fs::read_dir(path)
            .unwrap()
            .map(|v| v.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        result.sort();
        result
    }

    #[test]
    fn accepts_contract_and_rechecks_every_staged_hash() {
        let workspace = Workspace::new();
        let prepared = stage(&workspace, &files()).unwrap();
        assert_eq!(prepared.version(), "0.3.0");
        verify_staged(&prepared.workspace, &prepared.files).unwrap();
        fs::write(prepared.workspace.join("payload").join(APP), fake_pe(99)).unwrap();
        assert!(verify_staged(&prepared.workspace, &prepared.files)
            .unwrap_err()
            .to_string()
            .contains("SHA-256"));
    }

    #[test]
    fn rejects_archive_hash_mismatch_before_extraction() {
        let workspace = Workspace::new();
        let archive = make_zip(&workspace, &files());
        assert!(stage_archive(&archive, &workspace.0, "0.3.0", &"0".repeat(64)).is_err());
        assert!(!workspace.0.join("payload").exists());
    }

    #[test]
    fn rejects_traversal_scripts_unknown_executables_and_windows_aliases() {
        for path in [
            "../outside.exe",
            "/outside.exe",
            "C:/outside.exe",
            "licenses/../../outside",
            "licenses\\outside.txt",
            "licenses/a:stream.txt",
            "licenses/CON.txt",
            "licenses/x.",
            "licenses/x /NOTICE.md",
            "licenses/AUX/NOTICE.md",
            "install.ps1",
            "unknown.exe",
            "licenses/helper.exe",
            "licenses/font-awesome/script.js",
            "licenses/dependencies/start.lnk",
            "nested/popchat_windows.exe",
        ] {
            assert!(!allowed_relative(path, false), "{path}");
            let workspace = Workspace::new();
            let mut entries = files();
            entries.push((path.into(), b"unsafe".to_vec()));
            assert!(stage(&workspace, &entries).is_err(), "{path}");
            assert!(!workspace.0.parent().unwrap().join("outside.exe").exists());
        }
    }

    #[test]
    fn rejects_case_collisions_and_missing_required_files() {
        let workspace = Workspace::new();
        let mut entries = files();
        entries.push((
            "licenses/font-awesome/license.TXT".into(),
            b"collision".to_vec(),
        ));
        assert!(stage(&workspace, &entries)
            .unwrap_err()
            .to_string()
            .contains("colliding"));
        for required in [
            HELPER,
            "NOTICE.md",
            "THIRD_PARTY_NOTICES.md",
            "licenses/dependencies/NOTICE.md",
        ] {
            let workspace = Workspace::new();
            let entries: Vec<_> = files()
                .into_iter()
                .filter(|entry| entry.0 != required)
                .collect();
            assert!(stage(&workspace, &entries).is_err(), "{required}");
        }
    }

    #[test]
    fn accepts_all_packaged_dependency_notice_paths() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/licenses/dependencies/index.json");
        let inventory: serde_json::Value =
            serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        let mut count = 30; // root documents, two binaries, font notices/assets
        assert!(allowed_relative("licenses/dependencies/index.json", false));
        for package in inventory["packages"].as_array().unwrap() {
            for file in package["files"].as_array().unwrap() {
                let relative = format!("licenses/dependencies/{}", file.as_str().unwrap());
                assert!(allowed_relative(&relative, false), "{relative}");
                count += 1;
            }
        }
        assert!(
            count <= MAX_ENTRIES,
            "release notice inventory exceeds updater entry bound"
        );
    }

    fn patch_first_central(path: &Path, offset: usize, value: &[u8]) {
        let mut bytes = fs::read(path).unwrap();
        let position = bytes
            .windows(4)
            .position(|bytes| bytes == b"PK\x01\x02")
            .unwrap();
        bytes[position + offset..position + offset + value.len()].copy_from_slice(value);
        fs::write(path, bytes).unwrap();
    }

    #[test]
    fn rejects_zip_symlinks_and_reparse_attributes() {
        for attributes in [0xa1ff_0000_u32, 0x400_u32, 0x40_u32] {
            let workspace = Workspace::new();
            let archive = make_zip(&workspace, &files());
            if attributes == 0xa1ff_0000 {
                patch_first_central(&archive, 5, &[3]);
            }
            patch_first_central(&archive, 38, &attributes.to_le_bytes());
            assert!(validate_zip_attributes(&archive).is_err());
            assert!(!workspace.0.join("payload").exists());
        }
    }

    #[test]
    fn bounds_archive_entry_count_and_expanded_size() {
        let workspace = Workspace::new();
        let entries: Vec<_> = (0..=MAX_ENTRIES)
            .map(|i| (format!("licenses/dependencies/{i}.txt"), vec![]))
            .collect();
        let archive = make_zip(&workspace, &entries);
        assert!(validate_zip_attributes(&archive).is_err());
        let workspace = Workspace::new();
        let archive = make_zip(&workspace, &files());
        patch_first_central(&archive, 24, &((MAX_FILE_BYTES + 1) as u32).to_le_bytes());
        let digest = hash_file(&archive, MAX_ARCHIVE_BYTES).unwrap();
        assert!(stage_archive(&archive, &workspace.0, "0.3.0", &digest).is_err());
        let path = workspace.0.join("oversized.zip");
        File::create(&path)
            .unwrap()
            .set_len(MAX_ARCHIVE_BYTES + 1)
            .unwrap();
        assert!(validate_zip_attributes(&path).is_err());
        let workspace = Workspace::new();
        let prepared = stage(&workspace, &files()).unwrap();
        let oversized = vec![StagedFile {
            relative: "licenses/dependencies/a.txt".into(),
            sha256: "a".repeat(64),
            size: MAX_FILE_BYTES + 1,
        }];
        assert!(verify_staged(&prepared.workspace, &oversized).is_err());
    }

    #[test]
    fn rejects_bad_machine_console_subsystem_dll_and_malformed_pe() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.exe");
        fs::write(&path, fake_pe(1)).unwrap();
        validate_pe(&path).unwrap();
        for (offset, value) in [
            (68, 0x014c_u16),
            (88, 0x10b),
            (156, 3),
            (86, 0x2022),
            (70, 0),
            (84, 0),
        ] {
            let mut bytes = fake_pe(1);
            bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
            fs::write(&path, bytes).unwrap();
            assert!(validate_pe(&path).is_err(), "offset {offset}");
        }
        fs::write(&path, b"MZ not really PE").unwrap();
        assert!(validate_pe(&path).is_err());
        let workspace = Workspace::new();
        let mut entries = files();
        entries[1].1[156] = 3;
        assert!(
            stage(&workspace, &entries).is_err(),
            "helper must also be GUI"
        );
    }

    #[test]
    fn wait_timeout_and_changed_stage_never_touch_the_installation() {
        let workspace = Workspace::new();
        let prepared = stage(&workspace, &files()).unwrap();
        let installed = tempfile::tempdir().unwrap();
        let request = request(&prepared, installed.path(), true);
        let original_names = directory_names(&request.install_dir);
        let original_manifest = fs::read(request.install_dir.join(MANIFEST)).unwrap();
        let result = wait_then_install(
            &request,
            || anyhow::bail!("fake wait timeout"),
            || panic!("must not launch"),
        );
        assert!(result.is_err());
        assert_eq!(directory_names(&request.install_dir), original_names);
        assert_eq!(fs::read(request.install_dir.join(APP)).unwrap(), fake_pe(1));
        assert_eq!(
            fs::read(request.install_dir.join(MANIFEST)).unwrap(),
            original_manifest
        );
        fs::write(workspace.0.join("payload").join(HELPER), fake_pe(8)).unwrap();
        assert!(wait_then_install(&request, || Ok(()), || panic!("must not launch")).is_err());
        assert_eq!(directory_names(&request.install_dir), original_names);
    }

    #[test]
    fn portable_transaction_and_installer_manifest_preserve_side_files() {
        for with_manifest in [false, true] {
            let workspace = Workspace::new();
            let prepared = stage(&workspace, &files()).unwrap();
            let installed = tempfile::tempdir().unwrap();
            let request = request(&prepared, installed.path(), with_manifest);
            let old_manifest = if with_manifest {
                Some(
                    serde_json::from_slice::<serde_json::Value>(
                        &fs::read(request.install_dir.join(MANIFEST)).unwrap(),
                    )
                    .unwrap(),
                )
            } else {
                None
            };
            let old_names = directory_names(&request.install_dir);
            let launched = std::cell::Cell::new(false);
            wait_then_install(
                &request,
                || {
                    assert_eq!(
                        directory_names(&request.install_dir),
                        old_names,
                        "no writes before parent exit"
                    );
                    Ok(())
                },
                || {
                    launched.set(true);
                    Ok(())
                },
            )
            .unwrap();
            assert!(launched.get());
            assert_eq!(fs::read(request.install_dir.join(APP)).unwrap(), fake_pe(3));
            assert_eq!(
                fs::read(request.install_dir.join(HELPER)).unwrap(),
                fake_pe(4)
            );
            assert_eq!(
                fs::read(request.install_dir.join("user-side-file.dat")).unwrap(),
                b"never change"
            );
            assert!(directory_names(&request.install_dir)
                .iter()
                .any(|name| name.starts_with(".popchat-update-backup-")));
            if let Some(old) = old_manifest {
                let new: serde_json::Value =
                    serde_json::from_slice(&fs::read(request.install_dir.join(MANIFEST)).unwrap())
                        .unwrap();
                for (key, value) in old.as_object().unwrap() {
                    if !["version", "sha256", "updater_sha256"].contains(&key.as_str()) {
                        assert_eq!(&new[key], value, "{key}");
                    }
                }
                assert_eq!(new["version"], "0.3.0");
                assert_eq!(
                    new["sha256"],
                    hash_file(&request.install_dir.join(APP), MAX_FILE_BYTES).unwrap()
                );
                assert_eq!(
                    new["updater_sha256"],
                    hash_file(&request.install_dir.join(HELPER), MAX_FILE_BYTES).unwrap()
                );
            } else {
                assert!(!request.install_dir.join(MANIFEST).exists());
            }
        }
    }

    #[test]
    fn failed_relaunch_rolls_back_both_executables_docs_and_manifest() {
        let workspace = Workspace::new();
        let prepared = stage(&workspace, &files()).unwrap();
        let installed = tempfile::tempdir().unwrap();
        let request = request(&prepared, installed.path(), true);
        let old_manifest = fs::read(request.install_dir.join(MANIFEST)).unwrap();
        assert!(wait_then_install(
            &request,
            || Ok(()),
            || anyhow::bail!("fake relaunch failure")
        )
        .is_err());
        assert_eq!(fs::read(request.install_dir.join(APP)).unwrap(), fake_pe(1));
        assert_eq!(
            fs::read(request.install_dir.join(HELPER)).unwrap(),
            fake_pe(2)
        );
        assert_eq!(
            fs::read(request.install_dir.join("README.md")).unwrap(),
            b"old readme"
        );
        assert_eq!(
            fs::read(request.install_dir.join(MANIFEST)).unwrap(),
            old_manifest
        );
        assert!(!request.install_dir.join("NOTICE.md").exists());
        assert_eq!(
            fs::read(request.install_dir.join("user-side-file.dat")).unwrap(),
            b"never change"
        );
    }

    #[test]
    fn mid_transaction_failure_restores_already_replaced_app() {
        let workspace = Workspace::new();
        let prepared = stage(&workspace, &files()).unwrap();
        let installed = tempfile::tempdir().unwrap();
        let request = request(&prepared, installed.path(), false);
        fs::write(workspace.0.join("payload").join(HELPER), b"changed").unwrap();
        assert!(install_transaction(&request, None, || panic!("must not launch")).is_err());
        assert_eq!(fs::read(request.install_dir.join(APP)).unwrap(), fake_pe(1));
        assert_eq!(
            fs::read(request.install_dir.join(HELPER)).unwrap(),
            fake_pe(2)
        );
    }

    #[test]
    fn rollback_will_not_overwrite_unrelated_external_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("owned.txt");
        fs::write(&path, b"external change").unwrap();
        let entry = BackupEntry {
            target: path.clone(),
            backup: None,
            old_hash: None,
            new_hash: format!("{:x}", Sha256::digest(b"our copy")),
            max: MAX_FILE_BYTES,
        };
        assert!(rollback_entry(&entry).is_err());
        assert_eq!(fs::read(path).unwrap(), b"external change");
    }

    #[test]
    fn rejects_unrecognized_or_changed_manifest_and_binary() {
        let workspace = Workspace::new();
        let prepared = stage(&workspace, &files()).unwrap();
        let installed = tempfile::tempdir().unwrap();
        let request = request(&prepared, installed.path(), true);
        verify_existing(&request).unwrap();
        let path = request.install_dir.join(MANIFEST);
        let original = fs::read(&path).unwrap();
        let mut value: serde_json::Value = serde_json::from_slice(&original).unwrap();
        value["unknown_nested_field"]["keep"] = false.into();
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(
            verify_existing(&request).is_err(),
            "manifest snapshot is bound in the request"
        );
        value["app"] = "another app".into();
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(manifest_snapshot(
            &request.install_dir,
            &request.old_version,
            &request.old_app_sha256
        )
        .is_err());
        fs::write(path, original).unwrap();
        fs::write(request.install_dir.join(APP), fake_pe(7)).unwrap();
        assert!(verify_existing(&request).is_err());
    }

    #[test]
    fn session_is_nonblocking_and_only_returns_true_for_ready() {
        let (tx, receiver) = std::sync::mpsc::channel();
        let mut session = InstallSession {
            state: InstallState::Starting,
            receiver,
        };
        assert!(!session.poll().unwrap());
        tx.send(Ok(())).unwrap();
        assert!(session.poll().unwrap());
        assert!(session.poll().unwrap());
        let (tx, receiver) = std::sync::mpsc::channel();
        let mut session = InstallSession {
            state: InstallState::Starting,
            receiver,
        };
        tx.send(Err("not ready".into())).unwrap();
        assert!(session.poll().is_err());
    }

    #[test]
    fn dropping_ready_objects_does_not_remove_the_stage_or_kill_the_helper() {
        let workspace = Workspace::new();
        let prepared = stage(&workspace, &files()).unwrap();
        let path = prepared.workspace.join("payload").join(APP);
        drop(prepared);
        assert!(
            path.exists(),
            "stage must survive app shutdown; it is not a TempDir owner"
        );
    }
}
