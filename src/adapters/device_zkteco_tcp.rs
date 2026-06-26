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
const CMD_DB_RRQ: u8 = 7; // read templates/users buffer
const CMD_USERTEMP_RRQ: u8 = 9; // read users buffer
const CMD_ATTLOG_RRQ: u8 = 13;
const CMD_CLEAR_ATTLOG: u16 = 15;
const CMD_SAVE_USERTEMPS: u16 = 110; // (undocumented) save user + templates
const CMD_CONNECT: u16 = 1000;
const CMD_EXIT: u16 = 1001;
const CMD_REFRESHDATA: u16 = 1013;
const CMD_AUTH: u16 = 1102;
const CMD_PREPARE_DATA: u16 = 1500;
const CMD_DATA: u16 = 1501;
const CMD_FREE_DATA: u16 = 1502;
const CMD_PREPARE_BUFFER: u16 = 1503;
const CMD_READ_BUFFER: u16 = 1504;
const CMD_ACK_OK: u16 = 2000;
const CMD_ACK_UNAUTH: u16 = 2005;

const FCT_FINGERTMP: i32 = 2; // template function code
const FCT_USER: i32 = 5; // user function code

const MAX_CHUNK_TCP: usize = 0xff_c0; // 65 472 — pyzk TCP value
const MAX_CHUNK_UDP: usize = 16 * 1024; // 16 384 — pyzk UDP value
const MAX_CHUNK_SEND: usize = 1024; // pyzk _send_with_buffer chunk
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
        let (users_count, fingers_count, _) = self.read_sizes();
        if fingers_count == 0 {
            return Ok(Vec::new());
        }
        // Map uid -> (user_id, name) so templates can be joined with their user.
        let user_map: std::collections::HashMap<u32, (String, String)> = self
            .get_users(users_count)?
            .into_iter()
            .map(|u| (u.uid, (u.user_id, u.name)))
            .collect();

        let raw = self.read_with_buffer(CMD_DB_RRQ, FCT_FINGERTMP, 0)?;
        decode_templates(&raw, &user_map)
    }

    fn push_user_template(
        &mut self,
        user: &DeviceUser,
        finger: &FingerTemplate,
    ) -> Result<(), DeviceError> {
        // Build the HR_save_usertemplates buffer: head + user + table + template.
        // TCP devices use the 73-byte user packet (pyzk zk8 default).
        let upack = repack_user_73(user);
        let fpack = repack_finger_only(&finger.template);
        let table = repack_table_entry(user.uid, finger.fid, 0);

        let mut head = Vec::with_capacity(12);
        head.extend_from_slice(&(upack.len() as u32).to_le_bytes());
        head.extend_from_slice(&(table.len() as u32).to_le_bytes());
        head.extend_from_slice(&(fpack.len() as u32).to_le_bytes());

        let mut buffer = head;
        buffer.extend_from_slice(&upack);
        buffer.extend_from_slice(&table);
        buffer.extend_from_slice(&fpack);

        self.send_with_buffer(&buffer)?;

        // CMD_SAVE_USERTEMPS with command_string pack('<IHH', 12, 0, 8).
        let mut cmd_string = Vec::with_capacity(8);
        cmd_string.extend_from_slice(&12u32.to_le_bytes());
        cmd_string.extend_from_slice(&0u16.to_le_bytes());
        cmd_string.extend_from_slice(&8u16.to_le_bytes());
        let resp = self.send_command(CMD_SAVE_USERTEMPS, &cmd_string)?;
        if resp.command != CMD_ACK_OK {
            return Err(DeviceError::Message(format!(
                "save user/template returned command {}",
                resp.command
            )));
        }

        // Tell the device to refresh its internal data.
        let _ = self.send_command(CMD_REFRESHDATA, &[]);
        Ok(())
    }

    fn disconnect(&mut self) {
        let _ = self.send_command(CMD_EXIT, &[]);
    }
}

impl ZktecoClient {
    fn get_record_count(&mut self) -> Result<usize, DeviceError> {
        let (_, _, records) = self.read_sizes();
        Ok(records)
    }

    /// Returns (users, fingers, records) counts from CMD_GET_FREE_SIZES.
    /// The response is 20 × i32 LE: field[4]=users, field[6]=fingers,
    /// field[8]=records (pyzk read_sizes).
    fn read_sizes(&mut self) -> (usize, usize, usize) {
        let resp = match self.send_command(CMD_GET_FREE_SIZES, &[]) {
            Ok(r) => r,
            Err(_) => return (0, 0, 0),
        };
        if resp.command != CMD_ACK_OK || resp.data.len() < 80 {
            return (0, 0, 0);
        }
        let field = |i: usize| -> usize {
            let o = i * 4;
            i32::from_le_bytes([
                resp.data[o],
                resp.data[o + 1],
                resp.data[o + 2],
                resp.data[o + 3],
            ])
            .max(0) as usize
        };
        (field(4), field(6), field(8))
    }

    /// Read the user table and decode it into uid/user_id/name records.
    fn get_users(&mut self, users_count: usize) -> Result<Vec<DeviceUser>, DeviceError> {
        if users_count == 0 {
            return Ok(Vec::new());
        }
        let data = self.read_with_buffer(CMD_USERTEMP_RRQ, FCT_USER, 0)?;
        decode_users(&data, users_count)
    }

    /// Send a large buffer to the device (pyzk _send_with_buffer): free the
    /// device buffer, declare the size, then stream it in 1 KB chunks.
    fn send_with_buffer(&mut self, buffer: &[u8]) -> Result<(), DeviceError> {
        let _ = self.send_command(CMD_FREE_DATA, &[]);

        let size = buffer.len();
        let resp = self.send_command(CMD_PREPARE_DATA, &(size as u32).to_le_bytes())?;
        if resp.command != CMD_ACK_OK && resp.command != CMD_PREPARE_DATA {
            return Err(DeviceError::Message(format!(
                "prepare data returned command {}",
                resp.command
            )));
        }

        let mut start = 0;
        while start < size {
            let end = (start + MAX_CHUNK_SEND).min(size);
            let chunk = self.send_command(CMD_DATA, &buffer[start..end])?;
            if chunk.command != CMD_ACK_OK {
                return Err(DeviceError::Message(format!(
                    "data chunk returned command {}",
                    chunk.command
                )));
            }
            start = end;
        }
        Ok(())
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

// ── User & template decoding (pyzk get_users / get_templates) ──────────────────

/// Decode the user table. Layout: 4-byte total size header, then fixed-size user
/// records (28 bytes for zk6, 72 bytes for zk8) chosen by total_size / count.
fn decode_users(payload: &[u8], users_count: usize) -> Result<Vec<DeviceUser>, DeviceError> {
    if payload.len() < 4 {
        return Ok(Vec::new());
    }
    let total = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]) as usize;
    let body = &payload[4..];
    if total == 0 || body.is_empty() {
        return Ok(Vec::new());
    }
    let packet_size = total.checked_div(users_count).unwrap_or(72);
    let size = if packet_size == 28 { 28 } else { 72 };

    let mut out = Vec::new();
    for rec in body.chunks(size) {
        if rec.len() < size {
            break;
        }
        let uid = u16::from_le_bytes([rec[0], rec[1]]) as u32;
        let (name, user_id) = if size == 28 {
            // <HB5s8sIxBhI: name = [8..16], user_id = u32 at [24..28]
            let name = cstr(&rec[8..16]);
            let user_id = u32::from_le_bytes([rec[24], rec[25], rec[26], rec[27]]).to_string();
            (name, user_id)
        } else {
            // <HB8s24sIx7sx24s: name = [11..35], user_id = [48..72]
            (cstr(&rec[11..35]), cstr(&rec[48..72]))
        };
        let user_id = if user_id.is_empty() {
            uid.to_string()
        } else {
            user_id
        };
        out.push(DeviceUser { uid, user_id, name });
    }
    Ok(out)
}

/// Decode the template table. Layout: 4-byte total size header, then variable
/// records: size(u16) uid(u16) fid(i8) valid(i8) template[size-6].
fn decode_templates(
    payload: &[u8],
    users: &std::collections::HashMap<u32, (String, String)>,
) -> Result<Vec<FingerTemplate>, DeviceError> {
    if payload.len() < 4 {
        return Ok(Vec::new());
    }
    let mut total = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]) as usize;
    let mut data = &payload[4..];

    let mut out = Vec::new();
    while total >= 6 && data.len() >= 6 {
        let size = u16::from_le_bytes([data[0], data[1]]) as usize;
        if size < 6 || size > data.len() {
            break;
        }
        let uid = u16::from_le_bytes([data[2], data[3]]) as u32;
        let fid = data[4];
        let template = data[6..size].to_vec();

        let (user_id, name) = users
            .get(&uid)
            .cloned()
            .unwrap_or_else(|| (uid.to_string(), String::new()));

        out.push(FingerTemplate {
            uid,
            fid,
            user_id,
            name,
            template,
        });

        data = &data[size..];
        total = total.saturating_sub(size);
    }
    Ok(out)
}

// ── User & template encoding (pyzk repack for save_user_template) ──────────────

/// pyzk User.repack73 (zk8, 73 bytes):
/// <BHB8s24sIB7sx24s : 2, uid, privilege=0, password="", name, card=0, 1,
///                     group_id="0", pad, user_id
fn repack_user_73(user: &DeviceUser) -> Vec<u8> {
    let mut b = Vec::with_capacity(73);
    b.push(2);
    b.extend_from_slice(&(user.uid as u16).to_le_bytes());
    b.push(0); // privilege
    b.extend_from_slice(&fixed_bytes("", 8)); // password
    b.extend_from_slice(&fixed_bytes(&user.name, 24));
    b.extend_from_slice(&0u32.to_le_bytes()); // card
    b.push(1);
    b.extend_from_slice(&fixed_bytes("0", 7)); // group_id
    b.push(0); // pad
    b.extend_from_slice(&fixed_bytes(&user.user_id, 24));
    b
}

/// pyzk Finger.repack_only: <H%is : size, template
fn repack_finger_only(template: &[u8]) -> Vec<u8> {
    let mut b = Vec::with_capacity(2 + template.len());
    b.extend_from_slice(&(template.len() as u16).to_le_bytes());
    b.extend_from_slice(template);
    b
}

/// pyzk table entry: <bHbI : 2, uid, 0x10 + fid, tstart
fn repack_table_entry(uid: u32, fid: u8, tstart: u32) -> Vec<u8> {
    let mut b = Vec::with_capacity(8);
    b.push(2);
    b.extend_from_slice(&(uid as u16).to_le_bytes());
    b.push(0x10u8.wrapping_add(fid));
    b.extend_from_slice(&tstart.to_le_bytes());
    b
}

/// Encode a string into a fixed-width, NUL-padded byte field.
fn fixed_bytes(s: &str, width: usize) -> Vec<u8> {
    let mut b = vec![0u8; width];
    let bytes = s.as_bytes();
    let n = bytes.len().min(width);
    b[..n].copy_from_slice(&bytes[..n]);
    b
}

/// Decode a NUL-terminated byte field into a String (lossy).
fn cstr(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).trim().to_string()
}
