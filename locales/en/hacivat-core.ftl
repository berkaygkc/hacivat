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
# Run refused to send a selection as one statement.
exec-run-as-one-unsupported = Not sent: Hacivat runs a selection statement by statement, and can't send it as one.

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
