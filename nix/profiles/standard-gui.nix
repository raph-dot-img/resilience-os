# Resilience Standard with the graphical interface (ADR 0026): everything
# in Standard, and tty1 starts the GUI (cage + resilience-gui) instead of
# the TUI. The other consoles keep their text logins and the TUI.
{
  imports = [ ./standard.nix ];
  resilience.edition = "standard-gui";
  resilience.gui.enable = true;
}
