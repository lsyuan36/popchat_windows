//! Opt-in MCP client. Configuration never implies permission to execute a tool.
mod config;
mod onboarding;
mod runtime;
mod transport;

pub use config::{parse_chat_add, Draft, McpStore, Service, Transport};
#[cfg(test)]
pub(crate) use onboarding::fixture as onboarding_fixture;
pub use onboarding::{parse_chat_setup, LocalSetup};
pub use runtime::{ApprovalAction, ApprovalRequest, McpTurn, ToolCatalog};
pub use transport::Session;

#[cfg(test)]
mod tests;
