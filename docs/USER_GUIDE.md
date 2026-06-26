# fbsy — User Guide

A complete, first-user walkthrough of **fbsy** (FingerBridge): install it, run every
piece of functionality, and understand what happens technically at each step.

`fbsy` is a small **service manager** for a biometric attendance bridge. One binary
installs itself, then starts/stops/monitors long-running **services** by name. The main
service, `bridge`, pulls attendance from one or more ZKTeco devices over TCP and
forwards it to your HRMS webhook.

```
GATE-01  ─┐
GATE-02  ─┼─TCP 4370─▶  fbsy bridge (office machine)  ─HTTPS JSON─▶  one HRMS webhook
FLOOR-3  ─┘
```

---

## 1. Mental model (read this first)

Three ideas explain everything else:

1. **Services.** `fbsy` manages three of them:
   | Service | Role |
   |---|---|
   | `bridge` | the real bridge — device → HRMS. Run this in production. |
   | `zkteco` | a mock ZKTeco device (fake attendance) for testing without hardware. |
   | `hrms` | a mock HRMS webhook that prints what it receives, for testing. |

2. **Detached processes + a registry.** `fbsy run <service>` launches the service as a
   **detached background process** (it survives closing the terminal) and writes a small
   JSON **registry file** describing it. `show`, `dashboard`, `status`, and `close` all
   operate on that registry plus a live check of whether the process is still alive.
   Add `--name <instance>` when you want more than one copy of the same service, for example
   two mock ZKTeco devices on different ports.

3. **One per-OS data directory** holds everything:
   | OS | Base directory |
   |---|---|
   | Linux | `~/.config/fbsy/` |
   | macOS | `~/Library/Application Support/fbsy/` |
   | Windows | `%APPDATA%\fbsy\` |

   Inside it: `config/config.json`, `logs/<instance>.log`, `run/<instance>.json` (the registry).

---

## 2. Install

**Linux / macOS:**
```bash
curl -fsSL https://raw.githubusercontent.com/anonto42/fbsy/main/scripts/install.sh | sh
```
**Windows (PowerShell):**
```powershell
irm https://raw.githubusercontent.com/anonto42/fbsy/main/scripts/install.ps1 | iex
```
Open a new shell, then `fbsy --help`.

**What the install script does, in order (and what it touches):**

| # | Action | Touches |
|---|---|---|
| 1 | Detect OS + arch (`uname` / `$PROCESSOR_ARCHITECTURE`) → choose the right asset (`fbsy-linux-x86_64`, `fbsy-macos-arm64`, `fbsy-windows-x86_64.exe`, …) | nothing |
| 2 | Download from `github.com/anonto42/fbsy/releases/latest/download/<asset>` (public redirect — no token, no API). Pin with `FBSY_VERSION=0.2.x` | one download |
| 3 | Verify SHA-256 against `checksums.txt` (skip with `FBSY_NO_VERIFY=1`) | nothing |
| 4 | `chmod +x`; on macOS strip the `com.apple.quarantine` xattr (Gatekeeper) | the temp file |
| 5 | Move the binary to `~/.local/bin/fbsy` (Windows: `%LOCALAPPDATA%\Programs\fbsy\fbsy.exe`) | your user bin dir |
| 6 | Run `fbsy install` → create data dirs + add bin dir to PATH (shell rc / User PATH) | `~/.config/fbsy/`, one shell-rc line |

No root is required at any point.

**Env overrides:** `FBSY_VERSION` (pin a version), `FBSY_INSTALL_DIR` (custom bin dir),
`FBSY_NO_VERIFY=1` (skip checksum).

**Manual alternative:** download the binary from the Releases page, `chmod +x`, then run
`./fbsy install` yourself — it does steps 5–6.

---

## 3. First-run walkthrough (every command, in order)

### Step 1 — Discover commands
```bash
fbsy --help
```
Shows the surface: lifecycle (`install`/`uninstall`), service control
(`run`/`show`/`dashboard`/`status`/`logs`/`close`), and the three services as their own
command groups (`bridge`, `zkteco`, `hrms`). Running `fbsy` with no command = `fbsy show`.

### Step 2 — Start the mock services (testing without hardware)
```bash
fbsy run hrms      # mock HRMS webhook on :8800
fbsy run zkteco    # mock ZKTeco device on :4370
```
Multiple mock devices can run side by side by giving each one a unique instance name and port:
```bash
fbsy run zkteco --name dev1 -p 4370 --records 3
fbsy run zkteco --name dev2 -p 4371 --records 8
fbsy status dev1
fbsy logs dev2 -n 20
fbsy close dev1
```
Each prints `✔ <svc> started (pid …)` and writes `run/<svc>.json`, e.g.:
```json
{ "service": "hrms", "kind": "hrms", "pid": 5517, "port": 8800,
  "args": ["--port","8800"], "startedAt": "2026-…Z", "exe": "/…/fbsy" }
```
*Technically:* `fbsy` re-executes itself with a hidden `__service-run <svc>` subcommand as
a **detached child** (Unix `setsid`, Windows `DETACHED_PROCESS`), redirecting the child's
output to `logs/<instance>.log`. The parent records the instance name plus service kind
and exits; the child keeps running.

### Step 3 — Configure the bridge
On a real first run, `fbsy run bridge` launches an **interactive wizard** that asks for
the webhook URL, device IP/port, deviceCode, apiKey, etc., and writes
`config/config.json`. You can also run the wizard directly:
```bash
fbsy bridge config setup       # interactive wizard
fbsy bridge config validate    # check the config (exit 0/1)
fbsy bridge config show        # print config with secrets redacted (apiKey → ***)
fbsy bridge config path        # print the config file path
```
A minimal config for the mock setup (what the wizard produces):
```json
{
  "vpsWebhookUrl": "http://127.0.0.1:8800/webhook",
  "bridgePort": 7431,
  "devices": [
    { "deviceIp": "127.0.0.1", "devicePort": 4370, "deviceCode": "MOCK-GATE-01",
      "apiKey": "mock-key", "organizationId": 1, "syncIntervalSeconds": 300,
      "clearAttendanceAfterSync": false }
  ]
}
```

### Step 4 — Start the bridge
```bash
fbsy run bridge
```
- **No config yet** → offers the setup wizard.
- **Configured** → spawns the bridge detached on `bridgePort` (7431) and records its registry entry.
- **Already running** → prints status instead of starting a second copy.

The running bridge: schedules a sync per device, runs the optional HRMS job poller, and
exposes a local HTTP API (see §5).

### Step 5 — Watch the pipeline
```bash
fbsy show                       # table: SERVICE STATUS PID PORT UPTIME
fbsy bridge sync --once      # pull attendance now, then exit
fbsy logs hrms -n 20            # see what the HRMS received
fbsy status bridge           # detail for one service
```
A successful one-shot sync prints:
```json
{ "ok": true, "deviceCode": "MOCK-GATE-01", "pulled": 5, "forwarded": 5,
  "deviceAttendanceCleared": false, "startedAt": "2026-…Z",
  "message": "forwarded 5 event(s)" }
```
and the HRMS log shows the forwarded events:
```json
{ "deviceEmployeeId": "1005", "eventType": "check_in",
  "timestamp": "2026-…Z", "verificationMethod": "fingerbridge" }
```

### Step 6 — The live dashboard
```bash
fbsy dashboard
```
A full-screen TUI (needs a real terminal). It has **two ways to drive it**:

**Single keys:**
| Key | Action |
|---|---|
| ↑/↓ or k/j | move selection |
| s | start selected service |
| x | stop selected service |
| r | restart selected service |
| y | sync now (bridge) |
| l | toggle the log pane (live tail of the selected service) |
| a | toggle combined logs from all running instances |
| q / Esc | quit (restores the terminal cleanly) |

**Command bar:** press `:` then type a command, Enter to run, Esc to cancel:
| Command | Action |
|---|---|
| `start <bridge\|zkteco\|hrms>` | start the default instance of a service kind |
| `stop\|restart <instance>` | control a named instance, e.g. `dev1` |
| `sync [deviceCode]` | run a sync (all devices, or one) |
| `logs <instance>` / `logs all` | focus one log or the combined log view |
| `select <instance>` | move the selection |
| `help` | list commands |
| `quit` | exit |

The available commands are always shown in the dashboard's **commands** panel. It
auto-refreshes every 250 ms from the **same registry + liveness check** that `fbsy show`
uses — it is not a separate implementation.

### Step 7 — Logs, status, stop
```bash
fbsy logs <instance> [-n 50] [--follow]   # tail a service log (follow = live)
fbsy status <instance>                     # one service instance's status + log path
fbsy close <instance>                      # stop an instance (SIGTERM + clear registry)
```

### Step 8 — Uninstall
```bash
fbsy uninstall            # removes the binary, KEEPS ~/.config/fbsy
```
Full wipe: also `rm -rf ~/.config/fbsy` and remove the `# added by fbsy install` line from
your shell rc.

---

## 4. Full command reference

```
fbsy install | uninstall

fbsy run <bridge|zkteco|hrms> [service flags]
fbsy show
fbsy dashboard
fbsy status <instance>
fbsy logs <instance> [-n N] [--follow]
fbsy close <instance>

fbsy bridge run [--name NAME] [--config PATH --interval N --no-poll]
fbsy bridge sync [--once] [--device CODE] [--config PATH]
fbsy bridge config <validate|show|path|setup>
fbsy bridge doctor [--deep] [--json] [--config PATH]
fbsy bridge devices <list | test CODE>
fbsy bridge webhook test CODE

fbsy zkteco run [--name NAME] [-p 4370 --records 5]
fbsy hrms   run [--name NAME] [-p 8800]
```
`fbsy run bridge` and `fbsy bridge run` are the same thing.

---

## 5. How it works technically

### Detached process model
`fbsy run X --name NAME` → `spawn_detached`: re-exec `current_exe __service-run X [args]`
with stdin null and stdout/stderr → `logs/NAME.log`. Unix uses `setsid` (the child joins a new session,
gets reparented to init, survives the shell); Windows uses `DETACHED_PROCESS |
CREATE_NO_WINDOW`. The parent writes `run/NAME.json` and exits immediately. The child runs
the service kind's real blocking loop (`serve` for the bridge, the mock server for
`zkteco`/`hrms`).

### Registry & liveness
Each instance has one `run/<instance>.json` with `service` (instance name), `kind`, `pid`,
`port`, `args`, `startedAt`, and `exe`.
`show`/`dashboard`/`status` check liveness with `sysinfo` and compare the running process's
**exe name** to the recorded one (guards against PID reuse). If a recorded process is gone,
the entry is treated as `stopped` and **auto-cleared**.

### The sync pipeline (the core job)
For each device, on its schedule (or `sync --once`):
1. Connect to the device over TCP (or UDP if `deviceForceUdp`), with password auth if needed.
2. Read the record count, then pull attendance (handles the multi-packet ZKTeco buffer protocol).
3. Decode records (8-, 16-, or 40-byte formats depending on firmware).
4. Map punch codes → `check_in` (0 or 4) / `check_out` (else).
5. POST events to `vpsWebhookUrl` in **500-record batches**, retrying on network/429/5xx.
6. **Safety rule:** the device's attendance is **cleared only if** the upload fully
   succeeded **and** `clearAttendanceAfterSync` is `true`. A failed upload never clears.

### Multiple devices → one HRMS
All devices in `config.json` share the top-level `vpsWebhookUrl`, but each posts with its
own `deviceCode` + `apiKey` + `organizationId` and runs on its **own scheduler thread** at
its own `syncIntervalSeconds`. One offline device never blocks the others. `deviceCode`s
must be unique.

### bridge HTTP API
While `bridge` runs it serves a local API on `127.0.0.1:<bridgePort>` (default 7431):
```
GET  /health                 — agent + per-device status + last sync result
POST /sync                   — trigger a sync for all devices
POST /sync?device=GATE-01    — trigger a sync for one device
```

### HRMS job poller (optional)
If `hrmsBaseUrl` + `hrmsApiToken` are set, the bridge polls HRMS every
`jobPollIntervalSeconds` for pending jobs (`PUSH_USER`, `PULL_TEMPLATES`) and reports
results — so the server can push to devices without any inbound connection to your LAN.

---

## 6. Configuration reference (`config.json`)

**Top-level (shared):**
| Field | Meaning | Default |
|---|---|---|
| `vpsWebhookUrl` | HRMS webhook all devices post to | required |
| `bridgePort` | local HTTP API port | 7431 |
| `hrmsBaseUrl` / `hrmsApiToken` | optional job-poller endpoint + token | none |
| `jobPollIntervalSeconds` | poll cadence | 30 |
| `devices[]` | one or more device blocks | required |

**Per device:**
| Field | Meaning | Default |
|---|---|---|
| `deviceIp` / `devicePort` | device address | — / 4370 |
| `deviceCode` | unique id sent to HRMS | required |
| `apiKey` | per-device webhook key | required |
| `organizationId` | org id sent to HRMS | 1 |
| `devicePassword` | device comm password | 0 |
| `deviceTimeout` | connect timeout (s) | 15 |
| `deviceForceUdp` | use UDP instead of TCP | false |
| `syncIntervalSeconds` | per-device schedule | 300 |
| `clearAttendanceAfterSync` | clear device after a **successful** upload | false |

`config show` redacts `deviceCode` and `apiKey` as `***`.

**Self-update fields (top-level):**
| Field | Meaning | Default |
|---|---|---|
| `autoUpdate` | running bridge auto-installs newer releases | false |
| `updateCheckIntervalHours` | how often it checks GitHub | 6 |

---

## 6b. Updating fbsy

```bash
fbsy update --check     # is a newer release available?
fbsy update             # install it (restarts running services)
```

The update is a **safe, reversible swap**, with a diagnosis line per step:
1. Check the latest release (via the GitHub `latest/download` redirect — no API/token).
2. Download the matching asset; verify its SHA-256 against `checksums.txt`.
3. Smoke-test the new binary (`--version` must report the expected version).
4. Back up the current binary to `~/.config/fbsy/update/fbsy-backup`.
5. Replace the running binary atomically.
6. Restart the services that were running (terminate old by pid → respawn from new binary).
7. Health-check (each service alive; `bridge` answers `/health`).
8. **Auto-rollback** to the backup if any of 6–7 fail.

**Auto-update (opt-in):** with `autoUpdate: true`, the running bridge checks every
`updateCheckIntervalHours` and, when a newer release exists, launches a detached
`fbsy update --auto` that performs the same safe swap — including restarting the bridge itself.

**Uptime / data safety:** the swap restarts the bridge, so expect a few seconds of downtime —
not literal 100% uptime. **No data is lost:** attendance is buffered on the device and never
cleared until a successful HRMS upload, and config/logs/registry are untouched by an update.

---

## 7. Troubleshooting & known behaviors

- **Dashboard needs a terminal.** Piping it (`fbsy dashboard | cat`) prints a hint and
  exits; use `fbsy show` for a snapshot.
- **Port already in use.** Two services can't share a port. If a port is taken, the
  detached child fails to bind and exits; `fbsy show` then reports it `stopped` and clears
  the entry. Check `fbsy logs <svc>` for the bind error, then start it on a free port
  (`fbsy run zkteco -p 4371`).
- **macOS Gatekeeper / Windows SmartScreen.** Unsigned binaries may be blocked on first
  run. The install script clears the macOS quarantine xattr; a permanent fix is
  code-signing/notarization.
- **`clearAttendanceAfterSync`.** Leave it `false` until you've verified end-to-end; once
  enabled, device logs are wiped after each successful upload.
- **Stale registry self-heals.** If a service dies unexpectedly, the next `show`/`dashboard`
  marks it `stopped` and removes its registry file.

---

## 8. Teardown / clean slate

```bash
fbsy close bridge && fbsy close zkteco && fbsy close hrms   # stop everything
fbsy uninstall                                                 # remove binary, keep data
rm -rf ~/.config/fbsy                                          # full wipe (optional)
```
