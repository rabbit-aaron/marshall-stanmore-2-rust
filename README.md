# marshall-stanmore-2 (Rust)

A Rust rewrite of the Python Marshall Stanmore II BLE↔MQTT bridge, with
**Home Assistant MQTT auto-discovery** support.

The daemon connects to the speaker over Bluetooth LE (via BlueZ / `btleplug`),
exposes its controls and status over MQTT, and on startup publishes Home
Assistant discovery configs so the speaker shows up automatically as a device
with all its entities.

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

