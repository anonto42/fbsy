# Security And Safety

## Secrets

Sensitive config values:

- `apiKey`
- `deviceCode`

Rules:

- never log raw secrets
- redact secrets in `config show`
- sanitize secrets from error messages
- never commit `config.json`

## Device Safety

The bridge may clear attendance records from the device only when:

1. events were successfully uploaded to HRMS
2. `clearAttendanceAfterSync` is `true`

If the webhook fails, device attendance must stay untouched.

## Network Scope

The bridge should bind the local API to:

```text
127.0.0.1
```

unless a future release explicitly supports LAN exposure with authentication.

## Webhook Retry Safety

Retry only:

- network errors
- HTTP `429`
- HTTP `5xx`

Do not retry normal `4xx` errors because they usually mean invalid credentials or malformed payload.

