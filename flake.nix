{
  description = "DT Fetcher";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    crane = {
      url = "github:ipetkov/crane";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = {
    self,
    nixpkgs,
    rust-overlay,
    crane,
    flake-utils,
  }: let
    overlays = [
      (import rust-overlay)
    ];

    macFrameworks = pkgs: let
      frameworks = pkgs.darwin.apple_sdk.frameworks;
    in
      with frameworks; [
        CoreFoundation
        CoreServices
        Security
        SystemConfiguration
      ];
  in
    flake-utils.lib.eachDefaultSystem (
      system: let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [(import rust-overlay)];
        };
        rust = pkgs.rust-bin.stable.latest.default;
        craneLib = (crane.mkLib pkgs).overrideToolchain rust;
        overridableCrate = pkgs.lib.makeOverridable ({toolchain}: let
          craneLib = (crane.mkLib pkgs).overrideToolchain toolchain;
        in
          craneLib.buildPackage {
            pname = "dt-fetcher";
            version = "0.1.0";
            src = craneLib.cleanCargoSource (craneLib.path ./.);
            strictDeps = true;
            nativeBuildInputs = [
              pkgs.pkg-config
            ];
            buildInputs =
              [
                pkgs.openssl
              ]
              ++ pkgs.lib.optionals pkgs.stdenv.isDarwin (macFrameworks pkgs);
          }) {toolchain = rust;};
        container = crate: {
          name = crate.pname;
          tag = "latest";
          created = "now";
          contents = [pkgs.cacert];
          config = {
            Labels = {
              "org.opencontainers.image.source" = "https://github.com/capslock/dt-fetcher";
              "org.opencontainers.image.description" = "DT Fetcher";
              "org.opencontainers.image.licenses" = "MIT";
            };
            Entrypoint = ["${crate}/bin/${crate.name}"];
          };
        };
      in {
        packages = {
          default = overridableCrate;
          container = pkgs.lib.makeOverridable ({crate}: pkgs.dockerTools.buildLayeredImage (container crate)) {crate = overridableCrate;};
          streamedContainer = pkgs.lib.makeOverridable ({crate}: pkgs.dockerTools.streamLayeredImage (container crate)) {crate = overridableCrate;};
        };
        checks = {inherit overridableCrate;};
        apps.default = flake-utils.lib.mkApp {
          drv = overridableCrate;
        };
        devShells.default = craneLib.devShell {
          buildInputs = [pkgs.pkg-config pkgs.openssl.dev pkgs.sqlite];
          checks = self.checks.${system};
          packages = pkgs.lib.optionals pkgs.stdenv.isDarwin (
            with pkgs;
              [
                libiconv
              ]
              ++ macFrameworks pkgs
          );
        };
      }
    )
    // {
      nixosModules.default = {
        config,
        lib,
        pkgs,
        ...
      }: let
        cfg = config.services.dtFetcher;
        settingsFormat = pkgs.formats.toml {};
      in
        with lib; {
          options = {
            services.dtFetcher = {
              enable = mkOption {
                default = false;
                type = with types; bool;
                description = ''
                  Start the dt fetcher.
                '';
              };
              environmentFile = mkOption {
                example = "./dtfetcher.env";
                type = with types; nullOr str;
                default = null;
                description = ''
                  File which contains environment settings for the dt-fetcher service.
                '';
              };
              environment = mkOption {
                example = "RUSTLOG=info";
                type = with types; str;
                default = "\"RUSTLOG=info,hyper=error\"";
                description = ''
                  Environment settings for the dt-fetcher service.
                '';
              };
              persistAuth = mkOption {
                type = with types; bool;
                default = false;
                description = ''
                  Persist authentication tokens to disk.
                '';
              };
              disableSingle = mkOption {
                type = with types; bool;
                default = false;
                description = ''
                  Disable single-account endpoints.
                '';
              };
              listenAddr = mkOption {
                type = with types; nullOr str;
                default = null;
                description = ''
                  Address to listen on.
                '';
              };
              package = mkOption {
                type = types.package;
                default = self.packages.${pkgs.system}.default;
                defaultText = literalExpression "self.packages.$\{pkgs.system\}.default";
                description = ''
                  dt-fetcher package to use.
                '';
              };
            };
          };

          config = mkIf cfg.enable {
            systemd.services.dtFetcher = {
              wantedBy = ["multi-user.target"];
              after = ["network-online.target"];
              description = "DT Fetcher";
              serviceConfig = let
                pkg = cfg.package;
                args =
                  concatStringsSep " "
                  ([
                      "--log-to-systemd"
                    ]
                    ++ (pkgs.lib.optionals cfg.persistAuth
                      [
                        "--db-path"
                        "$\{STATE_DIRECTORY\}/db.sled"
                      ])
                    ++ (pkgs.lib.optional cfg.disableSingle "--disable-single")
                    ++ (pkgs.lib.optionals (cfg.listenAddr != null)
                      [
                        "--listen-addr"
                        cfg.listenAddr
                      ]));
              in {
                Type = "exec";
                DynamicUser = true;
                ExecStart = "${pkg}/bin/dt-fetcher ${args}";
                EnvironmentFile = mkIf (cfg.environmentFile != null) cfg.environmentFile;
                Environment = cfg.environment;
                NoNewPrivileges = true;
                PrivateTmp = true;
                PrivateDevices = true;
                DevicePolicy = "closed";
                ProtectSystem = "strict";
                ProtectHome = "read-only";
                ProtectControlGroups = true;
                ProtectKernelLogs = true;
                ProtectKernelModules = true;
                ProtectKernelTunables = true;
                RestrictNamespaces = true;
                RestrictAddressFamilies = ["AF_INET" "AF_INET6" "AF_UNIX"];
                RestrictRealtime = true;
                RestrictSUIDSGID = true;
                LockPersonality = true;
                CapabilityBoundingSet = [""];
                ProcSubset = "pid";
                ProtectClock = true;
                ProtectProc = "noaccess";
                ProtectHostname = true;
                StateDirectory = mkIf cfg.persistAuth "dt-fetcher";
                SystemCallArchitectures = "native";
                SystemCallFilter = ["@system-service" "~@resources" "~@privileged"];
                UMask = "0077";
                PrivateUsers = true;
                MemoryDenyWriteExecute = true;
              };
            };
          };
        };
    };
}
