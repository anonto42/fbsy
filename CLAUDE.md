# fbsy — product contract

fbsy (FingerBridge) has exactly one job: **pull attendance from ZKTeco biometric
devices and forward it to the HRMS webhook, reliably, unattended, surviving
reboots and crashes.** Everything in this repo serves that job. Nothing else.

## The command surface (complete — do not extend)

Every operation must work headless from the CLI. The dashboard is an optional
viewer, never a requirement.

```
fbsy setup      configure HRMS + devices (interactive wizard)
fbsy config     check it: validate + print a redacted view
fbsy start      start the bridge — AND install the boot unit when
                config has autoStartOnBoot (supervised: launchd /
                systemd --user / schtasks; per-user, never sudo)
fbsy stop       stop it — AND remove the boot unit (otherwise
                KeepAlive respawns it and stop is a lie)
fbsy restart    stop + start, preserving the supervised/detached mode
fbsy status     one table: running?, BOOT on/off, pid, port, uptime
fbsy sync       force one sync now, print the JSON result, exit
fbsy logs       tail the bridge log (-n N, -f to follow)
fbsy install    copy binary to ~/.local/bin, PATH, dirs; offer setup
fbsy uninstall  remove binary + PATH (+ all data with --full)
fbsy update     self-update from GitHub releases (--check to only ask)
fbsy dashboard  optional TUI viewer over the same core functions
```

Hidden internals (`__service-run`, `__service-supervised`) exist only so the
binary can re-enter itself; they never appear in help.

## Rules — hold this line

1. **No new commands, flags, or features** unless the user explicitly asks.
   When something feels missing, the answer is almost always to make one of
   the commands above work better, not to add another.
2. **Never require sudo/Administrator.** Boot persistence is per-user
   (LaunchAgent, systemd --user, schtasks onlogon). If a change would need
   elevation, it is the wrong change.
3. **The safety invariant is sacred:** device attendance is never cleared
   unless the HRMS upload succeeded. Tests cover this; keep them green.
4. **stop must really stop; start must really persist.** A supervised
   process is respawned by the OS, so stop removes the unit first. start
   re-derives supervised vs detached from `autoStartOnBoot` in config.
5. **Self-update must survive the supervised model.** After the binary swap
   the OS supervisor respawns the bridge from the new binary — the updater
   waits for re-registration instead of spawning a second copy, and rolls
   back on a failed health check.
6. **Everything verifiable from the CLI:** `status` answers "is it running
   and will it survive reboot", `sync` answers "does the pipeline work",
   `logs` answers "what happened", `config` answers "is it configured".

## Releases / auto-update

Releases are GitHub Releases on `anonto42/fbsy` (the `REPO` constant in
`src/application/update.rs`). CI (`.github/workflows/ci.yml`) publishes them
automatically when `Cargo.toml`'s version is bumped on `main`: 5 platform
binaries + checksums.txt + cosign bundles. Deployed bridges with
`autoUpdate: true` pick them up on a 6-hour cycle.

## Verification bar for any change

`cargo fmt --check` · `cargo clippy --all-targets --all-features -D warnings`
· `cargo test --all` — all green before committing (CI enforces exactly
these). For behavior changes, drive the real flow: mock device + mock HRMS
(`__service-run zkteco/hrms`) in an isolated `HOME`, and check
pulled/forwarded counts, never just compilation.
