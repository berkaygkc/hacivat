#!/bin/sh
# `sshd` (default): generate a fresh host key (regen-hostkey.sh publishes its public half as
# /artifacts/ssh-host-<HV_SSHD_ID>.pub), trust the generated client keys, and exec sshd as
# PID 1. `keygen`: write the client key pairs once, then stay up so the healthcheck can see
# them. Nothing private is committed: every key here is generated at startup. The client
# private keys are world-readable (0644) because on Linux CI the runner user is not root;
# they only open throwaway localhost test containers.
# HV_ARTIFACTS, HV_HOME_SSH and HV_SSHD_BIN override paths for the shell test.
set -eu

HERE="$(dirname "$0")"
ARTIFACTS="${HV_ARTIFACTS:-/artifacts}"
HOME_SSH="${HV_HOME_SSH:-/home/hv_ssh/.ssh}"
SSHD_BIN="${HV_SSHD_BIN:-/usr/sbin/sshd}"
PASSPHRASE=hv_test_pw
PLAIN="$ARTIFACTS/ssh-client-ed25519"
ENC="$ARTIFACTS/ssh-client-ed25519-enc"

case "${1:-sshd}" in
keygen)
    if [ ! -f "$PLAIN.pub" ] || [ ! -f "$ENC.pub" ]; then
        rm -f "$PLAIN" "$PLAIN.pub" "$ENC" "$ENC.pub"
        ssh-keygen -q -t ed25519 -N "" -C hv-test-client -f "$PLAIN"
        ssh-keygen -q -t ed25519 -N "$PASSPHRASE" -C hv-test-client-enc -f "$ENC"
        chmod 644 "$PLAIN" "$ENC" "$PLAIN.pub" "$ENC.pub"
    fi
    # `init: true` in compose gives this a PID 1 that forwards SIGTERM.
    exec sleep infinity
    ;;
sshd)
    # A fresh host key at every start: a restarted container still holds the old one.
    HV_NO_SIGNAL=1 "$HERE/regen-hostkey.sh"
    cat "$PLAIN.pub" "$ENC.pub" > "$HOME_SSH/authorized_keys"
    chown hv_ssh:hv_ssh "$HOME_SSH/authorized_keys"
    chmod 600 "$HOME_SSH/authorized_keys"
    exec "$SSHD_BIN" -D -e
    ;;
*)
    exec "$@"
    ;;
esac
