# Packaging

Rust should produce one native binary per platform.

## Local Build

```bash
cargo build --release
```

Output:

```text
target/release/fingerbridge
```

On Windows:

```text
target/release/fingerbridge.exe
```

## Planned Release Artifacts

```text
fingerbridge-linux-x86_64
fingerbridge-linux-aarch64
fingerbridge-macos-aarch64
fingerbridge-macos-x86_64
fingerbridge-windows-x86_64.exe
```

## Runtime Files

The binary should run with:

```text
fingerbridge
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

