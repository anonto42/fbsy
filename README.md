# FingerBridge (`fbsy`)

Native biometric attendance bridge, packaged as a small **service manager** — connects ZKTeco devices to HRMS via webhook.

Runs on a Windows, Linux, or macOS machine inside the office LAN. The `fbsy` binary installs itself, then starts/stops/monitors long-running **services** by name. The main service, `bridge`, pulls attendance from one or more ZKTeco devices over TCP, maps them to HRMS events, and posts them to your webhook URL.

```
GATE-01  ─┐
GATE-02  ─┼─TCP 4370─▶  fbsy bridge (office machine)  ─HTTPS JSON─▶  one HRMS webhook
FLOOR-3  ─┘
```

> **New here?** The [User Guide](docs/USER_GUIDE.md) is a complete first-user manual — install, every command, and how each piece works technically.

---

## Install

**Linux / macOS** — one line, detects your OS/arch, downloads the right binary, and sets up PATH:
```bash
curl -fsSL https://raw.githubusercontent.com/anonto42/fbsy/main/scripts/install.sh | sh
```

**Windows** (PowerShell):
```powershell
irm https://raw.githubusercontent.com/anonto42/fbsy/main/scripts/install.ps1 | iex
```

Then open a new shell and run `fbsy --help`.

The installer honors `FBSY_VERSION` (pin a version), `FBSY_INSTALL_DIR` (custom bin dir), and `FBSY_NO_VERIFY=1` (skip checksum). Prefer to inspect first? Download the script, read it, then run it.

### Manual download

Or grab the binary for your platform from the [Releases](../../releases) page and run `fbsy install` yourself:

| Platform | File |
|---|---|
| Windows 64-bit | `fbsy-windows-x86_64.exe` |
| Linux x86\_64 (Debian / Arch / Fedora) | `fbsy-linux-x86_64` |
| Linux ARM64 | `fbsy-linux-aarch64` |
| macOS Intel | `fbsy-macos-intel` |
| macOS Apple Silicon (M1/M2/M3) | `fbsy-macos-arm64` |

```bash
chmod +x fbsy-linux-x86_64        # Linux/macOS only
./fbsy-linux-x86_64 install       # copies to ~/.local/bin, sets up PATH + data dirs
```

---

## Updating

```bash
fbsy update --check     # report whether a newer release exists
fbsy update             # download + install it, restarting running services
```

`fbsy update` does a **safe, reversible swap**: check GitHub → download → verify SHA-256 →
smoke-test the new binary → back up the current one → replace → restart the services that
were running → health-check → **auto-rollback if anything fails**.

**Hands-off auto-update (opt-in):** set `autoUpdate: true` in `config.json` and the running
bridge checks every `updateCheckIntervalHours` (default 6) and applies new releases itself.

```jsonc
{ "autoUpdate": true, "updateCheckIntervalHours": 6 }
```

> **On "100% uptime":** a binary swap requires restarting the bridge, so there are a few
> seconds of downtime — literal 100% uptime isn't possible. But **no data is lost**:
> attendance stays buffered on the device and is never cleared until a successful HRMS upload,
> and your config/logs/registry live in the data dir untouched by an update.

---

## Quick start

```bash
# 1. Configure — HRMS webhook URL + your devices (install offers this too)
fbsy setup

# 2. Check the config
fbsy config

# 3. Start the bridge in the background
#    (with autoStartOnBoot in config it also survives reboots and crashes)
fbsy start

# 4. Day-to-day
fbsy status    # is it running? BOOT on?
fbsy logs -f   # watch it live
fbsy sync      # force one sync now, see pulled/forwarded counts
fbsy stop      # stop AND remove from boot
```

**Run on boot:** when `autoStartOnBoot` is `true` (the setup wizard's default), `fbsy start` registers a **per-user** boot unit — launchd LaunchAgent on macOS, `systemd --user` on Linux, a logon task on Windows — no sudo/Administrator needed. The OS starts the bridge at login and restarts it if it crashes; `fbsy stop` removes the unit again.

`fbsy dashboard` is an optional full-screen live monitor over the same functions (shortcuts `s`/`x`/`r` start/stop/restart, `y` sync, Tab logs, `q` quit, plus a typed command bar). Everything works headless without it.

---

## How it works

`fbsy start` runs the **bridge** as a background process (detached, or OS-supervised when `autoStartOnBoot` is set) and records a registry file so `fbsy status`/`dashboard` can track it. The bridge pulls attendance from every configured device on its own schedule and forwards to the HRMS webhook.

For testing without hardware, the same binary contains a mock ZKTeco device server and a mock HRMS webhook server (internal `__service-run` entrypoints) — see [docs/LOCAL-TEST-PLAN.md](docs/LOCAL-TEST-PLAN.md).

**Everything lives in one per-OS data directory** (created by `fbsy install`):

| OS | Data directory |
|---|---|
| Linux | `~/.config/fbsy/` |
| macOS | `~/Library/Application Support/fbsy/` |
| Windows | `%APPDATA%\fbsy\` |

with `config/config.json`, `logs/<service>.log`, and `run/<service>.json` (the pid/port registry) underneath.

---

## Configuration (`config.json`)

```jsonc
{
  "vpsWebhookUrl": "https://api.yourdomain.com/api/v1/biometric-devices/webhook",
  "bridgePort": 7431,
  "devices": [
    {
      "deviceIp": "192.168.1.100",
      "devicePort": 4370,
      "devicePassword": 0,
      "deviceTimeout": 15,
      "deviceOmitPing": true,
      "deviceCode": "GATE-01",
      "apiKey": "your-webhook-api-key",
      "organizationId": 1,
      "syncIntervalSeconds": 300,
      "clearAttendanceAfterSync": false
    }
  ]
}
```

**Multiple devices → one HRMS** is the normal case: add more objects to the `devices` array. All devices share the top-level `vpsWebhookUrl`, but each posts with its own `deviceCode` + `apiKey` + `organizationId`, has its own `syncIntervalSeconds`, and syncs on an independent schedule — one offline device never blocks the others. `deviceCode`s must be unique. The setup wizard's "Add another device?" prompt builds this for you.

Optional — enable HRMS job polling (push/pull templates):
```jsonc
{
  "hrmsBaseUrl": "https://app.yourdomain.com/api/v1",
  "hrmsApiToken": "device-token",
  "jobPollIntervalSeconds": 30
}
```

Full config reference: [docs/CONFIGURATION.md](docs/CONFIGURATION.md)

---

## Command reference

```bash
fbsy install       # copy binary to ~/.local/bin, set up PATH + data dirs; offers setup
fbsy uninstall     # remove the binary (--full also deletes all data, -y skips prompt)
fbsy update        # self-update from GitHub releases (--check to only report)

fbsy setup         # interactive wizard: HRMS webhook, devices, boot, auto-update
fbsy config        # validate config.json and print a redacted view

fbsy start         # start the bridge (installs the boot unit when autoStartOnBoot)
fbsy stop          # stop the bridge AND remove the boot unit
fbsy restart       # stop + start, preserving the supervised/detached mode
fbsy status        # table: running?, BOOT on/off, pid, uptime, port, address
fbsy sync          # pull + forward once, print JSON result (--device CODE, --config PATH)
fbsy logs          # print the bridge log (-n N lines, -f to follow)

fbsy dashboard     # optional full-screen TUI over the same functions
```

Full CLI reference: [docs/CLI.md](docs/CLI.md)

---

## The live dashboard (`fbsy dashboard`)

An optional page-based TUI: a **Home page** (logo, prompt card with live bridge status + last sync result, command palette), an **Output page** (full-height live execution log, opened with Tab or automatically when a command runs), and a **Help page** (`?`).

- **Single keys:** `s` start · `x` stop · `r` restart · `y` sync · `Tab` logs · `?` help · `q` quit · `Esc` back home (from home: quit).
- **Command bar:** just start typing (or press `:`) — `start`, `stop`, `restart`, `sync`, `logs`, `status`, `setup`, `home`, `help`, `install`, `uninstall`, `update`, `quit`. Results appear on the status line; `setup`/`install`/`uninstall`/`update` temporarily suspend the TUI and run attached.

Logs are structured (`<rfc3339> <LEVEL> [component] message`) and persist to files under the data dir. Needs a real terminal (prints a hint if piped).

---

## bridge HTTP API

While `bridge` is running it also exposes a local HTTP API on `127.0.0.1:<bridgePort>` (default `7431`):

```
GET  /health                 — agent status, device states, last sync result
POST /sync                   — trigger a sync for all devices
POST /sync?device=GATE-01    — trigger a sync for one device
```

```bash
curl http://127.0.0.1:7431/health
curl -X POST http://127.0.0.1:7431/sync
```

---

## Safety rule

**Attendance records are never cleared from the device unless the HRMS webhook upload fully succeeded.**

`clearAttendanceAfterSync` defaults to `false`. Enable it only after verifying that sync works correctly end-to-end.

---

## Non-functional requirements (NFRs)

These are measurable targets, not aspirations. They define "good enough" for a production office deployment.

| NFR | Target | Notes |
|---|---|---|
| **Binary size** | ≤ 15 MB stripped | Statically linked musl; no JVM/Python runtime |
| **Memory** | ≤ 50 MB RSS after 7-day soak | Thread-per-device; no heap growth by design |
| **Sync p99 latency** | ≤ 10 s per device | Network + ZKTeco protocol round-trips dominate |
| **RPO (data loss)** | Zero | Safety invariant: clear only after confirmed upload |
| **Availability** | Self-healing via service restart; survives reboot with OS autostart | |
| **Webhook retry** | 3 retries; exponential backoff (2 s × attempt) + up to 500 ms jitter | Retries 429 and 5xx; never retries 4xx |
| **Log disk usage** | Capped; rotates at 5 MB, keeps 5 files per service | Unbounded growth prevented |
| **HTTP API** | Loopback-only (127.0.0.1); max 8 KB headers; 400 on malformed | No auth needed at loopback |

---

## How data flows

```
1. Connect to ZKTeco device over TCP (with optional password auth)
2. Query record count via CMD_GET_FREE_SIZES
3. Pull attendance via CMD_PREPARE_BUFFER → CMD_DATA / CMD_PREPARE_DATA chunks
4. Decode records (8-byte, 16-byte, or 40-byte format depending on firmware)
5. Map punch codes → check_in / check_out  (0 or 4 = check_in, else check_out)
6. POST events in 500-record batches to HRMS webhook (retry on 429 / 5xx)
7. If upload succeeded AND clearAttendanceAfterSync = true → CMD_CLEAR_ATTLOG
```

---

## Local testing (no real device needed)

The binary contains mock servers for a fully local proof — staged plan with
visible attendance data in [docs/LOCAL-TEST-PLAN.md](docs/LOCAL-TEST-PLAN.md):

```bash
fbsy __service-run zkteco --port 14370 --records 5 &   # mock fingerprint device
fbsy __service-run hrms   --port 18800 &               # mock HRMS webhook server

# config.json: vpsWebhookUrl=http://127.0.0.1:18800/webhook,
#              device at 127.0.0.1:14370
fbsy sync                            # → ok: true, pulled 5, forwarded 5
curl -s http://127.0.0.1:18800/events   # the punches, visible on the HRMS side
```

---

## Development

Requirements: Rust stable (1.75+)

```bash
git clone https://github.com/anonto42/fbsy
cd fbsy

# Install git hooks (runs fmt + clippy + tests, and auto-bumps the version on every commit)
bash scripts/install-hooks.sh

cargo build
cargo test
cargo run -- status
cargo run -- config
```

To build a release binary locally:
```bash
cargo build --release
# binary at: target/release/fbsy
```

The pre-commit hook auto-increments the patch version in `Cargo.toml` (skip with `FBSY_NO_BUMP=1`), and GitHub Actions publishes a cross-platform release on every push with a new version — so each push ships binaries automatically.

Full dev guide: [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md)

---

## Architecture

```
src/
├── cli/          — argument parsing (clap), command dispatch
├── services/     — ServiceKind: the bridge / zkteco / hrms identities
├── config/       — BridgeConfig model, validation, defaults
├── domain/       — pure types: RawAttendance, HrmsEvent, FingerTemplate
├── ports/        — traits: DeviceClient, HrmsClient, ConfigStore
├── adapters/     — implementations: ZKTeco TCP+UDP, reqwest HRMS, JSON config file
├── application/  — use cases: install, service (run/show/close/status/logs),
│                   dashboard (ratatui TUI), serve, sync_once, doctor, setup, test_server
└── runtime/      — DeviceSyncState (per-device lock + safety rule), registry
                    (pid/port files), process (detached spawn / liveness / kill),
                    job poller, scheduler
```

The service model: `fbsy start` spawns a detached child that re-enters the binary through a hidden `__service-run` subcommand and runs the blocking bridge loop (`serve`) — or, with `autoStartOnBoot`, installs a per-user OS unit that runs `__service-supervised` in the foreground under launchd/systemd/schtasks. Either way the process self-registers so `status`/`logs`/`stop` operate on the registry plus a live process check.

Architecture decision record: [docs/CODEBASE_ARCHITECTURE_DECISION.md](docs/CODEBASE_ARCHITECTURE_DECISION.md)

---

## Docs

| Document | What it covers |
|---|---|
| [**PRODUCTION_PLAN.md**](docs/PRODUCTION_PLAN.md) | **Production build plan & cold-handoff spec** — invariants, current state, gap analysis, and an ordered, executable checklist to reach production-grade. Start here to continue the project. |
| [USER_GUIDE.md](docs/USER_GUIDE.md) | Full first-user manual: install, every command, and how each piece works technically |
| [INSTALL_FLOW.md](docs/INSTALL_FLOW.md) | End-to-end install → setup → run → dashboard lifecycle |
| [ARCHITECTURE.md](docs/ARCHITECTURE.md) | Layer diagram, module responsibilities |
| [CLI.md](docs/CLI.md) | All commands, flags, and examples |
| [CONFIGURATION.md](docs/CONFIGURATION.md) | Every config field with defaults and valid ranges |
| [DEVELOPMENT.md](docs/DEVELOPMENT.md) | Build, test, hooks, workflow |
| [TESTING.md](docs/TESTING.md) | Test strategy, mock servers, integration tests |
| [SECURITY.md](docs/SECURITY.md) | Safety invariants, secret redaction, threat model |
| [MIGRATION_FROM_PYTHON.md](docs/MIGRATION_FROM_PYTHON.md) | Differences from the Python zkteco-bridge |
| [PACKAGING.md](docs/PACKAGING.md) | Release CI, binary naming, checksums |
| [CODE_WALKTHROUGH.md](docs/CODE_WALKTHROUGH.md) | Trace through the code from main.rs |
