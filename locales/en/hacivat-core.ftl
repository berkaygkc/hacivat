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

# A duration under one millisecond.
exec-dur-lt-ms = <1 ms
# A duration under one second.
# Variables:
#   $ms (String) - the milliseconds, e.g. "340".
exec-dur-ms = { $ms } ms
# A duration of whole seconds, under a minute.
# Variables:
#   $s (String) - the seconds, e.g. "2".
exec-dur-s = { $s } s
# A duration of seconds and milliseconds, under a minute.
# Variables:
#   $s (String) - the seconds, e.g. "2".
#   $ms (String) - the milliseconds past them, e.g. "340".
exec-dur-s-ms = { $s } s { $ms } ms
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
#   $duration (String) - how long the run took (an exec-dur-* message), e.g. "1 s 200 ms".
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
#   $duration (String) - how long it ran (an exec-dur-* message), e.g. "2 s".
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
# Run refused to send a selection as one statement.
exec-run-as-one-unsupported = Not sent: Hacivat runs a selection statement by statement, and can't send it as one.

# A released console cursor (R-M3-23): the reason, and, when Hacivat's own transaction around
# it ended in a rollback, the sentence that says so (cursor-closed-rolled-back).

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
        [one] Cursor closed after { $rows } row: the server ended the session.
       *[other] Cursor closed after { $rows } rows: the server ended the session.
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
        [one] Cursor closed after { $rows } row: the server ended the session.
       *[other] Cursor closed after { $rows } rows: the server ended the session.
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
