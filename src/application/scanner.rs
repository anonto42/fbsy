//! Network scanner for discovering services and attendance devices on the LAN.
//!
//! Supports multi-port scanning with protocol identification:
//! - ZKTeco attendance devices (port 4370)
//! - HTTP/HTTPS servers (ports 80, 443, 3000, 8080, 8800, etc.)
//! - SSH servers (port 22)
//! - Databases (MySQL 3306, PostgreSQL 5432, Redis 6379)
//! - Any open TCP port
//!
//! Subnet sizes up to /16 are supported with the `--wide` safety flag.
//! Scanning is batched (64 hosts at a time) to avoid resource exhaustion.

use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpStream},
    sync::{Arc, Mutex},
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

// ── Constants ─────────────────────────────────────────────────────────────────

const DEFAULT_PORT: u16 = 4370;
const DEFAULT_SCAN_TIMEOUT_MS: u64 = 350;
const DEFAULT_DEVICE_TIMEOUT_SECS: u64 = 2;

/// Maximum concurrent threads per batch when scanning large networks.
const BATCH_SIZE: usize = 64;

/// Common server ports scanned when `--all-ports` is used.
pub const COMMON_PORTS: &[u16] = &[
    22,   // SSH
    80,   // HTTP
    443,  // HTTPS
    3000, // Dev servers (Node, Rails, etc.)
    3306, // MySQL
    4370, // ZKTeco attendance devices
    5432, // PostgreSQL
    6379, // Redis
    7431, // fbsy bridge
    8080, // HTTP alt / proxies
    8443, // HTTPS alt
    8800, // fbsy mock HRMS
    9090, // Prometheus / misc
];

// ── Options ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ScanOptions {
    pub cidr: Option<String>,
    pub hosts: Vec<Ipv4Addr>,
    /// Ports to scan. When empty, defaults to `[4370]`.
    pub ports: Vec<u16>,
    /// When true, scan all `COMMON_PORTS`.
    pub all_ports: bool,
    /// Allow network ranges wider than /24 (up to /16).
    pub wide: bool,
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
            ports: vec![DEFAULT_PORT],
            all_ports: false,
            wide: false,
            scan_timeout_ms: DEFAULT_SCAN_TIMEOUT_MS,
            device_timeout_secs: DEFAULT_DEVICE_TIMEOUT_SECS,
            device_password: 0,
            force_udp: false,
            include_open: false,
            json: false,
        }
    }
}

impl ScanOptions {
    /// Resolve which ports to actually scan based on flags.
    pub fn effective_ports(&self) -> Vec<u16> {
        if self.all_ports {
            return COMMON_PORTS.to_vec();
        }
        if self.ports.is_empty() {
            return vec![DEFAULT_PORT];
        }
        self.ports.clone()
    }
}

// ── Findings ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanFinding {
    pub ip: String,
    pub port: u16,
    pub service_type: String,
    pub attendance_device: bool,
    pub protocol: String,
    pub serial: Option<String>,
    pub firmware: Option<String>,
    pub platform: Option<String>,
    pub name: Option<String>,
    pub users: Option<usize>,
    pub fingers: Option<usize>,
    pub records: Option<usize>,
    pub http_status: Option<u16>,
    pub http_server: Option<String>,
    pub suggested_device_code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested_config: Option<SuggestedDeviceConfig>,
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

impl ScanFinding {
    fn ip_as_sort_key(&self) -> (u32, u16) {
        (
            self.ip.parse::<Ipv4Addr>().map(u32::from).unwrap_or(0),
            self.port,
        )
    }
}

// ── Public entry points ───────────────────────────────────────────────────────

/// One-shot scan: probe, print results, and exit.
pub fn run_scan(opts: ScanOptions) -> Result<()> {
    let targets = scan_targets(&opts)?;
    let ports = opts.effective_ports();
    let total_probes = targets.len() * ports.len();

    if !opts.json {
        println!(
            "{} Scanning {} host(s) × {} port(s) = {} probe(s)…",
            style("⟐").cyan().bold(),
            targets.len(),
            ports.len(),
            total_probes
        );
    }

    let findings = scan_targets_for_services(&opts, targets, &ports);

    if opts.json {
        println!("{}", serde_json::to_string_pretty(&findings)?);
    } else {
        print_findings(&findings);
    }
    Ok(())
}

/// Background service loop: scan at an interval, print results to log.
pub fn run_service(mut opts: ScanOptions, interval_seconds: u64) -> Result<()> {
    opts.json = false;
    let interval = interval_seconds.max(10);
    let ports = opts.effective_ports();

    println!("FingerBridge scanner service starting");
    println!("  Target: {}", target_label(&opts));
    println!("  Ports: {}", format_port_list(&ports));
    println!("  Interval: {interval}s");
    println!("  TCP timeout: {}ms", opts.scan_timeout_ms);
    println!("  Device timeout: {}s", opts.device_timeout_secs);
    println!("  Include open non-identified hosts: {}", opts.include_open);

    loop {
        let started = chrono::Utc::now().to_rfc3339();
        match scan_targets(&opts) {
            Ok(targets) => {
                println!(
                    "[{started}] scanner: scanning {} host(s) × {} port(s)",
                    targets.len(),
                    ports.len(),
                );
                let findings = scan_targets_for_services(&opts, targets, &ports);
                let attendance = findings.iter().filter(|f| f.attendance_device).count();
                let services = findings.len();
                println!(
                    "[{}] scanner: found {} service(s), {} attendance device(s)",
                    chrono::Utc::now().to_rfc3339(),
                    services,
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

// ── Target resolution ─────────────────────────────────────────────────────────

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
    cidr_hosts(opts.cidr.as_deref().unwrap_or(""), opts.wide)
}

fn cidr_hosts(cidr: &str, wide: bool) -> Result<Vec<Ipv4Addr>> {
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
    if prefix < 16 {
        bail!("scanner refuses networks larger than /16 for safety");
    }
    if prefix < 24 && !wide {
        bail!(
            "network /{prefix} has {} hosts — pass --wide to allow ranges larger than /24\n\
             Or use --host to scan specific IPs",
            1u32 << (32 - prefix)
        );
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
        // Skip network and broadcast addresses for ranges > /31.
        if total > 2 && (offset == 0 || offset == total - 1) {
            continue;
        }
        out.push(ip);
    }
    Ok(out)
}

// ── Multi-port parallel scanning ──────────────────────────────────────────────

/// Scan all targets across all ports, batched to avoid thread explosion.
fn scan_targets_for_services(
    opts: &ScanOptions,
    targets: Vec<Ipv4Addr>,
    ports: &[u16],
) -> Vec<ScanFinding> {
    let findings = Arc::new(Mutex::new(Vec::new()));

    // Build all (ip, port) pairs.
    let mut probes: Vec<(Ipv4Addr, u16)> = Vec::with_capacity(targets.len() * ports.len());
    for &ip in &targets {
        for &port in ports {
            probes.push((ip, port));
        }
    }

    // Process in batches to limit concurrency.
    for batch in probes.chunks(BATCH_SIZE) {
        let mut handles = Vec::with_capacity(batch.len());
        for &(ip, port) in batch {
            let opts = opts.clone();
            let findings = Arc::clone(&findings);
            handles.push(thread::spawn(move || {
                if let Some(finding) = scan_one_port(&opts, ip, port) {
                    findings.lock().unwrap().push(finding);
                }
            }));
        }
        for handle in handles {
            let _ = handle.join();
        }
    }

    let mut results = match Arc::try_unwrap(findings) {
        Ok(mutex) => mutex.into_inner().unwrap(),
        Err(arc) => arc.lock().unwrap().clone(),
    };
    results.sort_by_key(|f| f.ip_as_sort_key());
    results
}

/// Probe a single (ip, port) pair. Returns `Some(finding)` if the port is open
/// and either the service is identified or `include_open` is set.
fn scan_one_port(opts: &ScanOptions, ip: Ipv4Addr, port: u16) -> Option<ScanFinding> {
    let address = SocketAddr::from((ip, port));
    let scan_timeout = Duration::from_millis(opts.scan_timeout_ms.max(50));

    // Quick TCP connect probe — skip if port is closed.
    if !opts.force_udp {
        if TcpStream::connect_timeout(&address, scan_timeout).is_err() {
            return None;
        }
    }

    // Try protocol-specific identification based on port.
    match port {
        4370 => probe_zkteco(opts, ip, port),
        80 | 443 | 3000 | 7431 | 8080 | 8443 | 8800 | 9090 => probe_http(opts, ip, port),
        22 => probe_ssh(opts, ip, port),
        3306 => probe_generic(opts, ip, port, "mysql"),
        5432 => probe_generic(opts, ip, port, "postgresql"),
        6379 => probe_redis(opts, ip, port),
        _ => probe_generic(opts, ip, port, "open"),
    }
}

// ── Protocol probes ───────────────────────────────────────────────────────────

/// ZKTeco attendance device probe (existing logic).
fn probe_zkteco(opts: &ScanOptions, ip: Ipv4Addr, port: u16) -> Option<ScanFinding> {
    let connector = ZktecoTcpConnector;
    let cfg = BridgeDeviceConfig {
        device_ip: ip.to_string(),
        device_port: port,
        device_password: opts.device_password,
        device_timeout: opts.device_timeout_secs.max(1),
        device_force_udp: opts.force_udp,
        device_omit_ping: true,
        device_timezone: None,
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
                        port,
                        service_type: "zkteco".to_string(),
                        attendance_device: true,
                        protocol: "zkteco".to_string(),
                        serial,
                        firmware: none_if_empty(info.firmware),
                        platform: none_if_empty(info.platform),
                        name: none_if_empty(info.name),
                        users: Some(info.users),
                        fingers: Some(info.fingers),
                        records: Some(info.records),
                        http_status: None,
                        http_server: None,
                        suggested_device_code: device_code.clone(),
                        suggested_config: Some(suggested_config(&cfg, device_code)),
                        error: None,
                    })
                }
                Err(err) if opts.include_open => Some(open_finding(ip, port, err.to_string())),
                Err(_) => None,
            }
        }
        Err(err) if opts.include_open => Some(open_finding(ip, port, err.to_string())),
        Err(_) => None,
    }
}

/// HTTP service probe: send a minimal GET request, read status and Server header.
fn probe_http(opts: &ScanOptions, ip: Ipv4Addr, port: u16) -> Option<ScanFinding> {
    let address = SocketAddr::from((ip, port));
    let timeout = Duration::from_secs(opts.device_timeout_secs.max(1));

    let result = (|| -> std::result::Result<(u16, Option<String>), String> {
        let mut stream =
            TcpStream::connect_timeout(&address, timeout).map_err(|e| e.to_string())?;
        stream
            .set_read_timeout(Some(timeout))
            .map_err(|e| e.to_string())?;
        stream
            .set_write_timeout(Some(timeout))
            .map_err(|e| e.to_string())?;

        let request = format!("GET / HTTP/1.0\r\nHost: {ip}\r\nConnection: close\r\n\r\n");
        stream
            .write_all(request.as_bytes())
            .map_err(|e| e.to_string())?;

        let reader = BufReader::new(&mut stream);
        let mut status_code: u16 = 0;
        let mut server_header: Option<String> = None;

        for (i, line) in reader.lines().enumerate() {
            let line = match line {
                Ok(l) => l,
                Err(_) => break,
            };
            if i == 0 {
                // Parse "HTTP/1.x STATUS ..."
                let parts: Vec<&str> = line.splitn(3, ' ').collect();
                if parts.len() >= 2 {
                    status_code = parts[1].parse().unwrap_or(0);
                }
            }
            if line.is_empty() {
                break; // End of headers.
            }
            if let Some(val) = line
                .strip_prefix("Server: ")
                .or_else(|| line.strip_prefix("server: "))
            {
                server_header = Some(val.chars().take(80).collect());
            }
            // Only read headers (max ~30 lines for safety).
            if i > 30 {
                break;
            }
        }

        Ok((status_code, server_header))
    })();

    match result {
        Ok((status, server)) => {
            let service_label = match port {
                443 | 8443 => "https",
                7431 => "fbsy-bridge",
                8800 => "fbsy-hrms",
                _ => "http",
            };
            Some(ScanFinding {
                ip: ip.to_string(),
                port,
                service_type: service_label.to_string(),
                attendance_device: false,
                protocol: "http".to_string(),
                serial: None,
                firmware: None,
                platform: None,
                name: server.clone(),
                users: None,
                fingers: None,
                records: None,
                http_status: Some(status),
                http_server: server,
                suggested_device_code: format!("SRV-{}", ip.to_string().replace('.', "-")),
                suggested_config: None,
                error: None,
            })
        }
        Err(err) if opts.include_open => Some(open_finding(ip, port, err)),
        Err(_) => {
            // Port was open (we connected above) but HTTP probe failed.
            // Still report it since the port IS open.
            Some(ScanFinding {
                ip: ip.to_string(),
                port,
                service_type: port_label(port).to_string(),
                attendance_device: false,
                protocol: "tcp".to_string(),
                serial: None,
                firmware: None,
                platform: None,
                name: None,
                users: None,
                fingers: None,
                records: None,
                http_status: None,
                http_server: None,
                suggested_device_code: format!("SRV-{}", ip.to_string().replace('.', "-")),
                suggested_config: None,
                error: Some("HTTP probe failed; port is open".to_string()),
            })
        }
    }
}

/// SSH probe: connect and read the banner (e.g., "SSH-2.0-OpenSSH_9.6").
fn probe_ssh(opts: &ScanOptions, ip: Ipv4Addr, port: u16) -> Option<ScanFinding> {
    let address = SocketAddr::from((ip, port));
    let timeout = Duration::from_secs(opts.device_timeout_secs.max(1));

    let banner = (|| -> std::result::Result<String, String> {
        let mut stream =
            TcpStream::connect_timeout(&address, timeout).map_err(|e| e.to_string())?;
        stream
            .set_read_timeout(Some(timeout))
            .map_err(|e| e.to_string())?;
        let mut buf = [0u8; 256];
        let n = stream.read(&mut buf).map_err(|e| e.to_string())?;
        Ok(String::from_utf8_lossy(&buf[..n]).trim().to_string())
    })();

    match banner {
        Ok(b) if b.starts_with("SSH-") => Some(ScanFinding {
            ip: ip.to_string(),
            port,
            service_type: "ssh".to_string(),
            attendance_device: false,
            protocol: "ssh".to_string(),
            serial: None,
            firmware: None,
            platform: None,
            name: Some(b.chars().take(80).collect()),
            users: None,
            fingers: None,
            records: None,
            http_status: None,
            http_server: None,
            suggested_device_code: format!("SRV-{}", ip.to_string().replace('.', "-")),
            suggested_config: None,
            error: None,
        }),
        Ok(_) if opts.include_open => Some(open_finding(ip, port, "not SSH".to_string())),
        Ok(_) => Some(open_finding(ip, port, "open (not SSH banner)".to_string())),
        Err(err) if opts.include_open => Some(open_finding(ip, port, err)),
        Err(_) => None,
    }
}

/// Redis probe: send PING, expect +PONG.
fn probe_redis(opts: &ScanOptions, ip: Ipv4Addr, port: u16) -> Option<ScanFinding> {
    let address = SocketAddr::from((ip, port));
    let timeout = Duration::from_secs(opts.device_timeout_secs.max(1));

    let is_redis = (|| -> std::result::Result<bool, String> {
        let mut stream =
            TcpStream::connect_timeout(&address, timeout).map_err(|e| e.to_string())?;
        stream
            .set_read_timeout(Some(timeout))
            .map_err(|e| e.to_string())?;
        stream
            .set_write_timeout(Some(timeout))
            .map_err(|e| e.to_string())?;
        stream.write_all(b"PING\r\n").map_err(|e| e.to_string())?;
        let mut buf = [0u8; 64];
        let n = stream.read(&mut buf).map_err(|e| e.to_string())?;
        let resp = String::from_utf8_lossy(&buf[..n]);
        Ok(resp.contains("+PONG") || resp.contains("-NOAUTH"))
    })();

    match is_redis {
        Ok(true) => Some(ScanFinding {
            ip: ip.to_string(),
            port,
            service_type: "redis".to_string(),
            attendance_device: false,
            protocol: "redis".to_string(),
            serial: None,
            firmware: None,
            platform: None,
            name: Some("Redis".to_string()),
            users: None,
            fingers: None,
            records: None,
            http_status: None,
            http_server: None,
            suggested_device_code: format!("SRV-{}", ip.to_string().replace('.', "-")),
            suggested_config: None,
            error: None,
        }),
        Ok(false) => probe_generic(opts, ip, port, "open"),
        Err(err) if opts.include_open => Some(open_finding(ip, port, err)),
        Err(_) => None,
    }
}

/// Generic open port finding (MySQL, PostgreSQL, or unknown services).
fn probe_generic(opts: &ScanOptions, ip: Ipv4Addr, port: u16, label: &str) -> Option<ScanFinding> {
    // Port was already confirmed open by the caller (scan_one_port).
    let service_type = if label == "open" {
        port_label(port).to_string()
    } else {
        label.to_string()
    };
    if !opts.include_open && label == "open" {
        // For truly unknown ports, only report when include_open is set.
        return None;
    }
    Some(ScanFinding {
        ip: ip.to_string(),
        port,
        service_type,
        attendance_device: false,
        protocol: "tcp".to_string(),
        serial: None,
        firmware: None,
        platform: None,
        name: None,
        users: None,
        fingers: None,
        records: None,
        http_status: None,
        http_server: None,
        suggested_device_code: format!("SRV-{}", ip.to_string().replace('.', "-")),
        suggested_config: None,
        error: None,
    })
}

fn open_finding(ip: Ipv4Addr, port: u16, error: String) -> ScanFinding {
    ScanFinding {
        ip: ip.to_string(),
        port,
        service_type: port_label(port).to_string(),
        attendance_device: false,
        protocol: "tcp".to_string(),
        serial: None,
        firmware: None,
        platform: None,
        name: None,
        users: None,
        fingers: None,
        records: None,
        http_status: None,
        http_server: None,
        suggested_device_code: format!("SRV-{}", ip.to_string().replace('.', "-")),
        suggested_config: None,
        error: Some(error.chars().take(160).collect()),
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

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

pub fn suggested_device_code(serial: Option<&str>, ip: Ipv4Addr) -> String {
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

/// Human-readable label for a well-known port number.
fn port_label(port: u16) -> &'static str {
    match port {
        22 => "ssh",
        80 => "http",
        443 => "https",
        3000 => "http-dev",
        3306 => "mysql",
        4370 => "zkteco",
        5432 => "postgresql",
        6379 => "redis",
        7431 => "fbsy-bridge",
        8080 => "http-alt",
        8443 => "https-alt",
        8800 => "fbsy-hrms",
        9090 => "http-mgmt",
        _ => "open",
    }
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

fn format_port_list(ports: &[u16]) -> String {
    if ports.len() <= 5 {
        ports
            .iter()
            .map(|p| format!("{p} ({})", port_label(*p)))
            .collect::<Vec<_>>()
            .join(", ")
    } else {
        format!(
            "{} ports ({}–{})",
            ports.len(),
            ports.first().unwrap_or(&0),
            ports.last().unwrap_or(&0)
        )
    }
}

/// Parse a comma-separated port string like "80,443,3000" into a Vec<u16>.
pub fn parse_ports_csv(csv: &str) -> Vec<u16> {
    csv.split(',')
        .filter_map(|s| s.trim().parse::<u16>().ok())
        .collect()
}

// ── Output formatting ─────────────────────────────────────────────────────────

fn print_findings(findings: &[ScanFinding]) {
    if findings.is_empty() {
        println!(
            "{} No services found on the network.",
            style("!").yellow().bold()
        );
        println!("Try:  fbsy scan --all-ports           ← scan common ports");
        println!("      fbsy scan --host 192.168.1.50   ← scan a specific host");
        println!("      fbsy scan --include-open        ← include unidentified open ports");
        return;
    }

    // Summary line.
    let zk_count = findings.iter().filter(|f| f.attendance_device).count();
    let svc_count = findings.iter().filter(|f| !f.attendance_device).count();
    println!(
        "\n{} Found {} service(s): {} attendance device(s), {} other server(s)\n",
        style("✔").green().bold(),
        findings.len(),
        zk_count,
        svc_count
    );

    // Table header.
    println!(
        "  {:<16} {:<6} {:<14} {:<20} {}",
        style("IP").bold(),
        style("PORT").bold(),
        style("SERVICE").bold(),
        style("IDENTITY").bold(),
        style("DETAIL").bold(),
    );
    println!("  {}", "─".repeat(78));

    for finding in findings {
        let service_styled = match finding.service_type.as_str() {
            "zkteco" => style(&finding.service_type).green().bold().to_string(),
            "http" | "https" | "http-dev" | "http-alt" | "https-alt" | "fbsy-bridge"
            | "fbsy-hrms" => style(&finding.service_type).cyan().to_string(),
            "ssh" => style(&finding.service_type).blue().to_string(),
            "redis" | "mysql" | "postgresql" => style(&finding.service_type).magenta().to_string(),
            _ => style(&finding.service_type).yellow().to_string(),
        };

        let identity = if finding.attendance_device {
            finding
                .serial
                .as_deref()
                .or(finding.name.as_deref())
                .unwrap_or(&finding.suggested_device_code)
                .to_string()
        } else {
            finding
                .name
                .as_deref()
                .or(finding.http_server.as_deref())
                .unwrap_or("-")
                .to_string()
        };

        let detail = if finding.attendance_device {
            format!(
                "users={} fingers={} records={} fw={}",
                finding.users.unwrap_or(0),
                finding.fingers.unwrap_or(0),
                finding.records.unwrap_or(0),
                finding.firmware.as_deref().unwrap_or("-"),
            )
        } else if let Some(status) = finding.http_status {
            format!("HTTP {status}")
        } else if let Some(err) = &finding.error {
            err.clone()
        } else {
            "-".to_string()
        };

        println!(
            "  {:<16} {:<6} {:<14} {:<20} {}",
            finding.ip,
            finding.port,
            service_styled,
            truncate(&identity, 20),
            truncate(&detail, 40),
        );
    }

    // Suggested config for first attendance device found.
    let first_zk = findings.iter().find(|f| f.attendance_device);
    if let Some(zk) = first_zk {
        if let Some(cfg) = &zk.suggested_config {
            println!();
            println!(
                "{}",
                style("Suggested config block for first attendance device:")
                    .yellow()
                    .bold()
            );
            println!("{}", serde_json::to_string_pretty(cfg).unwrap_or_default());
        }
    }
    println!();
}

fn print_service_finding(finding: &ScanFinding) {
    if finding.attendance_device {
        println!(
            "scanner: {}:{} zkteco serial={} firmware={} users={} fingers={} records={} code={}",
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
            "scanner: {}:{} {} {}",
            finding.ip,
            finding.port,
            finding.service_type,
            finding
                .name
                .as_deref()
                .or(finding.http_server.as_deref())
                .or(finding.error.as_deref())
                .unwrap_or("")
        );
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max - 1])
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::{cidr_hosts, parse_ports_csv, port_label, suggested_device_code};

    #[test]
    fn cidr_hosts_expands_usable_hosts() {
        let hosts = cidr_hosts("192.168.10.0/30", false).expect("hosts");
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
            cidr_hosts("192.168.10.55/32", false).expect("host"),
            [Ipv4Addr::new(192, 168, 10, 55)]
        );
    }

    #[test]
    fn cidr_hosts_rejects_wide_without_flag() {
        assert!(cidr_hosts("192.168.0.0/16", false).is_err());
    }

    #[test]
    fn cidr_hosts_allows_wide_with_flag() {
        let hosts = cidr_hosts("192.168.0.0/20", true).expect("wide hosts");
        assert_eq!(hosts.len(), 4094);
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

    #[test]
    fn parse_ports_csv_works() {
        assert_eq!(parse_ports_csv("80,443,3000"), vec![80, 443, 3000]);
        assert_eq!(parse_ports_csv("4370"), vec![4370]);
        assert_eq!(parse_ports_csv("80, 443, bad, 22"), vec![80, 443, 22]);
    }

    #[test]
    fn port_label_known_ports() {
        assert_eq!(port_label(22), "ssh");
        assert_eq!(port_label(80), "http");
        assert_eq!(port_label(4370), "zkteco");
        assert_eq!(port_label(12345), "open");
    }
}
