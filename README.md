# FingerBridge (`fbsy`)

Native biometric attendance bridge — connects ZKTeco devices to HRMS via webhook.

Runs on a Windows or Linux machine inside the office LAN. Pulls attendance records from ZKTeco devices over TCP, maps them to HRMS events, and posts them to a webhook URL.

```
ZKTeco Device  ──TCP 4370──▶  fbsy (office machine)  ──HTTPS JSON──▶  HRMS API
```

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

## Quick start

```bash
# 1. Start the bridge — first run launches an interactive setup wizard
fbsy run at-bridge

# 2. Watch everything in a live dashboard (or `fbsy show` for a static snapshot)
fbsy dashboard

# 3. Pull attendance once on demand
fbsy at-bridge sync --once

# 4. Inspect / stop
fbsy logs at-bridge
fbsy close at-bridge
```

`fbsy dashboard` is a full-screen live monitor — ↑/↓ select a service, `s` start, `x` stop, `r` restart, `l` toggle the log pane, `q` quit. See [docs/INSTALL_FLOW.md](docs/INSTALL_FLOW.md) for the full install→run lifecycle.

Local testing without real hardware — spin up the mock device + HRMS servers:
```bash
fbsy run hrms              # mock HRMS webhook on :8800
fbsy run zkteco            # mock ZKTeco device on :4370
fbsy run at-bridge         # point its config at 127.0.0.1:8800 / 127.0.0.1:4370
```

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

Multiple devices are supported — add more objects to the `devices` array.

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

## CLI commands

```bash
fbsy                               # same as: fbsy doctor
fbsy doctor [--deep]               # readiness check; --deep tests live connections
fbsy setup                         # interactive first-time wizard
fbsy once [--device GATE-01]       # pull attendance once and exit
fbsy serve [--interval 120]        # run scheduler + local HTTP API

fbsy config validate               # validate config.json, exit 0 or 1
fbsy config show                   # print config with secrets redacted
fbsy config path                   # print path to config file

fbsy devices list                  # list configured devices
fbsy devices test GATE-01          # test TCP connection to one device
fbsy webhook test GATE-01          # test HRMS webhook for one device

fbsy logs path                     # print log directory

fbsy test-server device --port 14370 --records 5   # mock ZKTeco device
fbsy test-server hrms   --port 18800               # mock HRMS webhook
```

Backward-compatible aliases (parity with the Python bridge):
```bash
fbsy --setup
fbsy --once [--device GATE-01]
fbsy --interval 120
```

Full CLI reference: [docs/CLI.md](docs/CLI.md)

---

## Serve mode — HTTP API

When running `fbsy serve`, a local HTTP API listens on `127.0.0.1:7431`:

```
GET  /health                 — agent status, device states, last sync result
POST /sync                   — trigger sync for all devices
POST /sync?device=GATE-01    — trigger sync for one device
```

Example:
```bash
curl http://127.0.0.1:7431/health
curl -X POST http://127.0.0.1:7431/sync
```

---

## Safety rule

**Attendance records are never cleared from the device unless the HRMS webhook upload fully succeeded.**

`clearAttendanceAfterSync` defaults to `false`. Enable it only after verifying that sync works correctly end-to-end.

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

```bash
# Terminal 1 — mock HRMS webhook
fbsy test-server hrms --port 18800

# Terminal 2 — mock ZKTeco device
fbsy test-server device --port 14370 --records 5

# Terminal 3 — run sync against mocks
fbsy once --config config.mock.example.json
```

---

## Development

Requirements: Rust stable (1.75+)

```bash
git clone https://github.com/anonto42/fbsy
cd fbsy

# Install git hooks (runs fmt + clippy + tests on every commit)
bash scripts/install-hooks.sh

cargo build
cargo test
cargo run -- doctor
```

To build a release binary locally:
```bash
cargo build --release
# binary at: target/release/fbsy
```

Releases are built automatically by GitHub Actions on every version bump in `Cargo.toml`.

Full dev guide: [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md)

---

## Architecture

```
src/
├── cli/          — argument parsing (clap), command dispatch
├── config/       — BridgeConfig model, validation, defaults
├── domain/       — pure types: RawAttendance, HrmsEvent, FingerTemplate
├── ports/        — traits: DeviceClient, HrmsClient, ConfigStore
├── adapters/     — implementations: ZKTeco TCP, reqwest HRMS, JSON config file
├── application/  — use cases: sync_once, serve, doctor, setup
└── runtime/      — DeviceSyncState (per-device lock), job poller, scheduler
```

Architecture decision record: [docs/CODEBASE_ARCHITECTURE_DECISION.md](docs/CODEBASE_ARCHITECTURE_DECISION.md)

---

## Docs

| Document | What it covers |
|---|---|
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
