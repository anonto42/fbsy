# CLI Design

The Rust bridge uses `clap` for command parsing.

## Selected CLI Stack

```toml
clap = { version = "4", features = ["derive"] }
anyhow = "1"
thiserror = "2"
```

Add later:

```toml
dialoguer = "0.11"
console = "0.15"
indicatif = "0.17"
comfy-table = "7"
```

## Command Shape

Recommended command style:

```bash
zkteco-bridge doctor
zkteco-bridge setup
zkteco-bridge once
zkteco-bridge serve --interval 120
zkteco-bridge config validate
zkteco-bridge config show
zkteco-bridge autostart install
zkteco-bridge autostart uninstall
```

Compatibility flags from the Python version:

```bash
zkteco-bridge --setup
zkteco-bridge --once
zkteco-bridge --interval 120
zkteco-bridge --install-autostart
zkteco-bridge --uninstall-autostart
```

## Command Responsibilities

| Command | Purpose |
| --- | --- |
| `doctor` | Show local readiness and config path |
| `setup` | Run first-time configuration wizard |
| `once` | Pull and forward attendance once, then exit |
| `serve` | Start HTTP API and background scheduler |
| `config validate` | Validate `config.json` |
| `config show` | Print redacted config |
| `autostart install` | Register startup task/service |
| `autostart uninstall` | Remove startup task/service |

## First Output Goal

```text
ZKTeco Bridge Rust

Runtime: rust
Config:  missing
Path:    ./config.json

Next:
  zkteco-bridge setup
  zkteco-bridge config validate
```

