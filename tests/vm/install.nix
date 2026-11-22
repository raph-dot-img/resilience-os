# Installing Resilience on a disk (ADR 0022), through the real TUI.
#
# Two variants (flake checks vm-install and vm-install-uefi):
# - firmware = "bios": SeaBIOS, encryption on (installed-standard-bios-luks);
# - firmware = "uefi": OVMF, encryption off (installed-standard-uefi-plain),
#   booted through systemd-boot's fallback path EFI/BOOT/BOOTX64.EFI (the
#   installer never writes firmware variables; the target starts with an
#   empty variable store).
#
# `installer` is a live system (autologin, no administrator, the installer
# enabled). Its first disk (vda, 20 GiB, empty) is the target; it runs from
# vdb. The person chooses vda, the French keyboard, encryption (on for
# BIOS, off for UEFI), the two proposed accounts, confirms by typing "vda",
# then types the passphrase and passwords into cryptsetup and passwd.
# `target` then boots that disk (BIOS: then `target_uefi` boots it with OVMF). BIOS: through GRUB; the passphrase is
# typed on the keyboard at the initrd's prompt, which only works if the
# initrd loaded the same French layout as the installer did (q/a, z/w and
# m sit on other keys).
#
# The installed system is the real `installed-standard-<variant>`
# configuration plus the NixOS test instrumentation only (the test
# driver's root shell); nothing else changes.
{
  self,
  firmware ? "bios",
}:
{ lib, ... }:
let
  uefi = firmware == "uefi";
  encrypt = !uefi;
  variant = if uefi then "uefi-plain" else "bios-luks";
  installed = self.nixosConfigurations."installed-standard-${variant}".extendModules {
    modules = [
      (
        { modulesPath, ... }:
        {
          imports = [
            "${modulesPath}/testing/test-instrumentation.nix"
            "${modulesPath}/profiles/qemu-guest.nix"
          ];
        }
      )
    ];
  };
  toplevel = installed.config.system.build.toplevel;
  # Same drive for both machines.
  disk = {
    virtualisation.diskImage = "./target.qcow2";
    virtualisation.memorySize = 2048;
    virtualisation.cores = 2;
  };
  target = efi: {
    imports = [ disk ];
    virtualisation.useBootLoader = true;
    virtualisation.useEFIBoot = efi;
    # An empty variable store from the OVMF template (not one from a disk
    # image of this VM: the disk is the one the installer wrote).
    virtualisation.efi.keepVariables = false;
    virtualisation.useDefaultFilesystems = false;
    virtualisation.fileSystems."/" = {
      device = "/dev/disk/by-label/unused-the-installed-system-mounts-its-own";
      fsType = "ext4";
    };
    virtualisation.qemu.networkingOptions = lib.mkForce [ "-nic none" ];
  };
in
{
  name = "resilience-install-${firmware}";

  nodes.installer =
    { lib, ... }:
    {
      imports = [
        disk
        self.nixosModules.default
        ../../nix/systems/common.nix
        ../../nix/profiles/standard.nix
      ];
      # vda: the 20 GiB target; the installer itself runs from vdb.
      virtualisation.diskSize = 20 * 1024;
      virtualisation.emptyDiskImages = [ 4096 ];
      virtualisation.rootDevice = "/dev/vdb";
      # UEFI: OVMF starts the kernel directly (no boot loader on the
      # installer), so /sys/firmware/efi exists as on a UEFI computer.
      virtualisation.useEFIBoot = uefi;
      virtualisation.fileSystems."/".autoFormat = true;
      virtualisation.qemu.networkingOptions = lib.mkForce [ "-nic none" ];
      # As on the live ISO (nix/systems/live-iso.nix).
      resilience.users.resilience.description = "Resilience live session";
      users.users.resilience.hashedPassword = "!";
      resilience.interface.autologinUser = "resilience";
      resilience.installer = {
        enable = true;
        systems = lib.genAttrs [ "uefi_plain" "uefi_luks" "bios_plain" "bios_luks" ] (_: toplevel);
      };
      nix.settings.substituters = lib.mkForce [ ];
    };

  nodes.target = target uefi;
  # BIOS variant: the same disk started again by OVMF (hybrid install, 0.7.1).
  nodes.target_uefi = target true;

  testScript = ''
    import base64, re, time as _time

    UEFI = ${if uefi then "True" else "False"}
    ENCRYPT = ${if encrypt then "True" else "False"}

    # Typed with US key codes: with the French layout loaded they are other
    # characters, the same ones every time the same layout is loaded.
    PASSPHRASE = "quiz maze wave 42"
    PW_USER = "amazing quokka"
    PW_ADMIN = "zebra mqwa admin"

    def tty_text(m):
        cols = int(m.wait_until_succeeds("stty -F /dev/tty1 size", timeout=10).split()[1])
        raw = base64.b64decode(m.succeed("base64 -w0 /dev/vcsu1"))
        text = raw.decode("utf-32-le", errors="replace")
        return "\n".join(text[i:i + cols] for i in range(0, len(text), cols))

    def wait_tty(m, regexp, timeout=60):
        deadline = _time.monotonic() + timeout
        while True:
            text = tty_text(m)
            if re.search(regexp, text):
                return text
            if _time.monotonic() > deadline:
                print(text)
                raise Exception(f"timed out waiting for {regexp!r} on tty1")
            _time.sleep(0.5)

    def last_line(m):
        lines = [l.rstrip() for l in tty_text(m).split("\n") if l.strip()]
        return lines[-1] if lines else ""

    def answer(m, prompt, secret, timeout=120):
        """Wait until the cursor line is `prompt`, then type `secret`."""
        deadline = _time.monotonic() + timeout
        while not re.search(prompt, last_line(m)):
            if _time.monotonic() > deadline:
                print(tty_text(m))
                raise Exception(f"timed out waiting for the prompt {prompt!r}")
            _time.sleep(0.5)
        m.send_chars(secret + "\n")
        # The next prompt replaces this one as the last line.
        m.wait_until_succeeds("true")
        _time.sleep(1)

    with subtest("the live system offers the installer; the disks are seen"):
        installer.start()
        installer.wait_for_unit("multi-user.target")
        wait_tty(installer, "MAIN MENU", timeout=120)
        installer.send_chars("/install res")
        wait_tty(installer, "Install Resilience on this computer")
        installer.send_key("ret")
        wait_tty(installer, "INSTALL RESILIENCE ON THIS COMPUTER")
        installer.send_key("ret")
        if UEFI:
            installer.succeed("test -d /sys/firmware/efi")
        else:
            installer.fail("test -e /sys/firmware/efi")
        text = wait_tty(installer, "CHOOSE THE DISK")
        assert re.search(r"/dev/vdb .*in use", text), text
        installer.send_key("ret")
        wait_tty(installer, r"Disk +/dev/vda")

    with subtest("French keyboard, encryption, the two proposed accounts"):
        installer.send_key("down")
        if not ENCRYPT:
            installer.send_key("ret")
            wait_tty(installer, r"Encryption +off")
        installer.send_key("down")
        installer.send_key("ret")
        wait_tty(installer, "KEYBOARD OF THE INSTALLED SYSTEM")
        installer.send_key("down")
        installer.send_key("down")
        installer.send_key("ret")
        wait_tty(installer, r"Keyboard +French \(AZERTY\)")
        wait_tty(installer, r"Encryption +ON" if ENCRYPT else r"Encryption +off")
        wait_tty(installer, "UEFI" if UEFI else "BIOS")

    with subtest("nothing happens until the disk's name is typed"):
        HEAD = "head -c 4M /dev/vda | sha256sum"
        before = installer.succeed(HEAD)
        installer.send_key("end")
        installer.send_key("ret")
        text = wait_tty(installer, "ERASE THIS DISK AND INSTALL")
        assert "type the disk's name (vda)" in text, text
        installer.send_chars("vdb\n")
        wait_tty(installer, "Type vda exactly")
        assert installer.succeed(HEAD) == before, "the disk changed"
        for _ in range(3):
            installer.send_key("backspace")
        installer.send_chars("vda\n")
        wait_tty(installer, "INSTALLING RESILIENCE on /dev/vda")

    with subtest("passphrase and passwords are typed into the tools"):
        if ENCRYPT:
            answer(installer, r"Enter passphrase for /dev/vda3: *$", PASSPHRASE)
            answer(installer, r"Verify passphrase: *$", PASSPHRASE)
            wait_tty(installer, "to open the disk")
            answer(installer, r"Enter passphrase for /dev/vda3: *$", PASSPHRASE)
        wait_tty(installer, "Choose the password of 'user'", timeout=1800)
        answer(installer, r"New password: *$", PW_USER)
        answer(installer, r"Retype new password: *$", PW_USER)
        wait_tty(installer, "Choose the password of 'administrator'")
        answer(installer, r"New password: *$", PW_ADMIN)
        answer(installer, r"Retype new password: *$", PW_ADMIN)
        wait_tty(installer, "Resilience is installed on /dev/vda", timeout=600)
        # Nothing left mounted or open.
        installer.fail("findmnt /run/resilience-install/target")
        installer.fail("test -e /dev/mapper/resilience-install-root")
        installer.shutdown()

    with subtest("the installed system starts (BIOS: passphrase in the French layout)"):
        # The same computer started again: same state directory, same disk.
        target.state_dir = installer.state_dir
        target.start()
        if ENCRYPT:
            # GRUB waits 5 s, the prompt follows a few seconds later (it is
            # on the serial log too). Keys must be typed after the initrd
            # has loaded the layout: they are translated when pressed.
            # Machine.sleep() would wait for the root shell, which exists
            # only once the disk is unlocked: wait in the driver instead.
            _time.sleep(40)
            target.send_chars(PASSPHRASE + "\n")
        target.wait_for_unit("multi-user.target", timeout=600)
        if ENCRYPT:
            target.succeed("findmnt -no SOURCE / | grep -x /dev/mapper/root")
        else:
            target.succeed("findmnt -no SOURCE / | grep -x /dev/vda3")
            target.fail("test -e /dev/mapper/root")
        if UEFI:
            # Started by OVMF from the fallback path, with systemd-boot.
            target.succeed("test -d /sys/firmware/efi")
            target.succeed("findmnt -no SOURCE /boot | grep -x /dev/vda2")
            target.succeed("test -s /boot/EFI/BOOT/BOOTX64.EFI")
            target.succeed("ls /boot/loader/entries/ | grep -q nixos")
        else:
            target.fail("test -e /sys/firmware/efi")
        target.succeed("grep -x KEYMAP=fr /var/lib/resilience/keyboard")
        # The login prompts use the French layout (keycode 16 is a, not q).
        target.wait_for_unit("getty@tty1.service")
        target.wait_until_succeeds("dumpkeys -k 2>/dev/null | grep -E '^keycode +16 = +\\+?a\\b'", timeout=30)

    if not UEFI:
        with subtest("the BIOS-installed disk also starts in UEFI mode (hybrid GRUB)"):
            target.succeed("test -s /boot/EFI/BOOT/BOOTX64.EFI")
            target.succeed("test -d /boot/grub/i386-pc")
            target.shutdown()
            target_uefi.state_dir = installer.state_dir
            target_uefi.start()
            _time.sleep(40)
            target_uefi.send_chars(PASSPHRASE + "\n")
            target_uefi.wait_for_unit("multi-user.target", timeout=600)
            target_uefi.succeed("test -d /sys/firmware/efi")
            target_uefi.succeed("findmnt -no SOURCE / | grep -x /dev/mapper/root")
            target_uefi.shutdown()
            target.start()
            _time.sleep(40)
            target.send_chars(PASSPHRASE + "\n")
            target.wait_for_unit("multi-user.target", timeout=600)

    with subtest("local accounts with their modes; no autologin; sandbox on"):
        target.succeed("getent passwd user | grep -F ':/run/current-system/sw/bin/resilience-session'")
        target.succeed("id -nG administrator | grep -w wheel")
        target.succeed("id -nG user | grep -w resilience-stable")
        target.fail("id -nG user | grep -w wheel")
        out = target.succeed("resilience-admin list")
        print(out)
        assert re.search(r"^user +ultra-stable +stable", out, re.M), out
        assert re.search(r"^administrator +ultra-stable +advanced", out, re.M), out
        target.succeed("test \"$(sysctl -n user.max_user_namespaces)\" -gt 0")
        target.fail("pgrep -x resilience")

    with subtest("log in on tty1 with the password typed during the installation"):
        wait_tty(target, "login:", timeout=60)
        target.send_chars("user\n")
        wait_tty(target, "Password:")
        target.send_chars(PW_USER + "\n")
        target.wait_until_succeeds("pgrep -u user -x resilience", timeout=60)
        wait_tty(target, "MODE: STABLE")

    with subtest("first login: the licence notice (ADR 0024), then the menu"):
        wait_tty(target, "HIPPOCRATIC LICENSE 2.1: SUMMARY")
        target.send_key("tab")
        wait_tty(target, "ORIGINAL TEXT")
        target.send_chars(" ")
        wait_tty(target, r"\[x\] I certify")
        target.send_key("ret")
        wait_tty(target, "MAIN MENU")
        target.succeed("grep -q '^time=.* user=user ' /var/lib/resilience/licence-acceptances.log")
  '';
}
