//! Secret redaction helpers.

/// Replace a secret value with a fixed marker before printing/logging.
pub fn redact(value: &str) -> String {
    if value.is_empty() {
        return String::new();
    }
    "***".to_string()
}
