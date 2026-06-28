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
# 1. Start the bridge — first run launches an interactive setup wizard
fbsy run bridge

# 2. Watch everything in a live dashboard (or `fbsy show` for a static snapshot)
fbsy dashboard

# 3. Pull attendance once on demand
fbsy bridge sync --once

# 4. Make it survive reboots / power loss (runs `fbsy enable` once; needs admin)
fbsy enable bridge          # prints the exact `sudo …` command to run

# 5. Inspect / stop
fbsy logs bridge
fbsy close bridge
```

**Run on boot:** `fbsy run` starts *detached* processes that the OS kills on shutdown. To keep the bridge running across reboots and power cycles, `fbsy enable bridge` registers it with the OS init system (systemd / launchd / Windows scheduled task) so it auto-starts at boot and restarts on crash; `fbsy disable bridge` removes it. See [docs/USER_GUIDE.md](docs/USER_GUIDE.md) "Run on boot."

`fbsy dashboard` is a full-screen live monitor with both single-key shortcuts (↑/↓ select, `s`/`x`/`r` start/stop/restart, `y` sync, `l` logs, `q` quit) **and** a `:command` bar for the full vocabulary. See [docs/INSTALL_FLOW.md](docs/INSTALL_FLOW.md) for the full install→run lifecycle.

---

## How it works

`fbsy` manages three **services**, each of which runs as a **detached background process** (it survives closing the terminal) and writes a registry file so `fbsy show`/`dashboard` can track it:

| Service | What it is |
|---|---|
| `bridge` | The real bridge — pulls attendance from your devices and forwards to HRMS. This is the one you run in production. |
| `zkteco` | A mock ZKTeco device server (fake attendance) for local testing without hardware. |
| `hrms` | A mock HRMS webhook server that prints what it receives, for local testing. |

`fbsy run <service>` starts one; `fbsy show` / `fbsy dashboard` monitor them; `fbsy close <service>` stops one. Each service also has its own command group (e.g. `fbsy bridge sync`, `fbsy zkteco run -p 4370`).

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

### Install & lifecycle
```bash
fbsy install                 # copy binary to ~/.local/bin, set up PATH + data dirs
fbsy uninstall               # remove the binary (keeps your data dir)
```

### Service management
```bash
fbsy run bridge           # start the bridge (wizard on first run)
fbsy run zkteco [--name dev1 -p 4370 --records 5]   # start a mock device
fbsy run hrms   [--name hrmsA -p 8800]              # start a mock HRMS
fbsy show                    # table of all instances: instance / kind / status / port / uptime
fbsy dashboard               # live full-screen TUI (see below)
fbsy status <instance>       # detail for one instance
fbsy logs <instance> [-n 50] [--follow]  # tail an instance's log
fbsy close <instance>        # stop an instance
```
Running `fbsy` with no command is the same as `fbsy show`.

**Named instances — run a service more than once.** Each `run` accepts `--name` (default =
the kind's name), so you can run several of the same service on different ports and address
each by its name. This is how you simulate multiple devices locally to test a 2-device bridge:

```bash
fbsy run zkteco --name dev1 -p 4370     # mock device #1
fbsy run zkteco --name dev2 -p 4371     # mock device #2
fbsy show
#   INSTANCE  KIND     STATUS   PORT   ADDRESS
#   dev1      zkteco   running  4370   127.0.0.1:4370
#   dev2      zkteco   running  4371   127.0.0.1:4371
fbsy close dev2 ; fbsy logs dev1
```
The **bridge** itself is normally one instance handling N devices via its config `devices[]`
array (you point GATE-01→`127.0.0.1:4370`, GATE-02→`127.0.0.1:4371`), but it too can run
multiple named instances with `--name` + `--config` if you ever need separate bridges.

### `bridge` (the real bridge)
```bash
fbsy bridge run [--config PATH --interval N --no-poll]   # same as `fbsy run bridge`
fbsy bridge sync [--once] [--device GATE-01]             # pull attendance now, then exit
fbsy bridge config validate          # validate config.json (exit 0/1)
fbsy bridge config show              # print config with secrets redacted
fbsy bridge config path              # print the config path fbsy uses
fbsy bridge config setup             # (re)run the interactive setup wizard
fbsy bridge doctor [--deep] [--json] # readiness; --deep tests live device + webhook
fbsy bridge devices list             # list configured devices (no secrets)
fbsy bridge devices test GATE-01     # test TCP connection to one device
fbsy bridge devices info GATE-01     # read live device data (serial, firmware, counts)
fbsy bridge devices info GATE-01 --users   # also list enrolled users
fbsy bridge webhook test GATE-01     # send an empty batch to verify the webhook
```

### Mock servers (local testing)
```bash
fbsy zkteco run [-p 4370 --records 5]   # = fbsy run zkteco
fbsy hrms   run [-p 8800]               # = fbsy run hrms
```

Full CLI reference: [docs/CLI.md](docs/CLI.md)

---

## The live dashboard (`fbsy dashboard`)

A full-screen terminal UI that auto-refreshes and lets you control services — by single key or by typing a command:

```
┌ fbsy  service dashboard   —  : for command, q to quit ┐
│ SERVICE    STATUS   PID    PORT  UPTIME  DESCRIPTION   │
│ ▶ bridge   running  4821   7431  2m10s   attendance…   │  ← selected (cyan ▶)
│   zkteco   running  4830   4370  1m55s   mock device   │
│   hrms     stopped  -      -     -       mock HRMS      │  ← red = stopped
├ logs: bridge (running) ───────────────────────────────┤
│ ➡ Received HRMS Event Payload …                        │  ← live tail
├ commands ─────────────────────────────────────────────┤
│ ↑/↓ select  s start  x stop  r restart  y sync  l logs │
│ : command — start|stop|restart <svc> · sync · logs …   │
├───────────────────────────────────────────────────────┤
│ : start zkteco                                         │  ← command input
└───────────────────────────────────────────────────────┘
```

**Two ways to drive it:**
- **Single keys:** `Tab` switch focus (table ⇄ logs) · ↑/↓ (or j/k) select **or** scroll logs (depends on focus) · `s` start · `x` stop · `r` restart · `y` sync · `l` toggle+focus logs · **`a` all-instance logs** · `q` quit.
- **Command bar:** press `:` then type a full command — `start <kind>`, `stop|restart <instance>`, `sync [deviceCode]`, `logs <instance>|all`, `select <instance>`, `help`, `quit`. The available commands are always listed in the panel.

The dashboard lists **every running instance** (plus the default kinds as startable rows). The `a` key (or `logs all`) shows a **time-merged live tail of every running instance**, each line tagged `[instance]` — so you watch all services as one chronological stream. Press `Tab` (or `a`/`l`) to focus the log pane, then the arrow keys scroll it. Logs are structured (`<rfc3339> <LEVEL> [component] message`) and persist to per-instance files for diagnosis. Needs a real terminal (prints a hint if piped).

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

Spin up both mock services, point a config at them, and run a one-shot sync:

```bash
fbsy run hrms                      # mock HRMS webhook on :8800
fbsy run zkteco                    # mock ZKTeco device on :4370

# config.json with vpsWebhookUrl=http://127.0.0.1:8800/webhook
# and a device at deviceIp 127.0.0.1, devicePort 4370
fbsy bridge sync --once         # pulls from the mock device, forwards to mock HRMS

fbsy logs hrms                     # see the events the mock HRMS received
fbsy dashboard                     # watch all three services live
fbsy close zkteco && fbsy close hrms
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
cargo run -- show
cargo run -- bridge doctor
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

The service model: `fbsy run X` spawns a detached child that re-enters the binary through a hidden `__service-run` subcommand and runs the matching blocking loop (`serve` for the bridge, `test_server` for the mocks). The parent records a registry file and exits; `show`/`dashboard`/`close` operate on that registry plus a live process check.

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
