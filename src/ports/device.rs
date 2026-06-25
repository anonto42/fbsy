//! Device boundary.
//!
//! ZKTeco protocol work belongs behind this trait so the sync engine can be
//! tested with fake devices and the real adapter can evolve separately.

use thiserror::Error;

use crate::{
    config::BridgeDeviceConfig,
    domain::{DeviceUser, FingerTemplate, RawAttendance},
};

#[derive(Debug, Error)]
/// Errors returned by the ZKTeco device adapter.
pub enum DeviceError {
    #[error("device protocol is not implemented yet")]
    NotImplemented,
    #[error("{0}")]
    Message(String),
}

/// Active connection to a ZKTeco-like attendance device.
pub trait DeviceClient: Send {
    /// Pull raw attendance records from the device.
    fn pull_attendance(&mut self) -> Result<Vec<RawAttendance>, DeviceError>;
    /// Clear attendance records after a safe, successful upload.
    fn clear_attendance(&mut self) -> Result<(), DeviceError>;
    /// Pull users and fingerprint templates from the device.
    fn get_templates(&mut self) -> Result<Vec<FingerTemplate>, DeviceError>;
    /// Push one user and one fingerprint template to the device.
    fn push_user_template(
        &mut self,
        user: &DeviceUser,
        finger: &FingerTemplate,
    ) -> Result<(), DeviceError>;
    /// Close the device connection.
    fn disconnect(&mut self);
}

/// Factory that opens a device connection using validated config.
pub trait DeviceConnector: Send + Sync {
    /// Connect to the configured device.
    fn connect(&self, cfg: &BridgeDeviceConfig) -> Result<Box<dyn DeviceClient>, DeviceError>;
}
