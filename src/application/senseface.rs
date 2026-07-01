use std::{
    io::{self, Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

use anyhow::Result;
use chrono::Utc;

use crate::{
    config::SenseFaceConfig,
    domain::senseface::{
        compute_event_key, compute_request_hash, parse_attendance_line, parse_user_line,
    },
    ports::senseface_store::SenseFaceStore,
    support::log,
};

const MAX_HEADER_BYTES: usize = 16384;
const MAX_BODY_BYTES: usize = 10 * 1024 * 1024;

pub struct SenseFaceReceiver {
    store: Arc<dyn SenseFaceStore>,
    config: SenseFaceConfig,
    shutdown: Arc<AtomicBool>,
}

impl SenseFaceReceiver {
    pub fn new(
        store: Arc<dyn SenseFaceStore>,
        config: SenseFaceConfig,
        shutdown: Arc<AtomicBool>,
    ) -> Self {
        Self {
            store,
            config,
            shutdown,
        }
    }

    pub fn run(&self) -> Result<()> {
        let address = format!("{}:{}", self.config.bind_host, self.config.port);
        let listener = TcpListener::bind(&address)?;
        listener.set_nonblocking(true)?;

        log::info(
            "senseface",
            format_args!("ADMS receiver listening on http://{address}"),
        );

        loop {
            if self.shutdown.load(Ordering::SeqCst) {
                log::info("senseface", format_args!("receiver stopping"));
                break;
            }
            match listener.accept() {
                Ok((stream, _addr)) => {
                    let store = Arc::clone(&self.store);
                    let config = self.config.clone();
                    let shutdown = Arc::clone(&self.shutdown);
                    thread::spawn(move || {
                        let _ = handle_connection(stream, store, &config, shutdown);
                    });
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(200));
                }
                Err(err) => log::error("senseface", format_args!("accept failed: {err}")),
            }
        }

        Ok(())
    }
}

fn handle_connection(
    mut stream: TcpStream,
    store: Arc<dyn SenseFaceStore>,
    config: &SenseFaceConfig,
    shutdown: Arc<AtomicBool>,
) -> Result<()> {
    let remote = stream.peer_addr().ok();
    let remote_ip = remote
        .as_ref()
        .map(|a| a.ip().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let mut raw_headers = Vec::new();
    let mut buf = [0u8; 1];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => return Ok(()),
            Ok(_) => {
                raw_headers.push(buf[0]);
                if raw_headers.len() > MAX_HEADER_BYTES {
                    let _ = write_error(&mut stream, 400, "request headers too large");
                    return Ok(());
                }
                if raw_headers.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            Err(e) => {
                log::warn(
                    "senseface",
                    format_args!("read error from {remote_ip}: {e}"),
                );
                return Ok(());
            }
        }
    }

    let header_text = String::from_utf8_lossy(&raw_headers);
    let mut lines = header_text.lines();
    let request_line = match lines.next() {
        Some(line) => line,
        None => {
            let _ = write_error(&mut stream, 400, "empty request");
            return Ok(());
        }
    };

    let mut parts = request_line.split_ascii_whitespace();
    let method = match parts.next() {
        Some(m) if m.len() <= 16 => m,
        _ => {
            let _ = write_error(&mut stream, 400, "malformed request");
            return Ok(());
        }
    };
    let target = match parts.next() {
        Some(t) if t.starts_with('/') && t.len() <= 1024 => t,
        _ => {
            let _ = write_error(&mut stream, 400, "malformed request");
            return Ok(());
        }
    };

    let content_length: usize = lines
        .filter_map(|line| {
            let lower = line.to_ascii_lowercase();
            lower
                .strip_prefix("content-length:")
                .and_then(|v| v.trim().parse().ok())
        })
        .next()
        .unwrap_or(0);

    let body = if content_length > 0 {
        if content_length > MAX_BODY_BYTES {
            let _ = write_error(&mut stream, 413, "request body too large");
            return Ok(());
        }
        let mut body_buf = vec![0u8; content_length];
        let mut read_total = 0usize;
        while read_total < content_length {
            // Check shutdown periodically during slow body reads
            if shutdown.load(Ordering::SeqCst) {
                return Ok(());
            }
            match stream.read(&mut body_buf[read_total..]) {
                Ok(0) => break,
                Ok(n) => read_total += n,
                Err(ref e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => {
                    log::warn(
                        "senseface",
                        format_args!("body read error from {remote_ip}: {e}"),
                    );
                    return Ok(());
                }
            }
        }
        body_buf.truncate(read_total);
        body_buf
    } else {
        Vec::new()
    };

    if shutdown.load(Ordering::SeqCst) {
        return Ok(());
    }

    let (path, query) = target.split_once('?').unwrap_or((target, ""));

    match (method, path) {
        ("GET", "/iclock/cdata") => handle_get_cdata(&mut stream, query),
        ("GET", "/iclock/getrequest") => handle_get_getrequest(&mut stream, &store, query),
        ("POST", "/iclock/cdata") => {
            handle_post_cdata(&mut stream, &store, config, query, &body, &remote_ip)
        }
        ("POST", "/iclock/devicecmd") => handle_post_devicecmd(&mut stream),
        ("POST", "/iclock/registry") => handle_post_registry(&mut stream),
        ("GET", "/senseface/health") | ("GET", "/health") => {
            handle_senseface_health(&mut stream, &store, config)
        }
        ("GET", "/") => handle_root(&mut stream),
        ("OPTIONS", _) => write_options(&mut stream),
        _ => write_error(&mut stream, 404, "not found"),
    }
}

fn handle_get_cdata(stream: &mut TcpStream, query: &str) -> Result<()> {
    let sn = extract_sn(query);
    let now_local = Utc::now();
    let device_now = now_local.format("%Y-%m-%d %H:%M:%S").to_string();
    let tz_hours = 0;

    let reply = format!(
        "GET OPTION FROM: {sn}\r\n\
         ATTLOGStamp=0\r\n\
         OPERLOGStamp=0\r\n\
         ATTPHOTOStamp=0\r\n\
         ErrorDelay=60\r\n\
         Delay=10\r\n\
         TransTimes=00:00\r\n\
         TransInterval=1\r\n\
         TransFlag=TransData AttLog OpLog EnrollUser ChgUser\r\n\
         TimeZone={tz_hours}\r\n\
         DateTime={device_now}\r\n\
         Realtime=1\r\n\
         Encrypt=0\r\n\
         PushProtVer=2.4.1\r\n"
    );
    write_plain(stream, 200, &reply)
}

fn handle_get_getrequest(
    stream: &mut TcpStream,
    store: &Arc<dyn SenseFaceStore>,
    query: &str,
) -> Result<()> {
    let sn = extract_sn(query);
    let missing = store.count_missing_employees(&sn).unwrap_or(0);

    if missing > 0 {
        let command_id = Utc::now().timestamp();
        let reply = format!("C:{command_id}:DATA QUERY USERINFO\r\n");
        write_plain(stream, 200, &reply)
    } else {
        write_plain(stream, 200, "\r\n")
    }
}

fn handle_post_cdata(
    stream: &mut TcpStream,
    store: &Arc<dyn SenseFaceStore>,
    _config: &SenseFaceConfig,
    query: &str,
    body: &[u8],
    remote_ip: &str,
) -> Result<()> {
    let sn = extract_sn(query);
    let table = extract_table(query);

    let request_hash = compute_request_hash(&sn, query, body);
    let _ = store.save_raw_request(&request_hash, &sn, &table, query, body);

    let _ = store.upsert_device(&sn, remote_ip);

    let text = String::from_utf8_lossy(body);
    let mut inserted = 0usize;

    for line in text.lines() {
        if let Some(attendance) = parse_attendance_line(line) {
            let event_key = compute_event_key(&attendance.raw_line);
            let record = crate::domain::senseface::SenseFaceAttendance {
                employee_id: attendance.employee_id.clone(),
                event_time: attendance.event_time.clone(),
                status: attendance.status,
                verify_mode: attendance.verify_mode,
                work_code: attendance.work_code,
                reserved: attendance.reserved,
                raw_line: attendance.raw_line,
                event_key,
            };
            match store.save_attendance_record(&sn, &[record]) {
                Ok(n) => inserted += n,
                Err(err) => {
                    log::warn("senseface", format_args!("save attendance failed: {err}"));
                }
            }
            let employee_record = crate::domain::senseface::SenseFaceUser {
                employee_id: attendance.employee_id,
                name: String::new(),
                privilege: String::new(),
                card: String::new(),
                raw_line: "DISCOVERED_FROM_ATTENDANCE".to_string(),
            };
            let _ = store.upsert_employee(&sn, &employee_record);
        }
    }

    for line in text.lines() {
        if let Some(user) = parse_user_line(line) {
            if let Err(err) = store.upsert_employee(&sn, &user) {
                log::warn("senseface", format_args!("save user failed: {err}"));
            }
        }
    }

    let _ = stream.set_nonblocking(false);
    if inserted > 0 {
        write_plain(stream, 200, &format!("OK: {inserted}"))
    } else {
        write_plain(stream, 200, "OK")
    }
}

fn handle_post_devicecmd(stream: &mut TcpStream) -> Result<()> {
    write_plain(stream, 200, "OK")
}

fn handle_post_registry(stream: &mut TcpStream) -> Result<()> {
    write_plain(stream, 200, "OK")
}

fn handle_senseface_health(
    stream: &mut TcpStream,
    store: &Arc<dyn SenseFaceStore>,
    _config: &SenseFaceConfig,
) -> Result<()> {
    let pending_count = store
        .get_pending_forward_attendance(1)
        .ok()
        .map(|r| r.len())
        .unwrap_or(0);

    let body = serde_json::json!({
        "status": "ok",
        "service": "senseface-adms-receiver",
        "server_time": Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        "pending_forward": pending_count,
    });
    write_json(stream, 200, &body)
}

fn handle_root(stream: &mut TcpStream) -> Result<()> {
    let body = serde_json::json!({
        "service": "FingerBridge SenseFace ADMS Receiver",
        "status": "ok",
        "endpoints": {
            "health": "/senseface/health",
            "cdata": "GET/POST /iclock/cdata?SN=...",
            "getrequest": "GET /iclock/getrequest?SN=..."
        }
    });
    write_json(stream, 200, &body)
}

fn extract_sn(query: &str) -> String {
    for pair in query.split('&') {
        if let Some(value) = pair.strip_prefix("SN=") {
            return url_decode(value);
        }
    }
    "UNKNOWN".to_string()
}

fn extract_table(query: &str) -> String {
    for pair in query.split('&') {
        if let Some(value) = pair.strip_prefix("table=") {
            return value.to_uppercase();
        }
    }
    String::new()
}

fn url_decode(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            let hex: String = chars.by_ref().take(2).collect();
            if hex.len() == 2 {
                if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                    result.push(byte as char);
                    continue;
                }
            }
            result.push('%');
            result.push_str(&hex);
        } else if c == '+' {
            result.push(' ');
        } else {
            result.push(c);
        }
    }
    result
}

fn write_plain(stream: &mut TcpStream, status: u16, body: &str) -> Result<()> {
    let status_text = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        413 => "Request Entity Too Large",
        _ => "OK",
    };
    let response = format!(
        "HTTP/1.1 {status} {status_text}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes())?;
    Ok(())
}

fn write_json<T: serde::Serialize>(stream: &mut TcpStream, status: u16, body: &T) -> Result<()> {
    let status_text = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "OK",
    };
    let body = serde_json::to_string(body)?;
    let response = format!(
        "HTTP/1.1 {status} {status_text}\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes())?;
    Ok(())
}

fn write_options(stream: &mut TcpStream) -> Result<()> {
    stream.write_all(
        b"HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type, X-API-Key\r\nContent-Length: 0\r\n\r\n",
    )?;
    Ok(())
}

fn write_error(stream: &mut TcpStream, status: u16, message: &str) -> Result<()> {
    let body = serde_json::json!({ "error": message });
    write_json(stream, status, &body)
}
