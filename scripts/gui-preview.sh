#!/bin/sh
# Screenshot the GUI on this machine without a display: headless cage
# (pixman renderer) + grim. Development only (ADR 0026); no root.
#   scripts/gui-preview.sh OUT.png [WIDTHxHEIGHT] [KEYS...]
# KEYS: wtype arguments run one after another before the shot, e.g.
#   scripts/gui-preview.sh s.png 1366x768 'k' '-k Return'   (each item
#   is split on spaces and given to wtype; 1 s between items).
set -eu
out=${1:?usage: gui-preview.sh OUT.png [WxH]}
size=${2:-1366x768}
p() { nix build --no-link --print-out-paths --inputs-from . "nixpkgs#$1" | tail -1; }
cage=$(p cage); grim=$(p grim); foot=$(p foot); wtype=$(p wtype)
wl=$(p wayland.out); xkb=$(p libxkbcommon.out)
fonts=$(mktemp -d)
inter=$(p inter)/share/fonts/truetype; dv=$(p dejavu_fonts)/share/fonts/truetype
for f in "$inter/InterVariable.ttf" "$dv/DejaVuSans.ttf" "$dv/DejaVuSansMono.ttf" "$dv/DejaVuSansMono-Bold.ttf"; do
  ln -s "$(readlink -f "$f")" "$fonts/"
done
cargo build -q -p resilience-gui
gui=$PWD/target/debug/resilience-gui
run=$(mktemp -d); chmod 700 "$run"
# The handbook from this checkout (a system has it under /run/current-system).
# A stand-in nmcli with sample Wi-Fi networks (the host may have none).
cat > "$run/nmcli" <<'NM'
#!/bin/sh
case "$*" in
  *"TYPE device"*) echo wifi; echo ethernet ;;
  *"radio wifi"*) echo enabled ;;
  *"connection show"*) echo 'Home:5a1d-0001:802-11-wireless:yes' ;;
  *"wifi list"*) printf '%s\n' '*:Home:WPA2:82' ' :Cafe Libre::64' ' :Library 2nd floor:WPA2 WPA3:41' ' :Campus:WPA2 802.1X:30' ' :Neighbour:WPA2:12' ;;
  *) exit 1 ;;
esac
NM
chmod +x "$run/nmcli"
printf 'handbook_dir = "%s"\n\n[tools]\nnmcli = "%s"\n' "$PWD/handbook" "$run/nmcli" > "$run/system.toml"
export RESILIENCE_SYSTEM_CONFIG="$run/system.toml"
# A sample resource pack catalog (nothing is downloaded in a preview).
mkdir -p "$run/packs"
z=$(printf '%064d' 0)
cat > "$run/packs.toml" <<PK
dir = "$run/packs"
store_user = "resilience-packs"
store = "/bin/false"
pkexec = "/bin/false"
pkttyagent = "/bin/false"
curl = "/bin/false"
sha256sum = "/bin/false"
kiwix_serve = "/bin/false"
browser = ["/bin/false"]
session_shell = "/bin/sh"
PK
for e in "wikimed|Medical encyclopedia|Wikipedia's medicine articles, with pictures.|health|en|wikimed_2026-10.zim|2147483648" \
         "wikihow|Practical guides|How-to guides for everyday tasks.|practical|en|wikihow_2026-09.zim|9663676416" \
         "gutenberg|Free books|Project Gutenberg's public-domain books.|encyclopedia|fr|gutenberg_fr_2026-08.zim|5368709120"; do
  IFS='|' read -r id t d c l f n <<EOF2
$e
EOF2
  printf '[[pack]]\nid = "%s"\ntitle = "%s"\ndescription = "%s"\ncategory = "%s"\nlanguage = "%s"\nfile = "%s"\nsize = %s\nsha256 = "%s"\nurls = ["https://example.org/%s"]\nlicense = "CC-BY-SA-4.0"\nsource = "sample"\n' "$id" "$t" "$d" "$c" "$l" "$f" "$n" "$z" "$f" >> "$run/packs.toml"
done
: > "$run/packs/wikihow_2026-09.zim"
export RESILIENCE_PACKS_CONFIG="$run/packs.toml"
# A small home folder (Find files indexes it): a few sample files.
mkdir -p "$run/home/Documents" "$run/home/Notes"
cp LICENSE README.md "$run/home/Documents/" 2>/dev/null || true
cp handbook/*.md "$run/home/Notes/"
cp assets/logo/resilience-logo.png "$run/home/Documents/" 2>/dev/null || true
export HOME="$run/home"
export XDG_RUNTIME_DIR="$run" WLR_BACKENDS=headless WLR_RENDERER=pixman \
  WLR_HEADLESS_OUTPUTS=1 WLR_LIBINPUT_NO_DEVICES=1 \
  LD_LIBRARY_PATH="$wl/lib:$xkb/lib" RESILIENCE_GUI_FONTS="$fonts" \
  RESILIENCE_TERMINAL="$foot/bin/foot" RESILIENCE_DEBUG=1
unset WAYLAND_DISPLAY DISPLAY
"$cage/bin/cage" -- "$gui" >"$run/cage.log" 2>&1 &
pid=$!
sock=""
for _ in $(seq 50); do
  sock=$(ls "$run" | grep '^wayland-[0-9]*$' | head -1 || true)
  [ -n "$sock" ] && break; sleep 0.2
done
sleep 6
shift; [ $# -gt 0 ] && shift
for k in "$@"; do
  # shellcheck disable=SC2086
  WAYLAND_DISPLAY=$sock "$wtype/bin/wtype" -d 60 $k
  sleep 1
done
WAYLAND_DISPLAY=$sock "$grim/bin/grim" "$out" || { cat "$run/cage.log"; }
kill "$pid" 2>/dev/null || true
wait "$pid" 2>/dev/null || true
cp "$run/cage.log" "$out.log" 2>/dev/null || true
rm -rf "$fonts" "$run"
echo "$out ($size)"
