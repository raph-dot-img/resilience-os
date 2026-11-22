# Component inventory

Third-party components, as present in the pinned nixpkgs (`nixos-26.05`,
rev `f1e5c356`, 2026-11-13). Versions and licenses were read from nixpkgs
metadata (`nix eval`), not assumed. "TUI" = usable without any graphical
environment. Maintenance status reflects nixpkgs packaging and upstream
activity as known on 2026-11-14; re-check before each release.

## Integrated (in the system today)

| Component | Version | License | Upstream | Offline | Integration |
|---|---|---|---|---|---|
| NixOS / nixpkgs | 26.05 | MIT (expressions) | https://github.com/NixOS/nixpkgs | yes | base system |
| Linux kernel | 6.18.63 | GPL-2.0 | https://kernel.org | yes | default kernel of 26.05 |
| systemd | 260.5 | LGPL-2.1+ | https://systemd.io | yes | init, logind, journald, timedated (`timedatectl set-time` from the helper) |
| NetworkManager (nmtui) | 1.56.0 | GPL-2.0+ | https://networkmanager.dev | yes (LAN) | connectivity check disabled; app `nmtui` (Stable+); `nmcli` used by the TUI's Wi-Fi screen (every mode, polkit allowlist without the group, ADR 0010) |
| sudo | (nixpkgs) | ISC | https://www.sudo.ws | yes | wheel only, password |
| polkit (pkexec, pkttyagent) | 127 | LGPL-2.0+ | https://github.com/polkit-org/polkit | yes | authorises `resilience-admin set-mode` and `set-time` (auth_admin); text agent on the console |
| Linux-PAM (pam_faillock, faillock) | 1.7.2 | BSD-3-Clause | https://github.com/linux-pam/linux-pam | yes | rate limiting on login, su, sudo, polkit-1 |
| udisks2 (udisksctl) | 2.11.2 | GPL-2.0+ / LGPL-2.0+ | https://github.com/storaged-project/udisks | yes | mounts removable media for the console user (D-Bus activated, its polkit defaults) |
| kiwix-tools (kiwix-serve) | 3.8.2 | GPL-3.0-or-later | https://github.com/kiwix/kiwix-tools | yes | serves one installed ZIM file on 127.0.0.1 while it is read (ADR 0013) |
| libkiwix, libzim, Xapian | 14.2.1, 9.7.0, 1.4.31 | GPL-3.0+, GPL-2.0+, GPL-2.0+ | https://kiwix.org | yes | dependencies of kiwix-serve (ZIM reading, full-text search) |
| Lynx | 2.9.2 | GPL-2.0-or-later | https://lynx.invisible-island.net | yes | text browser of the offline library, `-restrictions=… -localhost` |
| nginx | 1.30.6 | BSD-2-Clause | https://nginx.org | yes | community network (ADR 0025): read-only web server on one interface while an administrator shares; DynamicUser, GET/HEAD only, kiwix-serve behind it on 127.0.0.1:81 |
| dnsmasq (through NetworkManager) | 2.93 | GPL-2.0-or-later | https://thekelleys.org.uk/dnsmasq/doc.html | yes | DHCP and DNS of the hotspot (ADR 0025 §6), started by NetworkManager's shared mode; `home.arpa` only, `no-resolv` |
| poppler-utils (pdftotext) | 26.10.0 | GPL-2.0-or-later | https://poppler.freedesktop.org | yes | text of PDF files in Find & view files (ADR 0014) |
| libarchive (bsdtar) | 3.8.9 | BSD-3-Clause | https://libarchive.org | yes | lists archive contents, never extracts (ADR 0014) |
| man-db, less | 2.13.1, 692 | GPL-2.0-or-later, GPL-3.0-or-later | https://man-db.gitlab.io, https://greenwoodsoftware.com/less | yes | Manual pages; less in secure mode (ADR 0014) |
| curl | 8.23.0 | curl | https://curl.se | downloads only | downloads resource packs on request, HTTPS only, resumable |
| Kiwix ZIM content (catalog) | 2024–2026 files | CC-BY-SA-4.0 and others (per pack) | https://download.kiwix.org | yes | not shipped: chosen by the user, hashes pinned in `nix/packs/kiwix.nix` |
| age (age, age-keygen, age-inspect) | 1.3.2 | BSD-3-Clause | https://age-encryption.org | yes | backup encryption (ADR 0008); file and message encryption, own keys, format details (ADR 0015) |
| minisign | 0.12 | ISC | https://jedisct1.github.io/minisign/ | yes | signing and verifying files (ADR 0015) |
| GnuPG | 2.4.9 | GPL-3.0-or-later | https://gnupg.org | yes | OpenPGP interoperability: verify, decrypt, encrypt, import, own key (ADR 0015); agent without passphrase cache, no key server |
| pinentry-curses | 1.3.2 | GPL-2.0-or-later | https://gnupg.org/related_software/pinentry/ | yes | GnuPG's passphrase dialog on the console |
| xkcdpass | 1.30.0 | BSD-3-Clause | https://github.com/redacted/XKCD-password-generator | yes | random passphrases from the EFF long list and a French list (ADR 0015) |
| OpenSSL (`openssl rand`) | 3.6.5 | Apache-2.0 | https://openssl.org | yes | random passwords and keys (ADR 0015) |
| coreutils checksums | (coreutils) | GPL-3.0+ | https://gnu.org/software/coreutils | yes | sha256sum, sha512sum, b2sum, sha1sum, md5sum (ADR 0015) |
| zstd | 1.5.7 | BSD-3-Clause / GPL-2.0 | https://facebook.github.io/zstd | yes | backup compression |
| GNU tar | 1.35 | GPL-3.0+ | https://www.gnu.org/software/tar | yes | backup archives |
| shadow (gpasswd) | 4.19.4 | BSD-3-Clause | https://github.com/shadow-maint/shadow | yes | group changes by `resilience-admin` |
| batctl | 2026.4 | GPL-2.0-only | https://www.open-mesh.org | yes | BATMAN-adv control (`batctl n`, `batctl o`) on systems with the mesh network (ADR 0021); the `batman_adv` module is the kernel's |
| iw | 6.17 | ISC | https://wireless.wiki.kernel.org/en/users/documentation/iw | yes | puts the Wi-Fi card in ad hoc mode for the mesh network (ADR 0021) |
| Terminus console font | (nixpkgs) | OFL-1.1 | https://terminus-font.sourceforge.net | yes | `ter-i16n` |
| GNU nano | 9.3 | GPL-3.0+ | https://nano-editor.org | yes | editor (Stable+), notes editor (`--restricted` for Ultra-Stable) |
| Vim | 9.2.1252 | Vim | https://www.vim.org | yes | Stable+ |
| Midnight Commander | 4.8.33 | GPL-3.0+ | https://midnight-commander.org | yes | file manager, Stable+ |
| GNU bc | 1.08.2 | GPL-3.0+ | https://www.gnu.org/software/bc | yes | calculator |
| GNU units | 2.27 | GPL-3.0+ | https://www.gnu.org/software/units | yes | unit converter (its currency updater needs Internet; not offered) |
| util-linux `cal` | (nixpkgs) | GPL-2.0+ | https://github.com/util-linux/util-linux | yes | calendar |
| htop | 3.5.1 | GPL-2.0-only | https://htop.dev | yes | `--readonly` for Ultra-Stable |
| ncdu | 2.9.2 | MIT | https://dev.yorhel.nl/ncdu | yes | `-rr` (read-only, no shell) for Ultra-Stable |
| nudoku | 5.0.0 | GPL-3.0-only | https://github.com/jubalh/nudoku | yes | game |
| tty-solitaire | 1.4.1 | MIT | https://github.com/mpereira/tty-solitaire | yes | game |
| ninvaders | 0.1.2 | GPL-2.0+ | https://ninvaders.sourceforge.net | yes | game |
| moon-buggy | 1.1.0 | GPL-2.0-only | https://www.seehuhn.de/pages/moon-buggy | yes | game |
| NetHack | 3.6.7 | NGPL | https://www.nethack.org | yes | game |
| cage | 0.3.0 | MIT | https://github.com/cage-kiosk/cage | yes | GUI edition only (ADR 0026): Wayland kiosk on tty1, one full-screen client, `-s` keeps VT switching; built without Xwayland |
| wlroots | 0.20.2 | MIT | https://gitlab.freedesktop.org/wlroots/wlroots | yes | GUI edition: cage's compositor library, pixman (CPU) renderer, `enableXWayland = false` |
| libinput, libxkbcommon, Wayland, pixman | 1.31.3, 1.13.1, 1.25.0, 0.46.4 | MIT | https://freedesktop.org | yes | GUI edition: input, keymaps (XKB names from `keyboard::xkb_for`), protocol, software rendering |
| foot | 1.27.0 | MIT | https://codeberg.org/dnkl/foot | yes | GUI edition: terminal window for programs started from the GUI (same argv, banner, pkttyagent as on the console) |
| Inter | 4.1 | OFL-1.1 | https://rsms.me/inter | yes | GUI edition: interface font (InterVariable.ttf only, bundled) |
| DejaVu fonts | 2.37 | Bitstream Vera / public domain | https://dejavu-fonts.github.io | yes | GUI edition: DejaVu Sans (glyph fallback), Sans Mono (text panel, foot) |
| Resilience OS logos (`assets/logo/`) | 2026-11-21 | all rights reserved, unmodified use to refer to Resilience OS (`assets/logo/LICENSE`) | made by the project's owner | yes | GUI top bar (icon, drawn as vector), README, release pages |

## Rust crates (Cargo.lock, hash-pinned)

ratatui 0.30 (MIT), crossterm 0.29 (MIT, via ratatui), serde 1 (MIT/Apache-2.0),
toml 1 (MIT/Apache-2.0), tracing 0.1 / tracing-subscriber 0.3 (MIT),
thiserror 2 (MIT/Apache-2.0), libc 0.2 (MIT/Apache-2.0). In `resilience-maps`
only (ADR 0016): **rusqlite 0.43.0** (MIT; `default-features = false`,
features `bundled`, `hooks`, `limits`; needs Rust 1.89) with
**libsqlite3-sys 0.40.0** (MIT) bundling **SQLite 3.54.1** (public domain;
compiled with `SQLITE_OMIT_LOAD_EXTENSION` and `SQLITE_MAX_MMAP_SIZE=0` from
`.cargo/config.toml`), and **miniz_oxide 0.9.1** (MIT OR Zlib OR Apache-2.0,
pure Rust) with adler2 2.0.1 (0BSD OR MIT OR Apache-2.0).
They add fallible-iterator and fallible-streaming-iterator (MIT/Apache-2.0),
reuse bitflags and smallvec (already locked) and, at build time only, add
cc, pkg-config, vcpkg, shlex and find-msvc-tools (MIT/Apache-2.0). 124
packages in Cargo.lock including the workspace's own. `cargo deny check licenses advisories`
is to be run before each release (cargo-deny is in the dev shell).

**GUI edition (ADR 0026, `resilience-gui` only):** **iced 0.14.0** (MIT;
`default-features = false`, features `tiny-skia`, `wayland`, `smol`,
`crisp`: software rendering, no wgpu/Vulkan/OpenGL, no X11) with its
winit, cosmic-text, tiny-skia, softbuffer, smithay-client-toolkit and smol
dependencies, and serde_json 1 (MIT/Apache-2.0) for run requests. With
them Cargo.lock has 344 registry packages; licences checked with `cargo
metadata` on 2026-11-21: all MIT, Apache-2.0, BSD, ISC, Zlib, BSL-1.0,
Unlicense or Unicode-3.0; the only GPL/LGPL appear as one choice of an
`OR` (r-efi, one Apache-2.0 OR GPL-2.0-only crate) and the permissive one
applies. The TUI package is built with `--exclude resilience-gui` and
contains none of them.

## Evaluated, available in nixpkgs, planned

| Component | Version | License | Purpose | Notes |
|---|---|---|---|---|
| kiwix-tools | 3.8.2 | GPL-3.0+ | ZIM serving (Phase 3) | `kiwix-serve` on 127.0.0.1 + text browser |
| w3m / lynx / links2 / elinks / chawan | 0.5.6 / 2.9.2 / 2.30 / 0.19.1 / 0.3.3 | MIT / GPL-2.0+ / GPL-2.0+ / GPL-2.0 / Unlicense | text browser for Kiwix | to compare (UTF-8, tables, link navigation) |
| cryptsetup | 2.8.6 | GPL-2.0+ | LUKS (installer) | |
| poppler-utils (pdftotext) | 26.10.0 | GPL-2.0+ | PDF text reading | |
| visidata | 3.3 | GPL-3.0 | CSV/spreadsheet | Python closure: evaluate size |
| sc-im | 0.8.5 | BSD-4-Clause | spreadsheet | advertising clause: review |
| glow / mdcat | 2.1.2 / 2.7.1 | MIT / MPL-2.0 | Markdown viewing | |
| yggdrasil | 0.5.13 | LGPL-3.0 | mesh (Phase 6) | evaluated, **not adopted** (ADR 0021): an IPv6 overlay with no program here to use it; makes no radio link |
| rns (Reticulum), lxmf (LXMF) | 1.2.9, 0.9.8 | **Reticulum License** (MIT text plus two conditions: no use in systems able to purposefully harm people, no use for AI training datasets). **Not free software**; nixpkgs marks both unfree | local messaging network (ADR 0018), with `resilience.comms.enable` (**on in the standard edition**, off in minimal): `rnsd` as a system service; LXMF for the messaging program to come | allowed by name in `nix/packages/resilience-reticulum` only; upstream markqvist/Reticulum, markqvist/LXMF; licence read on 2026-11-16 |
| toxic (Tox) | 0.16.1 | GPL-3.0-only | Internet messaging TUI (Phase 6) | |
| signal-cli | 0.14.7 | GPL-3.0 | **unofficial** Signal client (Phase 6) | Java; unofficial; may break with server changes |
| rtl-sdr / rtl_433 / soapysdr | 1.3.5 / 25.12 / 0.8.1-unstable | GPL-2.0+ / GPL-2.0+ / BSL-1.0 | receive-only SDR (Phase 7) | |
| gpsd | 3.27.5 | BSD-2-Clause | GPS for maps (Phase 5) | |
| osmium-tool, gdal (`gdalMinimal`) | 1.19.1, 3.12.4 | GPL-3.0+…, MIT | map data processing (build side); GDAL's `ogr2ogr` cuts the sea polygons to a coastal region | |
| OpenStreetMap water polygons (data) | `water-polygons-split-4326` of 2026-11-20 (909 MB zip, SHA-256 `8c02ca85…5fe39d05`) | ODbL-1.0, (c) OpenStreetMap contributors | the sea inside coastal map packs (Bretagne), cut to the region at build time (build side only) | from osmdata.openstreetmap.de (made with osmcoastline); it replaces the file daily, so the build fetches an unchanged copy from the project's mirror (`packs/maps/sources/`) |
| Natural Earth (data) | 5.1.2 (tag `v5.1.2` of nvkelso/natural-earth-vector, six 10m GeoJSON files, each pinned by hash) | Public domain | built-in world overview map (`resilience-world-map`, ADR 0016); **shipped in the image** as an 11.7 MB map pack | fetched at build time from raw.githubusercontent.com; every boundary line is drawn alike, disputed or not |
| tippecanoe | 2.79.0 | BSD-2-Clause | builds the tiles of map packs and of the world overview (build side only) | |
| frotz, angband, chess-tui, gnuchess | 2.55, 4.2.6, 2.7.0, 6.3.0 | GPL-2.0+, GPL-2.0, MIT, GPL-3.0+ | entertainment (Phase 7) | |
| tealdeer | 1.8.1 | MIT/Apache-2.0 | command examples | its cache must be shipped offline |

## Rejected or deferred

| Component | Reason |
|---|---|
| calcurse | Pulls ~128 MiB of Python (CalDAV sync helper) into the closure. Revisit with an override without the helper. |
| ModemManager (service) | Resident service unused by most machines; off by default (library stays in NetworkManager's closure). |
| simplex-chat-desktop | Graphical client only in nixpkgs; a CLI exists upstream but is not packaged — re-evaluate. |
| zathura, qalculate-gtk | Graphical. |
| `nomadnet`, `navit`, `dump1090`, `zimwriterfs`, `sudoku-tui` | Not in the pinned nixpkgs under these names (checked). |
