# Roadmap

Legend: ✅ done and tested · 🟡 partial · ⬜ not started. "Tested" means a
test in docs/TESTING.md actually ran and passed (VM only so far).

## Phase 0 — Project foundation ✅
- ✅ Environment inspection, Git, CLAUDE.md, documentation set, ADRs 0001–0006
- ✅ Flake pinned to nixos-26.05, dev shell, formatter, checks
- ✅ Build and test scripts (`nix flake check`, `tests/iso/smoke.py`, `scripts/boot-iso.sh`)

## Phase 1 — Resilience Core ✅
- ✅ Minimal NixOS configuration, editions minimal/standard
- ✅ Bootable x86_64 ISO (BIOS + UEFI), boot-tested in QEMU
- ✅ Starts and works without any network
- ✅ Rust TUI: main navigation, status panel, launcher, search, help,
  settings/themes, notifications, error dialogs, confirmations
- ✅ Logging (per-user log file) and crash recovery (session wrapper)
- 🟡 Text boot presentation: quiet kernel, systemd status on failure,
  branded getty and BIOS menu; UEFI GRUB still uses default colours

## Phase 2 — User and security architecture ✅
- ✅ Three modes as real privileges (groups, sudo, Nix daemon, polkit defaults)
- ✅ Account separation, private homes, locked root, permission VM tests
- ✅ Evaluation-time security invariants
- ✅ `resilience-admin`: typed helper (`set-mode`), polkit `auth_admin`
  every time, server-side validation, audit log + journal (ADR 0007)
- ✅ Guided mode switching in the TUI: consequences, confirmation
  (Cancel default), administrator password, persistence across reboots,
  sessions closed on demotion, new rights at next login
- ✅ Recovery access design for locked root (layered, ADR 0007); TUI Recovery
  section: health, failures, recent errors, generations, diagnostic report,
  emergency guide
- ✅ PAM faillock (login, su, sudo, polkit)
- ✅ Setting the clock from the TUI: `resilience-admin set-time`, own polkit
  action (auth_admin), bounds, audit, hardware clock (ADR 0009)
- ✅ Wi-Fi for every account from the TUI (unprivileged nmcli, private
  profiles, password via stdin); NetworkManager polkit allowlist for accounts
  without its group (ADR 0010)
- ✅ Making an earlier generation the default from the TUI:
  `resilience-admin set-default-generation`, own polkit action (auth_admin),
  boot only, undo on failure, audit (ADR 0011)
- ✅ Removable media for every account (udisks2, removable drives only) and
  encrypted backup/check/restore of the home folder (age, ADR 0008)
- ✅ Keyboard layout: US default, each account chooses its layout in
  Settings (helper `set-keymap`, no password at the console, reset before
  every login prompt; ADR 0017)
- ✅ User namespaces reviewed: refused to everyone on Ultra-Stable-only
  machines (live system), kept for the Nix sandbox elsewhere (ADR 0012)

## Phase 3 — Productivity and knowledge ✅
- ✅ Nano, Vim, Midnight Commander, calculator, units, calendar, notes, handbook
- ✅ Kiwix: `kiwix-serve` (127.0.0.1, one file) + lynx (restricted), links and full-text search tested (ADR 0013)
- ✅ Resource packs: choose by category/language with sizes, download (resumable) or copy from USB, hash-pinned catalog, unprivileged store (ADR 0013)
- ✅ Read packs directly from a stick (no installation; works on the live system), check them against the catalog
- ✅ Suggested sets of packs (Essential, Field guides, Knowledge, Extended documentation, Complete; Geography waits for maps)
- ✅ Built-in documentation pack: handbook, NixOS manual and options (searchable)
- ✅ First offline guides with provenance: practical and health packs from named sources (catalog `source`, `license`, medical notice)
- ✅ Manual pages from the TUI, less in secure mode, search of the manuals (ADR 0014)
- ✅ Find & view files: name search, text, PDF (pdftotext), CSV/TSV table, archive listing, SHA-256 (ADR 0014)

## Phase 4 — Cryptography ✅
Guided age/minisign/GnuPG workflows, checksums, random data, key management;
secrets never in argv or logs (ADR 0015).
- [x] Encrypt/decrypt files and messages (age; OpenPGP via GnuPG)
- [x] Own keys (stored encrypted), contacts' public keys, export/import
- [x] Sign files (minisign), verify signatures (minisign and OpenPGP)
- [x] Checksums: show, compare, check SHA256SUMS lists
- [x] Random passphrases, passwords and keys
- [x] Inspect encryption formats
- [ ] Unlock encrypted (LUKS) USB sticks in the media screens (later)
- [ ] OpenPGP signing in the interface (minisign signs; gpg on the command line)

## Phase 5 — Offline maps ✅ (release 0.6.0, 2026-11-21; open items below)
Design: ADR 0016 (MBTiles with vector tiles, block-character viewer, search
inside the pack, reproducible pack builder, routing deferred to its own ADR).
Slices 1 to 9 are in `develop`.
- ✅ Resource packs accept map packs (`.mbtiles`, `maps` category, data date, coverage, attribution)
- ✅ Geographic maths, bounded vector-tile decoder, canvas, style table, labels
- ✅ Hardened pack reader (`resilience-maps`, SQLite opened as untrusted input)
- ✅ Viewer (`resilience-maps view`): reviewed (risky parts), run on the real Luxembourg pack in QEMU at 170×48 (1366×768); label fixes from that run
- 🟡 Pack builder (`resilience-mapmaker`, `nix/maps/`): merged into `feat/maps`; the Luxembourg pack builds and the hardened reader accepts it; code not reviewed line by line, reproducibility check not re-run
- ✅ Built-in world overview (`resilience-world-map`: Natural Earth 5.1.2, zooms 0-7, 11.7 MB, in every image as a `[[builtin]]` map; check `map-world`); without elevation points (they come with spot heights)
- ✅ TUI "Maps & navigation": Map (the viewer on the built-in and installed maps) and the list of maps on this computer and on USB sticks; handbook page; first part of `vm-maps` (the overview opened from the menu)
- ✅ Search (`/`), nearby (`n`), go-to (`g`), spot heights on the world overview, accented names, `vm-maps` driving them (slices 10-12, on `feat/maps-search`)
- ✅ Catalog: Luxembourg and Bretagne on the mirror, rebuilt with the folded search index, inputs published beside them (slice 13)
- ✅ The sea inside regional packs (OpenStreetMap water polygons); independent review of the maps crate, its 10 findings fixed
- ⬜ A pack from a stick and hostile files in `vm-maps`; fewer point-of-interest names at city scales; routing (its own ADR)

## Phase 6 — Networking and communications 🟡
- ✅ Passive diagnostics + on-demand probe; LOCAL LINK detection tested
- ✅ Reticulum and LXMF evaluated and chosen for local messaging, as an opt-in module because of their licence (ADR 0018)
- ✅ Local messaging network: `resilience.comms.enable` (Reticulum instance per machine, unprivileged, local network only), tested between two VMs (`vm-comms`)
- ✅ Per-user messaging program `resilience-messages` (identity, queue with retries, delivery states, signature check), tested with the real libraries (`comms-messages`) and between two VMs
- ✅ TUI "Communications" > Local messages (conversations, people nearby with claimed names, writing, per-message state, "not verified", own names for addresses, my address), handbook page; shown only where the module is on
- ✅ Local messaging on by default in the standard edition (off in minimal); live ISO rebuilt and boot-tested with it (2026-11-17)
- ✅ Store-and-forward for recipients that are away: a message store on every machine with messaging, stores exchange messages on the local network (ADR 0019), tested between two VMs with the sender switched off
- ✅ Multi-line messages (Ctrl-N for a new line while writing)
- ✅ File exchange in local messages: one file per message up to 900 KB, received files kept apart and saved on request into ~/Received (ADR 0020)
- ✅ Mesh network without a router: BATMAN-adv over Wi-Fi ad hoc, turned on from Communications by the person at the console, local messages relayed across it (ADR 0021); Yggdrasil evaluated and not adopted
- ⏸ Internet messengers (Tox, SimpleX Chat, signal-cli documented as unofficial; spec §14A): deferred by the owner on 2026-11-17, to be done later

## Phase 7 — Radio and additional applications ⏸ (SDR postponed by the owner on 2026-11-17 until the hardware is bought)
- ✅ First offline games (nudoku, solitaire, ninvaders, moon-buggy, NetHack)
- ⏸ Receive-only SDR tools with hardware detection and legal notices (postponed: no SDR hardware yet)
- ⬜ Power profiles (Maximum Efficiency / Balanced / Performance) and measurements on hardware

## Phase 8 — Updates and installation 🟡
- ✅ Offline updates (ADR 0023): signed Nix binary cache on a USB stick, `scripts/make-update-bundle.sh`, Recovery → "Update from a USB stick" (check as the user, install with an administrator's password, starts at the next restart, previous version kept); 0.4.0; tested in VMs (`vm-update`) and end to end in QEMU with the real release key (not yet on physical hardware)
- ✅ Licence notice (ADR 0024, requested 2026-11-18, 0.4.0): summary and original text of the Hippocratic License 2.1 before an account's first use (every start on the live ISO), "I certify that I have read", timestamped root-owned record; `vm-licence`
- ✅ Text installer (ADR 0022, 0.3.0): one prebuilt system per edition and firmware, accounts/keyboard/encryption chosen at install as state; tested in VMs and by installing from the ISO in QEMU (not yet on physical hardware)
- ✅ Release publishing (0.4.0): `scripts/make-release.sh`, minisign-signed `SHA256SUMS`, mirror layout `releases/VERSION/` + `LATEST` (GitHub and mirror.canevet.eu)
Text installer, offline update bundles (`nix copy` + minisign manifest),
signed pack verification, rollback tests, installation docs.

### Planned for the final product: `resilience_builder.sh` (requested 2026-11-14, extended 2026-11-17)
A self-contained script that builds a custom ISO from answers to plain
questions, for people who do not write Nix. Sketch:

- Questions, each with a sensible default and an explanation:
  - edition (minimal or standard);
  - **live ISO or installer** ("Do you want an installer or a live
    ISO?"; the installer needs the Phase 8 installer below);
  - accounts, and the security mode of each;
  - which account logs in automatically, if any;
  - language and keyboard layout, time zone;
  - default theme of the interface (green, amber, cyan, monochrome: the
    four themes that exist today);
  - Wi-Fi on or off;
  - local messaging ("Do you want to install Reticulum? (Y/n)"), showing
    the Reticulum License's conditions first, since saying yes accepts
    them (ADR 0018), and whether this machine keeps a message store
    (ADR 0019);
  - **map regions from OpenStreetMap**, as Geofabrik paths separated by
    commas (`europe/france, north-america/us/california`): the script
    checks each path against Geofabrik's index, shows the download size,
    and builds the packs with `nix/maps` (osmium needs about 2 GB of RAM
    per region and big regions take a long time: say so before
    starting). The downloaded extract is pinned by its hash in the
    generated file, so a rebuild is the same;
  - which resource packs, or suggested sets, to **put on the image**
    ("Would you like to download Wikipedia?"), with sizes and the free
    space needed;
  - optional applications;
  - where to write the ISO.
- Writes the answers to a small generated Nix file (`custom.nix`, kept, so
  the same image can be rebuilt or edited later), shows a summary to
  confirm, then runs `nix build` for a `custom` configuration of the flake,
  and prints where the ISO is and how to write it to a stick.
- Requires only Nix with flakes. Never uses sudo; writing the ISO to a
  stick is left to the user, with instructions. Checks free space first
  (large packs make large images) and uses the pack catalog's pinned
  hashes to fetch packs at build time (fixed-output downloads).
- Packs on the image: either inside the squashfs (simple; image size grows)
  or on a second partition of the stick (better for tens of GiB). This is
  to be decided when it is built.
- Passwords are never asked by the script or stored in the Nix files.
  First-boot setup asks for them, or hashed passwords come from a file
  outside the repository.

### Planned: Resilience Netinstall, `resilience_netinst.iso` (requested 2026-11-17)
A small bootable installer that installs Resilience on a disk by
downloading it from the Internet, for machines that have a connection
at install time. The full offline ISO stays the main way to install:
the netinstall is an addition, never a requirement.

- Three modes: **Simplified** (a few questions, safe defaults: whole
  disk, standard edition, one account), **Basic** (the questions of
  `resilience_builder.sh`: accounts and modes, keyboard, theme,
  messaging, partitions from a short list) and **Expert** (manual
  partitioning, LUKS options, the generated Nix configuration shown and
  editable before installing).
- Choosing the offline resources to install or turn on: resource packs
  and suggested sets, map regions, messaging, with sizes and the free
  space left on the target disk.
- Complete tools on the image for repairs: partitioning, file system
  and LUKS tools, `nixos-enter`, network diagnostics, the handbook.
- Downloads are verified, never trusted: packs by their pinned SHA-256
  (as today), the system from signed binary caches. Open question for
  when it is built: Resilience's own packages are not in
  cache.nixos.org, so they must be built on the target (slow on small
  machines) or come from a cache of our own signed with a published key.
- The network connection uses the existing Wi-Fi flow (nmcli, password
  on stdin), and nothing is downloaded before the user confirms the
  summary with the total size.

## Phase 9 — Community network ✅ (requested 2026-11-18; ADR 0025; release 0.5.0)
An assisted tool that lets a person with little experience join or create
a network of Resilience machines and share offline resources on it. Builds
on Phase 6 (mesh, ADR 0021; Reticulum announces, ADR 0018), resource packs
(ADR 0013) and maps (ADR 0016).

- ✅ **ADR 0025 (accepted by the owner 2026-11-19)** settles it: one read-only nginx on TCP 80 on one interface while an administrator shares (`set-share`, auth_admin), discovery by Reticulum announces (no mDNS), joining as the user (packs through the store's hash check, pages through a loopback relay and restricted lynx), hotspot with WPA2 and no forwarding (`set-hotspot`), expert names in the hotspot's DNS (`home.arpa`). Slices 2 to 7 in the ADR; THREAT_MODEL T46 to T50.
- ✅ Slice 2, server side: `nix/modules/share.nix` (nginx, kiwix-serve, firewall chain, NetworkManager dispatcher), helper `set-share`, `vm-share`
- ✅ Slice 3, discovery: `resilience-messages share` announces while sharing (`resilience-share-announce@IF`); each account's program records what it hears in `shares.json` (checked, local addresses only, 64 machines, 1 hour); `resilience_core::share::parse_heard`
- ✅ Slice 4, joining: `resilience-library change --from-peer`, `peer-list`, `open-remote` (loopback relay, lynx `-localhost`), TUI Communications → Community network (computers heard, their library, pages and packs, sharing this computer), `vm-share` through the TUI
- ✅ Slice 5, shared folder: `resilience-library peer-files` / `peer-get` (exact listed size, hidden `.partial`, then into ~/Received under a safe name, never replacing), TUI "Files in its shared folder"; the share form counts what will and will not be served
- ✅ Slice 6, hotspot and expert DNS: `nix/modules/hotspot.nix`, helper `set-hotspot` (auth_admin, password made by the helper and shown once), WPA2, no forwarding (also through firewall reloads), DNS for `home.arpa` only (`dnsRecords`), mesh and hotspot exclusive; TUI rows to create a network (hotspot, mesh); `vm-hotspot`
- ✅ Slice 7: handbook page, docs, an independent review of the server side and the joining path, its fixes (ADR 0025)
- **An ADR comes first.** This is the first time a Resilience service
  accepts requests from the network: today nothing listens beyond
  `127.0.0.1` and the firewall allows nothing by default. The ADR settles
  the services, ports and discovery. `THREAT_MODEL.md` gets a section on
  serving to the local network.
- **The network is open:** anyone in range can join and read what is
  shared, and the interface says so plainly before anything is shared.
  Integrity does not depend on trusting the host: packs are checked
  against the catalog's pinned hashes.
- **Hosting rules:** off by default; each service is turned on separately
  through `resilience-admin` (own polkit action, `auth_admin` every time,
  audit), so only an Administrator can share. The machine always shows
  what it serves, on which network, and how to stop it.
- **The assistant, three choices:**
  - *Join a network:* find nearby networks (Wi-Fi, mesh) and the
    Resilience machines and services on them;
  - *Create a network:* a hotspot or the mesh, with plain explanations;
  - *Share something on this network:* pick services from a short list.
- **Services built for Resilience, not general-purpose servers:**
  - *Share my packs:* other machines list and download packs from this
    one, verified by the catalog hashes (a hostile host cannot plant
    content);
  - *Share my library and maps:* kiwix-serve and map packs opened to the
    local network instead of `127.0.0.1`;
  - *Shared folder:* read-only at first; receiving uploads from others
    only later, after its own review;
  - *Simple web pages:* a folder of static pages served read-only (for
    notices, local news, guides), readable with lynx.
- **Name resolution:** by default Resilience's simplified naming, with no
  DNS server: machines and services announce themselves (Reticulum
  announces or mDNS) and appear by name in the assistant.
  **Expert option:** a real DNS server (and DHCP on a hotspot, e.g.
  dnsmasq) for experienced users, with its own zone and records, behind a
  clear "advanced" step; it also serves devices that are not Resilience
  machines.
- **Tests:** VM tests with two or three machines (join, create, each
  service, the expert DNS, stopping a service closes its port), and the
  refusals (a non-Administrator cannot share; a tampered pack from a
  host is rejected).

## Phase 10 — Audit and release ⬜
Independent architecture and security reviews, `cargo deny`, license audit,
SBOM, physical hardware tests, release notes.

## Phase 12 — Graphical edition (GUI) ⬜ (requested 2026-11-21; release 0.7.0, owner 2026-11-21)
A second edition with a very light, minimalist, intuitive graphical interface:
cage (Wayland kiosk) + `resilience-gui` (Rust, iced), sharing one state machine
(`resilience-shell`) with the TUI. The TUI edition stays. See ADR 0026 (slices listed there).

## Phase 11 — Offline assistant (local AI) ⬜ (requested 2026-11-19; release 0.8.0, owner 2026-11-21: after the GUI)
A local language model that answers questions from the offline content
already on the computer: the Resilience handbook, the system
documentation and the installed resource packs (Wikipedia, WikiMed,
Wikivoyage, iFixit...), without any network. Order with the other phases
decided by the owner on 2026-11-20: release 0.7.0, after maps (0.6.0).

- **An ADR comes first.** It settles the points below; THREAT_MODEL.md
  gets a section on a model running on the machine.
- **Runtime**: Ollama (0.40.3 in the pinned nixpkgs, MIT, a NixOS module
  exists) or llama.cpp's server directly (MIT): chosen in the ADR. Either
  runs unprivileged (DynamicUser), listens on 127.0.0.1 only, no network
  access of its own (IPAddressDeny except loopback), memory limit, started
  only while someone uses the assistant. Off by default; probably the
  standard edition only, or an "extended" edition.
- **Models are resource packs** (ADR 0013): GGUF files with their size,
  SHA-256, licence and source pinned in the catalog, installed by the
  existing store (administrator's password, hash checked, USB stick or
  download), never pulled by the runtime itself (Ollama's own `pull` and
  `loadModels` download unchecked files: not used). Each model's licence
  is shown before installing; some restrict uses (Llama, Gemma), others
  are Apache 2.0 or MIT (Qwen, Mistral, Phi, ...). Sizes by memory: about
  1-2 GB for 1-3B parameter models (4 GB machines), 4-5 GB for 7-8B
  (8 GB+).
- **Grounded answers, with sources**: retrieval before generation.
  Kiwix's own full-text index (Xapian, already inside each ZIM) finds the
  articles, a small index covers the handbook and the NixOS
  documentation pack, and the model answers from those passages only,
  listing the articles it used; Enter opens one in the offline library.
  No embeddings needed at first (an embedding index per pack can come
  later if search quality requires it).
- **Honest limits in the interface**: a model can be wrong even with
  sources; health, safety and legal answers say so plainly and point to
  the source articles (WikiMed, the handbook); nothing it says is
  executed. It never sees the user's files unless they choose one.
- **Interface**: KNOWLEDGE & DOCUMENTATION > Ask the library: a question,
  the answer streamed as it comes, the sources below; a Stop key; how
  much memory it takes and how long the computer needs, shown before the
  first use on that machine (CPU only on most target hardware: several
  words per second for small models).
- **Tests**: a tiny test model (or a stub server with the same API) in VM
  tests: an answer cites the right test ZIM article; no network traffic;
  the service unprivileged and stopped after use; a model file with a
  wrong hash refused.

## Phase 5b — Build your own map ⬜ (requested 2026-11-21; release 0.7.1, owner 2026-11-21)
Catalog map packs are rebuilt once a year (release checklist); users update or add regions themselves:
- Maps → "Make or update a map": region list shipped offline (Geofabrik index, size + RAM needed),
  disk/RAM check before starting, `.osm.pbf` download on request only (HTTPS, Geofabrik checksum,
  resume) or from a USB stick, build in the foreground as the user under a memory limit
  (osmium + tippecanoe + resilience-mapmaker, no Nix), result in the user's maps, data date shown.
- Optional sea fill (OSM water polygons ~800 MB + GDAL); without it coastal maps keep their coastline.
- Standard edition only (+30-50 MB). ADR 0016 amendment, threat model entry, VM test on the test region.

## Phase 13 — Media and translation ⬜ (requested 2026-11-21; release 0.9.0, owner 2026-11-21)
All offline, the same screens in the TUI and the GUI (resilience-shell), ADR first.
- Music player: ~/Music, USB sticks, later music packs (public domain / CC); playback goes on
  in the background (mpv over JSON IPC, no network, no scripts); proposal: PipeWire user service.
- Audio recorder: microphone to ~/Recordings (Opus), level meter, pause, no upload.
- Video recorder: webcam (V4L2) to ~/Videos, visible recording indicator, camera off when idle.
- Offline translation: a local engine (candidate: Bergamot / Firefox Translations models, or
  Argos/OPUS-MT) with language pairs as hash-pinned downloadable packs (ADR 0013).
- Threat model: crafted media files, camera/microphone privacy, model packs integrity.

## Disk space (estimate, 2026-11-16)
Measured: the system closure (live-standard) is **2.3 GiB**, of which
`linux-firmware` 825 MiB, a nixpkgs source copy 197 MiB, Python 128 MiB.
An installed system needs room for a second generation during updates
(up to +2.3 GiB when everything changes), plus /var, logs, home and a
free-space margin: plan **about 8 GiB before content**. Content sizes: a
France map pack z0-z14 with search ~4.3 GB (ADR 0016 *estimate*);
Wikipedia FR "top" / Wiktionary FR / Wikipedia FR complete 1.4 / 4.6 /
12.0 GiB. Packs can also be read from a USB stick.

## Known limitations carried forward
See CLAUDE.md §15.
