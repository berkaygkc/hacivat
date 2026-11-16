# sshd-a / sshd-b and the ssh-client-keys generator (Task 16): one small image. Alpine's
# OpenSSH, user `hv_ssh` (password and generated key logins), forwarding on. iproute2 gives
# `ss` (the gate counts established tunnels); /regen-hostkey.sh rotates the host key.
FROM alpine:3.20@sha256:d9e853e87e55526f6b2917df91a2115c36dd7c696a35be12163d44e6e2a4b6bc
RUN apk add --no-cache openssh iproute2 \
    && adduser -D -s /bin/sh hv_ssh \
    && echo "hv_ssh:hv_ssh_pw" | chpasswd \
    && mkdir -p /home/hv_ssh/.ssh /artifacts \
    && chown -R hv_ssh:hv_ssh /home/hv_ssh/.ssh
COPY sshd_config /etc/ssh/sshd_config
COPY entrypoint.sh /entrypoint.sh
COPY regen-hostkey.sh /regen-hostkey.sh
RUN chmod 755 /entrypoint.sh /regen-hostkey.sh
ENTRYPOINT ["/entrypoint.sh"]
CMD ["sshd"]
