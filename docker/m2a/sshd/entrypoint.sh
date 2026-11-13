#!/bin/sh
# `sshd` (default): generate the host key, publish its public half as
# /artifacts/ssh-host-<HV_SSHD_ID>.pub, trust the generated client keys, run sshd.
# `keygen`: write the client key pairs once, then stay up so the healthcheck can see them.
# Nothing private is committed: every key here is generated at startup. The client private
# keys are world-readable (0644) because on Linux CI the runner user is not root; they only
# open throwaway localhost test containers.
set -eu

PASSPHRASE=hv_test_pw
PLAIN=/artifacts/ssh-client-ed25519
ENC=/artifacts/ssh-client-ed25519-enc

case "${1:-sshd}" in
keygen)
    if [ ! -f "$PLAIN.pub" ] || [ ! -f "$ENC.pub" ]; then
        rm -f "$PLAIN" "$PLAIN.pub" "$ENC" "$ENC.pub"
        ssh-keygen -q -t ed25519 -N "" -C hv-test-client -f "$PLAIN"
        ssh-keygen -q -t ed25519 -N "$PASSPHRASE" -C hv-test-client-enc -f "$ENC"
        chmod 644 "$PLAIN" "$ENC" "$PLAIN.pub" "$ENC.pub"
    fi
    exec sleep infinity
    ;;
sshd)
    : "${HV_SSHD_ID:?HV_SSHD_ID is required}"
    ssh-keygen -q -t ed25519 -N "" -f /etc/ssh/ssh_host_ed25519_key
    cp /etc/ssh/ssh_host_ed25519_key.pub "/artifacts/ssh-host-$HV_SSHD_ID.pub"
    chmod 644 "/artifacts/ssh-host-$HV_SSHD_ID.pub"
    cat "$PLAIN.pub" "$ENC.pub" > /home/hv_ssh/.ssh/authorized_keys
    chown hv_ssh:hv_ssh /home/hv_ssh/.ssh/authorized_keys
    chmod 600 /home/hv_ssh/.ssh/authorized_keys
    exec /usr/sbin/sshd -D -e
    ;;
*)
    exec "$@"
    ;;
esac
