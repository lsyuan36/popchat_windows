use crate::chat::store::SecretStore;
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(super) fn contains_secret(value: &serde_json::Value, token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    match value {
        serde_json::Value::String(s) => s.contains(token),
        serde_json::Value::Array(values) => values.iter().any(|v| contains_secret(v, token)),
        serde_json::Value::Object(values) => values
            .iter()
            .any(|(k, v)| k.contains(token) || contains_secret(v, token)),
        _ => false,
    }
}

pub(super) fn redact_secret(value: &mut serde_json::Value, token: &str) {
    if token.is_empty() {
        return;
    }
    match value {
        serde_json::Value::String(s) => *s = s.replace(token, "[REDACTED MCP TOKEN]"),
        serde_json::Value::Array(values) => {
            for v in values {
                redact_secret(v, token);
            }
        }
        serde_json::Value::Object(values) => {
            let old = std::mem::take(values);
            for (key, mut value) in old {
                redact_secret(&mut value, token);
                values.insert(key.replace(token, "[REDACTED MCP TOKEN]"), value);
            }
        }
        _ => {}
    }
}
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Transport {
    Http {
        url: String,
    },
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
    },
}

/// Model/chat input deliberately cannot choose IDs, enabled state, secrets,
/// headers or arbitrary environment variables. Only a human can enable it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Draft {
    pub name: String,
    pub transport: Transport,
    #[serde(default)]
    pub token_env: Option<String>,
}

impl Draft {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.name.trim().is_empty()
                && self.name.len() <= 100
                && !self.name.chars().any(char::is_control),
            "MCP 名稱須為 1–100 bytes，且不可包含控制字元。"
        );
        if let Some(env) = &self.token_env {
            ensure!(
                !env.is_empty()
                    && env.len() <= 100
                    && env.bytes().enumerate().all(|(i, c)| c == b'_'
                        || c.is_ascii_alphabetic()
                        || (i > 0 && c.is_ascii_digit())),
                "憑證環境變數名稱無效。"
            );
            ensure!(
                !matches!(
                    env.to_ascii_uppercase().as_str(),
                    "PATH"
                        | "PATHEXT"
                        | "SYSTEMROOT"
                        | "COMSPEC"
                        | "NODE_OPTIONS"
                        | "PYTHONPATH"
                        | "PYTHONHOME"
                        | "LD_PRELOAD"
                        | "LD_LIBRARY_PATH"
                ),
                "不可用憑證欄位覆寫程序啟動環境。"
            );
        }
        match &self.transport {
            Transport::Http { url } => {
                ensure!(url.len() <= 2048, "MCP URL 過長。");
                let url =
                    reqwest::Url::parse(url).map_err(|_| anyhow::anyhow!("MCP URL 無效。"))?;
                ensure!(url.username().is_empty() && url.password().is_none() && url.query().is_none() && url.fragment().is_none(),"不要把金鑰放進 URL；請使用獨立的 Bearer token 欄位。URL 不接受帳密、query 或 fragment。");
                let local = matches!(
                    url.host_str(),
                    Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
                );
                ensure!(
                    url.scheme() == "https" || (url.scheme() == "http" && local),
                    "遠端 MCP 必須使用 HTTPS；只有 localhost 可用 HTTP。"
                );
                ensure!(url.host_str().is_some(), "MCP URL 缺少主機。");
                ensure!(
                    self.token_env.is_none(),
                    "HTTP 憑證使用 Bearer token，不使用環境變數。"
                );
            }
            Transport::Stdio { command, args } => {
                ensure!(
                    !command.trim().is_empty()
                        && command.len() <= 4096
                        && !command.chars().any(|c| c.is_control() || c == '"'),
                    "stdio command 必須是可直接啟動的程式，不含外層引號或控制字元。"
                );
                ensure!(
                    args.len() <= 64 && args.iter().all(|a| a.len() <= 8192 && !a.contains('\0')),
                    "stdio args 過多、過長或包含 NUL。"
                );
                ensure!(
                    !args.iter().any(|arg| {
                        let arg = arg.to_ascii_lowercase();
                        let flag = arg.split('=').next().unwrap_or_default();
                        matches!(
                            flag,
                            "--token"
                                | "--api-key"
                                | "--apikey"
                                | "--api_key"
                                | "--password"
                                | "--secret"
                                | "--authorization"
                        ) || [
                            "bearer ",
                            "api_key=",
                            "apikey=",
                            "password=",
                            "secret=",
                            "token=",
                        ]
                        .iter()
                        .any(|word| arg.contains(word))
                            || arg.starts_with("sk-")
                            || arg.starts_with("ghp_")
                            || arg.starts_with("github_pat_")
                    }),
                    "不要把憑證放在 args；請使用獨立 token 與 token_env 欄位。"
                );
                ensure!(
                    !command.to_ascii_lowercase().ends_with(".cmd")
                        && !command.to_ascii_lowercase().ends_with(".bat"),
                    "不經 shell 執行 .cmd／.bat；請使用 node.exe／python.exe／uvx.exe 等實際程式。"
                );
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Service {
    pub id: Uuid,
    pub draft: Draft,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub secret_ref: Option<Uuid>,
}

impl Service {
    pub fn token(&self, secrets: &SecretStore) -> Result<String> {
        match self.secret_ref {
            Some(id) => read_secret(secrets, id),
            None => Ok(String::new()),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SecretManifest {
    version: u8,
    chunks: usize,
    digest: String,
}

fn read_secret(secrets: &SecretStore, id: Uuid) -> Result<String> {
    let header = secrets
        .get(&format!("mcp-token-{id}"))
        .context("MCP 憑證無法讀取，請重新輸入。")?;
    // Compatibility with initial plain credential entries, never JSON config.
    if !header.starts_with('{') {
        return Ok(header);
    }
    let manifest: SecretManifest =
        serde_json::from_str(&header).context("MCP 憑證 manifest 損壞")?;
    ensure!(
        manifest.version == 1 && (1..=32).contains(&manifest.chunks),
        "MCP 憑證 manifest 無效。"
    );
    let mut token = String::new();
    for index in 0..manifest.chunks {
        token.push_str(
            &secrets
                .get(&format!("mcp-token-{id}-{index}"))
                .context("MCP 憑證片段遺失")?,
        );
    }
    ensure!(
        token.len() <= 16384
            && format!("{:x}", Sha256::digest(token.as_bytes())) == manifest.digest,
        "MCP 憑證完整性驗證失敗。"
    );
    Ok(token)
}

fn delete_secret(secrets: &mut SecretStore, id: Uuid) {
    let count = secrets
        .get(&format!("mcp-token-{id}"))
        .and_then(|s| serde_json::from_str::<SecretManifest>(&s).ok())
        .map_or(0, |m| m.chunks.min(32));
    for index in 0..count {
        let _ = secrets.delete(&format!("mcp-token-{id}-{index}"));
    }
    let _ = secrets.delete(&format!("mcp-token-{id}"));
}

fn write_secret(secrets: &mut SecretStore, id: Uuid, token: &str) -> Result<()> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut units = 0;
    for (offset, ch) in token.char_indices() {
        if units + ch.len_utf16() > 1000 {
            parts.push(&token[start..offset]);
            start = offset;
            units = 0;
        }
        units += ch.len_utf16();
    }
    parts.push(&token[start..]);
    ensure!(parts.len() <= 32, "MCP token 過長。");
    let result = (|| -> Result<()> {
        for (index, part) in parts.iter().enumerate() {
            secrets.set(part, &format!("mcp-token-{id}-{index}"))?;
        }
        let manifest = SecretManifest {
            version: 1,
            chunks: parts.len(),
            digest: format!("{:x}", Sha256::digest(token.as_bytes())),
        };
        secrets.set(
            &serde_json::to_string(&manifest)?,
            &format!("mcp-token-{id}"),
        )?;
        ensure!(
            read_secret(secrets, id)? == token,
            "MCP token 寫入驗證失敗。"
        );
        Ok(())
    })();
    if result.is_err() {
        for index in 0..parts.len() {
            let _ = secrets.delete(&format!("mcp-token-{id}-{index}"));
        }
        let _ = secrets.delete(&format!("mcp-token-{id}"));
    }
    result
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    #[serde(default)]
    allow_chat_proposals: bool,
    #[serde(default)]
    servers: Vec<Service>,
}

pub struct McpStore {
    path: PathBuf,
    pub servers: Vec<Service>,
    pub allow_chat_proposals: bool,
    pub load_error: Option<String>,
}

impl McpStore {
    pub fn load() -> Self {
        Self::load_at(crate::chat::store::app_data_dir().join("mcp_servers.json"))
    }
    pub fn load_at(path: PathBuf) -> Self {
        let result = (|| -> Result<Document> {
            let data = match std::fs::read(&path) {
                Ok(d) => d,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(Document::default())
                }
                Err(e) => return Err(e.into()),
            };
            ensure!(data.len() <= 1024 * 1024, "MCP 設定過大。");
            let doc: Document = serde_json::from_slice(&data)?;
            ensure!(doc.servers.len() <= 16, "MCP 服務數量超過上限。");
            let mut ids = std::collections::HashSet::new();
            let mut names = std::collections::HashSet::new();
            for service in &doc.servers {
                service.draft.validate()?;
                ensure!(
                    ids.insert(service.id) && names.insert(service.draft.name.to_lowercase()),
                    "MCP 服務身分重複。"
                );
            }
            Ok(doc)
        })();
        match result {
            Ok(doc) => Self {
                path,
                servers: doc.servers,
                allow_chat_proposals: doc.allow_chat_proposals,
                load_error: None,
            },
            Err(_) => Self {
                path,
                servers: vec![],
                allow_chat_proposals: false,
                load_error: Some("MCP 設定讀取失敗；已停止 MCP 並禁止覆寫原檔。".into()),
            },
        }
    }
    fn commit(&self, servers: &[Service], allow: bool) -> Result<()> {
        ensure!(self.load_error.is_none(), "MCP 設定無法安全寫入。");
        let doc = Document {
            servers: servers.to_vec(),
            allow_chat_proposals: allow,
        };
        crate::paths::atomic_write(&self.path, &serde_json::to_vec_pretty(&doc)?)?;
        Ok(())
    }
    pub fn set_chat_proposals(&mut self, allow: bool) -> Result<()> {
        self.commit(&self.servers, allow)?;
        self.allow_chat_proposals = allow;
        Ok(())
    }
    pub fn save(
        &mut self,
        id: Option<Uuid>,
        draft: Draft,
        enabled: bool,
        new_token: Option<&str>,
        clear_token: bool,
        secrets: &mut SecretStore,
    ) -> Result<Uuid> {
        ensure!(self.load_error.is_none(), "MCP 設定無法安全寫入。");
        draft.validate()?;
        let old = id
            .and_then(|id| self.servers.iter().find(|s| s.id == id))
            .cloned();
        ensure!(id.is_none() || old.is_some(), "原 MCP 服務已不存在。");
        ensure!(
            !self
                .servers
                .iter()
                .any(|s| Some(s.id) != id
                    && s.draft.name.to_lowercase() == draft.name.to_lowercase()),
            "已有同名 MCP 服務；不會從對話覆寫既有服務。"
        );
        ensure!(
            old.is_some() || self.servers.len() < 16,
            "最多 16 個 MCP 服務。"
        );
        let inspect_token = new_token
            .filter(|t| !t.is_empty())
            .map(str::to_owned)
            .or_else(|| {
                if clear_token {
                    None
                } else {
                    old.as_ref().and_then(|s| s.token(secrets).ok())
                }
            });
        if let Some(token) = inspect_token {
            ensure!(
                !contains_secret(&serde_json::to_value(&draft)?, &token),
                "MCP 設定含有認證 token；請移除名稱、URL 或 args 中的憑證，只放在密碼欄。"
            );
        }
        let mut service = Service {
            id: id.unwrap_or_else(Uuid::new_v4),
            draft,
            enabled,
            secret_ref: old.as_ref().and_then(|s| s.secret_ref),
        };
        if clear_token {
            service.secret_ref = None;
        }
        // A new generation means a failed config commit cannot replace the
        // previous active credential. Secrets never enter the config JSON.
        let generation = if let Some(token) = new_token.filter(|t| !t.is_empty()) {
            ensure!(
                token.len() <= 16384 && !token.chars().any(|c| c == '\r' || c == '\n'),
                "MCP token 格式無效。"
            );
            let id = Uuid::new_v4();
            write_secret(secrets, id, token)?;
            service.secret_ref = Some(id);
            Some(id)
        } else {
            None
        };
        let mut next = self.servers.clone();
        if let Some(index) = next.iter().position(|s| s.id == service.id) {
            next[index] = service.clone();
        } else {
            next.push(service.clone());
        }
        if let Err(error) = self.commit(&next, self.allow_chat_proposals) {
            if let Some(id) = generation {
                delete_secret(secrets, id);
            }
            return Err(error);
        }
        self.servers = next;
        if let Some(id) = old
            .and_then(|s| s.secret_ref)
            .filter(|id| Some(*id) != service.secret_ref)
            .filter(|id| !self.servers.iter().any(|s| s.secret_ref == Some(*id)))
        {
            delete_secret(secrets, id);
        }
        Ok(service.id)
    }
    pub fn remove(&mut self, id: Uuid, secrets: &mut SecretStore) -> Result<()> {
        let old = self
            .servers
            .iter()
            .find(|s| s.id == id)
            .context("MCP 服務已不存在。")?
            .clone();
        let next = self
            .servers
            .iter()
            .filter(|s| s.id != id)
            .cloned()
            .collect::<Vec<_>>();
        self.commit(&next, self.allow_chat_proposals)?;
        self.servers = next;
        if let Some(id) = old
            .secret_ref
            .filter(|id| !self.servers.iter().any(|s| s.secret_ref == Some(*id)))
        {
            delete_secret(secrets, id);
        }
        Ok(())
    }
}

/// Explicit user command, intercepted before prompt history or API send. Never
/// inspect assistant messages or scraped/tool text for executable additions.
pub fn parse_chat_add(text: &str) -> Result<Option<Draft>> {
    let text = text.trim();
    let Some(rest) = text.strip_prefix("/mcp") else {
        return Ok(None);
    };
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return Ok(None);
    }
    let Some(json) = rest
        .trim()
        .strip_prefix("add")
        .filter(|s| s.starts_with(char::is_whitespace))
    else {
        anyhow::bail!("{}", "用法：/mcp add {\"name\":\"服務名稱\",\"transport\":{\"type\":\"http\",\"url\":\"https://example.com/mcp\"}}。請勿在聊天輸入金鑰。");
    };
    ensure!(json.len() <= 16384, "MCP 草稿過大。");
    let draft: Draft = serde_json::from_str(json.trim()).map_err(|_| {
        anyhow::anyhow!("MCP 草稿 JSON 無效或含不允許的欄位；金鑰、headers、env 請勿放入對話。")
    })?;
    draft.validate()?;
    Ok(Some(draft))
}
