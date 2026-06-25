//! Runtime/process concerns.
//!
//! This module will hold scheduler and graceful shutdown code once the bridge
//! becomes a long-running service.

pub mod job_poller;
pub mod process;
pub mod registry;
pub mod scheduler;
pub mod sync_state;

pub use sync_state::DeviceSyncState;
