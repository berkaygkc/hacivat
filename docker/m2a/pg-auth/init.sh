#!/usr/bin/env bash
# pg17-auth (Task 16): the md5 and cleartext-password fixture roles. password_encryption is
# md5 only while these two roles are created (a session setting); the server default stays
# scram-sha-256. Runs once at initdb; the entrypoint restarts the server afterwards, so the
# new pg_hba.conf is read before the healthcheck can pass.
set -e

psql -v ON_ERROR_STOP=1 --username "$POSTGRES_USER" --dbname "$POSTGRES_DB" <<'SQL'
SET password_encryption = 'md5';
CREATE ROLE hv_md5 LOGIN PASSWORD 'hv_test_pw';
CREATE ROLE hv_clear LOGIN PASSWORD 'hv_test_pw';
SQL

cp /hv-pg-auth/pg_hba.conf "$PGDATA/pg_hba.conf"
