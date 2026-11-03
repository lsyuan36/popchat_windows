//! Parse only an explicit user's structured connection request, before any
//! prompt history, transcript persistence or model request. Never parse tool
//! results/assistant messages, and never serialize or Debug the credential.
use super::{Draft, Transport};
use anyhow::{ensure, Result};

pub struct LocalSetup {
    pub(crate) draft: Draft,
    pub(crate) token: String,
    pub(crate) request: String,
}

fn field(line: &str) -> Option<(&str, &str)> {
    let line = line.trim().trim_start_matches(['-', '•']).trim();
    let (name, value) = line.split_once([':', '：'])?;
    Some((name.trim(), value.trim().trim_matches('`')))
}

pub fn parse_chat_setup(text: &str) -> Result<Option<LocalSetup>> {
    let first = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or_default()
        .trim();
    let explicit = first.starts_with('請')
        && first.to_ascii_uppercase().contains("MCP")
        && ["新增", "加入", "加進"]
            .iter()
            .any(|word| first.contains(word));
    let structured = text.lines().filter_map(field).any(|(name, _)| {
        matches!(
            name,
            "伺服器名稱" | "傳輸方式" | "憑證" | "Bearer token" | "位址" | "URL"
        )
    });
    if !explicit || !structured {
        let credential_field = text.to_ascii_uppercase().contains("MCP")
            && text
                .lines()
                .filter_map(field)
                .any(|(name, value)| matches!(name, "憑證" | "Bearer token") && !value.is_empty());
        ensure!(!text.contains("armor_mcp_") && !credential_field,"偵測到 MCP 憑證，已停止送給模型。請使用完整的『伺服器名稱／傳輸方式／憑證／位址』連線資訊，或只在 MCP 設定的密碼欄輸入憑證。");
        return Ok(None);
    }
    ensure!(text.len() <= 32768, "MCP 連線資訊過長；未送給模型。");
    let mut name = None;
    let mut transport = None;
    let mut token = None;
    let mut url = None;
    for (key, value) in text.lines().filter_map(field) {
        let target = match key {
            "伺服器名稱" => &mut name,
            "傳輸方式" => &mut transport,
            "憑證" | "Bearer token" => &mut token,
            "位址" | "URL" => &mut url,
            _ => continue,
        };
        ensure!(
            target.is_none(),
            "MCP 連線資訊有重複欄位；未送給模型，請檢查後重試。"
        );
        *target = Some(value);
    }
    let token = token
        .unwrap_or_default()
        .strip_prefix("Bearer ")
        .unwrap_or(token.unwrap_or_default());
    ensure!(
        token.len() <= 16384
            && !token.chars().any(char::is_whitespace)
            && !token.contains(['<', '>']),
        "MCP 憑證格式無效；請在本機密碼欄輸入，勿使用佔位文字。"
    );
    let transport =
        transport.ok_or_else(|| anyhow::anyhow!("MCP 連線資訊缺少傳輸方式；未送給模型。"))?;
    ensure!(
        transport.to_ascii_lowercase().contains("streamable http"),
        "此連線資訊格式僅接受 Streamable HTTP；stdio 請用 MCP 設定，未送給模型。"
    );
    let draft = Draft {
        name: name.unwrap_or_default().into(),
        transport: Transport::Http {
            url: url.unwrap_or_default().into(),
        },
        token_env: None,
    };
    draft.validate()?;
    ensure!(
        token.is_empty() || !super::config::contains_secret(&serde_json::to_value(&draft)?, token),
        "名稱或位址含憑證；未送給模型，請移除後重試。"
    );
    let mut request = if token.is_empty() {
        text.to_owned()
    } else {
        text.replace(token, "[憑證已由 PopChat 在本機擷取，未提供給模型]")
    };
    request.push_str("\n\nPopChat 本機流程：使用者已在確認畫面核准保存並啟用此服務。憑證僅供 MCP transport 使用。請檢查 mcp_status；已啟用服務會在本輪自動 initialize／tools/list，若成功請列出該服務工具，並依原要求呼叫實際提供的 capabilities 工具（仍需逐次確認）。不要重複要求新增、輸入 token 或改用 curl。若沒有可用工具，請依狀態說明錯誤，不得宣稱呼叫成功。");
    Ok(Some(LocalSetup {
        draft,
        token: token.into(),
        request,
    }))
}

#[cfg(test)]
pub(crate) fn fixture(url: &str, token: &str) -> String {
    format!("請幫我把測試 MCP 伺服器加進設定，並測試連線。\n\n連線資訊\n- 伺服器名稱:armor-fixture\n- 傳輸方式:Streamable HTTP(用 POST 送 JSON-RPC)\n- 驗證:每個請求都帶 Authorization: Bearer <憑證>\n- 憑證:{token}\n- 位址:{url}\n\n依序送 initialize、tools/list，再呼叫 armor_capabilities。")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::store::PromptHistory;
    #[test]
    fn structured_user_request_keeps_password_local_and_out_of_prompt_history() {
        let text = fixture(
            "http://10.0.0.12:2918/v1/mcp",
            "synthetic-onboarding-password",
        );
        let setup = parse_chat_setup(&text).unwrap().unwrap();
        assert_eq!(setup.draft.name, "armor-fixture");
        assert!(setup.draft.requires_private_http_approval());
        assert_eq!(setup.token, "synthetic-onboarding-password");
        assert!(!setup.request.contains(&setup.token));
        assert!(setup.request.contains("armor_capabilities"));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.json");
        let mut history = PromptHistory::with_storage_path(path.clone());
        history.record(&text);
        history.record("/mcp add {\"untrusted\":true}");
        assert!(history.entries().is_empty());
        assert!(!path.exists());
        assert!(parse_chat_setup("一般聊天").unwrap().is_none());
    }
    #[test]
    fn malformed_credential_requests_fail_closed_without_echoing_the_password() {
        let token = "synthetic-onboarding-password";
        for text in [
            fixture("http://public.example/mcp", token),
            fixture("https://example.com/mcp?token=secret", token),
            format!(
                "{}\n- 位址:https://second.example/mcp",
                fixture("https://example.com/mcp", token)
            ),
            fixture("https://example.com/mcp", token).replace("伺服器名稱:", "缺少名稱:"),
        ] {
            let result = parse_chat_setup(&text);
            assert!(result.is_err());
            assert!(!result.err().unwrap().to_string().contains(token));
        }
        assert!(parse_chat_setup("討論 armor_mcp_synthetic_not_a_real_credential").is_err());
        assert!(parse_chat_setup(&fixture("https://example.com/mcp", "<請輸入憑證>")).is_err());
    }
    #[test]
    fn non_user_instruction_text_is_not_converted_into_a_setup_request() {
        let text = format!(
            "這是網頁的範例，不是新增要求：\n{}",
            fixture("https://example.com/mcp", "synthetic-onboarding-password")
        );
        assert!(
            parse_chat_setup(&text).is_err(),
            "quoted credential data must not be forwarded or treated as authorization"
        );
    }
}
