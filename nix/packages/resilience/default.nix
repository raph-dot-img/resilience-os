# The Resilience userland: the `resilience` TUI and its helpers
# `resilience-admin` (privileged, ADR 0007), `resilience-backup` (ADR 0008),
# `resilience-library`, `resilience-crypto` and `resilience-maps` (ADR 0016).
# Unit and TUI tests (`cargo test`) run in the check phase of this build.
#
# resilience-maps bundles SQLite (C, compiled by libsqlite3-sys with stdenv's
# cc). Its compile flags (no extension loading, no mmap) come from
# .cargo/config.toml, which is part of the source below; the test
# `sqlite_is_built_without_extension_loading` fails if they are lost.
{
  lib,
  rustPlatform,
  age,
  zstd,
}:

let
  root = ../../..;
  cargoToml = lib.importTOML (root + "/Cargo.toml");
in
rustPlatform.buildRustPackage {
  pname = "resilience";
  inherit (cargoToml.workspace.package) version;

  # Only the Rust sources: documentation or Nix edits do not trigger a rebuild.
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
  # The graphical interface is its own package (nix/packages/resilience-gui).
  cargoBuildFlags = [
    "--workspace"
    "--exclude"
    "resilience-gui"
  ];
  cargoTestFlags = [
    "--workspace"
    "--exclude"
    "resilience-gui"
  ];

  # The backup round-trip test runs the real tools (tar, sha256sum: stdenv).
  nativeCheckInputs = [
    age
    zstd
  ];

  # The map pack builder is for build machines only (ADR 0016 §2): it is
  # built and tested here with the rest of the workspace, but not
  # installed. nix/maps builds it on its own for the pack derivations.
  postInstall = ''
    rm "$out/bin/resilience-mapmaker"
  '';

  meta = {
    description = "Terminal interface of Resilience OS";
    license = import ../../lib/license.nix;
    mainProgram = "resilience";
    platforms = lib.platforms.linux;
  };
}
