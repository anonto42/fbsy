//! Temporary device adapter used until the real ZKTeco protocol is implemented.

use crate::{
    config::BridgeDeviceConfig,
    domain::{DeviceUser, FingerTemplate, RawAttendance},
    ports::device::{DeviceClient, DeviceConnector, DeviceError},
};

#[derive(Debug, Default)]
/// Connector that reports the missing device protocol clearly.
pub struct PlaceholderDeviceConnector;

impl DeviceConnector for PlaceholderDeviceConnector {
    fn connect(&self, _cfg: &BridgeDeviceConfig) -> Result<Box<dyn DeviceClient>, DeviceError> {
        Err(DeviceError::NotImplemented)
    }
}

#[allow(dead_code)]
#[derive(Debug, Default)]
/// Empty client kept for future scaffold experiments.
pub struct PlaceholderDeviceClient;

impl DeviceClient for PlaceholderDeviceClient {
    fn pull_attendance(&mut self) -> Result<Vec<RawAttendance>, DeviceError> {
        Err(DeviceError::NotImplemented)
    }

    fn clear_attendance(&mut self) -> Result<(), DeviceError> {
        Err(DeviceError::NotImplemented)
    }

    fn get_templates(&mut self) -> Result<Vec<FingerTemplate>, DeviceError> {
        Err(DeviceError::NotImplemented)
    }

    fn push_user_template(
        &mut self,
        _user: &DeviceUser,
        _finger: &FingerTemplate,
    ) -> Result<(), DeviceError> {
        Err(DeviceError::NotImplemented)
    }

    fn disconnect(&mut self) {}
}
