{
  description = "Marshall Stanmore II BLE <-> MQTT bridge with Home Assistant discovery";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    crane.url = "github:ipetkov/crane";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, crane, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        craneLib = crane.mkLib pkgs;

        commonArgs = {
          src = craneLib.cleanCargoSource ./.;
          strictDeps = true;

          # btleplug's Linux backend links libdbus (bluez-async -> dbus -> libdbus-sys).
          nativeBuildInputs = [ pkgs.pkg-config ];
          buildInputs = [ pkgs.dbus ];
        };

        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        stanmore2 = craneLib.buildPackage (commonArgs // {
          inherit cargoArtifacts;
          meta.mainProgram = "stanmore2";
        });
      in
      {
        packages = {
          default = stanmore2;
          stanmore2 = stanmore2;
        };

        checks.stanmore2 = stanmore2;

        devShells.default = craneLib.devShell {
          packages = [ pkgs.pkg-config pkgs.dbus ];
        };
      })
    // {
      # NixOS service module. Import on the RPi 3 config and set
      # `services.stanmore2.bleAddress`.
      nixosModules.default = { config, lib, pkgs, ... }:
        let
          cfg = config.services.stanmore2;
        in
        {
          options.services.stanmore2 = {
            enable = lib.mkEnableOption "Marshall Stanmore II MQTT bridge";

            package = lib.mkOption {
              type = lib.types.package;
              default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
              description = "stanmore2 package to run.";
            };

            bleAddress = lib.mkOption {
              type = lib.types.str;
              example = "00:11:22:33:44:55";
              description = "BLE MAC address of the speaker (BLE_ADDRESS).";
            };

            environment = lib.mkOption {
              type = lib.types.attrsOf lib.types.str;
              default = { };
              example = {
                MQTT_HOSTNAME = "192.168.1.10";
                MQTT_TOPIC_PREFIX = "stanmore2";
                MQTT_RETAIN = "1";
              };
              description = "Extra environment variables (MQTT_*, HA_DISCOVERY_*, etc.).";
            };

            environmentFile = lib.mkOption {
              type = lib.types.nullOr lib.types.path;
              default = null;
              description = "File with secrets like MQTT_PASSWORD (systemd EnvironmentFile).";
            };
          };

          config = lib.mkIf cfg.enable {
            # The RPi 3's onboard bluetooth is on UART and needs the controller up.
            hardware.bluetooth.enable = lib.mkDefault true;

            systemd.services.stanmore2 = {
              description = "Marshall Stanmore II MQTT bridge";
              wantedBy = [ "multi-user.target" ];
              wants = [ "network-online.target" ];
              after = [ "network-online.target" "bluetooth.target" ];

              environment = { BLE_ADDRESS = cfg.bleAddress; } // cfg.environment;

              serviceConfig = {
                ExecStart = lib.getExe cfg.package;
                Restart = "on-failure";
                RestartSec = 5;
                EnvironmentFile = lib.mkIf (cfg.environmentFile != null) cfg.environmentFile;
                # BlueZ system-bus access is simplest as root; tighten with a
                # dedicated user + bluetooth group/polkit rule if desired.
              };
            };
          };
        };
    };
}
