# CLAUDE.md

Marshall Stanmore II BLE↔MQTT bridge with Home Assistant auto-discovery. Rust
rewrite of a Python project. Connects to the speaker over Bluetooth LE, exposes
its controls/status over MQTT, and publishes HA discovery configs on startup.

Binary name is `stanmore2` (crate is `marshall-stanmore-2`).

## Build & run

```bash
cargo build                      # dev build
cargo build --release
BLE_ADDRESS=54:B7:E5:A2:CA:41 MQTT_HOSTNAME=192.168.1.10 cargo run --release

nix build .#stanmore2            # -> result/bin/stanmore2
nix develop                      # dev shell (pkg-config + dbus)
nix flake check
```

The speaker's BLE MAC is `54:B7:E5:A2:CA:41` (advertised as "STANMORE II").
All config is via env vars — see README.md for the full table. `BLE_ADDRESS` is
required; `RUST_LOG` controls verbosity.

## Deployment

Target is a Raspberry Pi 3 (aarch64) running NixOS. The Pi compiles nothing
(1GB RAM) — build on a stronger machine and push the closure via
`nixos-rebuild switch --flake .#pi --target-host root@pi --build-host localhost`.
The flake exposes `nixosModules.default` (`services.stanmore2`). See README.md.

## Architecture

- `src/main.rs` — entrypoint. Loads `Config` from env, connects BLE, sets up the
  MQTT client (with LWT), publishes discovery + initial state, and runs the MQTT
  event loop. Spawns: initial-state push, BLE notification pump, shutdown handler.
- `src/ble.rs` — `Stanmore` struct wrapping the btleplug peripheral. Owns the
  characteristic handles, the canonical service UUIDs, command/getter methods,
  and notification decoding (`decode_notification`, media-info buffering).
- `src/mqtt.rs` — `App` struct ties speaker + MQTT together. `handle_command`
  dispatches `<prefix>/command/<action>`; publishes state to `<prefix>/info/<name>`.
- `src/discovery.rs` — builds HA MQTT discovery configs (one shared `device`
  block, shared `<prefix>/lwt` availability topic).
- `src/types.rs` — domain types (EqProfile/EqPreset, AudioSource, PlayStatus,
  Status, MediaInfo), `SpeakerError`, and the `cmd` command-byte constants.

## Behavior notes

- **BLE reconnect is crash-and-restart, not in-process.** On BLE disconnect the
  notification pump in `main.rs` calls `std::process::exit(1)`; the supervisor
  (systemd `Restart=on-failure` / docker `restart: unless-stopped`) restarts it.
- The underlying command/info MQTT topics match the original Python project, so
  manual integrations keep working — HA discovery is layered on top.
- Linux BLE pulls in libdbus (btleplug → bluez-async → dbus → libdbus-sys);
  that's why the flake/Docker need pkg-config + dbus.

## Conventions

- macOS can't be used for live BLE testing here (CoreBluetooth uses UUIDs, not
  MACs) — test on the Pi. Don't add macOS-specific BLE matching code.
- No unused/dead code; keep it building warning-clean.
