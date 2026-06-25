# Development Guide

For developers working on the bridge itself.

## Layout

```
fingerbridge/
  agent.py                 compatibility launcher (python agent.py --once still works)
  src/fingerbridge/       the importable package
    __main__.py            python -m fingerbridge
    cli.py                 argparse entrypoint, server + scheduler startup
    config.py              load/validate config.json, defaults, coercion
    exceptions.py          ConfigError, WebhookError (BridgeError base)
    api/
      server.py            /health and /sync HTTP handlers
    core/
      device.py            pyzk connect / pull / clear
      hrms.py              webhook batching, retries, HTTP errors
      sync.py              one sync cycle + concurrency lock
      scheduler.py         background interval loop
      setup.py             interactive --setup wizard + connection tests
    models/
      events.py            raw attendance -> HRMS webhook events
    utils/
      paths.py             base dir resolution (handles PyInstaller frozen builds)
      logging_setup.py     console + rotating file logging
  tests/                   mirrored suite: conftest, test_api/core/models/config
  docs/                    INSTALLATION, DEVELOPMENT, EDGE_CASES
  fingerbridge.spec       PyInstaller spec (per-OS output name)
  build.sh / build.bat     local single-OS build
  .github/workflows/ci.yml CI: test + build all three OSes, release on tag
```

## Setup

```bash
python3 -m pip install -r requirements-dev.txt
```

`requirements.txt` holds the single runtime dependency (`pyzk`).
`requirements-dev.txt` adds `pyinstaller` for building.

## Running locally

```bash
python3 -m fingerbridge --once          # pull once, print JSON, exit (preferred)
python3 -m fingerbridge                  # run server + scheduler (default interval)
python3 -m fingerbridge --interval 120   # override interval (seconds)
python3 agent.py --once                   # compatibility shim, same behaviour
```

`--once` exits `0` on success, `1` on failure — safe for scripting/supervisors.

## Tests

The suite uses only the standard library, so no install step is needed:

```bash
PYTHONPATH=src:tests python3 -m unittest discover -s tests
```

(`pytest` also works if you have it: just run `pytest` — `pyproject.toml` sets
the `src` and `tests` paths for you.)

Shared fakes/builders (`FakeConn`, `FakeAttendance`, `base_cfg`, `make_events`)
live in `tests/conftest.py`. Coverage:

| File              | What it covers                                                          |
| ----------------- | ---------------------------------------------------------------------- |
| `test_config.py`  | defaults, int coercion, bool-from-string, URL/port validation          |
| `test_models.py`  | timestamp parsing, punch mapping, skipping bad records, sorting        |
| `test_core.py`    | webhook batching/retry/backoff + sync lifecycle, lock, clear-safety    |
| `test_api.py`     | `/health`, `/sync` (202), unknown path 404, OPTIONS preflight          |

Everything uses fakes/mocks — no real device or network is touched.

## Building binaries

PyInstaller **cannot cross-compile** — each OS binary must be built on that OS.

```bash
./build.sh        # macOS or Linux  -> dist/fingerbridge-macos | -linux
build.bat         # Windows         -> dist/fingerbridge.exe
```

For all three at once, push a tag and let CI build them:

```bash
git tag v2.1.0
git push origin v2.1.0
```

`.github/workflows/ci.yml` builds + tests on ubuntu/macos/windows runners and,
on a `v*` tag, attaches the three binaries to a GitHub Release. The install
scripts look for those exact names.

## Conventions

- Standard-library only at runtime except `pyzk`. Keep it that way so the frozen
  binary stays small and dependency-free.
- All modules log through `logging.getLogger("bridge")`.
- Config is a plain `dict`; validation lives entirely in `config.py`.
- Never clear device attendance unless the webhook upload succeeded — see
  `sync.py` and the test `test_webhook_failure_does_not_clear_device`.

## Future Rust Implementation

A step-by-step Rust rewrite plan is documented in
[RUST_IMPLEMENTATION_PLAN.md](RUST_IMPLEMENTATION_PLAN.md). It maps the current
Python flow to the selected Rust CLI stack, including config validation, sync
lifecycle, HRMS webhook forwarding, HTTP endpoints, scheduler behavior, tests,
and packaging.
