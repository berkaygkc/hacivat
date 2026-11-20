-- Hacivat query pack: PostgreSQL, file 45-system-extras (HV-INTRO-04, R-M4-32; M4 Task 26,
-- ruling R-W5-6). Written from the PostgreSQL documentation only (the System Catalogs chapter;
-- the OID assignment rule of the Bootstrap chapter: objects created by
-- initdb have an OID below FirstNormalObjectId, 16384); no third-party catalog SQL.
--
-- A shipped system-catalog snapshot holds only the objects initdb creates (R-M4-17). Whether a
-- name it lacks is certainly absent therefore needs this fact: no relation, routine, type or
-- operator with an OID of 16384 or more lives in pg_catalog or information_schema (an
-- extension installed into pg_catalog, a superuser object, a managed service addition).
-- Read once per connect, in the same metadata snapshot as Identity. Every catalog is written
-- pg_catalog.<name>; the schema names are constants of the documentation.

--- name: SystemExtras ---
--- returns: extras bool ---
SELECT (
  EXISTS (SELECT 1 FROM pg_catalog.pg_class c
    JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
    WHERE n.nspname IN ('pg_catalog'::pg_catalog.name, 'information_schema'::pg_catalog.name)
      AND c.oid >= 16384::pg_catalog.oid)
  OR EXISTS (SELECT 1 FROM pg_catalog.pg_proc p
    JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace
    WHERE n.nspname IN ('pg_catalog'::pg_catalog.name, 'information_schema'::pg_catalog.name)
      AND p.oid >= 16384::pg_catalog.oid)
  OR EXISTS (SELECT 1 FROM pg_catalog.pg_type t
    JOIN pg_catalog.pg_namespace n ON n.oid = t.typnamespace
    WHERE n.nspname IN ('pg_catalog'::pg_catalog.name, 'information_schema'::pg_catalog.name)
      AND t.oid >= 16384::pg_catalog.oid)
  OR EXISTS (SELECT 1 FROM pg_catalog.pg_operator o
    JOIN pg_catalog.pg_namespace n ON n.oid = o.oprnamespace
    WHERE n.nspname IN ('pg_catalog'::pg_catalog.name, 'information_schema'::pg_catalog.name)
      AND o.oid >= 16384::pg_catalog.oid)
) AS extras
