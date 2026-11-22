{
  description = "Resilience OS — an offline-first, terminal-only operating system based on NixOS";

  inputs = {
    # Production systems track the NixOS stable release branch only.
    # Bump deliberately with `nix flake update` and record it in CHANGELOG.md.
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";
  };

  outputs =
    { self, nixpkgs }:
    let
      system = "x86_64-linux";
      lib = nixpkgs.lib;
      pkgs = import nixpkgs {
        inherit system;
        overlays = [ self.overlays.default ];
      };

      mkSystem =
        modules:
        lib.nixosSystem {
          inherit system;
          modules = [
            self.nixosModules.default
            ./nix/systems/common.nix
          ]
          ++ modules;
        };

      # The systems resilience-install copies (ADR 0022), prebuilt and
      # carried by the live ISO of the same edition.
      installedSystems = lib.listToAttrs (
        lib.concatMap
          (
            edition:
            lib.concatMap
              (
                boot:
                map
                  (
                    disk:
                    lib.nameValuePair "installed-${edition}-${boot}-${disk}" (mkSystem [
                      ./nix/profiles/${edition}.nix
                      (import ./nix/systems/installed.nix {
                        inherit boot;
                        encrypted = disk == "luks";
                      })
                    ])
                  )
                  [
                    "plain"
                    "luks"
                  ]
              )
              [
                "uefi"
                "bios"
              ]
          )
          [
            "standard"
            "standard-gui"
            "minimal"
          ]
      );

      # The live ISO of an edition carries that edition's installed systems.
      installerFor = edition: {
        resilience.installer = {
          enable = true;
          systems = lib.genAttrs [ "uefi_plain" "uefi_luks" "bios_plain" "bios_luks" ] (
            n:
            installedSystems."installed-${edition}-${
              lib.replaceStrings [ "_" ] [ "-" ] n
            }".config.system.build.toplevel
          );
        };
      };

      # The manifest of an offline update bundle of an edition (ADR 0023):
      # the new toplevel of each of its installed variants.
      updateManifest =
        edition:
        let
          names = lib.filter (n: lib.hasPrefix "installed-${edition}-" n) (lib.attrNames installedSystems);
        in
        pkgs.callPackage ./nix/lib/update-manifest.nix { } {
          inherit edition;
          version = installedSystems.${lib.head names}.config.resilience.version;
          systems = lib.genAttrs names (n: installedSystems.${n}.config.system.build.toplevel);
        };

      # VM tests that are about something else start without the licence
      # notice (ADR 0024); vm-licence and the installed system of
      # vm-install go through it.
      noLicenceNotice =
        { options, lib, ... }:
        {
          config = lib.optionalAttrs (options ? resilience) {
            resilience.licence.prompt = lib.mkDefault false;
          };
        };
      runVmTest =
        test:
        pkgs.testers.runNixOSTest {
          imports = [ test ];
          defaults = noLicenceNotice;
        };
      vmTest = file: runVmTest (import file { inherit self; });

      # Map packs (ADR 0016 §2), for build machines: not in any system.
      maps = pkgs.callPackage ./nix/maps { };
      # Local messaging (ADR 0018): unfree Reticulum and LXMF, opt-in.
      reticulum = pkgs.callPackage ./nix/packages/resilience-reticulum { };
      messages = pkgs.callPackage ./nix/packages/resilience-messages { inherit reticulum; };
      mapChecks = pkgs.callPackage ./nix/maps/checks.nix {
        inherit maps;
        worldMap = pkgs.resilience-world-map;
      };
    in
    {
      overlays.default = import ./nix/overlays/default.nix;

      nixosModules.default = ./nix/modules;

      nixosConfigurations = {
        live-standard = mkSystem [
          ./nix/profiles/standard.nix
          ./nix/systems/live-iso.nix
          (installerFor "standard")
        ];
        live-standard-gui = mkSystem [
          ./nix/profiles/standard-gui.nix
          ./nix/systems/live-iso.nix
          (installerFor "standard-gui")
        ];
        live-minimal = mkSystem [
          ./nix/profiles/minimal.nix
          ./nix/systems/live-iso.nix
          (installerFor "minimal")
        ];
        dev-vm = mkSystem [
          ./nix/profiles/standard.nix
          ./nix/systems/dev-vm.nix
        ];
      }
      # Installed systems (ADR 0022): installed-<edition>-<uefi|bios>-<plain|luks>.
      // installedSystems;

      packages.${system} = {
        inherit (pkgs)
          resilience
          resilience-session
          resilience-gui
          resilience-handbook
          resilience-world-map
          ;
        default = pkgs.resilience;
        iso = self.nixosConfigurations.live-standard.config.system.build.isoImage;
        iso-minimal = self.nixosConfigurations.live-minimal.config.system.build.isoImage;
        iso-gui = self.nixosConfigurations.live-standard-gui.config.system.build.isoImage;
        vm = self.nixosConfigurations.dev-vm.config.system.build.vm;
        # Offline update manifests (ADR 0023); bundles are made and signed
        # with scripts/make-update-bundle.sh.
        update-manifest-standard = updateManifest "standard";
        update-manifest-minimal = updateManifest "minimal";
        update-manifest-standard-gui = updateManifest "standard-gui";
      };

      # `nix build .#mapPacks.<id>` -> result/map-osm-<id>_<YYYY-MM>.mbtiles.
      # A set of packs is not a derivation, so it lives in legacyPackages
      # (`packages` may only hold derivations); `nix build` finds it there.
      legacyPackages.${system} = {
        mapPacks = maps.packs;
        # README screenshots (tests/screenshots.nix): `nix build .#screenshots`.
        screenshots = vmTest ./tests/screenshots.nix;
      };

      apps.${system}.vm = {
        type = "app";
        program = "${self.packages.${system}.vm}/bin/run-resilience-dev-vm";
        meta.description = "Run the Resilience development VM (2 vCPU, 2 GiB RAM, serial console)";
      };

      checks.${system} = {
        # Builds the Rust workspace and runs `cargo test`.
        resilience = pkgs.resilience;
        vm-boot-offline = vmTest ./tests/vm/boot-offline.nix;
        vm-modes = vmTest ./tests/vm/modes.nix;
        vm-admin = vmTest ./tests/vm/admin.nix;
        vm-backup = vmTest ./tests/vm/backup.nix;
        vm-wifi = vmTest ./tests/vm/wifi.nix;
        vm-generations = vmTest ./tests/vm/generations.nix;
        vm-packs = vmTest ./tests/vm/packs.nix;
        vm-documents = vmTest ./tests/vm/documents.nix;
        vm-crypto = vmTest ./tests/vm/crypto.nix;
        vm-install = vmTest ./tests/vm/install.nix;
        vm-install-uefi = runVmTest (
          import ./tests/vm/install.nix {
            inherit self;
            firmware = "uefi";
          }
        );
        vm-maps = vmTest ./tests/vm/maps.nix;
        vm-comms = vmTest ./tests/vm/comms.nix;
        vm-mesh = vmTest ./tests/vm/mesh.nix;
        vm-share = vmTest ./tests/vm/share.nix;
        vm-hotspot = vmTest ./tests/vm/hotspot.nix;
        vm-cable = vmTest ./tests/vm/cable.nix;
        vm-update = vmTest ./tests/vm/update.nix;
        vm-licence = vmTest ./tests/vm/licence.nix;
        vm-gui = vmTest ./tests/vm/gui.nix;
        # The messaging program with the real Reticulum and LXMF, no VM.
        comms-messages = pkgs.callPackage ./nix/packages/resilience-messages/check.nix {
          inherit reticulum messages;
        };
        # The synthetic test pack: reproducible, and read back by the reader.
        # The built-in world overview (Natural Earth), read back the same way.
        inherit (mapChecks) map-pack-reproducible map-pack-testland map-world;
        security-invariants = pkgs.callPackage ./tests/eval/security-invariants.nix {
          configurations = {
            inherit (self.nixosConfigurations) live-standard live-minimal dev-vm;
          }
          // installedSystems;
        };
        # Evaluates the ISO configurations without building the images.
        # (Context is discarded on purpose: this check must not build them.)
        eval-live-iso = pkgs.writeText "eval-live-iso" (
          builtins.unsafeDiscardStringContext (
            builtins.toJSON {
              standard = self.nixosConfigurations.live-standard.config.system.build.toplevel.drvPath;
              minimal = self.nixosConfigurations.live-minimal.config.system.build.toplevel.drvPath;
              dev-vm = self.nixosConfigurations.dev-vm.config.system.build.vm.drvPath;
            }
          )
        );
      };

      devShells.${system}.default = pkgs.mkShell {
        packages = with pkgs; [
          cargo
          rustc
          clippy
          rustfmt
          rust-analyzer
          cargo-deny
          nixfmt
          qemu_kvm
        ];
        RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
      };

      formatter.${system} = pkgs.nixfmt-tree;
    };
}
