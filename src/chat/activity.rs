//! UI-only tool lifecycle metadata. Never included in model history.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActivityKind {
    Search,
    Fetch,
    McpDiscovery,
    McpTool,
    Tool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActivityState {
    Running,
    WaitingApproval,
    Succeeded,
    Failed,
    Denied,
    Cancelled,
    Interrupted,
}

impl ActivityState {
    pub fn is_pending(self) -> bool {
        matches!(self, Self::Running | Self::WaitingApproval)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "執行中",
            Self::WaitingApproval => "等待你確認",
            Self::Succeeded => "已完成",
            Self::Failed => "失敗",
            Self::Denied => "未獲准／確認逾時，未執行",
            Self::Cancelled => "已取消",
            Self::Interrupted => "未完成",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Running => "◌",
            Self::WaitingApproval => "⏳",
            Self::Succeeded => "✓",
            Self::Failed => "⚠",
            Self::Denied | Self::Cancelled | Self::Interrupted => "−",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolActivity {
    pub id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<Uuid>,
    pub kind: ActivityKind,
    pub state: ActivityState,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl ToolActivity {
    pub fn new(kind: ActivityKind, label: impl AsRef<str>) -> Self {
        Self {
            id: Uuid::new_v4(),
            owner: None,
            kind,
            state: ActivityState::Running,
            label: single_line(label.as_ref(), 180),
            detail: None,
        }
    }

    pub fn web(name: &str, arguments: &str) -> Self {
        let args = serde_json::from_str::<serde_json::Value>(arguments).unwrap_or_default();
        match name {
            "web_search" => Self::new(
                ActivityKind::Search,
                format!("搜尋：{}", args["query"].as_str().unwrap_or("?")),
            ),
            "fetch_url" => {
                // Do not expose credentials, query tokens or fragments in a label.
                let host = args["url"]
                    .as_str()
                    .and_then(|url| reqwest::Url::parse(url).ok()?.host_str().map(str::to_owned));
                Self::new(
                    ActivityKind::Fetch,
                    format!("讀取網頁：{}", host.as_deref().unwrap_or("?")),
                )
            }
            _ => Self::new(ActivityKind::Tool, format!("工具：{name}")),
        }
    }

    pub fn finish(&mut self, result: &str) {
        if !self.state.is_pending() {
            return;
        }
        if let Some(error) = result.strip_prefix("ERROR:") {
            self.state = ActivityState::Failed;
            self.detail = Some(error_summary(self.kind, error));
        } else {
            self.state = ActivityState::Succeeded;
        }
    }
}

/// A display boundary, not a lossless error log. Never render arbitrary remote
/// bodies, model arguments or MCP result text as diagnostics.
pub fn error_summary(kind: ActivityKind, error: &str) -> String {
    let prefix = match kind {
        ActivityKind::Search => "搜尋未取得可用結果",
        ActivityKind::Fetch => "網頁讀取未完成",
        ActivityKind::McpDiscovery => "MCP 連線／工具探索未完成",
        _ => "工具未完成",
    };
    let error = error.trim();
    if kind == ActivityKind::Search && error.starts_with("SEARCH_PAUSED:") {
        return "搜尋本輪已暫停：連續收到 HTTP 202／429，未再送出請求".into();
    }
    // Only trust a leading status produced by our HTTP adapter, not an HTTP
    // looking string embedded in an arbitrary server response.
    let status = error
        .strip_prefix("HTTP ")
        .and_then(|s| s.split(|c: char| !c.is_ascii_digit()).next())
        .filter(|s| s.len() == 3)
        .and_then(|s| s.parse::<u16>().ok())
        .filter(|s| (100..600).contains(s));
    match status {
        Some(429) => format!("{prefix} · HTTP 429（請稍後再試）"),
        Some(status) => format!("{prefix} · HTTP {status}"),
        None => prefix.to_owned(),
    }
}

pub fn single_line(text: &str, limit: usize) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = normalized.chars();
    let mut short: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() {
        short.push('…');
    }
    short
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_summaries_never_expose_html_results_or_credentials() {
        let mut activity = ToolActivity::web("web_search", r#"{"query":"中文\n搜尋"}"#);
        let id = activity.id;
        activity.finish("ERROR: HTTP 202: <!DOCTYPE html>\n<head>private-token</head>");
        assert_eq!(activity.id, id);
        assert_eq!(activity.label, "搜尋：中文 搜尋");
        assert_eq!(
            activity.detail.as_deref(),
            Some("搜尋未取得可用結果 · HTTP 202")
        );
        assert_eq!(activity.state, ActivityState::Failed);
        assert_eq!(
            error_summary(ActivityKind::McpTool, "private-token HTTP 403"),
            "工具未完成"
        );
        let page = ToolActivity::web(
            "fetch_url",
            r#"{"url":"https://user:secret@example.com/a?token=private#secret"}"#,
        );
        assert_eq!(page.label, "讀取網頁：example.com");
        assert_eq!(single_line(&"中".repeat(300), 20).chars().count(), 21);
    }
}
