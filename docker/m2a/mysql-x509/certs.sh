#!/bin/bash
# mysql-x509-certs (T13 carry, m2b-fix-f2): a throwaway test CA, the certificate `mysql84-x509`
# serves and a client certificate for its `REQUIRE X509` account `hv_x509`, written into the
# artifacts directory (never committed), then `exec "$@"` (compose: `sleep infinity`, so the
# healthcheck can see them). Runs on the pinned mysql:8.4 image for its OpenSSL 3.
#
# - Kept while every file is there and both leaves stay valid for another day; otherwise all
#   are generated again. The CA's private key never leaves the container's temp directory.
# - Leaves are valid 397 days (Apple's cap for a server leaf), the CA ten years.
# - The private keys are PKCS #8; `mysql-x509-client-key-enc.pem` is the client key encrypted
#   with the throwaway passphrase (PBES2, PBKDF2-HMAC-SHA-256, AES-256-CBC). All files are 0644:
#   the server runs as `mysql`, and on Linux CI the runner user is not root. They only open a
#   throwaway localhost test container.
# HV_ARTIFACTS overrides the output directory.
set -euo pipefail

ARTIFACTS="${HV_ARTIFACTS:-/artifacts}"
PASSPHRASE=hv_test_pw
P=mysql-x509
FILES="$P-ca.pem $P-server.pem $P-server-key.pem $P-client.pem $P-client-key.pem $P-client-key-enc.pem"
# ## m4b-t19-docker (named exception): the healthcheck waits for this marker, written only
# after the checks at the end, so mysql84-x509 never starts on a set that is not ready.
READY="$ARTIFACTS/.$P-ready"
rm -f "$READY"
# ## m4b-t19-docker

fresh() {
    for f in $FILES; do
        [ -s "$ARTIFACTS/$f" ] || return 1
    done
    for leaf in server client; do
        openssl x509 -checkend 86400 -noout -in "$ARTIFACTS/$P-$leaf.pem" >/dev/null || return 1
    done
    # ## m4b-t19-docker (named exception; CI 44990057141, 45091594060): a kept set must also
    # be whole and of one generation. Both failures came on a run that restored the job's cache,
    # where mysqld could not load the kept CA ("SSL_CTX_set_default_verify_paths failed" is
    # MySQL's text for a failed `SSL_CTX_load_verify_locations`), served no TLS, and the
    # healthcheck's caching_sha2 sign-in then never passed. So: the CA must parse, both leaves
    # must verify against it, and each private key must be its certificate's.
    openssl verify -CAfile "$ARTIFACTS/$P-ca.pem" \
        "$ARTIFACTS/$P-server.pem" "$ARTIFACTS/$P-client.pem" >/dev/null 2>&1 || return 1
    local cert key
    for leaf in server client; do
        cert="$(openssl x509 -noout -pubkey -in "$ARTIFACTS/$P-$leaf.pem" 2>/dev/null)" || return 1
        key="$(openssl pkey -pubout -in "$ARTIFACTS/$P-$leaf-key.pem" 2>/dev/null)" || return 1
        [ -n "$cert" ] && [ "$cert" = "$key" ] || return 1
    done
    # ## m4b-t19-docker
}

# One leaf signed by the CA in $W: `leaf <name> <subject> <extensions…>`.
leaf() {
    local name="$1" subject="$2"
    shift 2
    local ext=()
    for e in "$@"; do
        ext+=(-addext "$e")
    done
    openssl req -x509 -new -nodes -newkey rsa:2048 -days 397 \
        -subj "$subject" \
        -addext "basicConstraints=critical,CA:FALSE" \
        "${ext[@]}" \
        -CA "$W/ca.pem" -CAkey "$W/ca-key.pem" \
        -keyout "$W/$name-key.pem" -out "$W/$name.pem" 2>/dev/null
}

if ! fresh; then
    # Never a mix of two generations: the healthcheck waits for the whole new set.
    for f in $FILES; do
        rm -f "$ARTIFACTS/$f"
    done
    W="$(mktemp -d)"
    trap 'rm -rf "$W"' EXIT
    openssl req -x509 -new -nodes -newkey rsa:2048 -days 3650 \
        -subj "/CN=Hacivat test CA (mysql84-x509)" \
        -addext "basicConstraints=critical,CA:TRUE" \
        -addext "keyUsage=critical,keyCertSign,cRLSign" \
        -keyout "$W/ca-key.pem" -out "$W/ca.pem" 2>/dev/null
    leaf server "/CN=localhost" \
        "subjectAltName=DNS:localhost,IP:127.0.0.1,DNS:mysql84-x509" \
        "extendedKeyUsage=serverAuth"
    leaf client "/CN=hv_x509" "extendedKeyUsage=clientAuth"
    openssl pkcs8 -topk8 -v2 aes-256-cbc -v2prf hmacWithSHA256 \
        -in "$W/client-key.pem" -out "$W/client-key-enc.pem" -passout "pass:$PASSPHRASE"
    # Renamed into place within the artifacts directory, so the healthcheck never sees half a
    # file; the CA certificate goes last.
    for f in server server-key client client-key client-key-enc ca; do
        cp "$W/$f.pem" "$ARTIFACTS/.$P-$f.pem.tmp"
        chmod 644 "$ARTIFACTS/.$P-$f.pem.tmp"
        mv "$ARTIFACTS/.$P-$f.pem.tmp" "$ARTIFACTS/$P-$f.pem"
    done
    # `exec` runs no EXIT trap: the CA key goes now.
    rm -rf "$W"
    trap - EXIT
    echo "mysql-x509-certs: generated a new set"
else
    echo "mysql-x509-certs: kept the existing set"
fi

# ## m4b-t19-docker (named exception): a kept set restored from a cache may carry other modes,
# and mysqld reads these files as `mysql`, never root. Every file 0644 and the folder
# traversable, then the server's own view checked (the image's `gosu`, as its entrypoint
# uses): a set mysqld cannot read stops here, named in this container's log, instead of a
# server that starts without TLS and never turns healthy.
chmod 755 "$ARTIFACTS"
for f in $FILES; do
    chmod 644 "$ARTIFACTS/$f"
done
openssl verify -CAfile "$ARTIFACTS/$P-ca.pem" "$ARTIFACTS/$P-server.pem" >/dev/null
if command -v gosu >/dev/null 2>&1; then
    for f in $FILES; do
        gosu mysql test -r "$ARTIFACTS/$f" || {
            echo "mysql-x509-certs: $f is not readable by the mysql user" >&2
            exit 1
        }
    done
fi
touch "$READY"
# ## m4b-t19-docker

exec "$@"
