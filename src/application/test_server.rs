//! Mock ZKTeco TCP device server and HRMS HTTP webhook server.

use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex},
    thread,
};

use anyhow::{bail, Result};
use chrono::Utc;
use console::style;

use crate::support::network;

const MACHINE_PREPARE_DATA_1: u16 = 20560;
const MACHINE_PREPARE_DATA_2: u16 = 32130;
const USHRT_MAX: u16 = 65535;

const CMD_GET_FREE_SIZES: u16 = 50;
const CMD_OPTIONS_RRQ: u16 = 11;
const CMD_CONNECT: u16 = 1000;
const CMD_EXIT: u16 = 1001;
const CMD_GET_VERSION: u16 = 1100;
const CMD_DB_RRQ: u8 = 7; // read templates buffer
const CMD_USERTEMP_RRQ: u8 = 9; // read users buffer
const CMD_ATTLOG_RRQ: u8 = 13;
const CMD_CLEAR_ATTLOG: u16 = 15;
const CMD_ACK_OK: u16 = 2000;
const CMD_DATA: u16 = 1501;
const CMD_RWB: u16 = 1503;

const MOCK_DEVICE_CODE: &str = "MOCK-GATE-01";
const MOCK_API_KEY: &str = "mock-key";
const MOCK_ORGANIZATION_ID: u64 = 1;
const MOCK_SERIAL: &str = "MOCK-SN-0001";
const MOCK_FIRMWARE: &str = "MockFW 6.60";
const MOCK_PLATFORM: &str = "MOCK_PLATFORM";
const MOCK_DEVICE_NAME: &str = "MockDevice";

#[derive(Clone, Debug)]
struct RawAttendanceMock {
    uid: u16,
    timestamp: u32, // encoded ZKTeco format
    punch: u8,
}

/// Run a mock ZKTeco device server speaking raw TCP protocol.
pub fn run_device(port: u16, records_count: usize) -> Result<()> {
    let bind_addr = format!("0.0.0.0:{port}");
    let display_addr = format!("{}:{port}", network::lan_host_or_loopback());
    let listener = TcpListener::bind(&bind_addr)?;
    println!(
        "{} Mock biometric device server listening on {} with {} records pre-populated.",
        style("✔").green().bold(),
        style(&display_addr).cyan().bold(),
        style(records_count).yellow().bold()
    );
    println!("  Reachable from another LAN device at {display_addr}.");
    println!();
    println!("{}", style("  Setup wizard values").yellow().bold());
    println!(
        "  Device IP:                  {}",
        network::lan_host_or_loopback()
    );
    println!("  Device port:                {port}");
    println!("  Device unique code:         {MOCK_DEVICE_CODE}");
    println!("  Device HRMS API key:        {MOCK_API_KEY}");
    println!("  Organization ID:            {MOCK_ORGANIZATION_ID}");
    println!("  Device connection password: 0");
    println!("  Omit ICMP ping check:       yes");
    println!("  Mock serial:                {MOCK_SERIAL}");
    println!("  Mock firmware:              {MOCK_FIRMWARE}");
    println!();

    // Pre-populate mock attendance records
    let now = Utc::now();
    let mut records = Vec::new();
    for i in 0..records_count {
        let ts = now - chrono::Duration::minutes((records_count - 1 - i) as i64 * 5);
        let year = ts.format("%y").to_string().parse::<u32>().unwrap_or(26);
        let month = ts.format("%m").to_string().parse::<u32>().unwrap_or(6);
        let day = ts.format("%d").to_string().parse::<u32>().unwrap_or(22);
        let hour = ts.format("%H").to_string().parse::<u32>().unwrap_or(8);
        let minute = ts.format("%M").to_string().parse::<u32>().unwrap_or(30);
        let second = ts.format("%S").to_string().parse::<u32>().unwrap_or(0);

        let encoded_time = (year * 12 * 31 + (month - 1) * 31 + day - 1) * (24 * 60 * 60)
            + hour * 3600
            + minute * 60
            + second;

        records.push(RawAttendanceMock {
            uid: (1001 + i) as u16,
            timestamp: encoded_time,
            punch: if i % 2 == 0 { 0 } else { 1 }, // Alternates check-in/out
        });
    }

    let records = Arc::new(Mutex::new(records));

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let records = Arc::clone(&records);
                thread::spawn(move || {
                    let _ = handle_device_client(stream, records);
                });
            }
            Err(err) => eprintln!("Device client accept failed: {err}"),
        }
    }
    Ok(())
}

fn handle_device_client(
    mut stream: TcpStream,
    records: Arc<Mutex<Vec<RawAttendanceMock>>>,
) -> Result<()> {
    let mut session_id = 0;

    loop {
        let mut tcp_header = [0u8; 8];
        if stream.read_exact(&mut tcp_header).is_err() {
            break;
        }
        let p1 = u16::from_le_bytes([tcp_header[0], tcp_header[1]]);
        let p2 = u16::from_le_bytes([tcp_header[2], tcp_header[3]]);
        if p1 != MACHINE_PREPARE_DATA_1 || p2 != MACHINE_PREPARE_DATA_2 {
            bail!("Invalid ZKTeco TCP packet signature");
        }
        let packet_len =
            u32::from_le_bytes([tcp_header[4], tcp_header[5], tcp_header[6], tcp_header[7]])
                as usize;
        let mut packet = vec![0u8; packet_len];
        if stream.read_exact(&mut packet).is_err() {
            break;
        }

        let cmd = u16::from_le_bytes([packet[0], packet[1]]);
        let recv_reply = u16::from_le_bytes([packet[6], packet[7]]);
        let cmd_data = &packet[8..];

        let mut reply_id = recv_reply.wrapping_add(1);
        if reply_id == USHRT_MAX {
            reply_id = 0;
        }

        if cmd == CMD_CONNECT {
            session_id = 12345;
            let reply = make_tcp_packet(CMD_ACK_OK, &[], session_id, reply_id);
            stream.write_all(&reply)?;
        } else if cmd == CMD_GET_FREE_SIZES {
            // Return 20 × i32. Field [4] = users, [6] = fingers, [8] = records.
            let record_count = records.lock().unwrap().len() as i32;
            let mut fields = [0i32; 20];
            fields[4] = 1; // one mock user
            fields[6] = 1; // one mock fingerprint template
            fields[8] = record_count;
            let data: Vec<u8> = fields.iter().flat_map(|f| f.to_le_bytes()).collect();
            let reply = make_tcp_packet(CMD_ACK_OK, &data, session_id, reply_id);
            stream.write_all(&reply)?;
        } else if cmd == CMD_GET_VERSION {
            let body = format!("{MOCK_FIRMWARE}\0");
            let reply = make_tcp_packet(CMD_ACK_OK, body.as_bytes(), session_id, reply_id);
            stream.write_all(&reply)?;
        } else if cmd == CMD_OPTIONS_RRQ {
            // cmd_data is the requested option name, e.g. "~SerialNumber\0".
            let name = cmd_data
                .iter()
                .position(|&b| b == 0)
                .map(|end| &cmd_data[..end])
                .unwrap_or(cmd_data);
            let name = String::from_utf8_lossy(name);
            let value = match name.as_ref() {
                "~SerialNumber" => MOCK_SERIAL,
                "~Platform" => MOCK_PLATFORM,
                "~DeviceName" => MOCK_DEVICE_NAME,
                _ => "",
            };
            let body = format!("{name}={value}\0");
            let reply = make_tcp_packet(CMD_ACK_OK, body.as_bytes(), session_id, reply_id);
            stream.write_all(&reply)?;
        } else if cmd == CMD_EXIT {
            let reply = make_tcp_packet(CMD_ACK_OK, &[], session_id, reply_id);
            stream.write_all(&reply)?;
            break;
        } else if cmd == CMD_RWB {
            let sub_cmd = if cmd_data.len() >= 2 { cmd_data[1] } else { 0 };
            if sub_cmd == CMD_ATTLOG_RRQ {
                let list = records.lock().unwrap();
                let mut record_bytes = Vec::new();
                for r in list.iter() {
                    record_bytes.extend(r.uid.to_le_bytes());
                    record_bytes.push(1); // status (1)
                    record_bytes.extend(r.timestamp.to_le_bytes());
                    record_bytes.push(r.punch);
                }
                let declared_size = record_bytes.len() as u32;
                let payload = [
                    declared_size.to_le_bytes().as_slice(),
                    record_bytes.as_slice(),
                ]
                .concat();
                let reply = make_tcp_packet(CMD_DATA, &payload, session_id, reply_id);
                stream.write_all(&reply)?;
            } else if sub_cmd == CMD_USERTEMP_RRQ {
                // One mock user (72-byte zk8 record): uid 1001, user_id "1001".
                let user = mock_user_record(1001, "1001", "MockUser");
                let payload = [
                    (user.len() as u32).to_le_bytes().as_slice(),
                    user.as_slice(),
                ]
                .concat();
                let reply = make_tcp_packet(CMD_DATA, &payload, session_id, reply_id);
                stream.write_all(&reply)?;
            } else if sub_cmd == CMD_DB_RRQ {
                // One mock fingerprint template for uid 1001.
                let rec = mock_template_record(1001, 0, &[0xAA, 0xBB, 0xCC, 0xDD]);
                let payload =
                    [(rec.len() as u32).to_le_bytes().as_slice(), rec.as_slice()].concat();
                let reply = make_tcp_packet(CMD_DATA, &payload, session_id, reply_id);
                stream.write_all(&reply)?;
            } else {
                let reply = make_tcp_packet(CMD_ACK_OK, &[], session_id, reply_id);
                stream.write_all(&reply)?;
            }
        } else if cmd == CMD_CLEAR_ATTLOG {
            if let Ok(mut list) = records.lock() {
                println!(
                    "{} Attendance logs cleared on mock device (records count: {})",
                    style("✔").green().bold(),
                    list.len()
                );
                list.clear();
            }
            let reply = make_tcp_packet(CMD_ACK_OK, &[], session_id, reply_id);
            stream.write_all(&reply)?;
        } else {
            let reply = make_tcp_packet(CMD_ACK_OK, &[], session_id, reply_id);
            stream.write_all(&reply)?;
        }
    }
    Ok(())
}

fn make_tcp_packet(command: u16, data: &[u8], session_id: u16, reply_id: u16) -> Vec<u8> {
    let mut header_template = Vec::with_capacity(8);
    header_template.extend(command.to_le_bytes());
    header_template.extend(0u16.to_le_bytes());
    header_template.extend(session_id.to_le_bytes());
    header_template.extend(reply_id.to_le_bytes());

    let checksum = calc_checksum(&[header_template.as_slice(), data].concat());
    let mut packet = Vec::with_capacity(16 + data.len());
    packet.extend(MACHINE_PREPARE_DATA_1.to_le_bytes());
    packet.extend(MACHINE_PREPARE_DATA_2.to_le_bytes());
    packet.extend(((8 + data.len()) as u32).to_le_bytes());
    packet.extend(command.to_le_bytes());
    packet.extend(checksum.to_le_bytes());
    packet.extend(session_id.to_le_bytes());
    packet.extend(reply_id.to_le_bytes());
    packet.extend(data);
    packet
}

fn calc_checksum(data: &[u8]) -> u16 {
    let mut checksum: u32 = 0;
    for chunk in data.chunks(2) {
        let value = if chunk.len() == 2 {
            u16::from_le_bytes([chunk[0], chunk[1]]) as u32
        } else {
            chunk[0] as u32
        };
        checksum += value;
        while checksum > USHRT_MAX as u32 {
            checksum -= USHRT_MAX as u32;
        }
    }
    !(checksum as u16)
}

/// Build a 72-byte zk8 user record (matches the bridge's `decode_users`).
fn mock_user_record(uid: u16, user_id: &str, name: &str) -> Vec<u8> {
    let fixed = |s: &str, n: usize| -> Vec<u8> {
        let mut b = vec![0u8; n];
        let bytes = s.as_bytes();
        let k = bytes.len().min(n);
        b[..k].copy_from_slice(&bytes[..k]);
        b
    };
    let mut b = Vec::with_capacity(72);
    b.extend(uid.to_le_bytes()); // uid (H)
    b.push(0); // privilege (B)
    b.extend(fixed("", 8)); // password (8s)
    b.extend(fixed(name, 24)); // name (24s)
    b.extend(0u32.to_le_bytes()); // card (I)
    b.push(0); // pad (x)
    b.extend(fixed("0", 7)); // group (7s)
    b.push(0); // pad (x)
    b.extend(fixed(user_id, 24)); // user_id (24s)
    b
}

/// Build a template record: size(H) uid(H) fid(b) valid(b) template[size-6].
fn mock_template_record(uid: u16, fid: u8, template: &[u8]) -> Vec<u8> {
    let size = (template.len() + 6) as u16;
    let mut b = Vec::with_capacity(size as usize);
    b.extend(size.to_le_bytes());
    b.extend(uid.to_le_bytes());
    b.push(fid);
    b.push(1); // valid
    b.extend_from_slice(template);
    b
}

/// Run a mock HRMS webhook API server.
pub fn run_hrms(port: u16) -> Result<()> {
    let bind_addr = format!("0.0.0.0:{port}");
    let display_url = format!("http://{}:{port}", network::lan_host_or_loopback());
    let listener = TcpListener::bind(&bind_addr)?;
    println!(
        "{} Mock HRMS server listening on {}.",
        style("✔").green().bold(),
        style(&display_url).cyan().bold()
    );
    println!("  Reachable from another LAN device at {display_url}/webhook.");
    println!();
    println!("{}", style("  Setup wizard values").yellow().bold());
    println!("  HRMS Webhook URL: {display_url}/webhook");
    println!("  HRMS Base URL:    {display_url}/api/v1");
    println!("  API token:        blank is OK for mock job polling");
    println!();
    println!("  POST  /webhook                         ← receives attendance events");
    println!("  GET   /api/v1/biometric-devices/pending-jobs ← returns [] (job poller)");
    println!("  POST  /api/v1/biometric-devices/jobs/*/complete ← job completion");
    println!("  GET   /events                          ← inspect received events");
    println!("  GET   /reset                           ← clear received events");
    println!("  GET   /health                          ← server health check");

    let received_events = Arc::new(Mutex::new(Vec::<serde_json::Value>::new()));

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let received_events = Arc::clone(&received_events);
                thread::spawn(move || {
                    let _ = handle_hrms_client(stream, received_events);
                });
            }
            Err(err) => eprintln!("HRMS client accept failed: {err}"),
        }
    }
    Ok(())
}

fn handle_hrms_client(
    mut stream: TcpStream,
    received_events: Arc<Mutex<Vec<serde_json::Value>>>,
) -> Result<()> {
    let mut buffer = [0u8; 65536];
    let read = stream.read(&mut buffer)?;
    let request = String::from_utf8_lossy(&buffer[..read]);
    let request_line = request.lines().next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let path = parts.next().unwrap_or_default();
    let (clean_path, _query) = path.split_once('?').unwrap_or((path, ""));

    if method == "POST" {
        if let Some(body_start) = request.find("\r\n\r\n") {
            let body = &request[body_start + 4..];
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(body) {
                println!(
                    "{} Received HRMS Event Payload:\n{}",
                    style("➡").cyan().bold(),
                    serde_json::to_string_pretty(&val).unwrap_or_default()
                );
                if let Ok(mut list) = received_events.lock() {
                    list.push(val);
                }
            }
        }
        let response = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: 11\r\n\r\n{\"ok\":true}";
        stream.write_all(response.as_bytes())?;
    } else if method == "GET" && clean_path == "/events" {
        // Debug: return all received attendance events
        let list = received_events.lock().unwrap();
        let body = serde_json::to_string_pretty(&*list)?;
        http_json(&mut stream, 200, &body)?;
    } else if method == "GET" && clean_path == "/reset" {
        // Debug: clear received events
        received_events.lock().unwrap().clear();
        http_json(&mut stream, 200, r#"{"ok":true}"#)?;
    } else if method == "GET" && clean_path == "/health" {
        http_json(&mut stream, 200, r#"{"ok":true,"agent":"mock-hrms"}"#)?;
    } else if method == "GET"
        && (clean_path.ends_with("/pending-jobs")
            || clean_path.contains("/biometric-devices/pending-jobs"))
    {
        // Job poller endpoint — return empty list so the poller runs without errors
        let body = r#"{"statusCode":200,"message":"Success","data":[]}"#;
        http_json(&mut stream, 200, body)?;
    } else if method == "POST"
        && (clean_path.contains("/jobs/") && clean_path.ends_with("/complete"))
    {
        // Job completion endpoint
        let job_id = clean_path
            .split("/jobs/")
            .nth(1)
            .and_then(|s| s.split('/').next())
            .unwrap_or("unknown");
        println!(
            "{} Job completed: {}",
            style("✔").green().bold(),
            style(job_id).yellow()
        );
        let body = r#"{"statusCode":200,"message":"Success","data":{}}"#;
        http_json(&mut stream, 200, body)?;
    } else {
        http_json(&mut stream, 404, r#"{"ok":false,"message":"not found"}"#)?;
    }
    Ok(())
}

fn http_json(stream: &mut TcpStream, status: u16, body: &str) -> Result<()> {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        _ => "Unknown",
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes())?;
    Ok(())
}
