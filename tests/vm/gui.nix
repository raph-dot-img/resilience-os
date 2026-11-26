# The graphical edition (ADR 0026): tty1 runs cage + resilience-gui for an
# autologin account; the GUI is driven with the keyboard and read by OCR;
# a program opens in foot and comes back; Ctrl+Alt+F2 still reaches a text
# login; three GUI failures fall back to the TUI on tty1.
{ self }:
{
  name = "resilience-gui";

  enableOCR = true;

  nodes.machine =
    { lib, ... }:
    {
      imports = [
        self.nixosModules.default
        ../../nix/systems/common.nix
        ../../nix/profiles/standard-gui.nix
      ];
      virtualisation.memorySize = 2048;
      virtualisation.cores = 2;
      virtualisation.resolution = {
        x = 1366;
        y = 768;
      };
      users.users.root.hashedPassword = lib.mkForce null;
      resilience.users.alice.mode = "ultra-stable";
      users.users.alice.hashedPassword = "!";
      resilience.interface.autologinUser = "alice";
    };

  testScript = ''
    import time

    # nixpkgs may wrap cage in a script: the process is then .cage-wrapped.
    CAGE = "'^\\.?cage(-wrapped)?$'"

    def rss_mib(pattern):
        out = machine.succeed(f"ps -o rss= -p \"$(pgrep -d, -u alice {pattern})\" | awk '{{s+=$1}} END {{print s+0}}'")
        return int(out.strip()) / 1024

    machine.start()
    machine.wait_for_unit("multi-user.target")

    def diagnose():
        print(machine.execute("ps -eo user,pid,tty,args | grep -v ' \\[' | tail -40")[1])
        print(machine.execute("loginctl list-sessions; ls -la /dev/dri /run/user || true")[1])
        print(machine.execute("journalctl -b --no-pager | grep -iE 'cage|wlr|gui|seat|drm|login|session' | tail -60")[1])

    with subtest("tty1 starts the graphical interface (cage + resilience-gui)"):
        try:
            machine.wait_until_succeeds(f"pgrep -u alice {CAGE}", timeout=120)
        except Exception:
            diagnose()
            raise
        machine.wait_until_succeeds("pgrep -u alice -x resilience-gui", timeout=60)
        machine.fail("pgrep -u alice -x resilience")
        machine.wait_for_text("Knowledge", timeout=120)
        machine.screenshot("gui-home")

    with subtest("memory at idle (ADR 0026 target: 150 MB for cage + GUI)"):
        time.sleep(5)
        cage, gui = rss_mib(CAGE), rss_mib("-x resilience-gui")
        print(f"RSS: cage {cage:.1f} MiB, resilience-gui {gui:.1f} MiB, total {cage + gui:.1f} MiB")
        assert cage + gui < 300, "far above the ADR 0026 target"

    with subtest("keyboard: open a section and a built-in screen"):
        machine.send_key("ret")
        machine.wait_for_text("Handbook", timeout=90)
        machine.send_key("ret")
        machine.wait_for_text("First steps", timeout=90)
        machine.send_key("ret")
        machine.wait_for_text("MOVING AROUND", timeout=90)
        machine.send_key("esc")
        machine.screenshot("gui-handbook")
        machine.send_key("esc")
        machine.send_key("esc")
        machine.wait_for_text("Hello", timeout=90)

    with subtest("a native dialog: Cancel is the default"):
        machine.send_key("q")
        machine.wait_for_text("Log out", timeout=90)
        machine.screenshot("gui-dialog")
        machine.send_key("ret")
        time.sleep(3)
        machine.succeed("pgrep -u alice -x resilience-gui")
        machine.wait_for_text("Hello", timeout=90)

    with subtest("native offline library and Find files"):
        machine.send_key("ret")
        machine.wait_for_text("Handbook", timeout=90)
        machine.send_key("right")
        machine.send_key("right")
        machine.send_key("ret")
        machine.wait_for_text("System documentation", timeout=90)
        machine.screenshot("gui-library")
        machine.send_key("esc")
        machine.send_key("esc")
        machine.succeed("runuser -u alice -- sh -c 'mkdir -p ~/Documents && echo hello > ~/Documents/field-notes.txt'")
        machine.send_chars("/view files")
        machine.wait_for_text("view files", timeout=90)
        machine.send_key("ret")
        machine.send_chars("field")
        machine.wait_for_text("field-notes", timeout=90)
        machine.screenshot("gui-files")
        machine.send_key("esc")
        machine.send_key("esc")
        machine.wait_for_text("Hello", timeout=90)

    with subtest("native local messages"):
        machine.wait_until_succeeds("test -e /run/resilience-reticulum/client/config", timeout=120)
        machine.send_chars("/local mess")
        machine.wait_for_text("Local messages", timeout=90)
        machine.send_key("ret")
        machine.wait_for_text("Messaging is on", timeout=120)
        machine.screenshot("gui-messages")
        machine.send_key("p")
        machine.wait_for_text("People nearby", timeout=90)
        machine.send_key("esc")
        machine.send_key("esc")
        machine.send_key("esc")
        machine.wait_for_text("Hello", timeout=90)

    with subtest("search, then a program opens in foot and comes back"):
        # Nano needs Stable; the unit converter is offered to Ultra-Stable.
        machine.send_chars("/unit conv")
        machine.wait_for_text("converter", timeout=90)
        machine.screenshot("gui-search")
        machine.send_key("ret")
        machine.wait_until_succeeds("pgrep -u alice -x foot", timeout=30)
        machine.wait_until_succeeds("pgrep -u alice -x units", timeout=30)
        machine.screenshot("gui-program")
        machine.send_key("ctrl-d")
        machine.wait_until_fails("pgrep -u alice -x foot", timeout=30)
        machine.wait_until_succeeds("pgrep -u alice -x resilience-gui", timeout=10)
        # No request file is left behind.
        machine.fail("ls /run/user/*/resilience-run-* 2>/dev/null | grep -q .")

    with subtest("the map opens in the graphical viewer, a separate limited process"):
        # ADR 0026 amendment 1: resilience-maps-gui, not foot. From the
        # main screen (the search above is still open).
        machine.send_key("esc")
        machine.send_key("esc")
        machine.wait_for_text("Hello", timeout=90)
        machine.send_chars("/map")
        machine.wait_for_text("Map", timeout=90)
        machine.screenshot("gui-map-search")
        # The first hit is the section Maps & navigation; its first tile
        # is Map.
        machine.send_key("ret")
        machine.wait_for_text("Make or update", timeout=90)
        machine.send_key("ret")
        try:
            machine.wait_until_succeeds("pgrep -u alice -x resilience-maps", timeout=30)
        except Exception:
            print(machine.get_screen_text())
            print(machine.execute("ps -u alice -o pid,args; tail -30 /home/alice/.local/state/resilience/gui.log")[1])
            raise
        machine.fail("pgrep -u alice -x foot")
        pid = machine.succeed("pgrep -u alice -x resilience-maps").strip()
        # The 1 GiB address-space limit is set in the viewer itself.
        machine.succeed(f"grep -E '^Max address space +1073741824 ' /proc/{pid}/limits")
        machine.wait_for_text("World overview", timeout=120)
        machine.screenshot("gui-map")
        machine.send_key("plus")
        machine.send_key("plus")
        time.sleep(2)
        machine.screenshot("gui-map-zoomed")
        machine.send_key("q")
        machine.wait_until_fails("pgrep -u alice -x resilience-maps", timeout=30)
        machine.wait_until_succeeds("pgrep -u alice -x resilience-gui", timeout=10)
        machine.wait_for_text("Make or update", timeout=90)
        machine.send_key("esc")
        machine.send_key("esc")

    with subtest("Ctrl+Alt+F2 reaches a text login (recovery)"):
        machine.send_key("ctrl-alt-f2")
        machine.wait_until_succeeds("test \"$(fgconsole)\" = 2", timeout=30)
        machine.send_key("ctrl-alt-f1")
        machine.wait_until_succeeds("test \"$(fgconsole)\" = 1", timeout=30)

    with subtest("three GUI failures fall back to the TUI on tty1"):
        for _ in range(3):
            machine.wait_until_succeeds("pgrep -u alice -x resilience-gui", timeout=60)
            machine.succeed("pkill -9 -u alice -x resilience-gui")
            machine.wait_until_fails("pgrep -u alice -x resilience-gui", timeout=30)
        machine.wait_until_succeeds("pgrep -u alice -x resilience", timeout=60)
        machine.fail(f"pgrep -u alice {CAGE}")
  '';
}
