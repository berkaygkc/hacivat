#!/usr/bin/env bash
# Build, sign (Developer ID, hardened runtime), notarize + staple when a notarytool profile exists,
# and minisign-sign the universal macOS bundle of spike-harman.
# usage: packaging/package-macos.sh <version>
#
# Env (all optional; nothing is ever written into the repo):
#   APPLE_SIGNING_IDENTITY   codesign identity (default: the first Developer ID Application in the keychain)
#   NOTARY_PROFILE           notarytool keychain profile (default: hacivat-notary)
#   UPDATE_KEY               minisign secret key (default: ~/.hacivat-keys/update.key)
# Output: target/packager/<version>/ (.dmg, .app.tar.gz + .minisig)
set -euo pipefail
VERSION="${1:?usage: package-macos.sh <version>}"
cd "$(dirname "$0")/.."
ROOT="$PWD"
PROFILE="${NOTARY_PROFILE:-hacivat-notary}"
KEY="${UPDATE_KEY:-$HOME/.hacivat-keys/update.key}"
IDENT="${APPLE_SIGNING_IDENTITY:-$(security find-identity -v -p codesigning | awk '/Developer ID Application/ {print $2; exit}')}"
[ -n "$IDENT" ] || { echo "no Developer ID Application identity found" >&2; exit 1; }
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
OUT="$TARGET_DIR/packager/$VERSION"
BIN="$OUT"   # cargo-packager resolves binaries relative to the out dir
rm -rf "$OUT"; mkdir -p "$OUT"

export HACIVAT_SPIKE_VERSION="$VERSION"
for t in aarch64-apple-darwin x86_64-apple-darwin; do
  cargo build --release -p spike-harman --target "$t"
done
lipo -create -output "$BIN/spike-harman" \
  "$TARGET_DIR/aarch64-apple-darwin/release/spike-harman" \
  "$TARGET_DIR/x86_64-apple-darwin/release/spike-harman"

CFG="$ROOT/packaging/.Packager.generated.toml"   # git-ignored; holds the identity hash
trap 'rm -f "$CFG"' EXIT
sed -e "s/@VERSION@/$VERSION/" -e "s/@SIGNING_IDENTITY@/$IDENT/" packaging/Packager.toml > "$CFG"

NOTARIZE=0
if xcrun notarytool history --keychain-profile "$PROFILE" >/dev/null 2>&1; then
  NOTARIZE=1
  export APPLE_KEYCHAIN_PROFILE="$PROFILE"   # cargo-packager then notarizes + staples the .app
else
  echo "WARNING: notarytool profile '$PROFILE' missing: building signed but NOT notarized" >&2
fi
cargo packager -c "$CFG" --formats app,dmg --out-dir "$OUT"

DMG="$(ls "$OUT"/*.dmg)"
if [ "$NOTARIZE" = 1 ]; then
  xcrun notarytool submit "$DMG" --keychain-profile "$PROFILE" --wait
  xcrun stapler staple "$DMG"
  xcrun stapler validate "$DMG"
fi

# Update artifact: the .app in a tarball (the updater skips the first path component), minisigned.
TARBALL="$OUT/hacivat-spike_${VERSION}_universal.app.tar.gz"
COPYFILE_DISABLE=1 tar -czf "$TARBALL" -C "$OUT" "Hacivat Spike.app"
minisign -S -s "$KEY" -m "$TARBALL" -x "$TARBALL.minisig" -t "hacivat-spike $VERSION"
codesign --verify --deep --strict --verbose=2 "$OUT/Hacivat Spike.app"
echo "done: $OUT"
