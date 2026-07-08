//! Production HRMS webhook adapter backed by `reqwest`.
//!
//! This adapter supports HTTPS through rustls, request batching, response
//! envelope parsing, and the retry policy inherited from the Python bridge.

use std::{
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    config::BridgeDeviceConfig,
    domain::{FingerTemplatePayload, HrmsEvent},
    ports::hrms::{HrmsClient, HrmsError, WebhookResult, BATCH_SIZE},
};

const MAX_RETRIES: usize = 3;
const RETRY_BACKOFF_SECONDS: u64 = 2;
/// Maximum jitter added to each retry delay (milliseconds).
const RETRY_JITTER_MS: u64 = 500;

#[derive(Debug, Clone)]
/// HTTPS-capable HRMS webhook client.
pub struct ReqwestHrmsClient {
    client: Client,
}

impl Default for ReqwestHrmsClient {
    fn default() -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(30))
                .user_agent(format!("FingerBridge/{}", env!("CARGO_PKG_VERSION")))
                .build()
                .expect("reqwest client should build"),
        }
    }
}

impl HrmsClient for ReqwestHrmsClient {
    fn forward_events(
        &self,
        webhook_url: &str,
        device: &BridgeDeviceConfig,
        events: &[HrmsEvent],
    ) -> Result<WebhookResult, HrmsError> {
        if events.is_empty() {
            return Ok(WebhookResult {
                received: 0,
                org_timezone: None,
            });
        }

        let mut received = 0;
        let mut org_timezone = None;
        for chunk in events.chunks(BATCH_SIZE) {
            let payload = WebhookPayload {
                organization_id: device.organization_id,
                device_code: &device.device_code,
                api_key: &device.api_key,
                events: chunk,
            };
            let response = self.post_with_retry(webhook_url, &payload)?;
            received += parse_received(&response, chunk.len())?;
            if let Some(tz) = parse_org_timezone(&response) {
                org_timezone = Some(tz);
            }
        }
        Ok(WebhookResult {
            received,
            org_timezone,
        })
    }
}

impl ReqwestHrmsClient {
    /// Fetch pending HRMS jobs for the configured device codes.
    pub fn fetch_pending_jobs(
        &self,
        base_url: &str,
        token: &str,
        device_codes: &[String],
    ) -> Result<Vec<PendingJob>, HrmsError> {
        let codes = device_codes.join(",");
        let url = format!(
            "{}/biometric-devices/pending-jobs?deviceCodes={}",
            base_url.trim_end_matches('/'),
            codes
        );
        let response = self
            .client
            .get(&url)
            .bearer_auth(token)
            .send()
            .map_err(|err| HrmsError::Message(format!("pending jobs request failed: {err}")))?;
        let status = response.status();
        let body = response
            .text()
            .map_err(|err| HrmsError::Message(format!("pending jobs read failed: {err}")))?;
        if !status.is_success() {
            return Err(HrmsError::Message(format!(
                "pending jobs HTTP {}: {}",
                status.as_u16(),
                body.chars().take(300).collect::<String>()
            )));
        }
        parse_jobs_response(&body)
    }

    /// Complete one HRMS job with a success or failure result.
    pub fn complete_job(
        &self,
        base_url: &str,
        token: &str,
        job_id: &str,
        result: &JobCompletion,
    ) -> Result<(), HrmsError> {
        let url = format!(
            "{}/biometric-devices/jobs/{}/complete",
            base_url.trim_end_matches('/'),
            job_id
        );
        let response = self
            .client
            .post(&url)
            .bearer_auth(token)
            .json(result)
            .send()
            .map_err(|err| HrmsError::Message(format!("job completion request failed: {err}")))?;
        let status = response.status();
        let body = response
            .text()
            .map_err(|err| HrmsError::Message(format!("job completion read failed: {err}")))?;
        if status.is_success() {
            Ok(())
        } else {
            Err(HrmsError::Message(format!(
                "job completion HTTP {}: {}",
                status.as_u16(),
                body.chars().take(300).collect::<String>()
            )))
        }
    }

    fn post_with_retry<T: Serialize>(&self, url: &str, payload: &T) -> Result<Value, HrmsError> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            match self.post_once(url, payload) {
                Ok(value) => return Ok(value),
                Err(PostFailure::NonRetryable(message)) => return Err(HrmsError::Message(message)),
                Err(PostFailure::Retryable(message)) if attempt <= MAX_RETRIES => {
                    let base_ms = RETRY_BACKOFF_SECONDS * attempt as u64 * 1000;
                    let jitter_ms = jitter(RETRY_JITTER_MS);
                    let wait = Duration::from_millis(base_ms + jitter_ms);
                    eprintln!(
                        "HRMS webhook failed: {message} - retry {attempt}/{MAX_RETRIES} in {:.1}s",
                        wait.as_secs_f64()
                    );
                    thread::sleep(wait);
                }
                Err(PostFailure::Retryable(message)) => return Err(HrmsError::Message(message)),
            }
        }
    }

    fn post_once<T: Serialize>(&self, url: &str, payload: &T) -> Result<Value, PostFailure> {
        let response = self
            .client
            .post(url)
            .json(payload)
            .send()
            .map_err(|err| PostFailure::Retryable(format!("webhook request failed: {err}")))?;

        let status = response.status();
        let body = response
            .text()
            .map_err(|err| PostFailure::Retryable(format!("webhook read failed: {err}")))?;

        if status.is_success() {
            serde_json::from_str(&body).map_err(|err| {
                PostFailure::NonRetryable(format!("webhook returned invalid JSON: {err}"))
            })
        } else if should_retry_status(status.as_u16()) {
            Err(PostFailure::Retryable(format!(
                "webhook HTTP {}: {}",
                status.as_u16(),
                body.chars().take(300).collect::<String>()
            )))
        } else {
            Err(PostFailure::NonRetryable(format!(
                "webhook HTTP {}: {}",
                status.as_u16(),
                body.chars().take(300).collect::<String>()
            )))
        }
    }
}

/// Pending job returned by HRMS.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PendingJob {
    pub id: String,
    #[serde(rename = "type")]
    pub job_type: String,
    pub device_code: String,
    #[serde(default)]
    pub payload: Option<Value>,
}

/// Result body sent back to HRMS when a job is completed.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobCompletion {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub templates: Option<Vec<FingerTemplatePayload>>,
}

impl JobCompletion {
    pub fn ok() -> Self {
        Self {
            ok: true,
            error: None,
            templates: None,
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            error: Some(truncate_error(message.into())),
            templates: None,
        }
    }

    pub fn templates(templates: Vec<FingerTemplatePayload>) -> Self {
        Self {
            ok: true,
            error: None,
            templates: Some(templates),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebhookPayload<'a> {
    organization_id: u64,
    device_code: &'a str,
    api_key: &'a str,
    events: &'a [HrmsEvent],
}

fn parse_received(value: &Value, fallback: usize) -> Result<usize, HrmsError> {
    if let Some(received) = value
        .get("data")
        .and_then(|data| data.get("received"))
        .and_then(Value::as_u64)
    {
        return Ok(received as usize);
    }
    if let Some(received) = value.get("received").and_then(Value::as_u64) {
        return Ok(received as usize);
    }
    if value.get("ok").and_then(Value::as_bool) == Some(true) {
        return Ok(fallback);
    }
    Ok(fallback)
}

/// Extract `orgTimezone` from either the wrapped (`data.orgTimezone`) or flat
/// (`orgTimezone`) response envelope. Absent/non-string values yield `None`
/// rather than an error — an older HRMS deployment that doesn't send this
/// field yet must not break the webhook call.
fn parse_org_timezone(value: &Value) -> Option<String> {
    value
        .get("data")
        .and_then(|data| data.get("orgTimezone"))
        .and_then(Value::as_str)
        .or_else(|| value.get("orgTimezone").and_then(Value::as_str))
        .map(str::to_string)
}

fn parse_jobs_response(body: &str) -> Result<Vec<PendingJob>, HrmsError> {
    let value: Value = serde_json::from_str(body)
        .map_err(|err| HrmsError::Message(format!("pending jobs returned invalid JSON: {err}")))?;
    let jobs_value = value.get("data").unwrap_or(&value);
    serde_json::from_value(jobs_value.clone())
        .map_err(|err| HrmsError::Message(format!("pending jobs payload is invalid: {err}")))
}

fn truncate_error(mut message: String) -> String {
    if message.len() > 300 {
        message.truncate(300);
    }
    message
}

/// Return a pseudo-random jitter in `0..max_ms` using wall-clock nanoseconds
/// as entropy. No external `rand` dependency needed for this small range.
fn jitter(max_ms: u64) -> u64 {
    if max_ms == 0 {
        return 0;
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    u64::from(nanos) % max_ms
}

fn should_retry_status(status: u16) -> bool {
    status == 429 || (500..600).contains(&status)
}

#[derive(Debug)]
enum PostFailure {
    Retryable(String),
    NonRetryable(String),
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{jitter, parse_jobs_response, parse_received, should_retry_status, JobCompletion};

    #[test]
    fn jitter_is_bounded() {
        for max in [0u64, 1, 100, 500, 1000] {
            let j = jitter(max);
            assert!(j < max.max(1), "jitter({max}) = {j} should be < max");
        }
    }

    #[test]
    fn jitter_zero_max_returns_zero() {
        assert_eq!(jitter(0), 0);
    }

    #[test]
    fn retry_policy_matches_bridge_contract() {
        assert!(should_retry_status(429));
        assert!(should_retry_status(500));
        assert!(should_retry_status(599));
        assert!(!should_retry_status(400));
        assert!(!should_retry_status(401));
        assert!(!should_retry_status(404));
    }

    #[test]
    fn parses_final_hrms_envelope() {
        let value = json!({
            "statusCode": 200,
            "message": "Success",
            "data": { "received": 7 }
        });
        assert_eq!(parse_received(&value, 99).expect("received"), 7);
    }

    #[test]
    fn parses_mock_ok_response() {
        assert_eq!(
            parse_received(&json!({ "ok": true }), 3).expect("received"),
            3
        );
    }

    #[test]
    fn parses_pending_jobs_from_hrms_envelope() {
        let jobs = parse_jobs_response(
            r#"{"statusCode":200,"message":"Success","data":[{"id":"job-1","type":"PUSH_USER","deviceCode":"GATE-01","payload":{"userId":"001"}}]}"#,
        )
        .expect("parse jobs");
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].id, "job-1");
        assert_eq!(jobs[0].job_type, "PUSH_USER");
        assert_eq!(jobs[0].device_code, "GATE-01");
    }

    #[test]
    fn job_completion_error_is_truncated() {
        let completion = JobCompletion::error("x".repeat(400));
        assert!(!completion.ok);
        assert_eq!(completion.error.expect("error").len(), 300);
    }
}
