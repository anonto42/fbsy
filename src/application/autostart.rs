//! Autostart use cases.
//!
//! The current implementation is a placeholder. Later this will call OS-specific
//! adapters for Windows Task Scheduler, systemd, or launchd.

use anyhow::Result;

/// Install automatic startup integration.
pub fn install() -> Result<()> {
    println!("Autostart install is planned for the Windows adapter.");
    Ok(())
}

/// Remove automatic startup integration.
pub fn uninstall() -> Result<()> {
    println!("Autostart uninstall is planned for the Windows adapter.");
    Ok(())
}
