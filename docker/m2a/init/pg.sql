-- M2a fixtures on every PostgreSQL server (R17): the extra databases the database picker
-- and the explorer list. Runs once, at initdb, as the `hv_test` superuser.
CREATE DATABASE orders OWNER hv_test;
CREATE DATABASE orders_archive OWNER hv_test;

-- ## m2b-t16-fixtures
-- Runs connected to hv_test (the entrypoint's POSTGRES_DB).
CREATE TABLE IF NOT EXISTS hv_probe (a int);
CREATE TABLE IF NOT EXISTS hv_probe_log (id serial PRIMARY KEY, v int);

-- Read-only role (Q21: the role is inherited, so the grant applies). pg13 has no
-- pg_read_all_data (added in 14), so it gets SELECT on the existing tables instead. CREATE on
-- the public schema is revoked from PUBLIC so `hv_ro` stays read-only on pg13 and pg14, where
-- PUBLIC may create there.
CREATE ROLE hv_ro LOGIN PASSWORD 'hv_test_pw';
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'pg_read_all_data') THEN
        EXECUTE 'GRANT pg_read_all_data TO hv_ro';
    ELSE
        EXECUTE 'GRANT SELECT ON ALL TABLES IN SCHEMA public TO hv_ro';
    END IF;
END
$$;
REVOKE TEMP ON DATABASE hv_test FROM PUBLIC;
REVOKE CREATE ON SCHEMA public FROM PUBLIC;
