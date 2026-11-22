# resilience-messages: the per-user local messaging program (ADR 0018 §5),
# run with the Python environment that holds Reticulum and LXMF. It exists
# only on systems with `resilience.comms.enable`.
{
  lib,
  runCommand,
  reticulum,
}:

runCommand "resilience-messages-0.7.1"
  {
    meta = {
      description = "Per-user local messaging program of Resilience OS (needs Reticulum and LXMF)";
      license = import ../../lib/license.nix;
      mainProgram = "resilience-messages";
    };
    passthru = { inherit reticulum; };
  }
  ''
    mkdir -p $out/bin $out/libexec
    cp ${./resilience_messages.py} $out/libexec/resilience_messages.py
    ${reticulum}/bin/python -m py_compile $out/libexec/resilience_messages.py
    rm -rf $out/libexec/__pycache__
    cat > $out/bin/resilience-messages <<EOF
    #!/bin/sh
    exec ${reticulum}/bin/python -B -I $out/libexec/resilience_messages.py "\$@"
    EOF
    chmod +x $out/bin/resilience-messages
  ''
