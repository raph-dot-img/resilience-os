# Architecture

Version 0.4.0. Decisions and their rationale live in `docs/decisions/`.

## 1. Layers

```
┌──────────────────────────────────────────────────────────────────────┐
│ Launched programs (nano, mc, htop, games, nmtui ...)  fixed argv      │
├──────────────────────────────────────────────────────────────────────┤
│ resilience TUI (ratatui)  |  resilience-gui (iced, ADR 0026, planned) │
│   rendering + input translation only    unprivileged, per user        │
├──────────────────────────────────────────────────────────────────────┤
│ resilience-shell (Rust library, no UI toolkit)                        │
│   app.rs state machine → Effect { None | Quit | Run }  help.rs        │
│   input.rs (keys)  list.rs (selection)  fmt.rs (local time)           │
├──────────────────────────────────────────────────────────────────────┤
│ resilience-core (Rust library)                                        │
│   status network mode catalog config handbook notes launch proc       │
│   admin (pure mode-change rules)  recovery (generations, report)      │
├──────────────────────────────────────────────────────────────────────┤
│ resilience-admin (root, via pkexec/sudo/activation) — typed ops only  │
├──────────────────────────────────────────────────────────────────────┤
│ resilience-session (login shell) — restart on crash, refuse -c        │
├──────────────────────────────────────────────────────────────────────┤
│ NixOS 26.05: systemd, getty, logind, polkit, sudo, NetworkManager,    │
│ nix-daemon — configured by nix/modules (declarative, reproducible)    │
├──────────────────────────────────────────────────────────────────────┤
│ Linux kernel (LTS from nixpkgs), firmware (redistributable, ISO)      │
└──────────────────────────────────────────────────────────────────────┘
```

## 2. Boot and session sequence

1. Bootloader (GRUB on the ISO in text mode; systemd-boot planned for
   installed UEFI systems), with all previous generations listed.
2. Kernel at `loglevel=3`: errors still visible. systemd `show_status=auto`:
   silent when healthy, shows failing or slow units.
3. multi-user.target. No display manager. NetworkManager starts but sends
   nothing until an interface is configured; no connectivity check.
4. getty on tty1–6 with the Resilience banner. tty1 may autologin an
   Ultra-Stable account (live ISO); other ttys always prompt.
5. Login shell `resilience-session` → `resilience`.
6. The TUI shows system health from `systemctl is-system-running` and
   failed units on the main screen, so a degraded boot is never hidden.

## 3. Configuration flow

```
nix/modules/*.nix ──(nixos-rebuild / ISO build)──► /etc/resilience/system.toml
resilience.apps.<id> ─(build-time executable check)► /etc/resilience/apps.toml
handbook/*.md ───────► /run/current-system/sw/share/resilience/handbook
user preferences ────► ~/.config/resilience/settings.toml   (0600, theme only)
resilience.users ────► /etc/resilience/admin.toml (helper config, root-owned)
mode changes ────────► /var/lib/resilience/modes.toml (root, 0644; overrides)
audit ───────────────► /var/log/resilience/audit.log (root:wheel 0640) + journal
diagnostic report ───► ~/Diagnostics/resilience-report-*.txt (0600)
TUI log ─────────────► ~/.local/state/resilience/tui.log    (rotated at 1 MiB)
```
The TUI only *reads* system files. Anything that changes the system goes
through Nix (an administrator rebuilding) or through the typed privileged
helper `resilience-admin` (ADR 0007):

```
TUI ──confirm──► pkttyagent (user) + pkexec resilience-admin set-mode U M
                     │ polkit org.resilience.admin.set-mode: auth_admin (wheel password)
                     ▼
               resilience-admin (root): validate → modes.toml → gpasswd → audit
                                         → loginctl terminate-user (demotion)

TUI ──confirm──► pkttyagent + pkexec resilience-admin set-time YYYY-MM-DDTHH:MM:SSZ
                     │ polkit org.resilience.admin.set-time: auth_admin (ADR 0009)
                     ▼
               resilience-admin (root): strict UTC, 2025 ≤ t < 2100 → audit
                                         → timedatectl set-time @t (clock + RTC)
TUI Settings ──► pkexec resilience-admin set-keymap LAYOUT (also at session start)
                     │ polkit org.resilience.admin.set-keymap: yes for the active session (ADR 0017)
                     ▼
               resilience-admin (root): LAYOUT in admin.toml keymaps → loadkeys -q -- LAYOUT → audit
               (getty@ ExecStartPre loads the system layout before every login prompt)
TUI ──confirm──► pkttyagent + pkexec resilience-admin set-default-generation N
                     │ polkit org.resilience.admin.set-default-generation: auth_admin (ADR 0011)
                     ▼
               resilience-admin (root): N exists, not default, closure in /nix/store
                 → nix-env --switch-generation N → <gen N>/bin/switch-to-configuration boot
                 (clean env; on failure the profile is switched back) → audit
boot/switch: activation `users` resets groups → `resilienceModes` → apply-state
```

## 4. Security modes

Mode = Unix group membership, computed at runtime (ADR 0003):

| | ultra-stable | stable | advanced |
|---|---|---|---|
| Groups | — | resilience-stable, networkmanager | + wheel |
| Shell in TUI | no | yes (bash, unprivileged) | yes |
| sudo | no | no | yes, password |
| Nix daemon | refused | yes | yes |
| Network config | own Wi-Fi profiles from the TUI only (polkit allowlist, ADR 0010) | nmtui, nmcli (networkmanager group) | nmtui, nmcli |
| Restricted variants (htop --readonly, ncdu -rr, nano --restricted) | yes | no | no |

User namespaces (ADR 0012) are a machine-wide kernel setting: refused to
everyone when every account is Ultra-Stable (live system), available
otherwise (Nix sandbox).

## 5. Networking state model

Passive: `/sys/class/net`, `getifaddrs`, `/proc/net/{route,ipv6_route,arp}`,
`/etc/resolv.conf` → OFFLINE / LOCAL LINK / ROUTE UP. Active probe on request
only, bounded by timeouts, skipped without a route (ADR 0004).

Wi-Fi (ADR 0010), as the user:
```
TUI Wi-Fi networks ── nmcli -t device wifi list (cached; rescan in a thread)
  Enter ──► password dialog (masked) ──► nmcli connection add … permissions user:U
            (no secret) ──► thread: nmcli --wait 45 connection up uuid X
            passwd-file /dev/stdin  ◄── "802-11-wireless-security.psk:…" (pipe)
  failure of a new profile ──► nmcli connection delete uuid X
polkit: no networkmanager/wheel group ⇒ only scan, network-control,
        settings.modify.own, enable-disable-wifi; all else NO
```

## 6. Resource packs and the offline library (ADR 0013)

```
/etc/resilience/packs.toml (root, from resilience.packs.catalog: file, size, SHA-256, https URLs)
TUI Resource packs ── tick [+]/[-] ── d (route needed) / u (stick via udisks)
  └─► resilience-library change [--from M] [--remove F]… [--get ID]…   (as the user)
        pkttyagent + pkexec --user resilience-packs resilience-pack-store change …
        store ──stdout──► "send ID OFFSET" | "skip ID REASON" … "ready"
        library: curl -q --proto =https [--continue-at N] | file on the stick
        library ──stdin──► [u32 len][bytes]… [0] per pack
        store: append .staging/F.part + sha256sum (old bytes + new) ──► match: rename
               into /var/lib/resilience/packs (resilience-packs, 0644); mismatch: delete;
               early end: keep for next time ──stdout──► "done ID …"
TUI Offline library ── Enter / s WORDS
  └─► resilience-library open FILE [--search WORDS]
        kiwix-serve --address=127.0.0.1 --port=<free> --blockexternal --attachToProcess
        lynx -restrictions=<all but redirects> -localhost http://localhost:P/content/<book>/ ; stopped
```

## 6b. Cryptography (ADR 0015)

```
TUI Security & cryptography ── picker (Find & view files) / "Encrypt for" checklist / checksum box
  └─► resilience-crypto OP [--passphrase | --me | --to NAME]… [SUM] [FILE]   (as the user, foreground)
        encrypt:  age --encrypt (-p | -r KEY…) FILE  ──► .NAME.age.partial ──RENAME_NOREPLACE──► NAME.age
        decrypt:  age --decrypt [-i own/encryption.age] FILE ──► ~/Decrypted/.partial ──► ~/Decrypted/NAME
        messages: text on stdin ──► age -a ──► ~/Messages/…;  read: age -d ──► memory ──► less (LESSSECURE=1)
        keys:     age-keygen | age -p -a ──► own/encryption.age (secret only in the pipe); minisign -G
        sign/verify: minisign -S / -V -P <contact's key, matched by key number>
        checksums: sha256sum/sha512sum/b2sum/… (LC_ALL=C), --check --strict for lists
  passphrases: typed into age/minisign on /dev/tty, never through the TUI or our process
```

## 7. Planned components (not implemented)

| Component | Phase | Sketch |
|---|---|---|
| More `resilience-admin` operations | 8 | ApplyUpdate: one polkit action, same validation/audit pattern |
| OpenPGP (GnuPG) workflows | 4 | interoperability: import/verify/decrypt/encrypt, pinentry-curses |
| Encrypted USB sticks (LUKS) | later | unlock existing LUKS sticks in the media screens |
| Maps | 5 | MBTiles/GeoPackage rendering with block characters |
| Local messaging | 6 | evaluate Reticulum (rns, in nixpkgs) before writing anything |
| Installer | 8 | text installer, explicit disk confirmation, optional LUKS |
| Offline updates | 8 | `nix copy` closures + signed manifest on removable media |

## 8. Resource budget (to be measured — see ROADMAP)

Targets, not measurements: idle RAM < 200 MiB without optional services; TUI
RSS < 15 MiB; boot to TUI < 20 s on SSD; no periodic wakeups from our code
except the 250 ms input poll and a 5 s status refresh.
