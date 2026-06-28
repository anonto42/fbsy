# fbsy — Production Build Plan & Cold-Handoff Spec

> **Purpose.** This single file lets *any* engineer or AI agent continue this project to
> **production-grade, 100%-quality** completion **without prior context**. It is the source of
> truth for: what fbsy is, the rules you must never break, the current state, the gaps, and an
> ordered, executable checklist to close them.
>
> **Audience.** A cold-start implementer. Read §0 → §3 before writing a single line of code.
> **Companion framework:** the reusable engineering playbook in [`docs/framework/`](framework/00-INDEX.md).
> Each task below cites the relevant chapter.
>
> **Status as read:** `fingerbridge` v0.2.18, ~9,300 LOC Rust. This plan reflects the **actual
> code**, which is far more complete than the older planning docs
> ([IMPLEMENTATION_PLAN.md](IMPLEMENTATION_PLAN.md), [FULL_WORKFLOW_PLAN.md](FULL_WORKFLOW_PLAN.md))
> imply. Where they conflict, **the code + this file win**; [BRIDGE_BUILD_SPEC.md](BRIDGE_BUILD_SPEC.md)
> remains the source of truth for the HRMS HTTP contract.

---

## 0. Prime directive (read first)

1. **Never break the safety invariant** (§3.1) or the **HRMS contract** (§3.2). They are
   load-bearing; everything else is negotiable.
2. **Respect the architecture boundaries** (§4). Domain stays pure; I/O stays in adapters;
   the CLI only dispatches.
3. **Every change ends green:** `cargo fmt --all -- --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test --all`.
4. **Work the checklist in order** (§8): P0 (release blockers) → P1 (hardening) → P2 (polish).
   Each task has acceptance criteria + a verify command — do not mark done until they pass.
5. **When a task needs a human decision (§10), stop and ask** — do not guess on timezone,
   signing keys, or auth model.

---

## 1. What fbsy is

A single native executable (`fbsy`) that runs unattended on an office-LAN machine and bridges
**on-prem ZKTeco biometric devices → an HRMS cloud webhook**. It is also a small **service
manager**: one binary starts/stops/monitors long-running background "services" by name.

```
GATE-01 ─┐
GATE-02 ─┼─ TCP 4370 ─▶ fbsy (office machine) ─ HTTPS JSON ─▶ HRMS webhook (api/v1)
FLOOR-3 ─┘                  │ per-device scheduler · sync engine · job poller
                           │ local HTTP API on 127.0.0.1 · self-update
```

**Services it manages** ([`src/services/mod.rs`](../src/services/mod.rs)):
| Service | Role |
|---|---|
| `bridge` | the real bridge (device → HRMS). Production service. |
| `scanner` | LAN discovery of ZKTeco devices. |
| `zkteco` | mock device server (testing without hardware). |
| `hrms` | mock HRMS webhook (testing). |

**Two jobs:** (1) **push** attendance up (pull punches → map → POST webhook → optional clear);
(2) **pull** template jobs down (poll HRMS → run on device → report). Outbound-only HTTPS, so it
works behind NAT.

**Data dir** (per-OS, via `directories`): Linux `~/.config/fbsy/`, macOS `~/Library/Application
Support/fbsy/`, Windows `%APPDATA%\fbsy\` → `config/config.json`, `logs/<instance>.log`,
`run/<instance>.json`, `update/`.

---

## 2. Architecture map (actual, as-built)

Hexagonal / clean. Dependency direction: `cli/runtime → application → ports/domain`;
`adapters → ports/domain`; **`domain → nothing`**. (Full rationale:
[CODEBASE_ARCHITECTURE_DECISION.md](CODEBASE_ARCHITECTURE_DECISION.md);
playbook [03](framework/03-architecture.md).)

```
src/
├── main.rs              tiny entry: tracing init, Cli::parse, cli::run, exit(1) on error
├── lib.rs               module root
├── cli/                 args.rs (clap top-level) · command.rs (enums) · dispatch.rs (→ application)
├── config/              model.rs (BridgeConfig/BridgeDeviceConfig + Redacted*) · impls.rs
│                        (from_json_value, defaults, validate, redacted) · error.rs · mod.rs
├── domain/              attendance.rs (RawAttendance) · event.rs (punch→event, timestamp, sort)
│                        · sync_result.rs · template.rs (DeviceUser/FingerTemplate + base64)
├── ports/               device.rs (DeviceClient/DeviceConnector + DeviceInfo) · hrms.rs
│                        (HrmsClient, BATCH_SIZE=500) · config_store.rs
├── adapters/            device_zkteco_tcp.rs (REAL pyzk-compatible TCP/UDP protocol) ·
│                        hrms_reqwest.rs (real webhook+job client) · hrms_http.rs (AUDIT: usage?) ·
│                        config_file.rs (JsonConfigStore)
├── application/         sync_once · serve · setup · doctor · config · install · update ·
│                        service (process orchestration) · dashboard (TUI) · scanner · test_server (mocks)
├── runtime/             sync_state.rs (DeviceSyncState — the sync lifecycle + invariant) ·
│                        job_poller.rs · process.rs (spawn_detached/is_alive/terminate via sysinfo) ·
│                        registry.rs (run/<name>.json, atomic write)
└── support/             log.rs (structured event logger) · paths.rs · redaction.rs · network.rs
```

**Important "as-built" facts that contradict the old plan docs — do not be misled:**
- **HTTP is hand-rolled** over `std::net::TcpListener` in [`serve.rs`](../src/application/serve.rs),
  **not `axum`**. Concurrency is **`std::thread`**, **not `tokio`**. `tokio`/`axum` remain
  *commented out* in `Cargo.toml` (deliberate dependency-light choice — keep it unless a task
  here says otherwise).
- **No OS service integration yet.** "Background service" = a **detached child process +
  registry file**, managed by fbsy itself ([`process.rs`](../src/runtime/process.rs),
  [`service.rs`](../src/application/service.rs)). It does **not** auto-start on reboot. (Gap — see §8 P0-3.)
- The real **ZKTeco protocol is fully implemented** ([`device_zkteco_tcp.rs`](../src/adapters/device_zkteco_tcp.rs)):
  TCP+UDP, auth (commkey), attendance pull (8/16/40-byte formats), clear, users, templates,
  push user+template. Not a stub.
- **Self-update is real and safe** ([`update.rs`](../src/application/update.rs)): check → download
  → SHA-256 verify → smoke-test → backup → atomic `self_replace` → restart services →
  health-check → **auto-rollback**.

---

## 3. Invariants & contracts — MUST NOT BREAK

> These are the guardrails. A change that violates any of these is a **defect**, even if it
> compiles and tests pass. Add a regression test for each you touch.

### 3.1 The safety invariant (data-loss prevention)
> **Never clear device attendance unless the HRMS upload succeeded.**

Enforced in [`runtime/sync_state.rs`](../src/runtime/sync_state.rs) `sync_with_client`: clear runs
**only** after `forward_events` returns `Ok` **and** `device.clear_attendance_after_sync == true`.
A failed/partial upload must leave device records intact. A failed *clear* after a *successful*
upload must **not** fail the sync (records simply remain). Covered by
[`tests/sync_tests.rs`](../tests/sync_tests.rs) — keep those green.

### 3.2 The HRMS contract (byte-for-byte — see [BRIDGE_BUILD_SPEC.md](BRIDGE_BUILD_SPEC.md) §1)
- Webhook `POST {vpsWebhookUrl}` body: `{ organizationId, deviceCode, apiKey, events[] }`,
  **≤ 500 events/request** (`ports::hrms::BATCH_SIZE`).
- Event keys: `deviceEmployeeId, timestamp, eventType("check_in"|"check_out"), verificationMethod`.
- Read success as `data.received` from the `{statusCode,message,data}` envelope.
- Pending jobs: `GET {hrmsBaseUrl}/biometric-devices/pending-jobs?deviceCodes=…`,
  `Authorization: Bearer <a device apiKey>`; read `data` (array).
- Complete: `POST {hrmsBaseUrl}/biometric-devices/jobs/{id}/complete` with `{ok}` /
  `{ok,error}` / `{ok,templates[]}`.
- **Retry policy:** retry **network errors, 429, 5xx** only; **never 4xx**
  ([`hrms_reqwest.rs`](../src/adapters/hrms_reqwest.rs) `should_retry_status`).

### 3.3 Config compatibility & validation
- `config.json` field names stay **`camelCase`** (Python-bridge compatibility — see
  [`config/model.rs`](../src/config/model.rs)). Don't rename.
- Legacy flat single-device config must still load (wrapped into `devices[0]`).
- Validation rules live in [`config/impls.rs`](../src/config/impls.rs): http(s) URL, unique
  `deviceCode`, required `deviceIp/deviceCode/apiKey`, `deviceTimeout 1..=120`, intervals clamped
  to ≥5, ports `1..=65535`. Defaults must match [BRIDGE_BUILD_SPEC.md](BRIDGE_BUILD_SPEC.md) §2.

### 3.4 Security
- Secrets (`apiKey`, `deviceCode`, device password) **never logged raw**: redact at the boundary
  (`support::redaction::redact`, `sync_state::sanitize`, `RedactedBridgeConfig`). See
  [SECURITY.md](SECURITY.md).
- Local bridge HTTP API **binds `127.0.0.1` only**. Do not bind it to `0.0.0.0` without an auth design (§10).
- Downloaded update binaries must be verified before execution (§3.2 integrity → strengthen to
  authenticity in P0-4).

### 3.5 CLI / behavioral compatibility
- `once` exits `0` on success, `1` on failure. `--once`, `at-bridge` alias, etc. preserved.
- Punch mapping: `0|4 → check_in`, else `check_out` (Python parity, [`domain/event.rs`](../src/domain/event.rs)).

---

## 4. Coding conventions & guardrails

- **Where code goes:** business rules → `domain` (no I/O, no frameworks). Orchestration →
  `application`. External systems → behind a `ports` trait, implemented in `adapters`.
  Long-running/process concerns → `runtime`. CLI parses & dispatches only.
- **Errors:** `thiserror` for typed domain/port errors (`ConfigError`, `DeviceError`,
  `HrmsError`); `anyhow::Result` at application/CLI boundaries. Sanitize secrets into error
  strings; truncate long errors (existing helpers do this).
- **Logging:** event logs via [`support::log`](../src/support/log.rs) (`log::info/warn/error`,
  `[component]` tag, RFC3339 prefix so the dashboard can time-merge). One-time human banners use
  `println!`/`console::style`. The bridge enables progress logging via
  `DeviceSyncState::with_progress_logging()`.
- **Dependency-light philosophy:** prefer std + the existing small crate set. Adding `tokio`/`axum`
  is a deliberate decision (a P1 task gates it) — don't pull them in casually.
- **Testing pattern:** test the domain with unit tests; test use cases against **fake adapters**
  (see `tests/sync_tests.rs`, `job_poller` tests). Integration-test the real protocol against the
  built-in **mock device** (`tests/device_protocol_tests.rs` + `application::test_server`).
- **Atomic file writes:** config and registry writes go through temp-file + `rename` (see
  `setup::save_config_atomically`, `registry::write`). Keep that pattern for any new state file.
- **Conventional Commits** + keep `main` green. SemVer; bump `Cargo.toml` version to trigger a
  release (CI auto-releases on a new version tag). Playbook [13](framework/13-code-management.md).

---

## 5. Cold-start: build / run / test / gates

```bash
# Build & gates (run all three before every commit — the CI runs the same):
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all

# Run locally (dev):
cargo run -- --help
cargo run -- bridge config setup        # interactive wizard → config/config.json
cargo run -- bridge config validate
cargo run -- bridge doctor [--deep] [--json]
cargo run -- bridge sync --once [--device CODE]

# Full local end-to-end with mocks (no hardware) — see LOGGING_CHECKLIST.md:
cargo run -- run hrms --name hrms1 --port 8800
cargo run -- run zkteco --name dev1 --port 4370 --records 5
cargo run -- bridge config setup        # point webhook at http://127.0.0.1:8800/webhook, device at 127.0.0.1:4370
cargo run -- run bridge
cargo run -- bridge sync --once
cargo run -- logs bridge -n 100 ; cargo run -- logs hrms1 -n 100
```

CI: [`.github/workflows/ci.yml`](../.github/workflows/ci.yml) (fmt+clippy+test on push/PR;
auto-build 5 targets + release with SHA-256 `checksums.txt` when `Cargo.toml` version is new).

---

## 6. Current state — what is DONE (don't rebuild it)

| Capability | Where | Notes |
|---|---|---|
| Multi-device + legacy config, defaults, validation, redaction, bool-coercion | `config/impls.rs` | ✅ tested (`tests/config_tests.rs`) |
| Domain mapping (punch→event, ISO timestamp, skip-malformed, sort) | `domain/event.rs` | ✅ tested (`tests/domain_tests.rs`) |
| Sync engine + **safety invariant** + per-device lock + last_result (in-memory) | `runtime/sync_state.rs` | ✅ tested (`tests/sync_tests.rs`) |
| HRMS webhook client: batch 500, retry 429/5xx (not 4xx), envelope parse, UA | `adapters/hrms_reqwest.rs` | ✅ tested |
| Job poller: PUSH_USER / PULL_TEMPLATES, complete, truncate errors | `runtime/job_poller.rs` | ✅ tested w/ fakes |
| **Real ZKTeco protocol** (TCP/UDP, auth, attendance, users, templates, push) | `adapters/device_zkteco_tcp.rs` | ✅ integration-tested vs mock |
| Local HTTP API `/health`,`/sync`,`/sync?device=`,`OPTIONS` | `application/serve.rs` | ✅ hand-rolled; ⚠ see gaps |
| Per-device schedulers + boot sync + job poller + auto-updater trigger | `application/serve.rs` | ✅ thread-based |
| Service manager: detached spawn, registry, show/status/close/restart, logs(+follow) | `application/service.rs`, `runtime/{process,registry}.rs` | ✅ |
| Setup wizard (atomic save + backup) | `application/setup.rs` | ✅ tested |
| Doctor (`--deep`,`--json`), devices test/info, webhook test | `application/doctor.rs` | ✅ |
| Self-install + PATH + uninstall (Win self-delete) | `application/install.rs` | ✅ |
| **Safe self-update** + auto-rollback + opt-in auto-update | `application/update.rs` | ✅ checksum-only (⚠ no signature) |
| LAN scanner (scan + service) | `application/scanner.rs` | ✅ |
| Mock device + mock HRMS servers | `application/test_server.rs` | ✅ |
| Live TUI dashboard | `application/dashboard.rs` | ✅ |
| CI build/test + 5-target release + checksums | `.github/workflows/` | ✅ |
| Install scripts (sh/ps1), structured logging | `scripts/`, `support/log.rs` | ✅ |

---

## 7. Production-readiness gap analysis (framework-wide)

Scored against the playbook. ✅ strong · 🟡 partial · 🔴 gap. Each 🟡/🔴 maps to a §8 task.

| # | Chapter | Status | Gap → task |
|---|---------|:----:|------|
| 01 Mindset / NFRs | 🟡 | NFRs implicit; pin them → P1-9 |
| 02 Planning / ADRs | ✅ | Decision docs exist; adopt ADR format going forward |
| 03 Architecture | ✅ | Clean; plan docs stale (this file fixes that) |
| 04 Principles | ✅ | DI, CIA, idempotency, redaction all present |
| 05 Tech selection | ✅ | Deliberate small crates; `hrms_http.rs` dead-code audit → P1-12 |
| 06 Tooling/CI | 🟡 | No `cargo audit`/secret-scan/SBOM → P0-4 / P1-10 |
| 07 Operations | 🔴 | **No log rotation** (P0-2), **no reboot autostart** (P0-3), no on-disk last-result (P1-5), no graceful shutdown (P1-6) |
| 08 Security | 🔴 | **Releases unsigned; updater is checksum-only & skips silently** → P0-4 |
| 09 Scaling | ✅ | Rung-0 by design; correct |
| 11 System design (HLD/LLD) | ✅ | Documented here + decision doc |
| 12 Data modeling | ✅ | No DB by design (stateless bridge) |
| 13 Code management | 🟡 | Add CODEOWNERS + CONTRIBUTING → P2-15 |
| 14 Design patterns | ✅ | Adapter/Strategy/DI throughout |
| 15 Testing | 🟡 | Strong unit/integration; **no HTTP-API or CLI tests, no protocol fuzzing** → P1-8 |
| 16 API design | 🟡 | Hand-rolled HTTP fragile; no auth/jitter → P1-6/P1-7 |
| 17 Migration | ✅ | Python→Rust; camelCase compat kept |
| 18 Reliability/SRE | 🟡 | Invariant = the SLO; add disk state + graceful shutdown |
| 19 AI | ✅ N/A | Correctly not an AI app |
| 20 Cost/sustainability | ✅ | Native binary, no cloud bill |
| 21 Compliance | 🟡 | Biometric-derived PII; document processing record → P2-14 |
| 22 DX | ✅ | Wizard/doctor/dashboard/clear errors |

**Correctness watch-items found in code (treat as P0/P1):**
- **Timezone**: `domain/event.rs` treats naive device timestamps as **UTC**. Real devices report
  **local wall-time**. If a device isn't on UTC, punches land on the wrong calendar day. → **P0-1**.
- **`verify_checksum` silently skips** if `checksums.txt` or the entry is missing (returns `Ok`).
  That's fail-*open* for integrity. → **P0-4**.
- **Unbounded logs**: every service appends to `logs/<name>.log` forever (`process.rs` opens
  append; no rotation). 24/7 → disk fill. → **P0-2**.

---

## 8. Execution checklist (do in order)

Format per task — **ID · what · why · files · acceptance · verify · tests · ref**. Mark `[x]`
only when *acceptance* holds and *verify* passes green.

### Phase P0 — Release blockers (correctness, safety, data integrity)

- [ ] **P0-1 · Confirm & fix timezone handling.**
  *Why:* wrong-day attendance is a correctness failure (§3.5). *Files:* `domain/event.rs`
  (`parse_timestamp`), possibly `config/model.rs`+`impls.rs` (add optional `deviceTimezone`/offset),
  `domain/event.rs` tests. *Acceptance:* a punch from a non-UTC device lands on the correct local
  day in HRMS; behavior is documented; **decision recorded with the human (§10-A)** before coding.
  *Verify:* `cargo test domain`; manual real-device check per [TESTING.md](TESTING.md) "Manual
  Real-Device Tests". *Tests:* add cases for an offset device and a UTC device. *Ref:*
  [BRIDGE_BUILD_SPEC.md](BRIDGE_BUILD_SPEC.md) §4, framework [15](framework/15-testing.md).

- [ ] **P0-2 · Add log rotation / size cap.**
  *Why:* unattended 24/7 process must not fill the disk ([07](framework/07-operations.md)).
  *Files:* `support/log.rs` and/or `runtime/process.rs` (the child redirects stdout→`logs/<name>.log`);
  introduce a size-based rotation (e.g. rotate at N MB, keep K files) — either a rotating writer in
  the child loop or a periodic rotate. *Acceptance:* a log file never exceeds the cap; old files are
  pruned; existing tail/follow/dashboard still work. *Verify:* unit test the rotation trigger; run a
  service and confirm rollover. *Tests:* rotation unit test. *Ref:* [07](framework/07-operations.md).

- [ ] **P0-3 · Reboot survival (OS autostart) — design + implement OR explicitly defer.**
  *Why:* a production office bridge must come back after a power cycle; today it does not
  (process-manager only). *Files:* new `application/autostart.rs` (or extend `service.rs`) +
  CLI `service install|uninstall|status` (`cli/command.rs`,`dispatch.rs`); generate **systemd**
  user/system unit (Linux), **launchd** plist (macOS), **Task Scheduler** entry (Windows) that runs
  `fbsy run bridge` on boot/login. *Acceptance:* after install + reboot, the bridge is running;
  uninstall removes it cleanly. *Verify:* per-OS manual reboot test ([TESTING.md](TESTING.md)
  "Manual Service Tests"). *Tests:* unit-test the generated unit/plist/command strings. *Ref:*
  [FULL_WORKFLOW_PLAN.md](FULL_WORKFLOW_PLAN.md) Phase 10, framework [07](framework/07-operations.md).
  *(If product decides the process-manager model is sufficient, record that decision (§10-D) and
  downgrade to documenting "start on login manually".)*

- [ ] **P0-4 · Supply-chain: sign releases + verify signature in the updater (fail-closed); add `cargo audit`.**
  *Why:* checksum proves integrity, not authenticity, and `verify_checksum` currently skips
  silently — a compromised release host could serve a malicious binary ([08](framework/08-security.md#supply-chain)).
  *Files:* `.github/workflows/ci.yml` (sign each artifact — minisign or cosign — publish signatures +
  SLSA provenance/attestations; add a `cargo audit`/`cargo-deny` job; generate an SBOM);
  `application/update.rs` (`verify_checksum` must **fail** if checksum is missing/mismatched;
  add signature verification before `self_replace`). *Acceptance:* tampered/unsigned binary is
  rejected; CI fails on a known-vuln dependency; SBOM is published. *Verify:* `cargo audit` in CI
  green; a deliberately-corrupted asset fails update in a manual test. *Tests:* unit-test the
  verify-fails-closed path. *Ref:* [08](framework/08-security.md), [06](framework/06-tooling.md).

### Phase P1 — Hardening (reliability, operability, robustness)

- [ ] **P1-5 · Persist last sync result to disk.**
  *Why:* `/health` & `doctor` lose history on restart; on-disk state aids diagnosis
  ([18](framework/18-reliability-sre.md)). *Files:* `runtime/sync_state.rs` (write
  `state/last-result.json` per device atomically after each sync); `serve.rs`/`doctor.rs` read it.
  *Acceptance:* last result survives a bridge restart and shows in `doctor`/`/health`. *Verify:*
  restart bridge, confirm last result persists. *Tests:* round-trip write/read test.

- [ ] **P1-6 · Graceful shutdown for `serve`.**
  *Why:* clean stop of the HTTP loop + schedulers on SIGTERM (close just kills the pid today).
  *Files:* `serve.rs` (install a signal handler / shutdown flag; stop accepting, let in-flight sync
  finish or checkpoint). *Acceptance:* `fbsy close bridge` stops without truncating an in-flight
  upload mid-batch; no data loss (invariant already protects, but make it clean). *Verify:* manual
  stop during a sync. *Ref:* [07](framework/07-operations.md).

- [ ] **P1-7 · Harden the local HTTP server (and decide auth posture).**
  *Why:* the hand-rolled parser reads a single 8 KB buffer, no `Content-Length` body handling,
  `Connection: close` only ([16](framework/16-api-design.md)). *Files:* `serve.rs` — robust
  request-line/header parsing, bounded reads, correct 400 on malformed; **keep `127.0.0.1` bind**;
  if any non-loopback exposure is ever wanted, add a token (decision §10-C). *Acceptance:* malformed
  requests get a clean 4xx, no panics; `/health` & `/sync` unchanged. *Verify:* P1-8 HTTP tests.
  *Ref:* [16](framework/16-api-design.md).

- [ ] **P1-8 · Test coverage: HTTP API + CLI + protocol fuzzing.**
  *Why:* the server, CLI, and the untrusted-byte decoder are under-tested
  ([15](framework/15-testing.md)). *Files:* new `tests/api_tests.rs` (spawn `serve` on an ephemeral
  port, assert `/health` shape, `/sync` 202, `/sync?device=` 404/429, OPTIONS 204, loopback bind);
  `tests/cli_tests.rs` (`assert_cmd`: `--help`, `show`, `config validate`, exit codes); a
  `cargo-fuzz`/proptest target for `decode_attendance_data`/`decode_users`/`decode_templates`.
  *Acceptance:* new tests pass; fuzzer finds no panics in a short run. *Verify:* `cargo test --all`
  (+ `cargo fuzz run` if added). *Ref:* [15](framework/15-testing.md).

- [ ] **P1-9 · Make NFRs explicit + retry jitter.**
  *Why:* turn implicit qualities into verifiable targets ([02](framework/02-planning.md#non-functional-requirements-nfrs)).
  *Files:* a short "NFRs" section in [README.md](../README.md)/this file (binary size cap, memory
  ceiling over 7-day soak, sync p99, RPO=0 = the invariant); add **jitter** to webhook backoff in
  `hrms_reqwest.rs`. *Acceptance:* NFRs written with numbers; backoff has jitter. *Verify:* `cargo test hrms`.

- [ ] **P1-10 · CI: secret scan + coverage visibility.**
  *Files:* `.github/workflows/ci.yml` — add gitleaks (or trufflehog) and a coverage report
  (`cargo-llvm-cov`) as non-blocking signal. *Acceptance:* secret scan runs on PRs. *Ref:* [06](framework/06-tooling.md).

- [ ] **P1-11 · Pin CI actions by commit SHA.**
  *Why:* the 2025 GitHub-Actions supply-chain attacks targeted floating tags
  ([08](framework/08-security.md#supply-chain)). *Files:* `.github/workflows/*` (pin `actions/*`,
  `dtolnay/rust-toolchain`, `softprops/action-gh-release` by digest). *Acceptance:* all actions pinned.

- [ ] **P1-12 · Audit/remove `adapters/hrms_http.rs`.**
  *Why:* possible dead code beside the real `hrms_reqwest.rs` ([13](framework/13-code-management.md)).
  *Files:* `adapters/hrms_http.rs`, `adapters/mod.rs`. *Acceptance:* if unused, deleted; if used,
  documented. *Verify:* `cargo build` + `cargo clippy` clean (no dead-code warnings).

### Phase P2 — Polish & governance

- [ ] **P2-13 · `clearAttendanceAfterSync` safety UX.** Keep default `false`; surface a clear
  warning in setup/doctor before enabling (it wipes device logs). *Files:* `setup.rs`, `doctor.rs`.
- [ ] **P2-14 · Compliance/data-governance note.** Document the data-flow (device→fbsy→HRMS),
  classify attendance as biometric-derived PII, note minimization-by-design, confirm jurisdiction
  obligations with counsel. *Files:* extend [SECURITY.md](SECURITY.md) or a new `docs/DATA_GOVERNANCE.md`.
  *Ref:* [21](framework/21-compliance-governance.md).
- [ ] **P2-15 · `CODEOWNERS` + `CONTRIBUTING.md`.** *Ref:* [13](framework/13-code-management.md).
- [ ] **P2-16 · Code-sign & notarize binaries** (macOS notarization, Windows Authenticode) to remove
  Gatekeeper/SmartScreen friction. *Files:* `.github/workflows/`. Needs signing certs (§10-B).
- [ ] **P2-17 · Clean up unused `deviceOmitPing`** (parsed but the TCP adapter never pings) — either
  implement a ping pre-check or drop the field from runtime use. *Files:* `device_zkteco_tcp.rs`, docs.

---

## 9. Definition of Done (release readiness)

A release is production-ready when **all P0 are done** and:
- [ ] `cargo fmt --check && cargo clippy -D warnings && cargo test --all` green on all 5 targets.
- [ ] Timezone correctness confirmed against a real device (P0-1).
- [ ] Logs rotate; disk usage bounded (P0-2).
- [ ] Bridge survives a reboot on Linux/macOS/Windows — or deferral is recorded (P0-3).
- [ ] Releases are signed; the updater verifies signature **and** checksum, fail-closed; `cargo audit` clean (P0-4).
- [ ] Safety invariant + HRMS contract tests present and green (§3.1/§3.2).
- [ ] Full mock end-to-end passes ([LOGGING_CHECKLIST.md](LOGGING_CHECKLIST.md) "End-To-End Mock Test").
- [ ] Real-device smoke test passes ([TESTING.md](TESTING.md)).
- [ ] [USER_GUIDE.md](USER_GUIDE.md) + [README.md](../README.md) match actual behavior.

---

## 10. Open decisions — ask the human before coding these

- **A. Timezone (P0-1):** are office devices on UTC or local time? Should the bridge attach the
  device's real offset (and where does the offset come from — config field, or device option)? This
  changes attendance correctness. **Do not guess.**
- **B. Signing infrastructure (P0-4 / P2-16):** which signing approach + key custody (minisign keypair?
  cosign/Sigstore keyless via GitHub OIDC? OS code-signing certs)? Needs secrets provisioned in CI.
- **C. HTTP API auth (P1-7):** stay loopback-only forever, or support authenticated LAN exposure?
  Determines whether to add a token/mTLS.
- **D. Reboot model (P0-3):** implement OS-service install, or officially keep the process-manager
  model and document manual start-on-login?
- **E. `hrmsApiToken` semantics:** [BRIDGE_BUILD_SPEC.md](BRIDGE_BUILD_SPEC.md) §14 open item — keep
  the field or always derive the job-poll bearer from `devices[0].apiKey`? (Code currently prefers
  the explicit token, else first device key — fine, just confirm the intended UX.)

---

## 11. Reference index

- **Contract / spec:** [BRIDGE_BUILD_SPEC.md](BRIDGE_BUILD_SPEC.md) (HRMS contract — authoritative) ·
  [CODEBASE_ARCHITECTURE_DECISION.md](CODEBASE_ARCHITECTURE_DECISION.md) (HLD/LLD)
- **Behavior:** [USER_GUIDE.md](USER_GUIDE.md) · [CLI.md](CLI.md) · [CONFIGURATION.md](CONFIGURATION.md) ·
  [INSTALL_FLOW.md](INSTALL_FLOW.md) · [NETWORK_SCANNER.md](NETWORK_SCANNER.md)
- **Quality:** [TESTING.md](TESTING.md) · [LOGGING_CHECKLIST.md](LOGGING_CHECKLIST.md) ·
  [SECURITY.md](SECURITY.md) · [PACKAGING.md](PACKAGING.md)
- **Legacy plans (stale — architecture/learning only):** [IMPLEMENTATION_PLAN.md](IMPLEMENTATION_PLAN.md) ·
  [FULL_WORKFLOW_PLAN.md](FULL_WORKFLOW_PLAN.md) · [CODE_WALKTHROUGH.md](CODE_WALKTHROUGH.md)
- **Framework playbook:** [docs/framework/00-INDEX.md](framework/00-INDEX.md) (chapters 01–22)
- **Key source files:** see §2 map. Start reading at `main.rs` → `cli/dispatch.rs` →
  `runtime/sync_state.rs` (the invariant) → `adapters/{device_zkteco_tcp,hrms_reqwest}.rs`.
```
