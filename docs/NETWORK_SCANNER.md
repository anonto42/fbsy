# Network Scanner

`fbsy scanner` helps find attendance devices before configuring the bridge.

It is not a general network inventory tool. It does two focused checks:

1. Test whether the target host has the ZKTeco port open, normally `4370`.
2. Try the ZKTeco protocol handshake and `device_info` read.

## One-Shot Scan

```bash
fbsy scanner scan
fbsy scanner scan --cidr 192.168.1.0/24
fbsy scanner scan --host 192.168.1.50
fbsy scanner scan --host 192.168.1.50 --json
```

Default behavior:

- Uses this machine's LAN `/24` when `--cidr` and `--host` are omitted.
- Scans port `4370`.
- Shows only confirmed ZKTeco-like attendance devices.
- Refuses networks larger than `/24` so an accidental scan does not sweep too much.

Useful flags:

```bash
--include-open       show hosts where the port is open but ZKTeco probing failed
--timeout-ms 500     TCP port-probe timeout
--device-timeout 3   ZKTeco protocol timeout
--password 1234      ZKTeco communication password
--udp                use UDP for the deeper protocol probe
```

## Background Service

```bash
fbsy run scanner --interval 300
fbsy logs scanner -n 100
fbsy close scanner
```

The service writes repeated discovery results into `logs/scanner.log`. It also appears in:

```bash
fbsy show
fbsy dashboard
fbsy status scanner
```

## Bridge Setup Use

For each confirmed device, the scanner prints:

- IP and port
- serial, firmware, platform, and device name when available
- user/template/attendance record counts
- a suggested `deviceCode`
- a suggested device config block

Use those values when running:

```bash
fbsy bridge config setup
```

The scanner cannot know the HRMS API key. It prints `CHANGE_ME` for `apiKey`; replace that
with the real value from HRMS.
