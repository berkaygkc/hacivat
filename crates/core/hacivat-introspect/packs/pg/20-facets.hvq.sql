-- Hacivat query pack: PostgreSQL, file 20-facets (HV-INTRO-04, R-M4-32).
-- Contents: grants, dependencies, statistics and server-level sections (R-M4-30). Sections are written by Task 16b from the vendor documentation only
-- (no third-party catalog SQL); the grammar is Task 4's (`--- name: <Section> ---`).
-- Sources: the system catalogs (chapter 51: pg_class, pg_attribute, pg_proc, pg_type,
-- pg_namespace, pg_depend, pg_rewrite, pg_constraint, pg_attrdef, pg_trigger, pg_index,
-- pg_policy, pg_statistic_ext, pg_roles), the aclitem functions aclexplode and acldefault,
-- and pg_identify_object (system information functions).
--
-- Rules every section follows (R-M4-3, R-M4-7, R-M4-46):
-- - every catalog and catalog function is written pg_catalog.<name>
-- - objects are selected by oid only, inside the namespace :ns::pg_catalog.oid, through the
--   list bind; no name reaches the server
-- - an ACL is never parsed as text: aclexplode returns its items as rows (grantee oid,
--   privilege keyword, grant option), and the grantee is named by a join on its oid
-- - a NULL ACL means the default privileges, which acldefault spells out. Every Grants
--   section returns the ACL (src acl, the defaults where it is NULL), the defaults (src dflt)
--   and the column ACLs (src col) as rows. The client keeps what the ACL adds to the
--   defaults, and an object whose ACL revokes a default is not complete (Task 16b)
-- - one obj row per object proves it exists and names it, so a missing object is never read
--   as an object without grants
-- - none of these sections is joinable: M7 re-grants from a loaded facet and checks drift
--   by state stamps, so no facet read runs inside a joined transaction in M4.

--- name: GrantsRelation ---
--- returns: oid u32, src text, own text, kind text, col text, colnum i32, grantee u32, grantee_name text, privilege text, grantable bool ---
SELECT s.o AS oid, s.src AS src, s.own AS own, s.kind AS kind, s.col AS col, s.colnum AS colnum,
  s.grantee AS grantee, r.rolname::pg_catalog.text AS grantee_name, s.privilege AS privilege, s.grantable AS grantable
FROM (
  SELECT c.oid AS o, c.relnamespace AS ns, 'obj' AS src, c.relname::pg_catalog.text AS own, c.relkind::pg_catalog.text AS kind,
    NULL::pg_catalog.text AS col, NULL::pg_catalog.int4 AS colnum, NULL::pg_catalog.oid AS grantee, NULL::pg_catalog.text AS privilege,
    NULL::pg_catalog.bool AS grantable
  FROM pg_catalog.pg_class c
  WHERE c.relkind IN ('r', 'p', 'v', 'm', 'f', 'S')
  UNION ALL
  SELECT c.oid, c.relnamespace, 'acl', c.relname::pg_catalog.text, c.relkind::pg_catalog.text, NULL::pg_catalog.text, NULL::pg_catalog.int4,
    e.grantee, e.privilege_type, e.is_grantable
  FROM pg_catalog.pg_class c
  CROSS JOIN LATERAL pg_catalog.aclexplode(COALESCE(c.relacl,
    CASE WHEN c.relkind = 'S' THEN pg_catalog.acldefault('s', c.relowner)
      ELSE pg_catalog.acldefault('r', c.relowner) END)) e
  WHERE c.relkind IN ('r', 'p', 'v', 'm', 'f', 'S')
  UNION ALL
  SELECT c.oid, c.relnamespace, 'dflt', c.relname::pg_catalog.text, c.relkind::pg_catalog.text, NULL::pg_catalog.text, NULL::pg_catalog.int4,
    e.grantee, e.privilege_type, e.is_grantable
  FROM pg_catalog.pg_class c
  CROSS JOIN LATERAL pg_catalog.aclexplode(
    CASE WHEN c.relkind = 'S' THEN pg_catalog.acldefault('s', c.relowner)
      ELSE pg_catalog.acldefault('r', c.relowner) END) e
  WHERE c.relkind IN ('r', 'p', 'v', 'm', 'f', 'S')
  UNION ALL
  SELECT c.oid, c.relnamespace, 'col', c.relname::pg_catalog.text, c.relkind::pg_catalog.text, a.attname::pg_catalog.text, a.attnum::pg_catalog.int4,
    e.grantee, e.privilege_type, e.is_grantable
  FROM pg_catalog.pg_class c
  JOIN pg_catalog.pg_attribute a ON a.attrelid = c.oid AND a.attnum > 0 AND NOT a.attisdropped
    AND a.attacl IS NOT NULL
  CROSS JOIN LATERAL pg_catalog.aclexplode(a.attacl) e
  WHERE c.relkind IN ('r', 'p', 'v', 'm', 'f')
) s
LEFT JOIN pg_catalog.pg_roles r ON r.oid = s.grantee
WHERE s.ns = :ns::pg_catalog.oid AND s.o IN :[oids]

--- name: GrantsRoutine ---
--- returns: oid u32, src text, own text, kind text, col text, colnum i32, grantee u32, grantee_name text, privilege text, grantable bool ---
SELECT s.o AS oid, s.src AS src, s.own AS own, s.kind AS kind, s.col AS col, s.colnum AS colnum,
  s.grantee AS grantee, r.rolname::pg_catalog.text AS grantee_name, s.privilege AS privilege, s.grantable AS grantable
FROM (
  SELECT p.oid AS o, p.pronamespace AS ns, 'obj' AS src, p.proname::pg_catalog.text AS own, p.prokind::pg_catalog.text AS kind,
    NULL::pg_catalog.text AS col, NULL::pg_catalog.int4 AS colnum, NULL::pg_catalog.oid AS grantee, NULL::pg_catalog.text AS privilege,
    NULL::pg_catalog.bool AS grantable
  FROM pg_catalog.pg_proc p
  UNION ALL
  SELECT p.oid, p.pronamespace, 'acl', p.proname::pg_catalog.text, p.prokind::pg_catalog.text, NULL::pg_catalog.text, NULL::pg_catalog.int4,
    e.grantee, e.privilege_type, e.is_grantable
  FROM pg_catalog.pg_proc p
  CROSS JOIN LATERAL pg_catalog.aclexplode(COALESCE(p.proacl, pg_catalog.acldefault('f', p.proowner))) e
  UNION ALL
  SELECT p.oid, p.pronamespace, 'dflt', p.proname::pg_catalog.text, p.prokind::pg_catalog.text, NULL::pg_catalog.text, NULL::pg_catalog.int4,
    e.grantee, e.privilege_type, e.is_grantable
  FROM pg_catalog.pg_proc p
  CROSS JOIN LATERAL pg_catalog.aclexplode(pg_catalog.acldefault('f', p.proowner)) e
) s
LEFT JOIN pg_catalog.pg_roles r ON r.oid = s.grantee
WHERE s.ns = :ns::pg_catalog.oid AND s.o IN :[oids]

--- name: GrantsType ---
--- returns: oid u32, src text, own text, kind text, col text, colnum i32, grantee u32, grantee_name text, privilege text, grantable bool ---
SELECT s.o AS oid, s.src AS src, s.own AS own, s.kind AS kind, s.col AS col, s.colnum AS colnum,
  s.grantee AS grantee, r.rolname::pg_catalog.text AS grantee_name, s.privilege AS privilege, s.grantable AS grantable
FROM (
  SELECT t.oid AS o, t.typnamespace AS ns, 'obj' AS src, t.typname::pg_catalog.text AS own, t.typtype::pg_catalog.text AS kind,
    NULL::pg_catalog.text AS col, NULL::pg_catalog.int4 AS colnum, NULL::pg_catalog.oid AS grantee, NULL::pg_catalog.text AS privilege,
    NULL::pg_catalog.bool AS grantable
  FROM pg_catalog.pg_type t
  UNION ALL
  SELECT t.oid, t.typnamespace, 'acl', t.typname::pg_catalog.text, t.typtype::pg_catalog.text, NULL::pg_catalog.text, NULL::pg_catalog.int4,
    e.grantee, e.privilege_type, e.is_grantable
  FROM pg_catalog.pg_type t
  CROSS JOIN LATERAL pg_catalog.aclexplode(COALESCE(t.typacl, pg_catalog.acldefault('T', t.typowner))) e
  UNION ALL
  SELECT t.oid, t.typnamespace, 'dflt', t.typname::pg_catalog.text, t.typtype::pg_catalog.text, NULL::pg_catalog.text, NULL::pg_catalog.int4,
    e.grantee, e.privilege_type, e.is_grantable
  FROM pg_catalog.pg_type t
  CROSS JOIN LATERAL pg_catalog.aclexplode(pg_catalog.acldefault('T', t.typowner)) e
) s
LEFT JOIN pg_catalog.pg_roles r ON r.oid = s.grantee
WHERE s.ns = :ns::pg_catalog.oid AND s.o IN :[oids]

-- The schema of the namespace itself (no list: the namespace oid selects it).
--- name: GrantsSchema ---
--- returns: oid u32, src text, own text, kind text, col text, colnum i32, grantee u32, grantee_name text, privilege text, grantable bool ---
SELECT s.o AS oid, s.src AS src, s.own AS own, s.kind AS kind, s.col AS col, s.colnum AS colnum,
  s.grantee AS grantee, r.rolname::pg_catalog.text AS grantee_name, s.privilege AS privilege, s.grantable AS grantable
FROM (
  SELECT n.oid AS o, 'obj' AS src, n.nspname::pg_catalog.text AS own, 'n' AS kind,
    NULL::pg_catalog.text AS col, NULL::pg_catalog.int4 AS colnum, NULL::pg_catalog.oid AS grantee, NULL::pg_catalog.text AS privilege,
    NULL::pg_catalog.bool AS grantable
  FROM pg_catalog.pg_namespace n
  UNION ALL
  SELECT n.oid, 'acl', n.nspname::pg_catalog.text, 'n', NULL::pg_catalog.text, NULL::pg_catalog.int4,
    e.grantee, e.privilege_type, e.is_grantable
  FROM pg_catalog.pg_namespace n
  CROSS JOIN LATERAL pg_catalog.aclexplode(COALESCE(n.nspacl, pg_catalog.acldefault('n', n.nspowner))) e
  UNION ALL
  SELECT n.oid, 'dflt', n.nspname::pg_catalog.text, 'n', NULL::pg_catalog.text, NULL::pg_catalog.int4,
    e.grantee, e.privilege_type, e.is_grantable
  FROM pg_catalog.pg_namespace n
  CROSS JOIN LATERAL pg_catalog.aclexplode(pg_catalog.acldefault('n', n.nspowner)) e
) s
LEFT JOIN pg_catalog.pg_roles r ON r.oid = s.grantee
WHERE s.o = :ns::pg_catalog.oid

-- What each object depends on: the pg_depend rows (deptype not e, x or p) whose depender is
-- the object itself or one of its parts. A relation owns its view rules, constraints, column
-- defaults, user triggers, indexes, row-level security policies and extended statistics
-- (review m-3: a function in a policy or a statistics expression). A type owns its domain
-- constraints and, for a composite type, its relation row (the attribute types). Each referenced
-- object is named by a join on its own catalog (an array type by its element, a table row
-- type by its table), or by pg_identify_object for the other catalogs (collations,
-- operators, languages and so on). The client drops what refers back to the object or its parts (refowner or selfrel),
-- and keeps one reference per object. Every depender part has at least one row (refcat none
-- when it depends on nothing), so an object that exists always answers.
--- name: Dependencies ---
--- returns: oid u32, cls text, own text, kind text, selfrel u32, refcat text, refid u32, refsub i32, refkind text, refschema text, refname text, refparent text, refowner u32 ---
SELECT s.o AS oid, s.cls AS cls, s.own AS own, s.kind AS kind, s.selfrel AS selfrel,
  CASE
    WHEN d.objid IS NULL THEN 'none'
    WHEN rtr.relkind IS NOT NULL AND rtr.relkind <> 'c' THEN 'class'
    WHEN rc.oid IS NOT NULL AND d.refobjsubid <> 0 THEN 'column'
    WHEN rc.oid IS NOT NULL THEN 'class'
    WHEN rp.oid IS NOT NULL THEN 'proc'
    WHEN rt.oid IS NOT NULL THEN 'type'
    WHEN rn.oid IS NOT NULL THEN 'namespace'
    WHEN rk.oid IS NOT NULL THEN 'constraint'
    ELSE 'other'
  END AS refcat,
  CASE
    WHEN rtr.relkind IS NOT NULL AND rtr.relkind <> 'c' THEN rtr.oid
    WHEN rc.relkind = 'c' THEN rc.reltype
    WHEN rte.oid IS NOT NULL THEN rte.oid
    ELSE d.refobjid
  END AS refid,
  d.refobjsubid AS refsub,
  CASE
    WHEN rtr.relkind IS NOT NULL AND rtr.relkind <> 'c' THEN rtr.relkind::pg_catalog.text
    ELSE COALESCE(rc.relkind::pg_catalog.text, rp.prokind::pg_catalog.text, rte.typtype::pg_catalog.text, rt.typtype::pg_catalog.text,
      rk.contype::pg_catalog.text, io.type)
  END AS refkind,
  COALESCE(refn.nspname::pg_catalog.text, io.schema) AS refschema,
  CASE
    WHEN rtr.relkind IS NOT NULL AND rtr.relkind <> 'c' THEN rtr.relname::pg_catalog.text
    ELSE COALESCE(ra.attname::pg_catalog.text, rc.relname::pg_catalog.text, rp.proname::pg_catalog.text, rte.typname::pg_catalog.text,
      rt.typname::pg_catalog.text, rn.nspname::pg_catalog.text, rk.conname::pg_catalog.text, io.name, io.identity)
  END AS refname,
  COALESCE(CASE WHEN ra.attname IS NOT NULL THEN rc.relname::pg_catalog.text END, ric.relname::pg_catalog.text,
    rkc.relname::pg_catalog.text, rkt.typname::pg_catalog.text) AS refparent,
  CASE
    WHEN rtr.relkind IS NOT NULL AND rtr.relkind <> 'c' THEN rtr.oid
    WHEN rc.oid IS NOT NULL THEN COALESCE(ri.indrelid, rc.oid)
    WHEN rk.oid IS NOT NULL AND rk.conrelid <> 0 THEN rk.conrelid
    WHEN rk.oid IS NOT NULL THEN rk.contypid
    WHEN rt.oid IS NOT NULL THEN COALESCE(rte.oid, rt.oid)
    ELSE 0::pg_catalog.oid
  END AS refowner
FROM (
  SELECT 'rel' AS cls, c.oid AS o, c.relnamespace AS ns, c.relname::pg_catalog.text AS own, c.relkind::pg_catalog.text AS kind,
    c.oid AS selfrel, 'pg_catalog.pg_class'::pg_catalog.regclass::pg_catalog.oid AS classid, c.oid AS objid
  FROM pg_catalog.pg_class c
  WHERE c.relkind IN ('r', 'p', 'v', 'm', 'f', 'S')
  UNION ALL
  SELECT 'rel', c.oid, c.relnamespace, c.relname::pg_catalog.text, c.relkind::pg_catalog.text, c.oid,
    'pg_catalog.pg_rewrite'::pg_catalog.regclass::pg_catalog.oid, w.oid
  FROM pg_catalog.pg_rewrite w JOIN pg_catalog.pg_class c ON c.oid = w.ev_class
  UNION ALL
  SELECT 'rel', c.oid, c.relnamespace, c.relname::pg_catalog.text, c.relkind::pg_catalog.text, c.oid,
    'pg_catalog.pg_constraint'::pg_catalog.regclass::pg_catalog.oid, k.oid
  FROM pg_catalog.pg_constraint k JOIN pg_catalog.pg_class c ON c.oid = k.conrelid
  UNION ALL
  SELECT 'rel', c.oid, c.relnamespace, c.relname::pg_catalog.text, c.relkind::pg_catalog.text, c.oid,
    'pg_catalog.pg_attrdef'::pg_catalog.regclass::pg_catalog.oid, ad.oid
  FROM pg_catalog.pg_attrdef ad JOIN pg_catalog.pg_class c ON c.oid = ad.adrelid
  UNION ALL
  SELECT 'rel', c.oid, c.relnamespace, c.relname::pg_catalog.text, c.relkind::pg_catalog.text, c.oid,
    'pg_catalog.pg_trigger'::pg_catalog.regclass::pg_catalog.oid, tg.oid
  FROM pg_catalog.pg_trigger tg JOIN pg_catalog.pg_class c ON c.oid = tg.tgrelid
  WHERE NOT tg.tgisinternal
  UNION ALL
  SELECT 'rel', c.oid, c.relnamespace, c.relname::pg_catalog.text, c.relkind::pg_catalog.text, c.oid,
    'pg_catalog.pg_class'::pg_catalog.regclass::pg_catalog.oid, i.indexrelid
  FROM pg_catalog.pg_index i JOIN pg_catalog.pg_class c ON c.oid = i.indrelid
  UNION ALL
  SELECT 'rel', c.oid, c.relnamespace, c.relname::pg_catalog.text, c.relkind::pg_catalog.text, c.oid,
    'pg_catalog.pg_policy'::pg_catalog.regclass::pg_catalog.oid, pol.oid
  FROM pg_catalog.pg_policy pol JOIN pg_catalog.pg_class c ON c.oid = pol.polrelid
  UNION ALL
  SELECT 'rel', c.oid, c.relnamespace, c.relname::pg_catalog.text, c.relkind::pg_catalog.text, c.oid,
    'pg_catalog.pg_statistic_ext'::pg_catalog.regclass::pg_catalog.oid, sx.oid
  FROM pg_catalog.pg_statistic_ext sx JOIN pg_catalog.pg_class c ON c.oid = sx.stxrelid
  UNION ALL
  SELECT 'fn', p.oid, p.pronamespace, p.proname::pg_catalog.text, p.prokind::pg_catalog.text, 0::pg_catalog.oid,
    'pg_catalog.pg_proc'::pg_catalog.regclass::pg_catalog.oid, p.oid
  FROM pg_catalog.pg_proc p
  UNION ALL
  SELECT 'typ', t.oid, t.typnamespace, t.typname::pg_catalog.text, t.typtype::pg_catalog.text, t.typrelid,
    'pg_catalog.pg_type'::pg_catalog.regclass::pg_catalog.oid, t.oid
  FROM pg_catalog.pg_type t
  UNION ALL
  SELECT 'typ', t.oid, t.typnamespace, t.typname::pg_catalog.text, t.typtype::pg_catalog.text, t.typrelid,
    'pg_catalog.pg_constraint'::pg_catalog.regclass::pg_catalog.oid, k.oid
  FROM pg_catalog.pg_constraint k JOIN pg_catalog.pg_type t ON t.oid = k.contypid
  UNION ALL
  SELECT 'typ', t.oid, t.typnamespace, t.typname::pg_catalog.text, t.typtype::pg_catalog.text, t.typrelid,
    'pg_catalog.pg_class'::pg_catalog.regclass::pg_catalog.oid, t.typrelid
  FROM pg_catalog.pg_type t
  WHERE t.typtype = 'c' AND t.typrelid <> 0
) s
LEFT JOIN pg_catalog.pg_depend d ON d.classid = s.classid AND d.objid = s.objid
  AND d.deptype <> 'e' AND d.deptype <> 'x' AND d.deptype <> 'p'
LEFT JOIN pg_catalog.pg_class rc ON d.refclassid = 'pg_catalog.pg_class'::pg_catalog.regclass
  AND rc.oid = d.refobjid
LEFT JOIN pg_catalog.pg_attribute ra ON rc.oid IS NOT NULL AND d.refobjsubid > 0
  AND ra.attrelid = rc.oid AND ra.attnum = d.refobjsubid
LEFT JOIN pg_catalog.pg_index ri ON ri.indexrelid = rc.oid
LEFT JOIN pg_catalog.pg_class ric ON ric.oid = ri.indrelid
LEFT JOIN pg_catalog.pg_proc rp ON d.refclassid = 'pg_catalog.pg_proc'::pg_catalog.regclass
  AND rp.oid = d.refobjid
LEFT JOIN pg_catalog.pg_type rt ON d.refclassid = 'pg_catalog.pg_type'::pg_catalog.regclass
  AND rt.oid = d.refobjid
LEFT JOIN pg_catalog.pg_type rte ON rt.typlen = -1 AND rt.typelem <> 0 AND rte.oid = rt.typelem
LEFT JOIN pg_catalog.pg_class rtr ON rtr.oid = COALESCE(rte.typrelid, rt.typrelid)
LEFT JOIN pg_catalog.pg_namespace rn ON d.refclassid = 'pg_catalog.pg_namespace'::pg_catalog.regclass
  AND rn.oid = d.refobjid
LEFT JOIN pg_catalog.pg_constraint rk ON d.refclassid = 'pg_catalog.pg_constraint'::pg_catalog.regclass
  AND rk.oid = d.refobjid
LEFT JOIN pg_catalog.pg_class rkc ON rkc.oid = rk.conrelid
LEFT JOIN pg_catalog.pg_type rkt ON rkt.oid = rk.contypid
LEFT JOIN pg_catalog.pg_namespace refn ON refn.oid = COALESCE(rtr.relnamespace, rc.relnamespace,
  rp.pronamespace, rte.typnamespace, rt.typnamespace, rk.connamespace, rn.oid)
LEFT JOIN LATERAL pg_catalog.pg_identify_object(d.refclassid, d.refobjid, d.refobjsubid) io ON true
WHERE s.ns = :ns::pg_catalog.oid AND s.o IN :[oids]

-- Row estimates on demand (Task 16 review m-5: VACUUM and ANALYZE update reltuples in place,
-- so its row keeps its xmin and an incremental pass never re-reads it). reltuples is -1 when
-- the relation was never vacuumed or analyzed (14 and later).
--- name: StatisticsRelation ---
--- returns: oid u32, own text, kind text, reltuples text ---
SELECT c.oid AS oid, c.relname::pg_catalog.text AS own, c.relkind::pg_catalog.text AS kind, c.reltuples::pg_catalog.text AS reltuples
FROM pg_catalog.pg_class c
WHERE c.relnamespace = :ns::pg_catalog.oid AND c.relkind IN ('r', 'p', 'm', 'f')
  AND c.oid IN :[oids]

-- The roles of the server (the NsKind::Server namespace, Names level). pg_roles is readable
-- by every login. A refused read fails the namespace and is never read as no roles.
--- name: ListRoles ---
--- returns: oid u32, name text ---
SELECT r.oid AS oid, r.rolname::pg_catalog.text AS name
FROM pg_catalog.pg_roles r
ORDER BY r.rolname
