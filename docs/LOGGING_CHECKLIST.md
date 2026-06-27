# Logging Checklist

Use this checklist when testing `fbsy` services, mock devices, and the dashboard.

## Where Logs Live

- [ ] `fbsy status <instance>` shows the exact log file path.
- [ ] `fbsy logs <instance> -n 100` prints recent lines for one instance.
- [ ] `fbsy logs <instance> --follow` follows new lines in a shell.
- [ ] `fbsy dashboard` shows selected-service logs in the bottom pane.
- [ ] In the dashboard, `a` toggles combined logs from all running instances.
- [ ] In the dashboard, `PgUp` scrolls older logs and `PgDn` scrolls newer logs.
- [ ] In the dashboard, `Home` jumps to the oldest loaded log line and `End` jumps to newest.

## Bridge Service Logs

When `fbsy run bridge` is running, `fbsy logs bridge -n 100` should show:

- [ ] Service startup, config path, configured device count, job polling state, and auto-update state.
- [ ] One startup line per device showing device address, sync interval, and clear-after-sync setting.
- [ ] Boot sync scheduling for each device.
- [ ] Scheduler startup for each device.
- [ ] Sync start timestamp.
- [ ] Device connection attempt with device IP, port, timeout, UDP flag, and ping flag.
- [ ] Device connected.
- [ ] Attendance pull started.
- [ ] Raw attendance record count pulled from the device.
- [ ] HRMS event count after mapping raw records.
- [ ] HRMS forward attempt count.
- [ ] HRMS accepted event count.
- [ ] Clear-attendance success, failure, or disabled/kept decision.
- [ ] Device disconnected.
- [ ] Final sync result with `ok`, pulled count, forwarded count, clear flag, and message.
- [ ] Manual HTTP sync logs for `POST /sync` and `POST /sync?device=CODE`.
- [ ] HRMS job poller startup and pending-job processing when job polling is configured.

## Mock ZKTeco Logs

When `fbsy run zkteco --name dev1 --port 4370 --records 5` is running,
`fbsy logs dev1 -n 100` should show:

- [ ] Startup address using the machine LAN IP.
- [ ] Setup wizard values: device IP, port, device unique code, API key, organization ID, password, serial, firmware.
- [ ] Client connected from address.
- [ ] `CMD_CONNECT` and session ID.
- [ ] `CMD_GET_FREE_SIZES` with attendance count.
- [ ] `CMD_GET_VERSION`.
- [ ] `CMD_OPTIONS_RRQ` values such as serial, platform, and device name.
- [ ] `CMD_ATTLOG_RRQ` with attendance record count.
- [ ] `CMD_USERTEMP_RRQ` when users are requested.
- [ ] `CMD_DB_RRQ` when templates are requested.
- [ ] `CMD_CLEAR_ATTLOG` when attendance is cleared.
- [ ] Client disconnected.

## Scanner Logs

When `fbsy run scanner --interval 300` is running, `fbsy logs scanner -n 100` should show:

- [ ] Startup target CIDR or host list.
- [ ] Port, scan interval, TCP timeout, and device protocol timeout.
- [ ] Scan cycle timestamp.
- [ ] Number of target hosts being scanned.
- [ ] Number of candidates found.
- [ ] Number of confirmed attendance devices.
- [ ] For each confirmed device: IP, port, serial, firmware, user count, template count, record count, and suggested device code.
- [ ] For `--include-open`, open-port hosts that failed ZKTeco probing.

## Mock HRMS Logs

When `fbsy run hrms --name hrms1 --port 8800` is running,
`fbsy logs hrms1 -n 100` should show:

- [ ] Startup URL using the machine LAN IP.
- [ ] Setup wizard values: webhook URL, base URL, and mock token guidance.
- [ ] Request method/path for every call.
- [ ] Webhook payload event count.
- [ ] Pretty-printed webhook payload.
- [ ] Stored payload count after a webhook POST.
- [ ] `/events` request and returned stored payload count.
- [ ] `/reset` request and reset confirmation.
- [ ] `/health` request.
- [ ] Pending-job poll request returning zero jobs.
- [ ] Job-complete request when the bridge reports HRMS job completion.

## End-To-End Mock Test

```bash
fbsy run hrms --name hrms1 --port 8800
fbsy run zkteco --name dev1 --port 4370 --records 5
fbsy bridge config setup
fbsy run bridge
fbsy bridge sync --once
fbsy logs bridge -n 100
fbsy logs dev1 -n 100
fbsy logs hrms1 -n 100
```

Expected result:

- [ ] Bridge log says it pulled `5` raw attendance records.
- [ ] Bridge log says it mapped `5` HRMS events.
- [ ] Bridge log says HRMS accepted `5` events.
- [ ] ZKTeco mock log says it served `5` attendance records.
- [ ] HRMS mock log says it received webhook payload with `5` events.
- [ ] Dashboard can show the same lines and scroll older/newer log output.
