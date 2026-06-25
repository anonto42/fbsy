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
fingerbridge doctor
fingerbridge setup
fingerbridge once
fingerbridge serve --interval 120
fingerbridge config validate
fingerbridge config show
fingerbridge autostart install
fingerbridge autostart uninstall
```

Compatibility flags from the Python version:

```bash
fingerbridge --setup
fingerbridge --once
fingerbridge --interval 120
fingerbridge --install-autostart
fingerbridge --uninstall-autostart
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
  fingerbridge setup
  fingerbridge config validate
```

