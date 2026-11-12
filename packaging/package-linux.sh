#!/usr/bin/env bash
# Linux: AppImage + .deb (cargo-packager 0.11 has no .rpm format), each with a minisign .minisig.
# usage: packaging/package-linux.sh <version>        (run on the Linux host; not yet exercised)
set -euo pipefail
VERSION="${1:?usage: package-linux.sh <version>}"
cd "$(dirname "$0")/.."
ROOT="$PWD"
KEY="${UPDATE_KEY:-$HOME/.hacivat-keys/update.key}"
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
OUT="$TARGET_DIR/packager/$VERSION"
rm -rf "$OUT"; mkdir -p "$OUT"
export HACIVAT_SPIKE_VERSION="$VERSION"
cargo build --release -p spike-harman
cp "$TARGET_DIR/release/spike-harman" "$OUT/"
CFG="$ROOT/packaging/.Packager.generated.toml"
trap 'rm -f "$CFG"' EXIT
sed -e "s/@VERSION@/$VERSION/" packaging/Packager.toml > "$CFG"
cargo packager -c "$CFG" --formats appimage,deb --out-dir "$OUT"
for f in "$OUT"/*.AppImage "$OUT"/*.deb; do
  minisign -S -s "$KEY" -m "$f" -x "$f.minisig" -t "hacivat-spike $VERSION"
done
echo "done: $OUT"
