-- Hacivat query pack: SQLite, file 10-core (HV-INTRO-04, R-M4-32).
-- Contents: schema list, sqlite_schema rows and the pragma passes (mode=ro, R-M4-38). Sections are written by Task 18 from the vendor documentation only
-- (no third-party catalog SQL); the grammar is Task 4's (`--- name: <Section> ---`).
--
-- ## m4-t18-sqlite (HV-INTRO-08, -13; sources: sqlite.org/pragma.html "PRAGMA functions",
-- sqlite.org/schematab.html). Every pragma is read through its table-valued function (no PRAGMA
-- statement); names and schemas travel as binds. A table-valued pragma takes the schema as its
-- last argument, but `sqlite_schema` itself needs the schema written as a qualifier, which a
-- bind cannot be: so the sections that read `sqlite_schema` come in three copies, `…Main`
-- (`main`), `…Temp` (`temp`, read on the user's own connection with only the pragmas its
-- read-only authorizer allows, R-M4-38) and `…Attached`, which reads the fixed alias
-- `hacivat_attached` that the introspector attaches an attached file under on the read-only
-- connection (ADR 0008 `## m4-t18`). No section uses LIKE, GLOB or a case-folding function, so
-- `case_sensitive_like` cannot change an answer. Every section is joinable: a joined read
-- (R-M4-29) runs the same text on the writer's connection.

--- name: DatabaseList ---
--- returns: seq i64, name text, file text ---
--- joinable ---
SELECT d.seq AS seq, d.name AS name, d.file AS file
FROM pragma_database_list d

--- name: ObjectsMain ---
--- returns: type text, name text, tbl_name text, sql text ---
--- joinable ---
SELECT m.type AS type, m.name AS name, m.tbl_name AS tbl_name, m.sql AS sql
FROM main.sqlite_schema m

--- name: ObjectsTemp ---
--- returns: type text, name text, tbl_name text, sql text ---
--- joinable ---
SELECT m.type AS type, m.name AS name, m.tbl_name AS tbl_name, m.sql AS sql
FROM temp.sqlite_schema m

--- name: ObjectsAttached ---
--- returns: type text, name text, tbl_name text, sql text ---
--- joinable ---
SELECT m.type AS type, m.name AS name, m.tbl_name AS tbl_name, m.sql AS sql
FROM hacivat_attached.sqlite_schema m

--- name: TableFlags ---
--- returns: name text, type text, wr i64, strict i64 ---
--- joinable ---
SELECT t.name AS name, t.type AS type, t.wr AS wr, t.strict AS strict
FROM pragma_table_list t
WHERE t.schema = :schema

--- name: TableColumnsMain ---
--- returns: rel text, cid i64, name text, type text, notnull i64, dflt_value text, pk i64, hidden i64 ---
--- joinable ---
SELECT m.name AS rel, p.cid AS cid, p.name AS name, p.type AS type, p."notnull" AS "notnull",
  p.dflt_value AS dflt_value, p.pk AS pk, p.hidden AS hidden
FROM main.sqlite_schema m JOIN pragma_table_xinfo(m.name, :schema) p
WHERE m.type = 'table'

--- name: TableColumnsTemp ---
--- returns: rel text, cid i64, name text, type text, notnull i64, dflt_value text, pk i64, hidden i64 ---
--- joinable ---
SELECT m.name AS rel, p.cid AS cid, p.name AS name, p.type AS type, p."notnull" AS "notnull",
  p.dflt_value AS dflt_value, p.pk AS pk, p.hidden AS hidden
FROM temp.sqlite_schema m JOIN pragma_table_xinfo(m.name, :schema) p
WHERE m.type = 'table'

--- name: TableColumnsAttached ---
--- returns: rel text, cid i64, name text, type text, notnull i64, dflt_value text, pk i64, hidden i64 ---
--- joinable ---
SELECT m.name AS rel, p.cid AS cid, p.name AS name, p.type AS type, p."notnull" AS "notnull",
  p.dflt_value AS dflt_value, p.pk AS pk, p.hidden AS hidden
FROM hacivat_attached.sqlite_schema m JOIN pragma_table_xinfo(m.name, :schema) p
WHERE m.type = 'table'

--- name: ViewColumnsMain ---
--- returns: rel text, cid i64, name text, type text, notnull i64, dflt_value text, pk i64, hidden i64 ---
--- joinable ---
SELECT m.name AS rel, p.cid AS cid, p.name AS name, p.type AS type, p."notnull" AS "notnull",
  p.dflt_value AS dflt_value, p.pk AS pk, p.hidden AS hidden
FROM main.sqlite_schema m JOIN pragma_table_xinfo(m.name, :schema) p
WHERE m.type = 'view'

--- name: ViewColumnsTemp ---
--- returns: rel text, cid i64, name text, type text, notnull i64, dflt_value text, pk i64, hidden i64 ---
--- joinable ---
SELECT m.name AS rel, p.cid AS cid, p.name AS name, p.type AS type, p."notnull" AS "notnull",
  p.dflt_value AS dflt_value, p.pk AS pk, p.hidden AS hidden
FROM temp.sqlite_schema m JOIN pragma_table_xinfo(m.name, :schema) p
WHERE m.type = 'view'

--- name: ViewColumnsAttached ---
--- returns: rel text, cid i64, name text, type text, notnull i64, dflt_value text, pk i64, hidden i64 ---
--- joinable ---
SELECT m.name AS rel, p.cid AS cid, p.name AS name, p.type AS type, p."notnull" AS "notnull",
  p.dflt_value AS dflt_value, p.pk AS pk, p.hidden AS hidden
FROM hacivat_attached.sqlite_schema m JOIN pragma_table_xinfo(m.name, :schema) p
WHERE m.type = 'view'

--- name: RelationColumns ---
--- returns: cid i64, name text, type text, notnull i64, dflt_value text, pk i64, hidden i64 ---
--- joinable ---
SELECT p.cid AS cid, p.name AS name, p.type AS type, p."notnull" AS "notnull",
  p.dflt_value AS dflt_value, p.pk AS pk, p.hidden AS hidden
FROM pragma_table_xinfo(:rel, :schema) p

--- name: IndexesMain ---
--- returns: rel text, name text, unique i64, origin text, partial i64 ---
--- joinable ---
SELECT m.name AS rel, il.name AS name, il."unique" AS "unique", il.origin AS origin,
  il.partial AS partial
FROM main.sqlite_schema m JOIN pragma_index_list(m.name, :schema) il
WHERE m.type = 'table'

--- name: IndexesTemp ---
--- returns: rel text, name text, unique i64, origin text, partial i64 ---
--- joinable ---
SELECT m.name AS rel, il.name AS name, il."unique" AS "unique", il.origin AS origin,
  il.partial AS partial
FROM temp.sqlite_schema m JOIN pragma_index_list(m.name, :schema) il
WHERE m.type = 'table'

--- name: IndexesAttached ---
--- returns: rel text, name text, unique i64, origin text, partial i64 ---
--- joinable ---
SELECT m.name AS rel, il.name AS name, il."unique" AS "unique", il.origin AS origin,
  il.partial AS partial
FROM hacivat_attached.sqlite_schema m JOIN pragma_index_list(m.name, :schema) il
WHERE m.type = 'table'

--- name: IndexPartsMain ---
--- returns: idx text, seqno i64, cid i64, name text, desc i64, coll text, key i64 ---
--- joinable ---
SELECT il.name AS idx, ix.seqno AS seqno, ix.cid AS cid, ix.name AS name, ix."desc" AS "desc",
  ix.coll AS coll, ix."key" AS "key"
FROM main.sqlite_schema m JOIN pragma_index_list(m.name, :schema) il
  JOIN pragma_index_xinfo(il.name, :schema) ix
WHERE m.type = 'table'

--- name: IndexPartsTemp ---
--- returns: idx text, seqno i64, cid i64, name text, desc i64, coll text, key i64 ---
--- joinable ---
SELECT il.name AS idx, ix.seqno AS seqno, ix.cid AS cid, ix.name AS name, ix."desc" AS "desc",
  ix.coll AS coll, ix."key" AS "key"
FROM temp.sqlite_schema m JOIN pragma_index_list(m.name, :schema) il
  JOIN pragma_index_xinfo(il.name, :schema) ix
WHERE m.type = 'table'

--- name: IndexPartsAttached ---
--- returns: idx text, seqno i64, cid i64, name text, desc i64, coll text, key i64 ---
--- joinable ---
SELECT il.name AS idx, ix.seqno AS seqno, ix.cid AS cid, ix.name AS name, ix."desc" AS "desc",
  ix.coll AS coll, ix."key" AS "key"
FROM hacivat_attached.sqlite_schema m JOIN pragma_index_list(m.name, :schema) il
  JOIN pragma_index_xinfo(il.name, :schema) ix
WHERE m.type = 'table'

--- name: ForeignKeysMain ---
--- returns: rel text, id i64, seq i64, ref_table text, from_col text, to_col text, on_update text, on_delete text, match_kind text ---
--- joinable ---
SELECT m.name AS rel, fk.id AS id, fk.seq AS seq, fk."table" AS ref_table, fk."from" AS from_col,
  fk."to" AS to_col, fk.on_update AS on_update, fk.on_delete AS on_delete,
  fk."match" AS match_kind
FROM main.sqlite_schema m JOIN pragma_foreign_key_list(m.name, :schema) fk
WHERE m.type = 'table'

--- name: ForeignKeysTemp ---
--- returns: rel text, id i64, seq i64, ref_table text, from_col text, to_col text, on_update text, on_delete text, match_kind text ---
--- joinable ---
SELECT m.name AS rel, fk.id AS id, fk.seq AS seq, fk."table" AS ref_table, fk."from" AS from_col,
  fk."to" AS to_col, fk.on_update AS on_update, fk.on_delete AS on_delete,
  fk."match" AS match_kind
FROM temp.sqlite_schema m JOIN pragma_foreign_key_list(m.name, :schema) fk
WHERE m.type = 'table'

--- name: ForeignKeysAttached ---
--- returns: rel text, id i64, seq i64, ref_table text, from_col text, to_col text, on_update text, on_delete text, match_kind text ---
--- joinable ---
SELECT m.name AS rel, fk.id AS id, fk.seq AS seq, fk."table" AS ref_table, fk."from" AS from_col,
  fk."to" AS to_col, fk.on_update AS on_update, fk.on_delete AS on_delete,
  fk."match" AS match_kind
FROM hacivat_attached.sqlite_schema m JOIN pragma_foreign_key_list(m.name, :schema) fk
WHERE m.type = 'table'

--- name: FunctionList ---
--- returns: name text, builtin i64, type text, narg i64, flags i64 ---
--- joinable ---
SELECT f.name AS name, f.builtin AS builtin, f.type AS type, f.narg AS narg, f.flags AS flags
FROM pragma_function_list f

--- name: ModuleList ---
--- returns: name text ---
--- joinable ---
SELECT ml.name AS name
FROM pragma_module_list ml

--- name: CollationList ---
--- returns: seq i64, name text ---
--- joinable ---
SELECT c.seq AS seq, c.name AS name
FROM pragma_collation_list c
