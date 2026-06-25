//! Application use cases.
//!
//! This layer is the product brain. CLI commands and future HTTP handlers call
//! these functions; device/webhook/file details stay behind ports and adapters.

pub mod autostart;
pub mod config;
pub mod doctor;
pub mod serve;
pub mod setup;
pub mod sync_once;
pub mod test_server;
