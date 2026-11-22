# ADR 0026: Graphical edition (GUI)

Status: accepted (2026-11-21, by the owner).

## Context

Up to now Resilience has been terminal-only (CLAUDE.md §1: "No X11, Wayland,
desktop or WM"). The owner wants a graphical edition from 0.7.0 on, offered
next to the terminal one. It should be very light, minimalist, modern and
above all **very intuitive**. The offline assistant (Phase 11) moves to
0.8.0.

The guiding rule does not change: nothing may depend on infrastructure
that might be gone. Neither do the priorities (security > correctness >
reliability > offline > maintainability > low resources > usability).

## Decision

### 1. One full-screen app, no desktop

Kernel KMS → **cage** (Wayland kiosk compositor, 0.3.0 in the pinned nixpkgs,
MIT) → **`resilience-gui`** (new Rust crate, **iced**, MIT). No window
manager, panel, desktop or file manager: the GUI *is* the system, the same
way the TUI is today. No X11, no Xwayland.

- Rendering: iced's software renderer (tiny-skia) by default. wgpu only if
  measurements show a gain on real hardware without new risks.
- Terminal programs (vim, lynx, htop, man, …) run in **foot** (1.27.0,
  MIT) inside cage, with the same fixed argv as in the TUI. Closing
  the program brings the GUI back.
- Font: Inter (OFL, in nixpkgs), bundled. Nothing is fetched at runtime.

### 2. One behaviour, two views

The TUI's state machine (`crates/resilience-tui/src/app.rs`, `App::handle_key`
→ `Effect`s) moves into a new UI-independent crate **`resilience-shell`**
(menus, catalog, confirmations, flows, effects). `resilience-tui` and
`resilience-gui` only render it and turn their input into its events.
The flow tests live in `resilience-shell` and cover both front ends. This
refactor must leave every existing TUI test green.

### 3. Security model unchanged

- The GUI runs as the user and launches the same unprivileged binaries
  (`resilience-library`, `-crypto`, `-backup`, `-maps`, …) and the same
  `resilience-admin` via pkexec + polkit, one action per operation.
- `pkttyagent` cannot be used without a tty. We write our own small
  polkit agent in the GUI: a password dialog that drives the setuid
  `polkit-agent-helper-1` (no PAM code of our own, no crypto of our own).
  It shows the action and its arguments exactly as polkit reports them.
  It gets an independent review before release, plus a THREAT_MODEL.md
  section (a fake dialog, the focus being taken).
- cage + one client: there is no other client that could read input or
  the screen. The compositor runs as the user, never root (seatd/logind seat).

### 4. Intuitive design rules

- Home = a grid of large tiles (Knowledge, Maps, Messages, Files, Security,
  Network, Settings, Help) and a search box that finds apps, handbook pages,
  files and places.
- At most two levels deep. "Back" is always visible in the same spot.
  Every TUI key still works, and mouse, touchpad and touch screens work too.
- Plain words. Confirmations keep Cancel as the default. A status bar shows
  network, battery, time and the sharing indicator (ADR 0025).
- One accent colour, light and dark themes, large hit targets, readable at
  1366x768. No animation beyond short fades.
- Maps: the existing MVT decoder and style (ADR 0016), drawn as vectors.

### 5. Editions

`resilience.interface = "tui" | "gui"` (interface.nix, default "tui").
Systems: `live-standard` (TUI), `live-standard-gui`, `live-minimal` (TUI
only). In the GUI edition, tty1 starts `resilience-session`, which starts cage +
GUI (and restarts after a crash, as today). tty2–6 keep their text logins,
and the TUI stays installed there for recovery. The installer (ADR 0022)
and offline updates (ADR 0023) gain the GUI variants.

### 6. Resource targets (to be measured, not promised)

- Idle RAM for cage + GUI: ≤ 150 MB.
- Boots and works with 2 GiB.
- ISO size: ≤ +60 MB over the TUI one.
- Has to work without a GPU (QEMU `-vga std`, virtio-gpu) and on the
  owner's laptops (ThinkPad L15, Yoga 11e: integrated Intel graphics).

Measured (2026-11-21, vm-gui and the installed UEFI systems): idle RSS
43.4 MiB (cage 18.2, GUI 25.2); system closure +58.6 MiB over the TUI
edition (cage without Xwayland, fonts copied: 4 files).

## Consequences

- CLAUDE.md §1 now says "terminal-first, with an optional graphical edition".
- A new component set (cage, wlroots, Mesa, foot, Inter, iced) goes into
  COMPONENTS.md with licences. Its attack surface (Mesa, libinput) goes
  into THREAT_MODEL.md.
- VM tests: `vm-gui` reads the screen via QEMU screendump (no vcs),
  drives it with keys, and compares against screenshots we look at.

## Slices

1. This ADR + CLAUDE.md/ROADMAP updates.
2. Extract `resilience-shell` from the TUI (pure refactor, every test green).
3. `resilience-gui` skeleton (home, search, launcher, settings),
   `nix/modules/gui.nix` (cage session), `live-standard-gui`, `vm-gui`.
4. GUI polkit agent + mode / keyboard / clock flows. Then a review.
5. Port the areas one by one: files, library/packs, messages, community
   network, crypto, backups, recovery, installer, then maps last (vector renderer).
6. Installer/update variants, handbook, measurements, release 0.7.0.

## Alternatives not chosen

- A full desktop (GNOME, XFCE, sway): too heavy, and against "minimalist and intuitive".
- Slint: polished tooling, but its GPL / royalty-free terms have to be
  checked against Hippocratic 2.1. egui: lighter, but looks less modern.
- Kiosk web UI (browser + local server): far heavier and a much larger attack surface.
