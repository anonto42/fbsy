# Testing

This project should be tested in layers. Most behavior must be proven with fake
adapters before we connect to a real ZKTeco device or a real HRMS server.

[BRIDGE_BUILD_SPEC.md](BRIDGE_BUILD_SPEC.md) is the source of truth for HRMS
request/response contracts. The Python bridge tests in
`~/developer_workspace/projects/levelaxis/fingerbridge/tests` are the parity
reference for behavior that already exists.

## Test Pyramid

```text
manual real-device tests
        ^
HTTP/CLI integration tests
        ^
application tests with fake ports
        ^
domain/config unit tests
```

The lower layers should contain most tests. Real-device tests are important, but
they should be the final confirmation, not the only proof.

## Always Run

Run these after every implementation phase:

```bash
cargo fmt -- --check
cargo check
cargo test
```

## Current Tests

```bash
cargo test
```

Current scaffold test:

- confirms `config.example.json` exists

## Phase Test Plan

| Phase | What To Test |
| --- | --- |
| Config | defaults, legacy config migration, multi-device config, unique device codes, invalid URL, invalid port, bool coercion, interval clamping, secret redaction |
| Domain | punch mapping, timestamp conversion, timezone behavior, sorting, malformed attendance skipping |
| Sync | per-device lock, selected-device sync, all-device sync, empty attendance, webhook failure, partial-batch failure, clear-after-success rule, error sanitization, oldest/newest logging |
| HRMS | exact webhook body, batch size `500`, retry network failures, retry `429`, retry `5xx`, no retry for normal `4xx`, parse `data.received`, user agent |
| API | `/health`, `/sync`, `/sync?device=CODE`, unknown device `404`, already syncing `429`, CORS `OPTIONS`, local-only bind |
| Runtime | boot sync, interval scheduling, shutdown, `--interval` override, `--no-poll`, last-result state |
| Setup | prompt flow, validation loop, existing config backup, atomic save |
| Service | generated systemd unit, generated launchd plist, Windows Task Scheduler command, service status parsing |
| CLI | help output, command aliases, JSON output modes, exit codes |
| Packaging | release binary starts, config path works, logs path works, service install uses expected paths |

## Non-Negotiable Safety Tests

These tests must exist before real device clearing is implemented:

```text
webhook failure does not clear device attendance
webhook partial failure does not clear device attendance
device clear runs only after all batches upload successfully
sync error output does not expose apiKey
sync error output does not expose deviceCode when marked sensitive
```

## Unit Tests

Use unit tests for pure logic:

- config validation
- config defaults
- legacy flat config wrapping
- bool string coercion
- sync interval clamping
- redaction
- attendance-to-event mapping
- event sorting
- retry policy decision
- service file generation
- path selection

Example command:

```bash
cargo test config
cargo test domain
```

## Application Tests

Use fake ports for application behavior.

Fake implementations needed:

- fake config store
- fake device client
- fake HRMS client
- fake state store
- fake clock if scheduling logic needs deterministic time

Application tests should prove:

- `once` works for all devices
- `once --device CODE` only touches that device
- `once --device UNKNOWN` exits with failure
- one device failure does not hide another device success
- locks prevent duplicate sync
- different devices can sync at the same time
- clear attendance follows the safety rule
- empty attendance does not call HRMS
- failed clear after successful upload does not fail the sync

Example command:

```bash
cargo test sync
```

## HTTP Integration Tests

Use `axum` test helpers or spawn the router in-process.

Test:

- health response shape
- sync-all response shape
- sync-one response shape
- unknown device response
- already-syncing response
- CORS preflight response
- `127.0.0.1` bind behavior

Legacy compatibility candidates:

- `GET /pull-templates?device=CODE`
- `POST /push-user?device=CODE`

Those endpoints existed in Python. The newer Rust build spec moves template
work into HRMS job polling, so only keep the local endpoints if product still
needs them.

Example command:

```bash
cargo test api
```

## CLI Tests

Use `assert_cmd` later for CLI behavior.

Test:

- `fingerbridge help`
- `fingerbridge doctor`
- `fingerbridge config validate`
- `fingerbridge config show`
- `fingerbridge once`
- compatibility aliases like `--once` and `--setup`
- `--interval` clamps to at least `5`
- success and failure exit codes

Example future command:

```bash
cargo test cli
```

## Manual Real-Device Tests

These tests need a real ZKTeco device on the same LAN.

Checklist:

1. `fingerbridge devices test CODE` connects.
2. `fingerbridge once --device CODE` pulls attendance.
3. HRMS receives events.
4. Failed HRMS upload does not clear attendance.
5. Successful HRMS upload clears attendance only when enabled.
6. `templates pull` returns users/templates.
7. `templates push` writes a test user/template.
8. Device disconnects cleanly after every command.

## Manual Service Tests

Run per OS before release.

Windows:

- install service/autostart
- reboot
- confirm bridge starts
- confirm logs are written
- uninstall cleanly

Linux:

- install systemd unit
- `systemctl start`
- `systemctl status`
- reboot
- uninstall cleanly

macOS:

- install launchd plist
- `launchctl start`
- reboot
- uninstall cleanly

## Release Smoke Test

On a clean machine:

1. Download release build.
2. Run installer or install command.
3. Confirm global command works:

```bash
fingerbridge help
```

4. Run setup.
5. Run doctor.
6. Start service.
7. Call health endpoint.
8. Run one sync.
9. Check logs.
10. Uninstall.
