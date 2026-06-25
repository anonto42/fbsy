# Development

## Setup

```bash
cargo check
cargo test
```

If you are learning the codebase, start with
[CODE_WALKTHROUGH.md](CODE_WALKTHROUGH.md). It follows execution from
`main.rs` through the module tree and explains the Rust keywords used in the
current scaffold.

Create a local config:

```bash
cp config.example.json config.json
cargo run -- config validate
```

## Run Commands

```bash
cargo run -- doctor
cargo run -- config show
cargo run -- once
cargo run -- serve --interval 120
```

## Current Scaffold Behavior

The project currently validates config and prints placeholder sync output. Real device communication and webhook forwarding are intentionally not implemented yet.

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
4. Build sync lifecycle with fake device and fake HRMS tests.
5. Build HRMS webhook client with retries.
6. Add HTTP API with `axum`.
7. Add scheduler and runtime state.
8. Add setup wizard.
9. Add install/service/autostart commands.
10. Add real ZKTeco protocol adapter.
11. Add packaging and release automation.

## Important Rule

Never clear device attendance unless webhook upload succeeded. This must be enforced in code and covered by tests.
