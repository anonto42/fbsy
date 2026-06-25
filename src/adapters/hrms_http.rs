//! Blocking HTTP HRMS webhook adapter.
//!
//! This is intentionally small and dependency-free so the bridge can be tested
//! against local mock HRMS servers before the production `reqwest` adapter is
//! introduced.

use std::{
    io::{Read, Write},
    net::TcpStream,
    thread,
    time::Duration,
};

use serde::Serialize;
use serde_json::Value;

use crate::{
    config::BridgeDeviceConfig,
    domain::HrmsEvent,
    ports::hrms::{HrmsClient, HrmsError, WebhookResult, BATCH_SIZE},
};

#[derive(Debug, Default)]
/// Minimal HTTP client for webhook POSTs.
pub struct BlockingHttpHrmsClient;

const MAX_RETRIES: usize = 3;
const RETRY_BACKOFF_SECONDS: u64 = 2;

impl HrmsClient for BlockingHttpHrmsClient {
    fn forward_events(
        &self,
        webhook_url: &str,
        device: &BridgeDeviceConfig,
        events: &[HrmsEvent],
    ) -> Result<WebhookResult, HrmsError> {
        if events.is_empty() {
            return Ok(WebhookResult { received: 0 });
        }

        let mut received = 0;
        for chunk in events.chunks(BATCH_SIZE) {
            let payload = WebhookPayload {
                organization_id: device.organization_id,
                device_code: &device.device_code,
                api_key: &device.api_key,
                events: chunk,
            };
            let body = serde_json::to_string(&payload)
                .map_err(|err| HrmsError::Message(format!("serialize webhook payload: {err}")))?;
            let response = post_json_with_retry(webhook_url, &body, MAX_RETRIES)?;
            received += parse_received(&response, chunk.len())?;
        }
        Ok(WebhookResult { received })
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

fn post_json_with_retry(url: &str, body: &str, retries: usize) -> Result<String, HrmsError> {
    let mut attempt = 0;
    loop {
        attempt += 1;
        match post_json_once(url, body) {
            Ok(response) => return Ok(response),
            Err(PostError::NonRetryable(message)) => return Err(HrmsError::Message(message)),
            Err(PostError::Retryable(message)) if attempt <= retries => {
                let wait_seconds = RETRY_BACKOFF_SECONDS * attempt as u64;
                eprintln!(
                    "HRMS webhook failed: {message} - retry {attempt}/{retries} in {wait_seconds}s"
                );
                thread::sleep(Duration::from_secs(wait_seconds));
            }
            Err(PostError::Retryable(message)) => return Err(HrmsError::Message(message)),
        }
    }
}

fn post_json_once(url: &str, body: &str) -> Result<String, PostError> {
    let parsed = ParsedHttpUrl::parse(url)?;
    let mut stream = TcpStream::connect((parsed.host.as_str(), parsed.port))
        .map_err(|err| PostError::Retryable(format!("HRMS connect failed: {err}")))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .map_err(|err| PostError::Retryable(format!("set HRMS read timeout failed: {err}")))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(30)))
        .map_err(|err| PostError::Retryable(format!("set HRMS write timeout failed: {err}")))?;

    let request = format!(
        "POST {} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nAccept: application/json\r\nUser-Agent: ZKTecoBridgeRust/0.1.0\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        parsed.path,
        parsed.host_header(),
        body.len(),
        body
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|err| PostError::Retryable(format!("HRMS write failed: {err}")))?;

    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|err| PostError::Retryable(format!("HRMS read failed: {err}")))?;
    let (head, body) = response
        .split_once("\r\n\r\n")
        .ok_or_else(|| PostError::Retryable("HRMS returned invalid HTTP response".to_string()))?;
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| PostError::Retryable("HRMS response missing status code".to_string()))?;

    if (200..300).contains(&status) {
        Ok(body.to_string())
    } else if should_retry_status(status) {
        Err(PostError::Retryable(format!(
            "HRMS webhook HTTP {status}: {}",
            body.chars().take(300).collect::<String>()
        )))
    } else {
        Err(PostError::NonRetryable(format!(
            "HRMS webhook HTTP {status}: {}",
            body.chars().take(300).collect::<String>()
        )))
    }
}

fn parse_received(body: &str, fallback: usize) -> Result<usize, HrmsError> {
    let value: Value = serde_json::from_str(body)
        .map_err(|err| HrmsError::Message(format!("HRMS returned invalid JSON: {err}")))?;
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

fn should_retry_status(status: u16) -> bool {
    status == 429 || (500..600).contains(&status)
}

#[derive(Debug)]
enum PostError {
    Retryable(String),
    NonRetryable(String),
}

impl From<HrmsError> for PostError {
    fn from(error: HrmsError) -> Self {
        Self::NonRetryable(error.to_string())
    }
}

#[derive(Debug)]
struct ParsedHttpUrl {
    host: String,
    port: u16,
    path: String,
}

impl ParsedHttpUrl {
    fn parse(url: &str) -> Result<Self, HrmsError> {
        let rest = url.strip_prefix("http://").ok_or_else(|| {
            HrmsError::Message("mock HRMS client supports http:// URLs".to_string())
        })?;
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) => {
                let port = port
                    .parse::<u16>()
                    .map_err(|_| HrmsError::Message("invalid HRMS URL port".to_string()))?;
                (host.to_string(), port)
            }
            None => (authority.to_string(), 80),
        };
        if host.is_empty() {
            return Err(HrmsError::Message("HRMS URL missing host".to_string()));
        }
        Ok(Self {
            host,
            port,
            path: format!("/{}", path),
        })
    }

    fn host_header(&self) -> String {
        if self.port == 80 {
            self.host.clone()
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_received, should_retry_status, ParsedHttpUrl};

    #[test]
    fn retry_policy_matches_python_bridge() {
        assert!(should_retry_status(429));
        assert!(should_retry_status(500));
        assert!(should_retry_status(503));
        assert!(!should_retry_status(400));
        assert!(!should_retry_status(401));
        assert!(!should_retry_status(404));
        assert!(!should_retry_status(200));
    }

    #[test]
    fn parses_final_hrms_response_envelope() {
        let received = parse_received(
            r#"{"statusCode":200,"message":"Success","data":{"received":2}}"#,
            99,
        )
        .expect("parse received");
        assert_eq!(received, 2);
    }

    #[test]
    fn parses_legacy_mock_response_shape() {
        let received = parse_received(r#"{"ok":true}"#, 3).expect("parse mock response");
        assert_eq!(received, 3);
    }

    #[test]
    fn http_url_parser_handles_host_port_and_path() {
        let parsed =
            ParsedHttpUrl::parse("http://127.0.0.1:18800/webhook").expect("parse mock URL");
        assert_eq!(parsed.host, "127.0.0.1");
        assert_eq!(parsed.port, 18800);
        assert_eq!(parsed.path, "/webhook");
        assert_eq!(parsed.host_header(), "127.0.0.1:18800");
    }

    #[test]
    fn http_url_parser_rejects_https_for_blocking_mock_client() {
        assert!(ParsedHttpUrl::parse("https://example.test/webhook").is_err());
    }
}
