-- Hacivat query pack: PostgreSQL, file 50-joined (HV-INTRO-04, R-M4-32).
-- Contents: the fragment read of a joined connection (R-M4-29, R-M4-46; Task 15b).
-- Written by M4 Task 15b from the PostgreSQL documentation only (system catalogs, chapter 51;
-- system information functions, section 9.26; the OPERATOR() syntax of section 4.2.5).
-- No third-party catalog SQL.
--
-- These sections run inside a transaction Hacivat owns on a job session (M7's drift check in
-- its apply transaction), whose search_path Hacivat cannot pin (no SET there, R-M4-7). So:
-- - nothing resolves through the path: every operator is OPERATOR(pg_catalog.op), every cast
--   names a pg_catalog type, every function is pg_catalog-qualified (T16 I-4, the
--   CVE-2018-1058 class). The IN of the oid list bind is the one exception: the pack grammar
--   admits a list bind only as <column> IN :[name] (chunking stays sound), IN has no
--   OPERATOR() form, and OPERATOR(pg_catalog.=) ANY needs an array bind (R-M4-3). So the
--   reader refuses a session whose current_schemas(true) does not start with pg_catalog once
--   the session's own temp schema (pg_my_temp_schema(), read in the same row) is skipped,
--   before it sends anything else (R-W3-11, corrected by R-W5-5: another backend's
--   pg_temp_N is searched for operators, so it is refused);
-- - no column is server-deparsed text (format_type, pg_get_expr, pg_get_*def qualify names by
--   the path): a column's type is its schema and name (R-M4-46); defaults and generation
--   expressions are not read;
-- - no name reaches the server: the reader picks the schema's oid from JoinedNamespaces and
--   the caller's relations from JoinedRelations by exact name, on the client (an object the
--   transaction created has no oid the caller knows); every bind is an oid list.

--- name: JoinedContext ---
--- returns: db text, nsp text, own_temp text ---
--- joinable ---
SELECT CAST(pg_catalog.current_database() AS pg_catalog.text) AS db,
  CAST(p.p AS pg_catalog.text) AS nsp,
  (SELECT CAST(t.nspname AS pg_catalog.text) FROM pg_catalog.pg_namespace t
    WHERE t.oid OPERATOR(pg_catalog.=) pg_catalog.pg_my_temp_schema()) AS own_temp
FROM pg_catalog.unnest(pg_catalog.current_schemas(true)) WITH ORDINALITY AS p
ORDER BY p.ordinality

--- name: JoinedNamespaces ---
--- returns: oid u32, name text ---
--- joinable ---
SELECT n.oid AS oid, CAST(n.nspname AS pg_catalog.text) AS name
FROM pg_catalog.pg_namespace n
ORDER BY n.oid

--- name: JoinedRelations ---
--- returns: oid u32, name text, relkind text ---
--- joinable ---
SELECT c.oid AS oid, CAST(c.relname AS pg_catalog.text) AS name,
  CAST(c.relkind AS pg_catalog.text) AS relkind
FROM pg_catalog.pg_class c
WHERE (c.relkind OPERATOR(pg_catalog.=) 'r'
    OR c.relkind OPERATOR(pg_catalog.=) 'p'
    OR c.relkind OPERATOR(pg_catalog.=) 'v'
    OR c.relkind OPERATOR(pg_catalog.=) 'm'
    OR c.relkind OPERATOR(pg_catalog.=) 'f')
  AND c.relnamespace IN :[oids]
ORDER BY c.oid

--- name: JoinedColumns ---
--- returns: relid u32, attnum i16, name text, type_schema text, type_name text, not_null bool, identity_kind text ---
--- joinable ---
SELECT a.attrelid AS relid, a.attnum AS attnum, CAST(a.attname AS pg_catalog.text) AS name,
  CAST(tn.nspname AS pg_catalog.text) AS type_schema,
  CAST(t.typname AS pg_catalog.text) AS type_name,
  a.attnotnull AS not_null,
  CAST(a.attidentity AS pg_catalog.text) AS identity_kind
FROM pg_catalog.pg_attribute a
JOIN pg_catalog.pg_type t ON t.oid OPERATOR(pg_catalog.=) a.atttypid
JOIN pg_catalog.pg_namespace tn ON tn.oid OPERATOR(pg_catalog.=) t.typnamespace
WHERE a.attnum OPERATOR(pg_catalog.>) 0 AND NOT a.attisdropped
  AND a.attrelid IN :[oids]
ORDER BY a.attrelid, a.attnum
