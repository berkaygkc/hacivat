### Core texts shared with other binaries: database and error explanations.
### Append-only: each task adds a block headed `## <task slug>`.

## m2a-t10-errors

# Connect errors (HV-CONN-03): a heading (`-title`) and an explanation (`-body`) per error class,
# addressed as `ErrClass::fluent_stem()` plus `-title` or `-body`. Every message is built from
# what Hacivat sent (the user, the host, the database), never from the server's text, which the
# error surface shows verbatim in its own field. Every variable is text: the port and the version
# arrive pre-formatted, so a port never gets grouping separators.
# `$engine` is the engine's product name (`DbmsKind::display_name`, never translated); the value
# `SQLite` selects the wording about a file instead of a host.

# network.dns
# Variables:
#   $host (String) - the host name Hacivat tried to look up.
error-conn-network-dns-title = { $host } couldn't be found.
error-conn-network-dns-body = This Mac couldn't look up the host name. Check the spelling, or connect to your VPN if the host is on a private network.

# network.refused
# Variables:
#   $host (String) - the host Hacivat connected to.
#   $port (String) - the port Hacivat connected to, e.g. "5432" (never grouped).
error-conn-network-refused-title = { $host } refused the connection on port { $port }.
error-conn-network-refused-body = The host is reachable, but nothing accepted the connection on that port. Check the port, and that the database server is running and accepts network connections.

# network.timeout
# Variables:
#   $host (String) - the host Hacivat connected to.
error-conn-network-timeout-title = { $host } didn't answer in time.
error-conn-network-timeout-body = The connection attempt ran out of time. The server may be busy, or a firewall may be dropping the connection. Try again, or check the host and port.

# network.reset
# Variables:
#   $host (String) - the host Hacivat connected to.
error-conn-network-reset-title = { $host } closed the connection.
error-conn-network-reset-body = The connection was cut before Hacivat could sign in. The server may have restarted, or a firewall or proxy may have closed the connection. Try again.

# network.unreachable
# Variables:
#   $host (String) - the host Hacivat connected to.
error-conn-network-unreachable-title = { $host } can't be reached from this Mac.
error-conn-network-unreachable-body = There is no network route to the host. Check your network connection, or connect to your VPN if the host is on a private network.

# network.ipv6_unavailable
# Variables:
#   $host (String) - the host Hacivat connected to.
error-conn-network-ipv6-unavailable-title = { $host } can only be reached over IPv6.
error-conn-network-ipv6-unavailable-body = The host has only IPv6 addresses, and this network doesn't carry IPv6. Try another network, or use the host's IPv4 address if it has one.

# tls.handshake
# Variables:
#   $host (String) - the host Hacivat connected to.
error-conn-tls-handshake-title = The secure connection to { $host } couldn't be set up.
error-conn-tls-handshake-body = Hacivat and the server couldn't agree on encryption. Check the connection's TLS settings; the technical details have the reason.

# tls.untrusted
# Variables:
#   $host (String) - the host Hacivat connected to.
error-conn-tls-untrusted-title = { $host } sent a certificate this Mac doesn't trust.
# Review: PO
# m2b-t14-pipeline (named exception): back to the approved mock text (`err-tls-b`), now that
# the certificate review exists (B6).
error-conn-tls-untrusted-body = It may be self-signed or issued by your company's own CA. Review it before you trust it.

# tls.hostname
# Variables:
#   $host (String) - the host Hacivat connected to.
error-conn-tls-hostname-title = The certificate from { $host } is for a different name.
error-conn-tls-hostname-body = The server's certificate doesn't cover the name Hacivat connected to. Connect with a name the certificate lists, such as the server's full domain name instead of an IP address or an alias.

# tls.protocol: an unacceptable TLS version, or TLS data whose signature doesn't verify.
# Variables:
#   $host (String) - the host Hacivat connected to.
error-conn-tls-protocol-title = The secure connection to { $host } failed a safety check.
error-conn-tls-protocol-body = The server uses a TLS version Hacivat doesn't accept, or sent encrypted data that didn't verify. Hacivat needs TLS 1.2 or later. If the server is up to date, something on the network may be interfering with the connection.

# tls.client_cert_required
# Variables:
#   $host (String) - the host Hacivat connected to.
error-conn-tls-client-cert-required-title = { $host } asks for a client certificate.
error-conn-tls-client-cert-required-body = The server only accepts connections that present a certificate of their own. Add the client certificate and its key in the connection's TLS settings.

# tls.not_supported_by_server
# Variables:
#   $host (String) - the host Hacivat connected to.
error-conn-tls-not-supported-by-server-title = { $host } doesn't offer encrypted connections.
error-conn-tls-not-supported-by-server-body = This connection requires TLS, and the server doesn't support it. Turn on TLS on the server, or change the connection's TLS mode if the network is private and trusted.

# auth.invalid
# Variables:
#   $user (String) - the user name Hacivat sent.
error-conn-auth-invalid-title = The server rejected the password for { $user }.
error-conn-auth-invalid-body = The password may have changed or expired. Hacivat stops after 3 attempts so the account isn't locked.

# auth.expired
# Variables:
#   $user (String) - the user name Hacivat sent.
error-conn-auth-expired-title = The password for { $user } has expired.
error-conn-auth-expired-body = The server wants a new password before this account can sign in. Change it with another tool or ask your database administrator, then enter the new password here.

# auth.locked
# Variables:
#   $user (String) - the user name Hacivat sent.
error-conn-auth-locked-title = The account { $user } is locked.
error-conn-auth-locked-body = The server doesn't let this account sign in, often after too many failed attempts or because an administrator locked it. Ask your database administrator to unlock it.

# auth.method_unsupported
# Variables:
#   $user (String) - the user name Hacivat sent.
error-conn-auth-method-unsupported-title = The server wants { $user } to sign in with a method Hacivat can't use.
error-conn-auth-method-unsupported-body = The account uses an authentication method Hacivat doesn't support over this connection. Some methods only work over an encrypted connection: try turning on TLS, or ask your database administrator which method the account uses.

# authz.database
# Variables:
#   $user (String) - the user name Hacivat sent.
#   $db (String) - the database Hacivat asked for.
error-conn-authz-database-title = { $user } isn't allowed to open { $db }.
error-conn-authz-database-body = The server doesn't let this account connect to this database from here. Ask your database administrator for access, or pick a database the account can open.

# config.invalid
# Variables:
#   $engine (String) - the engine's product name; `SQLite` selects the file wording.
#   $name (String) - the connection's name.
#   $file (String) - the SQLite file's path.
error-conn-config-invalid-title = { $engine ->
        [SQLite] { $file } isn't a SQLite database.
       *[other] The settings of { $name } can't be used.
    }
# Variables:
#   $engine (String) - the engine's product name; `SQLite` selects the file wording.
error-conn-config-invalid-body = { $engine ->
        [SQLite] The file exists, but it doesn't look like a database. Check that you picked the right file; it may be another kind of file, or damaged.
       *[other] Something in the connection can't be used as it is. The technical details name it; edit the connection to fix it.
    }

# config.unknown_database
# Variables:
#   $engine (String) - the engine's product name; `SQLite` selects the file wording.
#   $db (String) - the database Hacivat asked for.
#   $host (String) - the host Hacivat connected to.
#   $user (String) - the user name Hacivat sent (MySQL and MariaDB word it as "doesn't exist, or can't access").
#   $file (String) - the SQLite file's path.
error-conn-config-unknown-database-title = { $engine ->
        [SQLite] { $file } couldn't be opened.
        [MySQL] The database { $db } doesn't exist on { $host }, or { $user } can't access it.
        [MariaDB] The database { $db } doesn't exist on { $host }, or { $user } can't access it.
       *[other] The database { $db } doesn't exist on { $host }.
    }
# Followed by one button per database the user can open.
# Variables:
#   $engine (String) - the engine's product name; `SQLite` selects the file wording.
#   $user (String) - the user name Hacivat sent.
error-conn-config-unknown-database-body = { $engine ->
        [SQLite] The file may have been moved or deleted, or this Mac can't read the folder it's in. Check the path, or pick the file again.
       *[other] Signing in worked. These databases are open to { $user }:
    }

# server.too_many_connections
# Variables:
#   $host (String) - the host Hacivat connected to.
error-conn-server-too-many-connections-title = { $host } has no free connections.
error-conn-server-too-many-connections-body = The server has reached its connection limit. Try again in a moment, or close connections you no longer need.

# server.starting
# Variables:
#   $host (String) - the host Hacivat connected to.
error-conn-server-starting-title = { $host } is still starting up.
error-conn-server-starting-body = The database server is running but isn't ready for connections yet. Try again in a moment.

# server.shutdown
# Variables:
#   $host (String) - the host Hacivat connected to.
error-conn-server-shutdown-title = { $host } is shutting down.
error-conn-server-shutdown-body = The database server is stopping, or an administrator ended the session. Connect again once the server is back.

# server.read_only
# Variables:
#   $engine (String) - the engine's product name; `SQLite` selects the file wording.
#   $host (String) - the host Hacivat connected to.
#   $file (String) - the SQLite file's path.
error-conn-server-read-only-title = { $engine ->
        [SQLite] { $file } can only be read.
       *[other] { $host } only allows reading.
    }
# Variables:
#   $engine (String) - the engine's product name; `SQLite` selects the file wording.
error-conn-server-read-only-body = { $engine ->
        [SQLite] Hacivat can't write to this file or to its folder. Check the file's permissions, or open a copy you can write to.
       *[other] The server is in read-only mode, for example because it is a replica. Connect to the primary server to make changes.
    }

# server.unsupported_version: the server is older than the engine's floor (PostgreSQL 14,
# MySQL 8.0, MariaDB 10.6).
# Variables:
#   $host (String) - the host Hacivat connected to.
#   $engine (String) - the engine's product name, e.g. "PostgreSQL".
#   $version (String) - the server's version as it reported it, e.g. "9.6.24".
error-conn-server-unsupported-version-title = { $host } runs { $engine } { $version }, which Hacivat doesn't support.
# Variables:
#   $engine (String) - the engine's product name; `MySQL` and `MariaDB` select their floors.
error-conn-server-unsupported-version-body = { $engine ->
        [MySQL] Hacivat supports MySQL 8.0 and later.
        [MariaDB] Hacivat supports MariaDB 10.6 and later.
       *[other] Hacivat supports PostgreSQL 14 and later.
    } Its schema reading and safety checks rely on newer system catalogs.

# cancelled
# Variables:
#   $name (String) - the connection's name.
error-conn-cancelled-title = Connecting to { $name } was cancelled.
error-conn-cancelled-body = Nothing was changed. Connect again when you're ready.

# internal
# Variables:
#   $name (String) - the connection's name.
error-conn-internal-title = Something went wrong while connecting to { $name }.
error-conn-internal-body = Hacivat hit an error it didn't expect. The technical details have the server's message; copy them if you report the problem.

## m2b-t14-pipeline

# Connect errors that M2b reaches (HV-CONN-03; Task 2 NOTES "Idle and the new error copy").
# The SSH messages name the hop with `$hop_host`, never the server's text (which the error
# surface shows verbatim in its own field); the hop's number is the dialog's own line
# (`hv-conn-err-ssh-hop`), so no message here formats it.

# tls.pin_changed
# Variables:
#   $host (String) - the host Hacivat connected to.
# Review: PO
error-conn-tls-pin-changed-title = { $host } sent a different certificate from the pinned one.
# Review: PO
error-conn-tls-pin-changed-body = Hacivat stopped before signing in. Compare the new fingerprint with whoever runs the server before you replace the pin.

# ssh.host_key_unknown: a strict host-key policy, or a connect that could not ask (a
# reconnect). New copy: the mocks only have the prompt (`hk-title`, `hk-body`).
# Variables:
#   $hop_host (String) - the SSH server (hop) whose key is new.
# Review: PO
error-conn-ssh-host-key-unknown-title = Hacivat doesn't know the host key of { $hop_host } yet.
# Review: PO
error-conn-ssh-host-key-unknown-body = This Mac hasn't connected to this SSH server before, and the connection didn't ask whether to trust its key. Try again to review the key, or check the connection's host key setting.

# ssh.host_key_changed
# Variables:
#   $hop_host (String) - the SSH server (hop) whose key changed.
# Review: PO
error-conn-ssh-host-key-changed-title = The host key of { $hop_host } changed.
# Review: PO
error-conn-ssh-host-key-changed-body = Hacivat stopped before signing in. Review the new key before you trust it.

# ssh.auth: the mock names the SSH user and the key file; neither is in the error's arguments,
# so the copy names the host only.
# Variables:
#   $hop_host (String) - the SSH server (hop) that refused the sign-in.
# Review: PO
error-conn-ssh-auth-title = { $hop_host } rejected the SSH sign-in.
# Review: PO
error-conn-ssh-auth-body = Hacivat tried the sign-in methods this connection allows. Check that your public key is in ~/.ssh/authorized_keys on that host, or choose another sign-in method.

# ssh.channel_open: no full stop after the parenthesis (Task 2 ruling 14).
# Variables:
#   $hop_host (String) - the SSH server (hop) that refused to forward.
# Review: PO
error-conn-ssh-channel-open-title = { $hop_host } doesn't allow tunnels (AllowTcpForwarding)
# Variables:
#   $hop_host (String) - the SSH server (hop) that refused to forward.
#   $host (String) - the database host the tunnel was opened to.
# Review: PO
error-conn-ssh-channel-open-body = Signing in to { $hop_host } worked, but it refused to forward the connection to { $host }. Ask its administrator to allow TCP forwarding, and to allow { $host } if PermitOpen is set.

# ssh.agent_unavailable
# Review: PO
error-conn-ssh-agent-unavailable-title = Hacivat can't reach ssh-agent.
# Variables:
#   $hop_host (String) - the SSH server (hop) the agent's keys were for.
# Review: PO
error-conn-ssh-agent-unavailable-body = This connection signs in to { $hop_host } with keys from ssh-agent, but SSH_AUTH_SOCK isn't set for Hacivat. Start the agent and add your key with ssh-add, or choose a key file in the connection's SSH settings.

# ssh.host_key_revoked: the mock names the known_hosts file; the error does not carry it.
# Variables:
#   $hop_host (String) - the SSH server (hop) that sent the revoked key.
# Review: PO
error-conn-ssh-host-key-revoked-title = { $hop_host } sent a key that is marked as revoked.
# Review: PO
error-conn-ssh-host-key-revoked-body = Hacivat never connects with a revoked key. Ask the server's administrator for its current key.

# auth.insecure_method: one heading, two explanations (Task 2 ruling 10). `-body-tls` when TLS
# was up but not verified (the error carries the certificate), `-body-plain` without TLS.
# Review: PO
error-conn-auth-insecure-method-title = The server asked for the password in a form that isn't safe on this connection.
# Variables:
#   $host (String) - the host Hacivat connected to.
# Review: PO
error-conn-auth-insecure-method-body-tls = The connection is encrypted, but the certificate isn't verified, so Hacivat can't be sure the password reaches { $host }. It didn't send it. Review the certificate and pin it, then connect again.
# Review: PO
error-conn-auth-insecure-method-body-plain = The connection isn't encrypted, so the password would cross the network in a form others could read or reuse. Hacivat didn't send it. Turn on TLS on the server, or, if you accept the risk on this network, set Encryption to Disable.
# The class's default explanation (L026): the no-TLS one.
# Review: PO
error-conn-auth-insecure-method-body = { error-conn-auth-insecure-method-body-plain }

## m3a-t15-sqlite-typed

# The hint under an SQLite statement that failed with SQLITE_BUSY: another program holds a
# lock on the database file, and Hacivat stopped waiting for it (5 s; there is no retry). The
# engine's own message is shown verbatim next to it.
# Review: PO
error-exec-sqlite-busy-hint = Another program has locked this database file. Run the statement again when that program is done with it.
