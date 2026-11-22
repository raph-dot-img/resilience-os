# Login shell for Resilience accounts.
#
# Starts the TUI, restarts it after a crash (with a back-off so a broken
# interface cannot spin), and refuses `-c COMMAND` invocations so the account
# cannot be used as a generic command runner (su -c, ssh host cmd, scp).
# This is a usability measure; the security boundary is the account's lack of
# privileges (see docs/THREAT_MODEL.md).
{
  lib,
  writeShellApplication,
  coreutils,
  bash,
  shadow,
  resilience,
}:

(writeShellApplication {
  name = "resilience-session";
  runtimeInputs = [
    coreutils
    resilience
  ];
  # `set -e` would end the session on the first TUI crash.
  bashOptions = [
    "nounset"
    "pipefail"
  ];
  text = ''
    if [ "$#" -gt 0 ]; then
      echo "resilience-session: this account uses the Resilience interface;" >&2
      echo "running commands through the login shell is not supported." >&2
      exit 1
    fi
    if ! [ -t 0 ] || ! [ -t 1 ]; then
      echo "resilience-session: an interactive terminal is required." >&2
      exit 1
    fi

    # Programs that spawn "$SHELL" (editor shell escapes, file managers) get a
    # real shell only on accounts that are allowed one.
    if id -nG | tr ' ' '\n' | grep -qxE 'wheel|resilience-stable'; then
      export SHELL=${lib.getExe bash}
    else
      export SHELL=${shadow}/bin/nologin
    fi
    cd "$HOME" 2>/dev/null || cd /

    # Graphical edition (ADR 0026): the GUI on tty1, the TUI elsewhere and
    # after three quick GUI failures in a row.
    if [ -x /etc/resilience/gui-session ] && [ "$(tty 2>/dev/null)" = /dev/tty1 ]; then
      gui_failures=0
      while [ "$gui_failures" -lt 3 ]; do
        started=$(date +%s)
        rc=0
        /etc/resilience/gui-session || rc=$?
        if [ "$rc" -eq 0 ]; then
          exit 0
        fi
        if [ $(( $(date +%s) - started )) -lt 60 ]; then
          gui_failures=$((gui_failures + 1))
        else
          gui_failures=1
        fi
        printf '\033[0m\033[2J\033[H'
        echo "The graphical interface stopped (exit code $rc). Restarting it..."
        sleep 2
      done
      echo "The graphical interface failed 3 times: using the text interface."
      echo "Details: ~/.local/state/resilience/gui.log."
      sleep 3
    fi

    rapid=0
    while true; do
      started=$(date +%s)
      rc=0
      resilience || rc=$?
      if [ "$rc" -eq 0 ]; then
        exit 0
      fi
      if [ $(( $(date +%s) - started )) -lt 10 ]; then
        rapid=$((rapid + 1))
      else
        rapid=1
      fi
      # A crashed TUI leaves the terminal in raw mode / alternate screen.
      stty sane 2>/dev/null || true
      printf '\033[?1049l\033[?25h\033[0m\033[2J\033[H'
      echo "=================================================================="
      echo " RESILIENCE INTERFACE STOPPED UNEXPECTEDLY (exit code $rc)"
      echo "=================================================================="
      echo
      echo " Your files are safe. The interface will be restarted."
      echo
      echo " Log file:        ''${XDG_STATE_HOME:-$HOME/.local/state}/resilience/tui.log"
      echo " Other consoles:  Alt+F2 ... Alt+F6"
      echo " If this repeats: restart and choose an earlier generation in the"
      echo "                  boot menu (see the handbook, 'Recovery')."
      echo
      if [ "$rapid" -ge 3 ]; then
        echo " Repeated failures: waiting 30 seconds before the next attempt."
        sleep 30
      fi
      read -r -t 300 -p " Press Enter to restart the interface... " _ || true
    done
  '';
}).overrideAttrs
  (old: {
    passthru = (old.passthru or { }) // {
      shellPath = "/bin/resilience-session";
    };
  })
