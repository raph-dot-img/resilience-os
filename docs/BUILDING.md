# Building Resilience OS

No root access is needed for anything in this document.

## Requirements

- x86_64 Linux with **Nix ≥ 2.18** and flakes enabled
  (`experimental-features = nix-command flakes`).
- For VM tests: `/dev/kvm` and the Nix daemon's `kvm` + `nixos-test` system
  features (`nix config show system-features`).
- Disk: ~10 GiB free in `/nix/store` for an ISO build.
- RAM: 4 GiB minimum; the reference host has 12 GiB.
- Optional for fast iteration: a Rust toolchain (≥ 1.85) via rustup, or
  `nix develop`.

## Rust only

```sh
cargo build
cargo test
cargo clippy --all-targets
cargo run -p resilience-tui                 # runs the TUI on your terminal
RESILIENCE_SYSTEM_CONFIG=/dev/null cargo run -p resilience-tui -- status
```
On a non-Resilience host the TUI warns that `/etc/resilience/system.toml` is
missing and uses built-in defaults; the catalog will be empty.

## Nix outputs

| Output | Command | Result |
|---|---|---|
| TUI package (runs `cargo test`) | `nix build .#resilience` | `result/bin/resilience` |
| Live ISO, Standard | `nix build .#iso` | `result/iso/resilience-standard-0.4.0-x86_64-linux.iso` |
| Live ISO, Minimal | `nix build .#iso-minimal` | `result/iso/resilience-minimal-…` |
| Dev VM | `nix run .#vm` | boots on the serial console in your terminal |
| All checks | `nix flake check -L --max-jobs 1` | see docs/TESTING.md |
| Format Nix | `nix fmt` | |

### The development VM

`nix run .#vm` starts a headless VM (2 vCPU, 2 GiB) sharing the host store
(no image to build; a small `resilience-dev.qcow2` is created in the current
directory for /). It has three **test-only** accounts: `demo` (Ultra-Stable,
autologin on tty1 — not visible in headless mode), `stable` / password
`stable`, and `admin` / password `admin`. The serial console in your terminal
shows a login prompt; logging in as `stable` or `admin` starts the TUI over
the serial line. Quit QEMU with `Ctrl-A x`. These passwords are public: never
reuse `nix/systems/dev-vm.nix` for a real machine.

Verified: the VM boots to the serial login prompt (2026-11-14). Not yet
verified: driving the TUI over the serial console.

### Booting the ISO

```sh
scripts/boot-iso.sh --gui result/iso/*.iso            # window, no network
scripts/boot-iso.sh --gui --uefi result/iso/*.iso     # UEFI (OVMF)
scripts/boot-iso.sh --gui --net user result/iso/*.iso # with NAT networking
```

### Writing the ISO to a USB stick (on another machine, as root)

This is destructive for the target device and requires root; the
development tooling never does it for you. Double-check the device name:

```sh
lsblk                                   # identify the USB stick, e.g. /dev/sdX
sudo dd if=resilience-standard-0.4.0-x86_64-linux.iso of=/dev/sdX bs=4M status=progress conv=fsync
```

## Resource notes (reference host: 6 cores, 12 GiB)

- `nix.conf` uses `max-jobs = 2`, `cores = 2`; keep it that way.
- The ISO build is dominated by squashfs compression (`zstd -Xcompression-level 9`,
  chosen over the default 19 for build time). With a warm store it takes
  under a minute.
- Never run an ISO build and a VM test at the same time; never two VM tests.
- The closure is 2.2 GiB, mostly firmware (825 MiB). The nixpkgs source
  (197 MiB) is included deliberately so the system can rebuild its own
  configuration offline.

## Updating nixpkgs

```sh
git switch -c update/nixpkgs
nix flake update
nix flake check -L --max-jobs 1
nix build .#iso && python3 -I tests/iso/smoke.py result/iso/*.iso /tmp/shots
```
Record the new revision in CHANGELOG.md. Moving to a new NixOS release
(e.g. 26.11) requires an ADR update and a full test pass.

## Publishing a release on the mirrors

`scripts/make-release.sh [--minimal] [--gui] [--update-key NIX_KEY] [--sign MINISIGN_KEY] VERSION OUTDIR`
(at tag `vVERSION`, clean tree) fills `OUTDIR/releases/VERSION/` with the
ISOs, the offline update bundles (`.tar`, extracted to a stick's root), the
source archive and `SHA256SUMS`, signed with the minisign release key
(public key in `keys/`), plus `LATEST`, `README.txt` and the public key at
the top. Upload `releases/VERSION/` first and `LATEST` last. For folders
filled by hand (older releases), `--resign DIR` rewrites and signs the sums.
`--index VERSION OUTDIR` writes only `README.txt`, `LATEST` and the public
key, for a tree rebuilt from existing releases.

Publishing (since 2026-11-20): the mirror is served from the development
machine itself (`~/Mirror`, nginx in Docker behind an FRP tunnel; files in
`~/Mirror/files/resilience/`). Copy the release into the hidden
`.staging/VERSION/` (nginx does not serve dotfiles), run `sha256sum -c
SHA256SUMS` and `minisign -Vm SHA256SUMS -p resilience-release.minisign.pub`
there, `chmod 644` every file and `755` the folder, then one `mv -n` into
`releases/`; the top files (`LATEST` last) afterwards. Published files are
never changed. Check from outside that **every file answers HTTP 200**
(a file left 0600 gets 403 from nginx).

Then the torrent (since 2026-11-21): `~/Torrents/make-torrent.sh VERSION
[EDITION]` makes `releases/torrents/resilience_EDITION_VERSION.torrent` from the
published folder (qBittorrent's creator: hybrid v1+v2, the mirror as web
seed, the trackers of `~/Torrents/trackers.txt`), starts seeding it and
updates `releases/torrents/SHA256SUMS`. The torrent's name is the folder's
name (a multi-file web seed is fetched at `<url>/<name>/<file>`). The trust
chain is unchanged: the signed `SHA256SUMS` is inside the torrent.

The seed (`~/Torrents`, Docker): qBittorrent-nox shares the network of
gluetun, which holds a ProtonVPN WireGuard tunnel (P2P servers in Germany,
NAT-PMP port forwarding pushed into qBittorrent; gluetun's firewall drops
everything outside the tunnel). The private key is in `~/Torrents/.env`
(0600), the Web UI password in `~/Torrents/webui-password` (0600); the Web
UI listens on `127.0.0.1:8080` only. The release folders are mounted
read-only. See `~/Torrents/README.md`.

Until 0.4.0 the upload went to the owner's server through Tor (rsync over
SSH with an `rrsync`-restricted key, resumed transfers into `.staging/`);
that path is retired.

## Debian host: KVM for the Nix build users

NixOS VM tests run QEMU inside the Nix build sandbox as the `nixbld*`
users. NixOS makes `/dev/kvm` mode 0666; Debian makes it `root:kvm 0660`,
so in the sandbox QEMU prints `failed to initialize kvm: Permission
denied`, falls back to emulating the CPU, and the tests time out (first
full run on the Debian host, 2026-11-21). The owner, as root, once:

```sh
echo 'KERNEL=="kvm", GROUP="kvm", MODE="0666", OPTIONS+="static_node=kvm"' \
  | sudo tee /etc/udev/rules.d/99-kvm-nix.rules
sudo udevadm control --reload-rules
sudo udevadm trigger --name-match=kvm
ls -l /dev/kvm     # crw-rw-rw- ... /dev/kvm
```

Check without a full run: `nix build --rebuild .#checks.x86_64-linux.vm-licence -L 2>&1 | grep -c 'failed to initialize kvm'` must print 0.
