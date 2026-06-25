# Codebase Architecture Decision

This document decides the best codebase architecture for `fingerbridge_in_rust`.

The goal is twofold:

1. Build an optimized production bridge that can run as one native executable.
2. Use the project to learn Rust with scalable, professional code organization.

## Decision

Use a **single Cargo package now**, organized as a **modular clean architecture**.

Do not split into many crates yet. Split into a Cargo workspace only when module boundaries become independently useful.

## Why This Decision

The project is a local bridge agent, not a large distributed backend. It has clear components, but the first production goal is still one binary:

```text
fingerbridge executable
        |
        v
config -> sync engine -> device adapter
                    \-> HRMS webhook client
        |
        v
local HTTP API + scheduler
```

Starting with one package keeps the learning path simple:

- fewer Cargo files
- faster refactoring
- easier debugging
- one binary output
- still enough structure to learn real Rust architecture

But the internal module layout should already respect boundaries.

## Source Research Notes

This decision follows these Rust ecosystem conventions:

- The Cargo Book says standard packages keep `Cargo.toml` and `Cargo.lock` at the root, source code under `src/`, integration tests under `tests/`, examples under `examples/`, and benchmarks under `benches/`.
- The Cargo Book describes workspaces as a collection of packages managed together, sharing one `Cargo.lock` and output directory. That is useful later when we truly need separate crates.
- Rust API Guidelines are recommendations from Rust library-team experience for designing idiomatic, interoperable Rust APIs.
- Rust Design Patterns warns that patterns should be chosen for their trade-offs, not copied mechanically.
- Tokio recommends simple mutex/shared-state approaches for simple data, but message passing for I/O resources or dedicated async resource owners.
- Axum focuses on ergonomic, modular routing and supports typed shared state through `State`.
- Reqwest recommends reusing a `Client` for multiple HTTP requests to benefit from connection pooling.

## Architecture Style

Use this structure:

```text
src/
├── main.rs              # binary entrypoint only
├── lib.rs               # module root for testable library code
├── cli/                 # command parsing and terminal UX
├── config/              # config data model, defaults, validation
├── domain/              # core business data types and rules
├── application/         # use cases: sync once, setup, serve, doctor
├── ports/               # traits/interfaces for outside systems
├── adapters/            # device, HRMS, filesystem, HTTP server implementations
├── runtime/             # scheduler, process lifecycle, shutdown
└── support/             # paths, logging, redaction helpers
```

This is a Rust-friendly version of clean architecture:

```text
CLI / HTTP API / service runner
        |
        v
Application use cases
        |
        v
Domain models and policies
        |
        v
Ports / traits
        |
        v
Adapters for device, HRMS, files, OS services
```

Dependency direction:

```text
cli -> application
runtime -> application
adapters -> ports/domain
application -> ports/domain
domain -> no project dependency
```

The domain layer should not know about:

- `clap`
- `axum`
- `reqwest`
- filesystem paths
- ZKTeco protocol details
- service installers

## Recommended Final Module Layout

```text
src/
├── main.rs
├── lib.rs
├── cli/
│   ├── mod.rs
│   ├── args.rs
│   ├── command.rs
│   └── dispatch.rs
├── config/
│   ├── mod.rs
│   ├── model.rs
│   ├── error.rs
│   └── impls.rs
├── domain/
│   ├── mod.rs
│   ├── attendance.rs
│   ├── event.rs
│   └── sync_result.rs
├── application/
│   ├── mod.rs
│   ├── doctor.rs
│   ├── setup.rs
│   ├── sync_once.rs
│   └── serve.rs
├── ports/
│   ├── mod.rs
│   ├── device.rs
│   ├── hrms.rs
│   ├── config_store.rs
│   └── clock.rs
├── adapters/
│   ├── mod.rs
│   ├── device_zkteco.rs
│   ├── hrms_reqwest.rs
│   ├── config_file.rs
│   ├── http_axum.rs
│   └── autostart.rs
├── runtime/
│   ├── mod.rs
│   ├── scheduler.rs
│   └── shutdown.rs
└── support/
    ├── mod.rs
    ├── logging.rs
    ├── paths.rs
    └── redaction.rs
```

## HLD For This Product

High-level design:

```text
Office LAN machine
  └── fingerbridge process
        ├── CLI setup/control commands
        ├── local HTTP API on 127.0.0.1
        ├── scheduler
        ├── sync engine
        ├── ZKTeco device adapter
        └── HRMS webhook adapter
```

System-design decisions:

- The process is mostly stateless between sync runs.
- Durable state is config, logs, and last sync result.
- The bridge binds local API to `127.0.0.1` by default.
- No message queue is needed in version one because this is a single local agent.
- Retry/backoff exists only at the HRMS webhook boundary.
- The bridge should be idempotent where possible: repeated sync requests must not corrupt device state.

## LLD For This Product

Low-level design:

- `BridgeConfig` validates all config before runtime starts.
- `RawAttendance` maps into `HrmsEvent`.
- `SyncUseCase` coordinates device pull, event mapping, webhook forwarding, optional clear.
- `DeviceClient` trait hides ZKTeco protocol details.
- `HrmsClient` trait hides HTTP details.
- `SyncState` prevents overlapping syncs and stores `last_result`.
- `ConfigStore` trait allows file-backed config now and other storage later.

Critical invariant:

```text
Never clear device attendance unless webhook upload succeeded.
```

This belongs in `application/sync_once.rs`, not inside CLI or HTTP handlers.

## Async And State Rules

Use Tokio later for HTTP server, scheduler, signal handling, and async webhook requests.

Rules:

- Do not hold a mutex guard across `.await`.
- Use simple locks only for short, non-async state updates.
- Use message passing if a single async resource needs exclusive ownership.
- Reuse one `reqwest::Client` for HRMS webhook requests.
- Keep ZKTeco device communication behind a trait because it may be blocking or protocol-specific.

Recommended state shape:

```rust
pub struct AppState {
    pub config: Arc<BridgeConfig>,
    pub sync_state: Arc<SyncState>,
    pub hrms: Arc<dyn HrmsClient + Send + Sync>,
    pub device_connector: Arc<dyn DeviceConnector + Send + Sync>,
}
```

## When To Split Into A Workspace

Stay single-package until at least three of these are true:

- device protocol code becomes large or reusable
- HTTP API becomes independently testable/reusable
- multiple binaries are needed
- a library crate is needed by another project
- compile times become painful
- feature flags become hard to manage

When that happens, split to:

```text
crates/
├── fingerbridge-core/
├── fingerbridge-device/
├── fingerbridge-hrms/
├── fingerbridge-api/
└── fingerbridge-cli/
```

Until then, modules are enough.

## Code Patterns To Practice

This project should intentionally teach:

- typed config structs with `serde`
- custom domain errors with `thiserror`
- CLI boundary errors with `anyhow`
- traits as ports
- adapters for external systems
- pure functions for mapping and validation
- integration tests with fake adapters
- async runtime boundaries with Tokio
- HTTP handlers with typed state in Axum
- dependency injection with structs, not global state

## What Not To Do

Avoid:

- putting all logic in `main.rs`
- calling HTTP directly from CLI handlers
- calling ZKTeco protocol directly from sync orchestration
- clearing device records from adapter code automatically
- storing global mutable state
- creating many crates before the design has pressure
- copying OOP design patterns without Rust-specific trade-off thinking

## Final Recommendation

Refactor the current scaffold toward:

```text
single Cargo package
library-first module structure
clean application/domain/ports/adapters separation
one production binary
workspace split later only when needed
```

This gives the best balance for the project: professional architecture, low complexity, good learning value, and a direct path to production.

## References

- Rust API Guidelines: https://rust-lang.github.io/api-guidelines/
- Cargo package layout: https://doc.rust-lang.org/cargo/guide/project-layout.html
- Cargo workspaces: https://doc.rust-lang.org/cargo/reference/workspaces.html
- Rust Design Patterns: https://rust-unofficial.github.io/patterns/
- Tokio shared state: https://tokio.rs/tokio/tutorial/shared-state
- Tokio channels: https://tokio.rs/tokio/tutorial/channels
- Axum crate docs: https://docs.rs/axum/latest/axum/
- Reqwest crate docs: https://docs.rs/reqwest/latest/reqwest/
