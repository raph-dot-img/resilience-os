# Core Resilience options: edition, application catalog, system configuration.
#
# Applications are declared as `resilience.apps.<id>`; this module turns them
# into /etc/resilience/apps.toml, which the TUI reads. Every command path is
# checked at build time, so a catalog entry can never point at a program that
# is not in the system closure.
{
  config,
  lib,
  pkgs,
  ...
}:

let
  inherit (lib)
    mkOption
    mkDefault
    types
    literalExpression
    ;
  cfg = config.resilience;
  toml = pkgs.formats.toml { };

  modeType = types.enum [
    "ultra-stable"
    "stable"
    "advanced"
  ];

  categories = [
    "knowledge"
    "maps"
    "communications"
    "networking"
    "radio"
    "security"
    "productivity"
    "entertainment"
    "system"
  ];

  appModule =
    { name, ... }:
    {
      options = {
        name = mkOption {
          type = types.str;
          description = "Name shown in the menu.";
        };
        category = mkOption {
          type = types.enum categories;
          description = "Menu section.";
        };
        description = mkOption {
          type = types.str;
          description = "One or two sentences for beginners: what it is and how to quit it.";
        };
        package = mkOption {
          type = types.nullOr types.package;
          default = null;
          description = "Package added to the system so the program is also usable from shells.";
        };
        command = mkOption {
          type = types.nonEmptyListOf types.str;
          example = literalExpression "[ (lib.getExe pkgs.nano) ]";
          description = "Program (absolute store path) and fixed arguments. Never run through a shell.";
        };
        restrictedCommand = mkOption {
          type = types.nullOr (types.nonEmptyListOf types.str);
          default = null;
          description = "Alternative argv for Ultra-Stable accounts (e.g. a read-only or restricted mode).";
        };
        minMode = mkOption {
          type = modeType;
          default = "ultra-stable";
          description = "Least-capable security mode that is offered this entry.";
        };
        pauseAfterExit = mkOption {
          type = types.bool;
          default = false;
          description = "Wait for Enter after the program exits (for programs that print and exit).";
        };
        keywords = mkOption {
          type = types.listOf types.str;
          default = [ ];
          description = "Extra search terms.";
        };
      };
      config.name = mkDefault name;
    };

  appToToml =
    id: a:
    {
      inherit id;
      inherit (a)
        name
        category
        description
        command
        keywords
        ;
      min_mode = a.minMode;
      pause_after_exit = a.pauseAfterExit;
    }
    // lib.optionalAttrs (a.restrictedCommand != null) { restricted_command = a.restrictedCommand; };

  catalogUnchecked = toml.generate "resilience-apps.toml" {
    apps = lib.mapAttrsToList appToToml cfg.apps;
  };

  programs = lib.unique (
    lib.concatLists (
      lib.mapAttrsToList (
        _: a:
        [ (builtins.head a.command) ]
        ++ lib.optional (a.restrictedCommand != null) (builtins.head a.restrictedCommand)
      ) cfg.apps
    )
  );

  # Fails the system build if any catalog program is missing or not executable.
  catalog = pkgs.runCommand "resilience-apps-checked.toml" { } ''
    ${lib.concatMapStrings (p: ''
      if [ ! -x ${lib.escapeShellArg p} ]; then
        echo "resilience: catalog program is not executable: ${p}" >&2
        exit 1
      fi
    '') programs}
    cp ${catalogUnchecked} $out
  '';

  # Programs used for removable media and backups (absolute store paths).
  tools = {
    lsblk = "${lib.getBin pkgs.util-linux}/bin/lsblk";
    udisksctl = "${lib.getBin config.services.udisks2.package}/bin/udisksctl";
    tar = "${pkgs.gnutar}/bin/tar";
    zstd = "${lib.getBin pkgs.zstd}/bin/zstd";
    age = lib.getExe pkgs.age;
    sha256sum = "${pkgs.coreutils}/bin/sha256sum";
    backup = "${cfg.package}/bin/resilience-backup";
    nmcli = "${lib.getBin config.networking.networkmanager.package}/bin/nmcli";
    library = "${cfg.package}/bin/resilience-library";
    maps = "${cfg.package}/bin/resilience-maps";
    man = "${config.documentation.man.man-db.package}/bin/man";
    apropos = "${config.documentation.man.man-db.package}/bin/apropos";
    less = "${pkgs.less}/bin/less";
    pdftotext = "${lib.getBin pkgs.poppler-utils}/bin/pdftotext";
    bsdtar = "${lib.getBin pkgs.libarchive}/bin/bsdtar";
    # Cryptography (ADR 0015).
    crypto = "${cfg.package}/bin/resilience-crypto";
    age_keygen = "${pkgs.age}/bin/age-keygen";
    age_inspect = "${pkgs.age}/bin/age-inspect";
    minisign = lib.getExe pkgs.minisign;
    sha512sum = "${pkgs.coreutils}/bin/sha512sum";
    sha1sum = "${pkgs.coreutils}/bin/sha1sum";
    md5sum = "${pkgs.coreutils}/bin/md5sum";
    b2sum = "${pkgs.coreutils}/bin/b2sum";
    xkcdpass = "${pkgs.xkcdpass}/bin/xkcdpass";
    openssl = "${lib.getBin pkgs.openssl}/bin/openssl";
    gpg = "${config.programs.gnupg.package}/bin/gpg";
  };

  systemConfigUnchecked = toml.generate "resilience-system.toml" (
    {
      edition = cfg.edition;
      inherit (cfg) version;
      catalog = "/etc/resilience/apps.toml";
      handbook_dir = "${cfg.handbookPackage}/share/resilience/handbook";
      editor = [ (lib.getExe pkgs.nano) ];
      restricted_editor = [
        (lib.getExe pkgs.nano)
        "--restricted"
      ];
      shell = [
        (lib.getExe pkgs.bashInteractive)
        "-i"
      ];
      probe = {
        targets = cfg.probe.targets;
        dns_name = cfg.probe.dnsName;
        timeout_ms = cfg.probe.timeoutMs;
      };
      # Removable media and backups (media.nix, ADR 0008), Wi-Fi (ADR 0010).
      inherit tools;
      keyboard = {
        default = config.resilience.keyboard.layout;
        layouts = map (l: { inherit (l) id label; }) config.resilience.keyboard.layouts;
      }
      // lib.optionalAttrs (config.resilience.keyboard.stateFile != null) {
        state_file = config.resilience.keyboard.stateFile;
      };
    }
    // lib.optionalAttrs (cfg.updates.settings != null) {
      # Offline updates (ADR 0023): the TUI offers and checks them.
      updates = cfg.updates.settings;
    }
    // {
      # The licence notice (ADR 0024).
      licence = cfg.licence.settings;
    }
  );

  # Fails the system build if a tool path is wrong.
  systemConfig = pkgs.runCommand "resilience-system-checked.toml" { } ''
    ${lib.concatMapStrings (p: ''
      if [ ! -x ${lib.escapeShellArg p} ]; then
        echo "resilience: tool is not executable: ${p}" >&2
        exit 1
      fi
    '') (lib.attrValues tools)}
    cp ${systemConfigUnchecked} $out
  '';
in
{
  options.resilience = {
    edition = mkOption {
      type = types.enum [
        "minimal"
        "standard"
        "standard-gui"
        "extended"
      ];
      default = "standard";
      description = "Build flavour; selects which optional application groups are enabled by default.";
    };

    version = mkOption {
      type = types.strMatching "(0|[1-9][0-9]*)[.](0|[1-9][0-9]*)[.](0|[1-9][0-9]*)";
      default = cfg.package.version;
      defaultText = literalExpression "config.resilience.package.version";
      description = ''
        Release version of this system (MAJOR.MINOR.PATCH). Shown to people
        and compared by offline updates (ADR 0023): an update must carry a
        greater one. Only tests set it.
      '';
    };

    package = mkOption {
      type = types.package;
      default = pkgs.callPackage ../packages/resilience { };
      defaultText = literalExpression "pkgs.callPackage ./nix/packages/resilience { }";
      description = "The Resilience TUI package.";
    };

    sessionPackage = mkOption {
      type = types.package;
      default = pkgs.callPackage ../packages/resilience-session { resilience = cfg.package; };
      defaultText = literalExpression "pkgs.callPackage ./nix/packages/resilience-session { }";
      description = "Login shell wrapper that runs the TUI.";
    };

    handbookPackage = mkOption {
      type = types.package;
      default = pkgs.callPackage ../packages/resilience-handbook { };
      defaultText = literalExpression "pkgs.callPackage ./nix/packages/resilience-handbook { }";
      description = "Offline handbook pages.";
    };

    apps = mkOption {
      type = types.attrsOf (types.submodule appModule);
      default = { };
      description = "Applications offered by the TUI launcher.";
    };

    probe = {
      targets = mkOption {
        type = types.listOf types.str;
        default = [
          "9.9.9.9:443"
          "1.1.1.1:443"
        ];
        description = ''
          `ip:port` pairs contacted by the on-demand Internet probe (TCP connect only).
          The probe runs only when the user asks for it.
        '';
      };
      dnsName = mkOption {
        type = types.str;
        default = "nixos.org";
        description = "Hostname resolved by the on-demand DNS probe. Empty disables the DNS probe.";
      };
      timeoutMs = mkOption {
        type = types.ints.between 100 10000;
        default = 2500;
        description = "Timeout per probe step, in milliseconds.";
      };
    };
  };

  config = {
    system.nixos.distroName = "Resilience OS";

    environment.etc."resilience/apps.toml".source = catalog;
    environment.etc."resilience/system.toml".source = systemConfig;

    environment.systemPackages = [
      cfg.package
      cfg.handbookPackage
      # A backup can also be opened by hand (handbook, "Backups").
      pkgs.age
      pkgs.zstd
    ]
    ++ lib.filter (p: p != null) (lib.mapAttrsToList (_: a: a.package) cfg.apps);
  };
}
