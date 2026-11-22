# CLAUDE.md — Resilience OS project memory

Read this whole file at the start of every session. Keep it accurate: update it
whenever architecture, build process, conventions or status change. The full
original specification is `docs/ORIGINAL-SPEC.md` (authoritative for the
feature scope; this file is authoritative for *current state and
decisions*). Older commits cite it as "MASTER-PROMPT §N"; the sections
are numbered the same.

## 1. Identity and purpose

**Resilience OS** is an offline-first, low-resource, terminal-first Linux
distribution for **x86_64 PCs**, built on **NixOS**. It must stay useful — information,
productivity, local communication, security tools — with no Internet at all.
Primary interface: a Rust TUI (`resilience`). No X11, desktop or WM. From 0.7.0 a
**GUI edition** is planned (ADR 0026: cage Wayland kiosk + `resilience-gui` in iced, one
full-screen app, state machine shared with the TUI in `resilience-shell`).
Editions: **standard** is the main one (the live ISO we build and test);
**minimal** is kept as a secondary edition.

Guiding rule: *Resilience must not depend on the availability of the
infrastructure it is designed to survive without.*

Decision priorities (never reorder to go faster): security > correctness >
reliability > offline > maintainability > low resources > usability >
performance > visual polish.

## 2. Current architecture (summary — details in docs/ARCHITECTURE.md)

```
flake.nix ── nixosModules.default = nix/modules ── imported by every system
   │            core.nix        options: resilience.{edition,apps,probe,package...}
   │                            generates /etc/resilience/{system,apps}.toml
   │                            (build fails if an app command is not executable)
   │            admin.nix       privileged helper: /etc/resilience/admin.toml, polkit actions
   │                            org.resilience.admin.{set-mode,set-time,set-default-generation} (auth_admin)
   │                            and org.resilience.admin.set-keymap (allow_active=yes, ADR 0017),
   │                            tmpfiles for state/audit, activation `apply-state`
   │            media.nix       udisks2 (removable media; its polkit defaults, no overrides)
   │            crypto.nix      gpg-agent (pinentry-curses, cache TTL 0); age/minisign on PATH
   │            packs.nix       resource packs (ADR 0013): /etc/resilience/packs.toml (catalog with
   │                            pinned size+SHA-256, default nix/packs/kiwix.nix), system user
   │                            resilience-packs owns /var/lib/resilience/packs, polkit action
   │                            org.resilience.packs.change (auth_admin), lynx restrictions
   │            users.nix       resilience.users.<n>.mode -> groups, login shell,
   │                            nix allowed-users
   │            interface.nix   resilience.keyboard.{layout (system keymap, default "us"), layouts
   │                            (offered in Settings)}, all checked against kbd at build; getty@
   │                            ExecStartPre loads the system layout before every login prompt;
   │                            getty branding, console font ter-i16n, tty1-only
   │                            autologin via getty@tty1 drop-in (NOT services.getty.autologinUser)
   │            security.nix    sudo (wheel+password), pam_faillock, sysctls, no ssh,
   │                            user namespaces off on Ultra-Stable-only machines (ADR 0012)
   │            networking.nix  NetworkManager (no connectivity check), polkit allowlist for
   │                            accounts without its group (ADR 0010), firewall, no NTP; wired:
   │                            no-auto-default, profiles "Wired" (DHCP, prio 10, 1 retry) then
   │                            "Wired without router" (link-local v4+v6, prio -10) (vm-cable);
   │                            comms.nix adds an NM dispatcher: `up` → try-restart rnsd
   │            comms.nix       local messaging network (ADR 0018): resilience.comms.enable
   │                            (default off; the STANDARD profile turns it on)
   │                            -> resilience-reticulum.service (rnsd, DynamicUser, AutoInterface
   │                            only, UDP 29716+42671), /run/resilience-reticulum/client/config
   │                            (instance name + RPC key, world-readable by design), and the
   │                            systemd USER service resilience-messages (nix/packages/
   │                            resilience-messages: Python around LXMF.LXMRouter; per-account
   │                            identity + messages in ~/.local/share/resilience/messages;
   │                            outbox/ID.json + cancel/ID in, messages/ID.json out), and
   │                            resilience-messages-node.service (ADR 0019: message store = LXMF
   │                            propagation node, `resilience-messages node`, DynamicUser, AF_UNIX
   │                            only, no delivery address, cost 13, peers 1 hop, 32 MB;
   │                            resilience.comms.store.{enable (default on),limitMB}). Files
   │                            (ADR 0020): outbox/ID.file + "file" in the request, ≤ 900 KB, received
   │                            into files/ID.EXT (64 MB/account), stores pass ≤ 1000 KB. rns and lxmf
   │                            are UNFREE (Reticulum License): allowed by name only in
   │                            nix/packages/resilience-reticulum (its own nixpkgs import)
   │            mesh.nix        Wi-Fi mesh (ADR 0021): resilience.mesh.{enable (default = comms.enable),
   │                            autoStart, name, frequency, cellId, devices} -> resilience-mesh.service
   │                            (root oneshot: card out of NM, IBSS cell resilience-mesh 2412 MHz fixed
   │                            BSSID, batman_adv bat0 link-local only, restarts rnsd); started/stopped
   │                            only by `resilience-admin set-mesh on|off` (allow_active=yes)
   │            share.nix       community network, server side (ADR 0025): resilience.share.{enable
   │                            (standard: on, mkDefault), autoStart}; template units
   │                            resilience-share@IF (root oneshot: TCP 80 on IF only in iptables
   │                            chain `resilience-share`, nginx includes + index page in
   │                            /run/resilience-share, state.toml), resilience-share-web@IF (nginx,
   │                            DynamicUser, GET/HEAD), resilience-share-library@IF (kiwix-serve
   │                            127.0.0.1:81); NM dispatcher stops it when IF's connection changes;
   │                            shared folders /var/lib/resilience/shared/{files,pages} root:wheel 2775;
   │                            with comms: resilience-share-announce@IF (`resilience-messages share`,
   │                            DynamicUser, IPAddressDeny=any) announces aspect resilience.share;
   │                            each account's program writes heard machines to shares.json
   │            hotspot.nix     ADR 0025 §6-7: resilience.share.hotspot.{enable (= share.enable),
   │                            ssid, device, dnsRecords}; resilience-hotspot.service (root oneshot:
   │                            NM profile in /run/NetworkManager/system-connections 0600, WPA2,
   │                            ipv4 shared 10.42.0.1/24, IPv6 off; chains resilience-hotspot (DHCP/DNS
   │                            on the card) and resilience-hotspot-fwd in FORWARD (DROP in and out,
   │                            never flushed by the firewall's own scripts)); dnsmasq-shared.d:
   │                            no-resolv, home.arpa only; mesh and hotspot exclude each other
   │            gui.nix         GUI edition (ADR 0026): resilience.gui.{enable,package} (profile
   │                            standard-gui = standard + gui.enable, edition "standard-gui") ->
   │                            /etc/resilience/gui-session: cage (no Xwayland, pixman, -d -s)
   │                            + resilience-gui; fonts dir (InterVariable + 3 DejaVu), foot via
   │                            a wrapper with its own ini (RESILIENCE_TERMINAL); XKB layout from
   │                            `resilience-gui xkb`; resilience-session runs it on tty1 only,
   │                            back to the TUI after 3 quick failures
   │            boot.nix power.nix applications/{productivity,system-tools,networking,entertainment}.nix
   ├─ nixosConfigurations: live-standard, live-standard-gui, live-minimal (ISO), dev-vm (9p VM)
   ├─ packages: resilience (workspace minus the GUI), resilience-gui, iso-gui, resilience-session, resilience-handbook, resilience-world-map, iso, iso-minimal, vm
   ├─ legacyPackages.mapPacks.{luxembourg,testland}: map packs (nix/maps, build machines only)
   └─ checks: resilience (cargo test), vm-boot-offline, vm-modes, vm-admin,
              vm-backup, vm-wifi, vm-generations, vm-packs, vm-documents, vm-crypto, vm-install,
              vm-install-uefi, vm-maps, vm-comms, vm-mesh, vm-share, vm-hotspot, vm-cable, vm-update, vm-licence,
              vm-gui (OCR on the GUI, foot round trip, Ctrl+Alt+F2, fallback to the TUI, idle RSS),
              security-invariants, eval-live-iso, map-pack-testland, map-pack-reproducible, map-world,
              comms-messages (the messaging program with real Reticulum + LXMF, no VM)

crates/resilience-core  library: status, network (passive + on-demand probe),
                        mode detection, catalog, config, handbook, notes, launch, proc,
                        admin (pure mode-change rules), clock (time parsing/bounds),
                        recovery (generations, default-generation rules, report),
                        media (lsblk parsing, udisksctl mount/eject), files (name
                        index, kinds, text caps, CSV), manpage (names, secure env), crypto (key store,
                        key/format recognition, output names, checksums), openpgp
                        (gpg status/colons parsers), backup (tar|zstd|age
                        pipeline, checksum, restore into ~/Restored), wifi (nmcli parsing,
                        private profiles, password via stdin), packs (catalog, inventory,
                        stdin chunk framing, receive+hash, remove, reader URLs), keyboard
                        (layout list, names, loadkeys argv), messages (local messages store:
                        bounded JSON records, peers, contacts, outbox/cancel files, text
                        cleaning; ADR 0018), maps/ (geo, mvt decoder, canvas,
                        style, labels: pure, ADR 0016)
crates/resilience-admin binary `resilience-admin`: the ONLY privileged code (ADR 0007):
                        list | set-mode USER MODE | set-time UTC |
                        set-default-generation N | set-keymap LAYOUT | set-mesh on|off |
                        set-share on IF --name NAME SERVICE... | set-share off (auth_admin) |
                        set-hotspot on|off (auth_admin; prints a new password once) |
                        apply-update DIR | record-licence SHA256 | apply-state; check-update DIR
                        (any user, read-only)
crates/resilience-backup binary `resilience-backup` (UNPRIVILEGED, ADR 0008):
                        create MOUNT | check FILE | restore FILE
crates/resilience-library binaries (ADR 0013): `resilience-library` (as the user: list |
                        change [--from M] [--remove F].. [--get ID].. | open FILE [--search W] |
                        change --from-peer ADDR --get ID.. | peer-list ADDR |
                        open-remote ADDR index|library|pages | peer-files ADDR |
                        peer-get ADDR NAME; src/peer.rs = relay, ADR 0025)
                        and `resilience-pack-store` (as resilience-packs via pkexec --user,
                        never root: change; stdin bytes only hashed, never parsed)
crates/resilience-crypto binary `resilience-crypto` (UNPRIVILEGED, ADR 0015, foreground):
                        encrypt TO FILE | decrypt | write-message TO | read-message | sign |
                        verify | checksum | compare SUM FILE | check-list | inspect | new-keys |
                        add-contact [FILE] | export-keys DIR | new-openpgp-key;
                        src/pgp.rs = GnuPG paths (status-fd parsers in core openpgp.rs)
crates/resilience-maps  binaries (ADR 0016, UNPRIVILEGED): `resilience-maps` (view [--pack F]..
                        [--at LAT,LON] [--scale M] | info [--check] F: hardened MBTiles reader,
                        reader thread, tile cache, 1 GiB address-space limit) and
                        `resilience-mapmaker` (classify | natural-earth | bounds | sea |
                        finish; build machines only, not installed in the system; `sea`
                        writes OpenStreetMap water polygons cut by ogr2ogr as water/sea).
                        nix/packages/resilience-world-map: the built-in world overview
                        (Natural Earth, z0-7), a `[[builtin]]` maps entry of packs.toml
crates/resilience-shell library (ADR 0026): the front-end independent interface logic:
                        app.rs (state machine, returns Effects) + its screen modules
                        (*_ui.rs, library.rs), help.rs, input.rs (KeyEvent, crossterm
                        names), list.rs (ListState: selection + offset), fmt.rs (local time).
                        No ratatui/crossterm here: shared by the TUI and the GUI
crates/resilience-gui   binary `resilience-gui` (ADR 0026, iced 0.14, tiny-skia + wayland +
                        smol only): main.rs (Gui: native Home/Category/Search/Settings,
                        grid arrows, click = App::choose_row), textpanel.rs (other screens:
                        TUI ui::draw_panel into a TestBackend buffer -> styled mono text),
                        run.rs (Effect::Run -> 0600 JSON in $XDG_RUNTIME_DIR -> foot ->
                        `resilience-gui run FILE` -> resilience_tui::run_foreground), style.rs
                        (palette, iced key -> shell key). `resilience-gui xkb` for the session.
                        Preview without a display: scripts/gui-preview.sh OUT.png [WxH] [KEYS]
crates/resilience-tui   library + binary `resilience`: lib.rs (re-exports shell's app/help as
                        crate::app/help; run_foreground), keys.rs (crossterm -> shell keys), ui.rs
                        (rendering; theme from app.theme: ThemeName), theme.rs, tests.rs
                        (flow + screen tests; flow tests move to the shell later)
nix/packages/resilience-session  login shell: runs TUI, restarts after crash, refuses -c
handbook/*.md           offline user handbook shipped in the system
tests/vm/*.nix          NixOS VM tests (QEMU/KVM via nix build .#checks...)
```

Runtime flow: getty (tty1 autologin or login prompt) → login shell
`resilience-session` → `resilience` TUI → programs launched with fixed argv
(never via a shell) → back to TUI. tty2–tty6 always offer normal logins.

Mode change flow: TUI Users screen → consequences + confirm (Cancel default)
→ `pkttyagent` registered for the TUI pid + `pkexec <helper> set-mode U M`
(SHELL set to the session shell: pkexec rejects nologin) → admin password →
helper validates, writes /var/lib/resilience/modes.toml, gpasswd, audit log,
terminates sessions on demotion. Activation (`users` resets groups) →
`resilienceModes` script → `apply-state` re-applies overrides.

Backup flow: TUI Backups & USB sticks → lsblk lists removable drives only →
`udisksctl mount --no-user-interaction` (internal disks need auth → refused)
→ confirm → `resilience-backup create /run/media/U/X` in the foreground (age
reads the passphrase from /dev/tty itself) → `*.partial` renamed when done +
`.sha256`. Restore: checksum first, then into a NEW ~/Restored/backup-<stamp>.
Tool paths: `[tools]` in /etc/resilience/system.toml (checked at build).

Default generation flow: TUI Recovery → System generations (list) → Enter →
confirm (Cancel default) → pkttyagent + `pkexec <helper> set-default-generation
N` → helper: plan_set_default (exists, not default, closure in /nix/store with
switch-to-configuration) → `nix-env -p …/system --switch-generation N` →
`<gen>/bin/switch-to-configuration boot` (clean env; profile switched back on
failure) → audit. Never `switch`: the running system is untouched.

Resource packs flow: TUI Knowledge → Resource packs (Space ticks [+]/[-], f
language, d download, u USB stick) → confirm → `resilience-library change …`
in the foreground → pkttyagent + `pkexec --user resilience-packs
<store> change …` (one admin password) → store prints `send ID OFFSET`/`skip`
lines → library streams curl (HTTPS only, --continue-at) or the stick file as
[u32 len][bytes] chunks → store appends to .staging/F.part, sha256sum over
old+new bytes, rename only on match; early end keeps the part (resume).
Sets: `s` on Resource packs ticks a suggested set. Library also lists built-in
packs (`[[builtin]]`, the docs ZIM from nix/packages/resilience-docs) and packs on
mounted sticks (`u` opens one, `c` → `resilience-library check FILE`).
Reading: Offline library → `resilience-library open FILE` → kiwix-serve
127.0.0.1:<free port> + lynx with every restriction (`-restrictions=…`, not `all`) and `-localhost`.

Crypto flow (ADR 0015): TUI Security entries → Find & view files as picker
(`View::Files { pick: Some(Pick) }`) → "Encrypt for" checklist (passphrase | age
keys | OpenPGP keys, one kind) or checksum box → `resilience-crypto OP … FILE`
in the foreground → age / minisign / gpg (pinentry-curses, GPG_TTY) read
passphrases on the tty themselves; outputs `.partial` + RENAME_NOREPLACE;
decrypted files only in ~/Decrypted; keys in ~/.local/share/resilience/keys
(age identity = age-keygen | age -p, never in clear). Random passwords: TUI
runs xkcdpass/openssl and shows them in a dialog (never notified/logged).
Invariants kept from the ADR 0015 review (do not regress): a non-detached
OpenPGP signature `NAME.asc` beside `NAME` is refused (never VALID for NAME);
clear-signed text is shown exactly as signed; gpg always `--batch
--no-auto-key-retrieve --no-auto-key-locate`, file paths absolute, user id
after `--`, names start alphanumeric; gpg output into memory is bounded
(`pgp::capture`); the binary's SIGINT/SIGTERM/SIGHUP handler unlinks the
current `.partial` (one at a time, `PARTIAL` atomic) and stale partials of
dead PIDs are removed at start; checksum tools run with LC_ALL=C.

Keyboard flow (ADR 0017): boot and every login prompt load the system
layout (console.keyMap; getty@ ExecStartPre). TUI Settings → "Keyboard
<layout>" → `pkexec <helper> set-keymap ID` (SHELL = session shell; polkit
allows the active local session without a password) → helper: ID in
admin.toml `keymaps` → `loadkeys -q -- ID` → audit → TUI saves `keyboard =
"ID"` in the user's settings. Session start: the TUI loads the saved
layout again the same way (unless it is the system layout). The console
has ONE keymap for all ttys: unprivileged loadkeys fails (CAP_SYS_TTY_CONFIG).

Maps flow (ADR 0016): TUI Maps & navigation → "Map" → `resilience-maps view
--pack F...` in the foreground as the user (`[tools] maps`): built-in maps
(`[[builtin]]` with category maps: the world overview), then installed
catalog map packs, at most 8. "Maps on this computer & USB sticks" =
`View::Library { kind: PackKind::Map }` (the Offline library's list for
`.mbtiles`): Enter opens the map on the chosen file over the overview, `c`
→ `resilience-library check`. The viewer opens SQLite hardened, lowers its
memory limit, writes only ~/.local/state/resilience/maps.toml.

Local messages flow (ADR 0018, only with resilience.comms.enable): TUI
Communications → Local messages (shown when /run/resilience-reticulum/
client/config exists) reads ~/.local/share/resilience/messages every 2 s
(`resilience_core::messages::Store`: records, peers, contacts, status) and
writes `outbox/ID.json` (tmp + rename), `cancel/ID`, `contacts.json`. The
user service `resilience-messages` takes the request, sends through the
machine's Reticulum instance and rewrites `messages/ID.json` (queued →
sent → delivered only on LXMF's proof; failed). The TUI never touches the
network; names from the network are shown as claims (`"name"?`).
Store-and-forward (ADR 0019): still undelivered after 2 min → a copy with
the same LXMF hash (record `ts`) goes to the machine's store
(`/run/resilience-messages-node/address`) or the nearest heard; `sent` +
`"stored": true` ("sent, kept on the network"); direct attempts go on.
Stores peer on the local network; programs collect every 5 min. Programs
attach with `require_shared_instance` (exit 75 if rnsd is not up) and save
heard keys (`known_destinations`) every 10 min and at exit.
Files (ADR 0020): Chat `f` → Find & view files as picker (`Pick::SendFile`)
→ `ChatInput::FileText` → `Store::queue_file` copies it to `outbox/ID.file`
then writes `outbox/ID.json` with `"file"`; the program moves it to
`files/ID.data`, sends it in `FIELD_FILE_ATTACHMENTS`, deletes it once
delivered/failed. Incoming: first attachment → `files/ID.EXT` (0600; name
only in the record, cleaned); `o` views it (files_ui viewers), `s` moves it
to `~/Received/<safe name>` with RENAME_NOREPLACE.

Mesh flow (ADR 0021): TUI Communications → "Mesh network (Wi-Fi without
a router)" (`Builtin::Mesh`, shown when admin.toml has `mesh_unit`) →
Confirm (state from /run/resilience-mesh/devices) → `pkexec <helper>
set-mesh on|off` (SHELL = session shell; polkit allow_active=yes) → helper:
`systemctl start|stop -- resilience-mesh.service`, audited → the unit's
script (ExecStart / ExecStopPost) → `systemctl --no-block try-restart
resilience-reticulum` (AutoInterface lists interfaces only at start);
the messaging programs and stores re-announce when the client config's
mtime changes (instance restarted).

Community network flow (ADR 0025): server side = `set-share on IF --name
N SERVICE...` (auth_admin) → `/run/resilience-share/{services,name}` →
`resilience-share@IF` (root: TCP 80 on IF only, nginx includes, index,
state.toml) + `-web@IF` (nginx DynamicUser) + `-library@IF` (kiwix-serve
127.0.0.1:81) + `-announce@IF` (`resilience-messages share`, aspect
`resilience.share`). Each account's messaging program records heard
announces in `shares.json`. TUI Communications → Community network
(`Builtin::Community`, `share_ui.rs`, shown when admin.toml has `share`):
rows = sharing state / "Share this computer's resources" / "Type the
address" / heard machines (`share::parse_heard` + `peer_for`) → PeerMenu
→ `resilience-library open-remote ADDR index|library|pages` (loopback
`peer::Relay`, lynx `-localhost`) or `peer-list ADDR` → PeerPacks (Space
ticks) → `change --from-peer ADDR --get ID...` (plain HTTP to local
addresses only, `share::on_link` against /proc/net/route*, no redirects,
store checks the catalog hash). While sharing, a line on every screen.
Hotspot: Community network row → Confirm → `pkexec <helper> set-hotspot on`
in the foreground (pause_after: the password the helper prints stays on
screen) → helper writes /run/resilience-hotspot/password (0600) → unit.

Wi-Fi flow (as the user): nmcli list → password in a TUI dialog → profile
with `connection.permissions user:U`, no secret → thread: `nmcli connection
up uuid X passwd-file /dev/stdin` (secret on a pipe) → failure deletes a
new profile. Never `nmcli --ask` (wrong password = inescapable prompt loop).

## 3. Directory structure

| Path | Purpose |
|---|---|
| `flake.nix`, `flake.lock` | Entry point; nixpkgs pinned to `nixos-26.05` |
| `Cargo.toml`, `Cargo.lock` | Rust workspace (edition 2024) |
| `crates/` | Rust code (see above) |
| `nix/modules/` | NixOS modules (one concern per file) |
| `nix/packages/` | Derivations for our own software |
| `nix/profiles/` | Editions: `minimal.nix`, `standard.nix` (extended: not yet) |
| `nix/systems/` | Concrete systems: `common.nix`, `live-iso.nix`, `dev-vm.nix` |
| `nix/overlays/` | Overlay exposing our packages (used by the flake's `pkgs`) |
| `nix/maps/` | Map pack derivations (osmium, tippecanoe, mapmaker), checks, test OSM data |
| `nix/packs/` | Resource pack catalog (Kiwix) |
| `handbook/` | Offline handbook pages (`NN-name.md`, sorted by prefix) |
| `tests/vm/` | NixOS VM integration tests |
| `scripts/` | Developer helper scripts (no root, ever) |
| `docs/` | All documentation; ADRs in `docs/decisions/` |

## 4. Technology choices

- NixOS 26.05 stable, flakes, single input (nixpkgs). No flake-utils/crane: fewer inputs.
- Rust stable (host: 1.100 via rustup; Nix build: nixpkgs rustc). ratatui 0.30 (crossterm
  backend, re-exported as `ratatui::crossterm` — do not add a separate crossterm dep),
  serde + toml, tracing, thiserror, libc. **No tokio**: threads + timeouts suffice.
- Only the 8 basic ANSI colours (Linux console has no 256/RGB). Non-ASCII
  glyphs limited to `▀▄█═║╔╗╚╝─│┌┐└┘░▒▓` (console font `ter-i16n`, CP437
  repertoire, 256 glyphs so the VT keeps 16 colours). Enforced by the test
  `only_console_font_glyphs_are_drawn`. ter-v16n lacks ▀▄ and double lines.
  The map viewer may also draw the font's 34 accented letters
  `ÄÅÆÇÉÑÖÜßàáâäåæçèéêëìíîïñòóôöùúûüÿ` in place names (verified from the
  PSF table; `maps::canvas::CONSOLE_LETTERS`, `is_console_char`; ADR 0016).
- See `docs/COMPONENTS.md` for every third-party component, version and license.
- Resilience's own code: **Hippocratic License 2.1** (`LICENSE`, owner's decision
  2026-11-16; SPDX `Hippocratic-2.1` in Cargo.toml; Nix `meta.license = import
  nix/lib/license.nix`, which leaves `free` unset so nixpkgs still builds it).

## 5. Build

```sh
cargo build && cargo test                 # fast Rust loop on the host
nix build .#resilience                    # Nix build (runs cargo test too)
# not `nix flake check` here (10+ GB): evaluate per system/check, §6
nix build .#checks.x86_64-linux.vm-boot-offline -L   # VM test (minutes)
nix build .#iso                           # live ISO -> result/iso/*.iso (heavy)
nix run .#vm                              # dev VM on the serial console
scripts/boot-iso.sh result/iso/*.iso      # boot ISO in QEMU (2 vCPU, 2 GiB)
```
Details: docs/BUILDING.md. The ISO build is the heaviest operation: never run
it concurrently with VM tests.

## 6. Tests

- VM tests read the screen through `/dev/vcsuN` (helper `tty_text`/`wait_tty`
  in each test): the driver's `get_tty_text()` mis-decodes CP437 glyphs.
  Long TUI texts wrap: wait for short fragments.
- Wi-Fi in VM tests: `mac80211_hwsim radios=2`, hostapd+dnsmasq on wlan0
  (NM unmanaged, firewall UDP 67 on wlan0), `networking.wireless.enable =
  mkOverride 0 true` (qemu-vm.nix forces it off), `dmesg -n 1` (kernel
  messages draw over the TUI). See vm-wifi.
- polkit checks must use a process in a real (active) session, e.g. the
  TUI's pid with `pkcheck --process`; `runuser` has no session and gets "no"
  for unrelated reasons.
- Never grep for a secret (or pgrep for a command) with a pattern that
  appears literally in the grep's own command line: `grep 'secre[t]'`.
- pkttyagent asks "Choose identity" when several Advanced accounts exist;
  VM tests must answer it (see `authenticate_as_carol` in vm-admin).
- Generations in VM tests: `virtualisation.useBootLoader` + `useEFIBoot` +
  systemd-boot; a second toplevel via `extendModules` (guarded by a test
  option so it does not nest) in `system.extraDependencies`, installed with
  `nix-env --set` + `switch-to-configuration boot`; `machine.shutdown()` +
  `start()` keeps the disk. TUI lists wrap around: use End/Home, not
  repeated Down. See vm-generations.
- Resource packs in VM tests: never commit `.zim` files (gitignored); build a
  test ZIM with `zimwriterfs` at build time and append its catalog entry to
  `config.resilience.packs.configFile` (mkForce the etc file). A second node
  serves HTTPS with a test certificate (`security.pki.certificateFiles`) and
  `limit_rate` to interrupt a download. See vm-packs.
- Crypto in VM tests: type passphrases into age/minisign/pinentry prompts on
  tty1 (`type_secret`); grep for secrets with `grep -F -f FILE` (patterns in a
  file, never in grep's argv). In Nix `''` strings an empty shell argument
  must be written `""`, not `''`. Peer keys (Bob) are made by root with
  `gpg --homedir /root/bob --batch --pinentry-mode loopback`. See vm-crypto.
- pkexec in VM tests: run it under `setsid -w` when no agent is wanted,
  otherwise its text agent writes into the driver's terminal protocol.
- USB sticks in VM tests: `-device qemu-xhci,id=xhci` + QEMU monitor
  `drive_add`/`device_add usb-storage` (new ids per plug), see vm-backup.
- The age round-trip unit test needs age in PATH (Nix build has it; on the
  host prepend the store path of `nixpkgs#age`, otherwise it prints SKIPPED).
- Installing from the ISO: `python3 -I tests/iso/install.py ISO WORKDIR
  [--uefi OVMF.fd] [--encrypt] [--keymap fr]` (ISO as a USB stick, new
  qcow2 disk in WORKDIR: put it on a real disk such as ~/.cache, never a
  tmpfs; it keeps the disk, delete it). OVMF: `nix build --inputs-from .
  nixpkgs#OVMF.fd`. QEMU 10's `info blockstats` is a table per device.
- ISO: `python3 -I tests/iso/smoke.py ISO OUTDIR [--uefi OVMF.fd] [--keymap us|fr]`
  takes screenshots; look at them (the Read tool displays PNGs). `--keymap`
  is the ISO's layout (default us): QEMU sends physical keys.
- Keyboard: the NixOS test driver types US key codes, which matches the
  system default (`us`). A test that switches the layout must check the
  kernel keymap with `dumpkeys -k` (keycode 16 = q on US, a on AZERTY) and
  type nothing afterwards in that layout. See vm-boot-offline.
- The NixOS test driver type-checks the test script (`Optional` results
  must be checked before use; `machine.sleep` takes an int).
- Map viewer on a real pack: `nix build .#mapPacks.luxembourg`, then a VM
  with `boot.kernelParams = [ "video=1366x768" ]` and `-vga std` gives a
  170x48 console. QEMU's screendump of a 1366-wide framebuffer is skewed
  (1360-pixel PNG): re-stride the pixels to 1366 to look at it.
- README screenshots: `scripts/update-screenshots.sh` (VM
  `tests/screenshots.nix`, exposed as `legacyPackages.screenshots`, not a
  check; one VM) writes `docs/images/*.png`. Look at them before committing;
  retake them at each release.
- World overview: `nix build .#resilience-world-map`, then
  `RESILIENCE_WORLD_PACK=$(echo result/share/resilience/packs/*.mbtiles) cargo test -p
  resilience-maps --test world -- --ignored --nocapture` prints real screens (text only:
  land and sea differ by colour, which the test counts).
- Rust: `cargo test` (unit + TUI behaviour via ratatui TestBackend) and
  `cargo clippy --all-targets` must be clean. `RESILIENCE_DUMP=1 cargo test dump_screens -- --nocapture` prints screens.
- Nix/VM: do NOT run `nix flake check` on this host, not even
  `--no-build`: one evaluator holds every configuration (live ISOs, the
  eight installed variants, dev-vm, all checks) and reached 10.5 GB on
  2026-11-19; the kernel OOM-killed it (and earlier, the session, 2026-11-18).
  Instead evaluate each `nixosConfigurations.NAME.config.system.build.toplevel.drvPath`
  and each `checks.x86_64-linux.NAME.drvPath` in its own `nix eval`
  process (peak ~6.4 GB, freed on exit), then build the list of `.drv^*`
  with one `nix build --no-link --keep-going -L --max-jobs 1`. Same
  coverage. Run all of it inside `systemd-run --user --scope -p
  MemoryMax=7G` so that only our job can be killed. The host is dedicated
  to the project: full runs and ISO builds may start at any time (owner,
  2026-11-20), using as much of the machine as it allows (builds with
  `--max-jobs 2`, the daemon's limit).
- Report each test as PASS / FAIL / BLOCKED / NOT TESTED in docs/TESTING.md.
  Never mark untested things as passing.

## 7. Development environment constraints (HARD RULES)

Host (since 2026-11-20, a machine dedicated to development): Debian 13,
Intel i3-6100 (2c/4t), 11 GiB RAM, user `claude` (groups kvm, docker),
lingering on. Nix 2.36 multi-user daemon (user IS trusted; cache.nixos.org
works; `kvm` and `nixos-test` system features; max-jobs 2, cores 2;
the build users need `/dev/kvm` 0666, docs/BUILDING.md, or VM tests
emulate the CPU and time out). Rust
via rustup in ~/.cargo; age and gnupg in the user's nix profile; qemu,
minisign and jq from Debian. `/bin/sh` is dash (tests must not rely on
bash behaviour). The launcher may pass another user's
`XDG_RUNTIME_DIR`/`DBUS_SESSION_BUS_ADDRESS`: ~/.bashrc resets them to
/run/user/1000; check before `systemd-run --user` or a VM test. The
previous host (Fedora, Ryzen 5 PRO 4650U, 12 GB) is no longer used.
`~/Mirror` is the public mirror itself (nginx in Docker + FRP tunnel,
see its README): publishing = copying into `~/Mirror/files/resilience/`.

- Never use sudo/su/doas/pkexec, never modify the host system, its bootloader,
  firewall, networking, disks or anything outside this repo (and user-local
  caches). If root is required, write instructions for the user instead.
- Never install Resilience on the host. Destructive tests only in disposable VMs.
- The machine is dedicated to the project (owner, 2026-11-20): use as
  much of it as it allows. Memory is the limit (11 GiB): two VM tests at
  once fit (`--max-jobs 2`); an ISO build next to VM tests only while
  `free -h` shows room. Default test VM: 2 vCPU, ≤ 2 GiB RAM.

## 8. Git conventions

Conventional commits (`feat:`, `fix:`, `security:`, `test:`, `docs:`,
`chore:`, `refactor:`). Branches: `develop` is the development branch (work
happens there, or on `feat/<topic>` branches created from `develop` and merged
back into it); `main` only receives `develop` at tested milestones. Both are
on `origin` (GitHub). Push `develop` and `feat/<topic>` branches often,
after each coherent commit (owner, 2026-11-18); merging into `main`,
pushing `main` and pushing release tags only when the owner says so.
Never commit ISOs, disk images, `result*` links, large archives, keys or
secrets (see `.gitignore`). Never force-push, never push `main` or tags unless asked, never
run destructive git operations without explicit authorization.
Commit trailers: `Co-Authored-By: Claude …` as instructed by the session.

## 9. Security requirements (summary — docs/THREAT_MODEL.md, SECURITY.md)

- Modes are OS privileges (groups), never UI flags. Hidden menus are not a boundary.
- The TUI never builds shell commands; programs run from fixed argv.
- No privileged command runner. Privileged actions go only through
  `resilience-admin`: one sub-command + one polkit action (auth_admin, never
  _keep; the only exceptions are `set-keymap`, allow_active=yes for listed
  layouts only, ADR 0017, and `set-mesh on|off`, allow_active=yes, ADR 0021) per operation, every argument validated in the helper, absolute tool
  paths from root-owned admin.toml, environment ignored, every outcome
  audited (/var/log/resilience/audit.log + journal). Add new operations the
  same way (pure rule in resilience-core/admin.rs + unit tests + vm-admin).
- No secrets in Git, Nix expressions, the Nix store, logs or world-readable temp files.
- No cryptography of our own. Use age, minisign, GnuPG, cryptsetup, OpenSSL.
- No background network traffic. The Internet probe runs only on user request,
  with timeouts, and is skipped without a default route.

## 10. NixOS module conventions

- One concern per module file; every option under `resilience.*` with a
  description. Use `lib.mkDefault` for policy defaults users may override;
  hard security settings are plain definitions.
- Modules must work with read-only `pkgs` (NixOS tests): **never set
  `nixpkgs.overlays` in a module**; default packages via `pkgs.callPackage`.
- Applications: declare `resilience.apps.<id>` with absolute store paths
  (`lib.getExe pkg` if `meta.mainProgram` is set, else `"${pkg}/bin/x"`).
  Verify the package exists in the pinned nixpkgs before adding it.
  Provide `restrictedCommand` when a safer mode exists for Ultra-Stable users.
- Format with `nix fmt` (nixfmt).

## 11. Rust conventions

- `resilience-shell` has no ratatui/crossterm/iced code (ADR 0026); screen logic goes
  there, drawing in the front ends.
- `resilience-core` has no UI code; every system reader has a pure parser
  with unit tests. `unsafe` only for libc calls, each with a `// SAFETY:` comment.
- No `unwrap()` outside tests (clippy warns); `expect` only for invariants.
- Anything that touches the network or another process has a timeout.
- The TUI state machine (`App::handle_key`) returns `Effect`s; only `main.rs`
  touches the terminal. Add a test in `tests.rs` for every new screen/flow.
- `cargo fmt`, `cargo clippy --all-targets` clean before committing.

## 12. Resource management

See §7. Prefer `nix flake check --no-build` and `cargo test` for fast loops;
build the ISO only for milestones. Reuse the store; don't garbage-collect it
needlessly. Watch `free -h` during VM tests.

## 13. Application integration guidelines

1. Verify upstream, license, maintenance, nixpkgs presence (pinned rev),
   no graphical dependencies, offline behaviour. Record in docs/COMPONENTS.md.
2. Add it to the right `nix/modules/applications/*.nix` group with a beginner
   description that says how to quit it.
3. Choose `minMode`: anything that can spawn a shell or edit arbitrary files
   is `stable` unless a restricted variant exists.
4. The build-time catalog check ensures the binary exists. Add a VM test
   assertion if the app is important.

## 14. Current implementation status (2026-11-19)

**Now: release 0.4.0 (offline updates ADR 0023, licence notice ADR 0024,
the owner's release key, `scripts/make-release.sh`) is released
(2026-11-18): `main` merged and pushed, tag `v0.4.0` on `82f3152`, release
folder signed by the owner, ISO boot-tested (BIOS, UEFI).** **Release 0.5.0 (Phase 9, community network, ADR 0025) is released
(tag `v0.5.0` pushed, `main` merged; published on the mirror 2026-11-20,
every file HTTP 200, sums and signature checked). History of its
preparation (2026-11-19), done: `feat/community-network` merged (slices
1-7 and the independent review's fixes), version 0.5.0 + CHANGELOG
done; next: the full test run (25 checks incl. `vm-share`,
`vm-hotspot`), ISO build + boot BIOS/UEFI, offline update 0.4.0 → 0.5.0
→ rollback in QEMU (the owner signs the bundle), `make-release.sh` (the
owner signs), then merge into `main` and tag only when the owner says
so. The owner is testing 0.4.0 on new hardware meanwhile; findings may
go into 0.5.0.** **Release plan (owner, 2026-11-19): 0.5.0 = Phase 9
complete, released BEFORE resuming maps (maps then = 0.6.0). Before
tagging 0.5.0: slices 5-7; an independent review of the server side
(nginx config, firewall chain, `set-share`, relay, announce parser) with
its findings fixed; the full test run (24 checks) + ISO boot BIOS/UEFI;
an offline update 0.4.0 → 0.5.0 → rollback in QEMU (first real use of
ADR 0023 between releases); ideally two real laptops (sharing, discovery,
a pack over real Wi-Fi/mesh), otherwise the release notes say "VM-tested
only"; the notes say plainly that 0.5.0 is the first release that can
run a network server (offered in standard, off until an administrator
turns it on).** Release flow: version bump + CHANGELOG on `develop`, full
test run, `make-release.sh --update-key NIXKEY --sign MINISIGNKEY V OUT` at
tag `vV` (the owner signs in their own terminal; secret keys under
`~/.resilience-keys`, later a VeraCrypt container), merge into `main`, tag
and push only when the owner says so; upload `releases/V/` to the mirror
first, `LATEST` last (docs/BUILDING.md). Mirrors: GitHub and
https://mirror.canevet.eu/files/resilience/ = local `~/Mirror/files/resilience/`
on the development machine (hidden `.staging/` then one rename; the
former rsync over Tor is retired, 2026-11-20).
Torrents (owner, 2026-11-21): one per release folder, hybrid v1+v2,
the mirror as web seed, published as
`releases/torrents/resilience_EDITION_VERSION.torrent` + SHA256SUMS; seeded from
this machine by qBittorrent behind ProtonVPN (gluetun, port forwarding)
in `~/Torrents` (its README; secrets 0600 there, never in Git). After
publishing a release folder: `~/Torrents/make-torrent.sh V`.
Systems installed from 0.3.0 have no release key: reinstall once.
Offline updates were tested end to end with the real key in QEMU
(install 0.3.0-with-key, update to 0.3.1, roll back; TESTING.md).

Phase 8 text installer (ADR 0022, released in 0.3.0):
Slices 1 to 5 done (ADR, installed configurations, `resilience-core/src/
install.rs` + `crates/resilience-install`, TUI `install_ui.rs`, `vm-install`
BIOS + LUKS + French layout PASS, handbook `12-installing.md`,
docs/INSTALLATION.md); `vm-install-uefi` (OVMF, no encryption, systemd-boot
fallback path) PASS; the ISO with the installer built and installed from
in QEMU as a USB stick (`tests/iso/install.py`: BIOS + LUKS + fr, UEFI +
plain) PASS (2026-11-18); all 21 checks PASS at `84bf3c4`; merged into
`develop` (2026-11-18, not pushed). Next for 0.3.0: ask the owner.
VM-test lessons: `Machine.sleep` waits for the root shell (use time.sleep
before the installed disk is unlocked); a second machine on the same disk
needs `target.state_dir = installer.state_dir`; never run the test driver
by hand with the default XDG_RUNTIME_DIR (it puts the disk images in
/run/user/1000, a small tmpfs). UEFI without a boot loader: `useEFIBoot`
alone gives OVMF + direct kernel boot (`/sys/firmware/efi` exists); a
`useBootLoader` machine booting another machine's disk needs
`efi.keepVariables = false` (otherwise it builds its own disk image).
Phase 7 (SDR) is POSTPONED by the owner (2026-11-17) until they buy the
hardware: do not start it. Power profiles stay open (need hardware).
Maps (Phase 5) were parked on 2026-11-16 after slice 9 and resumed for
0.6.0 on 2026-11-19 (state below). Phase 6 so far: ADR 0018
(Reticulum + LXMF, opt-in because their licence is not free: owner's
decision); slice 1 = `nix/modules/comms.nix`, `nix/packages/
resilience-reticulum`, `vm-comms`; slice 2 = the per-user program
`resilience-messages` (file formats in ADR 0018), check `comms-messages`.
Slice 3 = `resilience-core/src/messages.rs` and the TUI screens
(`crates/resilience-tui/src/chat_ui.rs`: `Builtin::Chat` "Local
messages" under Communications, views Chats / Nearby / Chat, dialog
`Modal::ChatInput`), handbook `60-local-messages.md`, vm-comms through
the TUI. Slice 4 = ADR 0019 store-and-forward (`resilience-messages node`,
service `resilience-messages-node`, client fallback to the store, TUI
markers, vm-comms subtest with alpha switched off). Slice 5 (branch
`feat/comms-files`) = multi-line messages (Ctrl-N / Ctrl-J / Alt-Enter in
the dialog, at most 40 lines). Slice 6 = ADR 0020 files in messages
(`f` send, `o` open, `s` save into ~/Received). Slice 7 (branch
`feat/mesh`) = ADR 0021 mesh network (BATMAN-adv over IBSS; Yggdrasil
not adopted), `vm-mesh`. Phase 6's local part is then complete.
Internet messengers (Tox, SimpleX, signal-cli; spec §14A) are DEFERRED by
the owner (2026-11-17): do not start them unless asked.
`resilience.comms.enable` is ON by default in the standard edition
(owner, 2026-11-16; `nix/profiles/standard.nix`, mkDefault) and off in
minimal. After that change: full flake check passed (2026-11-17, after
fixing vm-crypto's menu number) and the standard ISO was rebuilt and
booted in QEMU (BIOS, UEFI, 1 GiB) with Local messages running. After
slice 4 (ADR 0019): full flake check passed again and the ISO, rebuilt,
boots with the message store running (2026-11-17).

Phase 5 state (2026-11-20): slices 10 (search), 11 (nearby `n`, go-to
`g`), 12 (spot heights on the world overview), accented names (the 34
CP437 letters of ter-i16n, `canvas::CONSOLE_LETTERS`) and slice 13's
catalog (Luxembourg + Bretagne in nix/packs/maps.nix, on the mirror)
and `vm-maps` (search, nearby, go-to, a spot height, an accented name)
are on `feat/maps-search`, with the TUI fix for keys typed right after
a program quits; merged into `develop` 2026-11-21. Then on
`feat/maps-sea` (2026-11-21): the sea inside regional packs
(OpenStreetMap water polygons, pinned on the mirror, `mapmaker sea`,
class water/sea; ADR 0016), the two catalog packs rebuilt with the
folded index (Bretagne with its sea) and republished with their inputs
(`packs/maps/sources/`), and the independent review of the maps crate
(code-review skill, high) with its 10 findings fixed; full run: all 26
checks PASS (2026-11-21, after a race in `vm-cable`'s test was fixed);
merged into `develop`. **Release 0.6.0 (offline maps)**: tag `v0.6.0`
on `1f7069b` pushed, `main` merged and pushed (`67fef9e`), GitHub
release with its files, release folder signed by the
owner, checked, published on the mirror with `LATEST` = 0.6.0 and its
torrent (2026-11-21); ISO boot BIOS/UEFI and update 0.5.0 → 0.6.0 →
back PASS in QEMU; installed on an MSI GL73 8SE (PASS); the Yoga 11e does
not start the installed system (open). 0.6.0 is complete.
History before slice 10:

Phases 0 to 4 are done (see ROADMAP.md). Phase 5 (offline maps, ADR 0016):
slices 1 to 9 are in `develop`. Slices 1 to 7 came with PR #1; slice 8
(built-in world overview: `resilience-mapmaker natural-earth`,
`nix/packages/resilience-world-map`, `[[builtin]]` entry, check
`map-world`) and slice 9 (TUI Maps & navigation: `Builtin::Map`,
`Builtin::MapFiles`, `[tools] maps`, handbook `45-maps.md`, first part of
`vm-maps`) were merged from `feat/maps-overview`, not reviewed, ISO not
rebuilt with them. Slices 1 to 5 done and reviewed (packs accept `.mbtiles`, geographic
maths, MVT decoder, canvas/style/labels, hardened pack reader); slice 6
(viewer `resilience-maps view`) reviewed in its risky parts and run on the
real Luxembourg pack in QEMU at 170x48 (label fixes followed); slice 7
(pack builder `resilience-mapmaker`, `nix/maps/`) merged, builds the
Luxembourg pack, code not reviewed line by line.

Also done with `feat/maps` (2026-11-16): keyboard layouts (ADR 0017: US default,
choice in Settings through `set-keymap`); `scripts/hw-report.sh` and
`docs/HARDWARE-BOOT-CHECKLIST.md` for trying real PCs; the original
specification as `docs/ORIGINAL-SPEC.md`.

Earlier phases: Phase 2 privileged helper and modes (ADR 0007), removable
media and backups (0008), clock (0009), Wi-Fi (0010), default generation
(0011), user namespaces (0012); Phase 3 resource packs and library (0013),
files and manual pages (0014); Phase 4 guided cryptography (0015).
See docs/TESTING.md for what has actually been executed and passed.

Git state (2026-11-18): release **0.4.0** is `main` = `develop`, tag `v0.4.0`, pushed (0.3.0: tag `v0.3.0` on its merge commit). Before: release **0.2.0** (Phases 2 to 6, local part) is `main` = `develop`, tagged `v0.2.0` (and `v0.1.0` on `3a3c629`), pushed; the 0.2.0 ISO boot-tested in QEMU. The installer (`feat/installer`) was merged into `develop` on 2026-11-18 after all 21 checks passed (not pushed); meant for 0.3.0. Earlier: `main` = `develop` at the Phase 6 milestone, merged and pushed after a full flake check and ISO boot; `develop` = `feat/maps`,
`feat/maps-overview`, `feat/comms` (Phase 6 slices 1 to 4, ADR 0018 and
0019) and `feat/comms-files` (slices 5 and 6, ADR 0020) merged, on origin;
both comms branches are fully merged (new Phase 6 work: a new
`feat/<topic>` branch from `develop`; push only when asked). Last
full `nix flake check -L --max-jobs 1 --keep-going`: `develop` at
`af4b661` (mesh network, ADR 0021), 2026-11-17, all 19 checks (13 VM
tests) passed. Rust: 202 core, 76 TUI, 134 maps (+1 on demand), 5 library/store tests;
clippy clean. The live ISO was rebuilt from `develop` at `af4b661`
(mesh included) and boot-tested in QEMU (BIOS, UEFI, 1-2 GiB) on 2026-11-17;
the end-of-Phase-4 ISO booted once on a ThinkPad L15 Gen 1.

## 15. Known bugs and limitations

- Installer (ADR 0022): whole disk only (no dual boot), ext4, no swap,
  host name `resilience` and UTC fixed; tested in QEMU only (VM tests and
  installs from the ISO), never on a physical disk. The live session
  itself has no admin account (by design).
- Mode changes need an Advanced account to authorise; the live ISO has none,
  so modes cannot be changed there (by design).
- pkexec's built-in text agent fails on success with NixOS 26.05's
  socket-activated polkit helper ("No session for cookie"); always register
  `pkttyagent` first (launch::run_with_polkit_agent).
- Default generation: tested with systemd-boot (UEFI) only; GRUB untested.
  Deleting old generations is not offered. Backups are full copies (no incremental), no rotation;
  LUKS-encrypted sticks are not supported.
- Wi-Fi: no hidden, 802.1X or WEP networks; a wrong password is reported
  only after the 45 s timeout (NetworkManager keeps retrying with it).
- Ultra-Stable restrictions on launched programs rely on restricted modes of
  those programs; they are usability measures, not a sandbox.
- Emergency/rescue shell requires root, which is locked, by design: layered
  recovery (TUI Recovery, other ttys + Advanced/sudo, generations, live ISO).
- Resource packs: installing needs an administrator (none on the live ISO;
  packs can be read from a stick there); Kiwix deletes old files,
  so ageing catalog entries become USB-only. No comms or SDR yet.
- Cryptography: no LUKS sticks, no OpenPGP signing in the TUI (minisign
  signs), no secure deletion of originals (the user is told); contacts'
  keys are only as trustworthy as the user's own check. Creating an
  OpenPGP key may ask the passphrase a third time on a slow machine
  (gpg-agent max-cache-ttl 0 expires the creation's cache nonce before
  the self-signature; the tool says so, vm-crypto answers it).
- Mesh network (ADR 0021): IBSS + BATMAN-adv tested in VMs (wired
  vlans and mac80211_hwsim) and once over the air between two laptops
  (owner, 2026-11-20; cards not recorded, one hop only); open by design;
  the card cannot join ordinary Wi-Fi while it is on; not remembered across
  reboots (unless `autoStart`); the TUI blocks while `set-mesh` runs (a few
  seconds).
- Local messaging: on in standard, off in minimal; two machines tested in
  VMs (minimal profile plus the option) and two real laptops on 0.5.0
  (owner, 2026-11-20: cable, hotspot, mesh; files and store-and-forward
  not tried on hardware);
  store-and-forward needs a third computer that is on (or the
  recipient's machine on while its account is logged out); stored
  messages never become "delivered" unless a direct attempt also
  succeeds; one file per message, at most 900 KB; no "read" state; received
  messages are not announced outside the messages screens.
- Power profiles not implemented (schedutil governor only). Not measured yet.
- Keyboard: all consoles share one keymap (kernel); a login prompt starting
  on any console resets it to the system layout, and the TUI reloads its
  user's layout only at session start (ADR 0017).
- Maps: search, nearby and go-to in 0.6.0; the search index is over the
  folded name since 2026-11-20 ("lodz" finds "Łódź"): packs built before
  have no search until rebuilt (the catalog packs were rebuilt on
  2026-11-21); two regional packs in the catalog (Luxembourg, Bretagne,
  hosted on the owner's mirror under packs/maps/, their inputs under
  packs/maps/sources/; others are built with `nix build .#mapPacks.ID`
  and opened from a stick); the sea of a coastal pack comes from
  OpenStreetMap's water polygons of 2026-11-20, a few days older than
  the extract; nearby's boxes are cut at the antimeridian; the world
  overview draws disputed borders like any other; too many
  point-of-interest names at city scales (style question open); an exact
  name ranks first only for towns or within 10 km (ADR 0016 §4,
  2026-11-20); osmium tags-filter needs ~2 GB RAM to build a pack.
- Boot menu: on a ThinkPad Yoga 11e through a Ventoy stick (UEFI), the
  ISO's GRUB menu is unreadable (white with blue streaks); it still boots
  after 10 s or with Enter. Boot loader unchanged since 0.4.0; cause not
  found (docs/TESTING.md, ISO boot, 2026-11-21).
- Physical hardware: the live ISO booted on a ThinkPad L15 Gen 1 (owner,
  2026-11-16); release 0.5.0 on a ThinkPad L15 + ThinkPad Yoga 11e (owner,
  2026-11-20: cable, sharing + a pack, hotspot, mesh, messages, no errors;
  docs/TESTING.md "Physical hardware: two laptops"). Nothing else outside QEMU.

## 16. Roadmap

See ROADMAP.md. Current: 0.6.0 (maps) is published (2026-11-21). Before: 0.5.0 is released and validated on two laptops
(2026-11-20); maps (Phase 5, slices 10-13; the TUI stop after quitting a
program, found 2026-11-20, is fixed on `feat/maps-search`) for 0.6.0
(owner, 2026-11-19). Release
0.7.0 (GUI edition, ADR 0026) is RELEASED (2026-11-22): tag `v0.7.0` on `main`
`21d163c`, release folder signed by the owner (`make-release.sh --minimal --gui`),
ISOs identical to the ones booted in QEMU, published on the mirror with `LATEST` =
0.7.0, torrent and GitHub release. Hardware: MSI GL73 install OK; Yoga 11e live OK,
installed system does not start (stick starts in legacy mode -> BIOS-only GRUB);
L15 live OK. Next: 0.7.1 = build your own map + hybrid BIOS+UEFI install;
0.7.2-0.7.3 = native GUI screens (ROADMAP Phase 12b). Release
0.8.0: Phase 11, offline assistant (local model via Ollama or
llama.cpp, models as hash-pinned packs, answers grounded in the
libraries and handbook with sources); ADR first; not started. Release 0.7.1
(owner, 2026-11-21): build your own map (Maps → Make or update a map, ROADMAP Phase 5b);
catalog map packs rebuilt once a year. Release 0.9.0
(owner, 2026-11-21): Phase 13, music player, audio recorder, webcam video recorder,
offline translation with downloadable languages; not started.

## 17. Important decisions (docs/decisions/)

- 0001 NixOS 26.05 stable + flakes, single nixpkgs input
- 0002 Rust TUI with ratatui, synchronous design, no tokio
- 0003 Security modes = per-account group membership
- 0004 Offline-first networking: passive detection, explicit probe only
- 0005 Application catalog generated by Nix, verified at build time
- 0006 Session model: login shell wrapper, autologin only for ultra-stable
- 0007 Privileged helper (pkexec + typed ops), persistent mode overrides, audit, recovery layers
- 0008 Removable media via udisks2 (no overrides), unprivileged age-encrypted backups, restore into a new folder
- 0009 Clock set via helper `set-time` (own auth_admin action, bounds, audit) → timedatectl
- 0010 Wi-Fi as the user via nmcli (private profiles, password on stdin); NM polkit allowlist without its group
- 0011 Default generation via helper `set-default-generation` (nix-env + that generation's switch-to-configuration boot)
- 0012 User namespaces: off (max_user_namespaces=0, no Nix sandbox) when every account is Ultra-Stable
- 0013 Resource packs: hash-pinned catalog, unprivileged store via pkexec --user, kiwix-serve + restricted lynx
- 0014 Find & view files inside the TUI (read-only viewers); man in less secure mode
- 0015 Guided cryptography: resilience-crypto (age, minisign, coreutils, xkcdpass, openssl, GnuPG), secrets only typed into the tools
- 0016 Offline maps: MBTiles vector packs (profile resilience-1) built reproducibly in Nix, hardened reader, block-character viewer
- 0017 Keyboard layout: US default, choice in Settings via helper `set-keymap` (allow_active=yes, listed layouts only), reset before every login prompt
- 0018 Local messaging: Reticulum + LXMF (Reticulum License, not free: opt-in module), one unprivileged instance per machine on the local network only, per-user identities and file-based message store
- 0019 Store-and-forward: a message store (LXMF propagation node, our own `resilience-messages node`, no delivery address) on every machine with messaging; clients go direct first, a copy to the store after 2 min
- 0020 Files in local messages: one per message ≤ 900 KB, sent as a copy, received under the message id (64 MB/account), saved on request into ~/Received
- 0021 Mesh network: BATMAN-adv over Wi-Fi ad hoc (IBSS; 802.11s is missing on Intel cards), off until turned on at the console via helper `set-mesh` (allow_active=yes), bat0 link-local only; Yggdrasil not adopted
- 0022 Installer: prebuilt generic installed systems (per edition, UEFI/BIOS, plain/LUKS), accounts as local accounts in modes.toml, keyboard in /var/lib/resilience/keyboard (+ initrd secret), `resilience-install` via polkit allow_active on the live ISO only
- 0023 Offline updates: signed Nix binary cache on a USB stick (release key in nix/release-keys.nix, passed per import, never in nix.conf), helper `apply-update` (auth_admin, newer version of this edition and variant only, switch-to-configuration boot), `check-update` as the user, TUI Recovery → Update from a USB stick
- 0024 Licence notice: before an account's first use of the TUI, Hippocratic License 2.1 summary + original text, "I certify that I have read" → helper `record-licence SHA256` (allow_active=yes) appends to /var/lib/resilience/licence-acceptances.log (root 0644); `resilience.licence.prompt` off only in VM tests (flake wrapper, mkDefault); live ISO asks at every start
- 0025 Community network (accepted 2026-11-19, implemented, reviewed; 0.5.0): sharing packs/library/folder/pages over HTTP (one read-only nginx, TCP 80, one interface) only via helper `set-share` (auth_admin, not kept across reboots), discovery by Reticulum announces, joining as the user (peer packs hash-checked by the store, pages via a loopback relay + restricted lynx), hotspot via `set-hotspot` (auth_admin, WPA2, no forwarding), expert DNS in the hotspot's dnsmasq (`home.arpa`)
- 0026 GUI edition (accepted 2026-11-21, 0.7.0): cage kiosk + iced app, shared `resilience-shell`, own polkit agent, `resilience.interface`

## 18. How to continue safely

1. Read this file, `git status`, `git log --oneline -20`, ROADMAP.md, docs/TESTING.md.
2. Run `cargo test` and `nix flake check --no-build` before changing anything.
3. Work in small vertical slices; add tests; update docs and this file.
4. Commit coherent milestones. Never weaken a test to make it pass.
