-- Hacivat query pack: PostgreSQL, file 30-state (HV-INTRO-04, R-M4-32).
-- Contents: StateOf<Kind> stamps for check_state (HV-INTRO-15. Joinable, R-M4-29, R-M4-48).
-- Written by M4 Task 16 from the PostgreSQL documentation only (system catalogs, chapter 51,
-- and the OPERATOR() syntax of section 4.2.5). No third-party catalog SQL.
--
-- Each section returns, for every object oid in :[oids], one row per catalog row that makes
-- up the object's state: its own row (src self, which proves the object exists), its comment
-- rows, its children's rows (a relation's columns, defaults, indexes, constraints, triggers,
-- view rule, inheritance and partition rows. A type's labels, constraints, range and
-- attributes) and the rows one pg_depend hop away (deptype not e, x, p): the referenced
-- relation, the referenced column (a column rename touches pg_attribute only), the referenced
-- type and routine, so a rename of a dependency moves the stamp too (PG spike section 7 Q2).
-- The stamp is the row's xmin as text (with a discriminator where one object owns several
-- rows). The client hashes the rows raw, and Task 22 compares by equality. A removed comment
-- removes its row, so the stamp moves as well.
--
-- These sections are joinable: they may run inside a Hacivat-owned job transaction whose
-- search_path Hacivat cannot pin (R-M4-46). So nothing in them resolves through the path:
-- every operator is OPERATOR(pg_catalog.op), every cast names a pg_catalog type, every
-- function is pg_catalog-qualified (review I-4, the CVE-2018-1058 class). The one exception
-- is the IN of the list bind, which the pack grammar fixes. Its operands are oid and a
-- parameter of inferred type oid, and a joined read must refuse a session whose path does not start
-- with pg_catalog (carried to T15b and M7). The union sits in a subquery so the list bind
-- stays a top-level conjunct.

--- name: StateOfRelation ---
--- returns: oid u32, src text, stamp text ---
--- joinable ---
SELECT s.o AS oid, s.src AS src, s.x AS stamp FROM (
  SELECT c.oid AS o, 'self' AS src, CAST(c.xmin AS pg_catalog.text) AS x FROM pg_catalog.pg_class c
  UNION ALL SELECT a.attrelid, 'att',
    CAST(a.attnum AS pg_catalog.text) OPERATOR(pg_catalog.||) ':' OPERATOR(pg_catalog.||) CAST(a.xmin AS pg_catalog.text)
  FROM pg_catalog.pg_attribute a
  WHERE a.attnum OPERATOR(pg_catalog.>) 0
  UNION ALL SELECT ad.adrelid, 'def',
    CAST(ad.adnum AS pg_catalog.text) OPERATOR(pg_catalog.||) ':' OPERATOR(pg_catalog.||) CAST(ad.xmin AS pg_catalog.text)
  FROM pg_catalog.pg_attrdef ad
  UNION ALL SELECT i.indrelid, 'idx',
    CAST(i.indexrelid AS pg_catalog.text) OPERATOR(pg_catalog.||) ':' OPERATOR(pg_catalog.||) CAST(i.xmin AS pg_catalog.text)
    OPERATOR(pg_catalog.||) ':' OPERATOR(pg_catalog.||) CAST(ic.xmin AS pg_catalog.text)
  FROM pg_catalog.pg_index i JOIN pg_catalog.pg_class ic ON ic.oid OPERATOR(pg_catalog.=) i.indexrelid
  UNION ALL SELECT k.conrelid, 'con',
    CAST(k.oid AS pg_catalog.text) OPERATOR(pg_catalog.||) ':' OPERATOR(pg_catalog.||) CAST(k.xmin AS pg_catalog.text)
  FROM pg_catalog.pg_constraint k
  WHERE k.conrelid OPERATOR(pg_catalog.<>) CAST(0 AS pg_catalog.oid)
  UNION ALL SELECT t.tgrelid, 'trg',
    CAST(t.oid AS pg_catalog.text) OPERATOR(pg_catalog.||) ':' OPERATOR(pg_catalog.||) CAST(t.xmin AS pg_catalog.text)
  FROM pg_catalog.pg_trigger t
  WHERE NOT t.tgisinternal
  UNION ALL SELECT r.ev_class, 'rule',
    CAST(r.oid AS pg_catalog.text) OPERATOR(pg_catalog.||) ':' OPERATOR(pg_catalog.||) CAST(r.xmin AS pg_catalog.text)
  FROM pg_catalog.pg_rewrite r
  UNION ALL SELECT ih.inhrelid, 'inh',
    CAST(ih.inhparent AS pg_catalog.text) OPERATOR(pg_catalog.||) ':' OPERATOR(pg_catalog.||) CAST(ih.xmin AS pg_catalog.text)
  FROM pg_catalog.pg_inherits ih
  UNION ALL SELECT pt.partrelid, 'part', CAST(pt.xmin AS pg_catalog.text) FROM pg_catalog.pg_partitioned_table pt
  UNION ALL SELECT sq.seqrelid, 'seq', CAST(sq.xmin AS pg_catalog.text) FROM pg_catalog.pg_sequence sq
  UNION ALL SELECT d.objoid, 'desc',
    CAST(d.objsubid AS pg_catalog.text) OPERATOR(pg_catalog.||) ':' OPERATOR(pg_catalog.||) CAST(d.xmin AS pg_catalog.text)
  FROM pg_catalog.pg_description d
  WHERE d.classoid OPERATOR(pg_catalog.=) CAST(CAST('pg_catalog.pg_class' AS pg_catalog.regclass) AS pg_catalog.oid)
  UNION ALL SELECT i.indrelid, 'idesc',
    CAST(d.objoid AS pg_catalog.text) OPERATOR(pg_catalog.||) ':' OPERATOR(pg_catalog.||) CAST(d.xmin AS pg_catalog.text)
  FROM pg_catalog.pg_index i JOIN pg_catalog.pg_description d ON d.objoid OPERATOR(pg_catalog.=) i.indexrelid
  WHERE d.classoid OPERATOR(pg_catalog.=) CAST(CAST('pg_catalog.pg_class' AS pg_catalog.regclass) AS pg_catalog.oid)
  UNION ALL SELECT k.conrelid, 'kdesc',
    CAST(d.objoid AS pg_catalog.text) OPERATOR(pg_catalog.||) ':' OPERATOR(pg_catalog.||) CAST(d.xmin AS pg_catalog.text)
  FROM pg_catalog.pg_constraint k JOIN pg_catalog.pg_description d ON d.objoid OPERATOR(pg_catalog.=) k.oid
  WHERE k.conrelid OPERATOR(pg_catalog.<>) CAST(0 AS pg_catalog.oid)
    AND d.classoid OPERATOR(pg_catalog.=) CAST(CAST('pg_catalog.pg_constraint' AS pg_catalog.regclass) AS pg_catalog.oid)
  UNION ALL SELECT t.tgrelid, 'tdesc',
    CAST(d.objoid AS pg_catalog.text) OPERATOR(pg_catalog.||) ':' OPERATOR(pg_catalog.||) CAST(d.xmin AS pg_catalog.text)
  FROM pg_catalog.pg_trigger t JOIN pg_catalog.pg_description d ON d.objoid OPERATOR(pg_catalog.=) t.oid
  WHERE d.classoid OPERATOR(pg_catalog.=) CAST(CAST('pg_catalog.pg_trigger' AS pg_catalog.regclass) AS pg_catalog.oid)
  UNION ALL SELECT dp.objid, 'dep',
    CAST(dp.refclassid AS pg_catalog.text) OPERATOR(pg_catalog.||) ':' OPERATOR(pg_catalog.||) CAST(dp.refobjid AS pg_catalog.text)
    OPERATOR(pg_catalog.||) ':' OPERATOR(pg_catalog.||) CAST(dp.refobjsubid AS pg_catalog.text)
    OPERATOR(pg_catalog.||) ':' OPERATOR(pg_catalog.||) COALESCE(CAST(rc.xmin AS pg_catalog.text), '')
    OPERATOR(pg_catalog.||) ':' OPERATOR(pg_catalog.||) COALESCE(CAST(ra.xmin AS pg_catalog.text), '')
    OPERATOR(pg_catalog.||) ':' OPERATOR(pg_catalog.||) COALESCE(CAST(rt.xmin AS pg_catalog.text), '')
    OPERATOR(pg_catalog.||) ':' OPERATOR(pg_catalog.||) COALESCE(CAST(rp.xmin AS pg_catalog.text), '')
  FROM pg_catalog.pg_depend dp
  LEFT JOIN pg_catalog.pg_class rc ON dp.refclassid OPERATOR(pg_catalog.=) CAST(CAST('pg_catalog.pg_class' AS pg_catalog.regclass) AS pg_catalog.oid)
    AND rc.oid OPERATOR(pg_catalog.=) dp.refobjid
  LEFT JOIN pg_catalog.pg_attribute ra ON dp.refclassid OPERATOR(pg_catalog.=) CAST(CAST('pg_catalog.pg_class' AS pg_catalog.regclass) AS pg_catalog.oid)
    AND ra.attrelid OPERATOR(pg_catalog.=) dp.refobjid AND ra.attnum OPERATOR(pg_catalog.=) dp.refobjsubid
  LEFT JOIN pg_catalog.pg_type rt ON dp.refclassid OPERATOR(pg_catalog.=) CAST(CAST('pg_catalog.pg_type' AS pg_catalog.regclass) AS pg_catalog.oid)
    AND rt.oid OPERATOR(pg_catalog.=) dp.refobjid
  LEFT JOIN pg_catalog.pg_proc rp ON dp.refclassid OPERATOR(pg_catalog.=) CAST(CAST('pg_catalog.pg_proc' AS pg_catalog.regclass) AS pg_catalog.oid)
    AND rp.oid OPERATOR(pg_catalog.=) dp.refobjid
  WHERE dp.classid OPERATOR(pg_catalog.=) CAST(CAST('pg_catalog.pg_class' AS pg_catalog.regclass) AS pg_catalog.oid)
    AND dp.deptype OPERATOR(pg_catalog.<>) 'e' AND dp.deptype OPERATOR(pg_catalog.<>) 'x'
    AND dp.deptype OPERATOR(pg_catalog.<>) 'p'
  UNION ALL SELECT r.ev_class, 'rdep',
    CAST(dp.refobjid AS pg_catalog.text) OPERATOR(pg_catalog.||) ':' OPERATOR(pg_catalog.||) CAST(dp.refobjsubid AS pg_catalog.text)
    OPERATOR(pg_catalog.||) ':' OPERATOR(pg_catalog.||) COALESCE(CAST(rc.xmin AS pg_catalog.text), '')
    OPERATOR(pg_catalog.||) ':' OPERATOR(pg_catalog.||) COALESCE(CAST(ra.xmin AS pg_catalog.text), '')
    OPERATOR(pg_catalog.||) ':' OPERATOR(pg_catalog.||) COALESCE(CAST(rt.xmin AS pg_catalog.text), '')
    OPERATOR(pg_catalog.||) ':' OPERATOR(pg_catalog.||) COALESCE(CAST(rp.xmin AS pg_catalog.text), '')
  FROM pg_catalog.pg_rewrite r
  JOIN pg_catalog.pg_depend dp ON dp.classid OPERATOR(pg_catalog.=) CAST(CAST('pg_catalog.pg_rewrite' AS pg_catalog.regclass) AS pg_catalog.oid)
    AND dp.objid OPERATOR(pg_catalog.=) r.oid
  LEFT JOIN pg_catalog.pg_class rc ON dp.refclassid OPERATOR(pg_catalog.=) CAST(CAST('pg_catalog.pg_class' AS pg_catalog.regclass) AS pg_catalog.oid)
    AND rc.oid OPERATOR(pg_catalog.=) dp.refobjid
  LEFT JOIN pg_catalog.pg_attribute ra ON dp.refclassid OPERATOR(pg_catalog.=) CAST(CAST('pg_catalog.pg_class' AS pg_catalog.regclass) AS pg_catalog.oid)
    AND ra.attrelid OPERATOR(pg_catalog.=) dp.refobjid AND ra.attnum OPERATOR(pg_catalog.=) dp.refobjsubid
  LEFT JOIN pg_catalog.pg_type rt ON dp.refclassid OPERATOR(pg_catalog.=) CAST(CAST('pg_catalog.pg_type' AS pg_catalog.regclass) AS pg_catalog.oid)
    AND rt.oid OPERATOR(pg_catalog.=) dp.refobjid
  LEFT JOIN pg_catalog.pg_proc rp ON dp.refclassid OPERATOR(pg_catalog.=) CAST(CAST('pg_catalog.pg_proc' AS pg_catalog.regclass) AS pg_catalog.oid)
    AND rp.oid OPERATOR(pg_catalog.=) dp.refobjid
  WHERE dp.deptype OPERATOR(pg_catalog.<>) 'e' AND dp.deptype OPERATOR(pg_catalog.<>) 'x'
    AND dp.deptype OPERATOR(pg_catalog.<>) 'p'
  UNION ALL SELECT k.conrelid, 'kdep',
    CAST(dp.refobjid AS pg_catalog.text) OPERATOR(pg_catalog.||) ':' OPERATOR(pg_catalog.||) CAST(dp.refobjsubid AS pg_catalog.text)
    OPERATOR(pg_catalog.||) ':' OPERATOR(pg_catalog.||) COALESCE(CAST(rc.xmin AS pg_catalog.text), '')
    OPERATOR(pg_catalog.||) ':' OPERATOR(pg_catalog.||) COALESCE(CAST(ra.xmin AS pg_catalog.text), '')
    OPERATOR(pg_catalog.||) ':' OPERATOR(pg_catalog.||) COALESCE(CAST(rt.xmin AS pg_catalog.text), '')
    OPERATOR(pg_catalog.||) ':' OPERATOR(pg_catalog.||) COALESCE(CAST(rp.xmin AS pg_catalog.text), '')
  FROM pg_catalog.pg_constraint k
  JOIN pg_catalog.pg_depend dp ON dp.classid OPERATOR(pg_catalog.=) CAST(CAST('pg_catalog.pg_constraint' AS pg_catalog.regclass) AS pg_catalog.oid)
    AND dp.objid OPERATOR(pg_catalog.=) k.oid
  LEFT JOIN pg_catalog.pg_class rc ON dp.refclassid OPERATOR(pg_catalog.=) CAST(CAST('pg_catalog.pg_class' AS pg_catalog.regclass) AS pg_catalog.oid)
    AND rc.oid OPERATOR(pg_catalog.=) dp.refobjid
  LEFT JOIN pg_catalog.pg_attribute ra ON dp.refclassid OPERATOR(pg_catalog.=) CAST(CAST('pg_catalog.pg_class' AS pg_catalog.regclass) AS pg_catalog.oid)
    AND ra.attrelid OPERATOR(pg_catalog.=) dp.refobjid AND ra.attnum OPERATOR(pg_catalog.=) dp.refobjsubid
  LEFT JOIN pg_catalog.pg_type rt ON dp.refclassid OPERATOR(pg_catalog.=) CAST(CAST('pg_catalog.pg_type' AS pg_catalog.regclass) AS pg_catalog.oid)
    AND rt.oid OPERATOR(pg_catalog.=) dp.refobjid
  LEFT JOIN pg_catalog.pg_proc rp ON dp.refclassid OPERATOR(pg_catalog.=) CAST(CAST('pg_catalog.pg_proc' AS pg_catalog.regclass) AS pg_catalog.oid)
    AND rp.oid OPERATOR(pg_catalog.=) dp.refobjid
  WHERE k.conrelid OPERATOR(pg_catalog.<>) CAST(0 AS pg_catalog.oid)
    AND dp.deptype OPERATOR(pg_catalog.<>) 'e' AND dp.deptype OPERATOR(pg_catalog.<>) 'x'
    AND dp.deptype OPERATOR(pg_catalog.<>) 'p'
  UNION ALL SELECT t.tgrelid, 'tfn',
    CAST(p.oid AS pg_catalog.text) OPERATOR(pg_catalog.||) ':' OPERATOR(pg_catalog.||) CAST(p.xmin AS pg_catalog.text)
  FROM pg_catalog.pg_trigger t JOIN pg_catalog.pg_proc p ON p.oid OPERATOR(pg_catalog.=) t.tgfoid
  WHERE NOT t.tgisinternal
  UNION ALL SELECT ad.adrelid, 'ddep',
    CAST(dp.refobjid AS pg_catalog.text) OPERATOR(pg_catalog.||) ':' OPERATOR(pg_catalog.||) CAST(dp.refobjsubid AS pg_catalog.text)
    OPERATOR(pg_catalog.||) ':' OPERATOR(pg_catalog.||) COALESCE(CAST(rc.xmin AS pg_catalog.text), '')
    OPERATOR(pg_catalog.||) ':' OPERATOR(pg_catalog.||) COALESCE(CAST(ra.xmin AS pg_catalog.text), '')
    OPERATOR(pg_catalog.||) ':' OPERATOR(pg_catalog.||) COALESCE(CAST(rt.xmin AS pg_catalog.text), '')
    OPERATOR(pg_catalog.||) ':' OPERATOR(pg_catalog.||) COALESCE(CAST(rp.xmin AS pg_catalog.text), '')
  FROM pg_catalog.pg_attrdef ad
  JOIN pg_catalog.pg_depend dp ON dp.classid OPERATOR(pg_catalog.=) CAST(CAST('pg_catalog.pg_attrdef' AS pg_catalog.regclass) AS pg_catalog.oid)
    AND dp.objid OPERATOR(pg_catalog.=) ad.oid
  LEFT JOIN pg_catalog.pg_class rc ON dp.refclassid OPERATOR(pg_catalog.=) CAST(CAST('pg_catalog.pg_class' AS pg_catalog.regclass) AS pg_catalog.oid)
    AND rc.oid OPERATOR(pg_catalog.=) dp.refobjid
  LEFT JOIN pg_catalog.pg_attribute ra ON dp.refclassid OPERATOR(pg_catalog.=) CAST(CAST('pg_catalog.pg_class' AS pg_catalog.regclass) AS pg_catalog.oid)
    AND ra.attrelid OPERATOR(pg_catalog.=) dp.refobjid AND ra.attnum OPERATOR(pg_catalog.=) dp.refobjsubid
  LEFT JOIN pg_catalog.pg_type rt ON dp.refclassid OPERATOR(pg_catalog.=) CAST(CAST('pg_catalog.pg_type' AS pg_catalog.regclass) AS pg_catalog.oid)
    AND rt.oid OPERATOR(pg_catalog.=) dp.refobjid
  LEFT JOIN pg_catalog.pg_proc rp ON dp.refclassid OPERATOR(pg_catalog.=) CAST(CAST('pg_catalog.pg_proc' AS pg_catalog.regclass) AS pg_catalog.oid)
    AND rp.oid OPERATOR(pg_catalog.=) dp.refobjid
  WHERE dp.deptype OPERATOR(pg_catalog.<>) 'e' AND dp.deptype OPERATOR(pg_catalog.<>) 'x'
    AND dp.deptype OPERATOR(pg_catalog.<>) 'p'
) AS s
WHERE s.o IN :[oids]

--- name: StateOfRoutine ---
--- returns: oid u32, src text, stamp text ---
--- joinable ---
SELECT s.o AS oid, s.src AS src, s.x AS stamp FROM (
  SELECT p.oid AS o, 'self' AS src, CAST(p.xmin AS pg_catalog.text) AS x FROM pg_catalog.pg_proc p
  UNION ALL SELECT d.objoid, 'desc', CAST(d.xmin AS pg_catalog.text) FROM pg_catalog.pg_description d
  WHERE d.classoid OPERATOR(pg_catalog.=) CAST(CAST('pg_catalog.pg_proc' AS pg_catalog.regclass) AS pg_catalog.oid)
  UNION ALL SELECT dp.objid, 'dep',
    CAST(dp.refclassid AS pg_catalog.text) OPERATOR(pg_catalog.||) ':' OPERATOR(pg_catalog.||) CAST(dp.refobjid AS pg_catalog.text)
    OPERATOR(pg_catalog.||) ':' OPERATOR(pg_catalog.||) COALESCE(CAST(rc.xmin AS pg_catalog.text), '')
    OPERATOR(pg_catalog.||) ':' OPERATOR(pg_catalog.||) COALESCE(CAST(rt.xmin AS pg_catalog.text), '')
    OPERATOR(pg_catalog.||) ':' OPERATOR(pg_catalog.||) COALESCE(CAST(rp.xmin AS pg_catalog.text), '')
  FROM pg_catalog.pg_depend dp
  LEFT JOIN pg_catalog.pg_class rc ON dp.refclassid OPERATOR(pg_catalog.=) CAST(CAST('pg_catalog.pg_class' AS pg_catalog.regclass) AS pg_catalog.oid)
    AND rc.oid OPERATOR(pg_catalog.=) dp.refobjid
  LEFT JOIN pg_catalog.pg_type rt ON dp.refclassid OPERATOR(pg_catalog.=) CAST(CAST('pg_catalog.pg_type' AS pg_catalog.regclass) AS pg_catalog.oid)
    AND rt.oid OPERATOR(pg_catalog.=) dp.refobjid
  LEFT JOIN pg_catalog.pg_proc rp ON dp.refclassid OPERATOR(pg_catalog.=) CAST(CAST('pg_catalog.pg_proc' AS pg_catalog.regclass) AS pg_catalog.oid)
    AND rp.oid OPERATOR(pg_catalog.=) dp.refobjid
  WHERE dp.classid OPERATOR(pg_catalog.=) CAST(CAST('pg_catalog.pg_proc' AS pg_catalog.regclass) AS pg_catalog.oid)
    AND dp.deptype OPERATOR(pg_catalog.<>) 'e' AND dp.deptype OPERATOR(pg_catalog.<>) 'x'
    AND dp.deptype OPERATOR(pg_catalog.<>) 'p'
) AS s
WHERE s.o IN :[oids]

--- name: StateOfType ---
--- returns: oid u32, src text, stamp text ---
--- joinable ---
SELECT s.o AS oid, s.src AS src, s.x AS stamp FROM (
  SELECT t.oid AS o, 'self' AS src, CAST(t.xmin AS pg_catalog.text) AS x FROM pg_catalog.pg_type t
  UNION ALL SELECT d.objoid, 'desc', CAST(d.xmin AS pg_catalog.text) FROM pg_catalog.pg_description d
  WHERE d.classoid OPERATOR(pg_catalog.=) CAST(CAST('pg_catalog.pg_type' AS pg_catalog.regclass) AS pg_catalog.oid)
  UNION ALL SELECT e.enumtypid, 'enum',
    CAST(e.oid AS pg_catalog.text) OPERATOR(pg_catalog.||) ':' OPERATOR(pg_catalog.||) CAST(e.xmin AS pg_catalog.text)
  FROM pg_catalog.pg_enum e
  UNION ALL SELECT k.contypid, 'con',
    CAST(k.oid AS pg_catalog.text) OPERATOR(pg_catalog.||) ':' OPERATOR(pg_catalog.||) CAST(k.xmin AS pg_catalog.text)
  FROM pg_catalog.pg_constraint k
  WHERE k.contypid OPERATOR(pg_catalog.<>) CAST(0 AS pg_catalog.oid)
  UNION ALL SELECT r.rngtypid, 'range', CAST(r.xmin AS pg_catalog.text) FROM pg_catalog.pg_range r
  UNION ALL SELECT t.oid, 'class', CAST(c.xmin AS pg_catalog.text) FROM pg_catalog.pg_type t
  JOIN pg_catalog.pg_class c ON c.oid OPERATOR(pg_catalog.=) t.typrelid
  UNION ALL SELECT t.oid, 'att',
    CAST(a.attnum AS pg_catalog.text) OPERATOR(pg_catalog.||) ':' OPERATOR(pg_catalog.||) CAST(a.xmin AS pg_catalog.text)
  FROM pg_catalog.pg_type t
  JOIN pg_catalog.pg_attribute a ON a.attrelid OPERATOR(pg_catalog.=) t.typrelid
  WHERE a.attnum OPERATOR(pg_catalog.>) 0
  UNION ALL SELECT dp.objid, 'dep',
    CAST(dp.refclassid AS pg_catalog.text) OPERATOR(pg_catalog.||) ':' OPERATOR(pg_catalog.||) CAST(dp.refobjid AS pg_catalog.text)
    OPERATOR(pg_catalog.||) ':' OPERATOR(pg_catalog.||) COALESCE(CAST(rc.xmin AS pg_catalog.text), '')
    OPERATOR(pg_catalog.||) ':' OPERATOR(pg_catalog.||) COALESCE(CAST(rt.xmin AS pg_catalog.text), '')
    OPERATOR(pg_catalog.||) ':' OPERATOR(pg_catalog.||) COALESCE(CAST(rp.xmin AS pg_catalog.text), '')
  FROM pg_catalog.pg_depend dp
  LEFT JOIN pg_catalog.pg_class rc ON dp.refclassid OPERATOR(pg_catalog.=) CAST(CAST('pg_catalog.pg_class' AS pg_catalog.regclass) AS pg_catalog.oid)
    AND rc.oid OPERATOR(pg_catalog.=) dp.refobjid
  LEFT JOIN pg_catalog.pg_type rt ON dp.refclassid OPERATOR(pg_catalog.=) CAST(CAST('pg_catalog.pg_type' AS pg_catalog.regclass) AS pg_catalog.oid)
    AND rt.oid OPERATOR(pg_catalog.=) dp.refobjid
  LEFT JOIN pg_catalog.pg_proc rp ON dp.refclassid OPERATOR(pg_catalog.=) CAST(CAST('pg_catalog.pg_proc' AS pg_catalog.regclass) AS pg_catalog.oid)
    AND rp.oid OPERATOR(pg_catalog.=) dp.refobjid
  WHERE dp.classid OPERATOR(pg_catalog.=) CAST(CAST('pg_catalog.pg_type' AS pg_catalog.regclass) AS pg_catalog.oid)
    AND dp.deptype OPERATOR(pg_catalog.<>) 'e' AND dp.deptype OPERATOR(pg_catalog.<>) 'x'
    AND dp.deptype OPERATOR(pg_catalog.<>) 'p'
) AS s
WHERE s.o IN :[oids]

--- name: StateOfConstraint ---
--- returns: oid u32, src text, stamp text ---
--- joinable ---
SELECT s.o AS oid, s.src AS src, s.x AS stamp FROM (
  SELECT k.oid AS o, 'self' AS src, CAST(k.xmin AS pg_catalog.text) AS x FROM pg_catalog.pg_constraint k
  UNION ALL SELECT d.objoid, 'desc', CAST(d.xmin AS pg_catalog.text) FROM pg_catalog.pg_description d
  WHERE d.classoid OPERATOR(pg_catalog.=) CAST(CAST('pg_catalog.pg_constraint' AS pg_catalog.regclass) AS pg_catalog.oid)
  UNION ALL SELECT k.oid, 'rel', CAST(c.xmin AS pg_catalog.text) FROM pg_catalog.pg_constraint k
  JOIN pg_catalog.pg_class c ON c.oid OPERATOR(pg_catalog.=) k.conrelid
  UNION ALL SELECT dp.objid, 'dep',
    CAST(dp.refclassid AS pg_catalog.text) OPERATOR(pg_catalog.||) ':' OPERATOR(pg_catalog.||) CAST(dp.refobjid AS pg_catalog.text)
    OPERATOR(pg_catalog.||) ':' OPERATOR(pg_catalog.||) CAST(dp.refobjsubid AS pg_catalog.text)
    OPERATOR(pg_catalog.||) ':' OPERATOR(pg_catalog.||) COALESCE(CAST(rc.xmin AS pg_catalog.text), '')
    OPERATOR(pg_catalog.||) ':' OPERATOR(pg_catalog.||) COALESCE(CAST(ra.xmin AS pg_catalog.text), '')
    OPERATOR(pg_catalog.||) ':' OPERATOR(pg_catalog.||) COALESCE(CAST(rt.xmin AS pg_catalog.text), '')
    OPERATOR(pg_catalog.||) ':' OPERATOR(pg_catalog.||) COALESCE(CAST(rp.xmin AS pg_catalog.text), '')
  FROM pg_catalog.pg_depend dp
  LEFT JOIN pg_catalog.pg_class rc ON dp.refclassid OPERATOR(pg_catalog.=) CAST(CAST('pg_catalog.pg_class' AS pg_catalog.regclass) AS pg_catalog.oid)
    AND rc.oid OPERATOR(pg_catalog.=) dp.refobjid
  LEFT JOIN pg_catalog.pg_attribute ra ON dp.refclassid OPERATOR(pg_catalog.=) CAST(CAST('pg_catalog.pg_class' AS pg_catalog.regclass) AS pg_catalog.oid)
    AND ra.attrelid OPERATOR(pg_catalog.=) dp.refobjid AND ra.attnum OPERATOR(pg_catalog.=) dp.refobjsubid
  LEFT JOIN pg_catalog.pg_type rt ON dp.refclassid OPERATOR(pg_catalog.=) CAST(CAST('pg_catalog.pg_type' AS pg_catalog.regclass) AS pg_catalog.oid)
    AND rt.oid OPERATOR(pg_catalog.=) dp.refobjid
  LEFT JOIN pg_catalog.pg_proc rp ON dp.refclassid OPERATOR(pg_catalog.=) CAST(CAST('pg_catalog.pg_proc' AS pg_catalog.regclass) AS pg_catalog.oid)
    AND rp.oid OPERATOR(pg_catalog.=) dp.refobjid
  WHERE dp.classid OPERATOR(pg_catalog.=) CAST(CAST('pg_catalog.pg_constraint' AS pg_catalog.regclass) AS pg_catalog.oid)
    AND dp.deptype OPERATOR(pg_catalog.<>) 'e' AND dp.deptype OPERATOR(pg_catalog.<>) 'x'
    AND dp.deptype OPERATOR(pg_catalog.<>) 'p'
) AS s
WHERE s.o IN :[oids]

--- name: StateOfTrigger ---
--- returns: oid u32, src text, stamp text ---
--- joinable ---
SELECT s.o AS oid, s.src AS src, s.x AS stamp FROM (
  SELECT t.oid AS o, 'self' AS src, CAST(t.xmin AS pg_catalog.text) AS x FROM pg_catalog.pg_trigger t
  UNION ALL SELECT d.objoid, 'desc', CAST(d.xmin AS pg_catalog.text) FROM pg_catalog.pg_description d
  WHERE d.classoid OPERATOR(pg_catalog.=) CAST(CAST('pg_catalog.pg_trigger' AS pg_catalog.regclass) AS pg_catalog.oid)
  UNION ALL SELECT t.oid, 'rel', CAST(c.xmin AS pg_catalog.text) FROM pg_catalog.pg_trigger t
  JOIN pg_catalog.pg_class c ON c.oid OPERATOR(pg_catalog.=) t.tgrelid
  UNION ALL SELECT t.oid, 'fn', CAST(p.xmin AS pg_catalog.text) FROM pg_catalog.pg_trigger t
  JOIN pg_catalog.pg_proc p ON p.oid OPERATOR(pg_catalog.=) t.tgfoid
) AS s
WHERE s.o IN :[oids]
