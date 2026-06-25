# Architecture

The Rust bridge should keep the same proven behavior as the Python bridge while becoming easier to ship as a native executable.

## Runtime Shape

```text
CLI / service entrypoint
        |
        v
Config loader and validator
        |
        v
Sync engine
   |          |
   v          v
Device     HRMS webhook
adapter    client
        |
        v
HTTP API and scheduler
```

## Architecture Decision

The chosen codebase architecture is documented in
[CODEBASE_ARCHITECTURE_DECISION.md](CODEBASE_ARCHITECTURE_DECISION.md).

Short version:

```text
single Cargo package now
clean internal module boundaries
library-first structure
workspace split later only when needed
```

## Current Module Layout

```text
src/
├── main.rs              # tiny binary entrypoint
├── lib.rs               # public module root
├── cli/                 # clap commands and terminal entry flow
├── config/              # config model and validation
├── domain/              # attendance, HRMS events, sync result
├── application/         # use cases called by clients
├── ports/               # traits for outside systems
├── adapters/            # file config store and placeholder clients
├── runtime/             # scheduler/process runtime placeholders
└── support/             # paths and redaction helpers
```

## Dependency Direction

The CLI calls application use cases. Application code depends on domain models and ports. Adapters implement the ports.

```text
cli -> application
application -> domain
application -> ports
application -> adapters
adapters -> ports
adapters -> domain
domain -> no project dependency
```

The device layer should be trait-based so tests can run without a real ZKTeco device.

## Device Boundary

```rust
pub trait DeviceClient {
    fn pull_attendance(&mut self) -> Result<Vec<RawAttendance>, DeviceError>;
    fn clear_attendance(&mut self) -> Result<(), DeviceError>;
    fn disconnect(&mut self);
}
```

This keeps the sync engine independent from the exact ZKTeco protocol implementation.

## HTTP Boundary

The local API should preserve the Python bridge endpoints:

```text
GET  /health
POST /sync
OPTIONS *
```

The HTTP server will be implemented with `axum` after the core sync lifecycle is ready.

## Future Growth

```text
single Cargo package now
split into crates only when module boundaries become independently useful
```

This keeps Rust learning practical while still following scalable architecture boundaries.
