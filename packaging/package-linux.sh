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
# ## m3b-ci (controller fix): `#linux-only# ` lines (the square PNG icons the AppImage requires)
# are enabled for Linux only; the macOS and Windows scripts leave them as comments.
sed -e "s/@VERSION@/$VERSION/" -e '/@SIGNING_IDENTITY@/d' -e 's/^#linux-only# //' \
  packaging/Packager.toml > "$CFG"
grep -q '^icons = ' "$CFG" || { echo "package-linux.sh: no icons line in $CFG" >&2; exit 1; }
cargo packager -c "$CFG" --formats appimage,deb --out-dir "$OUT"
echo "done (unsigned): $OUT"
