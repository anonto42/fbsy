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

/// Identity and storage data read live from a device.
#[derive(Debug, Clone, Default)]
pub struct DeviceInfo {
    pub serial: String,
    pub firmware: String,
    pub platform: String,
    pub name: String,
    /// Number of enrolled users.
    pub users: usize,
    /// Number of stored fingerprint templates.
    pub fingers: usize,
    /// Number of attendance records currently on the device.
    pub records: usize,
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
    /// Read identity + storage info from the device (serial, firmware, counts).
    fn device_info(&mut self) -> Result<DeviceInfo, DeviceError> {
        Err(DeviceError::NotImplemented)
    }
    /// List enrolled users (uid / user_id / name).
    fn get_users(&mut self) -> Result<Vec<DeviceUser>, DeviceError> {
        Err(DeviceError::NotImplemented)
    }
    /// Close the device connection.
    fn disconnect(&mut self);
}

/// Factory that opens a device connection using validated config.
pub trait DeviceConnector: Send + Sync {
    /// Connect to the configured device.
    fn connect(&self, cfg: &BridgeDeviceConfig) -> Result<Box<dyn DeviceClient>, DeviceError>;
}
