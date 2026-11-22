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

## m3a-t23-exec

# Durations and sizes in the output log, the gutter and the result strip (NOTES rule 6): each is
# one message with its unit inside. Every number arrives pre-formatted in the format locale.

# A duration under one second.
# Variables:
#   $ms (String) - the milliseconds, e.g. "340".
exec-dur-ms = { $ms } ms
# A duration of whole seconds, under a minute.
# Variables:
#   $s (String) - the seconds, e.g. "2".
exec-dur-s = { $s } s
# A duration of whole minutes.
# Variables:
#   $min (String) - the minutes, e.g. "3".
exec-dur-min = { $min } min
# A duration of minutes and seconds.
# Variables:
#   $min (String) - the minutes, e.g. "3".
#   $s (String) - the seconds past them, e.g. "5".
exec-dur-min-s = { $min } min { $s } s
# A size in kilobytes (1,024 bytes).
# Variables:
#   $n (String) - the kilobytes with one decimal, e.g. "12.5".
exec-size-kb = { $n } KB
# A size in megabytes.
# Variables:
#   $n (String) - the megabytes with one decimal, e.g. "5.2".
exec-size-mb = { $n } MB
# A size in gigabytes.
# Variables:
#   $n (String) - the gigabytes with one decimal, e.g. "1.5".
exec-size-gb = { $n } GB

# Output log lines (HV-EXEC-11, NOTES surface 12). Server text is never inside these: it sits
# verbatim on its own row.

# A statement returned rows.
# Variables:
#   $rows (Number) - the rows retrieved, e.g. 1234.
#   $duration (String) - how long it took (an exec-dur-* message), e.g. "340 ms".
exec-rows-retrieved = { $rows ->
        [one] { $rows } row retrieved in { $duration }
       *[other] { $rows } rows retrieved in { $duration }
    }
# A statement returned rows and its cursor stays open for more.
# Variables:
#   $rows (Number) - the rows retrieved so far, e.g. 500.
#   $duration (String) - how long it took (an exec-dur-* message), e.g. "340 ms".
exec-rows-retrieved-more = { $rows ->
        [one] { $rows } row retrieved in { $duration }; more are available
       *[other] { $rows } rows retrieved in { $duration }; more are available
    }
# A statement changed rows.
# Variables:
#   $rows (Number) - the rows affected, e.g. 3.
#   $duration (String) - how long it took (an exec-dur-* message), e.g. "12 ms".
exec-rows-affected = { $rows ->
        [one] { $rows } row affected in { $duration }
       *[other] { $rows } rows affected in { $duration }
    }
# A statement without rows or a count (DDL, SET) finished.
# Variables:
#   $duration (String) - how long it took (an exec-dur-* message), e.g. "12 ms".
exec-completed = Completed in { $duration }
# The user's COMMIT ended their transaction.
exec-tx-committed = Transaction committed
# The user's ROLLBACK ended their transaction.
exec-tx-rolled-back = Transaction rolled back
# The end of a run of several statements.
# Variables:
#   $executed (Number) - the statements that ran, e.g. 2.
#   $total (Number) - the statements in the run, e.g. 3.
#   $failed (Number) - the statements that failed, e.g. 1.
#   $duration (String) - how long the run took (an exec-dur-* message), e.g. "1.20 s".
exec-summary = { $total ->
        [one] { $executed } of { $total } statement executed, { $failed } failed, in { $duration }
       *[other] { $executed } of { $total } statements executed, { $failed } failed, in { $duration }
    }
# A statement failed; the server's text follows on its own row, with the SQLSTATE chip.
# Variables:
#   $n (String) - the statement's number in its run, from 1, in ASCII digits never grouped, e.g. "3".
exec-error = Statement { $n } failed
# The statement in flight was cancelled.
# Variables:
#   $duration (String) - how long it ran (an exec-dur-* message), e.g. "2.00 s".
exec-cancelled = Cancelled after { $duration }
# A statement returned more than one result set (R-M3-25).
# Variables:
#   $skipped (Number) - the result sets read and not shown, e.g. 2.
exec-more-result-sets = { $skipped ->
        [one] { $skipped } more result set was read and not shown; only the first one is kept.
       *[other] { $skipped } more result sets were read and not shown; only the first one is kept.
    }
# A read-only console refused a statement before sending it (input layer).
exec-deny-read-only-input = Not sent: this console is read-only, and the statement can change data or schema.
# A read-only console refused a transaction command.
exec-deny-read-only-transaction = Not sent: a read-only console can't start or end transactions.
# The server reported a write inside the read-only transaction.
exec-deny-read-only-protocol = Stopped: the server reported a write inside the read-only transaction.
# The server refused the statement inside the read-only transaction.
exec-deny-read-only-engine = The server refused the statement because the transaction is read-only.
# A read-only console refused a statement that calls a function a rollback can't undo.
exec-deny-read-only-function = Not sent: the statement calls a function whose effects a rollback can't undo.
# A burst of lines outran the log; this many were never shown.
# Variables:
#   $n (Number) - the lines not shown, e.g. 312.
exec-events-dropped = { $n ->
        [one] { $n } event not shown
       *[other] { $n } events not shown
    }
# Run refused a COPY that streams data through the client.
exec-copy-unsupported = Not sent: COPY … FROM STDIN and TO STDOUT need the client to stream data, which consoles don't do.
# A run that never started: its session was closed or lost before anything ran. The reason
# (an exec-error-* sentence) follows on its own row.
exec-not-started = Nothing ran.

# A released console cursor (R-M3-23): the reason, and, when Hacivat's own transaction around
# it ended in a rollback, the sentence that says so (cursor-closed-rolled-back).
# server-terminated: the server answered a fetch with an error; the session stays connected
# (a lost session is connection-lost).

# Variables:
#   $rows (Number) - the rows fetched before the cursor closed, e.g. 10000.
cursor-closed-session-needed = { $rows ->
        [one] Cursor closed after { $rows } row: another statement needed the session.
       *[other] Cursor closed after { $rows } rows: another statement needed the session.
    }
# Variables:
#   $rows (Number) - the rows fetched before the cursor closed, e.g. 10000.
#   $duration (String) - how long it sat idle (an exec-dur-* message), e.g. "10 min".
cursor-closed-idle-timeout = { $rows ->
        [one] Cursor closed after { $rows } row: idle for { $duration }.
       *[other] Cursor closed after { $rows } rows: idle for { $duration }.
    }
# Variables:
#   $rows (Number) - the rows fetched before the cursor closed, e.g. 10000.
cursor-closed-tab-closed = { $rows ->
        [one] Cursor closed after { $rows } row: its tab was closed.
       *[other] Cursor closed after { $rows } rows: its tab was closed.
    }
# Variables:
#   $rows (Number) - the rows fetched before the cursor closed, e.g. 10000.
cursor-closed-tx-ended = { $rows ->
        [one] Cursor closed after { $rows } row: its transaction ended.
       *[other] Cursor closed after { $rows } rows: its transaction ended.
    }
# Variables:
#   $rows (Number) - the rows fetched before the cursor closed, e.g. 10000.
cursor-closed-user-closed = { $rows ->
        [one] Cursor closed after { $rows } row.
       *[other] Cursor closed after { $rows } rows.
    }
# Variables:
#   $rows (Number) - the rows fetched before the cursor closed, e.g. 10000.
cursor-closed-memory-limit = { $rows ->
        [one] Cursor closed after { $rows } row: the memory limit was reached.
       *[other] Cursor closed after { $rows } rows: the memory limit was reached.
    }
# Variables:
#   $rows (Number) - the rows fetched before the cursor closed, e.g. 10000.
cursor-closed-connection-lost = { $rows ->
        [one] Cursor closed after { $rows } row: the connection was lost.
       *[other] Cursor closed after { $rows } rows: the connection was lost.
    }
# Variables:
#   $rows (Number) - the rows fetched before the cursor closed, e.g. 10000.
cursor-closed-server-terminated = { $rows ->
        [one] Cursor closed after { $rows } row: the server stopped the query with an error.
       *[other] Cursor closed after { $rows } rows: the server stopped the query with an error.
    }
# Variables:
#   $rows (Number) - the rows fetched before the cursor closed, e.g. 10000.
cursor-closed-cancelled = { $rows ->
        [one] Cursor closed after { $rows } row: fetching was stopped.
       *[other] Cursor closed after { $rows } rows: fetching was stopped.
    }
# Variables:
#   $rows (Number) - the rows fetched before the cursor closed, e.g. 10000.
cursor-closed-enforcement-gap = { $rows ->
        [one] Cursor closed after { $rows } row: the read-only guard closed the session.
       *[other] Cursor closed after { $rows } rows: the read-only guard closed the session.
    }
# The sentence after a cursor-closed-* reason when Hacivat's transaction rolled back.
cursor-closed-rolled-back = Hacivat's transaction was rolled back.
# Variables:
#   $rows (Number) - the rows fetched before the cursor closed, e.g. 10000.
cursor-closed-session-needed-rolled-back = { $rows ->
        [one] Cursor closed after { $rows } row: another statement needed the session.
       *[other] Cursor closed after { $rows } rows: another statement needed the session.
    } { cursor-closed-rolled-back }
# Variables:
#   $rows (Number) - the rows fetched before the cursor closed, e.g. 10000.
#   $duration (String) - how long it sat idle (an exec-dur-* message), e.g. "10 min".
cursor-closed-idle-timeout-rolled-back = { $rows ->
        [one] Cursor closed after { $rows } row: idle for { $duration }.
       *[other] Cursor closed after { $rows } rows: idle for { $duration }.
    } { cursor-closed-rolled-back }
# Variables:
#   $rows (Number) - the rows fetched before the cursor closed, e.g. 10000.
cursor-closed-tab-closed-rolled-back = { $rows ->
        [one] Cursor closed after { $rows } row: its tab was closed.
       *[other] Cursor closed after { $rows } rows: its tab was closed.
    } { cursor-closed-rolled-back }
# Variables:
#   $rows (Number) - the rows fetched before the cursor closed, e.g. 10000.
cursor-closed-tx-ended-rolled-back = { $rows ->
        [one] Cursor closed after { $rows } row: its transaction ended.
       *[other] Cursor closed after { $rows } rows: its transaction ended.
    } { cursor-closed-rolled-back }
# Variables:
#   $rows (Number) - the rows fetched before the cursor closed, e.g. 10000.
cursor-closed-user-closed-rolled-back = { $rows ->
        [one] Cursor closed after { $rows } row.
       *[other] Cursor closed after { $rows } rows.
    } { cursor-closed-rolled-back }
# Variables:
#   $rows (Number) - the rows fetched before the cursor closed, e.g. 10000.
cursor-closed-memory-limit-rolled-back = { $rows ->
        [one] Cursor closed after { $rows } row: the memory limit was reached.
       *[other] Cursor closed after { $rows } rows: the memory limit was reached.
    } { cursor-closed-rolled-back }
# Variables:
#   $rows (Number) - the rows fetched before the cursor closed, e.g. 10000.
cursor-closed-connection-lost-rolled-back = { $rows ->
        [one] Cursor closed after { $rows } row: the connection was lost.
       *[other] Cursor closed after { $rows } rows: the connection was lost.
    } { cursor-closed-rolled-back }
# Variables:
#   $rows (Number) - the rows fetched before the cursor closed, e.g. 10000.
cursor-closed-server-terminated-rolled-back = { $rows ->
        [one] Cursor closed after { $rows } row: the server stopped the query with an error.
       *[other] Cursor closed after { $rows } rows: the server stopped the query with an error.
    } { cursor-closed-rolled-back }
# Variables:
#   $rows (Number) - the rows fetched before the cursor closed, e.g. 10000.
cursor-closed-cancelled-rolled-back = { $rows ->
        [one] Cursor closed after { $rows } row: fetching was stopped.
       *[other] Cursor closed after { $rows } rows: fetching was stopped.
    } { cursor-closed-rolled-back }
# Variables:
#   $rows (Number) - the rows fetched before the cursor closed, e.g. 10000.
cursor-closed-enforcement-gap-rolled-back = { $rows ->
        [one] Cursor closed after { $rows } row: the read-only guard closed the session.
       *[other] Cursor closed after { $rows } rows: the read-only guard closed the session.
    } { cursor-closed-rolled-back }

# What kind of failure a failed statement had: the gutter tooltip's and the log's second row
# when the server sent no text of its own (NOTES ruling 17: no per-code titles in M3a).

# The server refused the statement (its text follows verbatim).
exec-error-server = The server refused the statement.
# The connection dropped before the statement could run.
exec-error-connection-lost = The connection was lost.
# The connection dropped while the statement ran: it may or may not have taken effect.
exec-error-connection-lost-unknown = The connection was lost while the statement ran. It may or may not have taken effect.
# The statement was cancelled.
exec-error-cancelled = The statement was cancelled.
# Commit (or any statement but a rollback) on a failed transaction (R-M3-16 amended).
exec-error-tx-failed = Not sent: the transaction failed, so the server accepts only a rollback.
# BEGIN while a transaction is open.
exec-error-tx-in-progress = Not sent: a transaction is already open.
# A transaction command this connection can't run.
exec-error-tx-unsupported = Not sent: this connection can't run that transaction command.
# The read-only guard found a possible write and discarded the connection.
exec-error-enforcement-gap = The read-only guard couldn't confirm that nothing was written, so it closed the session.
# Run on a console whose session is closed or not connected.
exec-error-not-connected = Not sent: the console isn't connected.
# Hacivat failed on its own side.
exec-error-internal = Hacivat couldn't run the statement because of an internal error.

# Why a statement boundary is uncertain, so Run sends nothing (R-M3-24): one sentence per
# reason, shown under "Boundary uncertain".

# A ; inside an executable comment.
exec-uncertain-exec-comment = A ; sits inside an executable comment (/*! … */).
# A string, quoted name, dollar quote, comment or bracket is still open.
exec-uncertain-unterminated = A quote, comment or bracket above isn't closed.
# BEGIN/END or CASE/END blocks don't match.
exec-uncertain-unknown-block = The statement's BEGIN … END blocks don't match up.
# Under a client delimiter, a ; inside one statement.
exec-uncertain-delimiter = A ; inside the statement would split it under the current DELIMITER.
# A ; seems to be missing between two statements.
exec-uncertain-missing-semicolon = A ; seems to be missing between two statements.
# An executable comment or optimizer hint whose end depends on the server.
exec-uncertain-comment-end = Where an executable comment or optimizer hint ends depends on the server.
# A client command inside the statement.
exec-uncertain-client-command = A client command inside the statement can't be run as written.
# Several carets or selections share part of one statement.
exec-uncertain-overlapping-units = Two of the carets or selections share part of a statement, which would run it twice.

## m3b-t04-safety

# The classifier's classes, reasons and destructive patterns (sql-safety, Task 4). A class is
# a short label for what a statement does (badges, dialog headings); a reason is one sentence
# on why a statement is refused or needs a confirmation; a destructive pattern is one sentence
# naming the table it touches. SQL keywords stay in English.

# Class: the statement only reads.
safety-class-read = Reads data
# Class: transaction control or session settings.
safety-class-session = Changes the session
# Class: the statement changes rows (or locks them).
safety-class-write = Modifies data
# Class: the statement changes the schema.
safety-class-ddl = Changes the schema
# Class: privileges, server administration, files, network or programs on the server.
safety-class-admin = Administrative or external side effects
# Class: Hacivat can't tell what the statement does.
safety-class-unknown = Effects can't be determined

# Reason: the check isn't built yet, so nothing runs.
safety-reason-not-implemented = Hacivat can't check this statement yet.
# Reason: an unterminated quote or comment, or a character the lexer rejects.
safety-reason-lex-error = The statement has a quote, comment or character Hacivat can't read.
# Reason: a psql or mysql client command (\copy, DELIMITER, …) inside the text.
safety-reason-client-command = The text contains a client command, which the server can't run.
# Reason: a template variable such as ${name} that nobody filled in.
safety-reason-template-variable = The text contains a template variable that hasn't been filled in.
# Reason: only comments or semicolons.
safety-reason-no-statement = There's no statement to run.
# Reason: a unit holds more than one statement.
safety-reason-multi-statement = The text holds more than one statement.
# Reason: the statement boundary is uncertain.
safety-reason-ambiguous-boundary = Where the statement ends is uncertain.
# Reason: the parser rejected the statement.
safety-reason-parse-error = The statement couldn't be parsed.
# Reason: two parsers read the statement differently.
safety-reason-parser-disagreement = Hacivat's two readings of the statement disagree.
# Reason: the statement is past the size, token or nesting limits.
safety-reason-too-complex = The statement is too large or too deeply nested to check.
# Reason: syntax from a newer server version than this build knows.
safety-reason-syntax-newer-than-agent = The statement uses syntax newer than this version of Hacivat knows.
# Reason: no classifier for this database yet.
safety-reason-unsupported-dialect = Hacivat can't check statements for this database yet.
# Reason: a function known to have side effects.
# Variables:
#   $name (String) - the function's name as written in the statement, e.g. "pg_sleep".
safety-reason-fn-denylisted = { $name } is on the list of functions with side effects.
# Reason: a function Hacivat has no data on.
# Variables:
#   $name (String) - the function's name as written in the statement.
safety-reason-fn-unknown = Hacivat doesn't know what { $name } does.
# Reason: an operator Hacivat has no data on.
# Variables:
#   $name (String) - the operator as written, e.g. "###".
safety-reason-operator-unknown = Hacivat doesn't know what the operator { $name } does.
# Reason: a cast to a type Hacivat has no data on.
# Variables:
#   $name (String) - the target type's name as written.
safety-reason-cast-unknown = Hacivat doesn't know what a conversion to { $name } does.
# Reason: a MySQL/MariaDB executable comment.
safety-reason-executable-comment = An executable comment (/*! … */) runs code that's hidden in a comment.
# Reason: SET sql_mode, NAMES, standard_conforming_strings and the like.
safety-reason-lex-mode-change = The statement changes how the server reads SQL text.
# Reason: SET ROLE, SET SESSION AUTHORIZATION and the like.
safety-reason-privilege-change = The statement changes roles or privileges.
# Reason: WITH x AS (DELETE …) and the like.
safety-reason-data-modifying-cte = A WITH clause changes data.
# Reason: FOR UPDATE, FOR SHARE, LOCK IN SHARE MODE.
safety-reason-locking-clause = The statement locks rows.
# Reason: EXPLAIN ANALYZE executes its statement.
safety-reason-explain-analyze = EXPLAIN ANALYZE runs the statement.
# Reason: SQL assembled from text at run time (EXECUTE, query_to_xml, …).
safety-reason-dynamic-sql = The statement runs SQL that's built from text.
# Reason: DO blocks and other procedural code.
safety-reason-procedural-code = The statement runs procedural code.
# Reason: BEGIN, COMMIT, ROLLBACK, SAVEPOINT.
safety-reason-txn-control = The statement controls the transaction.
# Reason: SET, RESET, USE, LISTEN and the like.
safety-reason-session-state = The statement changes the session's settings.
# Reason: reading or writing server files.
safety-reason-external-file = The statement reads or writes files on the server.
# Reason: dblink, foreign tables and the like.
safety-reason-external-network = The statement reaches another server over the network.
# Reason: COPY … PROGRAM and the like.
safety-reason-process-exec = The statement runs a program on the server.
# Reason: server administration (CHECKPOINT, ALTER SYSTEM, KILL, …).
safety-reason-admin-control = The statement administers the server.
# Reason: txid_current() and the like.
safety-reason-xid-consume = The statement uses up a transaction ID.
# Reason: nextval() and the like.
safety-reason-sequence-advance = The statement advances a sequence.
# Reason: writes to temporary tables.
safety-reason-write-temp = The statement writes to a temporary table.
# Reason: MAX_EXECUTION_TIME(0) or another hint that lifts the time limit.
safety-reason-hint-exec-time = An optimizer hint lifts the time limit.
# Reason: a SET_VAR optimizer hint.
safety-reason-hint-set-var = An optimizer hint changes a setting.
# Reason: CALL of a routine.
safety-reason-routine-call = The statement calls a routine whose effects can't be checked.
# Reason: SET of a variable that isn't on the harmless list, on a read-only connection.
# Variables:
#   $name (String) - the setting's name as written in the statement, e.g. "work_mem".
safety-reason-set-not-allowlisted = On a read-only connection only harmless settings can be changed, and { $name } isn't one of them.
# Reason: INSERT, UPDATE, DELETE, MERGE.
safety-reason-write-data = The statement changes data.
# Reason: CREATE, ALTER, DROP.
safety-reason-schema-change = The statement changes the schema.
# Reason: LOCK TABLE(S).
safety-reason-table-lock = The statement locks whole tables.
# Reason: pg_advisory_lock(), GET_LOCK() and the like.
safety-reason-advisory-lock = The statement takes an advisory lock.
# Reason: files or the network outside the database.
safety-reason-external-io = The statement reaches files or the network outside the database.
# Reason: PREPARE.
safety-reason-prepared-statement = The statement prepares another statement to run later.
# Reason: NOTIFY.
safety-reason-notify = The statement sends a notification to other sessions.
# Reason: the classifier couldn't determine the effects.
safety-reason-effects-unknown = What the statement does can't be determined.
# Reason: a destructive pattern (the patterns below say which).
safety-reason-destructive = The statement can change or remove a lot of data.
# Reason: GRANT, REVOKE, CREATE ROLE and the like.
safety-reason-privilege-grant = The statement grants or revokes privileges or roles.
# Reason: a volatile function no allowlist covers (its effects can't be checked).
safety-reason-volatile-function = The statement calls a volatile function whose effects can't be checked.

# Destructive pattern: UPDATE or DELETE without WHERE.
# Variables:
#   $table (String) - the table's name as written in the statement.
safety-destructive-no-where = { $table } has no WHERE clause, so every row changes.
# Destructive pattern: a WHERE that is always true (1=1, TRUE, …).
# Variables:
#   $table (String) - the table's name as written in the statement.
safety-destructive-always-true = The WHERE clause on { $table } is always true, so every row changes.
# Destructive pattern: a DELETE/UPDATE join that never limits the target's rows.
# Variables:
#   $table (String) - the table's name as written in the statement.
safety-destructive-join-without-on = The join never limits the rows of { $table }, so every row can change.
# Destructive pattern: DROP.
# Variables:
#   $table (String) - the object's name as written in the statement.
safety-destructive-drop = Removes { $table }.
# Destructive pattern: DROP … CASCADE.
# Variables:
#   $table (String) - the object's name as written in the statement.
safety-destructive-drop-cascade = Removes { $table } and everything that depends on it.
# Destructive pattern: TRUNCATE.
# Variables:
#   $table (String) - the table's name as written in the statement.
safety-destructive-truncate = Removes every row of { $table }.
# ## m3c-p2-07 (fix round 4, S2) Destructive pattern: PostgreSQL TRUNCATE … CASCADE.
# Variables:
#   $table (String) - the table's name as written in the statement.
safety-destructive-truncate-cascade = Removes every row of { $table } and of every table that references it (CASCADE).
# Destructive pattern: ALTER … DROP COLUMN.
# Variables:
#   $table (String) - the table's name as written in the statement.
safety-destructive-drop-column = Removes a column of { $table } and its data.
# Destructive pattern: ALTER … DROP CONSTRAINT.
# Variables:
#   $table (String) - the table's name as written in the statement.
safety-destructive-drop-constraint = Removes a constraint of { $table }.
# Destructive pattern: ALTER … TYPE (rewrites the column).
# Variables:
#   $table (String) - the table's name as written in the statement.
safety-destructive-alter-type = Changes a column type of { $table }, which rewrites its data.
# Destructive pattern: MERGE … WHEN MATCHED THEN DELETE.
# Variables:
#   $table (String) - the table's name as written in the statement.
safety-destructive-merge-delete = The MERGE deletes rows of { $table }.
# Destructive pattern: MySQL UPDATE … LIMIT without ORDER BY.
# Variables:
#   $table (String) - the table's name as written in the statement.
safety-destructive-update-limit-no-order = UPDATE … LIMIT on { $table } without ORDER BY changes rows in no fixed order.

## m3b-t16-run

# A duration of a second or more (the one duration rule of the log, the gutter and the status
# bar: milliseconds under a second, else seconds with two decimals).
# Variables:
#   $seconds (Decimal) - seconds with two decimals, e.g. 2.45.
exec-dur-seconds = { NUMBER($seconds, minimumFractionDigits: 2) } s
# A server message the statement produced (PostgreSQL DEBUG). The server's own text follows on
# its own row, verbatim.
exec-notice-debug = Server debug message
# A server message the statement produced (PostgreSQL LOG).
exec-notice-log = Server log message
# A server message the statement produced (PostgreSQL INFO).
exec-notice-info = Server info message
# A server notice the statement produced (PostgreSQL NOTICE, MySQL Note).
exec-notice-notice = Server notice
# A server warning the statement produced (PostgreSQL WARNING, MySQL Warning).
exec-notice-warning = Server warning
# Server notices past the per-statement limit were counted and not shown.
# Variables:
#   $n (Number) - the notices not shown, e.g. 12.
exec-notices-dropped = { $n ->
        [one] { $n } more server notice was not shown.
       *[other] { $n } more server notices were not shown.
    }
# A statement returned more result sets than a run keeps (R-M3-66).
# Variables:
#   $total (Number) - the result sets it returned, e.g. 150.
#   $limit (Number) - the result sets kept, e.g. 100.
exec-result-set-limit = The statement returned { $total } result sets; the first { $limit } are shown.
# The user skipped statements in the confirmation before the run.
# Variables:
#   $n (Number) - the statements skipped, e.g. 2.
exec-skipped-by-user = { $n ->
        [one] { $n } statement skipped at your request.
       *[other] { $n } statements skipped at your request.
    }
# A failed statement was run again (the error bar's Retry).
# Variables:
#   $n (String) - the statement's number in the run, e.g. "3".
exec-retried = Running statement { $n } again
# A background result was closed to free memory; its first rows stay.
# Variables:
#   $rows (Number) - the rows kept, e.g. 1000.
cursor-closed-evicted = { $rows ->
        [one] Closed to free memory; { $rows } row kept.
       *[other] Closed to free memory; { $rows } rows kept.
    }
# A text run as one was stopped; statements after the stopped result ran, and their results
# are not shown.
# Variables:
#   $n (Number) - the statements that ran, e.g. 2.
exec-run-as-one-closed-early = { $n ->
        [one] Stopped. { $n } more statement ran; its result isn't shown.
       *[other] Stopped. { $n } more statements ran; their results aren't shown.
    }
# As exec-run-as-one-closed-early, and the rest of the text did not run.
# Variables:
#   $n (Number) - the statements that ran, e.g. 2.
exec-run-as-one-closed-early-rest = { $n ->
        [one] Stopped. { $n } more statement ran (its result isn't shown); the rest didn't run.
       *[other] Stopped. { $n } more statements ran (their results aren't shown); the rest didn't run.
    }
# PostgreSQL ran a text sent as one in a single transaction, so a failure rolled back the
# statements before it too.
exec-run-as-one-rolled-back = PostgreSQL ran the text as one transaction, so the statements before the failing one were rolled back too, unless the text committed them itself.
# A text run as one failed or was stopped inside the user's open transaction on PostgreSQL:
# nothing was rolled back yet, but the transaction failed (fix round 1, review m9).
exec-run-as-one-tx-failed = The transaction failed: nothing more runs in it until you roll it back.

## m3b-t15-gate

# The exec gate (Task 15, R-M3-55): why Run sends nothing for a statement, and the extra
# reasons a confirmation lists. Shown in the Output log and in the confirmation dialog.

# The check no longer matches the text that would be sent (it changed after the check).
exec-gate-stale-classification = Not sent: the statement changed after Hacivat checked it. Run it again.
# The statement checker isn't available in this build.
exec-gate-not-classified = Not sent: Hacivat couldn't check this statement.
# A read-only connection runs reads only.
# Variables:
#   $class (String) - what the statement does: session, tx, lock, routine, dml, ddl, admin or unknown.
exec-gate-read-only = { $class ->
        [session] Not sent: this connection is read-only, and the statement changes the session's settings.
        [tx] Not sent: this connection is read-only, and the statement controls the transaction.
        [lock] Not sent: this connection is read-only, and the statement locks data.
        [routine] Not sent: this connection is read-only, and the routine it calls may change data.
        [dml] Not sent: this connection is read-only, and the statement changes data.
        [ddl] Not sent: this connection is read-only, and the statement changes the schema.
        [admin] Not sent: this connection is read-only, and the statement administers the server or reaches outside the database.
       *[other] Not sent: this connection is read-only, and Hacivat can't tell what the statement does.
    }
# The safety policy never runs this statement on a production connection.
exec-gate-policy-denied = Not sent: Hacivat doesn't run this statement on a production connection.
# MySQL, MariaDB or TiDB, a transaction is open, and the statement makes the server commit it
# first (DDL, GRANT, BEGIN, LOCK TABLES and the like).
exec-gate-implicit-commit = The server commits your open transaction before it runs this statement, so your earlier changes are committed too.
# Fix round 1 (I1): off production, a statement Hacivat couldn't analyse whose words may change data.
exec-gate-not-analysed = Hacivat couldn't analyse this statement, and it may change data. Check it before you run it.
# Fix round 1 (I2, M7): on production, what the rollback around EXPLAIN ANALYZE doesn't undo.
exec-explain-analyze-not-undone = EXPLAIN ANALYZE runs the statement and then rolls it back, but a rollback doesn't undo sequence advances, advisory locks, changes made through dblink or other files and network, or writes to non-transactional (MyISAM) tables.
# (Named exception, Task 15: exec-run-as-one-unsupported was removed from the m3a-t23 block; it
# was unused once a selection runs as one.)
# EXPLAIN ANALYZE runs its statement; Hacivat rolls it back, which can't undo a routine call, an
# administrative statement or a MySQL schema change.
exec-explain-analyze-refused = Not sent: EXPLAIN ANALYZE would run a statement that a rollback can't undo.
# On production, a statement that may change every row of a table Hacivat couldn't name.
exec-unsafe-no-target = Not sent: the statement may change every row, and Hacivat can't tell which table to ask you to confirm.
# On production, a run with more than one destructive target that asks for a typed name (two
# tables, the same table twice, a table and a schema): one typed name confirms exactly one
# (m3c-p2-07 fix round 3).
exec-unsafe-several-targets = Not sent: a typed name confirms one destructive target; this run has several. Run each on its own.

# Why a statement's parameters keep it from being sent (HV-EXEC-03).

# A template variable such as ${name} next to the statement's parameters.
exec-param-template-variable = Not sent: the statement holds a template variable, which can't be sent as a parameter.
# PostgreSQL: ? is an operator unless the setting makes it a parameter.
exec-param-question-mark-off = On PostgreSQL, ? is an operator here, not a parameter. Settings › Execution can make it a parameter.
# A parameter inside CREATE, ALTER or DROP while the setting is off.
exec-param-in-ddl = Not sent: parameters inside a schema change are off. Settings › Execution can turn them on.
# :name and ? (or $1) in one statement.
exec-param-mixed-styles = Not sent: the statement mixes parameter styles. Use one style per statement.
# More parameters than the protocol carries.
exec-param-too-many-binds = Not sent: the statement has more than 65,535 parameters.
# A marker the server would read as a parameter, but Hacivat can't give it a value.
exec-param-unbindable = Not sent: the statement holds a parameter marker Hacivat can't fill, so the server would get no value for it.

## m3b-t19-confirm-tx

# Confirm-in-transaction (HV-EXEC-04; D-014, R-M3-55, P-M3b-1). Output log lines and the
# titles of the errors the session ends a confirmed statement with.

# The user committed (or kept) the confirmed statement.
exec-confirm-committed = Committed: the confirmed statement's changes are saved.
# Savepoint form: the user undid the confirmed statement; their transaction stays open.
exec-confirm-undone = Undone: the confirmed statement was rolled back, and your transaction is still open.
# Hacivat's own transaction: the user rolled the confirmed statement back (or the console closed).
exec-confirm-rolled-back = Rolled back: the confirmed statement's changes were discarded.
# Nobody answered in time (D-014): the confirmed statement was rolled back.
exec-confirm-rolled-back-deadline = Rolled back: no answer came in time, so the confirmed statement's changes were discarded.
# Savepoint form: the confirmed statement failed; only it was rolled back.
exec-confirm-statement-failed = The confirmed statement failed. Only it was rolled back; your transaction is intact.
# MySQL/MariaDB: a statement the user confirmed committed their open transaction before it ran.
exec-implicit-commit = The server committed your open transaction before it ran this statement.
# MySQL/MariaDB: a statement would commit a transaction an earlier statement of the run opened.
exec-error-tx-implicit-commit = Not sent: this statement would commit the transaction an earlier statement of this run opened, which you didn't confirm. Run it on its own.
# The server committed the statement on its own (autocommit was on), so nothing could be asked.
exec-confirm-already-committed = Already committed: the server committed this statement on its own, so Hacivat couldn't ask first. The rest of the run wasn't sent.
# The deadline rolled the confirmed statement back; the run stopped.
exec-error-confirm-timed-out = Rolled back: no answer came in time, so the statement was undone and the rest of the run wasn't sent.
# The user rolled the confirmed statement back; the run treats it as not run.
exec-error-confirm-undone = Rolled back at your request: the statement's changes were discarded.
# MySQL/MariaDB: starting a transaction would release the user's LOCK TABLES locks.
exec-error-tx-table-locks = Not sent: your LOCK TABLES locks are held, and starting a transaction would release them. Run UNLOCK TABLES first.
# A statement failed and the server rolled back the whole open transaction with it.
exec-tx-rolled-back-by-server = The server rolled back your whole transaction: every change since it began is gone.
# A statement failed and the user's transaction ended with it, but the statement may have committed it first (fix round 2).
exec-tx-ended-by-statement = Your transaction ended with this statement. Check whether your earlier changes were committed.
# EXPLAIN ANALYZE ran with no transaction open, so what it changed couldn't be rolled back; the run stopped.
exec-explain-not-rolled-back = EXPLAIN ANALYZE ran outside a transaction, so its changes couldn't be rolled back. The rest of the run wasn't sent.

## m3b-t22-tx-ui

# A transaction's age under a minute (the badge, the idle toast); whole minutes are exec-dur-min.
exec-dur-lt-min = <1 min
# Output: the profile's startup script did not run, because this console's session is
# read-only (a read-only session never runs it). The script's SQL is never shown.
exec-startup-script-skipped-read-only = The connection's startup script was not run: this session is read-only.

## m3b-t18-safe06

# A read-only console refused a statement before sending it: Hacivat's classifier couldn't
# confirm that it only reads (or found that it doesn't). One sentence for every classifier
# denial (T1c review M2).
exec-deny-classifier = Not sent: this console is read-only, and Hacivat couldn't confirm that the statement only reads.

## m3b-t23-escalation

# Terminate on server ended the session (design/m3b-safe surface 6).
# Variables:
#   $n (Number) - statements in the user's open transaction it rolled back, e.g. 3.
exec-terminated = { $n ->
        [one] Session terminated on the server; its open transaction ({ $n } statement) was rolled back.
       *[other] Session terminated on the server; its open transaction ({ $n } statements) was rolled back.
    }
# As exec-terminated when no user transaction was open.
exec-terminated-no-tx = Session terminated on the server.
exec-terminate-failed = Couldn't terminate the session on the server.
# Variables:
#   $name (String) - the connection's name, e.g. "orders-dev".
exec-disconnected = Disconnected { $name }. The statement may still be running on the server.
# As exec-disconnected for a SQLite file: the statement stopped with its connection (fix round 1).
# Variables:
#   $name (String) - the connection's name, e.g. "app-local".
exec-disconnected-local = Disconnected { $name }. SQLite stopped the statement when its connection closed.
# The user answered Stop to the error bar (surface 7).
# Variables:
#   $i (String) - the failed statement's number in the run, e.g. "2".
exec-run-stopped = Stopped at statement { $i }: you chose Stop.
# Variables:
#   $i (String) - the number of the statement that runs next, e.g. "4".
exec-run-rolled-back-continue = Transaction rolled back; continuing with statement { $i }.

## m3b-t24-exec-ui

# The run UI's lines before a run is sent (Task 24): what the user answered, or why nothing
# was sent. A statement number is raw digits.
# The user cancelled the parameter prompt: nothing was sent.
exec-params-cancelled = Nothing was sent: you cancelled the parameter values.
# The connection's lexing settings couldn't be read, and the statement's parameters read
# differently under them: nothing was sent.
exec-param-modes-unknown = Not sent: Hacivat couldn't read this connection's settings, and the statement's parameters read differently under them.
# "Run as one" can't send a statement that needs its own transaction or its rollback wrap.
exec-gate-run-as-one-refused = Not sent: this statement can't run as part of one request here. Run its statements one at a time.
# The user cancelled at a confirmation: nothing of the run was sent.
# Variables:
#   $n (String) - the statement's position in the run, from 1.
exec-confirm-cancelled = Nothing was sent: you cancelled at statement { $n }.
# The user chose Skip for a statement at the confirmation.
# Variables:
#   $n (String) - the statement's position in the run, from 1.
exec-confirm-skipped = Statement { $n } skipped: you chose Skip.
# A statement ran without the confirmation the user turned off for statements like it.
# Variables:
#   $console (String) - the console's title, e.g. "orders-dev (2)".
exec-confirm-suppressed = Ran without asking: you chose not to be asked again for statements like this in { $console }.
# Fix round 1 (I4): this statement was not sent because another statement of the same run was
# refused (nothing of a run is sent then).
# Variables:
#   $n (String) - the refused statement's position in the run, from 1.
exec-gate-run-refused = Not sent: statement { $n } of this run was refused, so nothing of the run was sent.
# Fix round 1 (I1): the connection now reads statements with other settings than the console
# split them with.
exec-run-context-changed = Not sent: this connection's settings changed since Run read them. Run it again.
# Fix round 1 (M4): the console reconnected while the run was being prepared.
exec-run-connection-changed = Not sent: the connection changed while this run was being prepared. Run it again.
# Fix round 1 (M1): Run as one sends one request, which carries no parameter values.
exec-param-run-as-one = Not sent: Run as one can't take parameter values. Run the statements one at a time.
# Fix round 1 (M2): Run as one on a read-only connection.
exec-gate-run-as-one-read-only = Not sent: Run as one doesn't work on a read-only connection, whose guard checks one statement at a time.

## m3b-t26-prod-unlock

# Task 26 (carry MUST 2): on a production or read-only connection, Run could not read the
# connection's settings in time before sending (the read timed out), so it can't prove they
# are the ones it checked the statement under: nothing was sent.
exec-run-context-unconfirmed = Not sent: Hacivat couldn't confirm this connection's settings in time. Run it again.
# Fix round 1 (ruling on non-transactional tables): the confirmed statement's rollback ran, but
# the server said a table does not support transactions (MySQL/MariaDB 1196), so its changes
# stay.
exec-confirm-not-undone = Not undone: the table doesn't support transactions, so the statement's changes stay.
# The unit's error for the same case.
exec-error-confirm-not-undone = Not undone: the table doesn't support transactions, so this statement's changes stay.

## m3c-p2-07

# PO P2-07, destructive pattern: PostgreSQL DROP OWNED BY (the role's objects go, not the role).
# Variables:
#   $role (String) - the role's name as written in the statement (CURRENT_USER and the like
#                    as written too).
safety-destructive-drop-owned = Removes every object { $role } owns in this database. It also revokes the role's privileges here and on shared objects such as databases and tablespaces, and its default privileges.
# PO P2-07, destructive pattern: DROP OWNED BY … CASCADE.
# Variables:
#   $role (String) - the role's name as written in the statement.
safety-destructive-drop-owned-cascade = Removes every object { $role } owns in this database and everything that depends on those objects. It also revokes the role's privileges here and on shared objects such as databases and tablespaces, and its default privileges.
# PO P2-07: on production DROP OWNED asks for one role's name typed, and this run has more
# than one destructive target (several roles, the same role twice, or a role beside another
# statement that asks for a name).
exec-drop-owned-several-targets = Not sent: a typed role name confirms one DROP OWNED; this run has other targets. Run it on its own.
# PO P2-07: on production, a DROP OWNED whose role Hacivat couldn't name.
exec-drop-owned-no-target = Not sent: Hacivat can't tell whose objects this DROP OWNED removes, so it can't ask you to type the role's name.

## m4-t01a-model

# Object kind labels of the metadata model (alg/02 HV-MODEL-01): one whole-phrase message
# per kind, `model-kind-<id>` with `<id>` the kind's stable machine id, overridable per
# dialect (`model-kind-schema-mysql`). The label alone, never joined to a number in code;
# the count only selects the form.

# Kind label (root).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-root = { $count ->
        [one] server
       *[other] servers
    }
# Kind label (database).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-database = { $count ->
        [one] database
       *[other] databases
    }
# Kind label (schema).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-schema = { $count ->
        [one] schema
       *[other] schemas
    }
# Kind label (table).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-table = { $count ->
        [one] table
       *[other] tables
    }
# Kind label (view).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-view = { $count ->
        [one] view
       *[other] views
    }
# Kind label (materialized-view).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-materialized-view = { $count ->
        [one] materialized view
       *[other] materialized views
    }
# Kind label (foreign-table).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-foreign-table = { $count ->
        [one] foreign table
       *[other] foreign tables
    }
# Kind label (virtual-table).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-virtual-table = { $count ->
        [one] virtual table
       *[other] virtual tables
    }
# Kind label (column).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-column = { $count ->
        [one] column
       *[other] columns
    }
# Kind label (index).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-index = { $count ->
        [one] index
       *[other] indexes
    }
# Kind label (primary-key).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-primary-key = { $count ->
        [one] primary key
       *[other] primary keys
    }
# Kind label (unique-key).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-unique-key = { $count ->
        [one] unique key
       *[other] unique keys
    }
# Kind label (foreign-key).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-foreign-key = { $count ->
        [one] foreign key
       *[other] foreign keys
    }
# Kind label (check).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-check = { $count ->
        [one] check constraint
       *[other] check constraints
    }
# Kind label (exclusion).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-exclusion = { $count ->
        [one] exclusion constraint
       *[other] exclusion constraints
    }
# Kind label (default).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-default = { $count ->
        [one] default
       *[other] defaults
    }
# Kind label (trigger).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-trigger = { $count ->
        [one] trigger
       *[other] triggers
    }
# Kind label (rule).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-rule = { $count ->
        [one] rule
       *[other] rules
    }
# Kind label (policy).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-policy = { $count ->
        [one] policy
       *[other] policies
    }
# Kind label (partition).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-partition = { $count ->
        [one] partition
       *[other] partitions
    }
# Kind label (subpartition).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-subpartition = { $count ->
        [one] subpartition
       *[other] subpartitions
    }
# Kind label (routine).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-routine = { $count ->
        [one] function
       *[other] functions
    }
# Kind label (procedure).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-procedure = { $count ->
        [one] procedure
       *[other] procedures
    }
# Kind label (aggregate).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-aggregate = { $count ->
        [one] aggregate function
       *[other] aggregate functions
    }
# Kind label (argument).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-argument = { $count ->
        [one] argument
       *[other] arguments
    }
# Kind label (sequence).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-sequence = { $count ->
        [one] sequence
       *[other] sequences
    }
# Kind label (domain).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-domain = { $count ->
        [one] domain
       *[other] domains
    }
# Kind label (enum-type).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-enum-type = { $count ->
        [one] enum type
       *[other] enum types
    }
# Kind label (composite-type).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-composite-type = { $count ->
        [one] composite type
       *[other] composite types
    }
# Kind label (range-type).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-range-type = { $count ->
        [one] range type
       *[other] range types
    }
# Kind label (alias-type).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-alias-type = { $count ->
        [one] alias type
       *[other] alias types
    }
# Kind label (table-type).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-table-type = { $count ->
        [one] table type
       *[other] table types
    }
# Kind label (synonym).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-synonym = { $count ->
        [one] synonym
       *[other] synonyms
    }
# Kind label (extension).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-extension = { $count ->
        [one] extension
       *[other] extensions
    }
# Kind label (collation).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-collation = { $count ->
        [one] collation
       *[other] collations
    }
# Kind label (operator).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-operator = { $count ->
        [one] operator
       *[other] operators
    }
# Kind label (operator-class).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-operator-class = { $count ->
        [one] operator class
       *[other] operator classes
    }
# Kind label (operator-family).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-operator-family = { $count ->
        [one] operator family
       *[other] operator families
    }
# Kind label (cast).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-cast = { $count ->
        [one] cast
       *[other] casts
    }
# Kind label (language).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-language = { $count ->
        [one] language
       *[other] languages
    }
# Kind label (access-method).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-access-method = { $count ->
        [one] access method
       *[other] access methods
    }
# Kind label (foreign-data-wrapper).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-foreign-data-wrapper = { $count ->
        [one] foreign-data wrapper
       *[other] foreign-data wrappers
    }
# Kind label (foreign-server).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-foreign-server = { $count ->
        [one] foreign server
       *[other] foreign servers
    }
# Kind label (user-mapping).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-user-mapping = { $count ->
        [one] user mapping
       *[other] user mappings
    }
# Kind label (event-trigger).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-event-trigger = { $count ->
        [one] event trigger
       *[other] event triggers
    }
# Kind label (role).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-role = { $count ->
        [one] role
       *[other] roles
    }
# Kind label (user).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-user = { $count ->
        [one] user
       *[other] users
    }
# Kind label (login).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-login = { $count ->
        [one] login
       *[other] logins
    }
# Kind label (tablespace).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-tablespace = { $count ->
        [one] tablespace
       *[other] tablespaces
    }
# Kind label (event).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-event = { $count ->
        [one] event
       *[other] events
    }
# Kind label (virtual-table-module).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-virtual-table-module = { $count ->
        [one] virtual table module
       *[other] virtual table modules
    }
# Kind label (dictionary).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-dictionary = { $count ->
        [one] dictionary
       *[other] dictionaries
    }
# Kind label (row-policy).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-row-policy = { $count ->
        [one] row policy
       *[other] row policies
    }
# Kind label (settings-profile).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-settings-profile = { $count ->
        [one] settings profile
       *[other] settings profiles
    }
# Kind label (quota).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-quota = { $count ->
        [one] quota
       *[other] quotas
    }
# Kind label (projection).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-projection = { $count ->
        [one] projection
       *[other] projections
    }
# Kind label (package).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-package = { $count ->
        [one] package
       *[other] packages
    }
# Kind label (package-body).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-package-body = { $count ->
        [one] package body
       *[other] package bodies
    }
# Kind label (object-type).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-object-type = { $count ->
        [one] object type
       *[other] object types
    }
# Kind label (db-link).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-db-link = { $count ->
        [one] database link
       *[other] database links
    }
# Kind label (collection).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-collection = { $count ->
        [one] collection
       *[other] collections
    }
# Kind label (mongo-view).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-mongo-view = { $count ->
        [one] view
       *[other] views
    }
# Kind label (time-series).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-time-series = { $count ->
        [one] time series collection
       *[other] time series collections
    }
# Kind label (inferred-field).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-inferred-field = { $count ->
        [one] inferred field
       *[other] inferred fields
    }
# Kind label (redis-database).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-redis-database = { $count ->
        [one] database
       *[other] databases
    }
# Kind label (redis-module).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-redis-module = { $count ->
        [one] module
       *[other] modules
    }
# Kind label (unknown).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-unknown = { $count ->
        [one] object of an unknown kind
       *[other] objects of an unknown kind
    }
# Kind label (schema-mysql).
# Variables:
#   $count (Number) - how many objects the label names, e.g. 2.
model-kind-schema-mysql = { $count ->
        [one] database
       *[other] databases
    }
# The explorer badge on an object name that holds characters a reader cannot see (bidi
# controls, zero-width characters); the name itself is shown verbatim.
model-name-has-invisible-chars = This name contains characters that can't be seen.
# Notice when a connection's cached metadata could not be used and the cache was started again.
# Variables:
#   $reason (String) - why: "corrupt", "server-changed", "wrong-key" or "incompatible".
model-cache-rebuilt = { $reason ->
        [corrupt] The cached metadata was damaged, so it was cleared and is being read again.
        [server-changed] This connection now reaches a different server, so its cached metadata is being read again.
        [wrong-key] The key that protects this connection's cached metadata has changed, so the cache was cleared and is being read again.
       *[incompatible] The cached metadata was written by a different version, so it is being read again.
    }
# Strip above the explorer while the connection is offline or not yet connected.
# Variables:
#   $date (String) - when the metadata was read, preformatted for the UI locale, e.g. "17 November 2026 14:05".
model-cache-offline = Showing cached metadata from { $date }
# Notice when a policy keeps this connection's metadata out of the cache (memory only).
model-cache-disabled-by-policy = Metadata for this connection isn't cached on disk.

# DDL generation (alg/03 HV-DDL-07, -09, -10, -11). Generated SQL itself is never translated.
# Variables:
#   $dbms (String) - the engine's product name, e.g. "PostgreSQL".
#   $kind (String) - the object kind's label, e.g. "table".
ddl-unsupported-kind = { $dbms } doesn't support generating DDL for this kind of object ({ $kind }).
# Warning on server text that holds MySQL executable comments (`/*! … */`): they run as SQL.
# m4-t25-ddl-view (named exception in m4-t01a-model's block): the binding wording of
# design/m4-explorer NOTES surface 10.
ddl-warn-executable-comment = This text contains MySQL executable comments (/*! … */). MySQL runs what is inside them as SQL.
# Notice when a SQLite change needs the table to be rebuilt (create, copy, drop, rename).
ddl-sqlite-rebuild-notice = SQLite can't change this in place, so the script rebuilds the table and copies its rows.
# Notice when SQLite forces constraints inside the table although another placement was chosen.
ddl-sqlite-placement-forced = SQLite keeps constraints inside the table, so they're placed there.
# Notice when the grants of an object have not been read: the script has no GRANT statements.
ddl-grants-not-loaded = Grants haven't been read for this object, so the script has no GRANT statements.
# Notice when only some grants could be read (the login can't see them all).
ddl-grants-partial = Only the grants this login can see are included; others may exist.
# Notice when a PostgreSQL table has row-level security: its policies are not read in this version.
ddl-policies-not-loaded = Row-level security policies aren't read yet, so the script doesn't include them.

## m4-t19-produce (named exception inside m4-t01a-model's DDL block)
# Warning when a column's type is not in the loaded catalog: the script writes it as the server reported it.
# Variables:
#   $type (String) - the type's name as the server spells it, e.g. "public.citext".
ddl-warn-unknown-type = The type { $type } isn't in the loaded catalog. It's written as the server reported it, so make sure it exists before you run the script.
# Notice when the server doesn't enforce a constraint and this database version can't say so in DDL: the constraint is left out.
# Variables:
#   $name (String) - the constraint's name, or its kind when it has none, e.g. "orders_chk_1".
ddl-warn-not-enforced = The constraint { $name } isn't enforced on the server, and this version can't write NOT ENFORCED, so the script leaves it out.
## m4-t01a-model

# Generator option labels (HV-DDL-09, -10, -11).
ddl-opt-placement = Constraints
ddl-opt-placement-inside-column = Inside column
ddl-opt-placement-inside-table = Inside table
ddl-opt-placement-after-table = After table
ddl-opt-qualify = Qualification
ddl-opt-qualify-auto = Auto
ddl-opt-qualify-never = Never
ddl-opt-qualify-always = Always
ddl-opt-or-replace = Use OR REPLACE
ddl-opt-or-replace-tooltip = Write CREATE OR REPLACE where the database supports it.
ddl-opt-if-not-exists = Use IF NOT EXISTS
ddl-opt-if-not-exists-tooltip = Write IF NOT EXISTS where the database supports it.
ddl-opt-skip-grants = Skip grants
ddl-opt-skip-grants-tooltip = Leave out GRANT statements.
ddl-opt-skip-owner = Skip owner
ddl-opt-skip-owner-tooltip = Leave out ownership statements.
ddl-opt-skip-definer = Skip DEFINER
ddl-opt-skip-definer-tooltip = Leave out the DEFINER clause of MySQL and MariaDB views, routines, triggers and events.
ddl-opt-surrogate-names = Name unnamed constraints
ddl-opt-surrogate-names-tooltip = Write the names the database generated for unnamed constraints.
ddl-opt-server-definitions = Definitions provided by server
ddl-opt-server-definitions-tooltip = Show the definition text the database reports instead of generated DDL.
# Generator categories (HV-DDL-11).
ddl-cat-create = Create
ddl-cat-drop = Drop
ddl-cat-truncate = Truncate
ddl-cat-server-definitions = Definitions provided by server
ddl-cat-alter = Alter

## m4-t01c-core

# Introspection notices (alg/02 HV-INTRO-01..03, -05, -11..13, -15, -16). Codes travel from the
# core; the text is chosen at the edge. Names are user data: the caller isolates them
# (FSI/PDI) and never case-maps or translates them. Counts are Number arguments.

# How long ago a namespace was read.
# Variables:
#   $ago (String) - the relative time, preformatted for the UI locale, e.g. "5 minutes ago".
intro-freshness-loaded-ago = Loaded { $ago }
# Why an object lookup has no answer: the namespace has not been read yet.
intro-unknown-not-loaded = This object hasn't been read yet.
# Why an object lookup has no answer: the namespace is outside the connection's scope.
intro-unknown-out-of-scope = This object is outside the connection's scope, so it isn't read.
# A lookup in loaded metadata found no object of that name.
intro-missing-not-found = No object with this name is in the loaded metadata.
# Back-pressure (HV-INTRO-01 step 7): reading a namespace timed out twice, so its automatic
# level was lowered by one.
# Variables:
#   $name (String) - the namespace, e.g. "public".
#   $level (String) - the new level, a selector: "names" or "structure".
intro-hint-level-lowered = { $level ->
        [names] Reading { $name } timed out twice, so it is now read by name only.
       *[structure] Reading { $name } timed out twice, so it is now read without definitions.
    }
# The one-time prompt before a PostgreSQL scope that reads every database (HV-INTRO-02).
# Variables:
#   $count (Number) - how many databases the server lists, e.g. 12.
intro-scope-all-databases-cost = { $count ->
        [one] Reading { $count } database opens one more connection for it. Read it?
       *[other] Reading all { $count } databases opens one more connection for each. Read them all?
    }
# A scope pattern that does not parse, at a character position (Unicode scalar values).
# Variables:
#   $column (Number) - the 1-based character position, e.g. 7.
intro-scope-syntax-error = The pattern has an error at character { $column }.
# Variables:
#   $column (Number) - the 1-based character position, e.g. 7.
intro-scope-err-unclosed-brace = The pattern has an error at character { $column }: a brace isn't closed.
# Variables:
#   $column (Number) - the 1-based character position, e.g. 7.
intro-scope-err-bad-regex = The pattern has an error at character { $column }: the regular expression isn't valid.
# `@` in a scope pattern when the current database or schema cannot be detected.
intro-scope-current-unknown = The current database or schema couldn't be detected, so @ selects nothing.
# Object filter errors (HV-INTRO-03). Kind words in the grammar are English tokens.
# Variables:
#   $item (String) - the filter item as typed, e.g. "tabel:orders".
intro-filter-unknown-kind = { $item } doesn't name a kind of object.
# Variables:
#   $item (String) - the filter item as typed, e.g. "/a(/".
intro-filter-bad-regex = { $item } isn't a valid regular expression.
# Variables:
#   $item (String) - the filter item as typed, e.g. "a[b".
intro-filter-bad-glob = { $item } isn't a valid name pattern.
# Why a namespace was read in full instead of only its changes (HV-INTRO-05, -07).
# Variables:
#   $reason (String) - the reason code, a selector: "no-watermark", "restart", "identity",
#     "version", "standby", "wraparound", "mass-change" or "schema-rename".
intro-full-reason = { $reason ->
        [no-watermark] Read in full: there was no earlier read to compare with.
        [restart] Read in full: the server restarted since the last read.
        [identity] Read in full: this connection now reaches a different server or database.
        [version] Read in full: the server's version changed.
        [standby] Read in full: the server is a standby, where only full reads are reliable.
        [wraparound] Read in full: too many transactions ran since the last read.
        [mass-change] Read in full: a large part of the schema changed.
       *[schema-rename] Read in full: a schema that other objects refer to was renamed.
    }
# Progress of an incremental read (determinate; repainted when the count changes).
# Variables:
#   $count (Number) - how many changed tables are being read, e.g. 3.
intro-progress-tables-incremental = { $count ->
        [one] Reading { $count } changed table…
       *[other] Reading { $count } changed tables…
    }
# The mass-deletion guard (D-039, HV-INTRO-11).
# Variables:
#   $count (Number) - how many objects are no longer visible, e.g. 120.
#   $schema (String) - the namespace, e.g. "public".
intro-mass-drop-confirm = { $count ->
        [one] { $count } object in { $schema } is no longer visible to this connection. Remove it from the cached metadata?
       *[other] { $count } objects in { $schema } are no longer visible to this connection. Remove them from the cached metadata?
    }
# More than half of a namespace vanished in one pass: the login's privileges may have changed.
# Variables:
#   $schema (String) - the namespace, e.g. "public".
intro-visibility-changed = Most objects in { $schema } are no longer visible to this connection. Its privileges may have changed.
# Badge on a namespace the server changed after it was read.
intro-stale-namespace-badge = out of date
# Variables:
#   $name (String) - the object, e.g. "orders".
intro-stale-object = { $name } changed on the server since it was loaded.
# Variables:
#   $name (String) - the object, e.g. "orders".
intro-dropped-object = { $name } no longer exists on the server.
# An object whose definition refers to something that could not be resolved (kept, flagged).
# Variables:
#   $name (String) - the object, e.g. "orders_pk".
intro-object-broken = { $name } refers to a column that couldn't be resolved.
# Badge on the built-in objects read from a shipped system-catalog snapshot (HV-INTRO-16).
# Variables:
#   $version (String) - the server line the snapshot was made from, e.g. "17".
intro-pre-introspected-badge = Built-in catalog (version { $version })
# Refresh after DDL, "Ask" preference (HV-INTRO-12).
# Variables:
#   $count (Number) - how many objects changed, e.g. 3.
intro-refresh-after-ddl-ask = { $count ->
        [one] Refresh the database explorer to reflect { $count } changed object?
       *[other] Refresh the database explorer to reflect { $count } changed objects?
    }
# A statement whose effects cannot be read in advance (dynamic SQL, DO, CALL, an executable
# comment) refreshed the whole schema.
intro-refresh-broad-dynamic-sql = The script ran SQL whose changes can't be read in advance, so the whole schema is being refreshed.
# A list the server filtered with the object filter (HV-INTRO-03 pushdown).
intro-remote-filtered-notice = The server filtered this list: objects that don't match the filter weren't read.
# Debug-level: whether a non-ASCII name matches depends on the server (R-M4-41).
intro-name-server-dependent = Whether this name matches depends on the server's settings.
# Log only: a read failed because an object was dropped meanwhile, and was retried once
# (PostgreSQL "cache lookup failed").
# Variables:
#   $schema (String) - the namespace, e.g. "public".
intro-catalog-race-retried = An object in { $schema } was dropped while it was being read, so the read was retried.

## m4-t20-scope

# A scope pattern applies, but part of it is ignored (HV-INTRO-02 edge 11): a `db:` prefix on a
# one-level engine (MySQL, MariaDB, SQLite).
# Variables:
#   $column (Number) - the 1-based character position, e.g. 7.
intro-scope-warn-db-term-ignored = The database part at character { NUMBER($column, useGrouping: "false") } is ignored: this engine has no databases above its schemas.
# An object filter applies, but part of it is ignored (HV-INTRO-03 edge 5): an empty item
# between two separators.
# Variables:
#   $column (Number) - the 1-based character position, e.g. 7.
intro-filter-warn-empty-item = The empty part at character { NUMBER($column, useGrouping: "false") } is ignored.

## m4-t24-live
# A cache write failed for lack of disk space; the explorer keeps working from memory (shown
# once per connection; HV-MODEL-03, T11 carry).
model-cache-write-failed = The cached metadata couldn't be saved because the disk is full. The explorer keeps working; the metadata is read again on the next connect.

## m4b-t12-script-run
# Run as script (HV-SCRIPT-12) reads the whole file before it runs anything (R-M4b-47): these
# say why nothing in the file was run. Each names the line of the file where it found the reason.
# Variables:
#   $line (Number) - the 1-based line in the file, e.g. 12.
exec-script-uncertain = Nothing in the file was run: Hacivat can't be sure where the statement on line { NUMBER($line, useGrouping: "false") } ends.
# Variables:
#   $line (Number) - the 1-based line in the file, e.g. 12.
exec-script-too-long = Nothing in the file was run: the statement on line { NUMBER($line, useGrouping: "false") } is longer than 64 MiB.
# R-T8-3: the file has no place to cut it into statements for that long.
# Variables:
#   $line (Number) - the 1-based line in the file, e.g. 12.
exec-script-no-boundary = Nothing in the file was run: no statement boundary within 64 MiB of text from line { NUMBER($line, useGrouping: "false") }. Split the file, or open it and use Run.
# A psql or mysql client command (\c, \i, SOURCE…) that Hacivat doesn't run as the client would.
# Variables:
#   $line (Number) - the 1-based line in the file, e.g. 12.
exec-script-client-command = Nothing in the file was run: line { NUMBER($line, useGrouping: "false") } holds a client command that Run as script doesn't run.
# Variables:
#   $line (Number) - the 1-based line in the file, e.g. 12.
exec-script-params = Nothing in the file was run: the statement on line { NUMBER($line, useGrouping: "false") } has a parameter, and Run as script doesn't ask for values.
# Variables:
#   $line (Number) - the 1-based line in the file, e.g. 12.
exec-script-copy = Nothing in the file was run: line { NUMBER($line, useGrouping: "false") } has COPY … FROM STDIN or TO STDOUT, which needs the client to stream data.
# The file on disk changed between the check and the run.
exec-script-file-changed-before = Nothing in the file was run: the file changed on disk while Hacivat read it.
exec-script-decode-failed = Nothing in the file was run: the file isn't valid text in its encoding.
exec-script-read-failed = Nothing in the file was run: the file couldn't be read.
# The connection reads quotes and escapes differently from the way the tab splits the file.
exec-script-modes-differ = Nothing in the file was run: the connection's settings for quotes and escapes changed. Run it again.
exec-script-cancelled-before = Stopped. Nothing in the file was run.
# The run stopped part-way; the statements before the stop ran.
exec-script-cancelled = Stopped. The statements before the stop ran.
exec-script-file-changed = The file changed on disk, so the run stopped. The statements before the change ran.
# The safety check refused a statement; its own reason follows on its own row.
# Variables:
#   $line (Number) - the 1-based line in the file, e.g. 12.
exec-script-denied = Stopped at line { NUMBER($line, useGrouping: "false") }: the statement wasn't sent. The statements before it ran.
# Variables:
#   $line (Number) - the 1-based line in the file, e.g. 12.
exec-script-confirm-cancelled = Stopped at line { NUMBER($line, useGrouping: "false") }: you cancelled. The statements before it ran.
# Variables:
#   $line (Number) - the 1-based line in the file, e.g. 12.
exec-script-context-changed = Stopped at line { NUMBER($line, useGrouping: "false") }: the connection's settings changed while the file ran. The statements before it ran.
# Variables:
#   $line (Number) - the 1-based line in the file, e.g. 12.
exec-script-context-unconfirmed = Stopped at line { NUMBER($line, useGrouping: "false") }: Hacivat couldn't read the connection's settings, so the statement wasn't sent. The statements before it ran.
# Variables:
#   $line (Number) - the 1-based line in the file, e.g. 12.
exec-script-not-sent = Stopped at line { NUMBER($line, useGrouping: "false") }: the connection didn't run the statement. The statements before it ran.
# ## m4b-t12-script-run fix round 1
# The gate refuses a statement whatever the user answers (a read-only connection, a statement
# Hacivat couldn't check): nothing in the file runs. The gate's own reason follows on its row.
# Variables:
#   $line (Number) - the 1-based line in the file, e.g. 12.
exec-script-denied-before = Nothing in the file was run: the statement on line { NUMBER($line, useGrouping: "false") } wouldn't be sent.
# The profile's environment or read-only setting changed between the check and the send.
# Variables:
#   $line (Number) - the 1-based line in the file, e.g. 12.
exec-script-connection-changed = Stopped at line { NUMBER($line, useGrouping: "false") }: the connection's environment or read-only setting changed. The statements before it ran.
# The connection was lost or reset mid-run; the new connection lacks what the file set.
# Variables:
#   $n (Number) - how many statements of the file were handled before the stop, e.g. 41.
exec-script-reconnected = The connection was reset, so the script stopped after statement { NUMBER($n, useGrouping: "false") }. Nothing after it was sent.
# The file read differently the second time (never on an unchanged file).
# Variables:
#   $line (Number) - the 1-based line in the file, e.g. 12.
exec-script-split-changed = Stopped at line { NUMBER($line, useGrouping: "false") }: the file read differently the second time. The statements before it ran.
exec-script-decode-failed-mid = Stopped: the rest of the file isn't valid text in its encoding. The statements before it ran.
exec-script-read-failed-mid = Stopped: the rest of the file couldn't be read. The statements before it ran.

## m4b-t18-run-as-script
# Run as script (HV-SCRIPT-12) in the app: the T12 re-review's wording (n1, n3, n5) and the
# tab's database check before anything runs (R-M4b-27).
# The item cap (R-T8-3): a line with too many statements and no line break.
# Variables:
#   $line (Number) - the 1-based line in the file, e.g. 12.
exec-script-no-boundary-items = Nothing in the file was run: too many statements without a line break (more than 65,536 statements on line { NUMBER($line, useGrouping: "false") }). Split the file, or open it and use Run.
# The connection's lexing modes could not be read, and the file splits differently under some of them.
exec-script-modes-unknown = Nothing in the file was run: Hacivat couldn't read the connection's settings for quotes and escapes, and the file reads differently under the settings this server may have.
# The check before the run: the session must be on the database the tab was opened under.
exec-script-database-check = Nothing in the file was run: the connection isn't on this tab's database, or Hacivat couldn't check which database it's on.
# The run's view of the connection's events fell behind, so it can't tell whether the connection was reset.
# Variables:
#   $n (Number) - how many statements of the file were handled before the stop, e.g. 41.
exec-script-lost-track = Hacivat lost track of the connection's state, so the script stopped after statement { NUMBER($n, useGrouping: "false") }. Nothing after it was sent.
