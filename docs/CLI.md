# CLI Reference

The bridge ships as one binary, `fbsy`. Every operation works headless from
the CLI; the dashboard is an optional viewer. This is the complete surface —
see `CLAUDE.md` for the rule that it must not grow.

## Commands

| Command | Purpose |
| --- | --- |
| `fbsy install` | Copy the binary to a per-user bin dir, add PATH, create data dirs; offers the setup wizard when no config exists |
| `fbsy uninstall` | Remove the binary and PATH entries (`--full` also deletes config/logs/data, `-y` skips the prompt) |
| `fbsy update` | Self-update from GitHub releases (`--check` only reports, `-y` skips the prompt) |
| `fbsy setup` | Interactive wizard: HRMS webhook, devices, boot auto-start, auto-update |
| `fbsy config` | Check the config: validate it and print a redacted view |
| `fbsy start` | Start the bridge in the background; when config has `autoStartOnBoot`, installs the per-user boot unit and runs OS-supervised |
| `fbsy stop` | Stop the bridge AND remove the boot unit |
| `fbsy restart` | Stop + start, preserving supervised/detached mode |
| `fbsy status` | One table: running?, BOOT on/off, pid, uptime, port, address |
| `fbsy sync` | Pull attendance and forward to HRMS once, print the JSON result, exit (`--device CODE`, `--config PATH`) |
| `fbsy logs` | Print the bridge log (`-n N` lines, `-f` to follow) |
| `fbsy dashboard` | Full-screen TUI over the same core functions |

Hidden internals (`__service-run`, `__service-supervised`) let the binary
re-enter itself for detached/supervised execution and the built-in mock
servers used by `docs/LOCAL-TEST-PLAN.md`.

## Daily operation

```bash
fbsy status    # is it running? BOOT on?
fbsy logs -f   # watch it live
fbsy stop      # stop AND remove from boot
fbsy start     # start AND re-enable boot
fbsy sync      # force one sync now
```

## Boot persistence

Per-user, never needs sudo/Administrator:

- macOS: `~/Library/LaunchAgents/com.fbsy.bridge.plist` (RunAtLoad + KeepAlive)
- Linux: `~/.config/systemd/user/fbsy-bridge.service` (Restart=always)
- Windows: `schtasks` ONLOGON task for the current user

## CLI stack

```toml
clap = { version = "4", features = ["derive"] }
anyhow = "1"
dialoguer = "0.11"
console = "0.15"
ratatui = "0.29"
```
