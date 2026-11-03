//! Opt-in MCP client. Configuration never implies permission to execute a tool.
mod config;
mod runtime;
mod transport;

pub use config::{parse_chat_add, Draft, McpStore, Service, Transport};
pub use runtime::{ApprovalAction, ApprovalRequest, McpTurn, ToolCatalog};
pub use transport::Session;

#[cfg(test)]
mod tests;
