#!/usr/bin/env bash
# Build the universal macOS DMG of Hacivat.
#
# usage: packaging/package-macos.sh <version>
#   (default)  an UNSIGNED DMG, exactly what the CI nightly produces.
#   RELEASE=1  PO's Mac only (L13): Developer ID sign, notarize, staple, then minisign the DMG and
#              the update tarball with the passworded key (minisign prompts for the password).
#
# Env (nothing is ever written into the repo):
#   RELEASE                  1 turns on signing, notarization and minisign
#   NOTARY_PROFILE           notarytool keychain profile (default: hacivat-notary)
#   APPLE_SIGNING_IDENTITY   codesign identity (default: the first Developer ID Application in the keychain)
#   REQUIRE_NOTARIZATION     1|true|yes makes an unusable notary profile fatal (default: on when RELEASE=1)
#   UPDATE_KEY               minisign secret key (default: ~/.hacivat-keys/update.key)
# Output: target/packager/<version>/Hacivat_<version>_universal.dmg
set -euo pipefail
VERSION="${1:?usage: package-macos.sh <version>}"
cd "$(dirname "$0")/.."
ROOT="$PWD"
# shellcheck source=packaging/common.sh
. "$ROOT/packaging/common.sh"

TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
OUT="$TARGET_DIR/packager/$VERSION"
PROFILE="${NOTARY_PROFILE:-hacivat-notary}"

IDENT=""
if truthy "${RELEASE:-0}"; then
  # Fail before the long build: the key must exist and be passworded, the identity must exist.
  require_encrypted_key "$ROOT"
  IDENT="${APPLE_SIGNING_IDENTITY:-$(security find-identity -v -p codesigning | awk '/Developer ID Application/ {print $2; exit}')}"
  [ -n "$IDENT" ] || { echo "no Developer ID Application identity found" >&2; exit 1; }
  REQUIRE_NOTARIZATION="${REQUIRE_NOTARIZATION:-1}"
fi

rm -rf "$OUT"; mkdir -p "$OUT"
for t in aarch64-apple-darwin x86_64-apple-darwin; do
  cargo build --release --locked -p hacivat --target "$t"
done
lipo -create -output "$OUT/hacivat" \
  "$TARGET_DIR/aarch64-apple-darwin/release/hacivat" \
  "$TARGET_DIR/x86_64-apple-darwin/release/hacivat"

CFG="$ROOT/packaging/.Packager.generated.toml"   # ignored by the repo; may hold the identity hash
trap 'rm -f "$CFG"' EXIT
if [ -n "$IDENT" ]; then
  sed -e "s/@VERSION@/$VERSION/" -e "s/@SIGNING_IDENTITY@/$IDENT/" packaging/Packager.toml > "$CFG"
else
  sed -e "s/@VERSION@/$VERSION/" -e '/@SIGNING_IDENTITY@/d' packaging/Packager.toml > "$CFG"
fi

NOTARIZE=0
if [ -n "$IDENT" ]; then
  if probe_run "notarytool profile '$PROFILE'" xcrun notarytool history --keychain-profile "$PROFILE"; then
    NOTARIZE=1
    export APPLE_KEYCHAIN_PROFILE="$PROFILE"   # cargo-packager then notarizes + staples the .app
  elif truthy "${REQUIRE_NOTARIZATION:-0}"; then
    echo "FATAL: notarization required but notarytool profile '$PROFILE' is unusable (output above)" >&2
    exit 1
  else
    echo "WARNING: building signed but NOT notarized" >&2
  fi
fi
cargo packager -c "$CFG" --formats app,dmg --out-dir "$OUT"

DMG="$OUT/Hacivat_${VERSION}_universal.dmg"
BUILT="$(ls "$OUT"/*.dmg)"
[ "$BUILT" = "$DMG" ] || mv "$BUILT" "$DMG"

if [ "$NOTARIZE" = 1 ]; then
  xcrun notarytool submit "$DMG" --keychain-profile "$PROFILE" --wait
  xcrun stapler staple "$DMG"
  xcrun stapler validate "$DMG"
fi

if [ -n "$IDENT" ]; then
  # The updater takes the .app in a tarball (it skips the first path component).
  TARBALL="$OUT/hacivat_${VERSION}_universal.app.tar.gz"
  COPYFILE_DISABLE=1 tar -czf "$TARBALL" -C "$OUT" "Hacivat.app"
  codesign --verify --deep --strict --verbose=2 "$OUT/Hacivat.app"
  require_encrypted_key "$ROOT"
  for f in "$DMG" "$TARBALL"; do
    minisign -S -s "$UPDATE_KEY_PATH" -m "$f" -x "$f.minisig" -t "hacivat $VERSION"
  done
else
  echo "unsigned build (set RELEASE=1 on the PO's Mac to sign, notarize and minisign)"
fi
echo "done: $DMG"
