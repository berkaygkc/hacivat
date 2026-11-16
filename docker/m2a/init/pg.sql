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

-- ## m2b-t23-f1
-- Task 23 finding F1: PUBLIC may execute pg_logical_emit_message, whose non-transactional form
-- writes a WAL record no rollback undoes, so the role probe reads every role as guarded until
-- it is revoked (the remediation's third optional line). Its signatures gained `flush` in 17,
-- so every overload is revoked by name. Function privileges are per database: hv_test only
-- (the bypass suite's own database starts from the default and revokes it itself).
DO $$
DECLARE
    f regprocedure;
BEGIN
    FOR f IN
        SELECT p.oid FROM pg_proc p
        WHERE p.pronamespace = 'pg_catalog'::regnamespace AND p.proname = 'pg_logical_emit_message'
    LOOP
        EXECUTE format('REVOKE EXECUTE ON FUNCTION %s FROM PUBLIC', f);
    END LOOP;
END
$$;
