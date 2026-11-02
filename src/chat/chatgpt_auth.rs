//! "Sign in with ChatGPT": OAuth 2.1 PKCE flow letting a ChatGPT Plus/Pro
//! subscription pay for model usage instead of an API key. Port of
//! `ChatGPTAuth.swift`. The browser handles login; a one-shot localhost
//! listener (bound to 127.0.0.1 only) catches the redirect. Tokens
//! persist via a [`TokenStore`]; streaming itself lives in `responses.rs` (the
//! `CodexResponsesClient` equivalent) — this module only mints credentials via
//! [`valid_credentials`] / [`refreshed_credentials`].
//!
//! ⚠️ UNOFFICIAL / UNSUPPORTED BACKEND — RISK NOTICE ⚠️
//! This reuses the OAuth client id of OpenAI's Codex CLI against
//! `auth.openai.com` / the `chatgpt.com` Codex backend. That backend is NOT a
//! public API: requests may break without notice when OpenAI changes the flow,
//! and subscription-funded non-interactive use may violate the terms the user
//! agreed to. Same scope as Codex CLI: personal use of the user's own
//! subscription only. If sign-in starts failing after an OpenAI-side change,
//! treat this whole module as suspect first.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Context as _;
use rand::rngs::OsRng;
use rand::TryRngCore as _;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use super::store::{SecretBackend, SecretStore};

// Constants from OpenAI's Codex CLI (the officially sanctioned client of this
// flow). The redirect port is fixed by the client registration — not ours to
// choose.
pub const ISSUER: &str = "https://auth.openai.com";
pub const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub const REDIRECT_PORT: u16 = 1455;
pub const REDIRECT_PATH: &str = "/auth/callback";
pub const SCOPE: &str = "openid profile email offline_access";
pub const ORIGINATOR: &str = "popchat";
pub const USER_AGENT: &str = "PopChat/1.0 (Windows)";

pub const DEFAULT_MODEL: &str = "gpt-6.1-sol";
/// Subscription catalog snapshot checked against OpenAI's Codex model guide
/// on 2026-11-02: https://learn.chatgpt.com/docs/models . Not an account-specific
/// availability probe; plans/workspaces/rollouts may restrict individual models.
/// GPT-5.5/5.4/5.4-mini/Spark have retired from ChatGPT-authenticated Codex.
pub const MODEL_CATALOG: &[&str] = &["gpt-6.1-sol", "gpt-6.1-terra", "gpt-6-astra", "gpt-6-luna"];

/// Only known retired subscription choices are migrated. An offline catalog
/// must not erase a manually entered newer/account-specific model on restart.
pub const RETIRED_SUBSCRIPTION_MODELS: &[&str] = &[
    "gpt-5.5",
    "gpt-5.4",
    "gpt-5.4-mini",
    "gpt-5.3-codex-spark",
    "gpt-5.3-codex",
    "gpt-5.2",
];

/// Hand-maintained per-model reasoning-effort table: the direct subscription
/// backend has no capabilities endpoint, and an effort it rejects turns EVERY
/// turn into a 400 (`reasoning.effort` goes on the wire verbatim).
pub fn supported_reasoning_efforts(model: &str) -> &'static [&'static str] {
    match model {
        "gpt-6.1-sol" | "gpt-6.1-terra" | "gpt-6-astra" | "gpt-6-luna" => {
            &["low", "medium", "high", "xhigh", "max"]
        }
        "gpt-5.5" | "gpt-5.4" | "gpt-5.4-mini" => &["low", "medium", "high", "xhigh"],
        "gpt-5.3-codex-spark" => &["low", "medium", "high", "xhigh"],
        _ => &[],
    }
}

pub fn default_reasoning_effort(model: &str) -> Option<&'static str> {
    match model {
        "gpt-6.1-sol" | "gpt-6.1-terra" => Some("medium"),
        "gpt-6-astra" => Some("low"),
        "gpt-6-luna" => Some("high"),
        "gpt-5.5" | "gpt-5.4" | "gpt-5.4-mini" => Some("medium"),
        "gpt-5.3-codex-spark" => Some("high"),
        _ => None,
    }
}

/// Error from the token endpoint. `status` lets refresh distinguish a
/// revoked/expired grant (4xx — sign out) from a transient failure (keep the
/// tokens so a later retry can succeed).
#[derive(Debug)]
pub struct AuthError {
    pub message: String,
    pub status: Option<u16>,
}

impl AuthError {
    fn new(message: impl Into<String>, status: Option<u16>) -> Self {
        Self {
            message: message.into(),
            status,
        }
    }
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for AuthError {}

// ---------------------------------------------------------------------------
// Stored tokens
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: String,
    pub id_token: String,
    pub account_id: String,
    /// Unix seconds at which the access token expires.
    pub expires_at: u64,
    pub email: Option<String>,
    pub plan: Option<String>,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Token persistence. Tests inject an in-memory implementation; production
/// uses [`NativeTokenStore`] and never writes plaintext OAuth tokens.
pub trait TokenStore {
    fn load(&self) -> Option<Tokens>;
    fn save(&self, tokens: &Tokens) -> anyhow::Result<()>;
    fn delete(&self) -> anyhow::Result<()>;
}

pub const SECRETS_ACCOUNT: &str = "chatgpt-oauth";
const PENDING_ACCOUNT: &str = "chatgpt-oauth.pending";
// keyring's Windows password backend uses UTF-16, not UTF-8. Keep each
// entry below CRED_MAX_CREDENTIAL_BLOB_SIZE (2560 bytes), including emoji.
const CHUNK_UTF16_UNITS: usize = 1024;
const MAX_TOKEN_CHUNKS: usize = 4096;
// Separately constructed UI/backend adapters still serialize read/rotate/delete
// within this process. SecretStore applies its isolated-profile namespace to
// EVERY account below, including the manifest, journal and generation chunks.
static NATIVE_TOKEN_LOCK: Mutex<()> = Mutex::new(());

trait SecretEntries: Send + Sync {
    fn get(&self, account: &str) -> Option<String>;
    fn set(&mut self, value: &str, account: &str) -> anyhow::Result<()>;
    fn delete(&mut self, account: &str) -> anyhow::Result<()>;
}

impl SecretEntries for SecretStore {
    fn get(&self, account: &str) -> Option<String> {
        SecretStore::get(self, account)
    }
    fn set(&mut self, value: &str, account: &str) -> anyhow::Result<()> {
        SecretStore::set(self, value, account)
    }
    fn delete(&mut self, account: &str) -> anyhow::Result<()> {
        SecretStore::delete(self, account)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct ChunkSet {
    generation: String,
    chunks: usize,
    digest: String,
}

impl ChunkSet {
    fn account(&self, index: usize) -> String {
        format!("{SECRETS_ACCOUNT}.{}.{index}", self.generation)
    }

    fn valid(&self) -> bool {
        self.generation.len() == 32
            && self.generation.bytes().all(|b| b.is_ascii_hexdigit())
            && (1..=MAX_TOKEN_CHUNKS).contains(&self.chunks)
            && self.digest.len() == 43
            && self
                .digest
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct TokenManifest {
    version: u8,
    active: Option<ChunkSet>,
    // Retain cleanup references until a later transaction/sign-out. This also
    // permits a signed-out tombstone to survive a failed credential deletion.
    retired: Vec<ChunkSet>,
}

fn parse_manifest(raw: &str) -> Option<TokenManifest> {
    let manifest: TokenManifest = serde_json::from_str(raw).ok()?;
    (manifest.version == 1
        && manifest.retired.len() <= 3
        && manifest
            .active
            .iter()
            .chain(&manifest.retired)
            .all(ChunkSet::valid))
    .then_some(manifest)
}

fn token_digest(json: &str) -> String {
    base64url_encode(&Sha256::digest(json.as_bytes()))
}

fn token_chunks(json: &str) -> Vec<&str> {
    let mut chunks = Vec::new();
    let mut start = 0;
    let mut units = 0;
    for (offset, ch) in json.char_indices() {
        if units + ch.len_utf16() > CHUNK_UTF16_UNITS {
            chunks.push(&json[start..offset]);
            start = offset;
            units = 0;
        }
        units += ch.len_utf16();
    }
    if start < json.len() {
        chunks.push(&json[start..]);
    }
    chunks
}

fn set_verified(secrets: &mut dyn SecretEntries, value: &str, account: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        value.encode_utf16().count() * 2 <= 2560,
        "OAuth credential entry exceeds the native storage limit"
    );
    secrets.set(value, account)?;
    anyhow::ensure!(
        secrets.get(account).as_deref() == Some(value),
        "OAuth credential write could not be verified"
    );
    Ok(())
}

fn delete_chunks(secrets: &mut dyn SecretEntries, set: &ChunkSet) -> anyhow::Result<()> {
    // Continue after a failed deletion: the journal/manifest keeps the entire
    // generation discoverable for a later cleanup attempt.
    let mut failure = None;
    for index in 0..set.chunks {
        if let Err(error) = secrets.delete(&set.account(index)) {
            failure = Some(error);
        }
    }
    match failure {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn pending_set(secrets: &dyn SecretEntries) -> anyhow::Result<Option<ChunkSet>> {
    let Some(raw) = secrets.get(PENDING_ACCOUNT) else {
        return Ok(None);
    };
    let set: ChunkSet =
        serde_json::from_str(&raw).context("Invalid OAuth credential transaction journal")?;
    anyhow::ensure!(set.valid(), "Invalid OAuth credential transaction journal");
    Ok(Some(set))
}

fn rollback_chunks(secrets: &mut dyn SecretEntries, set: &ChunkSet) -> anyhow::Result<()> {
    delete_chunks(secrets, set)?;
    secrets.delete(PENDING_ACCOUNT)
}

fn save_chunked_tokens(secrets: &mut dyn SecretEntries, tokens: &Tokens) -> anyhow::Result<()> {
    let previous = secrets.get(SECRETS_ACCOUNT);
    let old = previous.as_deref().and_then(parse_manifest);
    // Recover a crash/failed write first. A journal matching the active
    // generation means the commit succeeded, so never delete those chunks.
    if let Some(pending) = pending_set(secrets)? {
        if old.as_ref().and_then(|m| m.active.as_ref()) != Some(&pending) {
            delete_chunks(secrets, &pending)
                .context("Could not clean up an interrupted OAuth save")?;
        }
        secrets.delete(PENDING_ACCOUNT)?;
    }
    if let Some(old) = &old {
        for retired in &old.retired {
            if old.active.as_ref() != Some(retired) {
                delete_chunks(secrets, retired)
                    .context("Could not clean up prior OAuth credentials")?;
            }
        }
    }
    let json = serde_json::to_string(tokens)?;
    let chunks = token_chunks(&json);
    anyhow::ensure!(
        !chunks.is_empty() && chunks.len() <= MAX_TOKEN_CHUNKS,
        "OAuth token data is too large"
    );
    let generation = ChunkSet {
        generation: uuid::Uuid::new_v4().simple().to_string(),
        chunks: chunks.len(),
        digest: token_digest(&json),
    };
    // A small journal is durable BEFORE writing any chunk. Failed rollback or
    // a crash cannot orphan untracked access/refresh token fragments.
    set_verified(
        secrets,
        &serde_json::to_string(&generation)?,
        PENDING_ACCOUNT,
    )?;
    for (index, chunk) in chunks.iter().enumerate() {
        if let Err(error) = set_verified(secrets, chunk, &generation.account(index)) {
            let cleanup = rollback_chunks(secrets, &generation);
            return Err(error).context(match cleanup { Ok(()) => "Could not stage OAuth credentials; previous tokens retained".to_string(), Err(e) => format!("Could not stage OAuth credentials; previous tokens retained; cleanup pending: {e:#}") });
        }
    }
    let manifest = TokenManifest {
        version: 1,
        active: Some(generation.clone()),
        retired: old
            .as_ref()
            .and_then(|m| m.active.clone())
            .into_iter()
            .collect(),
    };
    if let Err(error) = set_verified(secrets, &serde_json::to_string(&manifest)?, SECRETS_ACCOUNT) {
        // SecretStore may report failure AFTER a native write (e.g. legacy
        // cleanup/verification). Restore the prior manifest before removing
        // any staged chunks. If restore is uncertain, keep both generations
        // plus journal: whichever manifest is visible remains reconstructible.
        let restored = match previous.as_deref() {
            Some(raw) => set_verified(secrets, raw, SECRETS_ACCOUNT),
            None => secrets.delete(SECRETS_ACCOUNT),
        };
        if restored.is_ok() {
            let _ = rollback_chunks(secrets, &generation);
        }
        return Err(error).context("Could not commit OAuth credentials; staged native data retained if rollback needs retry");
    }
    // The new generation is now authoritative. Retired references remain in
    // the manifest even if cleanup fails, so sign-out can finish the work.
    for retired in &manifest.retired {
        delete_chunks(secrets, retired)
            .context("OAuth tokens saved, but previous native chunks could not be removed")?;
    }
    secrets
        .delete(PENDING_ACCOUNT)
        .context("OAuth tokens saved, but transaction cleanup is pending")
}

fn load_chunked_tokens(secrets: &dyn SecretEntries) -> Option<Tokens> {
    let raw = secrets.get(SECRETS_ACCOUNT)?;
    // Backward compatibility with a single native entry from older builds.
    if let Ok(tokens) = serde_json::from_str::<Tokens>(&raw) {
        return Some(tokens);
    }
    let set = parse_manifest(&raw)?.active?;
    let mut json = String::new();
    for index in 0..set.chunks {
        json.push_str(&secrets.get(&set.account(index))?);
    }
    if token_digest(&json) != set.digest {
        return None;
    }
    serde_json::from_str(&json).ok()
}

fn delete_chunked_tokens(secrets: &mut dyn SecretEntries) -> anyhow::Result<()> {
    let raw = secrets.get(SECRETS_ACCOUNT);
    let manifest = raw.as_deref().and_then(parse_manifest);
    let mut retired: Vec<ChunkSet> = manifest
        .into_iter()
        .flat_map(|m| m.active.into_iter().chain(m.retired))
        .collect();
    if let Some(pending) = pending_set(secrets)? {
        if !retired.contains(&pending) {
            retired.push(pending);
        }
    }
    if !retired.is_empty() {
        // Commit signed-out state BEFORE deleting chunks. If deletion fails,
        // load() stays signed out and a future delete can retry every fragment.
        let tombstone = TokenManifest {
            version: 1,
            active: None,
            retired: retired.clone(),
        };
        set_verified(
            secrets,
            &serde_json::to_string(&tombstone)?,
            SECRETS_ACCOUNT,
        )?;
        let mut failure = None;
        for set in &retired {
            if let Err(error) = delete_chunks(secrets, set) {
                failure = Some(error);
            }
        }
        if let Some(error) = failure {
            return Err(error).context("Signed out, but native OAuth chunk cleanup needs retry");
        }
    }
    secrets.delete(PENDING_ACCOUNT)?;
    secrets.delete(SECRETS_ACCOUNT)
}

/// Native SecretStore adapter. Explicitly select Keyring even on unsupported
/// platforms: failure must never silently create plaintext token storage.
#[derive(Clone)]
pub struct NativeTokenStore {
    secrets: Arc<Mutex<Box<dyn SecretEntries>>>,
}

impl Default for NativeTokenStore {
    fn default() -> Self {
        Self {
            secrets: Arc::new(Mutex::new(Box::new(SecretStore::with_backend(
                SecretBackend::Keyring,
            )))),
        }
    }
}

impl NativeTokenStore {
    /// Inherent convenience method for UI callers (no trait import required).
    pub fn load(&self) -> Option<Tokens> {
        <Self as TokenStore>::load(self)
    }
}

impl TokenStore for NativeTokenStore {
    fn load(&self) -> Option<Tokens> {
        let _transaction = NATIVE_TOKEN_LOCK.lock().ok()?;
        let secrets = self.secrets.lock().ok()?;
        load_chunked_tokens(secrets.as_ref())
    }

    fn save(&self, tokens: &Tokens) -> anyhow::Result<()> {
        let _transaction = NATIVE_TOKEN_LOCK
            .lock()
            .map_err(|_| anyhow::anyhow!("Credential transaction lock poisoned"))?;
        let mut secrets = self
            .secrets
            .lock()
            .map_err(|_| anyhow::anyhow!("Credential store lock poisoned"))?;
        save_chunked_tokens(secrets.as_mut(), tokens)
    }

    fn delete(&self) -> anyhow::Result<()> {
        let _transaction = NATIVE_TOKEN_LOCK
            .lock()
            .map_err(|_| anyhow::anyhow!("Credential transaction lock poisoned"))?;
        let mut secrets = self
            .secrets
            .lock()
            .map_err(|_| anyhow::anyhow!("Credential store lock poisoned"))?;
        delete_chunked_tokens(secrets.as_mut())
    }
}

/// Compatibility name retained for existing callers. The path is no longer
/// used for persistence: all writes now go to native credential storage.
pub struct FileTokenStore {
    native: NativeTokenStore,
}

impl FileTokenStore {
    pub fn new(_path: PathBuf) -> Self {
        Self {
            native: NativeTokenStore::default(),
        }
    }

    pub fn default_path() -> PathBuf {
        let base = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .unwrap_or_else(std::env::temp_dir);
        base.join("PopChat").join("chatgpt-oauth.json")
    }
}

impl TokenStore for FileTokenStore {
    fn load(&self) -> Option<Tokens> {
        self.native.load()
    }

    fn save(&self, tokens: &Tokens) -> anyhow::Result<()> {
        self.native.save(tokens)
    }

    fn delete(&self) -> anyhow::Result<()> {
        self.native.delete()
    }
}

// ---------------------------------------------------------------------------
// Interactive sign-in: login URL (PKCE) + callback + exchange
// ---------------------------------------------------------------------------

/// PKCE login challenge: open `url` in the browser, then redeem the captured
/// code with `verifier` via [`exchange_code`].
pub struct LoginChallenge {
    pub url: String,
    pub verifier: String,
    pub state: String,
}

pub fn redirect_uri() -> String {
    format!("http://localhost:{REDIRECT_PORT}{REDIRECT_PATH}")
}

/// Builds the authorize URL plus its PKCE verifier/state. Pure function of
/// secure randomness — unit-testable without a browser.
pub fn login_url() -> anyhow::Result<LoginChallenge> {
    let verifier = base64url_encode(&random_bytes(64)?);
    let challenge = base64url_encode(Sha256::digest(verifier.as_bytes()).as_slice());
    let state = base64url_encode(&random_bytes(32)?);
    // base64url output is already URL-safe; only the redirect URI and scope
    // need percent-encoding.
    let url = format!(
        "{ISSUER}/oauth/authorize?response_type=code&client_id={CLIENT_ID}\
         &redirect_uri={}&scope={}&code_challenge={challenge}\
         &code_challenge_method=S256&state={state}\
         &id_token_add_organizations=true&codex_cli_simplified_flow=true\
         &originator={ORIGINATOR}",
        percent_encode(&redirect_uri()),
        percent_encode(SCOPE),
    );
    Ok(LoginChallenge {
        url,
        verifier,
        state,
    })
}

/// Runs the full browser flow: opens the login URL, waits for the localhost
/// redirect, exchanges the code, persists the tokens. Dropping the returned
/// future cancels the wait (the listener is torn down with it).
pub async fn sign_in(store: &impl TokenStore) -> anyhow::Result<Tokens> {
    let challenge = login_url()?;
    // The fixed redirect port MUST be bound before launching the browser.
    // Both listener and accepted connections are owned by this future, so
    // dropping it (including browser-open failure) tears everything down.
    let listener = bind_callback(REDIRECT_PORT).await?;
    open::that(&challenge.url)
        .map_err(|e| anyhow::anyhow!("Couldn't open the browser for ChatGPT sign-in: {e:#}"))?;
    let code = callback_server(listener, &challenge.state).await?;
    let tokens = exchange_code(&code, &challenge.verifier).await?;
    store.save(&tokens)?;
    Ok(tokens)
}

pub fn sign_out(store: &impl TokenStore) -> anyhow::Result<()> {
    store.delete()
}

// ---------------------------------------------------------------------------
// Localhost callback listener (loopback only, cancellation-owned connections)
// ---------------------------------------------------------------------------

fn success_page() -> String {
    "<html><body style=\"font-family:sans-serif;text-align:center;padding-top:80px\"><h2>Signed in</h2><p>You can close this tab and return to PopChat.</p></body></html>".to_string()
}

fn failure_page(message: &str) -> String {
    format!(
        "<html><body style=\"font-family:sans-serif;text-align:center;padding-top:80px\"><h2>Sign-in failed</h2><p>{}</p></body></html>",
        html_escape(message)
    )
}

fn parse_callback(line: &str, expected_state: &str) -> Option<anyhow::Result<String>> {
    let mut parts = line.split_whitespace();
    if parts.next()? != "GET" {
        return None;
    }
    let url = reqwest::Url::parse(&format!("http://localhost{}", parts.next()?)).ok()?;
    if url.path() != REDIRECT_PATH {
        return None;
    }
    let query = |name: &str| {
        url.query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, v)| v.into_owned())
    };
    // Validate the nonce BEFORE even handling an OAuth error. Forged/stray
    // callbacks must not terminate a legitimate login.
    if query("state").as_deref() != Some(expected_state) {
        return None;
    }
    if let Some(error) = query("error") {
        return Some(Err(anyhow::anyhow!(
            "OpenAI refused the sign-in: {error} ({})",
            query("error_description").unwrap_or_else(|| "no details".into())
        )));
    }
    Some(
        query("code")
            .ok_or_else(|| anyhow::anyhow!("Redirect arrived without an authorization code.")),
    )
}

async fn bind_callback(port: u16) -> anyhow::Result<tokio::net::TcpListener> {
    tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await.map_err(|e| {
        anyhow::anyhow!("Couldn't listen on localhost:{port} for the sign-in redirect ({e:#}). Is another app (or Codex CLI login) using that port?")
    })
}

async fn callback_server(
    listener: tokio::net::TcpListener,
    expected_state: &str,
) -> anyhow::Result<String> {
    let (tx, mut rx) = tokio::sync::oneshot::channel();
    let sender = Arc::new(Mutex::new(Some(tx)));
    // JoinSet aborts every connection on drop, including partially sent request
    // lines. No detached axum connection tasks survive cancellation.
    let mut connections = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            result = &mut rx => return result.context("Sign-in ended before the browser redirect arrived")?,
            Some(_) = connections.join_next(), if !connections.is_empty() => {},
            accepted = listener.accept() => {
                let (mut socket, _) = accepted.context("sign-in callback server failed")?;
                let state = expected_state.to_owned();
                let sender = sender.clone();
                connections.spawn(async move {
                    let mut buffer = Vec::new();
                    let mut chunk = [0u8; 1024];
                    let outcome = loop {
                        match socket.read(&mut chunk).await {
                            Ok(0) | Err(_) => return,
                            Ok(count) => buffer.extend_from_slice(&chunk[..count]),
                        }
                        if let Some(end) = buffer.windows(2).position(|b| b == b"\r\n") {
                            break std::str::from_utf8(&buffer[..end]).ok().and_then(|line| parse_callback(line, &state));
                        }
                        if buffer.len() > 16 * 1024 { break None; }
                    };
                    let (status, body, completion) = match outcome {
                        None => ("404 Not Found", "PopChat: nothing here.".into(), None),
                        Some(result) => {
                            // Claim exactly once; a racing connection cannot overwrite
                            // the first accepted code/error.
                            let tx = sender.lock().ok().and_then(|mut guard| guard.take());
                            let body = match &result { Ok(_) => success_page(), Err(e) => failure_page(&e.to_string()) };
                            ("200 OK", body, tx.map(|tx| (tx, result)))
                        }
                    };
                    let response = format!("HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                    let _ = socket.write_all(response.as_bytes()).await;
                    let _ = socket.shutdown().await;
                    if let Some((tx, result)) = completion { let _ = tx.send(result); }
                });
            }
        }
    }
}

/// One-shot HTTP server: waits for the browser redirect, replies with a tiny
/// confirmation page, and hands back the authorization code.
pub async fn wait_for_callback(expected_state: &str) -> anyhow::Result<String> {
    callback_server(bind_callback(REDIRECT_PORT).await?, expected_state).await
}

// ---------------------------------------------------------------------------
// Token exchange & refresh
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    id_token: Option<String>,
    expires_in: Option<u64>,
}

fn http_client() -> anyhow::Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent(USER_AGENT)
        .build()
        .context("couldn't build the auth HTTP client")
}

async fn token_request(form: &[(&str, &str)]) -> Result<TokenResponse, AuthError> {
    let http = http_client().map_err(|e| AuthError::new(format!("{e:#}"), None))?;
    token_request_at(&http, &format!("{ISSUER}/oauth/token"), form).await
}

async fn token_request_at(
    http: &reqwest::Client,
    endpoint: &str,
    form: &[(&str, &str)],
) -> Result<TokenResponse, AuthError> {
    let response = http
        .post(endpoint)
        .form(form)
        .send()
        .await
        .map_err(|e| AuthError::new(format!("Token endpoint request failed: {e:#}"), None))?;
    let status = response.status().as_u16();
    let body = response
        .text()
        .await
        .unwrap_or_else(|_| "<unreadable body>".to_string());
    if status != 200 {
        let preview: String = body.chars().take(300).collect();
        return Err(AuthError::new(
            format!("Token endpoint returned HTTP {status}: {preview}"),
            Some(status),
        ));
    }
    serde_json::from_str(&body)
        .map_err(|e| AuthError::new(format!("Couldn't parse the token response: {e:#}"), None))
}

pub async fn exchange_code(code: &str, verifier: &str) -> anyhow::Result<Tokens> {
    let response = token_request(&[
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", &redirect_uri()),
        ("client_id", CLIENT_ID),
        ("code_verifier", verifier),
    ])
    .await
    .map_err(|e| anyhow::anyhow!(e.message))?;
    build_tokens(&response, None)
}

/// Fresh access token + ChatGPT account id, refreshing when the stored token
/// expires within a minute. Called by `client.rs` before each Responses round.
pub async fn valid_credentials(store: &impl TokenStore) -> anyhow::Result<(String, String)> {
    credentials_at(store, false, &format!("{ISSUER}/oauth/token")).await
}

/// Forces a refresh regardless of stored expiry — used after the backend
/// rejects a token with 401 (revoked, or clock skew).
pub async fn refreshed_credentials(store: &impl TokenStore) -> anyhow::Result<(String, String)> {
    credentials_at(store, true, &format!("{ISSUER}/oauth/token")).await
}

/// Shared production refresh path, with an injectable token endpoint for
/// loopback HTTP tests. Never changes the issuer used for interactive login.
pub(crate) async fn credentials_at(
    store: &(impl TokenStore + ?Sized),
    force_refresh: bool,
    endpoint: &str,
) -> anyhow::Result<(String, String)> {
    let Some(tokens) = store.load() else {
        return Err(anyhow::anyhow!(
            "Not signed in to ChatGPT — open Settings → Providers and sign in."
        ));
    };
    if !force_refresh && tokens.expires_at > now_secs().saturating_add(60) {
        return Ok((tokens.access_token, tokens.account_id));
    }
    let refreshed = refresh(&tokens, store, endpoint).await?;
    Ok((refreshed.access_token, refreshed.account_id))
}

async fn refresh(
    tokens: &Tokens,
    store: &(impl TokenStore + ?Sized),
    endpoint: &str,
) -> anyhow::Result<Tokens> {
    let http = http_client()?;
    let response = match token_request_at(
        &http,
        endpoint,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", &tokens.refresh_token),
            ("client_id", CLIENT_ID),
        ],
    )
    .await
    {
        Ok(response) => response,
        Err(e) => {
            // Only clear the session when the refresh grant itself is invalid
            // (400/401: invalid_grant). Transient failures — 429, 5xx, network
            // errors — keep the tokens so a later retry can succeed.
            if matches!(e.status, Some(400) | Some(401)) {
                let _ = store.delete();
            }
            return Err(anyhow::anyhow!(
                "ChatGPT session expired and refresh failed ({}). Sign in again in Settings → Providers.",
                e.message
            ));
        }
    };
    let updated = build_tokens(&response, Some(tokens))?;
    store.save(&updated)?;
    Ok(updated)
}

fn build_tokens(response: &TokenResponse, previous: Option<&Tokens>) -> anyhow::Result<Tokens> {
    let Some(refresh_token) = response
        .refresh_token
        .clone()
        .or_else(|| previous.map(|t| t.refresh_token.clone()))
    else {
        return Err(anyhow::anyhow!(
            "OpenAI returned no refresh token — sign in again."
        ));
    };
    let id_token = response
        .id_token
        .clone()
        .or_else(|| previous.map(|t| t.id_token.clone()))
        .unwrap_or_default();
    let id_claims = decode_jwt_claims(&id_token);
    let access_claims = decode_jwt_claims(&response.access_token);

    // Same extraction order as opencode: id token first, then access token;
    // top-level claim, then the proprietary auth claim, then first org.
    let Some(account_id) =
        account_id_in(id_claims.as_ref()).or_else(|| account_id_in(access_claims.as_ref()))
    else {
        return Err(anyhow::anyhow!(
            "Sign-in succeeded but no ChatGPT account id was found in the token. Is this a ChatGPT (not API-platform) account?"
        ));
    };

    let auth_claim = access_claims
        .as_ref()
        .and_then(|c| c.get("https://api.openai.com/auth"))
        .or_else(|| {
            id_claims
                .as_ref()
                .and_then(|c| c.get("https://api.openai.com/auth"))
        });
    Ok(Tokens {
        access_token: response.access_token.clone(),
        refresh_token,
        id_token,
        account_id,
        expires_at: now_secs().saturating_add(response.expires_in.unwrap_or(3600)),
        email: id_claims
            .as_ref()
            .and_then(|c| c.get("email"))
            .and_then(|e| e.as_str())
            .map(|s| s.to_string())
            .or_else(|| previous.and_then(|t| t.email.clone())),
        plan: auth_claim
            .and_then(|a| a.get("chatgpt_plan_type"))
            .and_then(|p| p.as_str())
            .map(|s| s.to_string())
            .or_else(|| previous.and_then(|t| t.plan.clone())),
    })
}

fn account_id_in(claims: Option<&Value>) -> Option<String> {
    let claims = claims?;
    if let Some(id) = claims.get("chatgpt_account_id").and_then(|v| v.as_str()) {
        return Some(id.to_string());
    }
    if let Some(id) = claims
        .get("https://api.openai.com/auth")
        .and_then(|a| a.get("chatgpt_account_id"))
        .and_then(|v| v.as_str())
    {
        return Some(id.to_string());
    }
    claims
        .get("organizations")
        .and_then(|o| o.as_array())
        .and_then(|orgs| orgs.first())
        .and_then(|org| org.get("id"))
        .and_then(|id| id.as_str())
        .map(|s| s.to_string())
}

fn decode_jwt_claims(jwt: &str) -> Option<Value> {
    let mut segments = jwt.split('.');
    let (_header, payload, _signature) = (segments.next()?, segments.next()?, segments.next()?);
    if segments.next().is_some() {
        return None;
    }
    let bytes = base64url_decode(payload).ok()?;
    serde_json::from_slice(&bytes).ok()
}

// ---------------------------------------------------------------------------
// Small codecs (no base64 crate in the dependency set — hand-rolled)
// ---------------------------------------------------------------------------

const B64URL_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

pub fn base64url_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let mut buf = [0u8; 3];
        buf[..chunk.len()].copy_from_slice(chunk);
        let n = ((buf[0] as u32) << 16) | ((buf[1] as u32) << 8) | (buf[2] as u32);
        out.push(B64URL_ALPHABET[((n >> 18) & 63) as usize] as char);
        out.push(B64URL_ALPHABET[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(B64URL_ALPHABET[((n >> 6) & 63) as usize] as char);
        }
        if chunk.len() > 2 {
            out.push(B64URL_ALPHABET[(n & 63) as usize] as char);
        }
    }
    out
}

fn b64url_value(byte: u8) -> Option<u32> {
    match byte {
        b'A'..=b'Z' => Some((byte - b'A') as u32),
        b'a'..=b'z' => Some((byte - b'a' + 26) as u32),
        b'0'..=b'9' => Some((byte - b'0' + 52) as u32),
        b'-' => Some(62),
        b'_' => Some(63),
        _ => None,
    }
}

pub fn base64url_decode(input: &str) -> anyhow::Result<Vec<u8>> {
    // JWT payloads sometimes arrive padded; tolerate trailing '='.
    let bytes = input.trim_end_matches('=').as_bytes();
    if bytes.len() % 4 == 1 {
        return Err(anyhow::anyhow!("invalid base64url length"));
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.chunks(4) {
        let mut buf = [0u32; 4];
        for (i, &b) in chunk.iter().enumerate() {
            buf[i] =
                b64url_value(b).ok_or_else(|| anyhow::anyhow!("invalid base64url character"))?;
        }
        let n = (buf[0] << 18) | (buf[1] << 12) | (buf[2] << 6) | buf[3];
        out.push((n >> 16) as u8);
        if chunk.len() > 2 {
            out.push((n >> 8) as u8);
        }
        if chunk.len() > 3 {
            out.push(n as u8);
        }
    }
    Ok(out)
}

fn random_bytes(count: usize) -> anyhow::Result<Vec<u8>> {
    let mut bytes = vec![0u8; count];
    let mut rng = OsRng;
    // Never fall back to zeros — that would void PKCE/state.
    rng.try_fill_bytes(&mut bytes)
        .map_err(|e| anyhow::anyhow!("Couldn't generate secure random data for sign-in: {e}"))?;
    Ok(bytes)
}

fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for &byte in value.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

// ---------------------------------------------------------------------------
// Tests — pure codecs and loopback HTTP only; never real credentials/browser
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #[test]
    fn subscription_catalog_uses_current_models_with_valid_reasoning_defaults() {
        assert_eq!(super::DEFAULT_MODEL, "gpt-6.1-sol");
        assert_eq!(
            super::MODEL_CATALOG,
            &["gpt-6.1-sol", "gpt-6.1-terra", "gpt-6-astra", "gpt-6-luna"]
        );
        for model in super::MODEL_CATALOG {
            let efforts = super::supported_reasoning_efforts(model);
            assert!(efforts.contains(&super::default_reasoning_effort(model).unwrap()));
            assert!(efforts.contains(&"max"));
            assert!(
                !efforts.contains(&"none"),
                "subscription picker conservatively exposes reasoning modes only"
            );
        }
        assert!(super::supported_reasoning_efforts("unknown-model").is_empty());
    }
    use super::*;

    #[test]
    fn native_store_is_send_sync_without_opening_credentials() {
        fn assert_store<T: TokenStore + Default + Send + Sync>() {}
        assert_store::<NativeTokenStore>();
    }

    #[derive(Default)]
    struct FakeSecretState {
        entries: std::collections::BTreeMap<String, String>,
        largest_blob: usize,
        fail_chunk_index: Option<usize>,
        // true = persist then report failure; false = fail before writing.
        root_failures: std::collections::VecDeque<bool>,
        fail_next_chunk_delete: bool,
    }

    struct FakeSecrets(Arc<Mutex<FakeSecretState>>);

    fn is_chunk_account(account: &str) -> bool {
        account.starts_with("chatgpt-oauth.") && account != PENDING_ACCOUNT
    }

    impl SecretEntries for FakeSecrets {
        fn get(&self, account: &str) -> Option<String> {
            self.0.lock().unwrap().entries.get(account).cloned()
        }
        fn set(&mut self, value: &str, account: &str) -> anyhow::Result<()> {
            let mut state = self.0.lock().unwrap();
            let blob_bytes = value.encode_utf16().count() * 2;
            anyhow::ensure!(blob_bytes <= 2560, "fake WinCred: oversized blob");
            state.largest_blob = state.largest_blob.max(blob_bytes);
            if is_chunk_account(account)
                && state.fail_chunk_index.is_some()
                && account
                    .rsplit('.')
                    .next()
                    .and_then(|part| part.parse::<usize>().ok())
                    == state.fail_chunk_index
            {
                state.fail_chunk_index = None;
                anyhow::bail!("injected chunk write failure");
            }
            if account == SECRETS_ACCOUNT {
                if let Some(write_first) = state.root_failures.pop_front() {
                    if write_first {
                        state.entries.insert(account.into(), value.into());
                    }
                    anyhow::bail!("injected manifest write failure");
                }
            }
            state.entries.insert(account.into(), value.into());
            Ok(())
        }
        fn delete(&mut self, account: &str) -> anyhow::Result<()> {
            let mut state = self.0.lock().unwrap();
            if is_chunk_account(account) && state.fail_next_chunk_delete {
                state.fail_next_chunk_delete = false;
                anyhow::bail!("injected chunk deletion failure");
            }
            state.entries.remove(account);
            Ok(())
        }
    }

    fn fake_native_store() -> (NativeTokenStore, Arc<Mutex<FakeSecretState>>) {
        let state = Arc::new(Mutex::new(FakeSecretState::default()));
        let store = NativeTokenStore {
            secrets: Arc::new(Mutex::new(Box::new(FakeSecrets(state.clone())))),
        };
        (store, state)
    }

    fn long_tokens(marker: &str) -> Tokens {
        Tokens {
            access_token: format!("{marker}.{}.signature", "access-jwt".repeat(1200)),
            id_token: format!("{marker}.{}.signature", "id-jwt".repeat(1500)),
            refresh_token: format!("{marker}-{}", "refresh-token".repeat(350)),
            account_id: "test-account".into(),
            expires_at: u64::MAX,
            email: Some("測試🦀".repeat(800)),
            plan: Some("plus".into()),
        }
    }

    fn assert_only_active_generation(state: &FakeSecretState) {
        let manifest = parse_manifest(state.entries.get(SECRETS_ACCOUNT).unwrap()).unwrap();
        let active = manifest.active.unwrap();
        assert_eq!(state.entries.len(), active.chunks + 1);
        assert!(!state.entries.contains_key(PENDING_ACCOUNT));
        for index in 0..active.chunks {
            assert!(state.entries.contains_key(&active.account(index)));
        }
    }

    #[test]
    fn native_chunks_handle_long_jwts_and_unicode_rotate_and_sign_out() {
        let (store, state) = fake_native_store();
        let first = long_tokens("first");
        assert!(serde_json::to_string(&first).unwrap().len() > 2560 * 10);
        store.save(&first).unwrap();
        assert_eq!(
            serde_json::to_value(store.load().unwrap()).unwrap(),
            serde_json::to_value(&first).unwrap()
        );
        let old_keys: Vec<_> = state
            .lock()
            .unwrap()
            .entries
            .keys()
            .filter(|key| is_chunk_account(key))
            .cloned()
            .collect();
        assert!(old_keys.len() > 10);
        let second = long_tokens("second");
        store.save(&second).unwrap();
        assert_eq!(store.load().unwrap().access_token, second.access_token);
        {
            let state = state.lock().unwrap();
            assert!(state.largest_blob <= 2048); // 1024 UTF-16 units; headroom below 2560.
            assert!(old_keys.iter().all(|key| !state.entries.contains_key(key)));
            assert_only_active_generation(&state);
        }
        store.delete().unwrap();
        assert!(store.load().is_none());
        assert!(state.lock().unwrap().entries.is_empty());
    }

    #[test]
    fn failed_chunk_stage_rolls_back_without_modifying_active_generation() {
        let (store, state) = fake_native_store();
        store.save(&long_tokens("old")).unwrap();
        let snapshot = state.lock().unwrap().entries.clone();
        state.lock().unwrap().fail_chunk_index = Some(2);
        assert!(store.save(&long_tokens("new")).is_err());
        assert_eq!(state.lock().unwrap().entries, snapshot);
        assert_eq!(
            store.load().unwrap().access_token,
            long_tokens("old").access_token
        );
    }

    #[test]
    fn failed_manifest_commit_before_or_after_write_restores_previous_tokens() {
        for write_first in [false, true] {
            let (store, state) = fake_native_store();
            store.save(&long_tokens("old")).unwrap();
            let snapshot = state.lock().unwrap().entries.clone();
            state.lock().unwrap().root_failures.push_back(write_first);
            assert!(store.save(&long_tokens("new")).is_err());
            assert_eq!(state.lock().unwrap().entries, snapshot);
            assert_eq!(
                store.load().unwrap().access_token,
                long_tokens("old").access_token
            );
        }
    }

    #[test]
    fn uncertain_commit_keeps_both_generations_and_journal_for_recovery() {
        let (store, state) = fake_native_store();
        store.save(&long_tokens("old")).unwrap();
        // Commit is durable but reports failure; restoring the prior pointer
        // also fails. The now-active new chunks must NOT be deleted.
        state.lock().unwrap().root_failures.extend([true, false]);
        assert!(store.save(&long_tokens("new")).is_err());
        assert_eq!(
            store.load().unwrap().access_token,
            long_tokens("new").access_token
        );
        assert!(state.lock().unwrap().entries.contains_key(PENDING_ACCOUNT));
        store.save(&long_tokens("third")).unwrap();
        assert_eq!(
            store.load().unwrap().access_token,
            long_tokens("third").access_token
        );
        assert_only_active_generation(&state.lock().unwrap());
    }

    #[test]
    fn failed_stage_cleanup_is_journaled_and_retried_without_orphaning_chunks() {
        let (store, state) = fake_native_store();
        store.save(&long_tokens("old")).unwrap();
        {
            let mut state = state.lock().unwrap();
            state.fail_chunk_index = Some(2);
            state.fail_next_chunk_delete = true;
        }
        assert!(store.save(&long_tokens("new")).is_err());
        assert_eq!(
            store.load().unwrap().access_token,
            long_tokens("old").access_token
        );
        assert!(state.lock().unwrap().entries.contains_key(PENDING_ACCOUNT));
        store.save(&long_tokens("retry")).unwrap();
        assert_only_active_generation(&state.lock().unwrap());
    }

    #[test]
    fn failed_signout_cleanup_leaves_signed_out_tombstone_and_retryable_chunks() {
        let (store, state) = fake_native_store();
        store.save(&long_tokens("old")).unwrap();
        state.lock().unwrap().fail_next_chunk_delete = true;
        assert!(store.delete().is_err());
        assert!(store.load().is_none());
        assert!(state.lock().unwrap().entries.contains_key(SECRETS_ACCOUNT));
        store.delete().unwrap();
        assert!(state.lock().unwrap().entries.is_empty());
    }

    #[test]
    fn cleanup_failure_after_rotation_keeps_new_tokens_and_all_cleanup_references() {
        let (store, state) = fake_native_store();
        store.save(&long_tokens("old")).unwrap();
        state.lock().unwrap().fail_next_chunk_delete = true;
        assert!(store.save(&long_tokens("new")).is_err());
        assert_eq!(
            store.load().unwrap().access_token,
            long_tokens("new").access_token
        );
        // Sign-out must remove both the active tokens and any retired/staged
        // fragments, even though the rotation reported a cleanup failure.
        store.delete().unwrap();
        assert!(store.load().is_none());
        assert!(state.lock().unwrap().entries.is_empty());
    }

    #[test]
    fn missing_or_mixed_generation_chunks_are_not_returned_as_tokens() {
        let (store, state) = fake_native_store();
        store.save(&long_tokens("old")).unwrap();
        let (key, original) = {
            let state = state.lock().unwrap();
            let (key, value) = state
                .entries
                .iter()
                .find(|(key, _)| is_chunk_account(key))
                .unwrap();
            (key.clone(), value.clone())
        };
        state
            .lock()
            .unwrap()
            .entries
            .insert(key.clone(), "corrupted".into());
        assert!(store.load().is_none());
        state.lock().unwrap().entries.insert(key.clone(), original);
        assert!(store.load().is_some());
        state.lock().unwrap().entries.remove(&key);
        assert!(store.load().is_none());
    }

    #[test]
    fn prior_single_native_entry_loads_and_migrates_on_next_save() {
        let (store, state) = fake_native_store();
        let legacy = test_tokens();
        state.lock().unwrap().entries.insert(
            SECRETS_ACCOUNT.into(),
            serde_json::to_string(&legacy).unwrap(),
        );
        assert_eq!(store.load().unwrap().refresh_token, legacy.refresh_token);
        store.save(&long_tokens("new")).unwrap();
        assert_only_active_generation(&state.lock().unwrap());
        assert_eq!(
            store.load().unwrap().access_token,
            long_tokens("new").access_token
        );
    }

    #[test]
    fn callback_checks_state_before_errors_and_requires_our_route() {
        for line in [
            "GET /auth/callback?state=forged&error=access_denied HTTP/1.1",
            "GET /auth/callback?code=abc HTTP/1.1",
            "GET /favicon.ico?state=nonce HTTP/1.1",
            "POST /auth/callback?state=nonce&code=abc HTTP/1.1",
        ] {
            assert!(parse_callback(line, "nonce").is_none());
        }
        assert_eq!(
            parse_callback(
                "GET /auth/callback?state=nonce&code=a%2Bb HTTP/1.1",
                "nonce"
            )
            .unwrap()
            .unwrap(),
            "a+b"
        );
        assert!(parse_callback(
            "GET /auth/callback?state=nonce&error=denied&error_description=nope HTTP/1.1",
            "nonce"
        )
        .unwrap()
        .unwrap_err()
        .to_string()
        .contains("denied (nope)"));
        assert!(
            parse_callback("GET /auth/callback?state=nonce HTTP/1.1", "nonce")
                .unwrap()
                .is_err()
        );
        assert!(failure_page("<script>\"&").contains("&lt;script&gt;&quot;&amp;"));
    }

    #[tokio::test]
    async fn callback_listener_ignores_stray_and_accepts_split_request_line() {
        let listener = bind_callback(0).await.unwrap();
        let address = listener.local_addr().unwrap();
        assert!(address.ip().is_loopback());
        let server = tokio::spawn(callback_server(listener, "nonce"));
        let http = reqwest::Client::new();
        let stray = http
            .get(format!(
                "http://{address}/auth/callback?state=forged&error=denied"
            ))
            .send()
            .await
            .unwrap();
        assert_eq!(stray.status(), 404);
        assert!(!server.is_finished());
        let mut socket = tokio::net::TcpStream::connect(address).await.unwrap();
        socket
            .write_all(b"GET /auth/callback?state=nonce&co")
            .await
            .unwrap();
        tokio::task::yield_now().await;
        socket
            .write_all(b"de=code123 HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .unwrap();
        let mut body = String::new();
        socket.read_to_string(&mut body).await.unwrap();
        assert!(body.starts_with("HTTP/1.1 200 OK"));
        assert!(body.contains("Signed in"));
        assert_eq!(server.await.unwrap().unwrap(), "code123");
    }

    #[tokio::test]
    async fn callback_cancellation_closes_listener_and_partial_connections() {
        let listener = bind_callback(0).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(callback_server(listener, "nonce"));
        let mut socket = tokio::net::TcpStream::connect(address).await.unwrap();
        socket.write_all(b"GET /auth/call").await.unwrap();
        tokio::task::yield_now().await;
        server.abort();
        assert!(server.await.unwrap_err().is_cancelled());
        let mut byte = [0];
        let read = tokio::time::timeout(Duration::from_secs(2), socket.read(&mut byte))
            .await
            .unwrap();
        assert!(matches!(read, Ok(0) | Err(_)));
        drop(socket);
        assert!(tokio::net::TcpStream::connect(address).await.is_err());
        let rebound = tokio::net::TcpListener::bind(address).await.unwrap();
        drop(rebound);
    }

    #[derive(Default)]
    struct MemoryStore(Mutex<Option<Tokens>>);

    impl TokenStore for MemoryStore {
        fn load(&self) -> Option<Tokens> {
            self.0.lock().unwrap().clone()
        }
        fn save(&self, tokens: &Tokens) -> anyhow::Result<()> {
            *self.0.lock().unwrap() = Some(tokens.clone());
            Ok(())
        }
        fn delete(&self) -> anyhow::Result<()> {
            *self.0.lock().unwrap() = None;
            Ok(())
        }
    }

    fn test_tokens() -> Tokens {
        Tokens {
            access_token: "cached-access".into(),
            refresh_token: "test-refresh".into(),
            id_token: format!(
                "e30.{}.sig",
                base64url_encode(
                    br#"{"chatgpt_account_id":"test-account","email":"test@example.invalid"}"#
                )
            ),
            account_id: "test-account".into(),
            expires_at: 0,
            email: Some("test@example.invalid".into()),
            plan: Some("plus".into()),
        }
    }

    #[tokio::test]
    async fn refresh_invalid_grants_delete_but_transient_failures_retain_tokens() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        for status in [400, 401, 429, 500, 503] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/token"))
                .respond_with(ResponseTemplate::new(status).set_body_string("test failure"))
                .expect(1)
                .mount(&server)
                .await;
            let store = MemoryStore::default();
            store.save(&test_tokens()).unwrap();
            let error = credentials_at(&store, false, &format!("{}/token", server.uri()))
                .await
                .unwrap_err();
            assert!(error.to_string().contains("refresh failed"));
            assert_eq!(store.load().is_none(), matches!(status, 400 | 401));
        }
    }

    #[tokio::test]
    async fn refresh_keeps_optional_claims_and_rotates_tokens() {
        use wiremock::matchers::{body_string_contains, header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/token"))
            .and(header("user-agent", USER_AGENT))
            .and(body_string_contains("grant_type=refresh_token"))
            .and(body_string_contains("refresh_token=test-refresh"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "refreshed-access", "refresh_token": "rotated-refresh", "expires_in": 3600,
            }))).expect(1).mount(&server).await;
        let store = MemoryStore::default();
        store.save(&test_tokens()).unwrap();
        assert_eq!(
            credentials_at(&store, false, &format!("{}/token", server.uri()))
                .await
                .unwrap(),
            ("refreshed-access".into(), "test-account".into())
        );
        let saved = store.load().unwrap();
        assert_eq!(saved.refresh_token, "rotated-refresh");
        assert_eq!(saved.email.as_deref(), Some("test@example.invalid"));
        assert_eq!(saved.plan.as_deref(), Some("plus"));
        assert!(saved.expires_at > now_secs() + 60);
        // A valid cached token never contacts the injected (invalid) endpoint.
        assert_eq!(
            credentials_at(&store, false, "not-an-endpoint")
                .await
                .unwrap()
                .0,
            "refreshed-access"
        );
    }

    #[test]
    fn base64url_roundtrip() {
        for len in [0, 1, 2, 3, 4, 5, 32, 64, 100] {
            let data: Vec<u8> = (0..len as u8).collect();
            let encoded = base64url_encode(&data);
            assert!(!encoded.contains(['+', '/', '=']));
            assert_eq!(base64url_decode(&encoded).unwrap(), data);
        }
    }

    #[test]
    fn pkce_challenge_matches_rfc7636_vector() {
        // RFC 7636 §B: verifier -> S256 challenge. (Cross-checked against
        // .NET's SHA256+base64: 32 digest bytes must yield 43 base64url chars.)
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = base64url_encode(Sha256::digest(verifier.as_bytes()).as_slice());
        assert_eq!(challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn login_url_carries_pkce_and_client_params() {
        let challenge = login_url().unwrap();
        assert!(challenge
            .url
            .starts_with("https://auth.openai.com/oauth/authorize?"));
        assert!(challenge
            .url
            .contains("client_id=app_EMoamEEZ73f0CkXaXp7hrann"));
        assert!(challenge.url.contains("code_challenge_method=S256"));
        assert!(challenge.url.contains("codex_cli_simplified_flow=true"));
        assert!(challenge.url.contains("originator=popchat"));
        // Verifier (64 bytes) and state (32 bytes) encode to fixed lengths.
        assert_eq!(challenge.verifier.len(), 86);
        assert_eq!(challenge.state.len(), 43);
        let expected_challenge =
            base64url_encode(Sha256::digest(challenge.verifier.as_bytes()).as_slice());
        assert!(challenge
            .url
            .contains(&format!("code_challenge={expected_challenge}")));
    }

    #[test]
    fn jwt_claims_decode_and_account_extraction() {
        fn segment(json: &str) -> String {
            base64url_encode(json.as_bytes())
        }
        let header = segment(r#"{"alg":"RS256"}"#);
        let payload = segment(
            r#"{"email":"user@example.com","chatgpt_account_id":"acct-123","https://api.openai.com/auth":{"chatgpt_plan_type":"plus"}}"#,
        );
        let token = format!("{header}.{payload}.sig");
        let claims = decode_jwt_claims(&token).unwrap();
        assert_eq!(account_id_in(Some(&claims)).as_deref(), Some("acct-123"));

        let response = TokenResponse {
            access_token: token.clone(),
            refresh_token: Some("refresh-1".to_string()),
            id_token: Some(token),
            expires_in: Some(3600),
        };
        let tokens = build_tokens(&response, None).unwrap();
        assert_eq!(tokens.account_id, "acct-123");
        assert_eq!(tokens.email.as_deref(), Some("user@example.com"));
        assert_eq!(tokens.plan.as_deref(), Some("plus"));
        assert!(tokens.expires_at > now_secs());
    }

    #[test]
    fn build_tokens_rejects_missing_account() {
        let header = base64url_encode(br#"{"alg":"RS256"}"#);
        let payload = base64url_encode(br#"{"email":"nobody@example.com"}"#);
        let token = format!("{header}.{payload}.sig");
        let response = TokenResponse {
            access_token: token,
            refresh_token: Some("refresh-1".to_string()),
            id_token: None,
            expires_in: None,
        };
        assert!(build_tokens(&response, None).is_err());
    }

    #[test]
    fn build_tokens_keeps_previous_refresh_token() {
        let header = base64url_encode(br#"{"alg":"RS256"}"#);
        let payload = base64url_encode(br#"{"chatgpt_account_id":"acct-9"}"#);
        let token = format!("{header}.{payload}.sig");
        let previous = Tokens {
            access_token: "old".to_string(),
            refresh_token: "prev-refresh".to_string(),
            id_token: String::new(),
            account_id: "acct-9".to_string(),
            expires_at: 0,
            email: Some("kept@example.com".to_string()),
            plan: None,
        };
        let response = TokenResponse {
            access_token: token,
            refresh_token: None,
            id_token: None,
            expires_in: None,
        };
        let tokens = build_tokens(&response, Some(&previous)).unwrap();
        assert_eq!(tokens.refresh_token, "prev-refresh");
        assert_eq!(tokens.email.as_deref(), Some("kept@example.com"));
    }
}
