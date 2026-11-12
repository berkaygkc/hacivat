# Update signing keys

`update.pub` is the minisign public key compiled into the app (`include_str!`). The private key
never enters the repo or CI: it lives at `~/.hacivat-keys/update.key` on the PO's Mac and is always
password-protected (the `*.key` pattern is also ignored by the repo). CI builds unsigned packages
and holds no signing secret (L13); releases are signed locally with `packaging/package-macos.sh`
(macOS) and `packaging/sign-artifacts.sh` (the Windows and Linux packages downloaded from CI).

Generate a keypair (no `-W`: minisign prompts for a password):

    mkdir -p ~/.hacivat-keys && chmod 700 ~/.hacivat-keys
    minisign -G -p packaging/keys/update.pub -s ~/.hacivat-keys/update.key

Both release scripts run `cargo xtask check-update-key ~/.hacivat-keys/update.key` first and refuse
an unencrypted key. Check one by hand the same way.

Rotating the key means shipping a build that embeds the new `.pub` before signing updates with the
new private key. Verify an artifact by hand with `minisign -Vm <file> -p packaging/keys/update.pub`.

Note: the `update.pub` committed during the M0 spike belongs to a passwordless spike key. Before the
first release, generate the passworded pair above and commit the new `update.pub` (PO step).
