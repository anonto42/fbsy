//! ZKTeco adapter — TCP and UDP transport, pyzk-compatible.
//!
//! Protocol flow (ported from pyzk/zk/base.py):
//!
//! 1. Connect: CMD_CONNECT. If CMD_ACK_UNAUTH, send CMD_AUTH with commkey.
//! 2. Pull attendance via CMD_PREPARE_BUFFER (1503):
//!    - CMD_DATA response: payload is inline.
//!    - CMD_PREPARE_DATA response: read chunks via CMD_READ_BUFFER, free with CMD_FREE_DATA.
//! 3. Three wire record formats (8, 16, or 40 bytes) depending on device firmware.
//! 4. Clear: CMD_CLEAR_ATTLOG.
//!
//! Transport differences (TCP vs UDP):
//!   TCP — packet = [TCP_HDR(8)] + [ZK_HEADER(8)] + data
//!         MAX_CHUNK = 65 472 bytes; chunks arrive via read_exact
//!   UDP — packet = [ZK_HEADER(8)] + data   (no TCP framing)
//!         MAX_CHUNK = 16 384 bytes; chunks arrive as multiple datagrams until CMD_ACK_OK

use std::{
    io::{Read, Write},
    net::{TcpStream, UdpSocket},
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

const MAX_CHUNK_TCP: usize = 0xff_c0; // 65 472 — pyzk TCP value
const MAX_CHUNK_UDP: usize = 16 * 1024; // 16 384 — pyzk UDP value
const UDP_BUF: usize = 65_536; // max UDP datagram

// ── Connector ────────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct ZktecoTcpConnector;

impl DeviceConnector for ZktecoTcpConnector {
    fn connect(&self, cfg: &BridgeDeviceConfig) -> Result<Box<dyn DeviceClient>, DeviceError> {
        if cfg.device_force_udp {
            connect_udp(cfg)
        } else {
            connect_tcp(cfg)
        }
    }
}

fn connect_tcp(cfg: &BridgeDeviceConfig) -> Result<Box<dyn DeviceClient>, DeviceError> {
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

    let mut client = ZktecoClient {
        transport: Transport::Tcp(stream),
        session_id: 0,
        reply_id: USHRT_MAX - 1,
    };
    authenticate(&mut client, cfg)?;
    Ok(Box::new(client))
}

fn connect_udp(cfg: &BridgeDeviceConfig) -> Result<Box<dyn DeviceClient>, DeviceError> {
    let timeout = Duration::from_secs(cfg.device_timeout);
    let socket =
        UdpSocket::bind("0.0.0.0:0").map_err(|e| DeviceError::Message(format!("udp bind: {e}")))?;
    socket
        .connect(format!("{}:{}", cfg.device_ip, cfg.device_port))
        .map_err(|e| DeviceError::Message(format!("udp connect: {e}")))?;
    socket
        .set_read_timeout(Some(timeout))
        .map_err(|e| DeviceError::Message(format!("set udp timeout: {e}")))?;

    let mut client = ZktecoClient {
        transport: Transport::Udp(socket),
        session_id: 0,
        reply_id: USHRT_MAX - 1,
    };
    authenticate(&mut client, cfg)?;
    Ok(Box::new(client))
}

/// Shared connect + optional password auth for both transports.
fn authenticate(client: &mut ZktecoClient, cfg: &BridgeDeviceConfig) -> Result<(), DeviceError> {
    let resp = client.send_command(CMD_CONNECT, &[])?;
    if resp.command == CMD_ACK_UNAUTH {
        let commkey = make_commkey(cfg.device_password as u32, resp.session_id);
        let auth = client.send_command(CMD_AUTH, &commkey)?;
        if auth.command != CMD_ACK_OK {
            return Err(DeviceError::Message(format!(
                "authentication failed (cmd {})",
                auth.command
            )));
        }
    } else if resp.command != CMD_ACK_OK {
        return Err(DeviceError::Message(format!(
            "connect returned command {}",
            resp.command
        )));
    }
    Ok(())
}

// ── Transport ─────────────────────────────────────────────────────────────────

enum Transport {
    Tcp(TcpStream),
    Udp(UdpSocket),
}

// ── Client ────────────────────────────────────────────────────────────────────

pub struct ZktecoClient {
    transport: Transport,
    session_id: u16,
    reply_id: u16,
}

impl DeviceClient for ZktecoClient {
    fn pull_attendance(&mut self) -> Result<Vec<RawAttendance>, DeviceError> {
        let record_count = self.get_record_count().unwrap_or(0);
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

impl ZktecoClient {
    fn get_record_count(&mut self) -> Result<usize, DeviceError> {
        let resp = self.send_command(CMD_GET_FREE_SIZES, &[])?;
        if resp.command != CMD_ACK_OK || resp.data.len() < 80 {
            return Ok(0);
        }
        // 20 × i32 LE; field [8] = attendance record count.
        let count =
            i32::from_le_bytes([resp.data[32], resp.data[33], resp.data[34], resp.data[35]]);
        Ok(count.max(0) as usize)
    }

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
                if resp.data.len() < 5 {
                    return Err(DeviceError::Message(
                        "CMD_PREPARE_DATA: response too short".to_string(),
                    ));
                }
                // pyzk read_with_buffer uses data[1:5] for size.
                let size =
                    u32::from_le_bytes([resp.data[1], resp.data[2], resp.data[3], resp.data[4]])
                        as usize;

                let max_chunk = self.max_chunk();
                let mut all_data = Vec::with_capacity(size);
                let remain = size % max_chunk;
                let full_chunks = (size - remain) / max_chunk;
                let mut start = 0usize;

                for _ in 0..full_chunks {
                    let chunk = self.read_chunk(start, max_chunk)?;
                    all_data.extend_from_slice(&chunk);
                    start += max_chunk;
                }
                if remain > 0 {
                    let chunk = self.read_chunk(start, remain)?;
                    all_data.extend_from_slice(&chunk);
                }

                let _ = self.send_command(CMD_FREE_DATA, &[]);
                Ok(all_data)
            }

            other => Err(DeviceError::Message(format!(
                "read_with_buffer: unexpected response {other}"
            ))),
        }
    }

    fn read_chunk(&mut self, start: usize, size: usize) -> Result<Vec<u8>, DeviceError> {
        let mut cmd_data = Vec::with_capacity(8);
        cmd_data.extend_from_slice(&(start as i32).to_le_bytes());
        cmd_data.extend_from_slice(&(size as i32).to_le_bytes());

        let resp = self.send_command(CMD_READ_BUFFER, &cmd_data)?;
        match resp.command {
            CMD_DATA => Ok(resp.data),
            // UDP: device sends CMD_PREPARE_DATA then streams datagrams until CMD_ACK_OK.
            CMD_PREPARE_DATA => self.recv_udp_stream(size),
            _ => Err(DeviceError::Message(format!(
                "read_chunk: expected CMD_DATA, got {}",
                resp.command
            ))),
        }
    }

    /// Receive multiple UDP datagrams until CMD_ACK_OK (pyzk UDP __recieve_chunk).
    fn recv_udp_stream(&mut self, expected: usize) -> Result<Vec<u8>, DeviceError> {
        let Transport::Udp(ref socket) = self.transport else {
            return Err(DeviceError::Message(
                "recv_udp_stream called on TCP transport".to_string(),
            ));
        };
        let mut data: Vec<u8> = Vec::with_capacity(expected);
        let mut buf = vec![0u8; 1024 + 8];
        loop {
            let n = socket
                .recv(&mut buf)
                .map_err(|e| DeviceError::Message(format!("udp recv stream: {e}")))?;
            if n < 8 {
                break;
            }
            let cmd = u16::from_le_bytes([buf[0], buf[1]]);
            match cmd {
                CMD_DATA => data.extend_from_slice(&buf[8..n]),
                CMD_ACK_OK => break,
                _ => break,
            }
        }
        Ok(data)
    }

    fn max_chunk(&self) -> usize {
        match self.transport {
            Transport::Tcp(_) => MAX_CHUNK_TCP,
            Transport::Udp(_) => MAX_CHUNK_UDP,
        }
    }

    fn send_command(&mut self, command: u16, data: &[u8]) -> Result<ResponsePacket, DeviceError> {
        self.reply_id = self.reply_id.wrapping_add(1);
        if self.reply_id == USHRT_MAX {
            self.reply_id = 0;
        }
        match &mut self.transport {
            Transport::Tcp(stream) => {
                let packet = make_tcp_packet(command, data, self.session_id, self.reply_id);
                stream
                    .write_all(&packet)
                    .map_err(|e| DeviceError::Message(format!("tcp write: {e}")))?;
                let resp = read_tcp_packet(stream)?;
                self.session_id = resp.session_id;
                self.reply_id = resp.reply_id;
                Ok(resp)
            }
            Transport::Udp(socket) => {
                let packet = make_raw_packet(command, data, self.session_id, self.reply_id);
                socket
                    .send(&packet)
                    .map_err(|e| DeviceError::Message(format!("udp send: {e}")))?;
                let mut buf = vec![0u8; UDP_BUF];
                let n = socket
                    .recv(&mut buf)
                    .map_err(|e| DeviceError::Message(format!("udp recv: {e}")))?;
                let resp = parse_raw_packet(&buf[..n])?;
                self.session_id = resp.session_id;
                self.reply_id = resp.reply_id;
                Ok(resp)
            }
        }
    }
}

// ── Packet helpers ────────────────────────────────────────────────────────────

#[derive(Debug)]
struct ResponsePacket {
    command: u16,
    session_id: u16,
    reply_id: u16,
    data: Vec<u8>,
}

/// TCP packet: [MACHINE_PREPARE_DATA_1(2), MACHINE_PREPARE_DATA_2(2), length(4),
///              command(2), checksum(2), session_id(2), reply_id(2), data...]
fn make_tcp_packet(command: u16, data: &[u8], session_id: u16, reply_id: u16) -> Vec<u8> {
    let zk = make_zk_header(command, data, session_id, reply_id);
    let mut pkt = Vec::with_capacity(8 + zk.len());
    pkt.extend_from_slice(&MACHINE_PREPARE_DATA_1.to_le_bytes());
    pkt.extend_from_slice(&MACHINE_PREPARE_DATA_2.to_le_bytes());
    pkt.extend_from_slice(&(zk.len() as u32).to_le_bytes());
    pkt.extend_from_slice(&zk);
    pkt
}

/// Raw/UDP packet: [command(2), checksum(2), session_id(2), reply_id(2), data...]
fn make_raw_packet(command: u16, data: &[u8], session_id: u16, reply_id: u16) -> Vec<u8> {
    make_zk_header(command, data, session_id, reply_id)
}

fn make_zk_header(command: u16, data: &[u8], session_id: u16, reply_id: u16) -> Vec<u8> {
    let template: Vec<u8> = [
        command.to_le_bytes().as_slice(),
        0u16.to_le_bytes().as_slice(),
        session_id.to_le_bytes().as_slice(),
        reply_id.to_le_bytes().as_slice(),
    ]
    .concat();
    let checksum = calc_checksum(&[template.as_slice(), data].concat());
    let mut hdr = Vec::with_capacity(8 + data.len());
    hdr.extend_from_slice(&command.to_le_bytes());
    hdr.extend_from_slice(&checksum.to_le_bytes());
    hdr.extend_from_slice(&session_id.to_le_bytes());
    hdr.extend_from_slice(&reply_id.to_le_bytes());
    hdr.extend_from_slice(data);
    hdr
}

fn read_tcp_packet(stream: &mut TcpStream) -> Result<ResponsePacket, DeviceError> {
    let mut hdr = [0u8; 8];
    stream
        .read_exact(&mut hdr)
        .map_err(|e| DeviceError::Message(format!("tcp read header: {e}")))?;

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
        .map_err(|e| DeviceError::Message(format!("tcp read packet: {e}")))?;
    parse_raw_packet(&pkt)
}

fn parse_raw_packet(pkt: &[u8]) -> Result<ResponsePacket, DeviceError> {
    if pkt.len() < 8 {
        return Err(DeviceError::Message(format!(
            "ZKTeco packet too short: {}",
            pkt.len()
        )));
    }
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

/// Derives the 4-byte commkey used in CMD_AUTH. Ported from pyzk MakeKey.
fn make_commkey(key: u32, session_id: u16) -> [u8; 4] {
    const TICKS: u8 = 50;
    let mut k: u32 = 0;
    for i in 0..32u32 {
        k = if (key & (1 << i)) != 0 {
            (k << 1) | 1
        } else {
            k << 1
        };
    }
    k = k.wrapping_add(session_id as u32);
    let b = k.to_le_bytes();
    let xored = [b[0] ^ b'Z', b[1] ^ b'K', b[2] ^ b'S', b[3] ^ b'O'];
    let h0 = u16::from_le_bytes([xored[0], xored[1]]);
    let h1 = u16::from_le_bytes([xored[2], xored[3]]);
    let swapped = [
        h1.to_le_bytes()[0],
        h1.to_le_bytes()[1],
        h0.to_le_bytes()[0],
        h0.to_le_bytes()[1],
    ];
    let t = TICKS;
    [swapped[0] ^ t, swapped[1] ^ t, t, swapped[3] ^ t]
}

// ── Attendance decoding ───────────────────────────────────────────────────────

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

fn detect_record_size(total_size: usize, data_len: usize) -> usize {
    for &s in &[8usize, 16, 40] {
        if total_size.is_multiple_of(s) && data_len >= s {
            return s;
        }
    }
    40
}

fn decode_8byte_records(data: &[u8]) -> Result<Vec<RawAttendance>, DeviceError> {
    let mut out = Vec::new();
    for rec in data.chunks_exact(8) {
        let uid = u16::from_le_bytes([rec[0], rec[1]]);
        let encoded_ts = u32::from_le_bytes([rec[3], rec[4], rec[5], rec[6]]);
        out.push(RawAttendance {
            user_id: uid.to_string(),
            timestamp: decode_timestamp(encoded_ts),
            punch: rec[7] as i64,
        });
    }
    Ok(out)
}

fn decode_16byte_records(data: &[u8]) -> Result<Vec<RawAttendance>, DeviceError> {
    let mut out = Vec::new();
    for rec in data.chunks_exact(16) {
        let user_id = u32::from_le_bytes([rec[0], rec[1], rec[2], rec[3]]);
        let encoded_ts = u32::from_le_bytes([rec[4], rec[5], rec[6], rec[7]]);
        out.push(RawAttendance {
            user_id: user_id.to_string(),
            timestamp: decode_timestamp(encoded_ts),
            punch: rec[9] as i64,
        });
    }
    Ok(out)
}

fn decode_40byte_records(data: &[u8]) -> Result<Vec<RawAttendance>, DeviceError> {
    let mut out = Vec::new();
    for rec in data.chunks(40) {
        if rec.len() < 40 || rec[0] == 0xff {
            continue;
        }
        let user_id: String = rec[2..26]
            .iter()
            .copied()
            .take_while(|&b| b != 0)
            .map(|b| b as char)
            .collect();
        if user_id.is_empty() {
            continue;
        }
        let encoded_ts = u32::from_le_bytes([rec[27], rec[28], rec[29], rec[30]]);
        out.push(RawAttendance {
            user_id,
            timestamp: decode_timestamp(encoded_ts),
            punch: rec[31] as i64,
        });
    }
    Ok(out)
}

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
