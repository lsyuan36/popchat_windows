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
    #[serde(other)]
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
    CancelledBeforeSend,
    OutcomeUnknown,
    Skipped,
    #[serde(other)]
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
            // Older transcripts didn't distinguish local Stop from remote cancellation.
            Self::Cancelled => "已停止等待",
            Self::CancelledBeforeSend => "已取消，未送出",
            Self::OutcomeUnknown => "已停止等待，遠端結果未知",
            Self::Skipped => "已略過",
            Self::Interrupted => "未完成",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Running => "◌",
            Self::WaitingApproval => "⏳",
            Self::Succeeded => "✓",
            Self::Failed | Self::OutcomeUnknown => "⚠",
            Self::Denied
            | Self::Cancelled
            | Self::CancelledBeforeSend
            | Self::Skipped
            | Self::Interrupted => "−",
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
    #[serde(flatten, with = "stored_state")]
    pub state: ActivityState,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Group actual HTTP failures by status, independent of a pause notice
    /// appended to the second response. Never stores the response body.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
}

/// v0.2.10 rejects unknown `state` values and would hide the *entire chat*.
/// Keep a v0.2.10-readable state, with an optional, more precise outcome.
mod stored_state {
    use super::ActivityState;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[derive(Serialize, Deserialize)]
    struct Fields {
        state: ActivityState,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        outcome: Option<ActivityState>,
    }

    pub fn serialize<S: Serializer>(
        value: &ActivityState,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        use ActivityState::*;
        let base = match *value {
            CancelledBeforeSend => Cancelled,
            OutcomeUnknown | Skipped => Interrupted,
            state => state,
        };
        Fields {
            state: base,
            outcome: (base != *value).then_some(*value),
        }
        .serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<ActivityState, D::Error> {
        use ActivityState::*;
        let fields = Fields::deserialize(deserializer)?;
        Ok(match (fields.state, fields.outcome) {
            (Cancelled, Some(CancelledBeforeSend)) => CancelledBeforeSend,
            (Interrupted, Some(s @ (OutcomeUnknown | Skipped))) => s,
            (state, _) => state,
        })
    }
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
            http_status: None,
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

    pub fn fail(&mut self, error: &str) {
        if self.state.is_pending() {
            self.state = ActivityState::Failed;
            self.detail = Some(error_summary(self.kind, error));
        }
    }

    pub(crate) fn fail_http(&mut self, status: reqwest::StatusCode, paused: bool) {
        if self.state.is_pending() {
            self.state = ActivityState::Failed;
            self.http_status = Some(status.as_u16());
            let mut detail = status_summary(self.kind, self.http_status);
            if paused {
                detail.push_str("；本輪後續搜尋已暫停");
            }
            self.detail = Some(detail);
        }
    }

    pub fn succeed(&mut self) {
        if self.state.is_pending() {
            self.state = ActivityState::Succeeded;
        }
    }

    /// A restored record is not evidence that the server cancelled anything.
    pub fn inactive_state(&self) -> ActivityState {
        match (self.kind, self.state) {
            (ActivityKind::McpTool, ActivityState::Running) => ActivityState::OutcomeUnknown,
            (_, s) if s.is_pending() => ActivityState::Interrupted,
            (_, s) => s,
        }
    }

    pub(super) fn stopped_state(&self) -> ActivityState {
        match self.state {
            ActivityState::WaitingApproval => ActivityState::CancelledBeforeSend,
            ActivityState::Running if self.kind == ActivityKind::McpTool => {
                ActivityState::OutcomeUnknown
            }
            ActivityState::Running => ActivityState::Cancelled,
            state => state,
        }
    }
}

/// A display boundary, not a lossless error log. Never render arbitrary remote
/// bodies, model arguments or MCP result text as diagnostics.
pub fn error_summary(kind: ActivityKind, error: &str) -> String {
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
    status_summary(kind, status)
}

fn status_summary(kind: ActivityKind, status: Option<u16>) -> String {
    let prefix = match kind {
        ActivityKind::Search => "搜尋未取得可用結果",
        ActivityKind::Fetch => "網頁讀取未完成",
        ActivityKind::McpDiscovery => "MCP 連線／工具探索未完成",
        _ => "工具未完成",
    };
    match status {
        Some(429) => format!("{prefix} · HTTP 429（請稍後再試）"),
        Some(status) => format!("{prefix} · HTTP {status}"),
        None => prefix.to_owned(),
    }
}

pub fn single_line(text: &str, limit: usize) -> String {
    let mut words = text.split_whitespace();
    let mut chars = words
        .next()
        .unwrap_or("")
        .chars()
        .chain(words.flat_map(|word| std::iter::once(' ').chain(word.chars())));
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
        activity.fail("HTTP 202: <!DOCTYPE html>\n<head>private-token</head>");
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

    #[test]
    fn precise_outcomes_remain_readable_to_old_history_readers() {
        use ActivityState::*;
        let mut a = ToolActivity::new(ActivityKind::McpTool, "fixture");
        for (state, legacy) in [
            (Skipped, "interrupted"),
            (CancelledBeforeSend, "cancelled"),
            (OutcomeUnknown, "interrupted"),
            (Succeeded, "succeeded"),
        ] {
            a.state = state;
            let json = serde_json::to_value(&a).unwrap();
            assert_eq!(json["state"].as_str(), Some(legacy));
            assert_eq!(json.get("outcome").is_some(), state != Succeeded);
            assert_eq!(serde_json::from_value::<ToolActivity>(json).unwrap(), a);
        }
        let mut old = serde_json::to_value(&a).unwrap();
        old["state"] = serde_json::json!("cancelled"); // v0.2.10 didn't know whether RPC ran
        assert_eq!(
            serde_json::from_value::<ToolActivity>(old.clone())
                .unwrap()
                .state,
            Cancelled
        );
        old["state"] = serde_json::json!("running");
        let restored = serde_json::from_value::<ToolActivity>(old.clone()).unwrap();
        assert_eq!(restored.inactive_state(), OutcomeUnknown);
        old["kind"] = serde_json::json!("futureTool");
        old["state"] = serde_json::json!("futureState");
        assert_eq!(
            serde_json::from_value::<ToolActivity>(old).unwrap().state,
            Interrupted
        );
    }
}
