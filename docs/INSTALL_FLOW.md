# Install → run flow

End-to-end lifecycle of `fbsy`, from a one-line install to running services and
the live dashboard. Each stage notes **what it touches on the machine**.

```
┌─────────────────┐   1. install script   ┌──────────────────┐
│  your terminal  │ ─────────────────────▶ │  ~/.local/bin    │  binary on PATH
└─────────────────┘                        │  ~/.config/fbsy  │  config / logs / run
                                           └──────────────────┘
        │ 2. fbsy run bridge (first run → wizard)
        ▼
┌──────────────────────────────────────────────────────────────┐
│  detached background processes (one per service)             │
│   bridge ──pull──▶ ZKTeco device ──forward──▶ HRMS webhook │
│   zkteco (mock device)        hrms (mock HRMS, local testing) │
└──────────────────────────────────────────────────────────────┘
        │ 3. fbsy dashboard / show / logs / close
        ▼
   monitor & control
```

---

## Stage 1 — Install (`install.sh` / `install.ps1`)

**Linux / macOS:**
```bash
curl -fsSL https://raw.githubusercontent.com/anonto42/fbsy/main/scripts/install.sh | sh
```
**Windows (PowerShell):**
```powershell
irm https://raw.githubusercontent.com/anonto42/fbsy/main/scripts/install.ps1 | iex
```

What the script does, in order:
1. **Detect OS + arch** (`uname` / `$PROCESSOR_ARCHITECTURE`) and pick the matching
   release asset (`fbsy-linux-x86_64`, `fbsy-macos-arm64`, `fbsy-windows-x86_64.exe`, …).
2. **Download** it from `https://github.com/anonto42/fbsy/releases/latest/download/<asset>`
   (public redirect — no GitHub token needed). Pin with `FBSY_VERSION=0.2.x`.
3. **Verify** the SHA-256 against `checksums.txt` (skip with `FBSY_NO_VERIFY=1`).
4. **Make runnable**: `chmod +x`; on macOS strip the `com.apple.quarantine` xattr so
   Gatekeeper doesn't block it.
5. **Place** the binary in `~/.local/bin/fbsy` (Windows: `%LOCALAPPDATA%\Programs\fbsy\fbsy.exe`).
6. **Hand off** to `fbsy install` (Stage 2).

> Touches: downloads one file; writes the binary into your user bin dir. No root.

## Stage 2 — `fbsy install` (machine setup)

Runs automatically at the end of the script (or manually if you downloaded the binary
yourself). It:
1. Creates the per-OS data directory and subfolders:
   - Linux `~/.config/fbsy/`, macOS `~/Library/Application Support/fbsy/`,
     Windows `%APPDATA%\fbsy\` — each with `config/`, `logs/`, `run/`.
2. Ensures the bin dir is on **PATH** (appends an idempotent line to your shell rc on
   Unix; sets the User PATH via the proper API on Windows) so `fbsy` works from any directory.
3. Migrates a legacy `./config.json` from the working directory if present.

> Touches: creates `~/.config/fbsy/{config,logs,run}`; edits one shell rc / User PATH entry.

After this, open a new shell and `fbsy --help` works anywhere.

## Stage 3 — Start the bridge (`fbsy run bridge`)

- **First run, no config:** launches the interactive setup wizard, which writes
  `~/.config/fbsy/config/config.json` (device IP/port, webhook URL, API key, …).
- **Configured:** spawns the bridge as a **detached background process** and records
  `~/.config/fbsy/run/bridge.json` (pid, port, start time). The bridge then pulls
  attendance from the device and forwards it to your HRMS webhook on its schedule.
- **Already running:** prints status instead of starting a second copy.

> Touches: writes the config (first run) and a registry file; starts one background process.

## Stage 4 — Mock services for local testing (optional)

```bash
fbsy run hrms      # mock HRMS webhook on :8800  (prints what it receives)
fbsy run zkteco    # mock ZKTeco device on :4370 (serves fake attendance)
```
Point `bridge`'s config at `127.0.0.1:8800` / `127.0.0.1:4370` to exercise the full
pipeline without real hardware.

> Touches: starts background processes + registry files; no network beyond localhost.

## Stage 5 — Monitor & control

| Command | What it does |
|---|---|
| `fbsy dashboard` | Live full-screen TUI: table of service instances + log pane. Keys: ↑/↓ select, `s` start, `x` stop, `r` restart, `l` toggle logs, `a` all logs, `q` quit. |
| `fbsy show` | One-shot snapshot table (script/pipe friendly). |
| `fbsy status <instance>` | Detail for one service instance. |
| `fbsy logs <instance> [-n N] [--follow]` | Tail an instance's log. |
| `fbsy bridge sync --once` | Pull attendance once, on demand. |
| `fbsy bridge config show/validate/setup` | Inspect or (re)configure. |
| `fbsy bridge doctor --deep` | Connectivity diagnostics. |
| `fbsy close <instance>` | Stop an instance and clear its registry entry. |

> The dashboard and `show` read the same registry + live process check; killing a service
> out-of-band shows up as `stopped` and the stale entry is auto-cleared.

## Stage 6 — Uninstall

```bash
fbsy uninstall      # removes the installed binary; leaves ~/.config/fbsy intact
```

> Touches: deletes the binary only. Remove `~/.config/fbsy` and the shell-rc PATH line
> manually if you want a full wipe.
