pub mod chat;
pub mod instance;
pub mod log;
pub mod paths;
pub mod persistence;
pub mod startup;
pub mod ui;
pub mod update;
pub mod updater;

pub const BUILD_CHANNEL: &str = "LOCAL BUILD";
pub const AUTHOR: &str = env!("CARGO_PKG_AUTHORS");
