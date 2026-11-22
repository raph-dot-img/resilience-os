# The graphical interface (ADR 0026): `resilience-gui` (iced, software
# rendering) built alone from the workspace. libwayland-client and
# libxkbcommon are loaded at run time (dlopen): their paths go into the
# binary's RUNPATH. Its unit tests run in the check phase.
{
  lib,
  rustPlatform,
  patchelf,
  wayland,
  libxkbcommon,
}:

let
  root = ../../..;
  cargoToml = lib.importTOML (root + "/Cargo.toml");
in
rustPlatform.buildRustPackage {
  pname = "resilience-gui";
  inherit (cargoToml.workspace.package) version;

  src = lib.fileset.toSource {
    inherit root;
    fileset = lib.fileset.unions [
      (root + "/Cargo.toml")
      (root + "/Cargo.lock")
      (root + "/.cargo/config.toml")
      (root + "/crates")
    ];
  };

  cargoLock.lockFile = root + "/Cargo.lock";
  cargoBuildFlags = [
    "-p"
    "resilience-gui"
  ];
  cargoTestFlags = [
    "-p"
    "resilience-gui"
  ];

  nativeBuildInputs = [ patchelf ];
  postFixup = ''
    patchelf --add-rpath ${
      lib.makeLibraryPath [
        wayland
        libxkbcommon
      ]
    } "$out/bin/resilience-gui"
  '';

  meta = {
    description = "Graphical interface of Resilience OS";
    license = import ../../lib/license.nix;
    mainProgram = "resilience-gui";
    platforms = lib.platforms.linux;
  };
}
