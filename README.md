# FingerBridge

Native biometric attendance bridge for HRMS webhook ingestion.

The bridge runs on a client machine inside the office LAN, connects to a ZKTeco attendance device, pulls attendance records, converts them into HRMS events, and forwards those events to the HRMS webhook API.

```text
ZKTeco Device
      |
      | TCP 4370
      v
FingerBridge
      |
      | HTTPS JSON webhook
      v
HRMS API
```

## Project Status

This project is currently in scaffold stage.

Implemented:

- Rust package structure
- CLI command skeleton with `clap`
- config model and validation skeleton
- device abstraction traits
- event model skeleton
- sync result skeleton
- example config
- project documentation

Not implemented yet:

- real ZKTeco protocol adapter
- HRMS HTTP client
- setup wizard
- HTTP server
- scheduler
- service/autostart installers
- release CI

## Commands

```bash
cargo run -- doctor
cargo run -- config validate
cargo run -- config show
cargo run -- once
cargo run -- serve --interval 120
```

Compatibility aliases planned from the Python version:

```bash
fingerbridge --setup
fingerbridge --once
fingerbridge --interval 120
fingerbridge --install-autostart
fingerbridge --uninstall-autostart
```

## First Local Setup

```bash
cp config.example.json config.json
cargo run -- config validate
cargo run -- doctor
```

## Documentation

- [Architecture](docs/ARCHITECTURE.md)
- [Code Walkthrough From main.rs](docs/CODE_WALKTHROUGH.md)
- [Codebase Architecture Decision](docs/CODEBASE_ARCHITECTURE_DECISION.md)
- [CLI Design](docs/CLI.md)
- [Configuration](docs/CONFIGURATION.md)
- [Development](docs/DEVELOPMENT.md)
- [Full Software Workflow Plan](docs/FULL_WORKFLOW_PLAN.md)
- [Implementation Plan](docs/IMPLEMENTATION_PLAN.md)
- [Migration From Python](docs/MIGRATION_FROM_PYTHON.md)
- [Packaging](docs/PACKAGING.md)
- [Security And Safety](docs/SECURITY.md)
- [Testing](docs/TESTING.md)

## Core Safety Rule

Never clear attendance records from the ZKTeco device unless the HRMS webhook upload succeeded.
