# Local Test Plan — prove the backend works with no hardware

Everything runs on one machine using fbsy's built-in mock servers. No ZKTeco
device, no HRMS deployment, no network. Each stage has a **proof** — a thing
you can see that shows the backend really did its job.

The mock servers are internal test entrypoints of the same `fbsy` binary:

```bash
fbsy __service-run zkteco --port 14370 --records 5   # mock fingerprint device
fbsy __service-run hrms   --port 18800               # mock HRMS webhook server
```

Use an isolated HOME so your real config, logs, and boot units are untouched:

```bash
export FBSY=~/Code/levelaxis/fbsy/target/release/fbsy
export HOME=/tmp/fbsy-test-home     # everything below stays inside here
mkdir -p "$HOME"
```

---

## Stage 1 — mock device + mock HRMS are up

```bash
$FBSY __service-run zkteco --port 14370 --records 5 > /tmp/zkteco.log 2>&1 &
$FBSY __service-run hrms   --port 18800             > /tmp/hrms.log   2>&1 &
sleep 1
curl -s http://127.0.0.1:18800/health    # → {"ok":true,"agent":"mock-hrms"}
curl -s http://127.0.0.1:18800/events    # → []   (nothing received yet)
```

**Proof:** health answers; events list is empty.

## Stage 2 — configure the bridge against the mocks

Write the test config directly (or run `$FBSY setup` and type the same values):

```bash
mkdir -p "$HOME/Library/Application Support/fbsy/config"
cat > "$HOME/Library/Application Support/fbsy/config/config.json" <<'EOF'
{
  "bridgeMode": "pull",
  "vpsWebhookUrl": "http://127.0.0.1:18800/webhook",
  "bridgePort": 17431,
  "autoStartOnBoot": false,
  "devices": [{
    "deviceIp": "127.0.0.1", "devicePort": 14370,
    "deviceCode": "MOCK-GATE-01", "apiKey": "mock-key",
    "organizationId": 1, "syncIntervalSeconds": 30,
    "deviceOmitPing": true, "clearAttendanceAfterSync": false
  }]
}
EOF
$FBSY config        # → ✔ Config is valid + redacted view
```

**Proof:** `fbsy config` prints “Config is valid”.

## Stage 3 — one-shot sync (the core pipeline)

```bash
$FBSY sync
```

Expected output — the whole job in one JSON:

```json
{ "ok": true, "deviceCode": "MOCK-GATE-01",
  "pulled": 5, "forwarded": 5, "deviceAttendanceCleared": false, ... }
```

Now look at what the HRMS actually received — this is the equivalent of the
old zkteco-bridge's `device-attendance.json`, the visible punch data:

```bash
curl -s http://127.0.0.1:18800/events
# → [{"deviceCode":"MOCK-GATE-01","organizationId":1,"events":[
#      {"deviceEmployeeId":"1001","eventType":"check_in","timestamp":"..."},
#      {"deviceEmployeeId":"1002","eventType":"check_out", ...}, ... ]}]
```

**Proof:** `pulled 5, forwarded 5`, and the five punches visible on the HRMS
side with employee ids, check_in/check_out types, and timestamps.

## Stage 4 — background runner

```bash
curl -s http://127.0.0.1:18800/reset     # clear received events
$FBSY start                              # detached background process
$FBSY status                             # → running · BOOT - · port 17431
$FBSY logs -f                            # watch scheduled syncs land every 30s
# Ctrl-C to stop following, then:
curl -s http://127.0.0.1:18800/events | grep -c deviceEmployeeId
```

**Proof:** the event count grows on its own every 30 seconds — nobody is
touching anything; the background runner is doing the job. Also:
`curl http://127.0.0.1:17431/health` shows the runner's own live status with
`lastResult` per device.

## Stage 5 — crash recovery + reboot (production machine)

On the machine where the bridge runs supervised (`autoStartOnBoot: true`,
`fbsy status` shows `BOOT on`) — using the real HOME, not the test one:

```bash
fbsy status                        # note the PID
kill -9 <PID>                      # simulate a crash
sleep 3; fbsy status               # → running again, NEW pid — launchd revived it
```

Then reboot (or log out/in) and run `fbsy status` — running again, `BOOT on`,
no manual start.

**Proof:** the OS supervisor restarts the bridge after a kill and after a
reboot; the same config keeps working.

## Cleanup

```bash
$FBSY stop
pkill -f "__service-run"
unset HOME FBSY     # or just close the shell
rm -rf /tmp/fbsy-test-home
```
