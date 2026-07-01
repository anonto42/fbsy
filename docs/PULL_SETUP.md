# Pull Mode — ZKTeco TCP Device

```
ZKTeco ──TCP 4370──▶ fbsy bridge ──HTTPS──▶ HRMS webhook
```

## 1. Install

```bash
curl -fsSL https://raw.githubusercontent.com/anonto42/fbsy/main/scripts/install.sh | sh
```

Open a new terminal, then:

```bash
fbsy --help
```

## 2. Setup the bridge

```bash
fbsy bridge config setup
```

Wizard will ask you for:
- **HRMS Webhook URL** — where attendance events are sent
- **Bridge HTTP port** — local API (default 7431)
- **Bridge mode** → choose **Pull**
- **Device IP, port, code, API key** — your ZKTeco device details
- **Sync interval** — how often to check for new attendance

Or for local testing with mock services:

```bash
fbsy bridge config setup --local
fbsy run hrms --name local-hrms -p 18800
fbsy run zkteco --name local-zkteco -p 14370 --records 5
```

## 3. Start the bridge

```bash
fbsy run bridge
```

The bridge will pull attendance from your device and forward it to HRMS.

## 4. Auto-start on boot (production)

```bash
fbsy enable bridge
```

This prints the exact `sudo` command needed. Run it so the bridge starts automatically after a reboot or power loss.

```bash
sudo fbsy enable bridge
```

## Verify

```bash
fbsy show              # check status
fbsy bridge sync       # force a sync now
fbsy logs bridge       # see the sync trail
```
