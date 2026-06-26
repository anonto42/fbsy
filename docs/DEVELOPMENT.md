# Development

## Setup

```bash
cargo check
cargo test
```

If you are learning the codebase, start with
[CODE_WALKTHROUGH.md](CODE_WALKTHROUGH.md). It follows execution from
`main.rs` through the module tree and explains the Rust keywords used by the
current service manager.

Create a local config:

```bash
cp config.example.json config.json
cargo run -- bridge config validate
```

## Run Commands

```bash
cargo run -- bridge doctor
cargo run -- bridge config show
cargo run -- bridge sync --once
cargo run -- run bridge
cargo run -- run zkteco --name dev1 -p 4370
```

## Current Behavior

The project is now a native `fbsy` service manager. It can run the bridge, mock
ZKTeco devices, and a mock HRMS server as detached named instances; the bridge
can sync attendance, serve the local HTTP API, poll HRMS jobs, and use the real
ZKTeco TCP and HRMS HTTP adapters.

The codebase now follows the researched architecture:

```text
src/
├── cli/
├── config/
├── domain/
├── application/
├── ports/
├── adapters/
├── runtime/
└── support/
```

Keep new code inside the matching boundary. CLI code should parse commands and call application use cases; it should not own sync, device, or webhook logic.

## Development Order

The full behavior plan is documented in
[FULL_WORKFLOW_PLAN.md](FULL_WORKFLOW_PLAN.md). Use that as the product roadmap.

1. Keep the CLI compiling.
2. Add multi-device config parity with the Python bridge.
3. Improve `doctor` and troubleshooting commands.
4. Keep mock-device and HRMS tests green.
5. Harden real-device protocol behavior against more firmware variants.
6. Keep setup/install/update flows cross-platform.
7. Add packaging and release automation.

## Important Rule

Never clear device attendance unless webhook upload succeeded. This must be enforced in code and covered by tests.
