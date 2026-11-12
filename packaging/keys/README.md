# Update signing keys

`update.pub` is the minisign public key compiled into the app (`include_str!`). The private key
never enters the repo: it lives at `~/.hacivat-keys/update.key` (the `*.key` pattern is also
git-ignored).

Generate a keypair (spike: no password, so packaging scripts run unattended; use a password and a
secret store for production):

    mkdir -p ~/.hacivat-keys && chmod 700 ~/.hacivat-keys
    minisign -G -W -p packaging/keys/update.pub -s ~/.hacivat-keys/update.key

Rotating the key means shipping a build that embeds the new `.pub` before signing updates with the
new private key. Verify an artifact by hand with `minisign -Vm <file> -p packaging/keys/update.pub`.
