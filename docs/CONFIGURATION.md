# Configuration

The Rust bridge keeps the same `config.json` field names as the Python bridge so existing installs can migrate cleanly.

## Example

See [config.example.json](../config.example.json).

## Required Fields

```text
deviceIp
deviceCode
apiKey
vpsWebhookUrl
```

## Defaults

| Field | Default |
| --- | --- |
| `devicePort` | `4370` |
| `devicePassword` | `0` |
| `deviceTimeout` | `15` |
| `deviceForceUdp` | `false` |
| `deviceOmitPing` | `true` |
| `deviceTimezone` | `UTC` (offset `+00:00`) |
| `eventTypeMode` | `punchCode` |
| `organizationId` | `1` |
| `clearAttendanceAfterSync` | `false` |
| `port` | `7431` |
| `syncIntervalSeconds` | `300` |

## Validation Rules

- `deviceIp` cannot be empty.
- `deviceCode` cannot be empty.
- `apiKey` cannot be empty.
- `vpsWebhookUrl` must start with `http://` or `https://`.
- `deviceTimeout` must be between `1` and `120`.
- `syncIntervalSeconds` must be at least `5`.
- `deviceTimezone` (optional) must be `UTC`/`Z`, a fixed UTC offset such as
  `+06:00`, `-05:30`, `+0600`, or `+06`, or an IANA timezone such as
  `Asia/Dhaka`. It is the timezone of the **device's own clock**: ZKTeco devices
  report naive wall-clock timestamps with no offset, so the bridge applies this
  timezone to map each punch to the correct calendar instant before sending it to
  the HRMS. Omit it (or set `UTC`) only if the device clock is on UTC. IANA names
  are resolved to the current offset when the sync runs.
- `eventTypeMode` controls how FBSY derives `check_in` / `check_out`.
  Use `punchCode` when the device's punch code is reliable (`0`/`4` are treated as
  check-in, all other codes as check-out). Use `firstInLastOut` for devices that
  send the same punch code for every swipe: per employee per local day, the first
  punch is sent as check-in and later punches are sent as check-out.

## Secret Handling

The CLI must redact:

- `apiKey`
- `deviceCode`

Never log raw secrets.
