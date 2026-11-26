# An installed Resilience system (ADR 0022). Prebuilt and the same for every
# machine: everything that differs between machines (accounts, keyboard
# layout) is state written by the installer, and the disks are found by
# their labels. The flake builds one per edition, firmware and encryption:
#   installed-<edition>-<uefi|bios>-<plain|luks>
{ boot, encrypted }:
{
  config,
  lib,
  modulesPath,
  ...
}:

assert lib.elem boot [
  "uefi"
  "bios"
];

{
  imports = [
    # Kernel modules for common hardware, as on the live ISO.
    "${modulesPath}/profiles/all-hardware.nix"
  ];

  networking.hostName = "resilience";

  # Disk layout made by resilience-install (ADR 0022 §3).
  fileSystems."/" = {
    device = "/dev/disk/by-label/resilience-root";
    fsType = "ext4";
  };
  fileSystems."/boot" = {
    device = "/dev/disk/by-label/RESBOOT";
    fsType = "vfat";
    options = [
      "fmask=0077"
      "dmask=0077"
    ];
  };
  boot.initrd.luks.devices = lib.mkIf encrypted {
    root.device = "/dev/disk/by-partlabel/resilience-root";
  };

  boot.loader = {
    systemd-boot = lib.mkIf (boot == "uefi") {
      enable = true;
      # Kernels are kept on the 1 GiB boot partition: bound the number.
      configurationLimit = 10;
    };
    # Never write the firmware's boot entries; systemd-boot is also the
    # removable fallback EFI/BOOT/BOOTX64.EFI, which firmware finds itself.
    efi.canTouchEfiVariables = false;
    efi.efiSysMountPoint = "/boot";
    grub = lib.mkIf (boot == "bios") {
      enable = true;
      # resilience-install runs grub-install (i386-pc) on the disk once;
      # later generations only rewrite grub.cfg.
      devices = [ "nodev" ];
      configurationLimit = 10;
      # Hybrid (0.7.1): GRUB for EFI too, as the removable fallback
      # EFI/BOOT/BOOTX64.EFI on the boot partition, so the disk also starts
      # when the firmware boots it in UEFI mode (a stick started in legacy
      # mode got a BIOS-only disk: ThinkPad Yoga 11e, 2026-11-22). Written by
      # nixos-install and again on an update; never touches NVRAM.
      efiSupport = true;
      efiInstallAsRemovable = true;
    };
  };

  # Accounts are created by the installer (useradd), their modes kept in
  # /var/lib/resilience; the keyboard layout chosen when installing.
  users.mutableUsers = true;
  resilience.localAccounts = true;
  resilience.keyboard.stateFile = "/var/lib/resilience/keyboard";

  # Offline updates (ADR 0023): a bundle's manifest names the new system of
  # each variant; the release key's public part is built in.
  resilience.updates = {
    variant = "installed-${config.resilience.edition}-${boot}-${if encrypted then "luks" else "plain"}";
    trustedKeys = import ../release-keys.nix;
  };

  hardware.enableRedistributableFirmware = true;
  hardware.cpu.intel.updateMicrocode = true;
  hardware.cpu.amd.updateMicrocode = true;
}
