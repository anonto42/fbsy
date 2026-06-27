//! Network address helpers.
//!
//! Services may bind to `0.0.0.0` so other machines on the LAN can reach them,
//! but users should see the machine's router-facing address, not the wildcard
//! bind address.

use std::net::{IpAddr, Ipv4Addr, UdpSocket};

/// Best-effort LAN IP for this machine.
///
/// The UDP socket trick asks the OS which local address it would use for a
/// normal outbound route. No packet is sent. If the machine has no route, we
/// return `None` and callers can fall back to loopback.
pub fn lan_ip() -> Option<IpAddr> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect((Ipv4Addr::new(8, 8, 8, 8), 80)).ok()?;
    let ip = socket.local_addr().ok()?.ip();
    is_displayable_ip(ip).then_some(ip)
}

/// LAN host string for display/URLs, falling back to localhost.
pub fn lan_host_or_loopback() -> String {
    lan_ip()
        .map(|ip| ip.to_string())
        .unwrap_or_else(|| Ipv4Addr::LOCALHOST.to_string())
}

fn is_displayable_ip(ip: IpAddr) -> bool {
    !ip.is_unspecified() && !ip.is_loopback()
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};

    use super::is_displayable_ip;

    #[test]
    fn displayable_ip_rejects_wildcard_and_loopback() {
        assert!(!is_displayable_ip(IpAddr::V4(Ipv4Addr::UNSPECIFIED)));
        assert!(!is_displayable_ip(IpAddr::V4(Ipv4Addr::LOCALHOST)));
        assert!(is_displayable_ip(IpAddr::V4(Ipv4Addr::new(
            192, 168, 1, 50
        ))));
    }
}
