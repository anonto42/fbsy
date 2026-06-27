//! Service identity.
//!
//! A "service" is a long-running background process `fbsy` can start, stop, and
//! inspect by name. This module is the single source of truth for service names
//! so the registry, process manager, and CLI never pass around bare strings.

/// The set of services `fbsy` can manage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceKind {
    /// The real attendance bridge (pull from device, forward to HRMS).
    AtBridge,
    /// Mock ZKTeco device server for local testing.
    Zkteco,
    /// Mock HRMS webhook server for local testing.
    Hrms,
    /// Local network scanner for discovering attendance devices.
    Scanner,
}

impl ServiceKind {
    /// Stable on-disk / CLI name.
    pub fn name(self) -> &'static str {
        match self {
            ServiceKind::AtBridge => "bridge",
            ServiceKind::Zkteco => "zkteco",
            ServiceKind::Hrms => "hrms",
            ServiceKind::Scanner => "scanner",
        }
    }

    /// Parse a service name; `None` if unknown. `at-bridge` is accepted as a
    /// legacy alias for `bridge`.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "bridge" | "at-bridge" => Some(ServiceKind::AtBridge),
            "zkteco" => Some(ServiceKind::Zkteco),
            "hrms" => Some(ServiceKind::Hrms),
            "scanner" => Some(ServiceKind::Scanner),
            _ => None,
        }
    }

    /// All services, in display order.
    pub fn all() -> [ServiceKind; 4] {
        [
            ServiceKind::AtBridge,
            ServiceKind::Scanner,
            ServiceKind::Zkteco,
            ServiceKind::Hrms,
        ]
    }

    /// One-line description for help and `show`.
    pub fn description(self) -> &'static str {
        match self {
            ServiceKind::AtBridge => "attendance bridge (device -> HRMS)",
            ServiceKind::Zkteco => "mock ZKTeco device server",
            ServiceKind::Hrms => "mock HRMS webhook server",
            ServiceKind::Scanner => "LAN attendance device scanner",
        }
    }
}
