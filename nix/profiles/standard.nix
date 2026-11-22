# Resilience Standard: the recommended everyday installation.
{ lib, ... }:
{
  resilience.edition = lib.mkDefault "standard";
  # Local messaging between nearby machines (ADR 0018), on by default in
  # this edition (owner's decision, 2026-11-16). It brings Reticulum and
  # LXMF, which are under the Reticulum License (not free software), and
  # the machine then announces itself on its local network. Turn it off
  # with `resilience.comms.enable = false;`. The minimal edition has none
  # of it.
  resilience.comms.enable = lib.mkDefault true;
  # Sharing offline resources with nearby machines (ADR 0025): offered,
  # but nothing is shared until an administrator turns it on.
  resilience.share.enable = lib.mkDefault true;
}
