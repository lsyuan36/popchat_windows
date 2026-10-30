//! Subscription backend dispatch. Register one shared `Arc<DelegatedBackend>`
//! with `client::set_delegated_backend`; Settings updates that same instance.

use std::path::PathBuf;
use std::sync::RwLock;

use tokio::sync::mpsc;

use super::client::{self, ChatBackend, ChatStreamEvent, ProviderConfig, WebAccess, WireMessage};
use super::codex::CodexBackend;
use super::providers::ProviderKind;
use super::responses::ResponsesBackend;

pub struct DelegatedBackend {
    codex_executable: RwLock<Option<PathBuf>>,
    responses: ResponsesBackend,
}

impl DelegatedBackend {
    pub fn new() -> Self {
        Self {
            codex_executable: RwLock::new(None),
            responses: ResponsesBackend::new(),
        }
    }

    /// Applies to subsequent turns only. Already-running children keep their
    /// executable, history, and permissions for their own lifetime.
    pub fn set_codex_executable(&self, executable: Option<PathBuf>) {
        *self
            .codex_executable
            .write()
            .unwrap_or_else(|e| e.into_inner()) = executable;
    }

    /// Swift's Responses client uses the conversation UUID as session_id,
    /// not a different randomly generated identity for each HTTP round.
    pub fn set_session_id(&self, session_id: String) {
        self.responses.set_session_id(session_id);
    }
}

impl Default for DelegatedBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl ChatBackend for DelegatedBackend {
    fn start(
        &self,
        history: Vec<WireMessage>,
        config: ProviderConfig,
        web_access: Option<WebAccess>,
    ) -> mpsc::UnboundedReceiver<ChatStreamEvent> {
        match config.kind {
            ProviderKind::ChatGPT => self.responses.start(history, config, web_access),
            ProviderKind::CodexAppServer => {
                let executable = self
                    .codex_executable
                    .read()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone();
                CodexBackend::new(executable).start(history, config, web_access)
            }
            // Do not re-enter client::run for a subscription kind (it delegates
            // back here); the ordinary kind is safe and keeps the adapter total.
            ProviderKind::OpenAICompatible => client::run(history, config, web_access),
        }
    }
}

/// Alternate name for callers that describe this as a composite backend.
pub type CompositeBackend = DelegatedBackend;
