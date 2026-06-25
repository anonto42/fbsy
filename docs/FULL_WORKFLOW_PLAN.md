# Full Software Workflow Plan

This document defines the complete intended behavior for the Rust ZKTeco Bridge, based on the existing Python bridge in `~/developer_workspace/projects/levelaxis/zkteco_bridge`.

The goal is to design the full product workflow before implementing the remaining behavior.

## Product Goal

Install one native executable on an office machine. The machine stays on the same LAN as one or more ZKTeco devices. The bridge runs in the background, syncs attendance to HRMS, exposes local troubleshooting commands, and writes logs/config in predictable locations.

```text
ZKTeco device(s)
      |
      | LAN TCP/UDP 4370
      v
zkteco-bridge Rust service
      |
      | HTTPS webhook / HRMS API
      v
HRMS cloud
```

## Existing Python Features To Preserve

The Python bridge currently supports:

- prebuilt per-OS binaries
- first-run setup wizard
- legacy single-device config
- new multi-device config
- per-device sync intervals
- `--once` sync for all devices
- `--device CODE` sync for one device
- local HTTP server on `127.0.0.1`
- `GET /health`
- `POST /sync`
- `POST /sync?device=CODE`
- `GET /pull-templates?device=CODE`
- `POST /push-user?device=CODE`
- optional HRMS job polling
- `PUSH_USER` jobs
- `PULL_TEMPLATES` jobs
- Windows Task Scheduler autostart
- Linux systemd service install
- macOS launchd service install
- rotating logs under `logs/`
- config validation
- webhook batching with batch size `500`
- retry for network errors, HTTP `429`, and HTTP `5xx`
- no retry for normal `4xx`
- process-wide/per-device sync lock
- secret redaction in errors
- clear device attendance only after successful webhook upload

## Installation Workflow

### User Goal

The user should be able to download a build and then run the command from any terminal directory:

```bash
zkteco-bridge help
zkteco-bridge doctor
zkteco-bridge setup
```

### Install Layout

The installed program should create or use a stable install directory.

Recommended locations:

```text
Windows: C:\Program Files\LevelAxis\zkteco-bridge\
Linux:   /opt/levelaxis/zkteco-bridge/
macOS:   /Applications/ZKTeco Bridge/ or /usr/local/levelaxis/zkteco-bridge/
```

For simple client installs, local-folder mode remains allowed:

```text
zkteco-bridge/
├── zkteco-bridge.exe       # or linux/macos binary
├── config.json
└── logs/
```

### Global Command

The installer should make this command available globally:

```bash
zkteco-bridge
```

Possible strategies:

```text
Windows: add install folder to PATH or install shim
Linux:   symlink /usr/local/bin/zkteco-bridge -> /opt/.../zkteco-bridge
macOS:   symlink /usr/local/bin/zkteco-bridge -> install binary
```

### Install Command Plan

Rust CLI command:

```bash
zkteco-bridge install
```

Responsibilities:

1. detect OS
2. choose install directory
3. copy current executable into install directory
4. create config/log directories
5. create global command shortcut/symlink/PATH shim
6. optionally run setup wizard
7. optionally install background service
8. print next commands

### Uninstall Command Plan

```bash
zkteco-bridge uninstall
```

Responsibilities:

1. stop background service
2. remove startup registration
3. remove global command shortcut
4. optionally keep or delete config/logs
5. remove installed executable

## Runtime Data Layout

The bridge needs durable files outside the executable.

Recommended data layout:

```text
zkteco-bridge/
├── zkteco-bridge          # executable
├── config/
│   └── config.json
├── logs/
│   ├── bridge.log
│   └── bridge-error.log
├── cache/
├── state/
│   └── last-result.json
└── backups/
    └── config-YYYYMMDD-HHMMSS.json
```

Current compatibility mode can still support:

```text
./config.json
./logs/bridge.log
```

## Setup Workflow

Command:

```bash
zkteco-bridge setup
```

Behavior:

1. detect existing config
2. ask whether to reconfigure
3. ask for top-level HRMS webhook URL
4. ask for bridge HTTP port
5. ask whether HRMS job polling should be enabled
6. ask for HRMS base URL/token if job polling is enabled
7. add/manage one or more devices
8. for each device, ask:
   - device IP
   - device port
   - device code
   - API key
   - organization ID
   - sync interval
   - clear attendance after successful sync
9. test ZKTeco device connection
10. test HRMS webhook with empty event list
11. validate full config
12. save config atomically
13. recommend `doctor`, `once`, and `service install`

## Doctor Workflow

Command:

```bash
zkteco-bridge doctor
```

Doctor should report:

- executable path
- install mode
- config path
- config exists/missing
- config validation result
- log directory
- bridge service status
- global command/PATH status
- local HTTP port availability
- configured devices count
- per-device required fields
- per-device last result
- HRMS job polling status
- OS/service backend detected

Optional flags:

```bash
zkteco-bridge doctor --deep
zkteco-bridge doctor --json
```

`--deep` should also test:

- device network connection
- HRMS webhook auth
- HRMS job API auth if configured

## Sync Workflow

Commands:

```bash
zkteco-bridge once
zkteco-bridge once --device DEVICE_CODE
```

Behavior:

1. load config
2. select all devices or one device
3. for each selected device:
   - acquire per-device lock
   - connect to device
   - pull raw attendance
   - convert to HRMS events
   - skip malformed records
   - sort events by timestamp
   - send events to HRMS in batches of `500`
   - retry retryable webhook failures
   - if upload succeeded and config allows, clear attendance
   - disconnect device
   - store last result
4. print JSON result
5. exit `0` only when all selected devices succeeded

Safety invariant:

```text
Never clear device attendance unless webhook upload succeeded.
```

## Serve / Background Workflow

Command:

```bash
zkteco-bridge serve
zkteco-bridge serve --interval 120
zkteco-bridge serve --no-poll
```

Behavior:

1. load config
2. create one sync state per device
3. start local HTTP server
4. run boot sync for every device
5. start one scheduler per device
6. optionally start HRMS job poller
7. listen for shutdown signal
8. stop cleanly

The service should bind to:

```text
127.0.0.1:<bridgePort>
```

by default.

## Local HTTP API Workflow

Endpoints to preserve:

```text
GET  /health
POST /sync
POST /sync?device=CODE
GET  /pull-templates?device=CODE
POST /push-user?device=CODE
OPTIONS *
```

### `GET /health`

Returns:

- agent name
- version
- runtime = `rust`
- webhook URL
- device count
- each device:
  - device code
  - device IP
  - syncing flag
  - last result

### `POST /sync`

Starts sync for all devices.

Returns:

- started device codes
- skipped device codes already syncing

### `POST /sync?device=CODE`

Starts sync for one device.

Returns:

- `404` if device not found
- `429` if already syncing
- `202` when started

### `GET /pull-templates?device=CODE`

Connects to the device and returns users/fingerprint templates as JSON-safe base64.

### `POST /push-user?device=CODE`

Writes one user/fingerprint template to the selected device.

## HRMS Job Polling Workflow

Optional config:

```json
{
  "hrmsBaseUrl": "https://app.example.com/api/v1",
  "hrmsApiToken": "...",
  "jobPollIntervalSeconds": 30
}
```

Behavior:

1. every `jobPollIntervalSeconds`
2. fetch pending jobs for configured device codes
3. execute each job locally
4. complete job back to HRMS

Supported job types:

```text
PUSH_USER
PULL_TEMPLATES
```

## Service / Autostart Workflow

Commands:

```bash
zkteco-bridge service install
zkteco-bridge service uninstall
zkteco-bridge service start
zkteco-bridge service stop
zkteco-bridge service status
```

OS backends:

```text
Windows: Task Scheduler first, Windows Service later if needed
Linux:   systemd
macOS:   launchd
```

Compatibility aliases:

```bash
zkteco-bridge --install-autostart
zkteco-bridge --uninstall-autostart
```

## Troubleshooting Commands

Planned commands:

```bash
zkteco-bridge help
zkteco-bridge doctor
zkteco-bridge doctor --deep
zkteco-bridge config show
zkteco-bridge config validate
zkteco-bridge config path
zkteco-bridge logs path
zkteco-bridge logs tail
zkteco-bridge once
zkteco-bridge once --device CODE
zkteco-bridge service status
zkteco-bridge service restart
zkteco-bridge devices list
zkteco-bridge devices test CODE
zkteco-bridge webhook test CODE
zkteco-bridge templates pull --device CODE
zkteco-bridge templates push --device CODE --file payload.json
```

## Config Format Plan

Rust should support the new multi-device format:

```json
{
  "vpsWebhookUrl": "https://api.example.com/api/v1/biometric-devices/webhook",
  "bridgePort": 7431,
  "devices": [
    {
      "deviceIp": "192.168.1.50",
      "devicePort": 4370,
      "deviceCode": "F22-01",
      "apiKey": "...",
      "organizationId": 1,
      "syncIntervalSeconds": 300,
      "clearAttendanceAfterSync": false
    }
  ]
}
```

Also preserve legacy single-device config compatibility during migration.

## Implementation Phases

### Phase 1: CLI And Config Parity

- add multi-device config model
- preserve legacy single-device parsing
- add `config path`
- add `logs path`
- improve `doctor`
- add `devices list`

### Phase 2: Core Sync With Fakes

- implement `DeviceSyncState`
- fake device adapter
- fake HRMS adapter
- one-sync lifecycle
- per-device lock
- last-result store
- tests for clear-after-upload safety

### Phase 3: HRMS Client

- implement reqwest webhook client
- batch size `500`
- retry/backoff
- test webhook command
- pending job API client

### Phase 4: Runtime Service

- implement `serve`
- implement scheduler
- implement shutdown
- implement state file
- implement local HTTP API

### Phase 5: Setup And Installer

- interactive setup wizard
- install/uninstall command
- service install/status/start/stop
- global command symlink/PATH shim
- logs/config directory creation

### Phase 6: Device Protocol

- implement or integrate ZKTeco protocol
- attendance pull
- clear attendance
- template pull
- push user/template
- real device integration tests

### Phase 7: Packaging And Release

- CI release matrix
- code signing plan
- checksums
- install script
- upgrade flow

## Completion Definition

The Rust bridge is complete when:

- it can be installed as a global command
- setup writes valid config
- doctor explains missing/working pieces
- serve runs as background service on Windows/Linux/macOS
- once syncs real attendance
- HTTP API works
- logs and last-result state are written
- service commands work
- all Python bridge feature workflows are covered or intentionally deprecated
- tests cover config, domain mapping, sync safety, HTTP API, HRMS retries, and service command generation

