# Migration From Python Bridge

The Rust bridge should be behavior-compatible with the existing Python bridge.

## Keep Compatible

- `config.json` field names
- `--once` behavior
- `--setup` behavior
- exit code `0` for success
- exit code `1` for failure
- `/health` endpoint shape
- `/sync` endpoint behavior
- log readability
- installer flow where possible

## Python To Rust Mapping

| Python File | Rust Module |
| --- | --- |
| `src/fingerbridge/cli.py` | `src/cli.rs` |
| `src/fingerbridge/config.py` | `src/config.rs` |
| `src/fingerbridge/core/device.py` | `src/device.rs` |
| `src/fingerbridge/core/hrms.py` | `src/hrms.rs` |
| `src/fingerbridge/core/sync.py` | `src/sync.rs` |
| `src/fingerbridge/core/scheduler.py` | `src/scheduler.rs` |
| `src/fingerbridge/core/setup.py` | `src/setup.rs` |
| `src/fingerbridge/api/server.py` | `src/api.rs` later |
| `src/fingerbridge/models/events.py` | `src/models.rs` |
| `src/fingerbridge/utils/paths.py` | `src/paths.rs` |

## Migration Strategy

1. Ship Rust version beside Python version for internal testing.
2. Validate existing `config.json` files without changes.
3. Compare `/health` output.
4. Compare one-sync output.
5. Test webhook delivery with mock HRMS.
6. Test against real ZKTeco device.
7. Update install scripts after parity is proven.

