# Packaging

Rust should produce one native binary per platform.

## Local Build

```bash
cargo build --release
```

Output:

```text
target/release/zkteco-bridge
```

On Windows:

```text
target/release/zkteco-bridge.exe
```

## Planned Release Artifacts

```text
zkteco-bridge-linux-x86_64
zkteco-bridge-linux-aarch64
zkteco-bridge-macos-aarch64
zkteco-bridge-macos-x86_64
zkteco-bridge-windows-x86_64.exe
```

## Runtime Files

The binary should run with:

```text
zkteco-bridge
config.json
logs/
```

Do not embed client secrets into the binary.

## Service Installers

Keep installer scripts first:

- Linux: `systemd`
- macOS: `launchd`
- Windows: Task Scheduler

Later, the Rust binary can implement `autostart install` and `autostart uninstall`.

