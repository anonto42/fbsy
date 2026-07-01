//! Ports are traits that describe what the application needs from the outside
//! world.
//!
//! The application depends on these traits, while adapters provide concrete
//! implementations. This is the key seam that keeps sync logic testable.

pub mod config_store;
pub mod device;
pub mod hrms;
pub mod senseface_store;
