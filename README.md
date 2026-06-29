# marshall-stanmore-2 (Rust)

A Rust rewrite of the Python Marshall Stanmore II BLE↔MQTT bridge, with
**Home Assistant MQTT auto-discovery** support.

The daemon connects to the speaker over Bluetooth LE (via BlueZ / `btleplug`),
exposes its controls and status over MQTT, and on startup publishes Home
Assistant discovery configs so the speaker shows up automatically as a device
with all its entities.

## Running on a Raspberry Pi (Raspbian)

The Pi's onboard Bluetooth talks to the speaker over BLE, and the bridge talks to
your MQTT broker over the network. These steps assume Raspberry Pi OS / Raspbian.

### 1. Find the speaker's BLE address

Make sure Bluetooth is up, then scan with `bluetoothctl`:

```bash
sudo rfkill unblock bluetooth
sudo systemctl enable --now bluetooth

bluetoothctl
[bluetooth]# scan on
# ...wait until a line with "STANMORE II" appears, e.g.
# [NEW] Device 54:B7:E5:A2:CA:41 STANMORE II
[bluetooth]# scan off
[bluetooth]# exit
```

The `54:B7:E5:A2:CA:41`-style MAC on the `STANMORE II` line is your
`BLE_ADDRESS`. (The speaker must be powered on and not already connected to a
phone.)

### 2. Download and install the binary

The releases publish a prebuilt `aarch64-unknown-linux-gnu` binary, so there's
nothing to compile. Install its one runtime dependency, then download and unpack
it into `/usr/local/bin`:

```bash
sudo apt update
sudo apt install -y libdbus-1-3 curl

VERSION=0.1.0
TARGET=aarch64-unknown-linux-gnu
curl -fsSL "https://github.com/rabbit-aaron/marshall-stanmore-2-rust/releases/download/v${VERSION}/stanmore2-v${VERSION}-${TARGET}.tar.gz" \
  | tar xz
sudo install -Dm755 "stanmore2-v${VERSION}-${TARGET}/stanmore2" /usr/local/bin/stanmore2
```

> The binary is 64-bit (aarch64), so this needs 64-bit Raspberry Pi OS. Check
> with `uname -m` — it should print `aarch64`. On a 32-bit OS, install Rust and
> build from source instead (`cargo build --release`).

### 3. Run it in the background with systemd

Put your settings (including secrets) in an environment file. Keep it readable
only by root since it holds the MQTT password:

```bash
sudo tee /etc/stanmore2.env >/dev/null <<'EOF'
BLE_ADDRESS=54:B7:E5:A2:CA:41
MQTT_HOSTNAME=192.168.1.10
MQTT_USERNAME=myuser
MQTT_PASSWORD=mypassword
EOF
sudo chmod 600 /etc/stanmore2.env
```

Create the service unit at `/etc/systemd/system/stanmore2.service`:

```ini
[Unit]
Description=Marshall Stanmore II MQTT bridge
Wants=network-online.target
After=network-online.target bluetooth.target

[Service]
EnvironmentFile=/etc/stanmore2.env
ExecStart=/usr/local/bin/stanmore2
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
```

Enable and start it, then watch the logs:

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now stanmore2
journalctl -u stanmore2 -f
```

On a BLE disconnect the process exits and systemd restarts it (5 s later), so it
reconnects on its own.

## Running with Docker

Prefer containers? A tiny `debian-slim` image that just downloads the prebuilt
release binary is enough — no Rust toolchain, no compiling. At runtime it only
needs `libdbus-1-3` to talk to the host's BlueZ over D-Bus. Drop these two files
in the repo root.

`Dockerfile`:

```dockerfile
FROM debian:bookworm-slim

ARG VERSION=0.1.0
ARG TARGET=aarch64-unknown-linux-gnu

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates curl libdbus-1-3 \
    && curl -fsSL "https://github.com/rabbit-aaron/marshall-stanmore-2-rust/releases/download/v${VERSION}/stanmore2-v${VERSION}-${TARGET}.tar.gz" \
        | tar xz --strip-components=1 -C /usr/local/bin "stanmore2-v${VERSION}-${TARGET}/stanmore2" \
    && apt-get purge -y curl && apt-get autoremove -y \
    && rm -rf /var/lib/apt/lists/*

CMD ["stanmore2"]
```

The release binary is `aarch64-unknown-linux-gnu`, so build this image on the Pi
(or any arm64 host). Bump `VERSION` to pull a newer release.

`docker-compose.yml`:

```yaml
services:
  stanmore2:
    build: .
    # BlueZ uses the host Bluetooth stack over D-Bus, so the container needs
    # host networking and the system bus socket.
    network_mode: host
    volumes:
      - /var/run/dbus:/var/run/dbus
    cap_add:
      - NET_ADMIN
    environment:
      BLE_ADDRESS: "54:B7:E5:A2:CA:41"
      MQTT_HOSTNAME: "192.168.1.10"
      MQTT_USERNAME: "myuser"
      MQTT_PASSWORD: "mypassword"
    restart: unless-stopped
```

Then:

```bash
docker compose up -d --build
docker compose logs -f
```

`restart: unless-stopped` plays the same role as the systemd restart above —
the container comes back after a BLE drop.

## Configuration

All configuration is via environment variables:

| Variable               | Default         | Description                                                              |
|------------------------|-----------------|--------------------------------------------------------------------------|
| `BLE_ADDRESS`          | *(required)*    | BLE MAC address of the speaker, e.g. `00:11:22:33:44:55`                 |
| `MQTT_HOSTNAME`        | `127.0.0.1`     | MQTT broker host                                                          |
| `MQTT_PORT`            | `1883`          | MQTT broker port                                                          |
| `MQTT_USERNAME`        | *(unset)*       | MQTT username                                                             |
| `MQTT_PASSWORD`        | *(unset)*       | MQTT password                                                             |
| `MQTT_TOPIC_PREFIX`    | `stanmore2`     | Prefix for all command/info/lwt topics                                   |
| `MQTT_RETAIN`          | `0`             | Retain flag on info messages (`1`/`0`)                                    |
| `ALLOW_PAIRING`        | `0`             | Allow the `enter_pairing_mode` command (drops BLE and exits)             |
| `HA_DISCOVERY_PREFIX`  | `homeassistant` | Home Assistant discovery topic prefix                                     |
| `HA_DISCOVERY_ENABLED` | `1`             | Publish Home Assistant discovery configs on startup                      |

`RUST_LOG` controls log verbosity (e.g. `RUST_LOG=debug`).

## Running

```bash
BLE_ADDRESS=00:11:22:33:44:55 MQTT_HOSTNAME=192.168.1.10 cargo run --release
```

Or with Docker (needs host networking + the D-Bus socket for BlueZ):

```bash
BLE_ADDRESS=00:11:22:33:44:55 docker compose up --build
```

## Home Assistant auto-discovery

When `HA_DISCOVERY_ENABLED=1` (the default), on startup the bridge publishes a
retained discovery config for each entity under:

```
<HA_DISCOVERY_PREFIX>/<component>/<node_id>/<object_id>/config
```

where `node_id` is `stanmore2_<sanitized BLE address>`. All entities share one
`device` block (so they group under a single "Marshall Stanmore II" device) and
the `<prefix>/lwt` availability topic.

Entities published:

| Entity                | HA component | Command topic                          | State topic                          |
|-----------------------|--------------|----------------------------------------|--------------------------------------|
| Volume                | `number`     | `…/command/set_volume`                 | `…/info/volume`                      |
| LED brightness        | `number`     | `…/command/set_led_brightness`         | `…/info/led_brightness`              |
| EQ 160 Hz … 6.25 kHz  | `number` ×5  | `…/command/set_eq_profile/<band>hz`    | `…/info/eq_profile/<band>hz`         |
| EQ preset             | `select`     | `…/command/set_eq_preset`              | `…/info/eq_preset`                   |
| Audio source          | `select`     | `…/command/set_source`                 | `…/info/audio_source`                |
| Interaction sound     | `switch`     | `…/command/set_interaction_sound`      | `…/info/interaction_sound_enabled`   |
| Device name           | `text`       | `…/command/set_device_name`            | `…/info/device_name`                 |
| Play status           | `sensor`     | —                                      | `…/info/play_status`                 |
| Media title/artist/album | `sensor` ×3 | —                                   | `…/info/media/{title,artist,album}`  |
| Play / Pause / Next / Previous | `button` ×4 | `…/command/{play,pause,next,previous}` | —                          |

The underlying command/info topics are unchanged from the original Python
project, so existing manual integrations keep working — discovery is layered on
top.

### Command topics

Publish to `<prefix>/command/<action>`; payloads are UTF-8 strings/integers.

| Action                       | Payload                                              |
|------------------------------|-----------------------------------------------------|
| `set_volume`                 | int (0–32)                                           |
| `get_volume`                 | (empty)                                              |
| `set_eq_preset`              | `flat,rock,metal,pop,hiphop,electronic,jazz`        |
| `get_eq_preset`              | (empty)                                              |
| `set_eq_profile`             | 5 space-delimited ints 0–10 (`"5 5 5 5 5"`)         |
| `set_eq_profile/<band>hz`    | int 0–10 (band ∈ 160,400,1000,2500,6250)            |
| `get_eq_profile`             | (empty)                                              |
| `set_device_name`            | string (≤17 bytes)                                   |
| `get_device_name`            | (empty)                                              |
| `set_led_brightness`         | int (0–35)                                           |
| `get_led_brightness`         | (empty)                                              |
| `play` / `pause` / `next` / `previous` | (empty)                                   |
| `set_interaction_sound`      | `1` / `0`                                            |
| `get_status`                 | (empty)                                              |
| `set_source`                 | `bluetooth,aux,rca`                                  |
| `enter_pairing_mode`*        | (empty)                                              |

\* only if `ALLOW_PAIRING=1`.

### Info topics

The bridge publishes state to `<prefix>/info/<name>` and connection status
(`online`/`offline`) to `<prefix>/lwt` (retained, also used as the MQTT LWT).

## Deploying to NixOS (Raspberry Pi 3)

This repo is a flake exposing `packages.<system>.default` and
`nixosModules.default`. Reference it from the Pi's flake config:

```nix
{
  inputs.stanmore2.url = "github:you/marshall-stanmore-2-rust"; # or path:/... / git+ssh://...

  outputs = { self, nixpkgs, stanmore2, ... }: {
    nixosConfigurations.pi = nixpkgs.lib.nixosSystem {
      system = "aarch64-linux";
      modules = [
        stanmore2.nixosModules.default
        {
          services.stanmore2 = {
            enable = true;
            bleAddress = "00:11:22:33:44:55";
            environment = {
              MQTT_HOSTNAME = "192.168.1.10";
              MQTT_TOPIC_PREFIX = "stanmore2";
              MQTT_RETAIN = "1";
            };
            # Keep secrets out of the store:
            environmentFile = "/run/secrets/stanmore2.env"; # MQTT_PASSWORD=...
          };
        }
      ];
    };
  };
}
```

Build on a stronger machine and push the closure to the Pi — the Pi compiles
nothing:

```bash
nixos-rebuild switch \
  --flake .#pi \
  --target-host root@pi \
  --build-host localhost \
  --use-remote-sudo
```

Since the target is `aarch64-linux`, the build host must be able to produce that
architecture. Use any one of:

- a build host that is itself `aarch64-linux`, or
- emulation on the build host: `boot.binfmt.emulatedSystems = [ "aarch64-linux" ];`
  (rebuild that host first), or
- a registered remote aarch64 builder via `nix.buildMachines`.

The first build pulls the Rust toolchain and dependencies from
`cache.nixos.org`; only this crate is compiled locally.

### Local builds

```bash
nix build .#stanmore2     # result/bin/stanmore2
nix develop               # dev shell with pkg-config + dbus
```

