-- Hacivat query pack: PostgreSQL, file 30-state (HV-INTRO-04, R-M4-32).
-- Contents: StateOf<Kind> stamps for check_state (HV-INTRO-15; joinable, R-M4-29, R-M4-48).
-- Written by M4 Task 16 from the PostgreSQL documentation only (system catalogs, chapter 51);
-- no third-party catalog SQL.
--
-- Each section returns, for every object oid in :[oids], one row per catalog row that makes
-- up the object's state: its own row (src self, which proves the object exists), its comment
-- rows, its children's rows (a relation's columns, defaults, indexes, constraints, triggers,
-- view rule, inheritance and partition rows; a type's labels, constraints, range and
-- attributes) and the rows one pg_depend hop away (deptype not e, x, p), so a rename of a
-- dependency moves the stamp too (PG spike section 7 Q2). The stamp is the row's xmin as text
-- (with a discriminator where one object owns several rows); the client hashes the rows
-- raw, and Task 22 compares by equality. A removed comment removes its row, so the stamp
-- moves as well.
--
-- These sections are joinable (they may run inside a Hacivat-owned job transaction): every
-- function call is pg_catalog-qualified and allowlisted, and no deparsed text is read
-- (R-M4-46). The union sits in a subquery so the list bind stays a top-level conjunct.

--- name: StateOfRelation ---
--- returns: oid u32, src text, stamp text ---
--- joinable ---
SELECT s.o AS oid, s.src AS src, s.x AS stamp FROM (
  SELECT c.oid AS o, 'self' AS src, CAST(c.xmin AS text) AS x FROM pg_catalog.pg_class c
  UNION ALL SELECT a.attrelid, 'att', CAST(a.attnum AS text) || ':' || CAST(a.xmin AS text) FROM pg_catalog.pg_attribute a
  WHERE a.attnum > 0
  UNION ALL SELECT ad.adrelid, 'def', CAST(ad.adnum AS text) || ':' || CAST(ad.xmin AS text) FROM pg_catalog.pg_attrdef ad
  UNION ALL SELECT i.indrelid, 'idx', CAST(i.indexrelid AS text) || ':' || CAST(i.xmin AS text) || ':' || CAST(ic.xmin AS text)
  FROM pg_catalog.pg_index i JOIN pg_catalog.pg_class ic ON ic.oid = i.indexrelid
  UNION ALL SELECT k.conrelid, 'con', CAST(k.oid AS text) || ':' || CAST(k.xmin AS text) FROM pg_catalog.pg_constraint k
  WHERE k.conrelid <> 0
  UNION ALL SELECT t.tgrelid, 'trg', CAST(t.oid AS text) || ':' || CAST(t.xmin AS text) FROM pg_catalog.pg_trigger t
  WHERE NOT t.tgisinternal
  UNION ALL SELECT r.ev_class, 'rule', CAST(r.oid AS text) || ':' || CAST(r.xmin AS text) FROM pg_catalog.pg_rewrite r
  UNION ALL SELECT ih.inhrelid, 'inh', CAST(ih.inhparent AS text) || ':' || CAST(ih.xmin AS text) FROM pg_catalog.pg_inherits ih
  UNION ALL SELECT pt.partrelid, 'part', CAST(pt.xmin AS text) FROM pg_catalog.pg_partitioned_table pt
  UNION ALL SELECT sq.seqrelid, 'seq', CAST(sq.xmin AS text) FROM pg_catalog.pg_sequence sq
  UNION ALL SELECT d.objoid, 'desc', CAST(d.objsubid AS text) || ':' || CAST(d.xmin AS text) FROM pg_catalog.pg_description d
  WHERE d.classoid = CAST('pg_catalog.pg_class' AS pg_catalog.regclass)
  UNION ALL SELECT i.indrelid, 'idesc', CAST(d.objoid AS text) || ':' || CAST(d.xmin AS text)
  FROM pg_catalog.pg_index i JOIN pg_catalog.pg_description d ON d.objoid = i.indexrelid
  WHERE d.classoid = CAST('pg_catalog.pg_class' AS pg_catalog.regclass)
  UNION ALL SELECT k.conrelid, 'kdesc', CAST(d.objoid AS text) || ':' || CAST(d.xmin AS text)
  FROM pg_catalog.pg_constraint k JOIN pg_catalog.pg_description d ON d.objoid = k.oid
  WHERE k.conrelid <> 0 AND d.classoid = CAST('pg_catalog.pg_constraint' AS pg_catalog.regclass)
  UNION ALL SELECT t.tgrelid, 'tdesc', CAST(d.objoid AS text) || ':' || CAST(d.xmin AS text)
  FROM pg_catalog.pg_trigger t JOIN pg_catalog.pg_description d ON d.objoid = t.oid
  WHERE d.classoid = CAST('pg_catalog.pg_trigger' AS pg_catalog.regclass)
  UNION ALL SELECT dp.objid, 'dep', CAST(dp.refclassid AS text) || ':' || CAST(dp.refobjid AS text) || ':'
    || COALESCE(CAST(rc.xmin AS text), CAST(rt.xmin AS text), CAST(rp.xmin AS text), '')
  FROM pg_catalog.pg_depend dp
  LEFT JOIN pg_catalog.pg_class rc ON dp.refclassid = CAST('pg_catalog.pg_class' AS pg_catalog.regclass) AND rc.oid = dp.refobjid
  LEFT JOIN pg_catalog.pg_type rt ON dp.refclassid = CAST('pg_catalog.pg_type' AS pg_catalog.regclass) AND rt.oid = dp.refobjid
  LEFT JOIN pg_catalog.pg_proc rp ON dp.refclassid = CAST('pg_catalog.pg_proc' AS pg_catalog.regclass) AND rp.oid = dp.refobjid
  WHERE dp.classid = CAST('pg_catalog.pg_class' AS pg_catalog.regclass) AND dp.deptype NOT IN ('e', 'x', 'p')
  UNION ALL SELECT r.ev_class, 'rdep', CAST(dp.refobjid AS text) || ':' || COALESCE(CAST(rc.xmin AS text), CAST(rp.xmin AS text), '')
  FROM pg_catalog.pg_rewrite r
  JOIN pg_catalog.pg_depend dp ON dp.classid = CAST('pg_catalog.pg_rewrite' AS pg_catalog.regclass) AND dp.objid = r.oid
  LEFT JOIN pg_catalog.pg_class rc ON dp.refclassid = CAST('pg_catalog.pg_class' AS pg_catalog.regclass) AND rc.oid = dp.refobjid
  LEFT JOIN pg_catalog.pg_proc rp ON dp.refclassid = CAST('pg_catalog.pg_proc' AS pg_catalog.regclass) AND rp.oid = dp.refobjid
  WHERE dp.deptype NOT IN ('e', 'x', 'p')
  UNION ALL SELECT k.conrelid, 'kdep', CAST(dp.refobjid AS text) || ':' || COALESCE(CAST(rc.xmin AS text), CAST(rp.xmin AS text), '')
  FROM pg_catalog.pg_constraint k
  JOIN pg_catalog.pg_depend dp ON dp.classid = CAST('pg_catalog.pg_constraint' AS pg_catalog.regclass) AND dp.objid = k.oid
  LEFT JOIN pg_catalog.pg_class rc ON dp.refclassid = CAST('pg_catalog.pg_class' AS pg_catalog.regclass) AND rc.oid = dp.refobjid
  LEFT JOIN pg_catalog.pg_proc rp ON dp.refclassid = CAST('pg_catalog.pg_proc' AS pg_catalog.regclass) AND rp.oid = dp.refobjid
  WHERE k.conrelid <> 0 AND dp.deptype NOT IN ('e', 'x', 'p')
  UNION ALL SELECT t.tgrelid, 'tfn', CAST(p.oid AS text) || ':' || CAST(p.xmin AS text)
  FROM pg_catalog.pg_trigger t JOIN pg_catalog.pg_proc p ON p.oid = t.tgfoid
  WHERE NOT t.tgisinternal
  UNION ALL SELECT ad.adrelid, 'ddep', CAST(dp.refobjid AS text) || ':' || COALESCE(CAST(rc.xmin AS text), CAST(rp.xmin AS text), '')
  FROM pg_catalog.pg_attrdef ad
  JOIN pg_catalog.pg_depend dp ON dp.classid = CAST('pg_catalog.pg_attrdef' AS pg_catalog.regclass) AND dp.objid = ad.oid
  LEFT JOIN pg_catalog.pg_class rc ON dp.refclassid = CAST('pg_catalog.pg_class' AS pg_catalog.regclass) AND rc.oid = dp.refobjid
  LEFT JOIN pg_catalog.pg_proc rp ON dp.refclassid = CAST('pg_catalog.pg_proc' AS pg_catalog.regclass) AND rp.oid = dp.refobjid
  WHERE dp.deptype NOT IN ('e', 'x', 'p')
) AS s
WHERE s.o IN :[oids]

--- name: StateOfRoutine ---
--- returns: oid u32, src text, stamp text ---
--- joinable ---
SELECT s.o AS oid, s.src AS src, s.x AS stamp FROM (
  SELECT p.oid AS o, 'self' AS src, CAST(p.xmin AS text) AS x FROM pg_catalog.pg_proc p
  UNION ALL SELECT d.objoid, 'desc', CAST(d.xmin AS text) FROM pg_catalog.pg_description d
  WHERE d.classoid = CAST('pg_catalog.pg_proc' AS pg_catalog.regclass)
  UNION ALL SELECT dp.objid, 'dep', CAST(dp.refclassid AS text) || ':' || CAST(dp.refobjid AS text) || ':'
    || COALESCE(CAST(rc.xmin AS text), CAST(rt.xmin AS text), CAST(rp.xmin AS text), '')
  FROM pg_catalog.pg_depend dp
  LEFT JOIN pg_catalog.pg_class rc ON dp.refclassid = CAST('pg_catalog.pg_class' AS pg_catalog.regclass) AND rc.oid = dp.refobjid
  LEFT JOIN pg_catalog.pg_type rt ON dp.refclassid = CAST('pg_catalog.pg_type' AS pg_catalog.regclass) AND rt.oid = dp.refobjid
  LEFT JOIN pg_catalog.pg_proc rp ON dp.refclassid = CAST('pg_catalog.pg_proc' AS pg_catalog.regclass) AND rp.oid = dp.refobjid
  WHERE dp.classid = CAST('pg_catalog.pg_proc' AS pg_catalog.regclass) AND dp.deptype NOT IN ('e', 'x', 'p')
) AS s
WHERE s.o IN :[oids]

--- name: StateOfType ---
--- returns: oid u32, src text, stamp text ---
--- joinable ---
SELECT s.o AS oid, s.src AS src, s.x AS stamp FROM (
  SELECT t.oid AS o, 'self' AS src, CAST(t.xmin AS text) AS x FROM pg_catalog.pg_type t
  UNION ALL SELECT d.objoid, 'desc', CAST(d.xmin AS text) FROM pg_catalog.pg_description d
  WHERE d.classoid = CAST('pg_catalog.pg_type' AS pg_catalog.regclass)
  UNION ALL SELECT e.enumtypid, 'enum', CAST(e.oid AS text) || ':' || CAST(e.xmin AS text) FROM pg_catalog.pg_enum e
  UNION ALL SELECT k.contypid, 'con', CAST(k.oid AS text) || ':' || CAST(k.xmin AS text) FROM pg_catalog.pg_constraint k
  WHERE k.contypid <> 0
  UNION ALL SELECT r.rngtypid, 'range', CAST(r.xmin AS text) FROM pg_catalog.pg_range r
  UNION ALL SELECT t.oid, 'class', CAST(c.xmin AS text) FROM pg_catalog.pg_type t
  JOIN pg_catalog.pg_class c ON c.oid = t.typrelid
  UNION ALL SELECT t.oid, 'att', CAST(a.attnum AS text) || ':' || CAST(a.xmin AS text) FROM pg_catalog.pg_type t
  JOIN pg_catalog.pg_attribute a ON a.attrelid = t.typrelid
  WHERE a.attnum > 0
  UNION ALL SELECT dp.objid, 'dep', CAST(dp.refclassid AS text) || ':' || CAST(dp.refobjid AS text) || ':'
    || COALESCE(CAST(rc.xmin AS text), CAST(rt.xmin AS text), CAST(rp.xmin AS text), '')
  FROM pg_catalog.pg_depend dp
  LEFT JOIN pg_catalog.pg_class rc ON dp.refclassid = CAST('pg_catalog.pg_class' AS pg_catalog.regclass) AND rc.oid = dp.refobjid
  LEFT JOIN pg_catalog.pg_type rt ON dp.refclassid = CAST('pg_catalog.pg_type' AS pg_catalog.regclass) AND rt.oid = dp.refobjid
  LEFT JOIN pg_catalog.pg_proc rp ON dp.refclassid = CAST('pg_catalog.pg_proc' AS pg_catalog.regclass) AND rp.oid = dp.refobjid
  WHERE dp.classid = CAST('pg_catalog.pg_type' AS pg_catalog.regclass) AND dp.deptype NOT IN ('e', 'x', 'p')
) AS s
WHERE s.o IN :[oids]

--- name: StateOfConstraint ---
--- returns: oid u32, src text, stamp text ---
--- joinable ---
SELECT s.o AS oid, s.src AS src, s.x AS stamp FROM (
  SELECT k.oid AS o, 'self' AS src, CAST(k.xmin AS text) AS x FROM pg_catalog.pg_constraint k
  UNION ALL SELECT d.objoid, 'desc', CAST(d.xmin AS text) FROM pg_catalog.pg_description d
  WHERE d.classoid = CAST('pg_catalog.pg_constraint' AS pg_catalog.regclass)
  UNION ALL SELECT k.oid, 'rel', CAST(c.xmin AS text) FROM pg_catalog.pg_constraint k
  JOIN pg_catalog.pg_class c ON c.oid = k.conrelid
  UNION ALL SELECT dp.objid, 'dep', CAST(dp.refclassid AS text) || ':' || CAST(dp.refobjid AS text) || ':'
    || COALESCE(CAST(rc.xmin AS text), CAST(rp.xmin AS text), '')
  FROM pg_catalog.pg_depend dp
  LEFT JOIN pg_catalog.pg_class rc ON dp.refclassid = CAST('pg_catalog.pg_class' AS pg_catalog.regclass) AND rc.oid = dp.refobjid
  LEFT JOIN pg_catalog.pg_proc rp ON dp.refclassid = CAST('pg_catalog.pg_proc' AS pg_catalog.regclass) AND rp.oid = dp.refobjid
  WHERE dp.classid = CAST('pg_catalog.pg_constraint' AS pg_catalog.regclass) AND dp.deptype NOT IN ('e', 'x', 'p')
) AS s
WHERE s.o IN :[oids]

--- name: StateOfTrigger ---
--- returns: oid u32, src text, stamp text ---
--- joinable ---
SELECT s.o AS oid, s.src AS src, s.x AS stamp FROM (
  SELECT t.oid AS o, 'self' AS src, CAST(t.xmin AS text) AS x FROM pg_catalog.pg_trigger t
  UNION ALL SELECT d.objoid, 'desc', CAST(d.xmin AS text) FROM pg_catalog.pg_description d
  WHERE d.classoid = CAST('pg_catalog.pg_trigger' AS pg_catalog.regclass)
  UNION ALL SELECT t.oid, 'rel', CAST(c.xmin AS text) FROM pg_catalog.pg_trigger t
  JOIN pg_catalog.pg_class c ON c.oid = t.tgrelid
  UNION ALL SELECT t.oid, 'fn', CAST(p.xmin AS text) FROM pg_catalog.pg_trigger t
  JOIN pg_catalog.pg_proc p ON p.oid = t.tgfoid
) AS s
WHERE s.o IN :[oids]
