-- Hacivat query pack: PostgreSQL, file 40-system (HV-INTRO-04, R-M4-32, R-M4-44).
-- Contents: system-mode sections (Mode::System) for the minicat snapshots (Task 23) and for
-- the live system introspection of versions without a snapshot (R-M4-6). Written by M4
-- Task 16 from the PostgreSQL documentation only (system catalogs, chapter 51; pg_settings,
-- section 52.24); no third-party catalog SQL.
--
-- Only built-in objects: OID below 16384 (FirstNormalObjectId), read from the database
-- postgres with search_path = pg_catalog (R-M4-17). :ns is pg_catalog's or
-- information_schema's oid. No server id, watermark or timestamp is a payload here; the
-- generator drops the oids.

--- name: SystemRelations ---
--- returns: oid u32, name text, relkind text, comment text ---
SELECT c.oid AS oid,
  c.relname::text AS name,
  c.relkind::text AS relkind,
  d.description AS comment
FROM pg_catalog.pg_class c
LEFT JOIN pg_catalog.pg_description d ON d.classoid = 'pg_catalog.pg_class'::pg_catalog.regclass
  AND d.objoid = c.oid AND d.objsubid = 0
WHERE c.relnamespace = :ns AND c.oid < 16384 AND c.relkind IN ('r', 'p', 'v', 'm', 'f')
ORDER BY c.relname

--- name: SystemColumns ---
--- returns: relid u32, attnum i16, name text, type_text text, not_null bool ---
SELECT a.attrelid AS relid,
  a.attnum AS attnum,
  a.attname::text AS name,
  pg_catalog.format_type(a.atttypid, a.atttypmod) AS type_text,
  a.attnotnull AS not_null
FROM pg_catalog.pg_attribute a
JOIN pg_catalog.pg_class c ON c.oid = a.attrelid
WHERE c.relnamespace = :ns AND c.oid < 16384 AND c.relkind IN ('r', 'p', 'v', 'm', 'f')
  AND a.attnum > 0 AND NOT a.attisdropped
ORDER BY a.attrelid, a.attnum

-- Routines: arguments, identity arguments (only when they differ), result, kind,
-- volatility, set-returning, description.
--- name: SystemRoutines ---
--- returns: oid u32, name text, prokind text, args text, identity_args text, result_type text, volatility text, returns_set bool, comment text ---
SELECT p.oid AS oid,
  p.proname::text AS name,
  p.prokind::text AS prokind,
  pg_catalog.pg_get_function_arguments(p.oid) AS args,
  CASE WHEN pg_catalog.pg_get_function_identity_arguments(p.oid) <> pg_catalog.pg_get_function_arguments(p.oid)
    THEN pg_catalog.pg_get_function_identity_arguments(p.oid) END AS identity_args,
  CASE WHEN p.prokind <> 'p' THEN pg_catalog.pg_get_function_result(p.oid) END AS result_type,
  p.provolatile::text AS volatility,
  p.proretset AS returns_set,
  d.description AS comment
FROM pg_catalog.pg_proc p
LEFT JOIN pg_catalog.pg_description d ON d.classoid = 'pg_catalog.pg_proc'::pg_catalog.regclass
  AND d.objoid = p.oid AND d.objsubid = 0
WHERE p.pronamespace = :ns AND p.oid < 16384
ORDER BY p.proname, p.oid

-- Operators: kind (b infix, l prefix), operand and result types, implementing function.
--- name: SystemOperators ---
--- returns: oid u32, name text, oprkind text, left_type text, right_type text, result_type text, function_name text, comment text ---
SELECT o.oid AS oid,
  o.oprname::text AS name,
  o.oprkind::text AS oprkind,
  CASE WHEN o.oprleft <> 0 THEN pg_catalog.format_type(o.oprleft, NULL) END AS left_type,
  CASE WHEN o.oprright <> 0 THEN pg_catalog.format_type(o.oprright, NULL) END AS right_type,
  pg_catalog.format_type(o.oprresult, NULL) AS result_type,
  o.oprcode::text AS function_name,
  d.description AS comment
FROM pg_catalog.pg_operator o
LEFT JOIN pg_catalog.pg_description d ON d.classoid = 'pg_catalog.pg_operator'::pg_catalog.regclass
  AND d.objoid = o.oid AND d.objsubid = 0
WHERE o.oprnamespace = :ns AND o.oid < 16384
ORDER BY o.oprname, o.oid

-- Types: SQL name, kind, category, array element, description. Relation row types are left
-- out (composites only for relkind c).
--- name: SystemTypes ---
--- returns: oid u32, name text, sql_name text, typtype text, category text, element text, comment text ---
SELECT t.oid AS oid,
  t.typname::text AS name,
  pg_catalog.format_type(t.oid, NULL) AS sql_name,
  t.typtype::text AS typtype,
  t.typcategory::text AS category,
  CASE WHEN t.typlen = -1 AND t.typelem <> 0 THEN pg_catalog.format_type(t.typelem, NULL) END AS element,
  d.description AS comment
FROM pg_catalog.pg_type t
LEFT JOIN pg_catalog.pg_description d ON d.classoid = 'pg_catalog.pg_type'::pg_catalog.regclass
  AND d.objoid = t.oid AND d.objsubid = 0
WHERE t.typnamespace = :ns AND t.oid < 16384
  AND (t.typrelid = 0 OR EXISTS (SELECT 1 FROM pg_catalog.pg_class r WHERE r.oid = t.typrelid AND r.relkind = 'c'))
ORDER BY t.typname, t.oid

-- GUC names with their type and context, never their values. Names with a dot (extension
-- and custom settings) are left out (R-M4-17).
--- name: ListSettings ---
--- returns: name text, vartype text, context text ---
SELECT s.name AS name,
  s.vartype AS vartype,
  s.context AS context
FROM pg_catalog.pg_settings s
WHERE s.name NOT LIKE '%.%'
ORDER BY s.name
