# shellcheck shell=bash
# Shared helpers for the packaging scripts. Source it; do not execute it.
#   . "$(dirname "$0")/common.sh"
# Nothing here reads or writes a secret: the update key stays on the PO's Mac (L13).

# The passworded minisign update key, defined once. UPDATE_KEY overrides it for tests.
UPDATE_KEY_PATH="${UPDATE_KEY:-$HOME/.hacivat-keys/update.key}"

# truthy <value>: succeeds for 1, true or yes (any case). Everything else, including empty, fails.
truthy() {
  case "$(printf '%s' "${1:-}" | tr '[:upper:]' '[:lower:]')" in
    1 | true | yes) return 0 ;;
    *) return 1 ;;
  esac
}

# probe_run <label> <command...>: runs the command with its output captured. On success the
# output is discarded; on failure it is printed to stderr (kept, not swallowed) and the
# command's status is returned. The captured output is also left in PROBE_OUTPUT.
probe_run() {
  local label="$1"
  shift
  local status=0
  PROBE_OUTPUT="$("$@" 2>&1)" || status=$?
  if [ "$status" -ne 0 ]; then
    echo "$label failed (exit $status):" >&2
    printf '%s\n' "$PROBE_OUTPUT" >&2
  fi
  return "$status"
}

# require_encrypted_key <root>: refuses a missing or unencrypted update key via xtask (L13).
require_encrypted_key() {
  [ -f "$UPDATE_KEY_PATH" ] || {
    echo "update key not found: $UPDATE_KEY_PATH" >&2
    return 1
  }
  (cd "$1" && cargo run --quiet --locked -p xtask -- check-update-key "$UPDATE_KEY_PATH")
}

# sha256_of <file>: prints "<hex>  <name>" using whichever tool the host has.
sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    (cd "$(dirname "$1")" && sha256sum "$(basename "$1")")
  else
    (cd "$(dirname "$1")" && shasum -a 256 "$(basename "$1")")
  fi
}
