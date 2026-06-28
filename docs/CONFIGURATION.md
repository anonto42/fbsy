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
- `deviceTimezone` (optional) must be `UTC`/`Z` or a fixed UTC offset such as
  `+06:00`, `-05:30`, `+0600`, or `+06`. It is the timezone of the **device's own
  clock**: ZKTeco devices report naive wall-clock timestamps with no offset, so the
  bridge applies this offset to map each punch to the correct calendar instant before
  sending it to the HRMS. Omit it (or set `UTC`) only if the device clock is on UTC.
  DST-aware IANA names (e.g. `Asia/Dhaka`) are intentionally not supported — use the
  fixed offset for your region.

## Secret Handling

The CLI must redact:

- `apiKey`
- `deviceCode`

Never log raw secrets.

