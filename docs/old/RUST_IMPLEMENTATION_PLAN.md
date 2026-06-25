# Rust Implementation Plan

This document explains what the current ZKTeco Bridge does at every step and how to rebuild the same behavior later in Rust using the selected workspace CLI stack.

The goal is not to rewrite blindly. The goal is to preserve the proven product flow while replacing the Python/PyInstaller implementation with a native Rust executable.

## Current Product Flow

```text
Client machine on office LAN
        |
        | TCP 4370
        v
ZKTeco device
        |
        | attendance records
        v
ZKTeco Bridge
        |
        | HTTPS JSON webhook
        v
HRMS API
```

The bridge exists because a browser cannot directly talk to a ZKTeco device over raw TCP. The local bridge runs near the device, pulls attendance records, converts them to HRMS events, and forwards them to the cloud.

## Selected Rust Stack

Start with the same CLI stack selected for this workspace:

```toml
clap = { version = "4", features = ["derive"] }
anyhow = "1"
thiserror = "2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"
directories = "6"
tracing = "0.1"
tracing-subscriber = "0.3"
```

Add when needed:

```toml
dialoguer = "0.11"
console = "0.15"
indicatif = "0.17"
comfy-table = "7"
reqwest = { version = "0.12", features = ["json", "rustls-tls"] }
tokio = { version = "1", features = ["rt-multi-thread", "macros", "signal", "time", "sync"] }
axum = "0.7"
tower-http = "0.6"
```

Potential ZKTeco protocol options:

```text
Option A: find a maintained Rust ZKTeco/ZK protocol crate.
Option B: implement the small protocol surface needed by this bridge.
Option C: keep a temporary compatibility adapter while the Rust protocol matures.
```

The safest rewrite plan is to build all non-device layers first, then plug in the device protocol adapter.

## Proposed Rust Layout

For a smaller first rewrite, start as one crate with modules:

```text
src/
├── main.rs
├── cli.rs
├── config.rs
├── device.rs
├── hrms.rs
├── sync.rs
├── scheduler.rs
├── api.rs
├── setup.rs
├── models.rs
├── paths.rs
└── logging.rs
```

If it grows, split it into crates:

```text
crates/
├── fingerbridge-cli/
├── fingerbridge-core/
├── fingerbridge-config/
├── fingerbridge-device/
├── fingerbridge-hrms/
└── fingerbridge-api/
```

## Step-By-Step Behavior And Rust Mapping

| Current Step | Python Location | What Happens | Rust Implementation |
| --- | --- | --- | --- |
| CLI parse | `cli.py` | Parses `--once`, `--setup`, `--interval`, autostart flags | `clap` derive enum commands/flags |
| First-run setup | `cli.py`, `core/setup.py` | If no `config.json`, prompt user and test connections | `dialoguer` prompts, config validation, device/webhook test functions |
| Config load | `config.py` | Load JSON, apply defaults, coerce bool/int, validate URL/ports | `serde` structs plus validation method returning typed errors |
| Logging | `utils/logging_setup.py` | Console + rotating file log | `tracing`, `tracing-subscriber`, optional rolling file appender |
| Device connect | `core/device.py` | Connect to ZKTeco over TCP/UDP using `pyzk` | `DeviceConnector` trait plus protocol adapter |
| Pull attendance | `core/device.py` | Read raw attendance records | `DeviceClient::pull_attendance()` |
| Map events | `models/events.py` | Convert raw attendance to HRMS events, skip malformed rows, sort | Pure Rust model conversion with unit tests |
| Webhook post | `core/hrms.py` | Batch events, POST JSON, retry retryable failures | `reqwest` client with retry/backoff |
| Sync lifecycle | `core/sync.py` | Lock one sync at a time, sanitize errors, optionally clear device | `tokio::sync` guard or `AtomicBool` plus typed `SyncResult` |
| HTTP API | `api/server.py` | `GET /health`, `POST /sync`, CORS, 404 | `axum` router |
| Scheduler | `core/scheduler.py` | Sleep interval, run sync forever | `tokio::time::interval` background task |
| Windows autostart | `core/windows_autostart.py` | Registers Task Scheduler entry | Windows-specific adapter using `std::process::Command` |
| Services | install scripts | systemd, launchd, Windows startup | Keep scripts first, later generate service files from Rust |
| Packaging | `build.sh`, `build.bat`, CI | PyInstaller builds one binary per OS | `cargo build --release`, CI release matrix |

## CLI Shape In Rust

Preserve the current flags where needed:

```bash
fingerbridge --setup
fingerbridge --once
fingerbridge --interval 120
fingerbridge --install-autostart
fingerbridge --uninstall-autostart
```

Recommended command style for the Rust version:

```bash
fingerbridge setup
fingerbridge once
fingerbridge serve --interval 120
fingerbridge autostart install
fingerbridge autostart uninstall
fingerbridge config validate
fingerbridge doctor
```

Keep compatibility aliases for the old flags if existing client scripts depend on them.

## Config Model

Keep the existing `config.json` field names so current installs can migrate without manual edits.

```rust
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeConfig {
    pub device_ip: String,
    pub device_port: u16,
    pub device_password: i32,
    pub device_timeout: u64,
    pub device_force_udp: bool,
    pub device_omit_ping: bool,
    pub device_code: String,
    pub organization_id: u64,
    pub api_key: String,
    pub vps_webhook_url: String,
    pub clear_attendance_after_sync: bool,
    pub port: u16,
    pub sync_interval_seconds: u64,
}
```

Validation rules to preserve:

- required: `deviceIp`, `deviceCode`, `apiKey`, `vpsWebhookUrl`
- `devicePort` and `port` must be `1..=65535`
- `deviceTimeout` must be `1..=120`
- `syncIntervalSeconds` must be at least `5`
- webhook URL must use `http` or `https`
- old JSON config files should keep working

## Device Layer Design

Use traits so the rest of the bridge can be tested without a real device.

```rust
pub trait DeviceClient {
    fn pull_attendance(&mut self) -> Result<Vec<RawAttendance>, DeviceError>;
    fn clear_attendance(&mut self) -> Result<(), DeviceError>;
    fn disconnect(&mut self);
}

pub trait DeviceConnector {
    type Client: DeviceClient;

    fn connect(&self, cfg: &BridgeConfig) -> Result<Self::Client, DeviceError>;
}
```

Why:

- unit tests can use a fake device
- sync logic does not depend on protocol details
- a future protocol implementation can replace a temporary adapter cleanly

## Sync Lifecycle In Rust

Preserve the current safety behavior:

1. Reject a second sync if one is already running.
2. Connect to the device.
3. Pull attendance records.
4. Convert raw records to HRMS events.
5. Forward events to HRMS.
6. Clear device attendance only after webhook success and only if enabled.
7. Disconnect device.
8. Store `last_result`.
9. Sanitize secrets from errors.

Important invariant:

```text
Never clear device attendance unless webhook upload succeeded.
```

## HRMS Webhook In Rust

Preserve current behavior:

- batch size: `500`
- retry network errors
- retry HTTP `429` and `5xx`
- do not retry normal `4xx`
- parse JSON response
- count accepted records from `data.received`

Use:

```text
reqwest
tokio::time::sleep
serde_json
thiserror
```

## HTTP API In Rust

Endpoints to preserve:

```text
GET  /health
POST /sync
OPTIONS *
```

`GET /health` should return the same shape, with `runtime` changed to `rust`.

`POST /sync`:

- returns `429` if sync is already running
- otherwise starts a background sync
- returns `202`

Use `axum` for a small async HTTP server.

## Setup Wizard In Rust

Use `dialoguer` for:

- text prompts
- password prompt for API key
- yes/no confirmation
- reconfigure existing config

Setup flow:

1. Load existing config if present.
2. Prompt user for missing or changed values.
3. Validate config.
4. Test device connection.
5. Test HRMS webhook with zero events.
6. Save config only when tests pass, unless user explicitly saves untested config.

## Logging And Paths

Current behavior stores `config.json` and `logs/bridge.log` next to the executable or project root.

Recommended for migration:

- preserve folder-local `config.json`
- preserve `logs/bridge.log`
- use `directories` later for a native install mode
- never log raw `apiKey`

## Packaging Plan

Python uses PyInstaller and cannot cross-compile from one OS to every target.

Rust release artifacts should be:

```text
fingerbridge-linux-x86_64
fingerbridge-linux-aarch64
fingerbridge-macos-aarch64
fingerbridge-macos-x86_64
fingerbridge-windows-x86_64.exe
```

Keep install scripts initially:

- `install-service.sh`
- `install-service.bat`
- `uninstall-service.sh`
- `uninstall-service.bat`

Later the Rust executable can manage service installation itself.

## Testing Plan

Preserve current test coverage:

| Current Test Area | Rust Test Equivalent |
| --- | --- |
| config defaults/coercion/validation | `config_tests.rs` |
| event mapping/sorting/skipping bad records | `models_tests.rs` |
| webhook batching/retry/backoff | `hrms_tests.rs` with mock HTTP server |
| sync lock and clear safety | `sync_tests.rs` with fake device/client |
| `/health` and `/sync` | `api_tests.rs` with axum test server |
| Windows autostart command generation | platform-gated unit tests |

## Migration Strategy

### Phase 1: Rust Skeleton

- create Rust CLI with `clap`
- implement `doctor`, `config validate`, `setup --dry-run`
- load and validate existing `config.json`

### Phase 2: Pure Core

- implement config model
- implement event model conversion
- implement webhook client
- implement sync state with fake device
- port Python tests into Rust tests

### Phase 3: HTTP Server And Scheduler

- implement `serve`
- implement `/health`
- implement `/sync`
- implement interval scheduler
- preserve current JSON response shapes

### Phase 4: Device Protocol

- select or implement ZKTeco protocol support
- test against a real device in a controlled environment
- preserve timeout, password, UDP, omit-ping options if supported

### Phase 5: Packaging

- build local release binary
- update install scripts to use Rust binary names
- add CI release matrix
- ship side-by-side with Python version until behavior is proven

## Compatibility Rules

- Keep `config.json` field names the same.
- Keep `/health` and `/sync` response shapes stable.
- Keep `--once` behavior or provide an alias.
- Keep exit code `0` for success and `1` for failure.
- Keep logs understandable for non-developer client support.
- Keep the bridge usable as a single downloaded executable plus `config.json`.

