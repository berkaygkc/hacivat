#!/bin/sh
# Replaces the host key with a fresh one and republishes its public half as
# /artifacts/ssh-host-<HV_SSHD_ID>.pub. Called by the entrypoint at every start (a restarted
# container still holds the old key, and ssh-keygen refuses to overwrite it) and by hand
# (`docker exec m2a-sshd-a /regen-hostkey.sh`), which also makes the running sshd (PID 1)
# re-read its key with SIGHUP. HV_NO_SIGNAL=1 skips the signal; HV_SSH_DIR and HV_ARTIFACTS
# override the paths (the shell test uses them).
set -eu

: "${HV_SSHD_ID:?HV_SSHD_ID is required}"
SSH_DIR="${HV_SSH_DIR:-/etc/ssh}"
ARTIFACTS="${HV_ARTIFACTS:-/artifacts}"
KEY="$SSH_DIR/ssh_host_ed25519_key"

rm -f "$KEY" "$KEY.pub"
ssh-keygen -q -t ed25519 -N "" -f "$KEY"
cp "$KEY.pub" "$ARTIFACTS/ssh-host-$HV_SSHD_ID.pub"
chmod 644 "$ARTIFACTS/ssh-host-$HV_SSHD_ID.pub"

if [ -z "${HV_NO_SIGNAL:-}" ]; then
    kill -HUP 1
fi
