#!/usr/bin/env bash
# Linux: UNSIGNED AppImage + .deb (cargo-packager 0.11 has no .rpm format). CI never signs (L13);
# release packages are minisigned afterwards on the PO's Mac with packaging/sign-artifacts.sh.
# usage: packaging/package-linux.sh <version>
set -euo pipefail
VERSION="${1:?usage: package-linux.sh <version>}"
cd "$(dirname "$0")/.."
ROOT="$PWD"
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
OUT="$TARGET_DIR/packager/$VERSION"
rm -rf "$OUT"; mkdir -p "$OUT"
cargo build --release --locked -p hacivat
cp "$TARGET_DIR/release/hacivat" "$OUT/"
CFG="$ROOT/packaging/.Packager.generated.toml"
trap 'rm -f "$CFG"' EXIT
sed -e "s/@VERSION@/$VERSION/" -e '/@SIGNING_IDENTITY@/d' packaging/Packager.toml > "$CFG"
cargo packager -c "$CFG" --formats appimage,deb --out-dir "$OUT"
echo "done (unsigned): $OUT"
