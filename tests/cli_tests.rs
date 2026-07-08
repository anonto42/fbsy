//! Basic CLI smoke tests — assert help text, subcommand presence, and clean
//! exit codes. Does not require a config file or live devices.

use std::process::Command;

fn fbsy() -> Command {
    Command::new(env!("CARGO_BIN_EXE_fbsy"))
}

// ── Help / top-level ──────────────────────────────────────────────────────────

#[test]
fn help_exits_zero() {
    let out = fbsy().arg("--help").output().expect("run fbsy --help");
    assert!(out.status.success(), "fbsy --help should exit 0");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("fbsy") || stdout.contains("fingerbridge"),
        "help text should mention the binary name"
    );
}

#[test]
fn version_flag_exits_zero() {
    let out = fbsy()
        .arg("--version")
        .output()
        .expect("run fbsy --version");
    assert!(out.status.success(), "fbsy --version should exit 0");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(env!("CARGO_PKG_VERSION")),
        "version output should include package version"
    );
}

#[test]
fn unknown_subcommand_exits_nonzero() {
    let out = fbsy()
        .arg("no-such-command-xyz")
        .output()
        .expect("run fbsy with unknown subcommand");
    assert!(
        !out.status.success(),
        "unknown subcommand should exit non-zero"
    );
}

// ── update subcommand ─────────────────────────────────────────────────────────

#[test]
fn update_check_help_exits_zero() {
    let out = fbsy()
        .args(["update", "--help"])
        .output()
        .expect("run fbsy update --help");
    assert!(out.status.success());
}
