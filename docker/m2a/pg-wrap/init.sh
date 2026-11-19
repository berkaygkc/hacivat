#!/usr/bin/env bash
# pg17wrap's entrypoint (M4 Task 1c, R-M4-43). On an empty data directory it builds a cluster
# whose next transaction id is 2^31 + 2000, with every row written before the jump frozen
# (ver/01 #pg-xmin-freeze: freezing keeps the raw xmin, so after the jump those rows have a
# negative `age(xmin)`). Then it hands over to the image's own entrypoint.
#
# The stock entrypoint initialises only an empty PGDATA, so this script does what it would:
# the `hv_test` superuser with the throwaway password, scram-sha-256 over TCP, the database
# `hv_test` and the shared fixtures (`init/pg.sql`, mounted where the other PG services mount
# it). Steps, all on a server that listens on its Unix socket only:
#   1. initdb; start; test role, databases and fixtures; `CREATE TABLE w(a int)`;
#   2. VACUUM FREEZE in every database (template0 made connectable for it first); stop;
#   3. pre-create the pg_xact segment of the new next id; `pg_resetwal -x 2^31 + 2000`;
#   4. start; VACUUM FREEZE in every database again, so each datfrozenxid moves past the jump
#      and the server assigns new ids (writes work); template0 back to not connectable; check
#      the jump and that an id can be assigned; stop;
#   5. write the marker, then exec the stock entrypoint (the server on 5432).
# A data directory without the marker is a cluster whose set-up did not finish: the script
# refuses to start it (remove the container's volume and start again).
set -Eeuo pipefail

MARKER="hv-pg-wrap.done"
JUMP=$(( 2147483648 + 2000 ))

if [ "$(id -u)" = "0" ]; then
    mkdir -p "$PGDATA"
    chown postgres:postgres "$PGDATA"
    chmod 0700 "$PGDATA"
    exec gosu postgres "${BASH_SOURCE[0]}" "$@"
fi

if [ -s "$PGDATA/PG_VERSION" ]; then
    if [ ! -f "$PGDATA/$MARKER" ]; then
        echo "pg17wrap: $PGDATA holds a cluster whose set-up did not finish; remove the volume" >&2
        exit 1
    fi
    exec docker-entrypoint.sh "$@"
fi

user="${POSTGRES_USER:?POSTGRES_USER is required}"
db="${POSTGRES_DB:?POSTGRES_DB is required}"
: "${POSTGRES_PASSWORD:?POSTGRES_PASSWORD is required}"

local_start() {
    pg_ctl -D "$PGDATA" -w -o "-c listen_addresses='' -p 5432" start >/dev/null
}

local_stop() {
    pg_ctl -D "$PGDATA" -w -m fast stop >/dev/null
}

sql() {
    local database="$1"
    shift
    psql -X -v ON_ERROR_STOP=1 -q -U "$user" -d "$database" "$@"
}

# Every database, template0 included (it is connectable while this script runs). The list is
# read into an array first: a failing command substitution in a `for` list would freeze
# nothing without failing, so the listing's own status is checked, and the list must hold the
# test database and template0.
freeze_all() {
    local d
    local -a dbs
    if ! mapfile -t dbs < <(sql postgres -At -c "SELECT datname FROM pg_catalog.pg_database ORDER BY datname"); then
        echo "pg17wrap: could not read the database list" >&2
        exit 1
    fi
    wait $! || { echo "pg17wrap: listing the databases failed" >&2; exit 1; }
    if [[ " ${dbs[*]} " != *" $db "* || " ${dbs[*]} " != *" template0 "* ]]; then
        echo "pg17wrap: the database list lacks $db or template0: ${dbs[*]}" >&2
        exit 1
    fi
    for d in "${dbs[@]}"; do
        sql "$d" -c "VACUUM (FREEZE)"
    done
}

# 1. The cluster, its role, databases and fixtures.
initdb -D "$PGDATA" --username="$user" --pwfile=<(printf '%s\n' "$POSTGRES_PASSWORD") \
    --auth-local=trust --auth-host=scram-sha-256 >/dev/null
printf '\n# pg17wrap: the test user over TCP from the compose network\nhost all all all scram-sha-256\n' \
    >> "$PGDATA/pg_hba.conf"
local_start
sql postgres -c "CREATE DATABASE \"$db\" OWNER \"$user\""
for f in /docker-entrypoint-initdb.d/*.sql; do
    [ -e "$f" ] || continue
    sql "$db" -f "$f"
done
sql "$db" -c "CREATE TABLE w(a int)"

# 2. Freeze everything written so far.
sql postgres -c "ALTER DATABASE template0 WITH ALLOW_CONNECTIONS true"
freeze_all
local_stop

# 3. The jump. One pg_xact segment file holds 2^20 transaction ids (32 pages of 8 KiB, four
#    ids per byte); the segment of the new next id must exist before the server starts.
segment=$(printf '%04X' $(( JUMP / 1048576 )))
dd if=/dev/zero of="$PGDATA/pg_xact/$segment" bs=262144 count=1 status=none
pg_resetwal -x "$JUMP" -D "$PGDATA" >/dev/null

# 4. Move every datfrozenxid past the jump, so new ids can be assigned; then check.
local_start
freeze_all
sql postgres -c "ALTER DATABASE template0 WITH ALLOW_CONNECTIONS false"
xmax=$(sql "$db" -At -c "SELECT pg_catalog.pg_snapshot_xmax(pg_catalog.pg_current_snapshot())::text")
if [ "$xmax" -le 2147483648 ]; then
    echo "pg17wrap: the next transaction id is $xmax, not past 2^31" >&2
    exit 1
fi
sql "$db" -c "BEGIN; SELECT pg_catalog.pg_current_xact_id(); ROLLBACK;" >/dev/null
local_stop

# 5. Done: the stock entrypoint finds PGDATA initialised and starts the server on 5432.
touch "$PGDATA/$MARKER"
exec docker-entrypoint.sh "$@"
