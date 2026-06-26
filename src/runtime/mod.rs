//! Runtime code for detached services, job polling, process state, and sync
//! coordination.

pub mod job_poller;
pub mod process;
pub mod registry;
pub mod sync_state;

pub use sync_state::DeviceSyncState;
