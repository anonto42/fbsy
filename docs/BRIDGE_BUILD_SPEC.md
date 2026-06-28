# FingerBridge — Full Build Spec

**Status:** authoritative build spec. Written against the **finalized** HRMS
contract (after the webhook-only / job-polling cleanup — see the HRMS repo
`docs/BIOMETRIC_WEBHOOK_REBUILD_PLAN.md`).

This file supersedes the HTTP-contract details in the older planning docs
(`FULL_WORKFLOW_PLAN.md`, `IMPLEMENTATION_PLAN.md`, `old/RUST_IMPLEMENTATION_PLAN.md`).
Those remain useful for architecture/learning; **this** file is the source of
truth for what the bridge must send and receive.

---

## 0. What the bridge is

A single native executable that runs on the office LAN next to one or more
ZKTeco devices. It is the **only** thing that talks to the devices. It makes
**outbound-only** HTTPS calls to HRMS (works behind NAT, no public URL needed).

It does exactly two jobs:

```text
1. PUSH attendance up      ── pull punches from device → POST webhook → HRMS
2. PULL template jobs down ── poll HRMS for jobs → run on device → report back
```

```text
ZKTeco device(s) ──TCP 4370──► fingerbridge ──HTTPS──► HRMS (api/v1)
                                   │  - scheduler (per-device interval)
                                   │  - sync engine (pull → map → forward → clear?)
                                   │  - job poller (PUSH_USER / PULL_TEMPLATES)
                                   │  - local HTTP API on 127.0.0.1 (health/sync)
```

---

## 1. THE HRMS CONTRACT (exact — match byte-for-byte)

HRMS runs behind a global prefix `api/v1`. Every HRMS response is wrapped by a
transform interceptor into an envelope:

```jsonc
{ "statusCode": 200, "message": "Success", "data": <actual payload> }
```

**→ Always read the real payload from the `data` field.**

### 1.1 POST attendance webhook  (push)

```
POST  {vpsWebhookUrl}          // full URL is in config, e.g.
                               // https://api.example.com/api/v1/biometric-devices/webhook
Content-Type: application/json
```

Request body:

```jsonc
{
  "organizationId": 1,            // number — scopes device lookup
  "deviceCode": "GATE-01",        // string — must match a device in HRMS for that org
  "apiKey": "plain-text-key",     // string — plaintext; HRMS compares (timing-safe)
                                  //          against the AES-256-GCM-decrypted stored key
  "events": [                     // array, MAX 500 per request
    {
      "deviceEmployeeId": "001",            // string — the user id enrolled on the device
      "timestamp": "2026-06-25T02:30:00Z",  // string — ISO 8601 (see §4 on timezone)
      "eventType": "check_in",              // "check_in" | "check_out"
      "verificationMethod": "fingerbridge" // optional string
    }
  ]
}
```

Success response:

```jsonc
{ "statusCode": 200, "message": "Success", "data": { "received": 2 } }
```

→ Parse `data.received` (number accepted).

Auth / errors:
- `401 Unauthorized` — unknown device, no key configured, or key mismatch.
- The device is matched by `deviceCode` **AND** `organizationId` **AND** `isActive=true`.
- Events are processed async (queue). `received` = count accepted for processing,
  not count applied to attendance.

### 1.2 GET pending jobs  (poll)

```
GET  {hrmsBaseUrl}/biometric-devices/pending-jobs?deviceCodes=GATE-01,GATE-02
Authorization: Bearer <apiKey>
```

- `hrmsBaseUrl` example: `https://app.example.com/api/v1`
- `deviceCodes` — comma-separated list of this bridge's device codes.
- **Bearer token** — ⚠️ HRMS authenticates the bridge by matching the bearer
  against the **plaintext apiKey of one of the devices in `deviceCodes`**
  (`BiometricJobService.authenticateBridge`). There is **no separate bridge
  token** in HRMS today. So send one device's `apiKey` as the bearer. (All
  devices on one bridge are assumed to be in the same HRMS org.)

Success response:

```jsonc
{ "statusCode": 200, "message": "Success",
  "data": [
    { "id": "uuid", "type": "PUSH_USER",      "deviceCode": "GATE-01", "payload": { ... } },
    { "id": "uuid", "type": "PULL_TEMPLATES", "deviceCode": "GATE-02", "payload": null }
  ] }
```

→ Parse `data` (array). Fetching marks the jobs `IN_PROGRESS` server-side.

### 1.3 POST job completion  (report back)

```
POST {hrmsBaseUrl}/biometric-devices/jobs/{jobId}/complete
Content-Type: application/json
```

(This route is public — no auth required — but sending the same Bearer is harmless.)

Body for a **PUSH_USER** result:

```jsonc
{ "ok": true }
// or on failure:
{ "ok": false, "error": "device unreachable" }
```

Body for a **PULL_TEMPLATES** result (include the templates so HRMS stores them):

```jsonc
{
  "ok": true,
  "templates": [
    {
      "uid": 1,                 // device internal slot
      "fid": 0,                 // finger index 0-9
      "userId": "001",          // the device user id (= deviceEmployeeId mapping)
      "name": "Alice",          // display name (may be "")
      "templateBytes": "<base64>"  // raw template bytes, base64
    }
  ]
}
```

HRMS only stores templates for `userId`s that have an active mapping; others are
ignored. A job already `DONE`/`FAILED` returns 400 if completed again.

### 1.4 Job payloads (what the device adapter must do)

| Job type | Incoming `payload` | Bridge action on device | Reported back |
| --- | --- | --- | --- |
| `PUSH_USER` | `{ userId, uid, fid, templateBytes(base64), name? }` | write user + finger template to the device | `{ ok }` |
| `PULL_TEMPLATES` | `null` | read all users + finger templates | `{ ok, templates:[...] }` |

---

## 2. config.json

Keep the same field names as the Python bridge so existing installs migrate
cleanly. Multi-device format:

```jsonc
{
  "vpsWebhookUrl": "https://api.example.com/api/v1/biometric-devices/webhook",
  "bridgePort": 7431,

  // Optional — enables the template job poller. Omit all three to disable polling.
  "hrmsBaseUrl": "https://app.example.com/api/v1",
  "hrmsApiToken": "<a device apiKey>",   // see §1.2 — must equal a device apiKey
  "jobPollIntervalSeconds": 30,

  "devices": [
    {
      "deviceIp": "192.168.1.100",
      "devicePort": 4370,
      "devicePassword": 0,
      "deviceTimeout": 15,
      "deviceForceUdp": false,
      "deviceOmitPing": true,
      "deviceTimezone": "+06:00",
      "deviceCode": "GATE-01",
      "apiKey": "webhook-api-key-for-this-device",
      "organizationId": 1,
      "syncIntervalSeconds": 300,
      "clearAttendanceAfterSync": false
    }
  ]
}
```

Per-device defaults (apply when missing): `devicePort=4370`, `devicePassword=0`,
`deviceTimeout=15`, `deviceForceUdp=false`, `deviceOmitPing=true`,
`deviceTimezone=UTC` (optional; fixed offset like `+06:00` for the device clock's
zone — naive device timestamps are interpreted in this offset),
`organizationId=1`, `syncIntervalSeconds=300` (clamp up to 5 if smaller),
`clearAttendanceAfterSync=false`. Top-level `bridgePort=7431`,
`jobPollIntervalSeconds=30` (clamp up to 5).

Validation: `vpsWebhookUrl` must be http(s); `deviceIp`, `deviceCode`, `apiKey`
required per device; `devicePort`/`bridgePort` in `1..=65535`; `deviceTimeout`
in `1..=120`; device codes unique. Also accept the legacy single-device flat
config (all keys at top level → wrap into `devices[0]`).

> **Open item (confirm):** `hrmsApiToken` semantics. HRMS authenticates job
> polling with a *device* apiKey (§1.2). Options: (a) drop `hrmsApiToken`, have
> the poller use `devices[0].apiKey`; (b) keep the field and require the user to
> set it to a device apiKey. Recommend **(a)** for less confusion. Decide before
> building the poller.

---

## 3. Device protocol surface (the ZKTeco adapter)

The sync engine and job poller depend only on these trait methods — keep the
real protocol behind a trait so everything else is testable with fakes.

```rust
pub struct RawAttendance { pub user_id: String, pub timestamp: String, pub punch: i64 }
pub struct DeviceUser    { pub uid: u32, pub user_id: String, pub name: String }
pub struct FingerTemplate{ pub uid: u32, pub fid: u8, pub user_id: String,
                           pub name: String, pub template: Vec<u8> }

pub trait DeviceClient {
    fn pull_attendance(&mut self) -> Result<Vec<RawAttendance>, DeviceError>;
    fn clear_attendance(&mut self) -> Result<(), DeviceError>;
    fn get_templates(&mut self) -> Result<Vec<FingerTemplate>, DeviceError>; // users+fingers joined
    fn push_user_template(&mut self, user: &DeviceUser, finger: &FingerTemplate)
        -> Result<(), DeviceError>;
    fn disconnect(&mut self);
}

pub trait DeviceConnector {
    fn connect(&self, cfg: &BridgeDeviceConfig) -> Result<Box<dyn DeviceClient>, DeviceError>;
}
```

ZKTeco protocol options (decide in the device phase): (A) a maintained Rust ZK
crate if one fits; (B) implement the small protocol surface; (C) temporarily
shell out to the Python `pyzk` script while the Rust protocol matures. Build all
non-device layers first against a **fake** adapter, then plug in the real one.

---

## 4. Domain mapping (pure, unit-tested)

Raw device punch → HRMS event, preserving the Python behavior exactly:

- `eventType`: punch `0` or `4` → `check_in`; everything else → `check_out`.
- `deviceEmployeeId` = the device `user_id` as a string; skip records with empty
  user id or unparseable timestamp.
- `timestamp` → ISO 8601 string. The device clock is local wall-time; emit it as
  an offset-aware ISO string. ⚠️ **Timezone caveat:** the Python bridge attaches
  UTC to naive device timestamps. HRMS then computes the employee's local day
  from the **employee's** timezone. Confirm during testing that punches land on
  the correct local day; if devices are not on UTC, the bridge must apply the
  device's real offset so the instant is correct.
- Sort events ascending by timestamp before sending.
- `verificationMethod`: `"fingerbridge"`.

---

## 5. Sync lifecycle (per device) + safety invariant

```text
acquire per-device lock (reject overlap)
  connect → pull_attendance
  map → HRMS events (skip malformed, sort)
  if events empty: record result, done
  else: forward_to_hrms in batches of 500 (retry policy §6)
        if upload fully succeeded AND clearAttendanceAfterSync:
            clear_attendance      ← ONLY here
  disconnect (always)
  store last_result (sanitize secrets from errors: redact apiKey, deviceCode)
```

**Non-negotiable invariant (test before any real clear code exists):**

```text
Never clear device attendance unless the webhook upload succeeded.
```

Required safety tests: webhook failure does not clear; partial-batch failure does
not clear; clear runs only after all batches succeed; error output never contains
`apiKey` or a secret `deviceCode`.

---

## 6. HRMS webhook client (reqwest)

- One reused `reqwest::Client` (connection pooling), `rustls-tls`, JSON.
- Batch events into chunks of **500**.
- Payload per batch: `{ organizationId, deviceCode, apiKey, events: chunk }`.
- Retry **network errors, HTTP 429, HTTP 5xx**; backoff (e.g. `2 * attempt` s),
  a few attempts. **Do not retry normal 4xx** (bad key / payload).
- Parse `data.received` from the response envelope.

---

## 7. Job poller (optional, when hrmsBaseUrl + token set)

```text
every jobPollIntervalSeconds:
  GET pending-jobs?deviceCodes=<all codes>  (Bearer = a device apiKey)
  for each job:
    PUSH_USER      → connect device → push_user_template → complete {ok}
    PULL_TEMPLATES → connect device → get_templates → complete {ok, templates}
    unknown type   → complete {ok:false, error}
  always complete every job (success or error); truncate/sanitize error strings
```

---

## 8. CLI commands

```bash
fingerbridge doctor                 # readiness + config path/status
fingerbridge setup                  # interactive config wizard (dialoguer)
fingerbridge once [--device CODE]   # one sync (all or one device), print JSON, exit 0/1
fingerbridge serve [--interval N] [--no-poll]   # HTTP API + schedulers + poller
fingerbridge config validate | show | path
fingerbridge devices list | test CODE
fingerbridge webhook test CODE
fingerbridge service install | uninstall | status   # systemd/launchd/Task Scheduler
```

Compatibility aliases: `--once`, `--setup`, `--interval`, `--install-autostart`,
`--uninstall-autostart`. Exit `0` success / `1` failure for `once`.

---

## 9. Local HTTP API (serve mode, axum, 127.0.0.1:bridgePort)

Preserve the Python bridge surface:

```text
GET  /health         → { status, agent, version, runtime:"rust", vpsWebhookUrl,
                         deviceCount, devices:[{deviceCode, deviceIp, syncing, lastResult}] }
POST /sync           → start sync for all devices → { started:[...], skipped:[...] }
POST /sync?device=X  → 202 started | 404 unknown device | 429 already syncing
OPTIONS *            → CORS preflight (204)
```

---

## 10. Module layout (map onto the existing scaffold)

```text
src/
├── main.rs                 # tiny entry: logging, Cli::parse, cli::run
├── lib.rs                  # pub mod ...
├── cli/        args, command, dispatch
├── config/     model (BridgeConfig, BridgeDeviceConfig), error, impls (defaults+validate+redact)
├── domain/     attendance (RawAttendance), event (mapping), sync_result, template
├── ports/      device (DeviceClient/DeviceConnector), hrms (HrmsClient), config_store, clock
├── application/ doctor, setup, sync_once, serve, config, job_poll
├── adapters/   device_zkteco, device_fake, hrms_reqwest, hrms_fake, config_file, http_axum, autostart
├── runtime/    scheduler (per-device interval), job_poller, shutdown, state (DeviceSyncState)
└── support/    paths, logging, redaction
```

`AppState { config, per-device SyncState, hrms: Arc<dyn HrmsClient>, connector: Arc<dyn DeviceConnector> }`.
Async rules: reuse one reqwest Client; don't hold a lock across `.await`; ZKTeco
calls behind a trait (may be blocking — use `spawn_blocking` if needed).

---

## 11. Cargo dependencies

```toml
clap = { version = "4", features = ["derive"] }
anyhow = "1"
thiserror = "2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tracing = "0.1"
tracing-subscriber = "0.3"
# added as phases need them:
reqwest = { version = "0.12", features = ["json", "rustls-tls"] }
tokio = { version = "1", features = ["rt-multi-thread", "macros", "signal", "time", "sync"] }
axum = "0.7"
tower-http = { version = "0.6", features = ["cors"] }
dialoguer = "0.11"   # setup wizard
base64 = "0.22"      # template bytes
directories = "6"    # native install paths (later)
```

---

## 12. Build order (inside-out, each phase ends green)

Every phase ends with: `cargo fmt --check && cargo clippy && cargo test`.

1. **Config parity** — `BridgeConfig` + `BridgeDeviceConfig`, defaults, validation,
   legacy single-device load, redaction. `config validate/show/path`, `devices list`.
2. **Domain mapping** — punch→event, ISO timestamp, skip malformed, sort. Pure tests.
3. **Ports + fakes** — `DeviceClient`/`DeviceConnector`/`HrmsClient` traits + fakes.
4. **Sync engine** — `DeviceSyncState` (per-device lock, last_result), `sync_once`
   over fakes, **clear-after-success safety tests**. `once`, `once --device`.
5. **HRMS webhook client** — reqwest, batch 500, retry policy, parse `data.received`.
   `webhook test`.
6. **Local HTTP API** — axum `/health`, `/sync`, `/sync?device=`, CORS.
7. **Scheduler + serve** — per-device interval, boot sync, graceful shutdown,
   last-result state file, logging.
8. **Job poller** — pending-jobs + complete; `PUSH_USER`, `PULL_TEMPLATES` over fakes.
9. **Setup wizard + doctor** — dialoguer prompts, connection/webhook tests, atomic save.
10. **Service installer** — systemd unit / launchd plist / Windows Task Scheduler.
11. **Real ZKTeco adapter** — replace the fake; manual device tests.
12. **Packaging** — release profile, CI matrix (linux/macos/win), checksums, install scripts.

---

## 13. Contract parity checklist (Rust ↔ HRMS)

| Item | Must equal |
| --- | --- |
| webhook body keys | `organizationId, deviceCode, apiKey, events[]` |
| event keys | `deviceEmployeeId, timestamp, eventType, verificationMethod?` |
| eventType values | `check_in` / `check_out` |
| batch size | `500` |
| webhook response read | `data.received` |
| pending-jobs response read | `data` (array) |
| job types | `PUSH_USER`, `PULL_TEMPLATES` |
| PUSH_USER payload | `userId, uid, fid, templateBytes(base64), name?` |
| PULL_TEMPLATES return | `templates:[{uid, fid, userId, name, templateBytes(base64)}]` |
| job-poll auth | `Authorization: Bearer <a device apiKey>` |
| retry | network / 429 / 5xx only |
| safety | never clear device unless upload succeeded |

---

## 14. Open items to confirm before building

1. **`hrmsApiToken` vs device apiKey** for job-poll auth (§2 open item) — recommend
   the poller uses a device apiKey and we drop the separate token.
2. **ZKTeco protocol approach** (§3) — Rust crate vs. implement vs. temporary
   Python shell-out.
3. **Timezone** (§4) — confirm device clocks/UTC handling so punches land on the
   right local day.
