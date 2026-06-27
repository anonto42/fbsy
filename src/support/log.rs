//! Structured service logging.
//!
//! Every service runs as a detached child whose stdout is appended to a durable
//! per-instance log file (see [`crate::runtime::process`]). Routing event logs
//! through this helper gives every line a uniform shape:
//!
//! ```text
//! 2026-06-27T10:15:02.310Z INFO  [sync GATE-01] device returned 5 record(s)
//! ```
//!
//! The leading RFC 3339 timestamp lets the dashboard merge multiple instances'
//! logs into one chronological stream, and the `LEVEL`/`[component]` tags make
//! the files greppable for after-the-fact diagnosis (e.g. `grep ' ERROR '`).
//!
//! One-time human banners (startup summaries, setup-wizard values) stay as plain
//! `println!` — this is for *events*, not UI.

use chrono::Utc;

/// Severity of a log event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

impl Level {
    fn label(self) -> &'static str {
        match self {
            Level::Info => "INFO ",
            Level::Warn => "WARN ",
            Level::Error => "ERROR",
        }
    }
}

/// Write one structured event line to stdout (captured into the instance log).
pub fn event(level: Level, component: &str, args: std::fmt::Arguments<'_>) {
    let ts = Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ");
    println!("{ts} {} [{component}] {args}", level.label());
}

/// Log an informational event.
pub fn info(component: &str, args: std::fmt::Arguments<'_>) {
    event(Level::Info, component, args);
}

/// Log a warning event (recoverable / expected-but-notable).
pub fn warn(component: &str, args: std::fmt::Arguments<'_>) {
    event(Level::Warn, component, args);
}

/// Log an error event (a failed operation).
pub fn error(component: &str, args: std::fmt::Arguments<'_>) {
    event(Level::Error, component, args);
}

/// Parse the leading RFC 3339 timestamp of a structured log line, if present.
/// Used by the dashboard to merge instance logs chronologically.
pub fn parse_timestamp(line: &str) -> Option<chrono::DateTime<Utc>> {
    let token = line.split_whitespace().next()?;
    chrono::DateTime::parse_from_rfc3339(token)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_timestamp_reads_leading_rfc3339() {
        let line = "2026-06-27T10:15:02.310Z INFO  [sync GATE-01] hello";
        assert!(parse_timestamp(line).is_some());
    }

    #[test]
    fn parse_timestamp_rejects_untimestamped_lines() {
        assert!(parse_timestamp("mock-zkteco: client connected").is_none());
        assert!(parse_timestamp("").is_none());
    }
}
