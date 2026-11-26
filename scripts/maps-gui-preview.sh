#!/bin/sh
# Screenshot the graphical map viewer without a display: headless cage
# (pixman) + grim, like gui-preview.sh. Development only; no root.
#   scripts/maps-gui-preview.sh OUT.png "VIEWER ARGS" [KEYS...]
# KEYS: wtype arguments run one after another before the shot (each item
# is split on spaces; 1 s between items).
set -eu
out=${1:?usage: maps-gui-preview.sh OUT.png "ARGS" [KEYS...]}
vargs=${2:-}
p() { nix build --no-link --print-out-paths --inputs-from . "nixpkgs#$1" | tail -1; }
cage=$(p cage); grim=$(p grim); wtype=$(p wtype)
wl=$(p wayland.out); xkb=$(p libxkbcommon.out)
fonts=$(mktemp -d)
inter=$(p inter)/share/fonts/truetype; dv=$(p dejavu_fonts)/share/fonts/truetype
for f in "$inter/InterVariable.ttf" "$dv/DejaVuSans.ttf" "$dv/DejaVuSansMono.ttf"; do
  ln -s "$(readlink -f "$f")" "$fonts/"
done
cargo build -q --release -p resilience-maps-gui
bin=$PWD/target/release/resilience-maps-gui
run=$(mktemp -d); chmod 700 "$run"
mkdir -p "$run/home"
export HOME="$run/home" XDG_RUNTIME_DIR="$run" WLR_BACKENDS=headless WLR_RENDERER=pixman \
  WLR_HEADLESS_OUTPUTS=1 WLR_LIBINPUT_NO_DEVICES=1 \
  LD_LIBRARY_PATH="$wl/lib:$xkb/lib" RESILIENCE_GUI_FONTS="$fonts"
unset WAYLAND_DISPLAY DISPLAY
# shellcheck disable=SC2086
"$cage/bin/cage" -- "$bin" $vargs >"$run/cage.log" 2>&1 &
pid=$!
sock=""
for _ in $(seq 50); do
  sock=$(ls "$run" | grep '^wayland-[0-9]*$' | head -1 || true)
  [ -n "$sock" ] && break; sleep 0.2
done
sleep 5
shift 2
for k in "$@"; do
  # shellcheck disable=SC2086
  WAYLAND_DISPLAY=$sock "$wtype/bin/wtype" -d 60 $k
  sleep 1
done
WAYLAND_DISPLAY=$sock "$grim/bin/grim" "$out" || cat "$run/cage.log"
kill "$pid" 2>/dev/null || true
wait "$pid" 2>/dev/null || true
cp "$run/cage.log" "$out.log" 2>/dev/null || true
rm -rf "$fonts" "$run"
echo "$out"
