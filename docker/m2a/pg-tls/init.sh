#!/usr/bin/env bash
# pg17-tls (R17): a self-signed server certificate and ssl=on, generated once at initdb.
# The entrypoint restarts the server after the init scripts, so the settings take effect
# before the healthcheck can pass. Plaintext connections stay allowed: `tls_default` is
# what the server offers, and the tests choose the client's sslmode.
set -e

# A leaf certificate (CA:FALSE, serverAuth, under the 398-day cap of Apple's verifier), not a
# CA: the platform verifiers reject a CA certificate used as the server's own.
openssl req -new -x509 -days 397 -nodes \
    -newkey rsa:2048 \
    -subj "/CN=localhost" \
    -addext "basicConstraints=critical,CA:FALSE" \
    -addext "extendedKeyUsage=serverAuth" \
    -addext "subjectAltName=DNS:localhost,IP:127.0.0.1" \
    -keyout "$PGDATA/server.key" \
    -out "$PGDATA/server.crt" 2>/dev/null
chmod 600 "$PGDATA/server.key"

cat >> "$PGDATA/postgresql.conf" <<'CONF'

# m2a pg17-tls
ssl = on
ssl_cert_file = 'server.crt'
ssl_key_file = 'server.key'
CONF
