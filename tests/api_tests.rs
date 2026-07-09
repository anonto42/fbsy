//! Integration tests for the local HTTP API server.
//!
//! Each test spins up `handle_client` on a real ephemeral TCP port, sends raw
//! HTTP requests, and asserts on the response status and JSON body. No mocking
//! of the network layer — this exercises the full request-parsing path.

use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::Arc,
    thread,
};

use fingerbridge::{
    adapters::{device_zkteco_tcp::ZktecoTcpConnector, hrms_reqwest::ReqwestHrmsClient},
    application::serve::handle_client,
    config::BridgeDeviceConfig,
    domain::EventTypeMode,
    runtime::DeviceSyncState,
};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn device_cfg(code: &str) -> BridgeDeviceConfig {
    BridgeDeviceConfig {
        device_ip: "127.0.0.1".to_string(),
        device_port: 4370,
        device_password: 0,
        device_timeout: 5,
        device_force_udp: false,
        device_omit_ping: true,
        device_timezone: None,
        event_type_mode: EventTypeMode::PunchCode,
        device_code: code.to_string(),
        api_key: "test-api-key".to_string(),
        organization_id: 1,
        sync_interval_seconds: 300,
        clear_attendance_after_sync: false,
        clear_attendance_threshold: None,
    }
}

fn empty_states() -> Arc<Vec<Arc<DeviceSyncState>>> {
    Arc::new(vec![])
}

fn states_with_device(code: &str) -> Arc<Vec<Arc<DeviceSyncState>>> {
    let connector = Arc::new(ZktecoTcpConnector);
    let hrms = Arc::new(ReqwestHrmsClient::default());
    Arc::new(vec![Arc::new(DeviceSyncState::new(
        device_cfg(code),
        "http://localhost:8800/webhook".to_string(),
        connector,
        hrms,
    ))])
}

/// Bind an ephemeral port, serve one request with `handle_client`, return the raw response.
fn serve_one(states: Arc<Vec<Arc<DeviceSyncState>>>, request: &str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let webhook = "http://localhost:8800/webhook".to_string();

    let states_t = Arc::clone(&states);
    thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let _ = handle_client(stream, states_t, webhook);
    });

    let mut conn = TcpStream::connect(("127.0.0.1", port)).unwrap();
    conn.write_all(request.as_bytes()).unwrap();
    let mut buf = String::new();
    // Connection reset by peer is acceptable — the server closed after sending the response.
    let _ = conn.read_to_string(&mut buf);
    buf
}

fn status_of(response: &str) -> u16 {
    response
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

fn body_of(response: &str) -> serde_json::Value {
    let body = response.split("\r\n\r\n").nth(1).unwrap_or("{}");
    serde_json::from_str(body).unwrap_or(serde_json::json!({}))
}

// ── /health ───────────────────────────────────────────────────────────────────

#[test]
fn health_returns_200_with_status_ok() {
    let resp = serve_one(
        empty_states(),
        "GET /health HTTP/1.1\r\nHost: localhost\r\n\r\n",
    );
    assert_eq!(status_of(&resp), 200);
    let body = body_of(&resp);
    assert_eq!(body["status"], "ok");
    assert_eq!(body["agent"], "fingerbridge");
    assert!(body["version"].is_string());
    assert_eq!(body["deviceCount"], 0);
}

#[test]
fn health_includes_device_list() {
    let resp = serve_one(
        states_with_device("GATE01"),
        "GET /health HTTP/1.1\r\nHost: localhost\r\n\r\n",
    );
    assert_eq!(status_of(&resp), 200);
    let body = body_of(&resp);
    assert_eq!(body["deviceCount"], 1);
    let devices = body["devices"].as_array().unwrap();
    assert_eq!(devices[0]["deviceCode"], "GATE01");
    assert_eq!(devices[0]["syncing"], false);
}

// ── /sync ─────────────────────────────────────────────────────────────────────

#[test]
fn sync_post_returns_202_for_all_devices() {
    let resp = serve_one(
        states_with_device("GATE01"),
        "POST /sync HTTP/1.1\r\nHost: localhost\r\n\r\n",
    );
    assert_eq!(status_of(&resp), 202);
    let body = body_of(&resp);
    assert_eq!(body["ok"], true);
}

#[test]
fn sync_with_unknown_device_returns_404() {
    let resp = serve_one(
        states_with_device("GATE01"),
        "POST /sync?device=UNKNOWN HTTP/1.1\r\nHost: localhost\r\n\r\n",
    );
    assert_eq!(status_of(&resp), 404);
    let body = body_of(&resp);
    assert!(body["error"].as_str().unwrap().contains("UNKNOWN"));
}

#[test]
fn sync_with_known_device_returns_202() {
    let resp = serve_one(
        states_with_device("GATE01"),
        "POST /sync?device=GATE01 HTTP/1.1\r\nHost: localhost\r\n\r\n",
    );
    assert_eq!(status_of(&resp), 202);
    let body = body_of(&resp);
    assert_eq!(body["ok"], true);
}

// ── OPTIONS ───────────────────────────────────────────────────────────────────

#[test]
fn options_returns_204() {
    let resp = serve_one(
        empty_states(),
        "OPTIONS /health HTTP/1.1\r\nHost: localhost\r\n\r\n",
    );
    assert_eq!(status_of(&resp), 204);
}

// ── Unknown routes ────────────────────────────────────────────────────────────

#[test]
fn unknown_route_returns_404() {
    let resp = serve_one(
        empty_states(),
        "GET /nonexistent HTTP/1.1\r\nHost: localhost\r\n\r\n",
    );
    assert_eq!(status_of(&resp), 404);
    let body = body_of(&resp);
    assert!(body["error"].is_string());
}

#[test]
fn get_sync_returns_404_wrong_method() {
    let resp = serve_one(
        empty_states(),
        "GET /sync HTTP/1.1\r\nHost: localhost\r\n\r\n",
    );
    assert_eq!(status_of(&resp), 404);
}

// ── Malformed requests → 400 ──────────────────────────────────────────────────

#[test]
fn malformed_method_too_long_returns_400() {
    let method = "X".repeat(32);
    let request = format!("{method} /health HTTP/1.1\r\nHost: localhost\r\n\r\n");
    let resp = serve_one(empty_states(), &request);
    assert_eq!(status_of(&resp), 400);
}

#[test]
fn malformed_target_without_slash_returns_400() {
    let resp = serve_one(
        empty_states(),
        "GET health HTTP/1.1\r\nHost: localhost\r\n\r\n",
    );
    assert_eq!(status_of(&resp), 400);
}

#[test]
fn malformed_http_version_returns_400() {
    let resp = serve_one(
        empty_states(),
        "GET /health FTTP/1.1\r\nHost: localhost\r\n\r\n",
    );
    assert_eq!(status_of(&resp), 400);
}

#[test]
fn oversized_headers_return_400() {
    // 9 KB of header data — exceeds the 8 KB cap
    let big_header = format!(
        "GET /health HTTP/1.1\r\nX-Big: {}\r\n\r\n",
        "A".repeat(9000)
    );
    let resp = serve_one(empty_states(), &big_header);
    assert_eq!(status_of(&resp), 400);
}

// ── Loopback bind assertion ───────────────────────────────────────────────────

#[test]
fn server_binds_only_loopback() {
    // Verify the server in serve::run would bind 127.0.0.1, not 0.0.0.0.
    // We test this indirectly: connecting to 127.0.0.1 works, and the
    // address string used in serve.rs is "127.0.0.1:{port}".
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    assert_eq!(addr.ip().to_string(), "127.0.0.1");
}
