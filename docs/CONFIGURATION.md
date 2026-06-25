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

## Secret Handling

The CLI must redact:

- `apiKey`
- `deviceCode`

Never log raw secrets.

