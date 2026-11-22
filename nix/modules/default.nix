# All Resilience OS NixOS modules. Import this (or `self.nixosModules.default`)
# into a NixOS configuration; then choose an edition profile from nix/profiles.
{
  imports = [
    ./core.nix
    ./admin.nix
    ./media.nix
    ./crypto.nix
    ./packs.nix
    ./users.nix
    ./interface.nix
    ./gui.nix
    ./security.nix
    ./networking.nix
    ./comms.nix
    ./mesh.nix
    ./share.nix
    ./hotspot.nix
    ./installer.nix
    ./updates.nix
    ./licence.nix
    ./boot.nix
    ./power.nix
    ./applications
  ];
}
