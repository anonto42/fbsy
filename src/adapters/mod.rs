//! Concrete adapter implementations.
//!
//! Adapters touch external systems such as files, HTTP, operating-system
//! services, or device protocols. Application code should call ports, not these
//! details directly, except while the scaffold is still wiring itself together.

pub mod config_file;
pub mod device_zkteco_tcp;
pub mod hrms_reqwest;
