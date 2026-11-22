#!/usr/bin/env bash
# Make a signed offline update bundle (ADR 0023) from this source tree.
#
#   scripts/make-update-bundle.sh EDITION SECRET_KEY_FILE OUTDIR
#
# EDITION is standard, standard-gui or minimal. SECRET_KEY_FILE is the Resilience release
# key (`nix key generate-secret`), kept offline: it is read by Nix to sign
# and never copied. OUTDIR/resilience-update/ is created (it must not exist);
# copy that folder to the root of a USB stick.
#
# Runs as an ordinary user, no root. Builds the edition's four installed
# systems and its manifest, copies their closures into a file:// binary
# cache with every path signed, checks the result with the public key, and
# writes the unsigned pointer resilience-update.toml.
set -euo pipefail

die() { echo "make-update-bundle: $*" >&2; exit 1; }

[ $# -eq 3 ] || die "usage: $0 EDITION SECRET_KEY_FILE OUTDIR"
edition=$1 key=$2 out=$3
case $edition in standard | standard-gui | minimal) ;; *) die "EDITION is standard, standard-gui or minimal" ;; esac
[ -f "$key" ] && [ -r "$key" ] || die "cannot read the secret key $key"
# Yours, and no access for group or others (0600, 0400, or 0700 on a FAT
# volume such as a VeraCrypt container mounted with fmask=0077).
if [ "$(stat -c %u "$key")" != "$(id -u)" ] || (( 8#$(stat -c %a "$key") & 8#077 )); then
  die "the secret key $key must be yours and readable by you only (chmod 600)"
fi
root=$(cd "$(dirname "$0")/.." && pwd)
dest=$(realpath -m "$out")/resilience-update
[ ! -e "$dest" ] || die "$dest already exists"

# Percent-encode a path for a file:// URL (stick labels may have spaces).
urlpath() {
  local s=$1 out="" c i
  for ((i = 0; i < ${#s}; i++)); do
    c=${s:i:1}
    case $c in [A-Za-z0-9/._~-]) out+=$c ;; *) out+=$(printf '%%%02X' "'$c") ;; esac
  done
  printf '%s' "$out"
}

pub=$(nix key convert-secret-to-public <"$key")
grep -qF "\"$pub\"" "$root/nix/release-keys.nix" ||
  die "the public key $pub is not in nix/release-keys.nix: installed systems would refuse this bundle"

echo "Building the manifest and the four installed systems of $edition..."
manifest=$(nix build --no-link --print-out-paths "$root#update-manifest-$edition")
systems=()
for v in uefi-plain uefi-luks bios-plain bios-luks; do
  t=$(nix build --no-link --print-out-paths \
    "$root#nixosConfigurations.installed-$edition-$v.config.system.build.toplevel")
  grep -qF "installed-$edition-$v = \"$t\"" "$manifest" ||
    die "the manifest does not list $t for $v"
  systems+=("$t")
done
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$manifest")
[ -n "$version" ] || die "no version in $manifest"

mkdir -p "$dest"
url="file://$(urlpath "$dest")"
echo "Copying and signing into $dest (several GB)..."
nix --extra-experimental-features nix-command copy \
  --to "$url?secret-key=$(urlpath "$(realpath "$key")")&compression=zstd" \
  "$manifest" "${systems[@]}"

echo "Checking every path (contents and signature)..."
nix --extra-experimental-features nix-command store verify --store "$url" \
  --option trusted-public-keys "$pub" --sigs-needed 1 --all

cat >"$dest/resilience-update.toml" <<EOF
# Resilience offline update (ADR 0023). This file is not signed: it only
# says which store path is the manifest. Everything else is checked.
format = 1
edition = "$edition"
version = "$version"
manifest = "$manifest"
EOF
echo "Done: Resilience $edition $version, $(du -sh "$dest" | cut -f1) in $dest"
