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
| `src/zkteco_bridge/cli.py` | `src/cli.rs` |
| `src/zkteco_bridge/config.py` | `src/config.rs` |
| `src/zkteco_bridge/core/device.py` | `src/device.rs` |
| `src/zkteco_bridge/core/hrms.py` | `src/hrms.rs` |
| `src/zkteco_bridge/core/sync.py` | `src/sync.rs` |
| `src/zkteco_bridge/core/scheduler.py` | `src/scheduler.rs` |
| `src/zkteco_bridge/core/setup.py` | `src/setup.rs` |
| `src/zkteco_bridge/api/server.py` | `src/api.rs` later |
| `src/zkteco_bridge/models/events.py` | `src/models.rs` |
| `src/zkteco_bridge/utils/paths.py` | `src/paths.rs` |

## Migration Strategy

1. Ship Rust version beside Python version for internal testing.
2. Validate existing `config.json` files without changes.
3. Compare `/health` output.
4. Compare one-sync output.
5. Test webhook delivery with mock HRMS.
6. Test against real ZKTeco device.
7. Update install scripts after parity is proven.

