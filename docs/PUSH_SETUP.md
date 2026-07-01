# Push Mode — SenseFace / ADMS Device

```
SenseFace ──HTTP push──▶ fbsy bridge ──HTTPS──▶ HRMS webhook
                         (port 8090)
```

The device pushes attendance to the bridge. No need to poll.

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
- **Bridge mode** → choose **Push** or **Hybrid** (both pull + push)
- **SenseFace bind host** — `0.0.0.0` (default)
- **SenseFace port** — where devices push attendance (default 8090)
- **Device code prefix** — for unknown serial numbers (default `SF`)
- **Serial-to-device mappings** — link device serial numbers to HRMS device codes

Then configure your SenseFace terminal's cloud server settings to point to:

```
http://YOUR_SERVER_IP:8090
```

## 3. Start the bridge

```bash
fbsy run bridge
```

The bridge listens for pushes from the SenseFace device.

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
fbsy show                    # check status
fbsy logs bridge             # see pushes + forwards in real time
```

On the SenseFace terminal, trigger a manual attendance push. The bridge will:

1. Receive the push → save to SQLite
2. Forward to HRMS webhook
3. You can see the result in the logs
