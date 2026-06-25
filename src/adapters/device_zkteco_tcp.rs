//! ZKTeco TCP adapter — pyzk-compatible protocol for real devices.
//!
//! Protocol flow (ported from pyzk/zk/base.py):
//!
//! 1. Connect: CMD_CONNECT. If CMD_ACK_UNAUTH, send CMD_AUTH with commkey.
//! 2. Pull attendance via CMD_PREPARE_BUFFER (1503):
//!    - CMD_DATA response: payload is inline.
//!    - CMD_PREPARE_DATA response: read chunks via CMD_READ_BUFFER, then CMD_FREE_DATA.
//! 3. Three wire record formats (8, 16, or 40 bytes each) depending on firmware.
//!    Record size is determined from CMD_GET_FREE_SIZES or detected heuristically.
//! 4. Clear: CMD_CLEAR_ATTLOG.

use std::{
    io::{Read, Write},
    net::TcpStream,
    time::Duration,
};

use crate::{
    config::BridgeDeviceConfig,
    domain::{DeviceUser, FingerTemplate, RawAttendance},
    ports::device::{DeviceClient, DeviceConnector, DeviceError},
};

// ── Protocol constants ────────────────────────────────────────────────────────

const MACHINE_PREPARE_DATA_1: u16 = 20560;
const MACHINE_PREPARE_DATA_2: u16 = 32130;
const USHRT_MAX: u16 = 65535;

const CMD_GET_FREE_SIZES: u16 = 50;
const CMD_ATTLOG_RRQ: u8 = 13;
const CMD_CLEAR_ATTLOG: u16 = 15;
const CMD_CONNECT: u16 = 1000;
const CMD_EXIT: u16 = 1001;
const CMD_AUTH: u16 = 1102;
const CMD_PREPARE_DATA: u16 = 1500;
const CMD_DATA: u16 = 1501;
const CMD_FREE_DATA: u16 = 1502;
const CMD_PREPARE_BUFFER: u16 = 1503;
const CMD_READ_BUFFER: u16 = 1504;
const CMD_ACK_OK: u16 = 2000;
const CMD_ACK_UNAUTH: u16 = 2005;

const MAX_CHUNK: usize = 0xff_c0; // 65 472 bytes per chunk

// ── Connector ────────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct ZktecoTcpConnector;

impl DeviceConnector for ZktecoTcpConnector {
    fn connect(&self, cfg: &BridgeDeviceConfig) -> Result<Box<dyn DeviceClient>, DeviceError> {
        let address = format!("{}:{}", cfg.device_ip, cfg.device_port);
        let timeout = Duration::from_secs(cfg.device_timeout);
        let stream = TcpStream::connect(&address)
            .map_err(|e| DeviceError::Message(format!("device connect failed: {e}")))?;
        stream
            .set_read_timeout(Some(timeout))
            .map_err(|e| DeviceError::Message(format!("set read timeout: {e}")))?;
        stream
            .set_write_timeout(Some(timeout))
            .map_err(|e| DeviceError::Message(format!("set write timeout: {e}")))?;

        if cfg.device_force_udp {
            eprintln!("warning: deviceForceUdp = true is not yet supported — connecting via TCP");
        }

        let mut client = ZktecoTcpClient {
            stream,
            session_id: 0,
            reply_id: USHRT_MAX - 1,
        };

        let resp = client.send_command(CMD_CONNECT, &[])?;

        if resp.command == CMD_ACK_UNAUTH {
            // Device requires password authentication.
            client.session_id = resp.session_id;
            client.reply_id = resp.reply_id;
            let commkey = make_commkey(cfg.device_password as u32, resp.session_id);
            let auth = client.send_command(CMD_AUTH, &commkey)?;
            if auth.command != CMD_ACK_OK {
                return Err(DeviceError::Message(format!(
                    "authentication failed (cmd {})",
                    auth.command
                )));
            }
            client.session_id = auth.session_id;
            client.reply_id = auth.reply_id;
        } else if resp.command == CMD_ACK_OK {
            client.session_id = resp.session_id;
            client.reply_id = resp.reply_id;
        } else {
            return Err(DeviceError::Message(format!(
                "connect returned command {}",
                resp.command
            )));
        }

        Ok(Box::new(client))
    }
}

// ── Client ───────────────────────────────────────────────────────────────────

pub struct ZktecoTcpClient {
    stream: TcpStream,
    session_id: u16,
    reply_id: u16,
}

impl DeviceClient for ZktecoTcpClient {
    fn pull_attendance(&mut self) -> Result<Vec<RawAttendance>, DeviceError> {
        let record_count = self.get_record_count().unwrap_or(0);
        if record_count == 0 {
            // Try anyway in case get_record_count failed (unsupported device).
        }

        let raw = self.read_with_buffer(CMD_ATTLOG_RRQ, 0, 0)?;
        decode_attendance_data(&raw, record_count)
    }

    fn clear_attendance(&mut self) -> Result<(), DeviceError> {
        let resp = self.send_command(CMD_CLEAR_ATTLOG, &[])?;
        if resp.command == CMD_ACK_OK {
            Ok(())
        } else {
            Err(DeviceError::Message(format!(
                "clear attendance returned command {}",
                resp.command
            )))
        }
    }

    fn get_templates(&mut self) -> Result<Vec<FingerTemplate>, DeviceError> {
        Err(DeviceError::Message(
            "template retrieval is not yet implemented".to_string(),
        ))
    }

    fn push_user_template(
        &mut self,
        _user: &DeviceUser,
        _finger: &FingerTemplate,
    ) -> Result<(), DeviceError> {
        Err(DeviceError::Message(
            "push user/template is not yet implemented".to_string(),
        ))
    }

    fn disconnect(&mut self) {
        let _ = self.send_command(CMD_EXIT, &[]);
    }
}

impl ZktecoTcpClient {
    /// How many attendance records the device currently holds.
    /// Uses CMD_GET_FREE_SIZES; returns 0 on any failure.
    fn get_record_count(&mut self) -> Result<usize, DeviceError> {
        let resp = self.send_command(CMD_GET_FREE_SIZES, &[])?;
        if resp.command != CMD_ACK_OK {
            return Ok(0);
        }
        // Response: 20 × i32 LE. Field [8] is the attendance record count.
        if resp.data.len() < 80 {
            return Ok(0);
        }
        let count =
            i32::from_le_bytes([resp.data[32], resp.data[33], resp.data[34], resp.data[35]]);
        Ok(count.max(0) as usize)
    }

    /// pyzk's read_with_buffer: sends CMD_PREPARE_BUFFER and collects the
    /// payload whether it arrives inline (CMD_DATA) or in chunks
    /// (CMD_PREPARE_DATA + CMD_READ_BUFFER).
    fn read_with_buffer(
        &mut self,
        sub_cmd: u8,
        fct: i32,
        ext: i32,
    ) -> Result<Vec<u8>, DeviceError> {
        // pack('<bhii', 1, sub_cmd, fct, ext)
        let mut cmd_data = vec![1u8];
        cmd_data.extend_from_slice(&(sub_cmd as i16).to_le_bytes());
        cmd_data.extend_from_slice(&fct.to_le_bytes());
        cmd_data.extend_from_slice(&ext.to_le_bytes());

        let resp = self.send_command(CMD_PREPARE_BUFFER, &cmd_data)?;

        match resp.command {
            CMD_DATA => Ok(resp.data),

            CMD_PREPARE_DATA => {
                // Bytes [1..5] hold the total payload size (pyzk read_with_buffer).
                if resp.data.len() < 5 {
                    return Err(DeviceError::Message(
                        "CMD_PREPARE_DATA: response too short".to_string(),
                    ));
                }
                let size =
                    u32::from_le_bytes([resp.data[1], resp.data[2], resp.data[3], resp.data[4]])
                        as usize;

                let mut all_data = Vec::with_capacity(size);
                let remain = size % MAX_CHUNK;
                let full_chunks = (size - remain) / MAX_CHUNK;
                let mut start = 0usize;

                for _ in 0..full_chunks {
                    let chunk = self.read_chunk(start, MAX_CHUNK)?;
                    all_data.extend_from_slice(&chunk);
                    start += MAX_CHUNK;
                }
                if remain > 0 {
                    let chunk = self.read_chunk(start, remain)?;
                    all_data.extend_from_slice(&chunk);
                }

                let _ = self.send_command(CMD_FREE_DATA, &[]);
                Ok(all_data)
            }

            other => Err(DeviceError::Message(format!(
                "read_with_buffer: unexpected response command {other}"
            ))),
        }
    }

    /// Read one chunk from the device's buffer (CMD_READ_BUFFER).
    fn read_chunk(&mut self, start: usize, size: usize) -> Result<Vec<u8>, DeviceError> {
        let mut cmd_data = Vec::with_capacity(8);
        cmd_data.extend_from_slice(&(start as i32).to_le_bytes());
        cmd_data.extend_from_slice(&(size as i32).to_le_bytes());

        let resp = self.send_command(CMD_READ_BUFFER, &cmd_data)?;
        if resp.command != CMD_DATA {
            return Err(DeviceError::Message(format!(
                "read_chunk: expected CMD_DATA, got {}",
                resp.command
            )));
        }
        Ok(resp.data)
    }

    fn send_command(&mut self, command: u16, data: &[u8]) -> Result<ResponsePacket, DeviceError> {
        self.reply_id = self.reply_id.wrapping_add(1);
        if self.reply_id == USHRT_MAX {
            self.reply_id = 0;
        }
        let packet = make_tcp_packet(command, data, self.session_id, self.reply_id);
        self.stream
            .write_all(&packet)
            .map_err(|e| DeviceError::Message(format!("device write failed: {e}")))?;
        let resp = read_tcp_packet(&mut self.stream)?;
        self.session_id = resp.session_id;
        self.reply_id = resp.reply_id;
        Ok(resp)
    }
}

// ── Packet encoding / decoding ────────────────────────────────────────────────

#[derive(Debug)]
struct ResponsePacket {
    command: u16,
    session_id: u16,
    reply_id: u16,
    data: Vec<u8>,
}

fn make_tcp_packet(command: u16, data: &[u8], session_id: u16, reply_id: u16) -> Vec<u8> {
    let header_template: Vec<u8> = [
        command.to_le_bytes().as_slice(),
        &0u16.to_le_bytes(),
        &session_id.to_le_bytes(),
        &reply_id.to_le_bytes(),
    ]
    .concat();
    let checksum = calc_checksum(&[header_template.as_slice(), data].concat());

    let mut packet = Vec::with_capacity(16 + data.len());
    packet.extend_from_slice(&MACHINE_PREPARE_DATA_1.to_le_bytes());
    packet.extend_from_slice(&MACHINE_PREPARE_DATA_2.to_le_bytes());
    packet.extend_from_slice(&((8 + data.len()) as u32).to_le_bytes());
    packet.extend_from_slice(&command.to_le_bytes());
    packet.extend_from_slice(&checksum.to_le_bytes());
    packet.extend_from_slice(&session_id.to_le_bytes());
    packet.extend_from_slice(&reply_id.to_le_bytes());
    packet.extend_from_slice(data);
    packet
}

fn read_tcp_packet(stream: &mut TcpStream) -> Result<ResponsePacket, DeviceError> {
    let mut hdr = [0u8; 8];
    stream
        .read_exact(&mut hdr)
        .map_err(|e| DeviceError::Message(format!("device read header failed: {e}")))?;

    let p1 = u16::from_le_bytes([hdr[0], hdr[1]]);
    let p2 = u16::from_le_bytes([hdr[2], hdr[3]]);
    if p1 != MACHINE_PREPARE_DATA_1 || p2 != MACHINE_PREPARE_DATA_2 {
        return Err(DeviceError::Message(
            "invalid ZKTeco TCP header".to_string(),
        ));
    }

    let pkt_len = u32::from_le_bytes([hdr[4], hdr[5], hdr[6], hdr[7]]) as usize;
    if pkt_len < 8 {
        return Err(DeviceError::Message(format!(
            "ZKTeco packet too short: {pkt_len}"
        )));
    }

    let mut pkt = vec![0u8; pkt_len];
    stream
        .read_exact(&mut pkt)
        .map_err(|e| DeviceError::Message(format!("device read packet failed: {e}")))?;

    Ok(ResponsePacket {
        command: u16::from_le_bytes([pkt[0], pkt[1]]),
        session_id: u16::from_le_bytes([pkt[4], pkt[5]]),
        reply_id: u16::from_le_bytes([pkt[6], pkt[7]]),
        data: pkt[8..].to_vec(),
    })
}

fn calc_checksum(data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    for chunk in data.chunks(2) {
        let value = if chunk.len() == 2 {
            u16::from_le_bytes([chunk[0], chunk[1]]) as u32
        } else {
            chunk[0] as u32
        };
        sum += value;
        while sum > USHRT_MAX as u32 {
            sum -= USHRT_MAX as u32;
        }
    }
    !(sum as u16)
}

// ── make_commkey ─────────────────────────────────────────────────────────────

/// Derives the 4-byte commkey used in CMD_AUTH. Ported from pyzk's MakeKey.
fn make_commkey(key: u32, session_id: u16) -> [u8; 4] {
    const TICKS: u8 = 50;

    // Reverse the bits of key.
    let mut k: u32 = 0;
    for i in 0..32u32 {
        k = if (key & (1 << i)) != 0 {
            (k << 1) | 1
        } else {
            k << 1
        };
    }
    k = k.wrapping_add(session_id as u32);

    // XOR each byte with b'Z', b'K', b'S', b'O'.
    let b = k.to_le_bytes();
    let xored = [b[0] ^ b'Z', b[1] ^ b'K', b[2] ^ b'S', b[3] ^ b'O'];

    // Interpret as two LE u16s, then swap them.
    let h0 = u16::from_le_bytes([xored[0], xored[1]]);
    let h1 = u16::from_le_bytes([xored[2], xored[3]]);
    let swapped = [
        h1.to_le_bytes()[0],
        h1.to_le_bytes()[1],
        h0.to_le_bytes()[0],
        h0.to_le_bytes()[1],
    ];

    // XOR bytes 0, 1, 3 with TICKS; byte 2 is set to TICKS directly.
    let t = TICKS;
    [swapped[0] ^ t, swapped[1] ^ t, t, swapped[3] ^ t]
}

// ── Attendance decoding ───────────────────────────────────────────────────────

/// Decode the raw payload returned by read_with_buffer for CMD_ATTLOG_RRQ.
///
/// The payload layout (matching pyzk's get_attendance):
///   bytes 0..4  – total_size: number of record bytes that follow
///   bytes 4..   – records
///
/// record_count is from CMD_GET_FREE_SIZES; 0 means unknown → detect from size.
fn decode_attendance_data(
    payload: &[u8],
    record_count: usize,
) -> Result<Vec<RawAttendance>, DeviceError> {
    if payload.len() < 4 {
        return Ok(vec![]);
    }
    let total_size = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]) as usize;
    if total_size == 0 {
        return Ok(vec![]);
    }

    let records_end = (4 + total_size).min(payload.len());
    let records_data = &payload[4..records_end];
    if records_data.is_empty() {
        return Ok(vec![]);
    }

    let record_size = if record_count > 0 {
        let s = total_size.checked_div(record_count).unwrap_or(0);
        if s == 8 || s == 16 || s == 40 {
            s
        } else {
            detect_record_size(total_size, records_data.len())
        }
    } else {
        detect_record_size(total_size, records_data.len())
    };

    match record_size {
        8 => decode_8byte_records(records_data),
        16 => decode_16byte_records(records_data),
        _ => decode_40byte_records(records_data),
    }
}

/// Choose record size heuristically when the device count is not available.
fn detect_record_size(total_size: usize, data_len: usize) -> usize {
    // Prefer 8-byte (oldest/simplest); only pick 16 or 40 when 8 does not fit.
    let sizes = [8usize, 16, 40];
    for &s in &sizes {
        if total_size.is_multiple_of(s) && data_len >= s {
            return s;
        }
    }
    40 // fallback
}

/// 8-byte record: uid(u16), status(u8), timestamp(4B LE encoded), punch(u8)
fn decode_8byte_records(data: &[u8]) -> Result<Vec<RawAttendance>, DeviceError> {
    let mut out = Vec::new();
    for rec in data.chunks_exact(8) {
        let uid = u16::from_le_bytes([rec[0], rec[1]]);
        let encoded_ts = u32::from_le_bytes([rec[3], rec[4], rec[5], rec[6]]);
        let punch = rec[7] as i64;
        out.push(RawAttendance {
            user_id: uid.to_string(),
            timestamp: decode_timestamp(encoded_ts),
            punch,
        });
    }
    Ok(out)
}

/// 16-byte record: user_id(u32 LE), timestamp(4B), status(u8), punch(u8), …
fn decode_16byte_records(data: &[u8]) -> Result<Vec<RawAttendance>, DeviceError> {
    let mut out = Vec::new();
    for rec in data.chunks_exact(16) {
        let user_id = u32::from_le_bytes([rec[0], rec[1], rec[2], rec[3]]);
        let encoded_ts = u32::from_le_bytes([rec[4], rec[5], rec[6], rec[7]]);
        let punch = rec[9] as i64;
        out.push(RawAttendance {
            user_id: user_id.to_string(),
            timestamp: decode_timestamp(encoded_ts),
            punch,
        });
    }
    Ok(out)
}

/// 40-byte record: uid(u16), user_id(24B null-term str), status(u8),
///                 timestamp(4B), punch(u8), padding(8B)
fn decode_40byte_records(data: &[u8]) -> Result<Vec<RawAttendance>, DeviceError> {
    let mut out = Vec::new();
    for rec in data.chunks(40) {
        if rec.len() < 40 {
            break;
        }
        // Skip sentinel records (pyzk: b'\xff255\x00...')
        if rec[0] == 0xff {
            continue;
        }
        let user_id_raw = &rec[2..26]; // 24 bytes
        let user_id = user_id_raw
            .iter()
            .copied()
            .take_while(|&b| b != 0)
            .map(|b| b as char)
            .collect::<String>();

        let encoded_ts = u32::from_le_bytes([rec[27], rec[28], rec[29], rec[30]]);
        let punch = rec[31] as i64;

        if user_id.is_empty() {
            continue;
        }
        out.push(RawAttendance {
            user_id,
            timestamp: decode_timestamp(encoded_ts),
            punch,
        });
    }
    Ok(out)
}

// ── Timestamp ────────────────────────────────────────────────────────────────

/// Decode ZKTeco's packed timestamp (seconds since 2000-01-01 00:00:00 UTC).
/// Formula from pyzk DecodeTime / zkemsdk.c.
fn decode_timestamp(value: u32) -> String {
    let second = value % 60;
    let t = value / 60;
    let minute = t % 60;
    let t = t / 60;
    let hour = t % 24;
    let t = t / 24;
    let day = (t % 31) + 1;
    let t = t / 31;
    let month = (t % 12) + 1;
    let year = (t / 12) + 2000;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}+00:00")
}
