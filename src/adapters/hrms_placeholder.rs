//! Temporary HRMS adapter used while the real HTTP client is not implemented.
//!
//! Keeping this as an adapter lets application code depend on the `HrmsClient`
//! port from day one.

use crate::{
    config::BridgeDeviceConfig,
    domain::HrmsEvent,
    ports::hrms::{HrmsClient, HrmsError, WebhookResult},
};

#[derive(Debug, Default)]
/// Placeholder client that pretends HRMS accepted every event.
pub struct PlaceholderHrmsClient;

impl HrmsClient for PlaceholderHrmsClient {
    /// Return the number of events as the accepted count.
    fn forward_events(
        &self,
        _webhook_url: &str,
        _device: &BridgeDeviceConfig,
        events: &[HrmsEvent],
    ) -> Result<WebhookResult, HrmsError> {
        Ok(WebhookResult {
            received: events.len(),
        })
    }
}
