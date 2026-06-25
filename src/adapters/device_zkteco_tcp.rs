//! Minimal blocking ZKTeco TCP adapter.
//!
//! This implements the protocol subset used by the Python project's
//! `tests/mock_device_server.py`: connect, pull attendance, clear attendance,
//! and exit. It is enough for local mock-device testing by IP/port.

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

const MACHINE_PREPARE_DATA_1: u16 = 20560;
const MACHINE_PREPARE_DATA_2: u16 = 32130;
const USHRT_MAX: u16 = 65535;

const CMD_CONNECT: u16 = 1000;
const CMD_EXIT: u16 = 1001;
const CMD_ATTLOG_RRQ: u8 = 13;
const CMD_USERTEMP_RRQ: u8 = 9;
const CMD_CLEAR_ATTLOG: u16 = 15;
const CMD_ACK_OK: u16 = 2000;
const CMD_DATA: u16 = 1501;
const CMD_RWB: u16 = 1503;

#[derive(Debug, Default)]
/// Connector for mock/real ZKTeco TCP devices.
pub struct ZktecoTcpConnector;

impl DeviceConnector for ZktecoTcpConnector {
    fn connect(&self, cfg: &BridgeDeviceConfig) -> Result<Box<dyn DeviceClient>, DeviceError> {
        let address = format!("{}:{}", cfg.device_ip, cfg.device_port);
        let timeout = Duration::from_secs(cfg.device_timeout);
        let stream = TcpStream::connect(address)
            .map_err(|err| DeviceError::Message(format!("device connect failed: {err}")))?;
        stream
            .set_read_timeout(Some(timeout))
            .map_err(|err| DeviceError::Message(format!("set read timeout failed: {err}")))?;
        stream
            .set_write_timeout(Some(timeout))
            .map_err(|err| DeviceError::Message(format!("set write timeout failed: {err}")))?;

        let mut client = ZktecoTcpClient {
            stream,
            session_id: 0,
            reply_id: USHRT_MAX - 1,
        };
        let response = client.send_command(CMD_CONNECT, &[])?;
        if response.command != CMD_ACK_OK {
            return Err(DeviceError::Message(format!(
                "device connect returned command {}",
                response.command
            )));
        }
        client.session_id = response.session_id;
        client.reply_id = response.reply_id;
        Ok(Box::new(client))
    }
}

/// Active TCP connection.
pub struct ZktecoTcpClient {
    stream: TcpStream,
    session_id: u16,
    reply_id: u16,
}

impl DeviceClient for ZktecoTcpClient {
    fn pull_attendance(&mut self) -> Result<Vec<RawAttendance>, DeviceError> {
        let response = self.send_command(CMD_RWB, &[0, CMD_ATTLOG_RRQ])?;
        if response.command != CMD_DATA {
            return Err(DeviceError::Message(format!(
                "attendance request returned command {}",
                response.command
            )));
        }
        decode_attendance_payload(&response.data)
    }

    fn clear_attendance(&mut self) -> Result<(), DeviceError> {
        let response = self.send_command(CMD_CLEAR_ATTLOG, &[])?;
        if response.command == CMD_ACK_OK {
            Ok(())
        } else {
            Err(DeviceError::Message(format!(
                "clear attendance returned command {}",
                response.command
            )))
        }
    }

    fn get_templates(&mut self) -> Result<Vec<FingerTemplate>, DeviceError> {
        let response = self.send_command(CMD_RWB, &[0, CMD_USERTEMP_RRQ])?;
        if response.command != CMD_DATA {
            return Err(DeviceError::Message(format!(
                "template request returned command {}",
                response.command
            )));
        }
        decode_template_payload(&response.data)
    }

    fn push_user_template(
        &mut self,
        _user: &DeviceUser,
        _finger: &FingerTemplate,
    ) -> Result<(), DeviceError> {
        Err(DeviceError::Message(
            "push user/template is not implemented for the TCP adapter yet".to_string(),
        ))
    }

    fn disconnect(&mut self) {
        let _ = self.send_command(CMD_EXIT, &[]);
    }
}

impl ZktecoTcpClient {
    fn send_command(&mut self, command: u16, data: &[u8]) -> Result<ResponsePacket, DeviceError> {
        self.reply_id = self.reply_id.wrapping_add(1);
        if self.reply_id == USHRT_MAX {
            self.reply_id = 0;
        }
        let packet = make_tcp_packet(command, data, self.session_id, self.reply_id);
        self.stream
            .write_all(&packet)
            .map_err(|err| DeviceError::Message(format!("device write failed: {err}")))?;
        let response = read_tcp_packet(&mut self.stream)?;
        self.session_id = response.session_id;
        self.reply_id = response.reply_id;
        Ok(response)
    }
}

#[derive(Debug)]
struct ResponsePacket {
    command: u16,
    session_id: u16,
    reply_id: u16,
    data: Vec<u8>,
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

fn read_tcp_packet(stream: &mut TcpStream) -> Result<ResponsePacket, DeviceError> {
    let mut tcp_header = [0u8; 8];
    stream
        .read_exact(&mut tcp_header)
        .map_err(|err| DeviceError::Message(format!("device read failed: {err}")))?;

    let p1 = u16::from_le_bytes([tcp_header[0], tcp_header[1]]);
    let p2 = u16::from_le_bytes([tcp_header[2], tcp_header[3]]);
    if p1 != MACHINE_PREPARE_DATA_1 || p2 != MACHINE_PREPARE_DATA_2 {
        return Err(DeviceError::Message(
            "invalid ZKTeco TCP header".to_string(),
        ));
    }
    let packet_len =
        u32::from_le_bytes([tcp_header[4], tcp_header[5], tcp_header[6], tcp_header[7]]) as usize;
    if packet_len < 8 {
        return Err(DeviceError::Message(
            "invalid ZKTeco packet length".to_string(),
        ));
    }

    let mut packet = vec![0u8; packet_len];
    stream
        .read_exact(&mut packet)
        .map_err(|err| DeviceError::Message(format!("device packet read failed: {err}")))?;

    Ok(ResponsePacket {
        command: u16::from_le_bytes([packet[0], packet[1]]),
        session_id: u16::from_le_bytes([packet[4], packet[5]]),
        reply_id: u16::from_le_bytes([packet[6], packet[7]]),
        data: packet[8..].to_vec(),
    })
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
    (!(checksum as u16)) & 0xffff
}

fn decode_attendance_payload(payload: &[u8]) -> Result<Vec<RawAttendance>, DeviceError> {
    if payload.len() < 4 {
        return Err(DeviceError::Message(
            "attendance payload missing size header".to_string(),
        ));
    }
    let declared_size =
        u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]) as usize;
    let records = &payload[4..];
    if declared_size > records.len() {
        return Err(DeviceError::Message(
            "attendance payload is incomplete".to_string(),
        ));
    }

    let mut result = Vec::new();
    for record in records[..declared_size].chunks_exact(8) {
        let uid = u16::from_le_bytes([record[0], record[1]]);
        let encoded_time = u32::from_le_bytes([record[3], record[4], record[5], record[6]]);
        let punch = record[7] as i64;
        result.push(RawAttendance {
            user_id: uid.to_string(),
            timestamp: decode_timestamp(encoded_time),
            punch,
        });
    }
    Ok(result)
}

fn decode_template_payload(payload: &[u8]) -> Result<Vec<FingerTemplate>, DeviceError> {
    if payload.len() < 4 {
        return Err(DeviceError::Message(
            "template payload missing size header".to_string(),
        ));
    }
    let declared_size =
        u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]) as usize;
    if declared_size == 0 {
        return Ok(Vec::new());
    }
    Err(DeviceError::Message(
        "template payload decoding is not implemented for non-empty payloads yet".to_string(),
    ))
}

fn decode_timestamp(value: u32) -> String {
    let seconds = value % 60;
    let minutes_total = value / 60;
    let minute = minutes_total % 60;
    let hours_total = minutes_total / 60;
    let hour = hours_total % 24;
    let days_total = hours_total / 24;
    let day = (days_total % 31) + 1;
    let months_total = days_total / 31;
    let month = (months_total % 12) + 1;
    let year = (months_total / 12) + 2000;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{seconds:02}+00:00")
}
