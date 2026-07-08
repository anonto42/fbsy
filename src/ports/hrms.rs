//! HRMS webhook boundary.
//!
//! The sync use case should know that events are forwarded, but not whether
//! that is done with `reqwest`, retries, or a test fake.

use serde::Serialize;
use thiserror::Error;

use crate::{config::BridgeDeviceConfig, domain::HrmsEvent};

/// Maximum number of events sent in one webhook request.
pub const BATCH_SIZE: usize = 500;

#[derive(Debug, Error)]
/// Errors returned by the HRMS webhook adapter.
pub enum HrmsError {
    #[error("HRMS operation is not supported by this adapter")]
    NotImplemented,
    #[error("{0}")]
    Message(String),
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
/// Summary returned after forwarding events to HRMS.
pub struct WebhookResult {
    /// Number of records accepted by HRMS.
    pub received: usize,
    /// The organization's configured IANA timezone, if HRMS returned one.
    ///
    /// Used as a fallback source of truth when this device has no explicit
    /// `deviceTimezone` configured locally — HRMS becomes the single source
    /// of truth for the org's timezone instead of requiring every bridge
    /// deployment to have it configured (and risking drift if it's ever
    /// changed in HRMS but not in the field).
    pub org_timezone: Option<String>,
}

/// Abstraction for sending attendance events to HRMS.
pub trait HrmsClient: Send + Sync {
    /// Forward a batch/list of normalized HRMS events.
    fn forward_events(
        &self,
        webhook_url: &str,
        device: &BridgeDeviceConfig,
        events: &[HrmsEvent],
    ) -> Result<WebhookResult, HrmsError>;
}
