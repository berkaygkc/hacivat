#!/bin/sh
# dev-run.sh <binary> [args...] — the macOS cargo runner (R7, P35).
#
# `.cargo/config.toml` sets this as the runner for aarch64- and x86_64-apple-darwin, so it
# wraps every `cargo run`, `cargo test` and `cargo xtask` on macOS, CI's included.
#
# It signs the binary with the stable local "Hacivat Dev" identity and the identifier
# dev.hacivat.app, so the macOS Keychain recognises each rebuild as the same app and does not
# prompt again for the `hacivat-dev/...` entries. Create the identity once with
# `scripts/dev-sign.sh --create-identity`.
#
# It never fails: when the identity is missing, or codesign fails or hangs for any reason (an
# untrusted certificate, a key-access prompt nobody answers), it prints one hint on stderr and
# runs the binary unsigned.

IDENTITY="Hacivat Dev"
SIGN_TIMEOUT=20

if [ "$#" -eq 0 ]; then
    echo "dev-run.sh: usage: dev-run.sh <binary> [args...]" >&2
    exit 2
fi
bin=$1
shift

hint() {
    echo "dev-run.sh: $1; running unsigned (create the identity once: scripts/dev-sign.sh --create-identity)" >&2
}

# Runs codesign, killing it after SIGN_TIMEOUT seconds. Succeeds only if codesign does.
sign() {
    codesign --force --sign "$IDENTITY" --identifier dev.hacivat.app "$bin" >/dev/null 2>&1 &
    cs=$!
    (
        trap 'kill "$nap" 2>/dev/null; exit 0' TERM
        sleep "$SIGN_TIMEOUT" &
        nap=$!
        wait "$nap"
        kill "$cs" 2>/dev/null
    ) >/dev/null 2>&1 &
    watchdog=$!
    # stderr off: the shell's own "Terminated" job notice would be a second line.
    wait "$cs" 2>/dev/null
    status=$?
    kill "$watchdog" 2>/dev/null
    wait "$watchdog" 2>/dev/null
    return "$status"
}

if [ "$(uname -s)" = "Darwin" ]; then
    if ! command -v codesign >/dev/null 2>&1 || ! command -v security >/dev/null 2>&1; then
        hint "codesign or security not found"
    elif ! security find-identity -p codesigning 2>/dev/null | grep -F "\"$IDENTITY\"" >/dev/null 2>&1; then
        hint "no \"$IDENTITY\" code-signing identity"
    elif ! sign; then
        hint "codesign with \"$IDENTITY\" failed (is the certificate trusted for code signing?)"
    fi
fi

exec "$bin" "$@"
