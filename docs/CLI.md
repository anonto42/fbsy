# CLI Design

The Rust bridge ships as `fbsy` and uses `clap` for command parsing.

## Selected CLI Stack

```toml
clap = { version = "4", features = ["derive"] }
anyhow = "1"
thiserror = "2"
dialoguer = "0.11"
console = "0.15"
indicatif = "0.17"
comfy-table = "7"
```

## Command Shape

Recommended command style:

```bash
fbsy show
fbsy dashboard
fbsy run bridge
fbsy run zkteco --name dev1 -p 4370
fbsy bridge doctor
fbsy bridge sync --once
fbsy bridge config validate
fbsy bridge config show
```

Compatibility aliases:

```bash
fbsy bridge run
fbsy at-bridge run
```

## Command Responsibilities

| Command | Purpose |
| --- | --- |
| `run <service>` | Start a detached named service instance |
| `show` | List running instances |
| `dashboard` | Monitor/control instances in a TUI |
| `bridge doctor` | Show local readiness and config path |
| `bridge config setup` | Run first-time configuration wizard |
| `bridge sync --once` | Pull and forward attendance once, then exit |
| `bridge config validate` | Validate `config.json` |
| `bridge config show` | Print redacted config |

## First Output Goal

```text
ZKTeco Bridge Rust

Runtime: rust
Config:  missing
Path:    ./config.json

Next:
fbsy bridge config setup
fbsy bridge config validate
```
