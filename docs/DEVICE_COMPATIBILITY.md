# Device Compatibility

`fbsy` talks to biometric devices through the classic ZKTeco/ZKSoftware local
SDK protocol. This is the same protocol used by many devices on port `4370`.

The bridge does not speak every biometric vendor protocol. Compatibility means
the device must expose a local PC/SDK connection that can return attendance logs
with ZKTeco-style commands.

## Confirmed Devices

| Vendor | Model | Firmware / platform | Status |
| --- | --- | --- | --- |
| ZKTeco | F22 / F22-ID | `Ver 6.60 Apr 27 2017`, `ZLM60_TFT` | Confirmed with real hardware |

Confirmed F22 behavior:

- Device serial read successfully: `BOCK194660021`
- Attendance pull tested with 1,000+ records
- HRMS forwarding tested against a local development HRMS webhook
- Supports buffered-read compatibility where the device returns `ACK_OK` / `2000`
  during `CMD_PREPARE_BUFFER`

## Likely Compatible Devices

Devices are likely compatible when they work with the Python `pyzk` package or
with ZKTeco PC software through a local IP and port.

Likely compatible families include many ZKTeco/ZKSoftware models such as:

- F18 / F22 access-control terminals
- K-series attendance terminals
- iClock series
- uFace series
- SpeedFace series
- MB series
- SilkBio series
- UA series
- OEM/rebranded devices that run ZKTeco-compatible firmware

Treat these as candidates until tested. ZKTeco firmware varies by region,
installer, and reseller.

## Not Supported By This Adapter

These modes/protocols are not the local ZKTeco SDK protocol:

- ADMS/cloud-server push mode only
- HTTP-only device APIs
- Hikvision biometric protocol
- Anviz protocol
- Suprema protocol
- Any OEM firmware that does not expose the ZKTeco PC/SDK connection

Cloud Server / ADMS can be useful for other products, but `fbsy bridge` pulls
attendance locally from the device and then forwards it to HRMS.

## Required Device Settings

On ZKTeco-style devices, check the device screen for settings like:

```text
Comm -> Ethernet
IP Address: static or DHCP-reserved LAN IP
Subnet:     same LAN as the bridge machine
Gateway:    router IP
DHCP:       off or router reservation recommended
```

```text
Comm -> PC Connection
Device ID:  usually 1
TCP Comm:   usually 4370
HTTPS:      off
Comm Key:   0 unless the device has a configured communication password
```

If the device has:

```text
Comm -> Cloud Server Settings
```

disable cloud/ADMS for local pull testing unless your installer requires it for a
separate workflow.

After changing communication settings, fully power-cycle the device before
testing from the bridge machine.

## Qualification Flow

Use this flow for every new device model before promising support.

1. Confirm the bridge machine can reach the device:

```bash
ping DEVICE_IP
```

2. Check the likely SDK port:

```bash
nc -vz DEVICE_IP 4370
```

3. Scan with `fbsy`:

```bash
fbsy scanner scan --host DEVICE_IP --include-open
fbsy scanner scan --host DEVICE_IP --udp --include-open
```

4. Configure a test device entry:

```bash
fbsy bridge config setup
```

5. Read live device info:

```bash
fbsy bridge devices test DEVICE_CODE
fbsy bridge devices info DEVICE_CODE
```

6. Pull once without clearing attendance:

```bash
fbsy bridge sync --device DEVICE_CODE
```

Keep `clearAttendanceAfterSync` disabled until HRMS mappings and uploads are
verified.

## Compatibility Signals

Good signs:

- `nc` succeeds on port `4370`
- `fbsy bridge devices info` shows serial, firmware, platform, users, and record counts
- `fbsy bridge sync --device CODE` returns `ok: true`
- Python `pyzk` can connect and read attendance

Bad or ambiguous signs:

- `Connection refused` on `4370`: the device is reachable but SDK/PC connection is closed
- `8000` or another port is open but `4370` is closed: often a different internal or
  cloud service, not attendance pull
- device answers ping but every useful TCP port refuses: check PC Connection, cloud
  mode, firmware restrictions, and router/client isolation
- `TCP packet invalid`: the port is open but not speaking the expected ZKTeco protocol

## Protocol Variants Covered

The Rust adapter intentionally mirrors `pyzk` behavior for common ZKTeco variants:

- inline `CMD_DATA` response from `CMD_PREPARE_BUFFER`
- `CMD_PREPARE_DATA` followed by `CMD_READ_BUFFER` chunks
- `CMD_ACK_OK` / `2000` as a valid buffered-read prepare response
- TCP prepared chunk streams that send `CMD_DATA` packets followed by `CMD_ACK_OK`
- UDP prepared chunk streams
- attendance record decoding for 8-byte, 16-byte, and 40-byte stored log formats

Regression tests live in:

```text
tests/device_protocol_tests.rs
```

## Adding A New Confirmed Model

When a new model is tested successfully, record:

- vendor and model
- firmware version
- platform/device name
- serial prefix if useful
- TCP or UDP mode
- port
- communication password behavior
- record count tested
- whether HRMS forwarding was verified

Then add the model to the Confirmed Devices table in this document.
