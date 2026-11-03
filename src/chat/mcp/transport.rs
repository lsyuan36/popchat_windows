use super::{Service, Transport};
use anyhow::{ensure, Context, Result};
use futures::StreamExt;
use serde_json::{json, Value};
use std::{process::Stdio, time::Duration};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const LIMIT: usize = 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(60);
const VERSION: &str = "2025-11-25";

struct StdioConnection {
    input: tokio::process::ChildStdin,
    output: BufReader<tokio::process::ChildStdout>,
    child: tokio::process::Child,
    #[cfg(windows)]
    _job: crate::chat::codex::ChildJob,
}
impl Drop for StdioConnection {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

struct HttpConnection {
    client: reqwest::Client,
    url: String,
    token: String,
    session: Option<String>,
    version: String,
    pending: Option<Value>,
}
impl HttpConnection {
    fn post(&self, body: &Value) -> reqwest::RequestBuilder {
        let mut req = self
            .client
            .post(&self.url)
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", &self.version)
            .json(body);
        if !self.token.is_empty() {
            req = req.bearer_auth(&self.token);
        }
        if let Some(id) = &self.session {
            req = req.header("Mcp-Session-Id", id);
        }
        req
    }
    async fn reply(&self, body: &Value) -> Result<()> {
        let response = self
            .post(body)
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("MCP HTTP 通知傳送失敗。"))?;
        ensure!(
            matches!(response.status().as_u16(), 202 | 204),
            "MCP 服務未接受通知／client 回覆。"
        );
        Ok(())
    }
    async fn request(&mut self, body: &Value) -> Result<Value> {
        self.pending = Some(body["id"].clone());
        let response = self
            .post(body)
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("MCP HTTP 連線失敗；未自動重送。"))?;
        ensure!(
            response.status().is_success(),
            "MCP HTTP {}；未自動重送工具。401 需手動設定 token，404 請於下輪重新連線。",
            response.status().as_u16()
        );
        if body["method"] == "initialize" {
            if let Some(id) = response.headers().get("Mcp-Session-Id") {
                let id = id
                    .to_str()
                    .map_err(|_| anyhow::anyhow!("MCP session header 無效。"))?;
                ensure!(
                    !id.is_empty()
                        && id.len() <= 1024
                        && id.bytes().all(|b| (0x21..=0x7e).contains(&b)),
                    "MCP session header 無效。"
                );
                self.session = Some(id.to_string());
            }
        }
        ensure!(
            response.content_length().is_none_or(|n| n <= LIMIT as u64),
            "MCP 回覆超過大小上限。"
        );
        let kind = response
            .headers()
            .get("content-type")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();
        let sse = kind.starts_with("text/event-stream");
        ensure!(
            sse || kind.starts_with("application/json"),
            "MCP 回覆不是 JSON 或 SSE。"
        );
        let mut stream = response.bytes_stream();
        let mut bytes = Vec::new();
        let mut event = String::new();
        let mut total = 0;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| anyhow::anyhow!("MCP HTTP 串流中斷；未重送工具。"))?;
            total += chunk.len();
            ensure!(total <= LIMIT, "MCP 回覆超過大小上限。");
            bytes.extend_from_slice(&chunk);
            if sse {
                while let Some(end) = bytes.iter().position(|b| *b == b'\n') {
                    let line: Vec<_> = bytes.drain(..=end).collect();
                    let line = std::str::from_utf8(&line)
                        .context("MCP SSE 不是 UTF-8")?
                        .trim_end_matches(['\r', '\n']);
                    if line.is_empty() {
                        if !event.is_empty() {
                            let value: Value =
                                serde_json::from_str(event.trim()).context("MCP SSE JSON 無效")?;
                            event.clear();
                            if value.get("id") == body.get("id") && value.get("method").is_none() {
                                self.pending = None;
                                return rpc_result(value);
                            }
                            if let Some(reply) = client_reply(&value) {
                                self.reply(&reply).await?;
                            }
                        }
                    } else if let Some(data) = line.strip_prefix("data:") {
                        event.push_str(data.trim_start_matches(' '));
                        event.push('\n');
                    }
                }
            }
        }
        if !sse {
            let value: Value = serde_json::from_slice(&bytes).context("MCP JSON 無效")?;
            ensure!(value.get("id") == body.get("id"), "MCP response ID 不符。");
            self.pending = None;
            return rpc_result(value);
        }
        anyhow::bail!("MCP SSE 未收到完整 response；不會重新執行工具。")
    }
}
impl Drop for HttpConnection {
    fn drop(&mut self) {
        // Bounded protocol cleanup only: never replay a tool on disconnect.
        if self.pending.is_none() && self.session.is_none() {
            return;
        }
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let client = self.client.clone();
            let url = self.url.clone();
            let token = self.token.clone();
            let session = self.session.clone();
            let pending = self.pending.take();
            let version = self.version.clone();
            runtime.spawn(async move {let _=tokio::time::timeout(Duration::from_secs(2),async {
                if let Some(id)=pending {
                    let mut req=client.post(&url).header("Accept","application/json, text/event-stream").header("MCP-Protocol-Version",&version).json(&json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":id,"reason":"Client stopped waiting"}}));
                    if !token.is_empty(){req=req.bearer_auth(&token);}if let Some(id)=&session{req=req.header("Mcp-Session-Id",id);}let _=req.send().await;
                }
                if let Some(id)=session {let mut req=client.delete(url).header("Mcp-Session-Id",id).header("MCP-Protocol-Version",version);if !token.is_empty(){req=req.bearer_auth(token);}let _=req.send().await;}
            }).await;});
        }
    }
}

fn rpc_result(value: Value) -> Result<Value> {
    ensure!(value["jsonrpc"] == "2.0", "MCP JSON-RPC 版本無效。");
    ensure!(
        value.get("error").is_none(),
        "MCP 回報 JSON-RPC error（內容已隱藏，避免洩漏憑證）。"
    );
    value
        .get("result")
        .cloned()
        .context("MCP response 缺少 result")
}
fn client_reply(value: &Value) -> Option<Value> {
    let id = value.get("id")?;
    let method = value.get("method")?.as_str()?;
    Some(if method == "ping" {
        json!({"jsonrpc":"2.0","id":id,"result":{}})
    } else {
        json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Client capability not supported"}})
    })
}

enum Connection {
    Stdio(StdioConnection),
    Http(HttpConnection),
}
pub struct Session {
    connection: Connection,
    next_id: u64,
    has_tools: bool,
    failed: bool,
}

impl Session {
    pub async fn connect(service: &Service, token: String) -> Result<Self> {
        service.draft.validate()?;
        let connection = match &service.draft.transport {
            Transport::Http { url } => Connection::Http(HttpConnection {
                client: reqwest::Client::builder()
                    .redirect(reqwest::redirect::Policy::none())
                    .timeout(TIMEOUT)
                    .build()?,
                url: url.clone(),
                token,
                session: None,
                version: VERSION.into(),
                pending: None,
            }),
            Transport::Stdio { command, args } => {
                let (command, args) = resolve_command(command, args)?;
                let mut process = tokio::process::Command::new(command);
                process
                    .args(args)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .kill_on_drop(true)
                    .env_clear();
                for name in [
                    "PATH",
                    "PATHEXT",
                    "SYSTEMROOT",
                    "WINDIR",
                    "TEMP",
                    "TMP",
                    "USERPROFILE",
                    "HOME",
                    "APPDATA",
                    "LOCALAPPDATA",
                ] {
                    if let Some(value) = std::env::var_os(name) {
                        process.env(name, value);
                    }
                }
                if let Some(name) = &service.draft.token_env {
                    ensure!(!token.is_empty(), "stdio 憑證環境變數尚未設定 token。");
                    process.env(name, token);
                }
                #[cfg(windows)]
                process.creation_flags(0x0800_0000);
                let mut child = process.spawn().map_err(|_| {
                    anyhow::anyhow!(
                        "無法啟動 MCP 程式；請檢查 executable 與 args，不會自動安裝套件。"
                    )
                })?;
                #[cfg(windows)]
                let job =
                    crate::chat::codex::ChildJob::new(&child).context("無法監督 MCP 子程序樹")?;
                Connection::Stdio(StdioConnection {
                    input: child.stdin.take().context("MCP stdin 不存在")?,
                    output: BufReader::new(child.stdout.take().context("MCP stdout 不存在")?),
                    child,
                    #[cfg(windows)]
                    _job: job,
                })
            }
        };
        let mut session = Self {
            connection,
            next_id: 1,
            has_tools: false,
            failed: false,
        };
        let result=session.request("initialize",json!({"protocolVersion":VERSION,"capabilities":{},"clientInfo":{"name":"PopChat","version":env!("CARGO_PKG_VERSION")}})).await?;
        let version = result["protocolVersion"]
            .as_str()
            .context("MCP 未回報 protocolVersion")?;
        ensure!(
            matches!(version, "2025-11-25" | "2025-06-18" | "2025-03-26")
                || (version == "2024-11-05" && matches!(session.connection, Connection::Stdio(_))),
            "不支援服務回報的 MCP protocol version。"
        );
        if let Connection::Http(http) = &mut session.connection {
            http.version = version.into();
        }
        session.has_tools = result["capabilities"]["tools"].is_object();
        session
            .notify(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
            .await?;
        Ok(session)
    }
    async fn notify(&mut self, value: Value) -> Result<()> {
        tokio::time::timeout(TIMEOUT, async {
            match &mut self.connection {
                Connection::Http(http) => http.reply(&value).await,
                Connection::Stdio(io) => {
                    let mut data = serde_json::to_vec(&value)?;
                    data.push(b'\n');
                    io.input.write_all(&data).await?;
                    io.input.flush().await?;
                    Ok(())
                }
            }
        })
        .await
        .context("MCP 通知逾時")?
    }
    async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        ensure!(
            !self.failed,
            "MCP 連線已失敗；下一輪才會建立新連線，不自動重送工具。"
        );
        let id = self.next_id;
        self.next_id += 1;
        let body = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
        let result = tokio::time::timeout(TIMEOUT, async {
            match &mut self.connection {
                Connection::Http(http) => http.request(&body).await,
                Connection::Stdio(io) => {
                    let mut data = serde_json::to_vec(&body)?;
                    ensure!(data.len() <= LIMIT, "MCP request 過大。");
                    data.push(b'\n');
                    io.input.write_all(&data).await?;
                    io.input.flush().await?;
                    for _ in 0..256 {
                        let mut bytes = Vec::new();
                        loop {
                            let buffer = io.output.fill_buf().await?;
                            ensure!(!buffer.is_empty(), "MCP 程式已關閉 stdout。");
                            let count = buffer
                                .iter()
                                .position(|b| *b == b'\n')
                                .map(|i| i + 1)
                                .unwrap_or(buffer.len());
                            ensure!(bytes.len() + count <= LIMIT, "MCP stdout frame 過大。");
                            let done = buffer[count - 1] == b'\n';
                            bytes.extend_from_slice(&buffer[..count]);
                            io.output.consume(count);
                            if done {
                                break;
                            }
                        }
                        let value: Value =
                            serde_json::from_slice(&bytes).context("MCP stdout 不是有效 JSON")?;
                        if value["id"] == id && value.get("method").is_none() {
                            return rpc_result(value);
                        }
                        if let Some(reply) = client_reply(&value) {
                            let mut data = serde_json::to_vec(&reply)?;
                            data.push(b'\n');
                            io.input.write_all(&data).await?;
                            io.input.flush().await?;
                        }
                    }
                    anyhow::bail!("MCP 回覆了過多非預期訊息。")
                }
            }
        })
        .await;
        let result = match result {
            Ok(result) => result,
            Err(_) => {
                self.failed = true;
                return Err(anyhow::anyhow!("MCP 請求逾時；不自動重送工具。"));
            }
        };
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    pub async fn tools(&mut self) -> Result<Vec<Value>> {
        if !self.has_tools {
            return Ok(vec![]);
        }
        let mut tools = Vec::new();
        let mut cursor = None;
        let mut seen = std::collections::HashSet::new();
        for _ in 0..8 {
            let result = self
                .request(
                    "tools/list",
                    cursor
                        .as_ref()
                        .map(|c| json!({"cursor":c}))
                        .unwrap_or(json!({})),
                )
                .await?;
            for tool in result["tools"]
                .as_array()
                .context("MCP tools/list 缺少 tools")?
            {
                let name = tool["name"].as_str().context("MCP tool 缺少名稱")?;
                ensure!(
                    !name.is_empty() && name.len() <= 128 && !name.chars().any(char::is_control),
                    "MCP 工具名稱無效。"
                );
                ensure!(
                    tool["inputSchema"].is_object()
                        && tool["inputSchema"].to_string().len() <= 16384,
                    "MCP inputSchema 無效或過大。"
                );
                ensure!(
                    !tools.iter().any(|t: &Value| t["name"] == name),
                    "MCP 工具名稱重複。"
                );
                tools.push(tool.clone());
                ensure!(tools.len() <= 64, "單一 MCP 服務超過 64 個工具。");
            }
            match result.get("nextCursor").and_then(Value::as_str) {
                None => return Ok(tools),
                Some(c) => {
                    ensure!(
                        c.len() <= 2048 && seen.insert(c.to_string()),
                        "MCP 工具分頁 cursor 無效或重複。"
                    );
                    cursor = Some(c.to_string());
                }
            }
        }
        anyhow::bail!("MCP 工具分頁超過安全上限。")
    }
    pub async fn call(&mut self, name: &str, args: Value) -> Result<Value> {
        self.request("tools/call", json!({"name":name,"arguments":args}))
            .await
    }
}

fn resolve_command(command: &str, args: &[String]) -> Result<(String, Vec<String>)> {
    // Windows npm shims require cmd.exe. Resolve plain npx through the adjacent
    // node.exe + npm JS entry instead; never build a shell command string.
    #[cfg(windows)]
    if command.eq_ignore_ascii_case("npx") {
        for dir in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
            let node = dir.join("node.exe");
            let script = dir.join("node_modules/npm/bin/npx-cli.js");
            if node.is_file() && script.is_file() {
                let mut argv = vec![script.to_string_lossy().into_owned()];
                argv.extend_from_slice(args);
                return Ok((node.to_string_lossy().into_owned(), argv));
            }
        }
        anyhow::bail!("找不到可直接啟動的 npx；請明確設定 node.exe 與 npx-cli.js 路徑。");
    }
    Ok((command.into(), args.to_vec()))
}
