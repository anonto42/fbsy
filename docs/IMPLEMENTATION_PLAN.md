# Implementation Plan

This document turns the full workflow in
[FULL_WORKFLOW_PLAN.md](FULL_WORKFLOW_PLAN.md) into concrete build steps.

[BRIDGE_BUILD_SPEC.md](BRIDGE_BUILD_SPEC.md) is the source of truth for the
current HRMS HTTP contract. If this plan disagrees with the build spec, follow
the build spec and update this plan.

The project should be developed from inside to outside:

```text
domain -> ports -> application -> adapters -> runtime -> cli/installers
```

That order lets us learn Rust with clean boundaries and lets most behavior be
tested before real devices, real webhooks, or OS services are involved.

## Current State

Already built:

- Rust crate structure
- CLI command skeleton
- config model skeleton
- config validation skeleton
- redacted config output
- ports for device, HRMS, and config storage
- placeholder application use cases
- documentation scaffold

Not built yet:

- multi-device config parity
- real sync lifecycle
- HRMS webhook client
- local HTTP API
- scheduler/runtime state
- setup wizard
- service/autostart installer
- real ZKTeco protocol adapter
- packaging/release automation

## Python Parity Reference

The Rust bridge is replacing the Python bridge in
`~/developer_workspace/projects/levelaxis/fingerbridge`.

Use these Python files as behavior references while porting:

- `src/fingerbridge/config.py`
- `src/fingerbridge/cli.py`
- `src/fingerbridge/core/sync.py`
- `src/fingerbridge/core/hrms.py`
- `src/fingerbridge/core/job_poller.py`
- `src/fingerbridge/core/templates.py`
- `src/fingerbridge/api/server.py`
- `src/fingerbridge/models/events.py`

Use these Python tests as parity references:

- `tests/test_config.py`
- `tests/test_models.py`
- `tests/test_core.py`
- `tests/test_api.py`
- `tests/test_integration_cli.py`
- `tests/test_integration_multidevice.py`
- `tests/test_setup.py`
- `tests/test_windows_autostart.py`

## Phase 1: Multi-Device Config Parity

Goal: make Rust understand the same config shape as the Python bridge.

Build:

1. Add `BridgeDeviceConfig`.
2. Change `BridgeConfig` to contain `devices: Vec<BridgeDeviceConfig>`.
3. Preserve legacy single-device config loading.
4. Add defaults:
   - `devicePort = 4370`
   - `devicePassword = 0`
   - `deviceTimeout = 15`
   - `deviceForceUdp = false`
   - `deviceOmitPing = true`
   - `organizationId = 1`
   - `syncIntervalSeconds = 300`
   - `bridgePort = 7431`
5. Validate:
   - webhook URL is valid
   - bridge port is valid
   - every device has `deviceIp`
   - every device has `deviceCode`
   - every device has `apiKey`
   - device codes are unique
   - sync interval is positive
   - sync interval is clamped to at least `5`
   - device timeout is in `1..=120`
   - job poll interval is at least `5` seconds
6. Coerce bool-like values for:
   - `deviceForceUdp`
   - `deviceOmitPing`
   - `clearAttendanceAfterSync`
7. Decide the `hrmsApiToken` behavior before job polling:
   - preferred: use one configured device `apiKey` for job-poll bearer auth
   - compatibility option: keep `hrmsApiToken`, but require it to equal a device key
8. Update redaction to hide all device API keys.
9. Add `config path`.
10. Add `devices list`.

Exit condition:

```bash
cargo test config
cargo run -- config validate
cargo run -- config show
cargo run -- devices list
```

## Phase 2: Domain Mapping

Goal: model attendance data and HRMS events without any network code.

Build:

1. Add raw ZKTeco attendance record type.
2. Add normalized HRMS attendance event type.
3. Add mapper from raw record to HRMS event.
4. Preserve punch mapping:
   - `0` and `4` become `check_in`
   - every other punch value becomes `check_out`
5. Skip malformed records safely.
6. Sort events by timestamp.
7. Preserve timestamp behavior from the Python bridge until confirmed:
   - timezone-aware timestamps stay unchanged
   - naive timestamps are currently treated as UTC
   - confirm real device timezone before release

Exit condition:

```bash
cargo test domain
```

## Phase 3: Sync Engine With Fake Adapters

Goal: implement the complete sync behavior using fake device and fake HRMS
adapters first.

Build:

1. Add `DeviceSyncState`.
2. Add one lock per device so overlapping sync cannot run.
3. Load selected device or all devices.
4. Connect to device through the `DeviceClient` port.
5. Pull attendance through the port.
6. Convert records to HRMS events.
7. Send events through the `HrmsClient` port.
8. Clear attendance only when:
   - webhook upload succeeded
   - every batch succeeded
   - device config allows clearing
9. Store last sync result.
10. Redact secrets from sync errors.
11. Log oldest/newest pulled timestamps for offline recovery visibility.
12. Make `once` print useful JSON.

Exit condition:

```bash
cargo test sync
cargo run -- once
cargo run -- once --device DEVICE_CODE
```

Safety gate:

```text
Webhook failure must not clear device attendance.
```

This test must exist before any real device clearing code is added.

## Phase 4: HRMS Webhook Client

Goal: replace the fake HRMS adapter with a real HTTP adapter.

Build:

1. Add `reqwest`.
2. Send attendance events to `vpsWebhookUrl`.
3. Chunk events into batches of `500`.
4. Retry network failures.
5. Retry HTTP `429`.
6. Retry HTTP `5xx`.
7. Do not retry normal HTTP `4xx`.
8. Parse `data.received`.
9. Send the exact webhook body from `BRIDGE_BUILD_SPEC.md`:
   - `organizationId`
   - `deviceCode`
   - `apiKey`
   - `events`
10. Use a `User-Agent` that identifies the bridge version.
11. Add `webhook test`.
12. Add HRMS job API client:
    - fetch pending jobs
    - complete job with result
    - complete job with error
    - parse the HRMS response envelope from the `data` field

Exit condition:

```bash
cargo test hrms
cargo run -- webhook test DEVICE_CODE
```

## Phase 5: Local HTTP API

Goal: preserve the Python bridge local API.

Build:

1. Add `axum`.
2. Add shared runtime state.
3. Implement `GET /health`.
4. Implement `POST /sync`.
5. Implement `POST /sync?device=CODE`.
6. Implement `OPTIONS *`.
7. Add response codes:
   - `202` when sync starts
   - `404` when device is unknown
   - `429` when device is already syncing
8. Keep template push/pull as job-poller behavior first.
9. Treat these Python local API endpoints as legacy compatibility only, not
   core behavior, unless product requires them:
   - `GET /pull-templates?device=CODE`
   - `POST /push-user?device=CODE`

Exit condition:

```bash
cargo test api
cargo run -- serve
curl http://127.0.0.1:7431/health
```

## Phase 6: Scheduler And Background Runtime

Goal: make `serve` behave like the long-running bridge service.

Build:

1. Create one sync state per configured device.
2. Run boot sync for each device.
3. Start one scheduler per device.
4. Respect per-device `syncIntervalSeconds`.
5. Support `serve --interval` override.
6. Support `serve --no-poll`.
7. Add graceful shutdown.
8. Write last-result state to disk.
9. Write logs.

Exit condition:

```bash
cargo test runtime
cargo run -- serve --interval 120
```

## Phase 7: HRMS Job Polling

Goal: let HRMS ask the local bridge to perform device jobs.

Build:

1. Poll HRMS every `jobPollIntervalSeconds`.
2. Include configured device codes in the pending-job request.
3. Support `PUSH_USER`.
4. Support `PULL_TEMPLATES`.
5. Complete every job with success or error.
6. Use bearer auth exactly as specified in `BRIDGE_BUILD_SPEC.md`.
7. Make errors safe to log.
8. Truncate long job errors before reporting them to HRMS.

Exit condition:

```bash
cargo test job_poller
```

## Phase 8: Setup Wizard

Goal: let a non-developer create a working config from the terminal.

Build:

1. Add `dialoguer`.
2. Detect existing config.
3. Ask whether to reconfigure.
4. Prompt for webhook URL.
5. Prompt for bridge port.
6. Prompt for job polling settings.
7. Add one or more devices.
8. Test device connection.
9. Test webhook.
10. Validate final config.
11. Save config atomically.
12. Back up previous config before overwrite.

Exit condition:

```bash
cargo run -- setup
cargo run -- doctor
```

## Phase 9: Doctor And Troubleshooting

Goal: make the bridge easy to debug on client machines.

Build:

1. Improve `doctor`.
2. Add `doctor --json`.
3. Add `doctor --deep`.
4. Add `logs path`.
5. Add `logs tail`.
6. Add `service status`.
7. Add `devices test CODE`.
8. Add `templates pull`.
9. Add `templates push`.

Exit condition:

```bash
cargo run -- doctor
cargo run -- doctor --json
cargo run -- doctor --deep
```

## Phase 10: Service And Installer

Goal: install the bridge as a normal background program on each OS.

Build:

1. Add `install`.
2. Add `uninstall`.
3. Add `service install`.
4. Add `service uninstall`.
5. Add `service start`.
6. Add `service stop`.
7. Add `service status`.
8. Create config/log/state directories.
9. Create global command:
   - Windows PATH shim or installer path update
   - Linux `/usr/local/bin` symlink
   - macOS `/usr/local/bin` symlink
10. Preserve compatibility aliases:
    - `--install-autostart`
    - `--uninstall-autostart`

Exit condition:

```bash
fingerbridge help
fingerbridge service status
fingerbridge doctor
```

## Phase 11: Real ZKTeco Adapter

Goal: replace the fake device adapter with real device communication.

Build:

1. Decide whether to use an existing Rust crate or implement the protocol.
2. Support TCP port `4370`.
3. Support timeout.
4. Support password.
5. Support force UDP if required.
6. Pull attendance.
7. Clear attendance.
8. Pull users/templates.
9. Push user/template.
10. Add real-device manual test scripts.

Exit condition:

```bash
cargo run -- devices test DEVICE_CODE
cargo run -- once --device DEVICE_CODE
```

## Phase 12: Packaging And Release

Goal: ship one executable per major OS.

Build:

1. Add release profile.
2. Add CI matrix:
   - Linux x86_64
   - Windows x86_64
   - macOS x86_64
   - macOS arm64
3. Produce checksums.
4. Add install scripts.
5. Add upgrade flow.
6. Document manual install.

Exit condition:

```text
A fresh machine can download the release, run setup, pass doctor, and start the service.
```

## Implementation Rule

Every phase should finish with:

```bash
cargo fmt -- --check
cargo check
cargo test
```

When a phase changes CLI behavior, also run the related `cargo run -- ...`
commands listed in that phase.
