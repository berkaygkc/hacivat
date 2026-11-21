-- Hacivat query pack: PostgreSQL, file 10-core (HV-INTRO-04, R-M4-32).
-- Contents: identity, namespaces, existence, change detection and the core object passes.
-- Written by M4 Task 16 from the PostgreSQL documentation only (system catalogs, chapter 51;
-- system information functions, section 9.26); no third-party catalog SQL.
--
-- Rules every section follows (R-M4-7, R-M4-8, R-M4-46, R-M4-48):
-- - every catalog and catalog function is written pg_catalog.<name>, so a hostile search_path
--   cannot shadow it (each pass also pins search_path = pg_catalog, pg_temp);
-- - objects are selected by oid only; the namespace is :ns::pg_catalog.oid (an oid bind) and the one name
--   that reaches the server is the schema name of Identity, as a bind;
-- - incremental and fragment reads add "AND <alias>.oid IN :[oids]" (a chunked list bind);
-- - a young row is pg_catalog.age(x.xmin) BETWEEN 0 AND pg_catalog.age(:w::pg_catalog.xid),
--   where :w is the previous watermark modulo 2^32 (finding 6: both ages count from the
--   same stable reference XID, so the bound is exact);
-- - nothing here assigns an XID; values whose text form matters are cast to text.

--- name: Identity ---
--- returns: version i32, in_recovery bool, start_epoch text, db_oid u32, db_name text, system_identifier text, timeline text, snap_xmin text, snap_xmax text, ns_oid u32, version_text text, encoding text, standard_conforming_strings text, db_collation text ---
SELECT pg_catalog.current_setting('server_version_num')::integer AS version,
  pg_catalog.pg_is_in_recovery() AS in_recovery,
  extract(epoch FROM pg_catalog.pg_postmaster_start_time())::text AS start_epoch,
  d.oid AS db_oid,
  d.datname::text AS db_name,
  CASE WHEN pg_catalog.has_function_privilege('pg_catalog.pg_control_system()', 'EXECUTE')
    THEN (SELECT s.system_identifier::text FROM pg_catalog.pg_control_system() s) END AS system_identifier,
  CASE WHEN pg_catalog.has_function_privilege('pg_catalog.pg_control_checkpoint()', 'EXECUTE')
    THEN (SELECT k.timeline_id::text FROM pg_catalog.pg_control_checkpoint() k) END AS timeline,
  pg_catalog.pg_snapshot_xmin(pg_catalog.pg_current_snapshot())::text AS snap_xmin,
  pg_catalog.pg_snapshot_xmax(pg_catalog.pg_current_snapshot())::text AS snap_xmax,
  (SELECT n.oid FROM pg_catalog.pg_namespace n WHERE n.nspname = :schema::pg_catalog.name) AS ns_oid,
  pg_catalog.current_setting('server_version') AS version_text,
  pg_catalog.pg_encoding_to_char(d.encoding)::text AS encoding,
  pg_catalog.current_setting('standard_conforming_strings') AS standard_conforming_strings,
  d.datcollate::text AS db_collation
FROM pg_catalog.pg_database d
WHERE d.datname = pg_catalog.current_database()

--- name: ListNamespaces ---
--- returns: oid u32, name text, owner text, comment text, db_name text ---
SELECT n.oid AS oid,
  n.nspname::text AS name,
  pg_catalog.pg_get_userbyid(n.nspowner)::text AS owner,
  d.description AS comment,
  pg_catalog.current_database()::text AS db_name
FROM pg_catalog.pg_namespace n
LEFT JOIN pg_catalog.pg_description d ON d.classoid = 'pg_catalog.pg_namespace'::pg_catalog.regclass
  AND d.objoid = n.oid AND d.objsubid = 0
ORDER BY n.nspname

-- The databases a login may connect to (database-level namespaces: a scope that names
-- another database loads it on its own connection).
--- name: ListDatabases ---
--- returns: name text ---
SELECT d.datname::text AS name
FROM pg_catalog.pg_database d
WHERE d.datallowconn AND NOT d.datistemplate
  AND pg_catalog.has_database_privilege(d.oid, 'CONNECT')
ORDER BY d.datname

--- name: RetrieveNamespace ---
--- returns: oid u32, name text, owner text, comment text ---
SELECT n.oid AS oid,
  n.nspname::text AS name,
  pg_catalog.pg_get_userbyid(n.nspowner)::text AS owner,
  d.description AS comment
FROM pg_catalog.pg_namespace n
LEFT JOIN pg_catalog.pg_description d ON d.classoid = 'pg_catalog.pg_namespace'::pg_catalog.regclass
  AND d.objoid = n.oid AND d.objsubid = 0
WHERE n.oid = :ns::pg_catalog.oid

-- Every object of the namespace by tag: rel (relation, sub = relkind), seq, idx (parent =
-- table), con (parent = table, sub = contype), trg (parent = table), fn (sub = prokind),
-- typ (sub = typtype, a colon, then the count of its domain constraints).
--- name: ListExistent ---
--- returns: tag text, oid u32, parent text, name text, sub text ---
SELECT 'rel' AS tag, c.oid AS oid, NULL::text AS parent, c.relname::text AS name, c.relkind::text AS sub
FROM pg_catalog.pg_class c
WHERE c.relnamespace = :ns::pg_catalog.oid AND c.relkind IN ('r', 'p', 'v', 'm', 'f')
  AND (:name_like::pg_catalog.text IS NULL OR c.relname LIKE :name_like::pg_catalog.text ESCAPE :like_escape::pg_catalog.text)
UNION ALL
SELECT 'seq', c.oid, NULL::text, c.relname::text, NULL::text
FROM pg_catalog.pg_class c
WHERE c.relnamespace = :ns::pg_catalog.oid AND c.relkind = 'S'
UNION ALL
SELECT 'idx', i.indexrelid, c.relname::text, ic.relname::text, NULL::text
FROM pg_catalog.pg_index i
JOIN pg_catalog.pg_class c ON c.oid = i.indrelid
JOIN pg_catalog.pg_class ic ON ic.oid = i.indexrelid
WHERE c.relnamespace = :ns::pg_catalog.oid AND c.relkind IN ('r', 'p', 'v', 'm', 'f')
  AND (:name_like::pg_catalog.text IS NULL OR c.relname LIKE :name_like::pg_catalog.text ESCAPE :like_escape::pg_catalog.text)
UNION ALL
SELECT 'con', k.oid, c.relname::text, k.conname::text, k.contype::text
FROM pg_catalog.pg_constraint k
JOIN pg_catalog.pg_class c ON c.oid = k.conrelid
WHERE c.relnamespace = :ns::pg_catalog.oid AND c.relkind IN ('r', 'p', 'v', 'm', 'f') AND k.contype IN ('p', 'u', 'f', 'c', 'x')
  AND (:name_like::pg_catalog.text IS NULL OR c.relname LIKE :name_like::pg_catalog.text ESCAPE :like_escape::pg_catalog.text)
UNION ALL
SELECT 'trg', t.oid, c.relname::text, t.tgname::text, NULL::text
FROM pg_catalog.pg_trigger t
JOIN pg_catalog.pg_class c ON c.oid = t.tgrelid
WHERE c.relnamespace = :ns::pg_catalog.oid AND c.relkind IN ('r', 'p', 'v', 'm', 'f') AND NOT t.tgisinternal
  AND (:name_like::pg_catalog.text IS NULL OR c.relname LIKE :name_like::pg_catalog.text ESCAPE :like_escape::pg_catalog.text)
UNION ALL
SELECT 'fn', p.oid, NULL::text, p.proname::text, p.prokind::text
FROM pg_catalog.pg_proc p
WHERE p.pronamespace = :ns::pg_catalog.oid
UNION ALL
SELECT 'typ', t.oid, NULL::text, t.typname::text,
  t.typtype::text || ':' || (SELECT pg_catalog.count(*) FROM pg_catalog.pg_constraint dk WHERE dk.contypid = t.oid)::text
FROM pg_catalog.pg_type t
WHERE t.typnamespace = :ns::pg_catalog.oid AND t.typtype IN ('b', 'c', 'd', 'e', 'r')
  AND NOT EXISTS (SELECT 1 FROM pg_catalog.pg_type e WHERE e.typarray = t.oid)
  AND (t.typtype <> 'c' OR EXISTS (SELECT 1 FROM pg_catalog.pg_class r WHERE r.oid = t.typrelid AND r.relkind = 'c'))

-- The (class tag, objoid, objsubid) key of every pg_description row of the objects of the
-- namespace. COMMENT ... IS NULL deletes the row (finding 3): only this existence check sees it.
-- Tags: c pg_class (relation, column by objsubid, index, sequence), k constraint, t trigger,
-- p routine, y type, n the schema.
--- name: ListExistentComments ---
--- returns: tag text, objoid u32, objsubid i32 ---
SELECT CASE d.classoid
    WHEN 'pg_catalog.pg_class'::pg_catalog.regclass THEN 'c'
    WHEN 'pg_catalog.pg_constraint'::pg_catalog.regclass THEN 'k'
    WHEN 'pg_catalog.pg_trigger'::pg_catalog.regclass THEN 't'
    WHEN 'pg_catalog.pg_proc'::pg_catalog.regclass THEN 'p'
    WHEN 'pg_catalog.pg_type'::pg_catalog.regclass THEN 'y'
    ELSE 'n' END AS tag,
  d.objoid AS objoid,
  d.objsubid AS objsubid
FROM pg_catalog.pg_description d
WHERE (d.classoid = 'pg_catalog.pg_class'::pg_catalog.regclass
    AND d.objoid IN (SELECT c.oid FROM pg_catalog.pg_class c WHERE c.relnamespace = :ns::pg_catalog.oid))
  OR (d.classoid = 'pg_catalog.pg_constraint'::pg_catalog.regclass
    AND d.objoid IN (SELECT k.oid FROM pg_catalog.pg_constraint k WHERE k.connamespace = :ns::pg_catalog.oid))
  OR (d.classoid = 'pg_catalog.pg_trigger'::pg_catalog.regclass
    AND d.objoid IN (SELECT t.oid FROM pg_catalog.pg_trigger t JOIN pg_catalog.pg_class c ON c.oid = t.tgrelid WHERE c.relnamespace = :ns::pg_catalog.oid))
  OR (d.classoid = 'pg_catalog.pg_proc'::pg_catalog.regclass
    AND d.objoid IN (SELECT p.oid FROM pg_catalog.pg_proc p WHERE p.pronamespace = :ns::pg_catalog.oid))
  OR (d.classoid = 'pg_catalog.pg_type'::pg_catalog.regclass
    AND d.objoid IN (SELECT t.oid FROM pg_catalog.pg_type t WHERE t.typnamespace = :ns::pg_catalog.oid))
  OR (d.classoid = 'pg_catalog.pg_namespace'::pg_catalog.regclass AND d.objoid = :ns::pg_catalog.oid)

-- (facet, owner oid) for every catalog row of the namespace written at or after the
-- watermark (the verified per-DDL matrix, ver/01 #pg-xmin, PG spike section 4). Facets: rel
-- (the own row of the relation, view rule, inheritance, partition key), columns (pg_attribute,
-- pg_attrdef), indexes, constraints, triggers, comments (relation and column comments),
-- sequence, routine, type (pg_type, pg_enum, domain constraints, pg_range, a composite
-- type: its class row and attributes, comments) and schema. A comment row goes to the facet
-- of its owner.
--- name: ChangedParents ---
--- returns: facet text, oid u32 ---
WITH rels AS MATERIALIZED (
  SELECT c.oid FROM pg_catalog.pg_class c WHERE c.relnamespace = :ns::pg_catalog.oid AND c.relkind IN ('r', 'p', 'v', 'm', 'f')
    AND (:name_like::pg_catalog.text IS NULL OR c.relname LIKE :name_like::pg_catalog.text ESCAPE :like_escape::pg_catalog.text)
)
SELECT DISTINCT y.f AS facet, y.o AS oid FROM (
  SELECT 'rel' AS f, c.oid AS o FROM pg_catalog.pg_class c
  WHERE c.relnamespace = :ns::pg_catalog.oid AND c.relkind IN ('r', 'p', 'v', 'm', 'f')
    AND (:name_like::pg_catalog.text IS NULL OR c.relname LIKE :name_like::pg_catalog.text ESCAPE :like_escape::pg_catalog.text)
    AND pg_catalog.age(c.xmin) BETWEEN 0 AND pg_catalog.age(:w::pg_catalog.xid)
  UNION ALL SELECT 'rel', r.ev_class FROM pg_catalog.pg_rewrite r JOIN rels ON rels.oid = r.ev_class
  WHERE pg_catalog.age(r.xmin) BETWEEN 0 AND pg_catalog.age(:w::pg_catalog.xid)
  UNION ALL SELECT 'rel', i.inhrelid FROM pg_catalog.pg_inherits i JOIN rels ON rels.oid = i.inhrelid
  WHERE pg_catalog.age(i.xmin) BETWEEN 0 AND pg_catalog.age(:w::pg_catalog.xid)
  UNION ALL SELECT 'rel', p.partrelid FROM pg_catalog.pg_partitioned_table p JOIN rels ON rels.oid = p.partrelid
  WHERE pg_catalog.age(p.xmin) BETWEEN 0 AND pg_catalog.age(:w::pg_catalog.xid)
  UNION ALL SELECT 'columns', a.attrelid FROM pg_catalog.pg_attribute a JOIN rels ON rels.oid = a.attrelid
  WHERE pg_catalog.age(a.xmin) BETWEEN 0 AND pg_catalog.age(:w::pg_catalog.xid)
  UNION ALL SELECT 'columns', ad.adrelid FROM pg_catalog.pg_attrdef ad JOIN rels ON rels.oid = ad.adrelid
  WHERE pg_catalog.age(ad.xmin) BETWEEN 0 AND pg_catalog.age(:w::pg_catalog.xid)
  UNION ALL SELECT 'indexes', i.indrelid FROM pg_catalog.pg_index i JOIN rels ON rels.oid = i.indrelid
  WHERE pg_catalog.age(i.xmin) BETWEEN 0 AND pg_catalog.age(:w::pg_catalog.xid)
  UNION ALL SELECT 'indexes', i.indrelid FROM pg_catalog.pg_index i JOIN rels ON rels.oid = i.indrelid
  JOIN pg_catalog.pg_class ic ON ic.oid = i.indexrelid
  WHERE pg_catalog.age(ic.xmin) BETWEEN 0 AND pg_catalog.age(:w::pg_catalog.xid)
  UNION ALL SELECT 'constraints', k.conrelid FROM pg_catalog.pg_constraint k JOIN rels ON rels.oid = k.conrelid
  WHERE pg_catalog.age(k.xmin) BETWEEN 0 AND pg_catalog.age(:w::pg_catalog.xid)
  UNION ALL SELECT 'triggers', t.tgrelid FROM pg_catalog.pg_trigger t JOIN rels ON rels.oid = t.tgrelid
  WHERE NOT t.tgisinternal AND pg_catalog.age(t.xmin) BETWEEN 0 AND pg_catalog.age(:w::pg_catalog.xid)
  UNION ALL SELECT 'comments', d.objoid FROM pg_catalog.pg_description d JOIN rels ON rels.oid = d.objoid
  WHERE d.classoid = 'pg_catalog.pg_class'::pg_catalog.regclass
    AND pg_catalog.age(d.xmin) BETWEEN 0 AND pg_catalog.age(:w::pg_catalog.xid)
  UNION ALL SELECT 'indexes', i.indrelid FROM pg_catalog.pg_description d
  JOIN pg_catalog.pg_index i ON i.indexrelid = d.objoid JOIN rels ON rels.oid = i.indrelid
  WHERE d.classoid = 'pg_catalog.pg_class'::pg_catalog.regclass
    AND pg_catalog.age(d.xmin) BETWEEN 0 AND pg_catalog.age(:w::pg_catalog.xid)
  UNION ALL SELECT 'constraints', k.conrelid FROM pg_catalog.pg_description d
  JOIN pg_catalog.pg_constraint k ON k.oid = d.objoid JOIN rels ON rels.oid = k.conrelid
  WHERE d.classoid = 'pg_catalog.pg_constraint'::pg_catalog.regclass
    AND pg_catalog.age(d.xmin) BETWEEN 0 AND pg_catalog.age(:w::pg_catalog.xid)
  UNION ALL SELECT 'triggers', t.tgrelid FROM pg_catalog.pg_description d
  JOIN pg_catalog.pg_trigger t ON t.oid = d.objoid JOIN rels ON rels.oid = t.tgrelid
  WHERE d.classoid = 'pg_catalog.pg_trigger'::pg_catalog.regclass
    AND pg_catalog.age(d.xmin) BETWEEN 0 AND pg_catalog.age(:w::pg_catalog.xid)
  UNION ALL SELECT 'sequence', c.oid FROM pg_catalog.pg_class c
  WHERE c.relnamespace = :ns::pg_catalog.oid AND c.relkind = 'S'
    AND pg_catalog.age(c.xmin) BETWEEN 0 AND pg_catalog.age(:w::pg_catalog.xid)
  UNION ALL SELECT 'sequence', s.seqrelid FROM pg_catalog.pg_sequence s
  JOIN pg_catalog.pg_class c ON c.oid = s.seqrelid
  WHERE c.relnamespace = :ns::pg_catalog.oid AND pg_catalog.age(s.xmin) BETWEEN 0 AND pg_catalog.age(:w::pg_catalog.xid)
  UNION ALL SELECT 'sequence', c.oid FROM pg_catalog.pg_description d
  JOIN pg_catalog.pg_class c ON c.oid = d.objoid
  WHERE d.classoid = 'pg_catalog.pg_class'::pg_catalog.regclass AND c.relnamespace = :ns::pg_catalog.oid AND c.relkind = 'S'
    AND pg_catalog.age(d.xmin) BETWEEN 0 AND pg_catalog.age(:w::pg_catalog.xid)
  UNION ALL SELECT 'sequence', c.oid FROM pg_catalog.pg_depend dep
  JOIN pg_catalog.pg_class c ON c.oid = dep.objid
  WHERE dep.classid = 'pg_catalog.pg_class'::pg_catalog.regclass AND c.relnamespace = :ns::pg_catalog.oid AND c.relkind = 'S'
    AND pg_catalog.age(dep.xmin) BETWEEN 0 AND pg_catalog.age(:w::pg_catalog.xid)
  UNION ALL SELECT 'routine', p.oid FROM pg_catalog.pg_proc p
  WHERE p.pronamespace = :ns::pg_catalog.oid AND pg_catalog.age(p.xmin) BETWEEN 0 AND pg_catalog.age(:w::pg_catalog.xid)
  UNION ALL SELECT 'routine', p.oid FROM pg_catalog.pg_description d
  JOIN pg_catalog.pg_proc p ON p.oid = d.objoid
  WHERE d.classoid = 'pg_catalog.pg_proc'::pg_catalog.regclass AND p.pronamespace = :ns::pg_catalog.oid
    AND pg_catalog.age(d.xmin) BETWEEN 0 AND pg_catalog.age(:w::pg_catalog.xid)
  UNION ALL SELECT 'type', t.oid FROM pg_catalog.pg_type t
  WHERE t.typnamespace = :ns::pg_catalog.oid AND pg_catalog.age(t.xmin) BETWEEN 0 AND pg_catalog.age(:w::pg_catalog.xid)
  UNION ALL SELECT 'type', e.enumtypid FROM pg_catalog.pg_enum e
  JOIN pg_catalog.pg_type t ON t.oid = e.enumtypid
  WHERE t.typnamespace = :ns::pg_catalog.oid AND pg_catalog.age(e.xmin) BETWEEN 0 AND pg_catalog.age(:w::pg_catalog.xid)
  UNION ALL SELECT 'type', k.contypid FROM pg_catalog.pg_constraint k
  JOIN pg_catalog.pg_type t ON t.oid = k.contypid
  WHERE t.typnamespace = :ns::pg_catalog.oid AND pg_catalog.age(k.xmin) BETWEEN 0 AND pg_catalog.age(:w::pg_catalog.xid)
  UNION ALL SELECT 'type', r.rngtypid FROM pg_catalog.pg_range r
  JOIN pg_catalog.pg_type t ON t.oid = r.rngtypid
  WHERE t.typnamespace = :ns::pg_catalog.oid AND pg_catalog.age(r.xmin) BETWEEN 0 AND pg_catalog.age(:w::pg_catalog.xid)
  UNION ALL SELECT 'type', c.reltype FROM pg_catalog.pg_class c
  WHERE c.relnamespace = :ns::pg_catalog.oid AND c.relkind = 'c'
    AND pg_catalog.age(c.xmin) BETWEEN 0 AND pg_catalog.age(:w::pg_catalog.xid)
  UNION ALL SELECT 'type', c.reltype FROM pg_catalog.pg_attribute a
  JOIN pg_catalog.pg_class c ON c.oid = a.attrelid
  WHERE c.relnamespace = :ns::pg_catalog.oid AND c.relkind = 'c'
    AND pg_catalog.age(a.xmin) BETWEEN 0 AND pg_catalog.age(:w::pg_catalog.xid)
  UNION ALL SELECT 'type', t.oid FROM pg_catalog.pg_description d
  JOIN pg_catalog.pg_type t ON t.oid = d.objoid
  WHERE d.classoid = 'pg_catalog.pg_type'::pg_catalog.regclass AND t.typnamespace = :ns::pg_catalog.oid
    AND pg_catalog.age(d.xmin) BETWEEN 0 AND pg_catalog.age(:w::pg_catalog.xid)
  UNION ALL SELECT 'schema', n.oid FROM pg_catalog.pg_namespace n
  WHERE n.oid = :ns::pg_catalog.oid AND pg_catalog.age(n.xmin) BETWEEN 0 AND pg_catalog.age(:w::pg_catalog.xid)
  UNION ALL SELECT 'schema', d.objoid FROM pg_catalog.pg_description d
  WHERE d.classoid = 'pg_catalog.pg_namespace'::pg_catalog.regclass AND d.objoid = :ns::pg_catalog.oid
    AND pg_catalog.age(d.xmin) BETWEEN 0 AND pg_catalog.age(:w::pg_catalog.xid)
) AS y

-- The dependency half of the change query (finding 4, R-M4-48): deparsed text names other
-- objects, whose renames leave the own rows of the dependent untouched. Every young pg_class,
-- pg_attribute, pg_type or pg_proc row of the whole database is a source (and, for renames of
-- an enum value, a collation, an operator or an operator class, young pg_enum, pg_collation,
-- pg_operator and pg_opclass rows: review m-1), and the objects of the namespace that depend
-- on a source through one pg_depend hop (deptype not e, x, p) are re-read. A young
-- pg_namespace row of another schema whose relations, types or routines the objects of the
-- namespace (relations, routines, types, constraints, triggers, column defaults, view rules)
-- depend on is reported as schema-rename (a two-hop change, PG spike
-- section 7 Q3): the pass then runs full.
-- m4-t22-state (Task 22 fix round 1 I-1, a named exception in the T16 pack): the renamed
-- schema may also hold a referenced collation, operator or operator class (their deparsed
-- names in column, index, constraint and view text), so those count as well.
--- name: ChangedDependents ---
--- returns: facet text, oid u32 ---
WITH src AS MATERIALIZED (
  SELECT 'pg_catalog.pg_class'::pg_catalog.regclass AS cls, c.oid FROM pg_catalog.pg_class c
  WHERE pg_catalog.age(c.xmin) BETWEEN 0 AND pg_catalog.age(:w::pg_catalog.xid)
  UNION SELECT 'pg_catalog.pg_class'::pg_catalog.regclass, a.attrelid FROM pg_catalog.pg_attribute a
  WHERE pg_catalog.age(a.xmin) BETWEEN 0 AND pg_catalog.age(:w::pg_catalog.xid)
  UNION SELECT 'pg_catalog.pg_type'::pg_catalog.regclass, t.oid FROM pg_catalog.pg_type t
  WHERE pg_catalog.age(t.xmin) BETWEEN 0 AND pg_catalog.age(:w::pg_catalog.xid)
  UNION SELECT 'pg_catalog.pg_proc'::pg_catalog.regclass, p.oid FROM pg_catalog.pg_proc p
  WHERE pg_catalog.age(p.xmin) BETWEEN 0 AND pg_catalog.age(:w::pg_catalog.xid)
  UNION SELECT 'pg_catalog.pg_type'::pg_catalog.regclass, e.enumtypid FROM pg_catalog.pg_enum e
  WHERE pg_catalog.age(e.xmin) BETWEEN 0 AND pg_catalog.age(:w::pg_catalog.xid)
  UNION SELECT 'pg_catalog.pg_collation'::pg_catalog.regclass, co.oid FROM pg_catalog.pg_collation co
  WHERE pg_catalog.age(co.xmin) BETWEEN 0 AND pg_catalog.age(:w::pg_catalog.xid)
  UNION SELECT 'pg_catalog.pg_operator'::pg_catalog.regclass, o.oid FROM pg_catalog.pg_operator o
  WHERE pg_catalog.age(o.xmin) BETWEEN 0 AND pg_catalog.age(:w::pg_catalog.xid)
  UNION SELECT 'pg_catalog.pg_opclass'::pg_catalog.regclass, oc.oid FROM pg_catalog.pg_opclass oc
  WHERE pg_catalog.age(oc.xmin) BETWEEN 0 AND pg_catalog.age(:w::pg_catalog.xid)
), dep AS MATERIALIZED (
  SELECT DISTINCT d.classid, d.objid, d.objsubid
  FROM pg_catalog.pg_depend d JOIN src ON d.refclassid = src.cls AND d.refobjid = src.oid
  WHERE d.deptype NOT IN ('e', 'x', 'p')
)
SELECT DISTINCT z.f AS facet, z.o AS oid FROM (
  SELECT CASE WHEN c.relkind IN ('i', 'I') THEN 'indexes' WHEN c.relkind = 'S' THEN 'sequence'
      WHEN c.relkind = 'c' THEN 'type' WHEN dep.objsubid <> 0 THEN 'columns' ELSE 'rel' END AS f,
    CASE WHEN c.relkind IN ('i', 'I') THEN (SELECT i.indrelid FROM pg_catalog.pg_index i WHERE i.indexrelid = c.oid)
      WHEN c.relkind = 'c' THEN c.reltype ELSE c.oid END AS o
  FROM dep JOIN pg_catalog.pg_class c ON dep.classid = 'pg_catalog.pg_class'::pg_catalog.regclass AND c.oid = dep.objid
  WHERE c.relnamespace = :ns::pg_catalog.oid AND c.relkind IN ('r', 'p', 'v', 'm', 'f', 'c', 'i', 'I', 'S')
  UNION ALL SELECT 'rel', r.ev_class FROM dep
  JOIN pg_catalog.pg_rewrite r ON dep.classid = 'pg_catalog.pg_rewrite'::pg_catalog.regclass AND r.oid = dep.objid
  JOIN pg_catalog.pg_class c ON c.oid = r.ev_class WHERE c.relnamespace = :ns::pg_catalog.oid
  UNION ALL SELECT CASE WHEN k.conrelid <> 0 THEN 'constraints' ELSE 'type' END,
    CASE WHEN k.conrelid <> 0 THEN k.conrelid ELSE k.contypid END FROM dep
  JOIN pg_catalog.pg_constraint k ON dep.classid = 'pg_catalog.pg_constraint'::pg_catalog.regclass AND k.oid = dep.objid
  WHERE k.connamespace = :ns::pg_catalog.oid
  UNION ALL SELECT 'triggers', t.tgrelid FROM dep
  JOIN pg_catalog.pg_trigger t ON dep.classid = 'pg_catalog.pg_trigger'::pg_catalog.regclass AND t.oid = dep.objid
  JOIN pg_catalog.pg_class c ON c.oid = t.tgrelid WHERE c.relnamespace = :ns::pg_catalog.oid
  UNION ALL SELECT 'columns', ad.adrelid FROM dep
  JOIN pg_catalog.pg_attrdef ad ON dep.classid = 'pg_catalog.pg_attrdef'::pg_catalog.regclass AND ad.oid = dep.objid
  JOIN pg_catalog.pg_class c ON c.oid = ad.adrelid WHERE c.relnamespace = :ns::pg_catalog.oid
  UNION ALL SELECT 'type', t.oid FROM dep
  JOIN pg_catalog.pg_type t ON dep.classid = 'pg_catalog.pg_type'::pg_catalog.regclass AND t.oid = dep.objid
  WHERE t.typnamespace = :ns::pg_catalog.oid
  UNION ALL SELECT 'routine', p.oid FROM dep
  JOIN pg_catalog.pg_proc p ON dep.classid = 'pg_catalog.pg_proc'::pg_catalog.regclass AND p.oid = dep.objid
  WHERE p.pronamespace = :ns::pg_catalog.oid
  UNION ALL SELECT 'schema-rename', n.oid FROM pg_catalog.pg_namespace n
  WHERE n.oid <> :ns::pg_catalog.oid AND pg_catalog.age(n.xmin) BETWEEN 0 AND pg_catalog.age(:w::pg_catalog.xid)
    AND EXISTS (SELECT 1 FROM pg_catalog.pg_depend d
      LEFT JOIN pg_catalog.pg_type rt ON d.refclassid = 'pg_catalog.pg_type'::pg_catalog.regclass AND rt.oid = d.refobjid
      LEFT JOIN pg_catalog.pg_proc rp ON d.refclassid = 'pg_catalog.pg_proc'::pg_catalog.regclass AND rp.oid = d.refobjid
      LEFT JOIN pg_catalog.pg_class rc ON d.refclassid = 'pg_catalog.pg_class'::pg_catalog.regclass AND rc.oid = d.refobjid
      LEFT JOIN pg_catalog.pg_collation rco ON d.refclassid = 'pg_catalog.pg_collation'::pg_catalog.regclass AND rco.oid = d.refobjid
      LEFT JOIN pg_catalog.pg_operator rop ON d.refclassid = 'pg_catalog.pg_operator'::pg_catalog.regclass AND rop.oid = d.refobjid
      LEFT JOIN pg_catalog.pg_opclass roc ON d.refclassid = 'pg_catalog.pg_opclass'::pg_catalog.regclass AND roc.oid = d.refobjid
      LEFT JOIN pg_catalog.pg_class oc ON d.classid = 'pg_catalog.pg_class'::pg_catalog.regclass AND oc.oid = d.objid
      LEFT JOIN pg_catalog.pg_proc op ON d.classid = 'pg_catalog.pg_proc'::pg_catalog.regclass AND op.oid = d.objid
      LEFT JOIN pg_catalog.pg_type ot ON d.classid = 'pg_catalog.pg_type'::pg_catalog.regclass AND ot.oid = d.objid
      LEFT JOIN pg_catalog.pg_constraint ok ON d.classid = 'pg_catalog.pg_constraint'::pg_catalog.regclass AND ok.oid = d.objid
      LEFT JOIN pg_catalog.pg_trigger otg ON d.classid = 'pg_catalog.pg_trigger'::pg_catalog.regclass AND otg.oid = d.objid
      LEFT JOIN pg_catalog.pg_attrdef oad ON d.classid = 'pg_catalog.pg_attrdef'::pg_catalog.regclass AND oad.oid = d.objid
      LEFT JOIN pg_catalog.pg_rewrite orw ON d.classid = 'pg_catalog.pg_rewrite'::pg_catalog.regclass AND orw.oid = d.objid
      LEFT JOIN pg_catalog.pg_class oo ON oo.oid = COALESCE(otg.tgrelid, oad.adrelid, orw.ev_class)
      WHERE d.deptype NOT IN ('e', 'x', 'p')
        AND (rt.typnamespace = n.oid OR rp.pronamespace = n.oid OR rc.relnamespace = n.oid
          OR rco.collnamespace = n.oid OR rop.oprnamespace = n.oid OR roc.opcnamespace = n.oid)
        AND (oc.relnamespace = :ns::pg_catalog.oid OR op.pronamespace = :ns::pg_catalog.oid OR ot.typnamespace = :ns::pg_catalog.oid
          OR ok.connamespace = :ns::pg_catalog.oid OR oo.relnamespace = :ns::pg_catalog.oid))
) AS z
WHERE z.o IS NOT NULL

-- Relations (tables, partitioned tables, views, materialized views, foreign tables) with
-- their own properties. Composite types are read as types (RetrieveTypes).
--- name: RetrieveRelations ---
--- returns: oid u32, name text, relkind text, owner text, persistence text, comment text, part_parent u32, part_parent_schema text, part_parent_name text, part_bound text, part_key text, view_def text, populated bool, options text, row_security bool, reltuples text, access_method text, tablespace text ---
SELECT c.oid AS oid,
  c.relname::text AS name,
  c.relkind::text AS relkind,
  pg_catalog.pg_get_userbyid(c.relowner)::text AS owner,
  c.relpersistence::text AS persistence,
  d.description AS comment,
  pp.oid AS part_parent,
  ppn.nspname::text AS part_parent_schema,
  pp.relname::text AS part_parent_name,
--# if level >= structure
  CASE WHEN c.relispartition THEN pg_catalog.pg_get_expr(c.relpartbound, c.oid) END AS part_bound,
  CASE WHEN c.relkind = 'p' THEN pg_catalog.pg_get_partkeydef(c.oid) END AS part_key,
--# else
  NULL::text AS part_bound,
  NULL::text AS part_key,
--# end
--# if level >= sources
  CASE WHEN c.relkind IN ('v', 'm') THEN pg_catalog.pg_get_viewdef(c.oid, true) END AS view_def,
--# else
  NULL::text AS view_def,
--# end
  c.relispopulated AS populated,
  c.reloptions::text AS options,
  c.relrowsecurity AS row_security,
  c.reltuples::text AS reltuples,
  am.amname::text AS access_method,
  ts.spcname::text AS tablespace
FROM pg_catalog.pg_class c
LEFT JOIN pg_catalog.pg_description d ON d.classoid = 'pg_catalog.pg_class'::pg_catalog.regclass
  AND d.objoid = c.oid AND d.objsubid = 0
LEFT JOIN pg_catalog.pg_inherits ih ON c.relispartition AND ih.inhrelid = c.oid
LEFT JOIN pg_catalog.pg_class pp ON pp.oid = ih.inhparent
LEFT JOIN pg_catalog.pg_namespace ppn ON ppn.oid = pp.relnamespace
LEFT JOIN pg_catalog.pg_am am ON am.oid = c.relam
LEFT JOIN pg_catalog.pg_tablespace ts ON ts.oid = c.reltablespace
WHERE c.relnamespace = :ns::pg_catalog.oid AND c.relkind IN ('r', 'p', 'v', 'm', 'f')
  AND (:name_like::pg_catalog.text IS NULL OR c.relname LIKE :name_like::pg_catalog.text ESCAPE :like_escape::pg_catalog.text)
--# if mode == incremental
  AND c.oid IN :[oids]
--# end
ORDER BY c.oid

--- name: RetrieveColumns ---
--- returns: relid u32, attnum i16, name text, type_oid u32, type_text text, type_schema text, type_name text, typtype text, element u32, domain_base u32, ndims i32, not_null bool, default_expr text, identity_kind text, generated_kind text, coll_name text, inhcount i32, comment text ---
SELECT a.attrelid AS relid,
  a.attnum AS attnum,
  a.attname::text AS name,
  a.atttypid AS type_oid,
  pg_catalog.format_type(a.atttypid, a.atttypmod) AS type_text,
  tn.nspname::text AS type_schema,
  t.typname::text AS type_name,
  t.typtype::text AS typtype,
  CASE WHEN t.typlen = -1 AND t.typelem <> 0 THEN t.typelem END AS element,
  CASE WHEN t.typtype = 'd' THEN t.typbasetype END AS domain_base,
  a.attndims AS ndims,
  a.attnotnull AS not_null,
  pg_catalog.pg_get_expr(ad.adbin, ad.adrelid) AS default_expr,
  a.attidentity::text AS identity_kind,
  a.attgenerated::text AS generated_kind,
  CASE WHEN a.attcollation <> t.typcollation THEN co.collname::text END AS coll_name,
  a.attinhcount AS inhcount,
  d.description AS comment
FROM pg_catalog.pg_attribute a
JOIN pg_catalog.pg_class c ON c.oid = a.attrelid
JOIN pg_catalog.pg_type t ON t.oid = a.atttypid
JOIN pg_catalog.pg_namespace tn ON tn.oid = t.typnamespace
LEFT JOIN pg_catalog.pg_attrdef ad ON ad.adrelid = a.attrelid AND ad.adnum = a.attnum
LEFT JOIN pg_catalog.pg_collation co ON co.oid = a.attcollation
LEFT JOIN pg_catalog.pg_description d ON d.classoid = 'pg_catalog.pg_class'::pg_catalog.regclass
  AND d.objoid = a.attrelid AND d.objsubid = a.attnum
WHERE c.relnamespace = :ns::pg_catalog.oid AND c.relkind IN ('r', 'p', 'v', 'm', 'f') AND a.attnum > 0 AND NOT a.attisdropped
--# if mode == incremental
  AND c.oid IN :[oids]
--# end
ORDER BY a.attrelid, a.attnum

-- One row per index: keys as ordered arrays (HV-INTRO-13), expression keys deparsed only for
-- their slots, operator classes only when not the default, collations only when they differ
-- from the collation of the column.
--- name: RetrieveIndexes ---
--- returns: relid u32, oid u32, name text, method text, is_unique bool, is_primary bool, is_exclusion bool, immediate bool, valid bool, nulls_not_distinct bool, nkey i16, keys text, key_options text, key_names text, exprs text, opclasses text, collations text, predicate text, definition text, comment text ---
SELECT i.indrelid AS relid,
  i.indexrelid AS oid,
  ic.relname::text AS name,
  am.amname::text AS method,
  i.indisunique AS is_unique,
  i.indisprimary AS is_primary,
  i.indisexclusion AS is_exclusion,
  i.indimmediate AS immediate,
  i.indisvalid AS valid,
--# if version >= 150000
  i.indnullsnotdistinct AS nulls_not_distinct,
--# else
  false AS nulls_not_distinct,
--# end
  i.indnkeyatts AS nkey,
  i.indkey::text AS keys,
  i.indoption::text AS key_options,
  ARRAY(SELECT a.attname FROM pg_catalog.generate_series(0, i.indnatts - 1) AS k
    LEFT JOIN pg_catalog.pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = i.indkey[k]
    ORDER BY k)::text AS key_names,
  ARRAY(SELECT CASE WHEN i.indkey[k] = 0 THEN pg_catalog.pg_get_indexdef(i.indexrelid, k + 1, true) END
    FROM pg_catalog.generate_series(0, i.indnatts - 1) AS k ORDER BY k)::text AS exprs,
  ARRAY(SELECT CASE WHEN oc.opcdefault THEN NULL
      WHEN ocn.nspname = 'pg_catalog' THEN pg_catalog.quote_ident(oc.opcname)
      ELSE pg_catalog.quote_ident(ocn.nspname) || '.' || pg_catalog.quote_ident(oc.opcname) END
    FROM pg_catalog.generate_series(0, i.indnkeyatts - 1) AS k
    JOIN pg_catalog.pg_opclass oc ON oc.oid = i.indclass[k]
    JOIN pg_catalog.pg_namespace ocn ON ocn.oid = oc.opcnamespace
    ORDER BY k)::text AS opclasses,
  ARRAY(SELECT CASE WHEN i.indcollation[k] = 0 THEN NULL
      WHEN i.indkey[k] <> 0 AND i.indcollation[k] = ka.attcollation THEN NULL
      WHEN i.indkey[k] = 0 AND i.indcollation[k] = 100 THEN NULL
      WHEN cln.nspname = 'pg_catalog' THEN pg_catalog.quote_ident(cl.collname)
      ELSE pg_catalog.quote_ident(cln.nspname) || '.' || pg_catalog.quote_ident(cl.collname) END
    FROM pg_catalog.generate_series(0, i.indnkeyatts - 1) AS k
    LEFT JOIN pg_catalog.pg_attribute ka ON ka.attrelid = i.indrelid AND ka.attnum = i.indkey[k]
    LEFT JOIN pg_catalog.pg_collation cl ON cl.oid = i.indcollation[k]
    LEFT JOIN pg_catalog.pg_namespace cln ON cln.oid = cl.collnamespace
    ORDER BY k)::text AS collations,
  pg_catalog.pg_get_expr(i.indpred, i.indrelid) AS predicate,
--# if level >= sources
  pg_catalog.pg_get_indexdef(i.indexrelid) AS definition,
--# else
  NULL::text AS definition,
--# end
  d.description AS comment
FROM pg_catalog.pg_index i
JOIN pg_catalog.pg_class c ON c.oid = i.indrelid
JOIN pg_catalog.pg_class ic ON ic.oid = i.indexrelid
JOIN pg_catalog.pg_am am ON am.oid = ic.relam
LEFT JOIN pg_catalog.pg_description d ON d.classoid = 'pg_catalog.pg_class'::pg_catalog.regclass
  AND d.objoid = i.indexrelid AND d.objsubid = 0
WHERE c.relnamespace = :ns::pg_catalog.oid AND c.relkind IN ('r', 'p', 'v', 'm', 'f')
--# if mode == incremental
  AND c.oid IN :[oids]
--# end
ORDER BY i.indrelid, i.indexrelid

-- Table constraints, the catalogued NOT NULL constraints of PostgreSQL 18 (contype n) included.
--- name: RetrieveConstraints ---
--- returns: relid u32, oid u32, name text, contype text, is_deferrable bool, is_deferred bool, is_validated bool, is_enforced bool, is_local bool, no_inherit bool, parent_oid u32, conkey text, key_names text, index_oid u32, index_name text, ref_oid u32, ref_schema text, ref_name text, ref_key text, ref_names text, on_update text, on_delete text, match_type text, definition text, check_expr text, excl_ops text, comment text ---
SELECT k.conrelid AS relid,
  k.oid AS oid,
  k.conname::text AS name,
  k.contype::text AS contype,
  k.condeferrable AS is_deferrable,
  k.condeferred AS is_deferred,
  k.convalidated AS is_validated,
--# if version >= 180000
  k.conenforced AS is_enforced,
--# else
  true AS is_enforced,
--# end
  k.conislocal AS is_local,
  k.connoinherit AS no_inherit,
  NULLIF(k.conparentid, 0) AS parent_oid,
  k.conkey::text AS conkey,
  ARRAY(SELECT a.attname FROM pg_catalog.unnest(k.conkey) WITH ORDINALITY AS u(n, p)
    LEFT JOIN pg_catalog.pg_attribute a ON a.attrelid = k.conrelid AND a.attnum = u.n
    ORDER BY u.p)::text AS key_names,
  NULLIF(k.conindid, 0) AS index_oid,
  ix.relname::text AS index_name,
  NULLIF(k.confrelid, 0) AS ref_oid,
  fn.nspname::text AS ref_schema,
  fc.relname::text AS ref_name,
  k.confkey::text AS ref_key,
  ARRAY(SELECT a.attname FROM pg_catalog.unnest(k.confkey) WITH ORDINALITY AS u(n, p)
    LEFT JOIN pg_catalog.pg_attribute a ON a.attrelid = k.confrelid AND a.attnum = u.n
    ORDER BY u.p)::text AS ref_names,
  k.confupdtype::text AS on_update,
  k.confdeltype::text AS on_delete,
  k.confmatchtype::text AS match_type,
  pg_catalog.pg_get_constraintdef(k.oid, true) AS definition,
  CASE WHEN k.contype = 'c' THEN pg_catalog.pg_get_expr(k.conbin, k.conrelid, true) END AS check_expr,
  ARRAY(SELECT CASE WHEN opn.nspname = 'pg_catalog' THEN o.oprname::text
      ELSE pg_catalog.quote_ident(opn.nspname) || '.' || o.oprname::text END
    FROM pg_catalog.unnest(k.conexclop) WITH ORDINALITY AS u(op, p)
    JOIN pg_catalog.pg_operator o ON o.oid = u.op
    JOIN pg_catalog.pg_namespace opn ON opn.oid = o.oprnamespace
    ORDER BY u.p)::text AS excl_ops,
  d.description AS comment
FROM pg_catalog.pg_constraint k
JOIN pg_catalog.pg_class c ON c.oid = k.conrelid
LEFT JOIN pg_catalog.pg_class ix ON ix.oid = k.conindid
LEFT JOIN pg_catalog.pg_class fc ON fc.oid = k.confrelid
LEFT JOIN pg_catalog.pg_namespace fn ON fn.oid = fc.relnamespace
LEFT JOIN pg_catalog.pg_description d ON d.classoid = 'pg_catalog.pg_constraint'::pg_catalog.regclass
  AND d.objoid = k.oid AND d.objsubid = 0
WHERE c.relnamespace = :ns::pg_catalog.oid AND c.relkind IN ('r', 'p', 'v', 'm', 'f')
--# if mode == incremental
  AND c.oid IN :[oids]
--# end
ORDER BY k.conrelid, k.oid

-- User triggers (internal ones, such as foreign-key triggers, are left out). tgtype bits are
-- decoded on the client.
--- name: RetrieveTriggers ---
--- returns: relid u32, oid u32, name text, tgtype i16, enabled text, function_oid u32, function_schema text, function_name text, definition text, is_constraint bool, comment text ---
SELECT t.tgrelid AS relid,
  t.oid AS oid,
  t.tgname::text AS name,
  t.tgtype AS tgtype,
  t.tgenabled::text AS enabled,
  p.oid AS function_oid,
  pn.nspname::text AS function_schema,
  p.proname::text AS function_name,
--# if level >= sources
  pg_catalog.pg_get_triggerdef(t.oid, true) AS definition,
--# else
  NULL::text AS definition,
--# end
  t.tgconstraint <> 0 AS is_constraint,
  d.description AS comment
FROM pg_catalog.pg_trigger t
JOIN pg_catalog.pg_class c ON c.oid = t.tgrelid
JOIN pg_catalog.pg_proc p ON p.oid = t.tgfoid
JOIN pg_catalog.pg_namespace pn ON pn.oid = p.pronamespace
LEFT JOIN pg_catalog.pg_description d ON d.classoid = 'pg_catalog.pg_trigger'::pg_catalog.regclass
  AND d.objoid = t.oid AND d.objsubid = 0
WHERE c.relnamespace = :ns::pg_catalog.oid AND c.relkind IN ('r', 'p', 'v', 'm', 'f') AND NOT t.tgisinternal
--# if mode == incremental
  AND c.oid IN :[oids]
--# end
ORDER BY t.tgrelid, t.oid

-- Sequences. OWNED BY from pg_depend (a: serial or OWNED BY, i: identity).
--- name: RetrieveSequences ---
--- returns: oid u32, name text, owner text, data_type text, seq_start i64, seq_increment i64, seq_min i64, seq_max i64, seq_cache i64, seq_cycle bool, owner_table u32, owner_column i32, deptype text, owner_table_schema text, owner_table_name text, owner_column_name text, comment text ---
SELECT c.oid AS oid,
  c.relname::text AS name,
  pg_catalog.pg_get_userbyid(c.relowner)::text AS owner,
  pg_catalog.format_type(s.seqtypid, NULL) AS data_type,
  s.seqstart AS seq_start,
  s.seqincrement AS seq_increment,
  s.seqmin AS seq_min,
  s.seqmax AS seq_max,
  s.seqcache AS seq_cache,
  s.seqcycle AS seq_cycle,
  dep.refobjid AS owner_table,
  dep.refobjsubid AS owner_column,
  dep.deptype::text AS deptype,
  otn.nspname::text AS owner_table_schema,
  ot.relname::text AS owner_table_name,
  oa.attname::text AS owner_column_name,
  d.description AS comment
FROM pg_catalog.pg_class c
JOIN pg_catalog.pg_sequence s ON s.seqrelid = c.oid
LEFT JOIN pg_catalog.pg_depend dep ON dep.classid = 'pg_catalog.pg_class'::pg_catalog.regclass
  AND dep.objid = c.oid AND dep.refclassid = 'pg_catalog.pg_class'::pg_catalog.regclass
  AND dep.deptype IN ('a', 'i')
LEFT JOIN pg_catalog.pg_class ot ON ot.oid = dep.refobjid
LEFT JOIN pg_catalog.pg_namespace otn ON otn.oid = ot.relnamespace
LEFT JOIN pg_catalog.pg_attribute oa ON oa.attrelid = dep.refobjid AND oa.attnum = dep.refobjsubid
LEFT JOIN pg_catalog.pg_description d ON d.classoid = 'pg_catalog.pg_class'::pg_catalog.regclass
  AND d.objoid = c.oid AND d.objsubid = 0
WHERE c.relnamespace = :ns::pg_catalog.oid AND c.relkind = 'S'
--# if mode == incremental
  AND c.oid IN :[oids]
--# end
ORDER BY c.oid

-- Routines. The result is not read for procedures, and pg_get_functiondef runs only for plain
-- functions and procedures (it errors on aggregates).
--- name: RetrieveRoutines ---
--- returns: oid u32, name text, prokind text, identity_args text, result_type text, lang text, volatility text, is_strict bool, security_definer bool, returns_set bool, parallel_safety text, owner text, from_extension bool, arg_names text, arg_modes text, arg_types text, nargs i16, nargdefaults i16, definition text, comment text ---
SELECT p.oid AS oid,
  p.proname::text AS name,
  p.prokind::text AS prokind,
  pg_catalog.pg_get_function_identity_arguments(p.oid) AS identity_args,
  CASE WHEN p.prokind <> 'p' THEN pg_catalog.pg_get_function_result(p.oid) END AS result_type,
  l.lanname::text AS lang,
  p.provolatile::text AS volatility,
  p.proisstrict AS is_strict,
  p.prosecdef AS security_definer,
  p.proretset AS returns_set,
  p.proparallel::text AS parallel_safety,
  pg_catalog.pg_get_userbyid(p.proowner)::text AS owner,
  EXISTS (SELECT 1 FROM pg_catalog.pg_depend dep
    WHERE dep.classid = 'pg_catalog.pg_proc'::pg_catalog.regclass AND dep.objid = p.oid AND dep.deptype = 'e') AS from_extension,
  p.proargnames::text AS arg_names,
  p.proargmodes::text AS arg_modes,
  ARRAY(SELECT pg_catalog.format_type(u.t, NULL)
    FROM pg_catalog.unnest(COALESCE(p.proallargtypes, p.proargtypes::pg_catalog.oid[])) WITH ORDINALITY AS u(t, n)
    ORDER BY u.n)::text AS arg_types,
  p.pronargs AS nargs,
  p.pronargdefaults AS nargdefaults,
--# if level >= sources
  CASE WHEN p.prokind IN ('f', 'p') THEN pg_catalog.pg_get_functiondef(p.oid) END AS definition,
--# else
  NULL::text AS definition,
--# end
  d.description AS comment
FROM pg_catalog.pg_proc p
JOIN pg_catalog.pg_language l ON l.oid = p.prolang
LEFT JOIN pg_catalog.pg_description d ON d.classoid = 'pg_catalog.pg_proc'::pg_catalog.regclass
  AND d.objoid = p.oid AND d.objsubid = 0
WHERE p.pronamespace = :ns::pg_catalog.oid
--# if mode == incremental
  AND p.oid IN :[oids]
--# end
ORDER BY p.oid

-- User types: not array types (an anti-join), not the row types of tables (composites only
-- for relkind c), no shell or pseudo types. Enum labels in enumsortorder.
--- name: RetrieveTypes ---
--- returns: oid u32, name text, typtype text, owner text, comment text, domain_base text, not_null bool, default_expr text, relid u32, labels text, range_subtype text ---
SELECT t.oid AS oid,
  t.typname::text AS name,
  t.typtype::text AS typtype,
  pg_catalog.pg_get_userbyid(t.typowner)::text AS owner,
  d.description AS comment,
  CASE WHEN t.typtype = 'd' THEN pg_catalog.format_type(t.typbasetype, t.typtypmod) END AS domain_base,
  t.typnotnull AS not_null,
  t.typdefault AS default_expr,
  NULLIF(t.typrelid, 0) AS relid,
  CASE WHEN t.typtype = 'e' THEN ARRAY(SELECT e.enumlabel FROM pg_catalog.pg_enum e
    WHERE e.enumtypid = t.oid ORDER BY e.enumsortorder)::text END AS labels,
  CASE WHEN t.typtype = 'r' THEN pg_catalog.format_type(r.rngsubtype, NULL) END AS range_subtype
FROM pg_catalog.pg_type t
LEFT JOIN pg_catalog.pg_range r ON r.rngtypid = t.oid
LEFT JOIN pg_catalog.pg_description d ON d.classoid = 'pg_catalog.pg_type'::pg_catalog.regclass
  AND d.objoid = t.oid AND d.objsubid = 0
WHERE t.typnamespace = :ns::pg_catalog.oid AND t.typtype IN ('b', 'c', 'd', 'e', 'r')
  AND NOT EXISTS (SELECT 1 FROM pg_catalog.pg_type e WHERE e.typarray = t.oid)
  AND (t.typtype <> 'c' OR EXISTS (SELECT 1 FROM pg_catalog.pg_class r WHERE r.oid = t.typrelid AND r.relkind = 'c'))
--# if mode == incremental
  AND t.oid IN :[oids]
--# end
ORDER BY t.oid

-- The attributes of composite types (CREATE TYPE ... AS (...)).
--- name: RetrieveTypeAttributes ---
--- returns: type_oid u32, attnum i16, name text, type_text text ---
SELECT t.oid AS type_oid,
  a.attnum AS attnum,
  a.attname::text AS name,
  pg_catalog.format_type(a.atttypid, a.atttypmod) AS type_text
FROM pg_catalog.pg_type t
JOIN pg_catalog.pg_attribute a ON a.attrelid = t.typrelid
WHERE t.typnamespace = :ns::pg_catalog.oid AND t.typtype = 'c' AND a.attnum > 0 AND NOT a.attisdropped
--# if mode == incremental
  AND t.oid IN :[oids]
--# end
ORDER BY t.oid, a.attnum

-- Domain constraints. A domain NOT NULL is a contype n row from PostgreSQL 17 (R-M4-47).
--- name: RetrieveDomainConstraints ---
--- returns: type_oid u32, oid u32, name text, contype text, definition text, is_validated bool ---
SELECT k.contypid AS type_oid,
  k.oid AS oid,
  k.conname::text AS name,
  k.contype::text AS contype,
  pg_catalog.pg_get_constraintdef(k.oid, true) AS definition,
  k.convalidated AS is_validated
FROM pg_catalog.pg_constraint k
JOIN pg_catalog.pg_type t ON t.oid = k.contypid
WHERE t.typnamespace = :ns::pg_catalog.oid AND t.typtype = 'd'
--# if mode == incremental
  AND t.oid IN :[oids]
--# end
ORDER BY k.contypid, k.oid

-- The comments of the given relations and their columns (the incremental comments facet).
--- name: Comments ---
--- returns: objoid u32, objsubid i32, description text ---
SELECT d.objoid AS objoid,
  d.objsubid AS objsubid,
  d.description AS description
FROM pg_catalog.pg_description d
WHERE d.classoid = 'pg_catalog.pg_class'::pg_catalog.regclass
  AND d.objoid IN :[oids]
ORDER BY d.objoid, d.objsubid

-- The no-write canary (ver/01 #pg-canary), run just before COMMIT: true while this
-- transaction has no XID. The own pg_stat_activity row of the backend (backend_xid, read from
-- the state of the process when the row is read) carries it: the token guard of Task 5
-- refuses every pg_current_xact_id* and txid_current* name, the _if_assigned forms included.
--- name: Canary ---
--- returns: no_xid bool ---
SELECT a.backend_xid IS NULL AS no_xid
FROM pg_catalog.pg_stat_activity a
WHERE a.pid = pg_catalog.pg_backend_pid()
