# Code Walkthrough From `main.rs`

This document explains how the Rust project connects together, starting from `src/main.rs` and following the tree one module at a time.

Use this when you want to read the project like a tutorial.

## 1. The Big Tree

```text
src/
├── main.rs
├── lib.rs
├── cli/
│   ├── mod.rs
│   ├── args.rs
│   ├── command.rs
│   └── dispatch.rs
├── application/
│   ├── mod.rs
│   ├── doctor.rs
│   ├── config.rs
│   ├── sync_once.rs
│   ├── serve.rs
│   ├── setup.rs
│   └── autostart.rs
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
├── ports/
│   ├── mod.rs
│   ├── config_store.rs
│   ├── device.rs
│   └── hrms.rs
├── adapters/
│   ├── mod.rs
│   ├── config_file.rs
│   └── hrms_placeholder.rs
├── runtime/
│   ├── mod.rs
│   └── scheduler.rs
└── support/
    ├── mod.rs
    ├── paths.rs
    └── redaction.rs
```

Mental model:

```text
main.rs
  -> cli
    -> application
      -> config/domain/ports
        -> adapters/support
```

## 2. `main.rs`: Program Starts Here

File:

```text
src/main.rs
```

Code shape:

```rust
use anyhow::Result;
use clap::Parser;

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = fingerbridge::cli::Cli::parse();
    fingerbridge::cli::run(cli)
}
```

What happens:

1. Rust starts at `fn main()`.
2. Logging is initialized.
3. `clap` reads terminal arguments.
4. Those arguments become a typed `Cli` struct.
5. The parsed CLI is passed to `fingerbridge::cli::run(cli)`.

Important keywords:

| Code | Meaning |
| --- | --- |
| `use anyhow::Result;` | Import `Result` so the function can return errors cleanly |
| `use clap::Parser;` | Import the `Parser` trait so `Cli::parse()` works |
| `fn main()` | Program entrypoint |
| `-> Result<()>` | Main can either succeed with `()` or return an error |
| `let cli = ...` | Create a local variable named `cli` |
| `fingerbridge::cli` | Access the library crate module named `cli` |

## 3. Why `fingerbridge::...` Works

The package name in `Cargo.toml` is:

```toml
name = "fingerbridge"
```

Rust converts the crate name to:

```rust
fingerbridge
```

So this:

```rust
fingerbridge::cli::Cli
```

means:

```text
inside this package's library
go to cli module
get Cli type
```

## 4. `lib.rs`: Project Map

File:

```text
src/lib.rs
```

Code shape:

```rust
pub mod adapters;
pub mod application;
pub mod cli;
pub mod config;
pub mod domain;
pub mod ports;
pub mod runtime;
pub mod support;
```

What happens:

`lib.rs` tells Rust which top-level modules exist.

Example:

```rust
pub mod cli;
```

connects to:

```text
src/cli/mod.rs
```

Important keywords:

| Code | Meaning |
| --- | --- |
| `mod` | Declare a module |
| `pub mod` | Declare a public module that other modules can access |
| `//!` | Documentation comment for the whole module/file |
| `///` | Documentation comment for the item below it |

## 5. `cli/mod.rs`: CLI Module Door

File:

```text
src/cli/mod.rs
```

Code shape:

```rust
mod args;
mod command;
mod dispatch;

use anyhow::Result;

pub use args::Cli;
pub use command::{AutostartCommand, Command, ConfigCommand};

pub fn run(cli: Cli) -> Result<()> {
    dispatch::run(cli)
}
```

What happens:

1. `mod args;` loads `src/cli/args.rs`.
2. `mod command;` loads `src/cli/command.rs`.
3. `mod dispatch;` loads `src/cli/dispatch.rs`.
4. `pub use args::Cli;` re-exports `Cli`.
5. Other code can now write `fingerbridge::cli::Cli`.
6. `run(cli)` forwards work into `dispatch::run(cli)`.

Important keywords:

| Code | Meaning |
| --- | --- |
| `mod args;` | Add child module from `args.rs` |
| `mod command;` | Add child module from `command.rs` |
| `mod dispatch;` | Add child module from `dispatch.rs` |
| `pub use` | Re-export something from inside the module |
| `pub fn` | Public function |

Why this file exists:

```text
cli/mod.rs is the front door of the cli folder.
args.rs holds the top-level CLI struct.
command.rs holds command enums.
dispatch.rs sends parsed commands to application use cases.
```

## 6. `cli/args.rs`: Top-Level CLI Arguments

File:

```text
src/cli/args.rs
```

Important code shape:

```rust
#[derive(Debug, Parser)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    #[arg(long)]
    pub once: bool,
}
```

What happens:

`clap` uses this struct to understand terminal input.

Example:

```bash
fingerbridge doctor
```

becomes:

```rust
Cli {
    command: Some(Command::Doctor),
    once: false,
    ...
}
```

Important keywords and syntax:

| Code | Meaning |
| --- | --- |
| `struct` | A named data shape |
| `enum` | A value that can be one of several variants |
| `Option<T>` | Either `Some(value)` or `None` |
| `bool` | `true` or `false` |
| `PathBuf` | Owned filesystem path |
| `#[derive(...)]` | Ask Rust/macros to generate code automatically |
| `#[command(...)]` | `clap` metadata for commands |
| `#[arg(...)]` | `clap` metadata for arguments/flags |

## 7. `cli/command.rs`: Command Enums

File:

```text
src/cli/command.rs
```

Important code shape:

```rust
#[derive(Debug, Subcommand)]
pub enum Command {
    Doctor,
    Setup,
    Once { config: Option<PathBuf> },
    Serve { interval: Option<u64>, config: Option<PathBuf> },
    Config { command: ConfigCommand },
    Autostart { command: AutostartCommand },
}
```

What happens:

This file defines every command group the program understands.

Example:

```bash
fingerbridge serve --interval 120
```

becomes:

```rust
Command::Serve {
    interval: Some(120),
    config: None,
}
```

## 8. `cli/dispatch.rs`: CLI Calls Application

In `dispatch.rs`:

```rust
pub fn run(cli: Cli) -> Result<()> {
    if cli.setup {
        return application::setup::run();
    }

    match cli.command.unwrap_or(Command::Doctor) {
        Command::Doctor => application::doctor::run(),
        Command::Setup => application::setup::run(),
        Command::Once { config } => application::sync_once::run(config),
        Command::Serve { interval, config } => application::serve::run(interval, config),
        ...
    }
}
```

What happens:

1. Compatibility flags like `--setup` and `--once` are checked first.
2. If no command exists, it defaults to `doctor`.
3. `match` chooses the correct application use case.

Important keywords:

| Code | Meaning |
| --- | --- |
| `if` | Branch if condition is true |
| `return` | Exit function early |
| `match` | Pattern-match an enum/value |
| `Command::Doctor` | The `Doctor` variant of the `Command` enum |
| `{ config }` | Pull the `config` field out of the enum variant |
| `unwrap_or(...)` | Use the value if present, otherwise use a default |

Architecture rule:

```text
CLI does not do product logic.
CLI only decides which application use case to call.
```

## 9. `application/`: Product Use Cases

Folder:

```text
src/application/
```

Each file is one product action:

| File | Purpose |
| --- | --- |
| `doctor.rs` | Print local status |
| `config.rs` | Validate/show config |
| `sync_once.rs` | Run one sync attempt |
| `serve.rs` | Start service mode later |
| `setup.rs` | Setup wizard later |
| `autostart.rs` | OS startup integration later |

Example:

```rust
pub fn run(config: Option<PathBuf>) -> Result<()> {
    let path = config.unwrap_or_else(default_config_path);
    let store = JsonConfigStore;
    let _cfg = store.load(&path)?;
    let result = placeholder_result();
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
```

Important syntax:

| Code | Meaning |
| --- | --- |
| `Option<PathBuf>` | Maybe a custom config path exists |
| `unwrap_or_else(default_config_path)` | Use custom path, otherwise call default function |
| `let store = JsonConfigStore;` | Create the file-backed config adapter |
| `?` | If this returns an error, return early from this function |
| `Ok(())` | Successful result with no value |

## 10. `config/`: Config Shape And Rules

Folder:

```text
src/config/
```

Files:

| File | Purpose |
| --- | --- |
| `model.rs` | Defines `BridgeConfig` |
| `error.rs` | Defines `ConfigError` |
| `impls.rs` | Defines `BridgeConfig::redacted()` and `BridgeConfig::validate()` |
| `mod.rs` | Re-exports public config types |

Important code:

```rust
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeConfig {
    pub device_ip: String,
    pub device_port: u16,
    ...
}
```

Meaning:

```text
BridgeConfig is the Rust version of config.json.
serde reads/writes JSON.
rename_all = "camelCase" keeps compatibility with deviceIp, devicePort, etc.
```

Important types:

| Type | Meaning |
| --- | --- |
| `String` | Owned text |
| `u16` | Unsigned 16-bit integer, good for ports |
| `u64` | Unsigned 64-bit integer |
| `i32` | Signed 32-bit integer |
| `bool` | true/false |

## 11. `adapters/config_file.rs`: Real File Loading

File:

```text
src/adapters/config_file.rs
```

Code:

```rust
impl ConfigStore for JsonConfigStore {
    fn load(&self, path: &Path) -> Result<BridgeConfig, ConfigError> {
        ...
    }
}
```

Meaning:

```text
JsonConfigStore is the real implementation of the ConfigStore trait.
It knows how to read config.json from disk.
```

Important keyword:

| Code | Meaning |
| --- | --- |
| `impl Trait for Type` | Make a type implement a trait |
| `&self` | Borrow the current object |
| `&Path` | Borrow a filesystem path |

## 12. `ports/`: Interfaces

Folder:

```text
src/ports/
```

Ports are traits. They describe what the application needs from the outside world.

Example:

```rust
pub trait ConfigStore {
    fn load(&self, path: &Path) -> Result<BridgeConfig, ConfigError>;
}
```

Meaning:

```text
Any type that can load config can implement ConfigStore.
The application does not need to know how config is loaded.
```

Why this matters:

```text
Today: JsonConfigStore loads config from file.
Tests: FakeConfigStore can return test config.
Future: NativeConfigStore can load from OS app data.
```

## 13. `domain/`: Pure Business Types

Folder:

```text
src/domain/
```

Domain code should not know about:

```text
CLI
files
HTTP
ZKTeco protocol
operating system
```

Example:

```rust
pub fn event_type_from_punch(punch: i32) -> &'static str {
    match punch {
        0 | 4 => "check_in",
        _ => "check_out",
    }
}
```

Meaning:

```text
This is a pure rule:
ZKTeco punch codes 0 and 4 become check_in.
Everything else becomes check_out.
```

Because this is pure logic, it is easy to test.

## 14. `support/`: Small Helpers

Folder:

```text
src/support/
```

Examples:

```rust
pub fn default_config_path() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| app_base_dir())
        .join("config.json")
}
```

Meaning:

```text
Find the default config path.
Currently it uses ./config.json.
```

## 15. Request Flow Example: `cargo run -- doctor`

```text
main.rs
  calls Cli::parse()

cli/dispatch.rs
  parses command as Command::Doctor

application/doctor.rs
  checks config path
  prints status
```

## 16. Request Flow Example: `cargo run -- config validate`

```text
main.rs
  starts executable

cli/dispatch.rs
  matches Command::Config -> ConfigCommand::Validate

application/config.rs
  creates JsonConfigStore
  loads config

adapters/config_file.rs
  reads config.json
  deserializes JSON into BridgeConfig
  calls cfg.validate()

config/impls.rs
  checks required fields and limits
```

## 17. Request Flow Example: `cargo run -- once`

```text
main.rs
  starts executable

cli/dispatch.rs
  matches Command::Once

application/sync_once.rs
  loads config
  returns placeholder SyncResult

domain/sync_result.rs
  defines the JSON result shape
```

Later this flow will expand:

```text
sync_once
  -> device connector
  -> raw attendance
  -> domain event mapping
  -> HRMS client
  -> optional clear attendance
```

## 18. Symbols You Will See Often

| Symbol | Meaning |
| --- | --- |
| `::` | Path separator, like `crate::config::BridgeConfig` |
| `&` | Borrow/reference instead of taking ownership |
| `?` | Return early if the result is an error |
| `()` | Empty value/unit type |
| `<T>` | Generic type parameter, like `Option<PathBuf>` |
| `#[...]` | Attribute/macro metadata |
| `//!` | Documentation for the whole module |
| `///` | Documentation for the next item |

## 19. Reading Order For Learning

Read in this order:

1. `src/main.rs`
2. `src/lib.rs`
3. `src/cli/mod.rs`
4. `src/cli/args.rs`
5. `src/cli/command.rs`
6. `src/cli/dispatch.rs`
7. `src/application/doctor.rs`
8. `src/application/config.rs`
9. `src/adapters/config_file.rs`
10. `src/config/model.rs`
11. `src/config/impls.rs`
12. `src/domain/event.rs`
13. `src/ports/device.rs`
14. `src/application/sync_once.rs`

That path teaches how a command enters the program, moves through the architecture, and reaches business logic.
