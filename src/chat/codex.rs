//! Local Codex backend: stdio JSON-RPC app-server transport.
//!
//! Port of `CodexAppServerClient.swift` (process management, JSONL transport,
//! ephemeral-thread turn loop) and the streaming-event shape of
//! `CodexResponsesClient.swift`. Both Swift clients emit the same
//! `ChatStreamEvent` stream. The production adapter uses `client.rs` wire types
//! without flattening history or attachment parts.
//!
//! Design notes (all load-bearing, mirroring the Swift original):
//! - One ephemeral `codex app-server` child per turn, started with the machine
//!   tool surfaces disabled (`--disable shell_tool`, `unified_exec`, …) and a
//!   read-only thread. This provider is a chat transport, not an authorization
//!   for Codex to operate on the user's machine. The only exception is Codex's
//!   own `web_search`, which runs on the model backend and follows the globe
//!   toggle via the launch argument.
//! - stdin goes through a **bounded** mpsc channel, so a child that stops
//!   draining applies backpressure to the sender instead of buffering
//!   unboundedly in memory.
//! - Reads use a per-message inactivity timeout (the Swift `InactivityWatchdog`
//!   equivalent): a turn may run long, but must not stay wedged forever after
//!   app-server stops producing protocol traffic.
//! - PopChat never installs, updates, or authenticates Codex; it only starts
//!   the user's own `codex` executable.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use anyhow::Context as _;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStderr, Command};
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;
use tokio::time::timeout;

use super::client::{self, ChatBackend, ProviderConfig, WebAccess, WireContent, WireMessage};
use super::providers;

/// User-facing message when no usable `codex` executable is found.
pub const MISSING_MESSAGE: &str = "Codex is not installed or PopChat cannot find it. Install Codex yourself and run `codex login`. If it is already installed, find its path (`where codex` in Command Prompt) and paste it into the Codex path field in Settings → Providers.";
/// Overall deadline for a readiness probe (`check_ready`).
pub const INSPECT_TIMEOUT: Duration = Duration::from_secs(30);
/// Capacity of the stdin writer channel: a full channel means the child is not
/// draining, and senders wait (backpressure) instead of buffering unboundedly.
const STDIN_CHANNEL_CAPACITY: usize = 16;
const EVENT_CHANNEL_CAPACITY: usize = 64;
const STDERR_TAIL_CAP: usize = 8192;

// ---------------------------------------------------------------------------
// Public event / config types
// ---------------------------------------------------------------------------

/// Stream events for a Codex turn. Same shape as the Swift `ChatStreamEvent`
/// (plus `Activity`, which carries permanent transcript rows such as tool-use
/// notices, distinct from transient `Status` text).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatStreamEvent {
    /// Transient status line (starting up, reasoning, searching…).
    Status(String),
    /// Collapsed reasoning disclosure content (summarized thinking).
    Reasoning(String),
    /// Latest assembled visible text (re-yielded on every delta).
    Partial(String),
    /// Permanent transcript row (tool notices, warnings).
    Activity(String),
    /// Turn finished; carries the final visible text.
    Done(String),
    /// Turn failed; carries a user-facing message.
    Error(String),
}

/// Readiness of the user's own Codex installation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodexStatus {
    Unknown,
    Checking,
    Ready {
        email: Option<String>,
        plan: Option<String>,
    },
    Missing,
    NotSignedIn,
    Failed(String),
}

/// Why a Codex request failed, as data. Callers map UI state from `reason`,
/// never by substring-matching `message` (which is user-facing prose).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexErrorReason {
    Missing,
    NotSignedIn,
    ProtocolFailure,
}

#[derive(Debug)]
pub struct CodexError {
    pub message: String,
    pub reason: CodexErrorReason,
}

impl CodexError {
    fn not_signed_in(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            reason: CodexErrorReason::NotSignedIn,
        }
    }

    fn protocol(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            reason: CodexErrorReason::ProtocolFailure,
        }
    }
}

impl std::fmt::Display for CodexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for CodexError {}

/// Configuration for one Codex client. `executable_path` is the user override
/// from Settings; `None` means "search PATH and well-known locations".
#[derive(Debug, Clone)]
pub struct CodexConfig {
    pub executable_path: Option<PathBuf>,
    pub model: String,
    pub reasoning_effort: Option<String>,
    pub system_prompt: Option<String>,
    pub web_search: bool,
    pub inactivity_timeout: Duration,
}

impl Default for CodexConfig {
    fn default() -> Self {
        Self {
            executable_path: None,
            model: "gpt-5.5".to_string(),
            reasoning_effort: None,
            system_prompt: None,
            web_search: false,
            inactivity_timeout: Duration::from_secs(300),
        }
    }
}

/// Legacy text-only message value. The production adapter accepts the complete
/// `client::WireMessage` history, including image parts.
#[derive(Debug, Clone)]
pub struct TurnMessage {
    pub role: String,
    pub text: String,
}

// ---------------------------------------------------------------------------
// Executable resolution
// ---------------------------------------------------------------------------

fn executable_names() -> &'static [&'static str] {
    // On Windows the npm shim is `codex.cmd`, which must be resolved (and
    // later spawned via `cmd /C`) — probing bare `codex` alone misses it.
    #[cfg(windows)]
    return &["codex.exe", "codex.cmd", "codex.bat", "codex"];
    #[cfg(not(windows))]
    return &["codex"];
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn candidate_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    #[cfg(windows)]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            dirs.push(PathBuf::from(appdata).join("npm"));
        }
        if let Ok(profile) = std::env::var("USERPROFILE") {
            let home = PathBuf::from(profile);
            for sub in [
                ".local/bin",
                ".npm-global/bin",
                ".cargo/bin",
                ".volta/bin",
                ".bun/bin",
            ] {
                dirs.push(home.join(sub));
            }
        }
    }
    #[cfg(not(windows))]
    {
        for fixed in ["/opt/homebrew/bin", "/usr/local/bin"] {
            dirs.push(PathBuf::from(fixed));
        }
        if let Ok(home) = std::env::var("HOME") {
            let home = PathBuf::from(home);
            for sub in [
                ".local/bin",
                ".npm-global/bin",
                ".cargo/bin",
                ".volta/bin",
                ".bun/bin",
            ] {
                dirs.push(home.join(sub));
            }
        }
    }
    dirs
}

/// Resolve the user's `codex` executable: explicit setting first, then
/// `POPCHAT_CODEX_PATH`, then well-known install locations, then `PATH`.
/// Uses `std::env::split_paths` so `PATH` separators are correct per platform.
pub fn resolve_executable(explicit: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = explicit {
        if is_executable_file(path) {
            return Some(path.to_path_buf());
        }
    }
    if let Ok(env) = std::env::var("POPCHAT_CODEX_PATH") {
        let trimmed = env.trim();
        if !trimmed.is_empty() && is_executable_file(Path::new(trimmed)) {
            return Some(PathBuf::from(trimmed));
        }
    }
    for dir in candidate_dirs() {
        for name in executable_names() {
            let candidate = dir.join(name);
            if is_executable_file(&candidate) {
                return Some(candidate);
            }
        }
    }
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        for name in executable_names() {
            let candidate = dir.join(name);
            if is_executable_file(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// JSONL process transport
// ---------------------------------------------------------------------------

type BoxReader = Box<dyn AsyncRead + Unpin + Send + Sync>;
type BoxWriter = Box<dyn AsyncWrite + Unpin + Send + Sync>;

/// One `codex app-server` child with a JSON-RPC-over-JSONL session.
/// Notifications that arrive while a `request()` is outstanding are buffered
/// and served first by `next_message()`, so long-running methods (`turn/start`)
/// can observe their own notifications instead of having them swallowed.
struct AppServerSession {
    next_id: i64,
    stdin_tx: mpsc::Sender<String>,
    lines: Lines<BufReader<BoxReader>>,
    buffered: VecDeque<Value>,
    stderr_tail: Arc<Mutex<Vec<u8>>>,
    child: Option<Child>,
    #[cfg(windows)]
    child_job: Option<ChildJob>,
    _writer: JoinHandle<()>,
    stderr_task: Option<JoinHandle<()>>,
    inactivity_timeout: Duration,
}

/// npm's Windows shim starts a second process. Killing only cmd.exe leaves the
/// real app-server (and its pipes) alive, so bind the owned process tree to a
/// kill-on-close Job Object. No shell/taskkill or unrelated process lookup.
#[cfg(windows)]
pub(crate) struct ChildJob(isize);

#[cfg(windows)]
impl ChildJob {
    pub(crate) fn new(child: &Child) -> anyhow::Result<Self> {
        #[repr(C)]
        #[derive(Default)]
        struct BasicLimits {
            process_time: i64,
            job_time: i64,
            flags: u32,
            min_working_set: usize,
            max_working_set: usize,
            active_processes: u32,
            affinity: usize,
            priority: u32,
            scheduling: u32,
        }
        #[repr(C)]
        #[derive(Default)]
        struct ExtendedLimits {
            basic: BasicLimits,
            io_counters: [u64; 6],
            process_memory: usize,
            job_memory: usize,
            peak_process_memory: usize,
            peak_job_memory: usize,
        }
        #[link(name = "kernel32")]
        extern "system" {
            fn CreateJobObjectW(attributes: *const std::ffi::c_void, name: *const u16) -> isize;
            fn SetInformationJobObject(
                job: isize,
                class: i32,
                info: *const std::ffi::c_void,
                size: u32,
            ) -> i32;
            fn AssignProcessToJobObject(job: isize, process: isize) -> i32;
        }
        // All pointers refer to live, correctly laid out Win32 input structs;
        // this object uniquely owns the returned kernel handle.
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle == 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            let job = Self(handle);
            let mut limits = ExtendedLimits::default();
            limits.basic.flags = 0x0000_2000; // JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
            if SetInformationJobObject(
                handle,
                9,
                &limits as *const _ as *const _,
                std::mem::size_of_val(&limits) as u32,
            ) == 0
            {
                return Err(std::io::Error::last_os_error().into());
            }
            let process = child
                .raw_handle()
                .context("Codex child has no process handle")?;
            if AssignProcessToJobObject(handle, process as isize) == 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            Ok(job)
        }
    }
}

#[cfg(windows)]
impl Drop for ChildJob {
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

fn app_server_args(web_search: bool) -> Vec<String> {
    // The key is an enum (`disabled`/`cached`/`indexed`/`live`); `live` is what
    // `codex --search` sets. A fresh process per turn is what makes the globe
    // toggle a launch argument.
    let search = if web_search { "live" } else { "disabled" };
    [
        "--disable",
        "shell_tool",
        "--disable",
        "unified_exec",
        "--disable",
        "multi_agent",
        "--disable",
        "apps",
        "--disable",
        "plugins",
        "--disable",
        "remote_plugin",
        "--disable",
        "browser_use",
        "--disable",
        "computer_use",
        "--disable",
        "image_generation",
        "-c",
        &format!("web_search=\"{search}\""),
        "-c",
        "mcp_servers={}",
        "-c",
        "tools_view_image=false",
        "app-server",
        "--listen",
        "stdio://",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

fn spawn_command(exe: &Path) -> Command {
    // Rust's Windows process implementation recognizes .cmd/.bat and invokes
    // the system cmd.exe with its batch-specific escaping. Hand-building /C
    // loses quotes in paths with spaces and in web_search="live" arguments.
    Command::new(exe)
}

impl AppServerSession {
    /// Build a session over any byte transport. Production passes the child
    /// stdio halves; tests pass a `tokio::io::duplex` pair (no quota spent).
    fn new(
        reader: impl AsyncRead + Unpin + Send + Sync + 'static,
        writer: impl AsyncWrite + Unpin + Send + Sync + 'static,
    ) -> Self {
        let (tx, mut rx) = mpsc::channel::<String>(STDIN_CHANNEL_CAPACITY);
        let mut writer: BoxWriter = Box::new(writer);
        let handle = tokio::spawn(async move {
            while let Some(line) = rx.recv().await {
                if writer.write_all(line.as_bytes()).await.is_err() {
                    break;
                }
                if writer.flush().await.is_err() {
                    break;
                }
            }
        });
        let reader: BoxReader = Box::new(reader);
        Self {
            next_id: 1,
            stdin_tx: tx,
            lines: BufReader::new(reader).lines(),
            buffered: VecDeque::new(),
            stderr_tail: Arc::new(Mutex::new(Vec::new())),
            child: None,
            #[cfg(windows)]
            child_job: None,
            _writer: handle,
            stderr_task: None,
            inactivity_timeout: Duration::from_secs(300),
        }
    }

    async fn spawn(exe: &Path, web_search: bool) -> anyhow::Result<Self> {
        let mut cmd = spawn_command(exe);
        cmd.args(app_server_args(web_search));
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        // Give the child a PATH that includes its own directory first, so its
        // bundled helpers resolve the way they do in the user's terminal.
        if let Some(dir) = exe.parent() {
            let mut paths = vec![dir.to_path_buf()];
            #[cfg(not(windows))]
            {
                paths.push(PathBuf::from("/opt/homebrew/bin"));
                paths.push(PathBuf::from("/usr/local/bin"));
            }
            if let Some(existing) = std::env::var_os("PATH") {
                paths.extend(std::env::split_paths(&existing));
            }
            if let Ok(joined) = std::env::join_paths(paths) {
                cmd.env("PATH", joined);
            }
        }
        #[cfg(windows)]
        {
            // Keep the helper windowless: it is a background transport.
            // (tokio's Command exposes creation_flags inherently on Windows.)
            cmd.creation_flags(0x0800_0000);
        }
        let mut child = cmd
            .spawn()
            .with_context(|| format!("Couldn't start Codex at {}", exe.display()))?;
        #[cfg(windows)]
        let child_job =
            ChildJob::new(&child).context("Couldn't supervise the Codex process tree")?;
        let stdin = child.stdin.take().context("Codex child has no stdin")?;
        let stdout = child.stdout.take().context("Codex child has no stdout")?;
        let stderr = child.stderr.take().context("Codex child has no stderr")?;
        let mut session = Self::new(stdout, stdin);
        session.child = Some(child);
        #[cfg(windows)]
        {
            session.child_job = Some(child_job);
        }
        // Drain stderr so a verbose app-server cannot fill its pipe and
        // deadlock, while retaining a short tail for failure messages.
        let tail = Arc::clone(&session.stderr_tail);
        session.stderr_task = Some(tokio::spawn(async move {
            drain_stderr(stderr, tail).await;
        }));
        Ok(session)
    }

    /// Serialize one JSON-RPC object onto the child's stdin. Awaits channel
    /// capacity: backpressure when the child stops draining.
    async fn send(&self, object: &Value) -> anyhow::Result<()> {
        let mut line = serde_json::to_string(object)?;
        line.push('\n');
        timeout(self.inactivity_timeout, self.stdin_tx.send(line))
            .await
            .map_err(|_| {
                CodexError::protocol(timeout_message(self.inactivity_timeout.as_secs_f64()))
            })?
            .map_err(|_| anyhow::anyhow!("Codex app-server stdin is closed"))?;
        Ok(())
    }

    /// Start a request without consuming stdout, so notifications for
    /// long-running methods can be handled while the response is pending.
    async fn begin_request(&mut self, method: &str, params: Value) -> anyhow::Result<i64> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({"id": id, "method": method, "params": params}))
            .await?;
        Ok(id)
    }

    async fn notify(&self, method: &str, params: Value) -> anyhow::Result<()> {
        self.send(&json!({"method": method, "params": params}))
            .await
    }

    async fn request(&mut self, method: &str, params: Value) -> anyhow::Result<Value> {
        let id = self.begin_request(method, params).await?;
        loop {
            match self.read_message().await? {
                Some(message) => {
                    if message.get("id").and_then(id_as_i64) == Some(id) {
                        return Ok(message);
                    }
                    self.buffered.push_back(message);
                }
                None => {
                    return Err(CodexError::protocol(format!(
                        "Codex app-server exited while waiting for {method}. Make sure your Codex installation is current.{}",
                        self.stderr_suffix_blocking()
                    ))
                    .into());
                }
            }
        }
    }

    async fn next_message(&mut self) -> anyhow::Result<Option<Value>> {
        if let Some(message) = self.buffered.pop_front() {
            return Ok(Some(message));
        }
        self.read_message().await
    }

    async fn read_message(&mut self) -> anyhow::Result<Option<Value>> {
        let deadline = tokio::time::Instant::now() + self.inactivity_timeout;
        loop {
            match tokio::time::timeout_at(deadline, self.lines.next_line())
                .await
                .map_err(|_| {
                    CodexError::protocol(timeout_message(self.inactivity_timeout.as_secs_f64()))
                })?? {
                Some(line) => {
                    if line.trim().is_empty() {
                        continue;
                    }
                    let message: Value = serde_json::from_str(&line).map_err(|_| {
                        CodexError::protocol(
                            "Codex app-server sent invalid JSON. Try updating Codex.",
                        )
                    })?;
                    if !message.is_object() {
                        return Err(CodexError::protocol(
                            "Codex app-server sent invalid JSON. Try updating Codex.",
                        )
                        .into());
                    }
                    return Ok(Some(message));
                }
                None => return Ok(None),
            }
        }
    }

    fn stderr_suffix_blocking(&self) -> String {
        // Only used on paths where no other task can be appending (session is
        // exclusively owned here); try_lock keeps it non-blocking.
        let text = self
            .stderr_tail
            .try_lock()
            .map(|guard| String::from_utf8_lossy(&guard).trim().to_string())
            .unwrap_or_default();
        if text.is_empty() {
            String::new()
        } else {
            format!(" Codex said: {text}")
        }
    }

    async fn shutdown(mut self) {
        #[cfg(windows)]
        drop(self.child_job.take());
        if let Some(mut child) = self.child.take() {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
    }
}

impl Drop for AppServerSession {
    fn drop(&mut self) {
        // Cancellation can occur at ANY await (initialize, account/read,
        // injection, backpressured stdin or streaming), not just next_message.
        self._writer.abort();
        if let Some(task) = &self.stderr_task {
            task.abort();
        }
        if let Some(child) = &mut self.child {
            let _ = child.start_kill();
        }
    }
}

async fn drain_stderr(mut stderr: ChildStderr, tail: Arc<Mutex<Vec<u8>>>) {
    use tokio::io::AsyncReadExt as _;
    let mut chunk = [0u8; 4096];
    loop {
        match stderr.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let mut guard = tail.lock().await;
                guard.extend_from_slice(&chunk[..n]);
                if guard.len() > STDERR_TAIL_CAP {
                    let overflow = guard.len() - STDERR_TAIL_CAP;
                    guard.drain(..overflow);
                }
            }
        }
    }
}

fn response_result(message: &Value, method: &str) -> Result<Value, CodexError> {
    if let Some(error) = message.get("error") {
        let detail = error
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown protocol error");
        return Err(CodexError::protocol(format!(
            "Codex app-server rejected {method}: {detail}. Try updating Codex."
        )));
    }
    message
        .get("result")
        .filter(|r| r.is_object())
        .cloned()
        .ok_or_else(|| {
            CodexError::protocol(format!(
                "Codex app-server returned an invalid {method} response. Try updating Codex."
            ))
        })
}

/// Lenient JSON integer extraction. Never truncates via `as`: NaN, ±Infinity
/// or out-of-range magnitudes yield `None` instead of trapping or wrapping.
fn id_as_i64(value: &Value) -> Option<i64> {
    match value {
        Value::Number(n) => n
            .as_i64()
            .or_else(|| n.as_u64().and_then(|u| i64::try_from(u).ok()))
            .or_else(|| {
                n.as_f64().and_then(|f| {
                    if f.is_finite() && f.fract() == 0.0 {
                        // Range-check before casting: float-to-int `as` saturates.
                        // i64::MAX rounds UP to 2^63 as f64. Exclude that
                        // endpoint, matching Swift Int64(exactly:).
                        if f >= i64::MIN as f64 && f < -(i64::MIN as f64) {
                            Some(f as i64)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                })
            }),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/// Production `client::ChatBackend` adapter. Each request snapshots the configured
/// executable and owns one ephemeral child for the entire turn.
pub struct CodexBackend {
    executable: Option<PathBuf>,
    inactivity_timeout: Duration,
}

impl CodexBackend {
    pub fn new(executable: Option<PathBuf>) -> Self {
        Self {
            executable,
            inactivity_timeout: Duration::from_secs(300),
        }
    }
}

impl ChatBackend for CodexBackend {
    fn start(
        &self,
        history: Vec<WireMessage>,
        config: ProviderConfig,
        web_access: Option<WebAccess>,
    ) -> mpsc::UnboundedReceiver<client::ChatStreamEvent> {
        let (tx, rx) = mpsc::unbounded_channel();
        let codex_config = CodexConfig {
            executable_path: self.executable.clone(),
            model: config.model.clone(),
            reasoning_effort: config.reasoning_effort,
            web_search: matches!(web_access, Some(WebAccess::CodexNativeSearch)),
            inactivity_timeout: self.inactivity_timeout,
            system_prompt: None, // The actual resolved system message is in history.
        };
        tokio::spawn(async move {
            // Race the whole operation. Dropping the receiver cancels process
            // launch, handshake, blocked request writes and stalled streams.
            tokio::select! {
                biased;
                _ = tx.closed() => {},
                _ = async {
                    let (events_tx, mut events_rx) = mpsc::channel(EVENT_CHANNEL_CAPACITY);
                    let run = async {
                        if !emit(&events_tx, ChatStreamEvent::Status("Starting Codex…".into())).await {
                            return;
                        }
                        let Some(exe) = resolve_executable(codex_config.executable_path.as_deref()) else {
                            emit(&events_tx, ChatStreamEvent::Error(MISSING_MESSAGE.into())).await;
                            return;
                        };
                        match AppServerSession::spawn(&exe, codex_config.web_search).await {
                            Ok(session) => drive_history(session, &codex_config, history, config.model, events_tx).await,
                            Err(error) => {
                                emit(&events_tx, ChatStreamEvent::Error(format!("{error:#}"))).await;
                            }
                        }
                    };
                    let forward = async {
                        while let Some(event) = events_rx.recv().await {
                            let event = match event {
                                ChatStreamEvent::Status(s) => client::ChatStreamEvent::Status(s),
                                ChatStreamEvent::Reasoning(s) => client::ChatStreamEvent::Reasoning(s),
                                ChatStreamEvent::Partial(s) => client::ChatStreamEvent::Partial(s),
                                ChatStreamEvent::Activity(s) => client::ChatStreamEvent::Activity(s),
                                ChatStreamEvent::Done(s) => client::ChatStreamEvent::Done(s),
                                ChatStreamEvent::Error(s) => client::ChatStreamEvent::Error(s),
                            };
                            if tx.send(event).is_err() { break; }
                        }
                    };
                    tokio::join!(run, forward);
                } => {},
            }
        });
        rx
    }
}

/// The local-Codex backend. Owns config; each `stream_turn` spawns one
/// ephemeral child and returns its event stream.
pub struct CodexClient {
    config: CodexConfig,
}

impl CodexClient {
    pub fn new(config: CodexConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &CodexConfig {
        &self.config
    }

    /// Stream one turn for `prompt` on `model`. Compatibility entry point: builds a
    /// single-message history (plus the configured system prompt) and drives
    /// the app-server thread/turn protocol on a background task.
    pub fn stream_turn(
        &self,
        prompt: impl Into<String>,
        model: impl Into<String>,
    ) -> ChatEventStream {
        let (tx, rx) = mpsc::channel::<ChatStreamEvent>(EVENT_CHANNEL_CAPACITY);
        let config = self.config.clone();
        let prompt = prompt.into();
        let model = model.into();
        tokio::spawn(async move {
            let Some(exe) = resolve_executable(config.executable_path.as_deref()) else {
                emit(&tx, ChatStreamEvent::Error(MISSING_MESSAGE.to_string())).await;
                return;
            };
            match AppServerSession::spawn(&exe, config.web_search).await {
                Ok(session) => drive_turn(session, &config, prompt, model, tx).await,
                Err(e) => {
                    emit(
                        &tx,
                        ChatStreamEvent::Error(format!(
                            "Couldn't start Codex at {}: {e:#}",
                            exe.display()
                        )),
                    )
                    .await;
                }
            }
        });
        ChatEventStream { rx }
    }
}

async fn emit(tx: &mpsc::Sender<ChatStreamEvent>, event: ChatStreamEvent) -> bool {
    tx.send(event).await.is_ok()
}

/// Probe readiness: resolve, spawn, initialize, read the ChatGPT account.
/// Never blocks longer than [`INSPECT_TIMEOUT`].
pub async fn check_ready(config: &CodexConfig) -> CodexStatus {
    let Some(exe) = resolve_executable(config.executable_path.as_deref()) else {
        return CodexStatus::Missing;
    };
    let outcome = timeout(INSPECT_TIMEOUT, async {
        let mut session = AppServerSession::spawn(&exe, false)
            .await
            .map_err(|e| CodexError::protocol(format!("Couldn't start Codex: {e:#}")))?;
        initialize(&mut session).await?;
        let account = read_chatgpt_account(&mut session).await?;
        session.shutdown().await;
        Ok::<_, CodexError>(account)
    })
    .await;
    match outcome {
        Err(_) => CodexStatus::Failed(
            "Codex app-server did not answer within 30 seconds. Update Codex and check its login, then try again.".to_string(),
        ),
        Ok(Err(e)) => match e.reason {
            CodexErrorReason::Missing => CodexStatus::Missing,
            CodexErrorReason::NotSignedIn => CodexStatus::NotSignedIn,
            CodexErrorReason::ProtocolFailure => CodexStatus::Failed(e.message),
        },
        Ok(Ok((email, plan))) => CodexStatus::Ready { email, plan },
    }
}

/// Inspect only the user's selected app-server protocol, never its auth/config
/// files. Cancellation drops the owned process; the complete probe has a 30s cap.
pub async fn inspect(executable: Option<PathBuf>) -> anyhow::Result<providers::CodexInspection> {
    timeout(INSPECT_TIMEOUT, async move {
        let exe = resolve_executable(executable.as_deref()).ok_or_else(|| CodexError {
            message: MISSING_MESSAGE.into(), reason: CodexErrorReason::Missing,
        })?;
        let mut session = AppServerSession::spawn(&exe, false).await?;
        initialize(&mut session).await?;
        let (email, plan) = read_chatgpt_account(&mut session).await?;
        let response = session.request("model/list", json!({"includeHidden": false, "limit": 200})).await?;
        let result = response_result(&response, "model/list")?;
        let entries = result.get("data").and_then(Value::as_array).cloned().unwrap_or_default();
        let mut inspection = providers::CodexInspection { email, plan, ..Default::default() };
        // Preserve server order. ProviderStore is responsible for UI sorting.
        inspection.models = entries.iter().filter_map(|e| e.get("id")?.as_str().map(str::to_owned)).collect();
        inspection.default_model = entries.iter().find(|e| e.get("isDefault").and_then(Value::as_bool) == Some(true))
            .and_then(|e| e.get("id")).and_then(Value::as_str).map(str::to_owned);
        for entry in &entries {
            let Some(id) = entry.get("id").and_then(Value::as_str) else { continue };
            let efforts: Vec<String> = entry.get("supportedReasoningEfforts").and_then(Value::as_array)
                .into_iter().flatten().filter_map(|e| e.get("reasoningEffort")?.as_str().map(str::to_owned)).collect();
            if !efforts.is_empty() { inspection.supported_efforts.insert(id.into(), efforts); }
            if let Some(effort) = entry.get("defaultReasoningEffort").and_then(Value::as_str) {
                inspection.default_efforts.insert(id.into(), effort.into());
            }
        }
        if inspection.models.is_empty() {
            return Err(CodexError::protocol("Codex app-server returned no available models. Update Codex and try again.").into());
        }
        // model/list, like the Swift source, supplies efforts but no attachment
        // capabilities; do not manufacture per-model capability metadata.
        session.shutdown().await;
        Ok(inspection)
    }).await.map_err(|_| CodexError::protocol(
        "Codex app-server did not answer within 30 seconds. Update Codex and check its login, then try again."
    ))?
}

/// Preserve typed inspection failures at the ProviderStore publishing boundary.
pub fn inspection_failure(error: &anyhow::Error) -> providers::CodexFailure {
    let reason = match error.downcast_ref::<CodexError>().map(|e| e.reason) {
        Some(CodexErrorReason::Missing) => providers::CodexFailureReason::Missing,
        Some(CodexErrorReason::NotSignedIn) => providers::CodexFailureReason::NotSignedIn,
        _ => providers::CodexFailureReason::ProtocolFailure,
    };
    providers::CodexFailure {
        message: format!("{error:#}"),
        reason,
    }
}

async fn initialize(session: &mut AppServerSession) -> Result<(), CodexError> {
    let response = session
        .request(
            "initialize",
            json!({"clientInfo": {"name": "popchat", "title": "PopChat", "version": "1.0.0"}}),
        )
        .await
        .map_err(|e| CodexError::protocol(format!("Codex app-server initialize failed: {e:#}")))?;
    response_result(&response, "initialize")?;
    // Notification; a failure here means the child is already gone, which the
    // next request will report precisely — don't mask it.
    session
        .notify("initialized", json!({}))
        .await
        .map_err(|e| CodexError::protocol(format!("Codex app-server initialize failed: {e:#}")))?;
    Ok(())
}

async fn read_chatgpt_account(
    session: &mut AppServerSession,
) -> Result<(Option<String>, Option<String>), CodexError> {
    let response = session
        .request("account/read", json!({"refreshToken": false}))
        .await
        .map_err(|e| CodexError::protocol(format!("Codex account/read failed: {e:#}")))?;
    let result = response_result(&response, "account/read")?;
    let account = result.get("account").and_then(|a| a.as_object()).ok_or_else(|| {
        CodexError::not_signed_in(
            "Codex is installed but not signed in. Run `codex login` in a terminal, then check again.",
        )
    })?;
    if account.get("type").and_then(|t| t.as_str()) != Some("chatgpt") {
        return Err(CodexError::not_signed_in(
            "Codex is not using a ChatGPT subscription. Run `codex login` and choose ChatGPT sign-in, then check again.",
        ));
    }
    let email = account
        .get("email")
        .and_then(|e| e.as_str())
        .map(|s| s.to_string());
    let plan = account
        .get("planType")
        .and_then(|p| p.as_str())
        .map(|s| s.to_string());
    Ok((email, plan))
}

// ---------------------------------------------------------------------------
// Turn driver
// ---------------------------------------------------------------------------

struct WireHistorySplit {
    system_prompt: Option<String>,
    prior_messages: Vec<WireMessage>,
    current_message: WireMessage,
}

fn text_content(message: &WireMessage) -> Option<String> {
    match &message.content {
        Some(WireContent::Text(text)) => Some(text.clone()),
        Some(WireContent::Parts(parts)) => Some(
            parts
                .iter()
                .filter_map(|p| p.text.as_deref())
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        None => None,
    }
}

fn split_wire_history(history: &[WireMessage]) -> Result<WireHistorySplit, CodexError> {
    let system_prompt = history
        .iter()
        .find(|m| m.role == "system")
        .and_then(text_content);
    let conversational: Vec<&WireMessage> = history
        .iter()
        .filter(|m| m.role == "user" || m.role == "assistant")
        .collect();
    let last_user = conversational
        .iter()
        .rposition(|m| m.role == "user")
        .ok_or_else(|| CodexError::protocol("The Codex app-server request has no user message."))?;
    Ok(WireHistorySplit {
        system_prompt,
        prior_messages: conversational[..last_user]
            .iter()
            .map(|m| (*m).clone())
            .collect(),
        current_message: conversational[last_user].clone(),
    })
}

/// The Swift source's `responseItem` and `turnInput` mappings are different:
/// injected images use input_image/image_url; turn images use image/url.
/// Images remain owned data URLs in the request; no temporary file lifetime or
/// access to the user's attachment files is required by this transport.
fn wire_parts(message: &WireMessage, current_turn: bool) -> Vec<Value> {
    let text_type = if current_turn {
        "text"
    } else if message.role == "assistant" {
        "output_text"
    } else {
        "input_text"
    };
    match &message.content {
        Some(WireContent::Text(text)) => vec![json!({"type": text_type, "text": text})],
        Some(WireContent::Parts(parts)) => parts
            .iter()
            .filter_map(|part| {
                if let Some(text) = &part.text {
                    Some(json!({"type": text_type, "text": text}))
                } else if let Some(image) = &part.image_url {
                    Some(if current_turn {
                        json!({"type": "image", "url": image.url})
                    } else {
                        json!({"type": "input_image", "image_url": image.url})
                    })
                } else {
                    None
                }
            })
            .collect(),
        None => Vec::new(),
    }
}

fn wire_response_item(message: &WireMessage) -> Value {
    json!({"type": "message", "role": message.role, "content": wire_parts(message, false)})
}

fn app_server_working_directory() -> anyhow::Result<PathBuf> {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(std::env::temp_dir);
    let dir = base.join("PopChat").join("codex-app-server-sandbox");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Ordered assembly of one turn's agentMessage items, keyed by item id:
/// a delta appends to ITS item; `item/completed` text is authoritative for its
/// id (replayed completions settle, never duplicate); a retryable turn error
/// drops in-flight items only, so the retry's re-stream does not glue onto the
/// aborted prefix.
#[derive(Debug, Default)]
struct ItemAssembly {
    entries: Vec<ItemEntry>,
}

#[derive(Debug)]
struct ItemEntry {
    id: String,
    text: String,
    completed: bool,
}

impl ItemAssembly {
    fn delta(&mut self, id: &str, text: &str) {
        match self.entries.iter_mut().rev().find(|e| e.id == id) {
            Some(entry) => {
                // A delta for a settled id is a replay; completed text wins.
                if !entry.completed {
                    entry.text.push_str(text);
                }
            }
            None => self.entries.push(ItemEntry {
                id: id.to_string(),
                text: text.to_string(),
                completed: false,
            }),
        }
    }

    fn completed(&mut self, id: &str, text: &str) {
        match self.entries.iter_mut().rev().find(|e| e.id == id) {
            Some(entry) => {
                if !text.is_empty() {
                    entry.text = text.to_string();
                }
                entry.completed = true;
            }
            None => {
                if !text.is_empty() {
                    self.entries.push(ItemEntry {
                        id: id.to_string(),
                        text: text.to_string(),
                        completed: true,
                    });
                }
            }
        }
    }

    /// Drops in-flight entries; returns whether anything was dropped so the
    /// caller knows the visible snapshot shrank and must be re-yielded.
    fn drop_in_flight(&mut self) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| e.completed);
        self.entries.len() != before
    }

    fn snapshot(&self) -> String {
        self.entries
            .iter()
            .map(|e| e.text.as_str())
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

/// Assembly key for a reasoning delta: item id plus the part index the
/// protocol requires, so separate thoughts under one item stay separate.
fn reasoning_key(params: &Value, index: &str) -> String {
    let item_id = params
        .get("itemId")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let part = params.get(index).and_then(id_as_i64).unwrap_or(0);
    format!("{item_id}#{part}")
}

fn timeout_message(seconds: f64) -> String {
    let duration = if seconds >= 60.0 {
        format!("{} minutes", (seconds / 60.0).round() as u64)
    } else if seconds >= 1.0 {
        format!("{} seconds", seconds.round() as u64)
    } else {
        format!("{} milliseconds", (seconds * 1000.0).round() as u64)
    };
    format!(
        "Codex app-server stopped responding: no protocol event arrived for {duration}. Stop the turn or update Codex, then try again."
    )
}

fn friendly_error(message: &str) -> String {
    let lower = message.to_lowercase();
    if lower.contains("usage limit") || lower.contains("rate limit") || lower.contains("quota") {
        return format!("Your ChatGPT plan's Codex usage limit was reached. {message}");
    }
    format!("Codex app-server: {message}")
}

/// Transcript label for a completed web search. Labeled from `item/completed`
/// only: the begin event carries just a call id, the query arrives with the end.
fn web_search_label(item: &Value) -> String {
    let query = item
        .get("query")
        .and_then(|q| q.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    if query.is_empty() {
        "Codex app-server searched the web.".to_string()
    } else {
        format!("Codex app-server searched the web: {query}")
    }
}

/// Permanent-row label for tool attempts the read-only policy denies.
fn activity_label(item: &Value) -> Option<String> {
    let item_type = item.get("type").and_then(|t| t.as_str())?;
    match item_type {
        "commandExecution" => Some(
            "⚠︎ Codex app-server attempted a local command; PopChat's read-only, no-approval policy applies.".to_string(),
        ),
        "fileChange" => Some(
            "⚠︎ Codex app-server attempted a file change; PopChat's read-only policy applies.".to_string(),
        ),
        "mcpToolCall" | "dynamicToolCall" | "collabAgentToolCall" => Some(
            "⚠︎ Codex app-server attempted a tool call; PopChat did not grant additional permissions.".to_string(),
        ),
        _ => None,
    }
}

/// Core turn protocol over an already-spawned session. Factored out of
/// [`CodexClient::stream_turn`] so tests can drive it over a mock transport
/// (`tokio::io::duplex`) without spending quota.
async fn drive_turn(
    session: AppServerSession,
    config: &CodexConfig,
    prompt: String,
    model: String,
    tx: mpsc::Sender<ChatStreamEvent>,
) {
    if !emit(&tx, ChatStreamEvent::Status("Starting Codex…".into())).await {
        return;
    }
    let mut history = Vec::new();
    if let Some(system) = &config.system_prompt {
        history.push(WireMessage::new(
            "system",
            Some(WireContent::Text(system.clone())),
            None,
            None,
        ));
    }
    history.push(WireMessage::new(
        "user",
        Some(WireContent::Text(prompt)),
        None,
        None,
    ));
    drive_history(session, config, history, model, tx).await;
}

async fn drive_history(
    mut session: AppServerSession,
    config: &CodexConfig,
    history: Vec<WireMessage>,
    model: String,
    tx: mpsc::Sender<ChatStreamEvent>,
) {
    session.inactivity_timeout = config.inactivity_timeout.max(Duration::from_millis(50));
    tokio::select! {
        biased;
        _ = tx.closed() => {},
        _ = drive_history_inner(session, config, history, model, tx.clone()) => {},
    }
}

async fn drive_history_inner(
    mut session: AppServerSession,
    config: &CodexConfig,
    history: Vec<WireMessage>,
    model: String,
    tx: mpsc::Sender<ChatStreamEvent>,
) {
    if tx.is_closed() {
        return;
    }
    if let Err(e) = initialize(&mut session).await {
        emit(&tx, ChatStreamEvent::Error(e.message)).await;
        session.shutdown().await;
        return;
    }
    if let Err(e) = read_chatgpt_account(&mut session).await {
        emit(&tx, ChatStreamEvent::Error(e.message)).await;
        session.shutdown().await;
        return;
    }
    if tx.is_closed() {
        session.shutdown().await;
        return;
    }

    let split = match split_wire_history(&history) {
        Ok(split) => split,
        Err(e) => {
            emit(&tx, ChatStreamEvent::Error(e.message)).await;
            session.shutdown().await;
            return;
        }
    };
    let sandbox_dir = match app_server_working_directory() {
        Ok(dir) => dir,
        Err(e) => {
            emit(
                &tx,
                ChatStreamEvent::Error(format!(
                    "Couldn't prepare the Codex sandbox directory: {e:#}"
                )),
            )
            .await;
            session.shutdown().await;
            return;
        }
    };
    // Say nothing about network when web search is on: the old blanket
    // "no tool network access" line reads as an instruction not to search.
    let boundary = if config.web_search {
        "你正在 PopChat 中提供一般聊天服務，請優先使用繁體中文回覆，除非使用者指定其他語言。不得檢查本機檔案、執行 shell 指令、修改檔案、呼叫 MCP 工具、啟動其他代理，或以其他方式操作使用者電腦。請依對話回答；問題需要最新或外部資訊時，可以使用網路搜尋。PopChat 已將此執行緒設為唯讀檔案系統，且不允許本機工具連網。"
    } else {
        "你正在 PopChat 中提供一般聊天服務，請優先使用繁體中文回覆，除非使用者指定其他語言。不得檢查本機檔案、執行 shell 指令、修改檔案、呼叫 MCP 工具、啟動其他代理，或以其他方式操作使用者電腦。請直接依對話回答。PopChat 已將此執行緒設為唯讀檔案系統，且不允許工具連網。"
    };
    let developer_instructions = [split.system_prompt.as_deref(), Some(boundary)]
        .into_iter()
        .flatten()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");

    let thread_response = match session
        .request(
            "thread/start",
            json!({
                "approvalPolicy": "never",
                "cwd": sandbox_dir.to_string_lossy(),
                "developerInstructions": developer_instructions,
                "ephemeral": true,
                "model": model,
                "sandbox": "read-only",
                "serviceName": "popchat",
            }),
        )
        .await
    {
        Ok(response) => response,
        Err(e) => {
            emit(&tx, ChatStreamEvent::Error(format!("{e:#}"))).await;
            session.shutdown().await;
            return;
        }
    };
    let thread_result = match response_result(&thread_response, "thread/start") {
        Ok(result) => result,
        Err(error) => {
            emit(&tx, ChatStreamEvent::Error(error.message)).await;
            session.shutdown().await;
            return;
        }
    };
    let thread_id = match Some(thread_result)
        .and_then(|r| r.get("thread")?.get("id")?.as_str().map(|s| s.to_string()))
    {
        Some(id) => id,
        None => {
            emit(
                &tx,
                ChatStreamEvent::Error(
                    "Codex app-server returned an invalid thread/start response.".to_string(),
                ),
            )
            .await;
            session.shutdown().await;
            return;
        }
    };

    if !split.prior_messages.is_empty() {
        let items: Vec<Value> = split
            .prior_messages
            .iter()
            .map(wire_response_item)
            .collect();
        let inject = session
            .request(
                "thread/inject_items",
                json!({"threadId": thread_id, "items": items}),
            )
            .await;
        match inject {
            Ok(response) => {
                if let Err(error) = response_result(&response, "thread/inject_items") {
                    emit(&tx, ChatStreamEvent::Error(error.message)).await;
                    session.shutdown().await;
                    return;
                }
            }
            Err(e) => {
                emit(&tx, ChatStreamEvent::Error(format!("{e:#}"))).await;
                session.shutdown().await;
                return;
            }
        }
    }

    let mut turn_params = json!({
        "approvalPolicy": "never",
        "input": wire_parts(&split.current_message, true),
        "model": model,
        "sandboxPolicy": {"type": "readOnly", "networkAccess": false},
        "threadId": thread_id,
    });
    if let Some(effort) = config.reasoning_effort.as_ref() {
        turn_params["effort"] = Value::String(effort.clone());
    }
    // A turn can start emitting notifications before app-server writes the
    // matching response; begin (don't `request`) so deltas stream immediately
    // instead of buffering until the whole answer arrives at once.
    if !emit(
        &tx,
        ChatStreamEvent::Status(format!("Waiting for {model}…")),
    )
    .await
    {
        session.shutdown().await;
        return;
    }
    let turn_id = match session.begin_request("turn/start", turn_params).await {
        Ok(id) => id,
        Err(e) => {
            emit(&tx, ChatStreamEvent::Error(format!("{e:#}"))).await;
            session.shutdown().await;
            return;
        }
    };

    let timeout_secs = config.inactivity_timeout.as_secs_f64().max(0.05);
    let mut items = ItemAssembly::default();
    let mut reasoning_summary = ItemAssembly::default();
    let mut reasoning_raw = ItemAssembly::default();
    let reasoning_snapshot = |summary: &ItemAssembly, raw: &ItemAssembly| {
        let snapshot = summary.snapshot();
        if snapshot.is_empty() {
            raw.snapshot()
        } else {
            snapshot
        }
    };
    let mut finished = false;
    loop {
        // Dropping the stream must stop the turn: `tx.closed()` fires when the
        // last receiver goes away, and killing the child unblocks the reader.
        tokio::select! {
            biased;
            _ = tx.closed() => {
                session.shutdown().await;
                return;
            }
            message = timeout(Duration::from_secs_f64(timeout_secs), session.next_message()) => {
                let message = match message {
                    Err(_) => {
                        emit(&tx, ChatStreamEvent::Error(timeout_message(timeout_secs))).await;
                        finished = true;
                        break;
                    }
                    Ok(Err(e)) => {
                        emit(&tx, ChatStreamEvent::Error(format!("Codex app-server failed: {e:#}"))).await;
                        finished = true;
                        break;
                    }
                    Ok(Ok(None)) => break, // EOF: child exited.
                    Ok(Ok(Some(message))) => message,
                };
                if message.get("id").and_then(id_as_i64) == Some(turn_id) {
                    if let Err(e) = response_result(&message, "turn/start") {
                        emit(&tx, ChatStreamEvent::Error(e.message)).await;
                        finished = true;
                        break;
                    }
                    continue;
                }
                let (Some(method), Some(params)) = (
                    message.get("method").and_then(|m| m.as_str()),
                    message.get("params"),
                ) else {
                    continue;
                };
                let emit_ok = match method {
                    "item/agentMessage/delta" => {
                        match params.get("delta").and_then(|d| d.as_str()) {
                            Some(delta) => {
                                // `itemId` is required; "" is a last-resort key so a
                                // nonconforming server still streams.
                                let item_id = params.get("itemId").and_then(|v| v.as_str()).unwrap_or_default();
                                items.delta(item_id, delta);
                                emit(&tx, ChatStreamEvent::Partial(items.snapshot())).await
                            }
                            None => true,
                        }
                    }
                    "item/reasoning/summaryTextDelta" => {
                        match params.get("delta").and_then(|d| d.as_str()) {
                            Some(delta) => {
                                reasoning_summary.delta(&reasoning_key(params, "summaryIndex"), delta);
                                emit(&tx, ChatStreamEvent::Reasoning(reasoning_snapshot(&reasoning_summary, &reasoning_raw))).await
                            }
                            None => true,
                        }
                    }
                    "item/reasoning/textDelta" => {
                        match params.get("delta").and_then(|d| d.as_str()) {
                            Some(delta) => {
                                reasoning_raw.delta(&reasoning_key(params, "contentIndex"), delta);
                                emit(&tx, ChatStreamEvent::Reasoning(reasoning_snapshot(&reasoning_summary, &reasoning_raw))).await
                            }
                            None => true,
                        }
                    }
                    "item/started" => {
                        let item = params.get("item");
                        match item.and_then(activity_label) {
                            Some(label) => emit(&tx, ChatStreamEvent::Activity(label)).await,
                            None => match item.and_then(|i| i.get("type")).and_then(|t| t.as_str()) {
                                Some("reasoning") => emit(&tx, ChatStreamEvent::Status("Reasoning…".to_string())).await,
                                Some("webSearch") => emit(&tx, ChatStreamEvent::Status("Searching the web…".to_string())).await,
                                _ => true,
                            },
                        }
                    }
                    "item/completed" => {
                        match params.get("item") {
                            Some(item) => match item.get("type").and_then(|t| t.as_str()) {
                                Some("agentMessage") => {
                                    items.completed(
                                        item.get("id").and_then(|v| v.as_str()).unwrap_or_default(),
                                        item.get("text").and_then(|v| v.as_str()).unwrap_or_default(),
                                    );
                                    emit(&tx, ChatStreamEvent::Partial(items.snapshot())).await
                                }
                                Some("webSearch") => emit(&tx, ChatStreamEvent::Activity(web_search_label(item))).await,
                                _ => true,
                            },
                            None => true,
                        }
                    }
                    "error" => {
                        if params.get("willRetry").and_then(|v| v.as_bool()) == Some(true) {
                            // The retry re-delivers the aborted attempt; drop
                            // in-flight text or it would double up.
                            let mut ok = true;
                            if items.drop_in_flight() {
                                ok = emit(&tx, ChatStreamEvent::Partial(items.snapshot())).await;
                            }
                            if reasoning_summary.drop_in_flight() | reasoning_raw.drop_in_flight() {
                                ok = emit(&tx, ChatStreamEvent::Reasoning(reasoning_snapshot(&reasoning_summary, &reasoning_raw))).await && ok;
                            }
                            ok && emit(&tx, ChatStreamEvent::Status("Temporary error — Codex is retrying…".to_string())).await
                        } else {
                            match params.get("error").and_then(|e| e.get("message")).and_then(|m| m.as_str()) {
                                Some(message) => emit(&tx, ChatStreamEvent::Error(friendly_error(message))).await,
                                None => true,
                            }
                        }
                    }
                    "turn/completed" => {
                        let turn = params.get("turn");
                        let status = turn.and_then(|t| t.get("status")).and_then(|s| s.as_str()).unwrap_or("failed");
                        let done = status == "completed" || status == "interrupted";
                        let event = if done {
                            ChatStreamEvent::Done(items.snapshot())
                        } else {
                            let message = turn
                                .and_then(|t| t.get("error"))
                                .and_then(|e| e.get("message"))
                                .and_then(|m| m.as_str())
                                .map(|m| friendly_error(m))
                                .unwrap_or_else(|| friendly_error(&format!("Codex app-server turn failed (status: {status}).")));
                            ChatStreamEvent::Error(message)
                        };
                        let ok = emit(&tx, event).await;
                        finished = true;
                        if !ok {
                            session.shutdown().await;
                            return;
                        }
                        // Terminal event: stop reading. Continuing would block
                        // on the (now idle) child until the inactivity timeout
                        // and emit a spurious second error after Done.
                        break;
                    }
                    _ => true,
                };
                if !emit_ok {
                    session.shutdown().await;
                    return;
                }
            }
        }
    }
    if !finished {
        emit(
            &tx,
            ChatStreamEvent::Error(
                "Codex app-server exited before the response completed. Update Codex and try again."
                    .to_string(),
            ),
        )
        .await;
    }
    session.shutdown().await;
}

// ---------------------------------------------------------------------------
// Event stream handle
// ---------------------------------------------------------------------------

/// Owning handle for a turn's event stream. Implements
/// `futures::Stream<Item = ChatStreamEvent>`; dropping it stops the turn.
pub struct ChatEventStream {
    rx: mpsc::Receiver<ChatStreamEvent>,
}

impl ChatEventStream {
    /// Collect every event until the turn ends. Handy for tests and scripting.
    pub async fn collect_all(mut self) -> Vec<ChatStreamEvent> {
        let mut events = Vec::new();
        while let Some(event) = self.rx.recv().await {
            events.push(event);
        }
        events
    }
}

impl futures::Stream for ChatEventStream {
    type Item = ChatStreamEvent;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.rx.poll_recv(cx)
    }
}

// ---------------------------------------------------------------------------
// Tests — fake transport, no quota
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    // A standalone, std-only executable, compiled once then copied into each
    // test's private directory. It never searches PATH for Codex, reads Codex
    // credentials/config, or accesses the network. Unlike a duplex-only mock,
    // this exercises the production spawn/args/stdio/drop path.
    const FIXTURE_SOURCE: &str = r###"
use std::io::{self, BufRead, Write};
use std::fs::{self, OpenOptions};
fn main() {
    let exe = std::env::current_exe().unwrap();
    let dir = exe.parent().unwrap();
    fs::write(dir.join("pid"), std::process::id().to_string()).unwrap();
    fs::write(dir.join("arguments"), std::env::args().skip(1).collect::<Vec<_>>().join("\n")).unwrap();
    let mode = fs::read_to_string(dir.join("mode")).unwrap();
    let mut log = OpenOptions::new().create(true).append(true).open(dir.join("requests")).unwrap();
    let mut out = io::stdout().lock();
    for line in io::stdin().lock().lines() {
        let line = line.unwrap();
        writeln!(log, "{line}").unwrap(); log.flush().unwrap();
        let method = line.split("\"method\":\"").nth(1).unwrap_or("").split('"').next().unwrap();
        let id = line.split("\"id\":").nth(1).unwrap_or("0").split(|c: char| !c.is_ascii_digit()).next().unwrap();
        if method == "initialized" { continue; }
        if mode == format!("stall:{method}") { loop { std::thread::sleep(std::time::Duration::from_secs(60)); } }
        let result = match method {
            "initialize" => "{}",
            "account/read" if mode == "signed-out" => r#"{"account":null}"#,
            "account/read" => r#"{"account":{"type":"chatgpt","email":"fixture@example.com","planType":"plus"}}"#,
            "model/list" if mode == "empty-models" => r#"{"data":[]}"#,
            "model/list" => r#"{"data":[{"id":"z-model","supportedReasoningEfforts":[{"reasoningEffort":"low"},{"reasoningEffort":"high"},{}],"defaultReasoningEffort":"high"},{"id":"a-model","isDefault":true},{}]}"#,
            "thread/start" if mode == "reject-thread" => {
                writeln!(out, "{{\"id\":{id},\"error\":{{\"code\":401,\"message\":\"fixture rejected thread\"}}}}").unwrap();
                out.flush().unwrap(); continue;
            }
            "thread/start" => r#"{"thread":{"id":"fixture-thread"}}"#,
            "thread/inject_items" => "{}",
            "turn/start" => {
                // Notifications intentionally precede turn/start's response.
                for event in [
                    r#"{"method":"item/reasoning/textDelta","params":{"itemId":"r","contentIndex":0,"delta":"raw"}}"#,
                    r#"{"method":"item/reasoning/summaryTextDelta","params":{"itemId":"r","summaryIndex":0,"delta":"summary"}}"#,
                    r#"{"method":"item/reasoning/summaryTextDelta","params":{"itemId":"r","summaryIndex":1,"delta":"second"}}"#,
                    r#"{"method":"item/agentMessage/delta","params":{"itemId":"first","delta":"draft"}}"#,
                    r#"{"method":"item/completed","params":{"item":{"type":"agentMessage","id":"first","text":"settled"}}}"#,
                    r#"{"method":"item/completed","params":{"item":{"type":"agentMessage","id":"first","text":"settled"}}}"#,
                    r#"{"method":"item/agentMessage/delta","params":{"itemId":"first","delta":"stale"}}"#,
                    r#"{"method":"item/agentMessage/delta","params":{"itemId":"retry","delta":"aborted"}}"#,
                    r#"{"method":"error","params":{"willRetry":true,"error":{"message":"temporary"}}}"#,
                    r#"{"method":"item/agentMessage/delta","params":{"itemId":"retry","delta":"final"}}"#,
                    r#"{"method":"item/started","params":{"item":{"type":"webSearch","id":"search"}}}"#,
                    r#"{"method":"item/completed","params":{"item":{"type":"webSearch","id":"search","query":"  fixture query  "}}}"#,
                    r#"{"method":"item/started","params":{"item":{"type":"commandExecution"}}}"#,
                    r#"{"method":"turn/completed","params":{"turn":{"status":"completed"}}}"#,
                ] { writeln!(out, "{event}").unwrap(); }
                "{}"
            }
            _ => "{}",
        };
        writeln!(out, "{{\"id\":{id},\"result\":{result}}}").unwrap();
        out.flush().unwrap();
        if method == "thread/start" && mode == "stall-before-injection" {
            loop { std::thread::sleep(std::time::Duration::from_secs(60)); }
        }
    }
}
"###;

    fn fixture(mode: &str) -> (tempfile::TempDir, PathBuf) {
        static COMPILED: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
        let compiled = COMPILED.get_or_init(|| {
            let dir = tempfile::tempdir().unwrap();
            let source = dir.path().join("fixture.rs");
            std::fs::write(&source, FIXTURE_SOURCE).unwrap();
            let output = std::process::Command::new("rustc")
                .arg("--edition=2021")
                .arg(&source)
                .arg("-o")
                .arg(dir.path().join("fixture.exe"))
                .output()
                .expect("rustc is required to compile the isolated app-server fixture");
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            dir
        });
        let dir = tempfile::tempdir().unwrap();
        let executable = dir.path().join("app-server fixture.exe");
        std::fs::copy(compiled.path().join("fixture.exe"), &executable).unwrap();
        std::fs::write(dir.path().join("mode"), mode).unwrap();
        (dir, executable)
    }

    fn wire(role: &str, text: &str) -> WireMessage {
        WireMessage::new(role, Some(WireContent::Text(text.into())), None, None)
    }

    fn provider_config() -> ProviderConfig {
        ProviderConfig {
            base_url: String::new(),
            api_key: String::new(),
            model: "fixture-model".into(),
            reasoning_effort: Some("high".into()),
            kind: providers::ProviderKind::CodexAppServer,
        }
    }

    async fn collect_adapter(
        mut rx: mpsc::UnboundedReceiver<client::ChatStreamEvent>,
    ) -> Vec<client::ChatStreamEvent> {
        timeout(Duration::from_secs(5), async {
            let mut events = Vec::new();
            while let Some(event) = rx.recv().await {
                events.push(event);
            }
            events
        })
        .await
        .expect("fixture adapter must terminate")
    }

    async fn wait_for_request(dir: &Path, method: &str) {
        timeout(Duration::from_secs(5), async {
            loop {
                let requests = std::fs::read_to_string(dir.join("requests")).unwrap_or_default();
                if requests.contains(&format!("\"method\":\"{method}\"")) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("fixture did not reach requested protocol stage");
    }

    #[cfg(windows)]
    fn fixture_alive(pid: u32) -> bool {
        #[link(name = "kernel32")]
        extern "system" {
            fn OpenProcess(access: u32, inherit: i32, pid: u32) -> isize;
            fn WaitForSingleObject(handle: *mut std::ffi::c_void, millis: u32) -> u32;
            fn CloseHandle(handle: isize) -> i32;
        }
        // Open only the PID written by this test's isolated fixture.
        unsafe {
            let handle = OpenProcess(0x0010_0000, 0, pid); // SYNCHRONIZE
            if handle == 0 {
                return false;
            }
            let alive = WaitForSingleObject(handle as *mut std::ffi::c_void, 0) == 258; // WAIT_TIMEOUT
            CloseHandle(handle);
            alive
        }
    }

    #[cfg(not(windows))]
    fn fixture_alive(pid: u32) -> bool {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }

    async fn assert_fixture_stopped(dir: &Path) {
        let pid: u32 = std::fs::read_to_string(dir.join("pid"))
            .unwrap()
            .parse()
            .unwrap();
        timeout(Duration::from_secs(3), async {
            while fixture_alive(pid) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("dropping the production receiver must kill its owned child");
    }

    #[tokio::test]
    async fn production_adapter_preserves_history_images_effort_search_and_events() {
        let (dir, executable) = fixture("stream");
        let old_image = "data:image/png;base64,aGlzdG9yeQ==";
        let new_image = "data:image/jpeg;base64,Y3VycmVudA==";
        let history = vec![
            WireMessage::new(
                "system",
                Some(WireContent::Parts(vec![
                    client::WirePart::text("Be brief."),
                    client::WirePart::text("Use facts."),
                ])),
                None,
                None,
            ),
            WireMessage::new(
                "user",
                Some(WireContent::Parts(vec![
                    client::WirePart::text("old question"),
                    client::WirePart::image_data_url(old_image),
                ])),
                None,
                None,
            ),
            wire("assistant", "old answer"),
            wire("tool", "not conversational"),
            WireMessage::new(
                "user",
                Some(WireContent::Parts(vec![
                    client::WirePart::image_data_url(new_image),
                    client::WirePart::text("new question"),
                ])),
                None,
                None,
            ),
            wire("assistant", "trailing assistant excluded, as in Swift"),
        ];
        let backend = CodexBackend::new(Some(executable));
        let events = collect_adapter(backend.start(
            history,
            provider_config(),
            Some(WebAccess::CodexNativeSearch),
        ))
        .await;
        assert_eq!(
            events.last(),
            Some(&client::ChatStreamEvent::Done("settled\n\nfinal".into()))
        );
        assert!(events.contains(&client::ChatStreamEvent::Reasoning("raw".into())));
        assert!(events.contains(&client::ChatStreamEvent::Reasoning(
            "summary\n\nsecond".into()
        )));
        assert!(events.contains(&client::ChatStreamEvent::Reasoning(String::new())));
        assert!(events.contains(&client::ChatStreamEvent::Partial("settled".into())));
        assert!(events.contains(&client::ChatStreamEvent::Status(
            "Temporary error — Codex is retrying…".into()
        )));
        assert!(events.contains(&client::ChatStreamEvent::Activity(
            "Codex app-server searched the web: fixture query".into()
        )));
        assert!(events.iter().any(|e| matches!(e, client::ChatStreamEvent::Activity(s) if s.contains("attempted a local command"))));
        let requests: Vec<Value> = std::fs::read_to_string(dir.path().join("requests"))
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        let params = |method: &str| {
            requests.iter().find(|r| r["method"] == method).unwrap()["params"].clone()
        };
        assert_eq!(
            params("initialize"),
            json!({"clientInfo":{"name":"popchat","title":"PopChat","version":"1.0.0"}})
        );
        assert_eq!(params("account/read"), json!({"refreshToken":false}));
        let thread = params("thread/start");
        assert_eq!(thread["ephemeral"], true);
        assert_eq!(thread["sandbox"], "read-only");
        assert_eq!(thread["approvalPolicy"], "never");
        let instructions = thread["developerInstructions"].as_str().unwrap();
        assert!(instructions.starts_with("Be brief.\nUse facts.\n\n"));
        assert!(instructions.contains("問題需要最新或外部資訊時，可以使用網路搜尋"));
        assert!(instructions.contains("不得檢查本機檔案"));
        assert!(instructions.contains("呼叫 MCP 工具"));
        assert_eq!(
            params("thread/inject_items")["items"],
            json!([
                {"type":"message","role":"user","content":[{"type":"input_text","text":"old question"},{"type":"input_image","image_url":old_image}]},
                {"type":"message","role":"assistant","content":[{"type":"output_text","text":"old answer"}]}
            ])
        );
        let turn = params("turn/start");
        assert_eq!(
            turn["input"],
            json!([{"type":"image","url":new_image},{"type":"text","text":"new question"}])
        );
        assert_eq!(turn["effort"], "high");
        assert_eq!(turn["model"], "fixture-model");
        assert_eq!(
            turn["sandboxPolicy"],
            json!({"type":"readOnly","networkAccess":false})
        );
        let args = std::fs::read_to_string(dir.path().join("arguments")).unwrap();
        assert!(args.contains("web_search=\"live\""));
        assert!(args.contains("--disable\nshell_tool"));
        assert!(args.contains("mcp_servers={}"));
        assert_fixture_stopped(dir.path()).await;
    }

    #[tokio::test]
    async fn production_inspection_maps_server_models_and_efforts_without_guessing() {
        let (dir, executable) = fixture("stream");
        let inspection = inspect(Some(executable)).await.unwrap();
        assert_eq!(inspection.email.as_deref(), Some("fixture@example.com"));
        assert_eq!(inspection.plan.as_deref(), Some("plus"));
        assert_eq!(inspection.models, ["z-model", "a-model"]);
        assert_eq!(inspection.default_model.as_deref(), Some("a-model"));
        assert_eq!(inspection.supported_efforts["z-model"], ["low", "high"]);
        assert_eq!(inspection.default_efforts["z-model"], "high");
        assert!(!inspection.supported_efforts.contains_key("a-model"));
        let log = std::fs::read_to_string(dir.path().join("requests")).unwrap();
        let request: Value = log
            .lines()
            .map(|l| serde_json::from_str::<Value>(l).unwrap())
            .find(|r| r["method"] == "model/list")
            .unwrap();
        assert_eq!(
            request["params"],
            json!({"includeHidden":false,"limit":200})
        );
        assert!(std::fs::read_to_string(dir.path().join("arguments"))
            .unwrap()
            .contains("web_search=\"disabled\""));
        assert_fixture_stopped(dir.path()).await;
    }

    #[tokio::test]
    async fn production_receiver_drop_cancels_handshake_requests_and_stalled_stream() {
        for method in [
            "initialize",
            "account/read",
            "thread/start",
            "thread/inject_items",
            "turn/start",
        ] {
            let (dir, executable) = fixture(&format!("stall:{method}"));
            let backend = CodexBackend::new(Some(executable));
            let rx = backend.start(
                vec![
                    wire("user", "old"),
                    wire("assistant", "answer"),
                    wire("user", "current"),
                ],
                provider_config(),
                None,
            );
            wait_for_request(dir.path(), method).await;
            drop(rx);
            assert_fixture_stopped(dir.path()).await;
        }
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn production_windows_batch_shim_preserves_arguments_and_kills_descendant() {
        let (dir, _executable) = fixture("stall:initialize");
        let shim = dir.path().join("codex fixture.cmd");
        std::fs::write(&shim, "@echo off\r\n\"%~dp0app-server fixture.exe\" %*\r\n").unwrap();
        let backend = CodexBackend::new(Some(shim));
        let rx = backend.start(
            vec![wire("user", "hi")],
            provider_config(),
            Some(WebAccess::CodexNativeSearch),
        );
        wait_for_request(dir.path(), "initialize").await;
        let args = std::fs::read_to_string(dir.path().join("arguments")).unwrap();
        assert!(
            args.contains("web_search=\"live\""),
            "batch arguments: {args}"
        );
        drop(rx);
        assert_fixture_stopped(dir.path()).await;
    }

    #[tokio::test]
    async fn production_handshake_inactivity_and_inspection_cancellation_kill_child() {
        let (dir, executable) = fixture("stall:initialize");
        let mut backend = CodexBackend::new(Some(executable));
        // Leave enough time for Windows/Defender to start the fixture under a
        // parallel full-suite load. The timeout under test is a stalled
        // initialize response, not cold executable startup before PID logging.
        backend.inactivity_timeout = Duration::from_secs(2);
        let rx = backend.start(vec![wire("user", "hi")], provider_config(), None);
        wait_for_request(dir.path(), "initialize").await;
        let events = collect_adapter(rx).await;
        assert!(
            matches!(events.last(), Some(client::ChatStreamEvent::Error(s)) if s.contains("stopped responding"))
        );
        assert_fixture_stopped(dir.path()).await;

        let (dir, executable) = fixture("stall:initialize");
        let task = tokio::spawn(inspect(Some(executable)));
        wait_for_request(dir.path(), "initialize").await;
        task.abort();
        let _ = task.await;
        assert_fixture_stopped(dir.path()).await;
    }

    #[tokio::test]
    async fn production_receiver_drop_cancels_backpressured_image_write() {
        let (dir, executable) = fixture("stall-before-injection");
        let image = format!("data:image/png;base64,{}", "A".repeat(2 * 1024 * 1024));
        let history = vec![
            WireMessage::new(
                "user",
                Some(WireContent::Parts(vec![client::WirePart::image_data_url(
                    image,
                )])),
                None,
                None,
            ),
            wire("assistant", "old answer"),
            wire("user", "current"),
        ];
        let rx = CodexBackend::new(Some(executable)).start(history, provider_config(), None);
        wait_for_request(dir.path(), "thread/start").await;
        // The server has answered thread/start but never reads injection. Its
        // pipe cannot hold this image; the production stdin task blocks here.
        tokio::time::sleep(Duration::from_millis(100)).await;
        drop(rx);
        assert_fixture_stopped(dir.path()).await;
    }

    #[test]
    fn protocol_integer_extraction_never_saturates_or_accepts_fractional_ids() {
        assert_eq!(id_as_i64(&json!(i64::MAX)), Some(i64::MAX));
        assert_eq!(id_as_i64(&json!(i64::MIN)), Some(i64::MIN));
        assert_eq!(id_as_i64(&json!(1.0)), Some(1));
        assert_eq!(id_as_i64(&json!(1.5)), None);
        assert_eq!(id_as_i64(&json!(9_223_372_036_854_775_808.0)), None);
        assert_eq!(id_as_i64(&json!(u64::MAX)), None);
    }

    #[tokio::test]
    async fn production_inspection_and_protocol_errors_preserve_typed_reason_and_detail() {
        let (dir, executable) = fixture("signed-out");
        let error = inspect(Some(executable)).await.unwrap_err();
        assert_eq!(
            inspection_failure(&error).reason,
            providers::CodexFailureReason::NotSignedIn
        );
        assert_fixture_stopped(dir.path()).await;
        let (dir, executable) = fixture("empty-models");
        let error = inspect(Some(executable)).await.unwrap_err();
        assert!(error.to_string().contains("no available models"));
        assert_eq!(
            inspection_failure(&error).reason,
            providers::CodexFailureReason::ProtocolFailure
        );
        assert_fixture_stopped(dir.path()).await;
        let (dir, executable) = fixture("reject-thread");
        let events = collect_adapter(CodexBackend::new(Some(executable)).start(
            vec![wire("user", "hi")],
            provider_config(),
            None,
        ))
        .await;
        assert!(
            matches!(events.last(), Some(client::ChatStreamEvent::Error(s)) if s.contains("fixture rejected thread"))
        );
        // An app-server JSON-RPC 401 is NOT an HTTP/OAuth retry/fallback signal.
        let log = std::fs::read_to_string(dir.path().join("requests")).unwrap();
        assert_eq!(log.matches("\"method\":\"thread/start\"").count(), 1);
        assert!(!log.contains("turn/start"));
        assert_fixture_stopped(dir.path()).await;
    }

    fn test_config(inactivity: Duration) -> CodexConfig {
        CodexConfig {
            executable_path: None,
            model: "gpt-test".to_string(),
            reasoning_effort: None,
            system_prompt: Some("Be brief.".to_string()),
            web_search: false,
            inactivity_timeout: inactivity,
        }
    }

    async fn write_line(
        writer: &mut (impl AsyncWrite + Unpin),
        value: &Value,
    ) -> anyhow::Result<()> {
        let mut line = serde_json::to_string(value)?;
        line.push('\n');
        writer.write_all(line.as_bytes()).await?;
        writer.flush().await?;
        Ok(())
    }

    /// Fake app-server: answers the handshake from the request's own id (so the
    /// test never depends on the client's id sequence), then either streams a
    /// canned turn or stays silent to exercise the inactivity timeout.
    async fn fake_server(io: tokio::io::DuplexStream, respond_to_turn: bool) -> anyhow::Result<()> {
        let (reader, mut writer) = tokio::io::split(io);
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let request: Value = serde_json::from_str(&line)?;
            let id = request.get("id").cloned().unwrap_or(Value::Null);
            let method = request
                .get("method")
                .and_then(|m| m.as_str())
                .unwrap_or_default()
                .to_string();
            match method.as_str() {
                "initialize" => write_line(&mut writer, &json!({"id": id, "result": {}})).await?,
                "initialized" => {}
                "account/read" => {
                    write_line(
                        &mut writer,
                        &json!({"id": id, "result": {"account": {"type": "chatgpt", "email": "test@example.com", "planType": "plus"}}}),
                    )
                    .await?;
                }
                "thread/start" => {
                    write_line(
                        &mut writer,
                        &json!({"id": id, "result": {"thread": {"id": "thread-1"}}}),
                    )
                    .await?;
                }
                "thread/inject_items" => {
                    write_line(&mut writer, &json!({"id": id, "result": {}})).await?;
                }
                "turn/start" => {
                    if respond_to_turn {
                        write_line(
                            &mut writer,
                            &json!({"method": "item/agentMessage/delta", "params": {"itemId": "m1", "delta": "Hello "}}),
                        )
                        .await?;
                        write_line(
                            &mut writer,
                            &json!({"method": "item/reasoning/summaryTextDelta", "params": {"itemId": "r1", "summaryIndex": 0, "delta": "thinking…"}}),
                        )
                        .await?;
                        write_line(
                            &mut writer,
                            &json!({"method": "item/agentMessage/delta", "params": {"itemId": "m1", "delta": "world"}}),
                        )
                        .await?;
                        write_line(
                            &mut writer,
                            &json!({"method": "item/completed", "params": {"item": {"type": "agentMessage", "id": "m1", "text": "Hello world"}}}),
                        )
                        .await?;
                        write_line(
                            &mut writer,
                            &json!({"method": "turn/completed", "params": {"turn": {"status": "completed"}}}),
                        )
                        .await?;
                        write_line(&mut writer, &json!({"id": id, "result": {}})).await?;
                    }
                    // Silent mode: never answer, so the client must time out.
                }
                _ => {}
            }
        }
        Ok(())
    }

    async fn drive_with_fake(respond_to_turn: bool, inactivity: Duration) -> Vec<ChatStreamEvent> {
        let (client_io, server_io) = duplex(256 * 1024);
        tokio::spawn(async move {
            let _ = fake_server(server_io, respond_to_turn).await;
        });
        let (reader, writer) = tokio::io::split(client_io);
        let session = AppServerSession::new(reader, writer);
        let (tx, rx) = mpsc::channel::<ChatStreamEvent>(EVENT_CHANNEL_CAPACITY);
        drive_turn(
            session,
            &test_config(inactivity),
            "Say hello.".to_string(),
            "gpt-test".to_string(),
            tx,
        )
        .await;
        ChatEventStream { rx }.collect_all().await
    }

    #[tokio::test]
    async fn streams_canned_turn_to_done() {
        let events = drive_with_fake(true, Duration::from_secs(5)).await;
        assert!(
            matches!(events.first(), Some(ChatStreamEvent::Status(_))),
            "first event should be a status, got {events:?}"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, ChatStreamEvent::Reasoning(r) if r.contains("thinking"))),
            "reasoning delta should surface, got {events:?}"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, ChatStreamEvent::Partial(p) if p == "Hello world")),
            "assembled partial should surface, got {events:?}"
        );
        assert_eq!(
            events.last(),
            Some(&ChatStreamEvent::Done("Hello world".to_string()))
        );
    }

    #[tokio::test]
    async fn silent_server_yields_timeout_error() {
        let events = drive_with_fake(false, Duration::from_millis(150)).await;
        let last = events.last().expect("should emit at least one event");
        assert!(
            matches!(last, ChatStreamEvent::Error(message) if message.contains("stopped responding")),
            "last event should be a timeout error, got {events:?}"
        );
    }

    #[test]
    fn assembly_keys_deltas_by_item_id() {
        let mut assembly = ItemAssembly::default();
        assembly.delta("a", "Hello ");
        assembly.delta("b", "Other ");
        assembly.delta("a", "world");
        assert_eq!(assembly.snapshot(), "Hello world\n\nOther ");
        assembly.completed("a", "Hello world!");
        assert_eq!(assembly.snapshot(), "Hello world!\n\nOther ");
    }

    #[test]
    fn assembly_replay_never_duplicates() {
        let mut assembly = ItemAssembly::default();
        assembly.delta("a", "half ");
        assembly.completed("a", "full text");
        // Late / replayed deltas and completions settle, never append.
        assembly.delta("a", "stale");
        assembly.completed("a", "full text");
        assert_eq!(assembly.snapshot(), "full text");
    }

    #[test]
    fn assembly_retry_drops_only_in_flight() {
        let mut assembly = ItemAssembly::default();
        assembly.delta("done", "kept");
        assembly.completed("done", "kept");
        assembly.delta("aborted", "half-");
        assert!(assembly.drop_in_flight());
        assert_eq!(assembly.snapshot(), "kept");
        assert!(!assembly.drop_in_flight());
    }

    #[test]
    fn history_split_requires_a_user_message() {
        let history = vec![wire("system", "sys")];
        assert!(split_wire_history(&history).is_err());
        let history = vec![
            wire("system", "sys"),
            wire("user", "first"),
            wire("assistant", "answer"),
            wire("user", "second"),
        ];
        let split = split_wire_history(&history).unwrap();
        assert_eq!(split.system_prompt.as_deref(), Some("sys"));
        assert_eq!(split.prior_messages.len(), 2);
        assert_eq!(
            text_content(&split.current_message).as_deref(),
            Some("second")
        );
    }
}
