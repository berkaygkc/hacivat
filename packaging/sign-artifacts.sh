#!/usr/bin/env bash
# Run on the PO's Mac (L13): minisign the Windows and Linux packages that CI built unsigned, and
# write SHA-256 checksums. Refuses an unencrypted update key before touching anything.
# usage: packaging/sign-artifacts.sh <dir-with-downloaded-ci-packages>
# Env: UPDATE_KEY (default ~/.hacivat-keys/update.key); minisign prompts for the key password.
set -euo pipefail
DIR="${1:?usage: sign-artifacts.sh <dir>}"
[ -d "$DIR" ] || { echo "not a directory: $DIR" >&2; exit 1; }
DIR="$(cd "$DIR" && pwd)"
cd "$(dirname "$0")/.."
ROOT="$PWD"
# shellcheck source=packaging/common.sh
. "$ROOT/packaging/common.sh"

require_encrypted_key "$ROOT"

# Downloaded CI artifacts keep their per-artifact subdirectories, so search recursively.
FILES=()
while IFS= read -r f; do FILES+=("$f"); done < <(
  find "$DIR" -type f \( -name '*-setup.exe' -o -name '*.AppImage' -o -name '*.deb' \
    -o -name '*.dmg' -o -name '*.tar.gz' \) | sort)
[ "${#FILES[@]}" -gt 0 ] || { echo "no packages found under $DIR" >&2; exit 1; }

SUMS="$DIR/SHA256SUMS"
: > "$SUMS"
for f in "${FILES[@]}"; do
  minisign -S -s "$UPDATE_KEY_PATH" -m "$f" -x "$f.minisig" -t "hacivat $(basename "$f")"
  sha256_of "$f" >> "$SUMS"
done
echo "signed ${#FILES[@]} package(s); checksums in $SUMS"
