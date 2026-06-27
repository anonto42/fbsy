//! Network scanner for discovering ZKTeco-like attendance devices.
//!
//! The scanner is intentionally conservative: it probes one port first, then
//! only performs the ZKTeco protocol handshake for hosts where that port is
//! reachable. That keeps subnet scans useful without pretending to be a general
//! purpose network inventory tool.

use std::{
    net::{Ipv4Addr, SocketAddr, TcpStream},
    thread,
    time::Duration,
};

use anyhow::{bail, Result};
use console::style;
use serde::Serialize;

use crate::{
    adapters::device_zkteco_tcp::ZktecoTcpConnector, config::BridgeDeviceConfig,
    ports::device::DeviceConnector, support::network,
};

const DEFAULT_PORT: u16 = 4370;
const DEFAULT_SCAN_TIMEOUT_MS: u64 = 350;
const DEFAULT_DEVICE_TIMEOUT_SECS: u64 = 2;
#[derive(Debug, Clone)]
pub struct ScanOptions {
    pub cidr: Option<String>,
    pub hosts: Vec<Ipv4Addr>,
    pub port: u16,
    pub scan_timeout_ms: u64,
    pub device_timeout_secs: u64,
    pub device_password: i32,
    pub force_udp: bool,
    pub include_open: bool,
    pub json: bool,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            cidr: None,
            hosts: Vec::new(),
            port: DEFAULT_PORT,
            scan_timeout_ms: DEFAULT_SCAN_TIMEOUT_MS,
            device_timeout_secs: DEFAULT_DEVICE_TIMEOUT_SECS,
            device_password: 0,
            force_udp: false,
            include_open: false,
            json: false,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanFinding {
    pub ip: String,
    pub port: u16,
    pub attendance_device: bool,
    pub protocol: String,
    pub serial: Option<String>,
    pub firmware: Option<String>,
    pub platform: Option<String>,
    pub name: Option<String>,
    pub users: Option<usize>,
    pub fingers: Option<usize>,
    pub records: Option<usize>,
    pub suggested_device_code: String,
    pub suggested_config: SuggestedDeviceConfig,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SuggestedDeviceConfig {
    pub device_ip: String,
    pub device_port: u16,
    pub device_password: i32,
    pub device_timeout: u64,
    pub device_force_udp: bool,
    pub device_omit_ping: bool,
    pub device_code: String,
    pub api_key: String,
    pub organization_id: u64,
    pub sync_interval_seconds: u64,
    pub clear_attendance_after_sync: bool,
}

pub fn run_scan(opts: ScanOptions) -> Result<()> {
    let targets = scan_targets(&opts)?;
    let findings = scan_targets_for_devices(&opts, targets);

    if opts.json {
        println!("{}", serde_json::to_string_pretty(&findings)?);
    } else {
        print_findings(&findings);
    }
    Ok(())
}

pub fn run_service(mut opts: ScanOptions, interval_seconds: u64) -> Result<()> {
    opts.json = false;
    let interval = interval_seconds.max(10);
    println!("FingerBridge scanner service starting");
    println!("  Target: {}", target_label(&opts));
    println!("  Port: {}", opts.port);
    println!("  Interval: {interval}s");
    println!("  TCP timeout: {}ms", opts.scan_timeout_ms);
    println!("  Device timeout: {}s", opts.device_timeout_secs);
    println!("  Include open non-ZKTeco hosts: {}", opts.include_open);

    loop {
        let started = chrono::Utc::now().to_rfc3339();
        match scan_targets(&opts) {
            Ok(targets) => {
                println!(
                    "[{started}] scanner: scanning {} target(s) on port {}",
                    targets.len(),
                    opts.port
                );
                let findings = scan_targets_for_devices(&opts, targets);
                let attendance = findings
                    .iter()
                    .filter(|finding| finding.attendance_device)
                    .count();
                println!(
                    "[{}] scanner: found {} candidate(s), {} confirmed attendance device(s)",
                    chrono::Utc::now().to_rfc3339(),
                    findings.len(),
                    attendance
                );
                for finding in &findings {
                    print_service_finding(finding);
                }
            }
            Err(err) => eprintln!("[{started}] scanner: {err}"),
        }
        thread::sleep(Duration::from_secs(interval));
    }
}

pub fn default_cidr() -> String {
    match network::lan_ip() {
        Some(std::net::IpAddr::V4(ip)) => {
            let [a, b, c, _] = ip.octets();
            format!("{a}.{b}.{c}.0/24")
        }
        _ => "127.0.0.1/32".to_string(),
    }
}

fn scan_targets(opts: &ScanOptions) -> Result<Vec<Ipv4Addr>> {
    if !opts.hosts.is_empty() {
        return Ok(opts.hosts.clone());
    }
    cidr_hosts(opts.cidr.as_deref().unwrap_or(""))
}

fn cidr_hosts(cidr: &str) -> Result<Vec<Ipv4Addr>> {
    let cidr = if cidr.trim().is_empty() {
        default_cidr()
    } else {
        cidr.trim().to_string()
    };
    let (base, prefix) = cidr
        .split_once('/')
        .ok_or_else(|| anyhow::anyhow!("CIDR must look like 192.168.1.0/24"))?;
    let base: Ipv4Addr = base.parse()?;
    let prefix: u32 = prefix.parse()?;
    if prefix > 32 {
        bail!("CIDR prefix must be 0..=32");
    }
    if prefix < 24 {
        bail!("scanner refuses networks larger than /24 for safety; pass --host for specific IPs");
    }

    let base = u32::from(base);
    let mask = if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix)
    };
    let network = base & mask;
    let total = 1u32 << (32 - prefix);

    let mut out = Vec::new();
    for offset in 0..total {
        let ip = Ipv4Addr::from(network + offset);
        if total > 2 && (offset == 0 || offset == total - 1) {
            continue;
        }
        out.push(ip);
    }
    Ok(out)
}

fn scan_targets_for_devices(opts: &ScanOptions, targets: Vec<Ipv4Addr>) -> Vec<ScanFinding> {
    let mut handles = Vec::with_capacity(targets.len());
    for ip in targets {
        let opts = opts.clone();
        handles.push(thread::spawn(move || scan_one(&opts, ip)));
    }

    let mut findings = handles
        .into_iter()
        .filter_map(|handle| handle.join().ok().flatten())
        .collect::<Vec<_>>();
    findings.sort_by_key(|finding| finding.ip_as_sort_key());
    findings
}

fn scan_one(opts: &ScanOptions, ip: Ipv4Addr) -> Option<ScanFinding> {
    if !opts.force_udp {
        let address = SocketAddr::from((ip, opts.port));
        let scan_timeout = Duration::from_millis(opts.scan_timeout_ms.max(50));
        if TcpStream::connect_timeout(&address, scan_timeout).is_err() {
            return None;
        }
    }

    let connector = ZktecoTcpConnector;
    let cfg = BridgeDeviceConfig {
        device_ip: ip.to_string(),
        device_port: opts.port,
        device_password: opts.device_password,
        device_timeout: opts.device_timeout_secs.max(1),
        device_force_udp: opts.force_udp,
        device_omit_ping: true,
        device_code: suggested_device_code(None, ip),
        api_key: String::new(),
        organization_id: 1,
        sync_interval_seconds: 300,
        clear_attendance_after_sync: false,
    };

    match connector.connect(&cfg) {
        Ok(mut client) => {
            let info = client.device_info();
            client.disconnect();
            match info {
                Ok(info) => {
                    let serial = none_if_empty(info.serial);
                    let device_code = suggested_device_code(serial.as_deref(), ip);
                    Some(ScanFinding {
                        ip: ip.to_string(),
                        port: opts.port,
                        attendance_device: true,
                        protocol: "zkteco".to_string(),
                        serial,
                        firmware: none_if_empty(info.firmware),
                        platform: none_if_empty(info.platform),
                        name: none_if_empty(info.name),
                        users: Some(info.users),
                        fingers: Some(info.fingers),
                        records: Some(info.records),
                        suggested_device_code: device_code.clone(),
                        suggested_config: suggested_config(&cfg, device_code),
                        error: None,
                    })
                }
                Err(err) if opts.include_open => Some(open_only_finding(&cfg, err.to_string())),
                Err(_) => None,
            }
        }
        Err(err) if opts.include_open => Some(open_only_finding(&cfg, err.to_string())),
        Err(_) => None,
    }
}

fn open_only_finding(cfg: &BridgeDeviceConfig, error: String) -> ScanFinding {
    let device_code =
        suggested_device_code(None, cfg.device_ip.parse().unwrap_or(Ipv4Addr::LOCALHOST));
    ScanFinding {
        ip: cfg.device_ip.clone(),
        port: cfg.device_port,
        attendance_device: false,
        protocol: "open-port".to_string(),
        serial: None,
        firmware: None,
        platform: None,
        name: None,
        users: None,
        fingers: None,
        records: None,
        suggested_device_code: device_code.clone(),
        suggested_config: suggested_config(cfg, device_code),
        error: Some(error.chars().take(160).collect()),
    }
}

fn suggested_config(cfg: &BridgeDeviceConfig, device_code: String) -> SuggestedDeviceConfig {
    SuggestedDeviceConfig {
        device_ip: cfg.device_ip.clone(),
        device_port: cfg.device_port,
        device_password: cfg.device_password,
        device_timeout: 15,
        device_force_udp: cfg.device_force_udp,
        device_omit_ping: true,
        device_code,
        api_key: "CHANGE_ME".to_string(),
        organization_id: 1,
        sync_interval_seconds: 300,
        clear_attendance_after_sync: false,
    }
}

fn suggested_device_code(serial: Option<&str>, ip: Ipv4Addr) -> String {
    if let Some(serial) = serial {
        let serial = serial.trim();
        if !serial.is_empty() {
            return serial
                .chars()
                .map(|ch| {
                    if ch.is_ascii_alphanumeric() {
                        ch.to_ascii_uppercase()
                    } else {
                        '-'
                    }
                })
                .collect();
        }
    }
    format!("ZK-{}", ip.to_string().replace('.', "-"))
}

fn none_if_empty(value: String) -> Option<String> {
    let value = value.trim().to_string();
    (!value.is_empty()).then_some(value)
}

fn target_label(opts: &ScanOptions) -> String {
    if !opts.hosts.is_empty() {
        return opts
            .hosts
            .iter()
            .map(Ipv4Addr::to_string)
            .collect::<Vec<_>>()
            .join(", ");
    }
    opts.cidr.clone().unwrap_or_else(default_cidr)
}

fn print_findings(findings: &[ScanFinding]) {
    if findings.is_empty() {
        println!(
            "{} No ZKTeco-like attendance devices found.",
            style("!").yellow().bold()
        );
        println!("Try a specific host: fbsy scanner scan --host 192.168.1.50");
        println!("Or include open ports: fbsy scanner scan --include-open");
        return;
    }

    println!(
        "{:<15} {:<6} {:<10} {:<18} {:<14} {:<8} {:<8} {:<8} {}",
        style("IP").bold(),
        style("PORT").bold(),
        style("TYPE").bold(),
        style("DEVICE CODE").bold(),
        style("SERIAL").bold(),
        style("USERS").bold(),
        style("FINGERS").bold(),
        style("RECORDS").bold(),
        style("FIRMWARE").bold()
    );
    for finding in findings {
        println!(
            "{:<15} {:<6} {:<10} {:<18} {:<14} {:<8} {:<8} {:<8} {}",
            finding.ip,
            finding.port,
            if finding.attendance_device {
                "zkteco"
            } else {
                "open"
            },
            finding.suggested_device_code,
            finding.serial.as_deref().unwrap_or("-"),
            finding
                .users
                .map(|v| v.to_string())
                .unwrap_or_else(|| "-".into()),
            finding
                .fingers
                .map(|v| v.to_string())
                .unwrap_or_else(|| "-".into()),
            finding
                .records
                .map(|v| v.to_string())
                .unwrap_or_else(|| "-".into()),
            finding.firmware.as_deref().unwrap_or("-")
        );
    }

    println!();
    println!(
        "{}",
        style("Suggested config block for first device:")
            .yellow()
            .bold()
    );
    if let Some(first) = findings.first() {
        println!(
            "{}",
            serde_json::to_string_pretty(&first.suggested_config).unwrap_or_default()
        );
    }
}

fn print_service_finding(finding: &ScanFinding) {
    if finding.attendance_device {
        println!(
            "scanner: {}:{} zkteco serial={} firmware={} users={} fingers={} records={} suggestedDeviceCode={}",
            finding.ip,
            finding.port,
            finding.serial.as_deref().unwrap_or("-"),
            finding.firmware.as_deref().unwrap_or("-"),
            finding.users.unwrap_or(0),
            finding.fingers.unwrap_or(0),
            finding.records.unwrap_or(0),
            finding.suggested_device_code
        );
    } else {
        println!(
            "scanner: {}:{} open but not confirmed as ZKTeco ({})",
            finding.ip,
            finding.port,
            finding.error.as_deref().unwrap_or("unknown protocol")
        );
    }
}

impl ScanFinding {
    fn ip_as_sort_key(&self) -> u32 {
        self.ip.parse::<Ipv4Addr>().map(u32::from).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::{cidr_hosts, suggested_device_code};

    #[test]
    fn cidr_hosts_expands_usable_hosts() {
        let hosts = cidr_hosts("192.168.10.0/30").expect("hosts");
        assert_eq!(
            hosts,
            [
                Ipv4Addr::new(192, 168, 10, 1),
                Ipv4Addr::new(192, 168, 10, 2)
            ]
        );
    }

    #[test]
    fn cidr_hosts_accepts_single_host() {
        assert_eq!(
            cidr_hosts("192.168.10.55/32").expect("host"),
            [Ipv4Addr::new(192, 168, 10, 55)]
        );
    }

    #[test]
    fn suggested_code_prefers_serial() {
        assert_eq!(
            suggested_device_code(Some("zk sn 01"), Ipv4Addr::new(192, 168, 1, 50)),
            "ZK-SN-01"
        );
        assert_eq!(
            suggested_device_code(None, Ipv4Addr::new(192, 168, 1, 50)),
            "ZK-192-168-1-50"
        );
    }
}
