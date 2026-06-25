//! Library root for FingerBridge.
//!
//! The binary (`main.rs`) depends on this library. Keeping behavior here makes
//! the code easier to test and prevents the executable entrypoint from growing
//! into a mixed bag of CLI, config, device, and sync logic.

/// Concrete implementations of external systems, such as file-backed config.
pub mod adapters;
/// Product use cases called by the CLI and, later, the local HTTP API.
pub mod application;
/// Command parsing and terminal-facing flow.
pub mod cli;
/// Configuration data model and validation rules.
pub mod config;
/// Core business types that should not depend on CLI, HTTP, or filesystem code.
pub mod domain;
/// Traits that describe external systems the application depends on.
pub mod ports;
/// Long-running process concerns such as scheduling and shutdown.
pub mod runtime;
/// Service identity shared by the registry, process manager, and CLI.
pub mod services;
/// Small shared helpers for paths, redaction, and future logging setup.
pub mod support;
