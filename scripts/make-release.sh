#!/usr/bin/env bash
# Prepare a Resilience release for the download mirrors.
#
#   scripts/make-release.sh [options] VERSION OUTDIR
#   scripts/make-release.sh --resign OUTDIR/releases/VERSION [--sign KEY]
#
# Builds, from this source tree at tag vVERSION:
#
#   OUTDIR/
#   ├── README.txt                        how to download and check
#   ├── LATEST                            VERSION
#   ├── resilience-release.minisign.pub   the release signing key (public)
#   └── releases/VERSION/
#       ├── resilience-standard-VERSION-x86_64-linux.iso
#       ├── resilience-minimal-VERSION-x86_64-linux.iso        (--minimal)
#       ├── resilience-update-EDITION-VERSION.tar              (--update-key)
#       ├── resilience-VERSION-source.tar.zst                  (git archive)
#       ├── SHA256SUMS
#       └── SHA256SUMS.minisig                                 (--sign)
#
# Options:
#   --minimal            also publish the minimal edition
#   --gui                also publish the graphical edition (standard-gui)
#   --update-key FILE    also make each edition's offline update bundle
#                        (scripts/make-update-bundle.sh, Nix release key)
#   --sign FILE          sign SHA256SUMS with this minisign secret key
#                        (asks for its password on the terminal); without
#                        it, the command to sign is printed at the end
#   --allow-untagged     for trial runs: HEAD need not be tagged vVERSION
#
# --index VERSION OUTDIR only writes README.txt, LATEST (= VERSION) and the
# public key at the top of OUTDIR, for a tree of existing releases.
#
# --resign DIR rewrites SHA256SUMS (and signs it) for a folder filled by
# hand, e.g. older releases whose ISOs were already built.
#
# Runs as an ordinary user, never root. Heavy: builds the ISOs (do not run
# it alongside VM tests). Upload releases/ first and LATEST last, so a
# script never follows LATEST into a half-copied folder.
set -euo pipefail

die() { echo "make-release: $*" >&2; exit 1; }
say() { echo "== $*"; }

root=$(cd "$(dirname "$0")/.." && pwd)
pubkey=$root/keys/resilience-release.minisign.pub
[ -f "$pubkey" ] || die "missing $pubkey"

# Tools from the pinned nixpkgs, so the host needs only Nix.
tool() {
  local p paths
  # A package may have several outputs (zstd: bin, dev, man...).
  paths=$(nix build --no-link --print-out-paths --inputs-from "$root" "nixpkgs#$1") ||
    die "cannot build $1"
  for p in $paths; do
    [ -x "$p/bin/${2:-$1}" ] && { printf '%s/bin/%s' "$p" "${2:-$1}"; return; }
  done
  die "no bin/${2:-$1} in $1"
}

checksums_and_sign() { # DIR VERSION SIGNKEY
  local dir=$1 version=$2 sign=$3 minisign
  minisign=$(tool minisign)
  (
    cd "$dir"
    rm -f SHA256SUMS SHA256SUMS.minisig
    files=()
    for f in *; do
      [ -f "$f" ] && [ ! -L "$f" ] || die "$dir/$f is not a plain file"
      files+=("$f")
    done
    [ ${#files[@]} -gt 0 ] || die "$dir is empty"
    chmod 644 -- "${files[@]}"
    LC_ALL=C sha256sum -- "${files[@]}" >SHA256SUMS
    chmod 644 SHA256SUMS
  )
  say "SHA256SUMS:"
  cat "$dir/SHA256SUMS"
  if [ -n "$sign" ]; then
    [ -t 0 ] || die "signing asks for the key's password: run it from a terminal"
    say "Signing SHA256SUMS (minisign asks for the key's password)"
    "$minisign" -S -s "$sign" -m "$dir/SHA256SUMS" -x "$dir/SHA256SUMS.minisig" \
      -c "Resilience $version release checksums" \
      -t "Resilience $version: SHA256SUMS of releases/$version"
    chmod 644 "$dir/SHA256SUMS.minisig"
    "$minisign" -V -q -p "$pubkey" -m "$dir/SHA256SUMS" -x "$dir/SHA256SUMS.minisig" ||
      die "the signature does not verify with $pubkey: wrong secret key?"
    say "Signature checked with $pubkey"
  else
    say "Not signed. To sign, from a terminal:"
    echo "  $minisign -S -s SECRET_KEY -m '$dir/SHA256SUMS' -x '$dir/SHA256SUMS.minisig' \\"
    echo "    -t 'Resilience $version: SHA256SUMS of releases/$version'"
  fi
}

write_index() { # OUTDIR VERSION: the files at the top of the mirror
  local out=$1 version=$2
  cp -- "$pubkey" "$out/resilience-release.minisign.pub"
  chmod 644 "$out/resilience-release.minisign.pub"
  printf '%s\n' "$version" >"$out/LATEST"
  chmod 644 "$out/LATEST"
  cat >"$out/README.txt" <<EOF
Resilience OS: offline-first terminal Linux distribution for x86_64 PCs.
Source and documentation: https://github.com/raph-dot-img/resilience-os

releases/VERSION/   one folder per version, never changed once published
releases/torrents/  resilience_EDITION_VERSION.torrent: a whole release folder by
                    BitTorrent (hybrid v1+v2; this mirror is its web seed;
                    DHT, peer exchange and several trackers)
LATEST              the newest version, as one line

Each release folder holds:
  resilience-EDITION-VERSION-x86_64-linux.iso   live system and installer
  resilience-update-EDITION-VERSION.tar         offline update: extract it to
                                                the root of a USB stick, then
                                                Recovery > Update from a USB stick
  resilience-VERSION-source.tar.zst             source code of this version
  SHA256SUMS, SHA256SUMS.minisig                checksums, signed

Check a download before using it:

  minisign -V -p resilience-release.minisign.pub -m SHA256SUMS
  sha256sum -c --ignore-missing SHA256SUMS

The same check applies to a release downloaded by BitTorrent, from any peer:
the signed SHA256SUMS is part of the torrent.

The public key resilience-release.minisign.pub is also published in the
source repository (keys/). Compare the two copies: if they differ, do not
trust the download. Update bundles are also signed with a separate key that
installed systems check themselves.
EOF
  chmod 644 "$out/README.txt"
}

minimal=0 gui=0 updatekey="" sign="" untagged=0 resign="" index=0
while [ $# -gt 0 ]; do
  case $1 in
    --minimal) minimal=1 ;;
    --gui) gui=1 ;;
    --update-key) updatekey=${2:?}; shift ;;
    --sign) sign=${2:?}; shift ;;
    --allow-untagged) untagged=1 ;;
    --resign) resign=${2:?}; shift ;;
    --index) index=1 ;;
    -*) die "unknown option $1" ;;
    *) break ;;
  esac
  shift
done
[ -z "$sign" ] || [ -r "$sign" ] || die "cannot read the minisign key $sign"

if [ -n "$resign" ]; then
  [ $# -eq 0 ] || die "--resign takes only a folder"
  [ -d "$resign" ] || die "no folder $resign"
  checksums_and_sign "$(realpath "$resign")" "$(basename "$resign")" "$sign"
  exit 0
fi

if [ $index = 1 ]; then
  [ $# -eq 2 ] || die "usage: $0 --index VERSION OUTDIR"
  [ -d "$2/releases/$1" ] || die "no folder $2/releases/$1"
  write_index "$(realpath "$2")" "$1"
  exit 0
fi

[ $# -eq 2 ] || die "usage: $0 [--minimal] [--gui] [--update-key FILE] [--sign FILE] [--allow-untagged] VERSION OUTDIR"
version=$1 out=$(realpath -m "$2")
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "VERSION must look like 1.2.3"
dest=$out/releases/$version
[ ! -e "$dest" ] || die "$dest already exists"

cd "$root"
[ -z "$(git status --porcelain)" ] || die "the source tree has changes: commit or stash them first"
head=$(git rev-parse HEAD)
if tagged=$(git rev-parse -q --verify "refs/tags/v$version^{commit}"); then
  [ "$tagged" = "$head" ] || [ $untagged = 1 ] || die "tag v$version is not HEAD"
else
  [ $untagged = 1 ] || die "no tag v$version (use --allow-untagged for a trial)"
fi

editions=(standard)
[ $gui = 0 ] || editions+=(standard-gui)
[ $minimal = 0 ] || editions+=(minimal)
for ed in "${editions[@]}"; do
  v=$(nix eval --raw ".#nixosConfigurations.live-$ed.config.resilience.version")
  [ "$v" = "$version" ] || die "the $ed edition says version $v, not $version"
done

mkdir -p "$dest"
work=$out/.work-$version
rm -rf "$work"
mkdir -p "$work"
trap 'rm -rf "$work"' EXIT

for ed in "${editions[@]}"; do
  attr=iso
  case $ed in standard) ;; standard-gui) attr=iso-gui ;; *) attr=iso-$ed ;; esac
  say "Building the $ed ISO (long)"
  p=$(nix build --no-link --print-out-paths ".#$attr")
  name=resilience-$ed-$version-x86_64-linux.iso
  [ -f "$p/iso/$name" ] || die "expected $p/iso/$name"
  cp -- "$p/iso/$name" "$dest/$name"
done

if [ -n "$updatekey" ]; then
  for ed in "${editions[@]}"; do
    say "Making the $ed update bundle"
    "$root/scripts/make-update-bundle.sh" "$ed" "$updatekey" "$work/$ed"
    grep -qx "version = \"$version\"" "$work/$ed/resilience-update/resilience-update.toml" ||
      die "the $ed bundle is not version $version"
    tar -C "$work/$ed" --owner=0 --group=0 --numeric-owner -cf \
      "$dest/resilience-update-$ed-$version.tar" resilience-update
    rm -rf "${work:?}/${ed:?}"
  done
fi

say "Source archive"
zstd=$(tool zstd)
git archive --format=tar --prefix="resilience-$version/" "$head" |
  "$zstd" -q -19 -T2 -o "$dest/resilience-$version-source.tar.zst"

checksums_and_sign "$dest" "$version" "$sign"

write_index "$out" "$version"

say "Done: $dest"
ls -l "$dest"
echo "Upload releases/$version first, then LATEST, README.txt and the key."
