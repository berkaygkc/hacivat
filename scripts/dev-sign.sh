#!/bin/sh
# dev-sign.sh --create-identity — makes the stable "Hacivat Dev" code-signing identity (R7, P35).
#
# Run it once per Mac, from an interactive terminal. It is idempotent: a second run only reports
# that the identity exists, or finishes a trust step an earlier run did not complete.
#
# What it does:
#   1. Makes a self-signed certificate with the Code Signing extended key usage, CN "Hacivat
#      Dev", and its RSA key, using the system openssl (/usr/bin/openssl).
#   2. Imports both into the login keychain (`security import`), allowing /usr/bin/codesign
#      to use the key.
#   3. Lets Apple tools and codesign use the key without a dialog
#      (`security set-key-partition-list`). This asks for your LOGIN KEYCHAIN PASSWORD in the
#      terminal.
#   4. Trusts the certificate for code signing (`security add-trusted-cert -p codeSign`). macOS
#      shows ONE authorization dialog for the change to your certificate trust settings.
#
# Afterwards `scripts/dev-run.sh` (the macOS cargo runner) signs every dev binary with this
# identity and the identifier dev.hacivat.app, so the Keychain stops prompting for the
# `hacivat-dev/...` entries after each rebuild.
#
# The identity is for local development only. To remove it, delete the "Hacivat Dev"
# certificate and key in Keychain Access (login keychain).

set -eu

NAME="Hacivat Dev"
OPENSSL=/usr/bin/openssl

usage() {
    echo "usage: scripts/dev-sign.sh --create-identity" >&2
    exit 2
}

[ "$#" -eq 1 ] && [ "$1" = "--create-identity" ] || usage

if [ "$(uname -s)" != "Darwin" ]; then
    echo "dev-sign.sh: macOS only; nothing to do." >&2
    exit 0
fi

KEYCHAIN=$(security login-keychain | sed -e 's/^[[:space:]]*"//' -e 's/"[[:space:]]*$//')

has_identity() {
    security find-identity -p codesigning "$KEYCHAIN" 2>/dev/null | grep -F "\"$NAME\"" >/dev/null
}

has_valid_identity() {
    security find-identity -v -p codesigning "$KEYCHAIN" 2>/dev/null | grep -F "\"$NAME\"" >/dev/null
}

if has_valid_identity; then
    echo "dev-sign.sh: \"$NAME\" exists and is trusted for code signing; nothing to do."
    exit 0
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM

if has_identity; then
    echo "dev-sign.sh: \"$NAME\" exists but is not trusted yet; finishing the trust step."
    security find-certificate -c "$NAME" -p "$KEYCHAIN" >"$tmp/cert.pem"
else
    echo "dev-sign.sh: creating \"$NAME\" in $KEYCHAIN"
    cat >"$tmp/req.cnf" <<EOF
[req]
distinguished_name = dn
x509_extensions = ext
prompt = no

[dn]
CN = $NAME

[ext]
basicConstraints = critical, CA:false
keyUsage = critical, digitalSignature
extendedKeyUsage = critical, codeSigning
subjectKeyIdentifier = hash
EOF
    "$OPENSSL" req -x509 -newkey rsa:2048 -nodes -days 3650 -sha256 \
        -config "$tmp/req.cnf" -keyout "$tmp/key.pem" -out "$tmp/cert.pem" 2>/dev/null

    # A one-off transport password for the PKCS#12 file, which is deleted on exit.
    HV_P12_PASS=$(LC_ALL=C tr -dc 'A-Za-z0-9' </dev/urandom | head -c 32)
    export HV_P12_PASS
    "$OPENSSL" pkcs12 -export -inkey "$tmp/key.pem" -in "$tmp/cert.pem" -name "$NAME" \
        -out "$tmp/identity.p12" -passout env:HV_P12_PASS
    rm -f "$tmp/key.pem"

    security import "$tmp/identity.p12" -k "$KEYCHAIN" -f pkcs12 -P "$HV_P12_PASS" \
        -T /usr/bin/codesign
    unset HV_P12_PASS

    echo "dev-sign.sh: allowing codesign to use the key without a dialog."
    echo "dev-sign.sh: enter your login keychain password when asked."
    if ! security set-key-partition-list -S apple-tool:,apple:,codesign: -s -l "$NAME" \
        "$KEYCHAIN" >/dev/null; then
        echo "dev-sign.sh: could not set the key's partition list; the first codesign may" \
            "show a dialog: choose Always Allow." >&2
    fi
fi

echo "dev-sign.sh: trusting the certificate for code signing (macOS asks once to authorize)."
security add-trusted-cert -p codeSign -k "$KEYCHAIN" "$tmp/cert.pem"

if has_valid_identity; then
    echo "dev-sign.sh: done. \"$NAME\" is a valid code-signing identity."
else
    echo "dev-sign.sh: \"$NAME\" is still not valid for code signing; check it in" \
        "Keychain Access (Trust > Code Signing: Always Trust)." >&2
    exit 1
fi
