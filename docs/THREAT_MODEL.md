# Threat model

Version 0.5 (2026-11-19, Phase 9: serving to the local network, ADR 0025, implemented and reviewed; first version 2026-11-14). Reviewed whenever a security-relevant feature is
added. "Implemented" means implemented **and tested**; see docs/TESTING.md.

## 1. What we protect

| Asset | Why it matters |
|---|---|
| System integrity (Nix store, /etc, bootloader, generations) | A broken system may be unrecoverable without the Internet |
| User data (home directories, notes, keys) | Irreplaceable; private |
| Private keys and passphrases (age, GnuPG, minisign) — future | Authenticity of communications and updates |
| Availability of the interface | The machine is useless to beginners without it |
| Authenticity of updates and resource packs — future | Offline media are an easy injection vector |

## 2. Actors and capabilities

| Actor | Capabilities | In scope |
|---|---|---|
| **Inexperienced user** (Ultra-Stable) | Keyboard at the TUI; may press anything | Yes — primary design driver: *mistakes* must not damage the system |
| **Curious/hostile local user** (Ultra-Stable/Stable account) | Arbitrary code as their own UID (assume shell escapes exist) | Yes — must not gain root, read others' files, change the system, or fill the Nix store (Ultra-Stable) |
| **Malicious program or document** (e.g. a crafted handbook page, ZIM, map tile, resource pack) | Data parsed by our code or by integrated tools | Yes — parsers must fail safely; packs verified before install (Phase 3/8) |
| **Local-network attacker** | Packets on the LAN / Wi-Fi | Yes — no listening services by default; messaging authenticates peers (ADR 0018); sharing (ADR 0025) only on one network while an administrator wants it |
| **Supply chain** | Compromised upstream source or binary cache | Partly — pinned nixpkgs, hash-locked Cargo deps, signed caches; we cannot audit all of nixpkgs |
| **Physical attacker** | Boots other media, removes the disk, keyboard at boot menu | **Out of scope for integrity** unless full-disk encryption and firmware passwords are used. Documented, not hidden. |
| **Remote Internet attacker** | Nothing listens; outbound connections only on request | Yes, but surface is minimal |

## 3. Trust boundaries

1. **Kernel/UID boundary** between accounts. The only real boundary for
   modes. Enforced by Linux permissions, sudo (wheel + password), polkit,
   the Nix daemon's `allowed-users`.
2. **TUI ↔ launched programs**: the TUI runs programs from fixed argv taken
   from a root-owned catalog; it never interprets user input as a command.
3. **System ↔ external media** (future): resource packs and update bundles
   are untrusted until their signature is verified.
4. **Machine ↔ network**: default deny inbound (firewall, no services);
   outbound only on explicit request.

## 4. Threats and mitigations

| # | Threat | Mitigation | Status |
|---|---|---|---|
| T1 | Beginner deletes or corrupts system files | Ultra-Stable has no write access outside home; Nix store read-only; no sudo | Implemented (vm-modes) |
| T2 | Beginner reaches a shell and runs damaging commands | No shell entry; login shell refuses `-c`; `$SHELL=nologin` for program escapes; **even with a shell, no privileges** | Implemented (vm-modes, vm-boot-offline) |
| T3 | Ultra-Stable user installs or builds software, fills disk via Nix | Nix daemon refuses non-allowed users | Implemented (vm-modes) |
| T4 | Privilege escalation via the TUI | TUI runs unprivileged; no command construction; power actions via logind policy with `--no-ask-password`; the only privileged paths are `pkexec resilience-admin set-mode` and `set-time`, each of which needs an administrator's password every time | Implemented (vm-admin) |
| T5 | Stable user changes system configuration | No sudo; /etc and store read-only; polkit denies unit management | Implemented (vm-modes) |
| T6 | Advanced user's password guessed by another local user | sudo/polkit require the Advanced user's password; pam_faillock on login, su, sudo, polkit-1 (5 failures / 15 min → 5 min lock) | Implemented (vm-admin, security-invariants) |
| T7 | Root login | Root password locked ("!"), no SSH | Implemented (security-invariants) |
| T8 | Users read each other's files | Home directories mode 0700 (NixOS default); notes and settings created 0600 | Implemented (vm-modes, unit tests) |
| T9 | Background network traffic leaks presence/metadata | No NTP, no NM connectivity check, no update checks; probe only on keypress, configurable targets | Implemented (security-invariants, vm-boot-offline `ss` check) |
| T10 | Hang or freeze when the network is absent/broken | Passive detection; probe skipped without route; timeouts on every external command | Implemented (unit tests, vm-boot-offline) |
| T11 | TUI crash locks user out | Session wrapper restarts it; ttys 2–6; previous generations in boot menu | Implemented (vm-boot-offline crash test) |
| T12 | Malformed catalog/config breaks the interface | Strict parsing, defaults on failure, built-ins keep working | Implemented (unit tests) |
| T13 | Path traversal via note names | Strict name validation, no `/`, no leading `.`, no `..` | Implemented (unit tests) |
| T14 | Secrets in logs, store, process lists | Passphrases are typed into age/minisign themselves on the terminal (backups, cryptography): never in our process, argv, env or logs; rules in SECURITY.md | Implemented (vm-backup, vm-crypto greps the journal, home and /tmp for every passphrase) |
| T15 | Malicious, damaged, replayed or downgraded offline update (ADR 0023): a stick with a forged system, an older signed release, another edition, or a bundle used to run code as root | Every store path signed with the release key built into the system (`nix/release-keys.nix`), passed to Nix per import (`require-sigs`, never in nix.conf); `nix copy` re-hashes every NAR and registers nothing on a bad hash or signature; the signed manifest is imported alone first and must name the same edition, a strictly greater version and this variant; the new system must announce the same version and edition; only folders under /run/media; `apply-update` is auth_admin and audited; the new system starts at the next boot (`switch-to-configuration boot`), the previous one stays in the menu; checking runs as the user and imports nothing. Resource packs: T31 | Implemented (vm-update: tampered, wrong key, older, other edition, manifest without system refused; unit tests); end to end with the real key in QEMU (TESTING.md). The release key's secret part is the single point of trust: kept offline by the owner; losing it leaves a reinstall as the only way to change keys |
| T16 | Kernel info leaks / ptrace abuse | sysctls: kptr_restrict, dmesg_restrict, ptrace_scope=1, unprivileged BPF off, kexec off | Implemented (config) |
| T17 | Physical access bypass | Out of scope without FDE; installer will offer LUKS | Documented |
| T18 | Recovery needs root but root is locked | Layered recovery without a root password (ADR 0007): TUI Recovery section, other consoles + Advanced account with sudo, earlier generations, live ISO; the installer refuses to install without an Advanced account (ADR 0022) | Recovery section tested (unit/TUI); installer's administrator rule tested (unit); an Advanced account with sudo on an installed system tested in `vm-install` (wheel, login); recovering an installed system through generations NOT TESTED |
| T19 | Abuse of the privileged helper (argument injection, unmanaged accounts, lock-out of all admins, privileges kept after demotion) | Fixed sub-commands; every argument validated in the helper; only accounts in `resilience.users`; only the three mode groups touched; absolute tool paths from root-owned config, environment ignored; last administrator protected; autologin stays Ultra-Stable; demotion terminates sessions; refusal on corrupt state | Implemented (vm-admin, unit tests) |
| T20 | Unaudited security-sensitive changes | Append-only logfmt audit log (root:wheel 0640) + journal (authpriv) for every set-mode/set-time/apply-state outcome (set-time records the clock before and after); polkit logs denied authorisations | Implemented (vm-admin) |
| T21 | Fake password prompt captures an administrator's password in another user's session | Inherent to "over-the-shoulder" authorisation; Ultra-Stable sessions cannot run arbitrary programs; handbook tells administrators to authorise only on a trusted screen | Accepted risk, documented |
| T22 | Mode overrides silently reverted or stale | Recorded overrides re-applied after every activation; overrides older than a changed Nix declaration are dropped and audited | Implemented (vm-admin reboot test; staleness: unit tests) |
| T23 | Malicious or damaged removable media (USB stick) | Only removable/hot-plugged drives offered; mounted only on user request (no automount), by udisks2, `nosuid,nodev`, per user under /run/media; internal disks need an administrator (udisks polkit, `--no-user-interaction`); common file systems only. Kernel file-system drivers still parse the stick | Implemented (vm-backup); kernel parsing: accepted risk |
| T24 | Lost or stolen backup stick exposes user files | Backups encrypted with age (passphrase, scrypt); passphrase typed into age on the terminal, never in our argv, env or logs; no plaintext written to the stick (vm-backup scans the raw device) | Implemented (vm-backup, unit tests) |
| T25 | Restoring a backup destroys newer files, or a bad backup goes unnoticed | Restore only into a new 0700 folder under ~/Restored; checksum checked first, damaged copies refused; age authenticates every chunk; partial backups never listed (`.partial` + rename); failed restores marked `-incomplete` | Implemented (vm-backup, unit tests) |
| T26 | Clock moved to hide log entries, shorten faillock lock-outs or (later) accept expired signatures | Only via `resilience-admin set-time` with an administrator's password every time (own polkit action, auth_admin); strict UTC format and bounds 2025 ≤ t < 2100 checked in the helper; audited with the previous time; no polkit rule relaxes timedated; no NTP in the background | Implemented (vm-admin, unit tests, security-invariants). Root and Advanced users can still use `date`/`timedatectl` directly |
| T27 | Ultra-Stable account disrupts or reconfigures networking (switches it off, hotspot, connectivity checks = background traffic) through NetworkManager's permissive polkit defaults for active sessions | polkit allowlist for accounts without the networkmanager group: only wifi.scan, network-control, settings.modify.own, enable-disable-wifi; everything else NO (ADR 0010) | Implemented (vm-wifi pkcheck on the TUI process, security-invariants). Before 2026-11-14 these defaults applied unnoticed |
| T28 | Wi-Fi password exposed | Typed in the TUI (masked), passed to nmcli on stdin (`passwd-file /dev/stdin`), never argv/env/logs/files we write; stored by NetworkManager root 0600; profile private to the account | Implemented (vm-wifi, unit test with fake nmcli) |
| T29 | Old, vulnerable or broken generation made permanent, or the boot menu left broken | Only via `resilience-admin set-default-generation` with an administrator's password every time (own polkit action, auth_admin); number validated, generation must exist and be a system closure in /nix/store; `boot` only (running system unchanged); profile switched back if writing the boot menu fails; audited; clean environment for nix-env and switch-to-configuration | Implemented (vm-generations, unit tests). Tested with systemd-boot (UEFI) only |
| T30 | Kernel privilege escalation through user namespaces | `user.max_user_namespaces = 0` on Ultra-Stable-only machines (live system), checked by an invariant; kept elsewhere for the Nix sandbox (ADR 0012) | Implemented (vm-boot-offline, vm-modes, security-invariants). Not restricted on mixed machines |
| T31 | Malicious, damaged or substituted resource pack (mirror, network, USB stick) | Catalog in the root-owned system configuration pins each file's size and SHA-256; the store installs only on an exact match (mismatch or extra bytes: deleted); HTTPS only, also after redirects; files from sticks checked the same way (ADR 0013) | Implemented (vm-packs: tampered download refused, stick checked; unit tests) |
| T32 | Pack installation used to gain privileges or damage the system | No root: `resilience-pack-store` runs as the unprivileged `resilience-packs` account via `pkexec --user` (refuses any other uid), with an administrator's password every time (own action, auth_admin); typed arguments (catalog ids, plain `.zim` names); content never parsed, only hashed; O_NOFOLLOW, regular files only, lock, free-space margin; journal log | Implemented (vm-packs, unit tests) |
| T34 | Shell from a file viewer or the manual pager for Ultra-Stable accounts; terminal escape sequences in a viewed file | Files are shown inside the TUI (text read directly, control characters replaced; PDF via pdftotext; archives only listed by bsdtar -t; nothing extracted or run); man runs less with LESSSECURE=1 and LESSOPEN/MANOPT/BROWSER/EDITOR emptied; typed names validated (no options, no shell) (ADR 0014) | Implemented (vm-documents, unit tests) |
| T35 | Private key stolen from a copied home folder, backup or lost laptop | Own keys always stored encrypted with the user's passphrase (age identity via `age-keygen \| age -p`, minisign's own encryption); key folder 0700, files 0600; private keys refused when "importing a contact" | Implemented (vm-crypto: no `AGE-SECRET-KEY` on disk, modes; unit tests). Brute force of a weak passphrase: scrypt slows it, a strong passphrase is the defence |
| T36 | Encryption or signature results misunderstood (false sense of security) | Results in words with their limits: valid signature ≠ safe file; unknown signer = "NOT CHECKED"; checksum match only as good as its source; MD5/SHA-1 flagged; weak formats named; original left in place and the user told; lost passphrase = lost data, said at every creation | Implemented (unit tests, vm-crypto) |
| T37 | Substituted contact key (man in the middle when exchanging keys) | Short key forms to compare by voice; the user is asked to check in person or by phone; a contact whose key changes is refused until removed; no automatic key fetching (offline) | Implemented (host end-to-end, unit tests). Depends on the user really checking |
| T38 | Decrypted data left behind or overwriting files | Decrypted files only in ~/Decrypted (0700/0600), O_EXCL + RENAME_NOREPLACE, partial output deleted on failure (wrong passphrase leaves nothing); messages decrypted in memory only | Implemented (vm-crypto, unit tests) |
| T39 | GnuPG leaks presence, fetches keys, keeps passphrases, or trusts keys for the user | Always `--batch --no-auto-key-retrieve --no-auto-key-locate`, no key server; gpg-agent cache TTL 0 (invariant); fingerprint shown and confirmation asked before any import (`--show-keys`), private keys refused; `--trust-model always` only for keys the user chose; messages without integrity protection refused | Implemented (vm-crypto with a real GnuPG peer, security-invariants, unit tests on real gpg status output) |
| T40 | A signature made to look like it covers a file it does not (clear-signed `NAME.asc` next to `NAME`), or options injected into gpg | Such signatures refused next to a same-named file; clear-signed text shown exactly as signed; "not a detached signature" is a refusal; names typed for keys cannot start with `-`, `--` before the user id; absolute paths everywhere (ADR 0015 review) | Implemented (vm-crypto regression step, host reproduction of the review's cases, unit tests) |
| T41 | Console keymap used against another person: a key bound to a typed command line, or a login password typed with the wrong layout until faillock locks the account | The kernel allows keymap changes only with CAP_SYS_TTY_CONFIG; the only path for users is `resilience-admin set-keymap` with a name from the root-owned list in admin.toml (no user-supplied keymap files, so no string bindings of their choosing), `loadkeys -q -- NAME` with fixed argv, audited; polkit allows it without a password to the active local session only (ADR 0017); the system layout is loaded again before every login prompt (getty@ ExecStartPre) | Implemented (vm-boot-offline, unit tests). Concurrent sessions on several consoles share one keymap (kernel) |
| T42 | Files from nearby computers in local messages: filling the disk, overwriting or hiding among the user's files, a name that is a path or an escape sequence, a file that runs | Received files kept apart in the messages folder (0700), named by message id (`ID.EXT`, extension `[a-z0-9]{1,8}` only), 0600, never executable, at most 900 KB each and 64 MB per account (then not kept, the text is); the name that came with them is cleaned and only shown; saved only on request, moved into ~/Received under a safe name (no `/`, no leading `.`, `-` or `~`) with RENAME_NOREPLACE; viewed only with the read-only viewers of T34; the sender's copy is a snapshot (ADR 0020) | Implemented (comms-messages, vm-comms, unit tests). The viewers parse hostile files (pdftotext, bsdtar) as they do for USB sticks |
| T43 | The mesh network (ADR 0021) used to reach or misroute the computer: a neighbour in radio range sends router advertisements or DHCP to become its gateway, reaches services on it, or the mesh is turned on behind the user's back | bat0 has only an IPv6 link-local address (accept_ra=0, autoconf=0, no forwarding, no IPv4, NetworkManager never manages it); the firewall is unchanged (only Reticulum's two UDP ports); the mesh starts only when the person at the console asks (`set-mesh`, polkit allow_active only, audited) or with `autoStart` set by the administrator; turning it off restores the card | Implemented (vm-mesh, security-invariants, unit tests). Frames from anyone in range are parsed by the kernel's batman_adv and mac80211; the mesh is open by design (traffic on it is Reticulum's, end-to-end encrypted) |
| T44 | The installer (ADR 0022) erases the wrong disk, or is used to take over a computer: a person at a live system's console erases an internal disk, the program is run with chosen arguments, the live stick itself is chosen, or secrets end up on the disk or in the Nix store | `resilience-install` runs as root only through pkexec and the polkit action `org.resilience.install.run`, which exists on live systems only (assertion: never with local accounts) and allows only the active local session; such a person can already boot anything and erase the disk. Only whole, writable, unused disks of at least 16 GiB are offered; disks with a mounted partition (the live stick) are "in use"; the TUI requires the disk's name to be typed; the root program re-checks every argument and that the disk still has the size and model shown. Passphrase and passwords are typed into cryptsetup and passwd, never passed in arguments, files or the store; the installed systems are prebuilt and identical for every machine (no secret in them) | Implemented; `vm-install`, `vm-install-uefi` (wrong name refused and the disk unchanged; the running disk listed "in use") and installs from the ISO with the stick "in use" (TESTING.md) |
| T45 | The licence record (ADR 0024) forged or used to write as root | `record-licence` takes only a SHA-256 that must equal the hash of the licence file named in root-owned admin.toml; the helper writes one line with the caller's uid as set by pkexec (`PKEXEC_UID`, not an argument) and its account name and the system clock, `O_APPEND \| O_NOFOLLOW`, root-owned 0644; polkit allows it without a password to the active local session only; audited | Implemented (vm-licence, unit tests). The record shows the screen was shown and the box ticked; it is not a signature |
| T46 | Sharing (ADR 0025) turned on without the administrator, on more networks than intended, or kept on another network | Only `resilience-admin set-share` with an administrator's password every time (own action, auth_admin, also to stop); one interface only, TCP 80 opened on it alone in the `resilience-share` chain, flushed at stop or failure; not kept across reboots (except `autoStart` in the configuration); stopped when the interface disappears or its NetworkManager connection changes; a line on the main screen while sharing; audited | Implemented (vm-share, security-invariants, unit tests) |
| T47 | Hostile requests from the network against the server: parser bugs, path traversal, symlinks, dotfiles, uploads, exhausting the machine | nginx only while sharing, DynamicUser, CAP_NET_BIND_SERVICE only, read-only file system, MemoryMax/TasksMax; GET/HEAD only, body size 0, `disable_symlinks on`, dotfiles refused, only the turned-on paths; limit_conn/limit_req per address, short timeouts; kiwix-serve behind it on 127.0.0.1, its own hardened unit; no access log | Implemented (vm-share). nginx and kiwix-serve parse hostile requests: accepted risk while sharing |
| T48 | Private files published by mistake through the shared folder or pages | Served by a dynamic user: only world-readable files; root:wheel 2775 folders under /var/lib/resilience/shared, nothing from home folders; the Share screen lists every served file; the warning "anyone on this network can read it" before sharing | Implemented (vm-share: 0600 file 403, dotfile and symbolic link refused). An administrator who makes a private file world-readable there publishes it |
| T49 | A hostile machine on the network: fake announces pointing clients at the Internet or at themselves, forged packs, hostile pages or files, lynx used to escape | Announced and typed addresses accepted only in local ranges on an interface whose network holds them; names shown as claims with the announcing identity; packs from peers go through the store and the catalog hash (plain HTTP allowed for local addresses only); pages read through a loopback relay with restricted lynx `-localhost` (T33); folder files into ~/Received with the ADR 0020 rules and read-only viewers (T34, T42); records of heard machines bounded | Implemented: discovery (comms-messages: public address and markup in a name not recorded), joining (vm-share: a tampered pack from the server refused through the TUI, the library read through the relay with lynx on localhost; unit tests: addresses not on this computer's networks refused, curl without redirects or proxy). Shared folder (vm-share: copied into ~/Received 0600 at its listed size only, a refused file leaves nothing). Content other than packs is unverified, by design, and said so |
| T50 | The hotspot used as a gateway or listened to | `set-hotspot` auth_admin; WPA2 with a random password made by the helper (getrandom), shown once on the console, never in a log, a pipe or a command line (root-only file, NetworkManager in-memory profile 0600); every packet forwarded from or to the hotspot's card dropped although shared mode turns forwarding on (no NAT to the Internet); IPv6 off on it; DHCP/DNS (NetworkManager's dnsmasq) opened on that card only and only while it is on; `home.arpa` answered locally, nothing forwarded (`no-resolv`); never at boot (invariant) | Implemented (vm-hotspot, unit tests, security-invariants). dnsmasq parses DHCP and DNS from anyone with the password while it is on |
| T51 | The graphical edition (ADR 0026) adds attack surface or a way around the console's protections: a fake password dialog, input or screen read by another program, the GUI's run requests altered, no way out to a text console, hostile input to the compositor | cage shows one client (the GUI, then foot on top while a program runs); no X11/Xwayland, no other Wayland client is started; the compositor runs as the user in the tty1 logind session, never root, software rendering (no GPU driver code in the compositor process); passwords are still typed only into pkttyagent/age/gpg/passwd running in foot, exactly as on the console (no password dialog of our own yet); run requests are JSON in a new 0600 file in the user's 0700 `$XDG_RUNTIME_DIR` (O_EXCL, deleted at once) and carry no secrets, so changing one gains nothing over running the program directly; `-s` keeps Ctrl+Alt+F2..F6 (text logins, TUI); three quick GUI failures fall back to the TUI | Implemented (vm-gui). libinput, wlroots and the fonts' parsers handle local input and bundled files only |
| T33 | Shell or Internet access from the offline library for Ultra-Stable accounts | lynx with `-restrictions=… -localhost` (no shell escape, no external programs, no typed address, no remote host); kiwix-serve on 127.0.0.1 only, `--blockexternal`, one file, stopped with the browser | Implemented (vm-packs: `!` refused, no shell process, loopback only). Usability measure, not a sandbox |

## 5. Known gaps (honest list)

- Local messaging (ADR 0018, opt-in): every local account can read the
  key of the machine's Reticulum instance, by design, and can therefore
  disturb it (drop its paths). Anyone on the local network sees that the
  machine runs Reticulum and can send it packets: the service is
  unprivileged, sandboxed by systemd and limited to 256 MiB, but
  Reticulum's own parsing of hostile packets has not been reviewed by us.
  Wi-Fi networks with client isolation block its multicast.
  The mesh network (ADR 0021), when someone turns it on, is open: anyone
  in radio range with BATMAN-adv can join it, see the computer and send
  it frames, which the kernel's batman_adv code parses.
  Files in messages (ADR 0020) are accepted from any address, verified
  or not, within 64 MB per account; their names and content are the
  sender's choice.

- **Serving to the local network** (ADR 0025; server side, discovery, joining and the shared folder implemented; the hotspot not yet). Other local accounts can use a user's relay port while it is open: they reach the same peer they could reach directly.
  A machine that shares runs servers for strangers: whoever is on the
  chosen network (a café's Wi-Fi, a campus, the open mesh, a hotspot
  whose password was given out) can read everything shared and send
  requests that nginx, and through it kiwix-serve, parse. The defences
  are limiting *when* (only while an administrator wants it, never after
  a reboot unless declared), *where* (one interface), *what* (read-only,
  world-readable files under one folder, packs, the library) and *how
  much* (per-address limits, a memory cap). Confidentiality of shared
  content is not a goal: it is public by design. Integrity is
  guaranteed only for packs (catalog hashes). Library pages, pages and
  folder files from a peer are as trustworthy as the peer, which cannot
  be identified beyond a claimed name and a Reticulum identity. Someone
  on the network can answer at a peer's address first (ARP or neighbour
  spoofing); for packs this only wastes the download. An open network can
  be flooded; the limits keep the machine usable, not the service up.
  Accepted after the independent review (ADR 0025): accounts in the
  `networkmanager` group (Stable, Advanced) can read and change the
  running hotspot's profile, password included, through NetworkManager;
  nginx's listing of the shared folder shows names and sizes of private
  files (they cannot be fetched); a partial pack left by a nearby
  computer that stopped early makes a later download of that pack fail
  its hash at the end (nothing wrong is installed); announces go out on
  every network Reticulum uses.

- The privileged helper offers `set-mode`, `set-time` and
  `set-default-generation` so far. The time zone can only be changed in the
  Nix configuration.
- Making an older generation the default brings back its older software,
  including fixed vulnerabilities (ADR 0011); the confirmation says so, and
  only an administrator's password allows it.
- Ultra-Stable accounts keep NetworkManager's `network-control` (they can
  disconnect any connection, including an administrator's Ethernet) and
  `settings.modify.own` (profiles for themselves with any settings, e.g. a
  static address or cloned MAC). The TUI creates only plain Wi-Fi profiles.
- The TUI holds a typed Wi-Fi password in memory until nmcli has read it;
  wiping our copies is best effort.
- The audit log is append-only by convention (root writes with O_APPEND);
  root can still edit it. No remote/WORM log copy.
- pkexec is setuid and its generic action allows any program with an
  administrator's password (equivalent to that administrator's sudo).
- Full-disk encryption only when installed with encryption on (LUKS2;
  the default); the live system and the boot partition are not
  encrypted. No Secure Boot integration.
- Launched third-party programs may contain their own shell escapes; this
  is acceptable only because accounts have no privileges (T2).
- Resource packs are opened by libzim/kiwix-serve and rendered by lynx as
  the user; a pack that matches the catalog is trusted to the degree the
  catalog is. `resilience-library open` also opens any `.zim` file the user
  names (their own risk, as with any document). While a pack is open, other
  local accounts can read it through the loopback port (public content).
- The pack catalog ages with the system: Kiwix deletes old files, so some
  packs can then only be installed from a stick until a system update.
- User namespaces stay available on machines with Stable or Advanced
  accounts (the Nix sandbox needs them), so Ultra-Stable accounts there can
  create them too. Ultra-Stable-only machines, including the live system,
  refuse them to everyone (ADR 0012). The mainline kernel has no
  `unprivileged_userns_clone` switch.
- Backup checksums are not signatures: someone who can rewrite a stick can
  replace both files. age's authentication still detects tampering at restore
  time, but cannot tell an attacker's validly encrypted archive (made with a
  known passphrase) from yours.
- Backups have no rotation or retention policy; old backups stay until the
  user deletes them from the stick.
- Not reviewed by an independent security auditor.
