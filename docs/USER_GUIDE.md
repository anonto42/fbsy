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

1. **Services.** `fbsy` manages four of them:
   | Service | Role |
   |---|---|
   | `bridge` | the real bridge — device → HRMS. Run this in production. |
   | `scanner` | discovers likely ZKTeco attendance devices on the LAN. |
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
(`run`/`show`/`dashboard`/`status`/`logs`/`close`), and the services as their own
command groups (`bridge`, `scanner`, `zkteco`, `hrms`). Running `fbsy` with no command =
`fbsy show`.

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
`fbsy show`, `fbsy status`, and the dashboard display the machine's LAN IP for mock
`zkteco` and `hrms` services, for example `192.168.1.24:4370`. That is the address
other machines/devices on the same router should use. On the same machine, `127.0.0.1`
still works.
The mock servers print a **Setup wizard values** block. For the first mock device,
use:
```text
Device unique code:  MOCK-GATE-01
Device HRMS API key: mock-key
Organization ID:    1
```
`deviceCode` is not a hardware serial number. It is the identifier that FingerBridge
sends to HRMS so HRMS knows which configured biometric device produced the attendance.
Each prints `✔ <svc> started (pid …)` and writes `run/<svc>.json`, e.g.:
```json
{ "service": "hrms", "kind": "hrms", "pid": 5517, "port": 8800,
  "args": ["--port","8800"], "startedAt": "2026-…Z", "exe": "/…/fbsy" }
```
*Technically:* `fbsy` re-executes itself with a hidden `__service-run <svc>` subcommand as
a **detached child** (Unix `setsid`, Windows `DETACHED_PROCESS`), redirecting the child's
output to `logs/<instance>.log`. The parent records the instance name plus service kind
and exits; the child keeps running.

### Step 3 — Discover real attendance devices
```bash
fbsy scanner scan                         # scan this machine's LAN /24 on port 4370
fbsy scanner scan --host 192.168.1.50     # scan one known IP
fbsy scanner scan --cidr 192.168.1.0/24   # scan a specific subnet
fbsy scanner scan --json                  # machine-readable output
```
The scanner first checks whether port `4370` is open, then tries the ZKTeco protocol.
When a device responds, it prints IP, firmware, serial, user/template/attendance counts,
and a suggested device config block you can copy into the setup wizard values.

Run it as a background service when you want repeated discovery logs:
```bash
fbsy run scanner --interval 300
fbsy logs scanner -n 100
```

### Step 4 — Configure the bridge
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
If the bridge runs on a different machine than the mock HRMS/device, use the LAN IP
shown by `fbsy show` instead of `127.0.0.1`.

### Step 5 — Start the bridge
```bash
fbsy run bridge
```
- **No config yet** → offers the setup wizard.
- **Configured** → spawns the bridge detached on `bridgePort` (7431) and records its registry entry.
- **Already running** → prints status instead of starting a second copy.

The running bridge: schedules a sync per device, runs the optional HRMS job poller, and
exposes a local HTTP API (see §5).

### Step 6 — Watch the pipeline
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

### Step 7 — The live dashboard
```bash
fbsy dashboard
```
A full-screen TUI (needs a real terminal). It has **two ways to drive it**:

**Focus model:** `Tab` switches focus between the **service table** and the **log pane**.
When the table is focused, ↑/↓ move the selection; when the logs are focused, ↑/↓ scroll
them. The focused pane's border brightens so you always know what the arrows will do.

**Single keys:**
| Key | Action |
|---|---|
| Tab | switch focus: service table ⇄ log pane |
| ↑/↓ or k/j | table focus: move selection · log focus: scroll logs |
| s | start selected service |
| x | stop selected service |
| r | restart selected service |
| y | sync now (bridge) |
| l | toggle the log pane and focus it |
| a | combined logs from all running instances, **time-merged** and tagged `[instance]` |
| PgUp / PgDn | scroll the log pane older / newer |
| Home / End | jump to oldest / newest loaded log lines |
| Esc | log focus → table · table focus → quit |
| ? | help overlay |
| q | quit (restores the terminal cleanly) |

**Command bar:** press `:` then type a command, Enter to run, Esc to cancel:
| Command | Action |
|---|---|
| any CLI command without `fbsy` | `show`, `bridge doctor --json`, `bridge devices info CODE --users`, etc. |
| `bridge config setup` | suspends the TUI for the interactive wizard, then resumes |
| `start <kind> [flags]` | alias for `run <kind> [flags]` |
| `stop <instance>` | alias for `close <instance>` |
| `restart <instance>` | dashboard restart helper |
| `sync [deviceCode]` | alias for `bridge sync --once [--device CODE]` |
| `logs all` | combined running-instance log view |
| `select <instance>` | move the highlight |
| `help` / `?` | open help |
| `quit` | exit |

Report-style commands are captured into a scrollable overlay. Interactive commands
such as `bridge config setup`, `install`, and prompted `update` temporarily return to
the real terminal, then resume the dashboard. The dashboard auto-refreshes every
250 ms from the **same registry + liveness check** that `fbsy show` uses.

Example 2-device test from inside the dashboard:
```text
:start zkteco --name dev1 --port 4370
:start zkteco --name dev2 --port 4371
:start hrms
:start scanner
:bridge config setup
:start bridge
:sync
:logs all
```

### Step 8 — Logs, status, stop
```bash
fbsy logs <instance> [-n 50] [--follow]   # tail a service log (follow = live)
fbsy status <instance>                     # one service instance's status + log path
fbsy close <instance>                      # stop an instance (SIGTERM + clear registry)
```
**Structured logs (observability).** Every service writes timestamped, leveled, tagged
lines to its durable per-instance log file (append mode — they survive restarts for
after-the-fact diagnosis). The shape is `<rfc3339> <LEVEL> [component] message`, e.g.:
```text
2026-06-27T05:31:49.089Z INFO  [sync MOCK-GATE-01] device responded: connected
2026-06-27T05:31:49.089Z INFO  [sync MOCK-GATE-01] device returned 5 attendance record(s)
2026-06-27T05:31:49.091Z INFO  [sync MOCK-GATE-01] forwarded 5/5 event(s) to HRMS → ok
2026-06-27T05:31:49.091Z INFO  [sync MOCK-GATE-01] sync done: ok=true pulled=5 forwarded=5 cleared=false
```
Because every line is timestamped, the dashboard's `a` view interleaves all running
services into one chronological stream. Grep for problems with `grep ' ERROR ' <log>`.
The bridge trail covers: device call → connected/failed, records pulled, mapped HRMS
event count, HRMS forward ok/failed, clear/keep decision, and the final sync result.
Mock ZKTeco logs show protocol commands served; mock HRMS logs show request paths and
webhook event counts. See `docs/LOGGING_CHECKLIST.md` for the full manual test checklist.

### Step 8.5 — Run on boot (survive reboots / power loss)

`fbsy run` / the dashboard start **detached** processes — handy, but the OS kills them on
shutdown and **nothing brings them back after a reboot**. For an unattended attendance device,
register the bridge with the OS init system so it **auto-starts at boot and restarts on crash**:

```bash
fbsy enable bridge      # prints the exact `sudo …` command (needs admin once)
sudo fbsy enable bridge --config /home/<you>/.config/fbsy/config/config.json
```
Run `fbsy enable bridge` **without** sudo first — it prints the precise elevated command with the
right absolute `--config` baked in (so the boot service uses *your* config, not root's). Then:

| OS | What `enable` installs | Inspect | Disable |
|---|---|---|---|
| Linux | systemd unit `/etc/systemd/system/fbsy-bridge.service` (`Restart=always`, runs as you) | `systemctl status fbsy-bridge` · `journalctl -u fbsy-bridge` | `sudo fbsy disable bridge` |
| macOS | LaunchDaemon `/Library/LaunchDaemons/com.fbsy.bridge.plist` (`RunAtLoad`, `KeepAlive`) | `sudo launchctl list \| grep com.fbsy` | `sudo fbsy disable bridge` |
| Windows | Scheduled task `fbsy-bridge` (ONSTART, SYSTEM) — run from an **Administrator** PowerShell | `schtasks /query /tn fbsy-bridge` | `fbsy disable bridge` (Administrator) |

The boot service runs the bridge in the foreground under the init system and **self-registers**,
so `fbsy show` (see the **BOOT** column), `fbsy status bridge` (the **On boot** line), and
`fbsy logs bridge` all keep working exactly as before. The structured sync trail is redirected to
the same per-instance log file. To verify without a full reboot: `sudo systemctl restart
fbsy-bridge` then `fbsy show` — the bridge comes back on its own.

> Tip: only the **bridge** (and optionally **scanner**) are production daemons; the mock device
> and HRMS servers are for testing and aren't meant to run on boot.

### Step 9 — Uninstall
```bash
fbsy uninstall            # removes the binary, KEEPS ~/.config/fbsy
```
Linux/macOS delete the installed binary immediately. Windows schedules removal after
the command exits because a running `.exe` is locked by the OS. Full wipe: also remove
the data directory and the PATH entry manually.

---

## 4. Full command reference

```
fbsy install | uninstall

fbsy run <bridge|scanner|zkteco|hrms> [--name NAME] [service flags] # --name = run >1 instance
fbsy show
fbsy dashboard
fbsy status <instance>
fbsy logs <instance> [-n N] [--follow]
fbsy close <instance>
fbsy update [--check]                                         # self-update

fbsy bridge run [--name NAME] [--config PATH --interval N --no-poll]
fbsy bridge sync [--once] [--device CODE] [--config PATH]
fbsy bridge config <validate|show|path|setup>
fbsy bridge doctor [--deep] [--json] [--config PATH]
fbsy bridge devices <list | test CODE | info CODE [--users]>
fbsy bridge webhook test CODE

fbsy scanner scan [--cidr CIDR | --host IP] [-p 4370] [--json] [--include-open]
fbsy scanner run [--name NAME] [--interval N] [scan flags]

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
On Windows, replace the last line with deleting `%APPDATA%\fbsy` after the command exits.
