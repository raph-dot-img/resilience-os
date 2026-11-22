# Graphical interface (ADR 0026): with `resilience.gui.enable`, the
# login session on tty1 runs /etc/resilience/gui-session = cage (Wayland
# kiosk, software renderer, VT switching allowed) showing resilience-gui
# full screen. Programs open in foot on top of it. tty2-tty6 keep their
# text logins and the TUI; resilience-session falls back to the TUI on tty1
# when the GUI fails three times in a row.
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.resilience;
  gui = cfg.gui.package;
  # No X11 at all (ADR 0026 §1): wlroots and cage without Xwayland, which
  # alone brought ~180 MiB (Xwayland, GTK through libdecor).
  cage = pkgs.cage.override {
    xwayland = null;
    wlroots_0_20 = pkgs.wlroots_0_20.override { enableXWayland = false; };
  };
  # Exactly the fonts the GUI and its terminal use, copied (not linked, so
  # the 23 MiB font packages stay out of the system: ~3 MiB instead).
  fonts = pkgs.runCommand "resilience-gui-fonts" { } ''
    mkdir -p $out
    cp ${pkgs.inter}/share/fonts/truetype/InterVariable.ttf $out/
    for f in DejaVuSans DejaVuSansMono DejaVuSansMono-Bold; do
      cp -L ${pkgs.dejavu_fonts}/share/fonts/truetype/$f.ttf $out/
    done
  '';
  # foot in the GUI's colours; no shell of its own (it runs one program).
  footIni = pkgs.writeText "resilience-foot.ini" ''
    font=DejaVu Sans Mono:size=12
    pad=16x12
    [colors]
    background=111317
    foreground=f1f3f5
    regular0=111317
    regular1=f87171
    regular2=4ade80
    regular3=fbbf24
    regular4=60a5fa
    regular5=c084fc
    regular6=38bdf8
    regular7=f1f3f5
    [csd]
    preferred=none
  '';
  terminal = pkgs.writeShellScript "resilience-terminal" ''
    exec ${pkgs.foot}/bin/foot --config=${footIni} "$@"
  '';
  session = pkgs.writeShellScript "resilience-gui-session" ''
    set -eu
    # The account's keyboard layout, as XKB names (Wayland ignores the
    # console keymap).
    set -- $(${lib.getExe gui} xkb)
    export XKB_DEFAULT_LAYOUT="''${1:-us}" XKB_DEFAULT_VARIANT="''${2:-}"
    export WLR_RENDERER=pixman WLR_NO_HARDWARE_CURSORS=1
    export RESILIENCE_GUI_FONTS=${fonts}
    export RESILIENCE_TERMINAL=${terminal}
    export FONTCONFIG_FILE=${pkgs.makeFontsConf { fontDirectories = [ fonts ]; }}
    # -d: no client decorations; -s: Ctrl+Alt+F2... still reach the other
    # consoles (recovery, ADR 0026 §5).
    exec ${lib.getExe cage} -d -s -- ${lib.getExe gui}
  '';
in
{
  options.resilience = {
    gui.enable = lib.mkEnableOption ''
      the graphical interface on tty1 (ADR 0026; other consoles always use
      the terminal interface)
    '';
    gui.package = lib.mkOption {
      type = lib.types.package;
      default = pkgs.callPackage ../packages/resilience-gui { };
      defaultText = lib.literalExpression "pkgs.callPackage ../packages/resilience-gui { }";
      description = "The resilience-gui package.";
    };
  };

  config = lib.mkIf cfg.gui.enable {
    environment.etc."resilience/gui-session".source = session;
    # Pointer and touch screens; logind gives the tty1 session its seat.
    services.libinput.enable = lib.mkDefault true;
    environment.systemPackages = [ gui ];
  };
}
