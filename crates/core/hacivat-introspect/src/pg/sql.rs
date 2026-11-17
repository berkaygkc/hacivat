//! The catalog queries (HV-INTRO-04 sections, as Rust renderers for the spike).
//!
//! Every section starts with `/* hv:<name> */` so a query log names it. The only values
//! rendered into the text are numbers the introspector owns (object ids, the watermark, the
//! server version gates): never a name, never user text (HV-INTRO-04 edge 6). Catalog names
//! are schema-qualified with `pg_catalog`, so a hostile `search_path` cannot shadow them.
//! Name predicates never fold case (HV-INTRO-14): objects are selected by oid only.
//!
//! The M4 pack format (`*.hvq.sql` with `--# if version >= …` blocks and real binds through
//! the typed path) replaces these renderers; the SQL text carries over unchanged.

use super::model::Oid;

/// `server_version_num` (`170012`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PgVersion(pub u32);

impl PgVersion {
    pub fn major(self) -> u32 {
        self.0 / 10_000
    }
}

/// Which objects a retrieve section reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Filter<'a> {
    /// Every object of the namespace.
    Namespace(Oid),
    /// The objects (for child sections: of the parents) with these ids.
    Oids(&'a [Oid]),
}

impl Filter<'_> {
    /// The predicate on `alias`: its namespace column `ns_col`, or its `oid`.
    fn on(&self, alias: &str, ns_col: &str) -> String {
        match self {
            Filter::Namespace(ns) => format!("{alias}.{ns_col} = {ns}"),
            Filter::Oids(oids) => format!("{alias}.oid = ANY({})", oid_array(oids)),
        }
    }
}

/// `'{1,2,3}'::pg_catalog.oid[]`: digits and commas only.
pub fn oid_array(oids: &[Oid]) -> String {
    let mut s = String::with_capacity(oids.len() * 7 + 24);
    s.push_str("'{");
    for (i, o) in oids.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&o.to_string());
    }
    s.push_str("}'::pg_catalog.oid[]");
    s
}

/// The relkinds read as [`super::model::PgRelation`]s.
const REL_KINDS: &str = "('r','p','v','m','f','c')";

/// The user types listed as [`super::model::PgType`]s: not array types, not the row types of
/// tables, not shell or pseudo types (multiranges, created with their range, are left out).
const TYPE_KINDS: &str = "t.typtype IN ('b','c','d','e','r') \
     AND NOT EXISTS (SELECT 1 FROM pg_catalog.pg_type e WHERE e.typarray = t.oid) \
     AND (t.typtype <> 'c' OR EXISTS (SELECT 1 FROM pg_catalog.pg_class r \
     WHERE r.oid = t.typrelid AND r.relkind = 'c'))";

/// The probe that opens every pass: version, recovery, identity, the snapshot watermark and
/// the namespace's existence. Run first in the transaction, so the snapshot it reports is the
/// one every later section reads with.
pub fn probe(ns: Oid) -> String {
    format!(
        "/* hv:pg/Identity */ SELECT pg_catalog.current_setting('server_version_num'), \
         pg_catalog.pg_is_in_recovery(), \
         extract(epoch FROM pg_catalog.pg_postmaster_start_time())::text, \
         (SELECT d.oid FROM pg_catalog.pg_database d WHERE d.datname = pg_catalog.current_database()), \
         CASE WHEN pg_catalog.has_function_privilege('pg_catalog.pg_control_system()', 'EXECUTE') \
         THEN (SELECT s.system_identifier::text FROM pg_catalog.pg_control_system() s) END, \
         CASE WHEN pg_catalog.has_function_privilege('pg_catalog.pg_control_checkpoint()', 'EXECUTE') \
         THEN (SELECT c.timeline_id::text FROM pg_catalog.pg_control_checkpoint() c) END, \
         pg_catalog.pg_snapshot_xmin(pg_catalog.pg_current_snapshot())::text, \
         pg_catalog.pg_snapshot_xmax(pg_catalog.pg_current_snapshot())::text, \
         EXISTS (SELECT 1 FROM pg_catalog.pg_namespace n WHERE n.oid = {ns})"
    )
}

/// The no-write canary (ver/01 #pg-canary): true while this transaction has no XID.
pub const CANARY: &str =
    "/* hv:pg/Canary */ SELECT pg_catalog.pg_current_xact_id_if_assigned() IS NULL";

/// Every schema of the database.
pub const SCHEMAS: &str = "/* hv:pg/ListNamespaces */ SELECT n.oid, n.nspname, \
     pg_catalog.pg_get_userbyid(n.nspowner), d.description \
     FROM pg_catalog.pg_namespace n \
     LEFT JOIN pg_catalog.pg_description d ON d.classoid = 'pg_catalog.pg_namespace'::pg_catalog.regclass \
     AND d.objoid = n.oid AND d.objsubid = 0";

/// One schema (same columns as [`SCHEMAS`]).
pub fn schema(ns: Oid) -> String {
    format!("{SCHEMAS} WHERE n.oid = {ns}").replacen(
        "hv:pg/ListNamespaces",
        "hv:pg/RetrieveNamespace",
        1,
    )
}

/// Relations (`'r','p','v','m','f','c'`) with their own properties.
pub fn relations(f: &Filter<'_>, sources: bool) -> String {
    let view = if sources {
        "CASE WHEN c.relkind IN ('v','m') THEN pg_catalog.pg_get_viewdef(c.oid, true) END"
    } else {
        "NULL"
    };
    format!(
        "/* hv:pg/RetrieveRelations */ SELECT c.oid, c.relname, c.relkind, \
         pg_catalog.pg_get_userbyid(c.relowner), c.relpersistence, d.description, \
         ARRAY(SELECT i.inhparent FROM pg_catalog.pg_inherits i WHERE i.inhrelid = c.oid \
         ORDER BY i.inhseqno)::text, \
         CASE WHEN c.relispartition THEN pg_catalog.pg_get_expr(c.relpartbound, c.oid) END, \
         CASE WHEN c.relkind = 'p' THEN pg_catalog.pg_get_partkeydef(c.oid) END, \
         {view}, c.relispopulated, c.reloptions::text \
         FROM pg_catalog.pg_class c \
         LEFT JOIN pg_catalog.pg_description d ON d.classoid = 'pg_catalog.pg_class'::pg_catalog.regclass \
         AND d.objoid = c.oid AND d.objsubid = 0 \
         WHERE {} AND c.relkind IN {REL_KINDS}",
        f.on("c", "relnamespace")
    )
}

/// Columns of the filtered relations.
pub fn columns(f: &Filter<'_>) -> String {
    format!(
        "/* hv:pg/RetrieveColumns */ SELECT a.attrelid, a.attnum, a.attname, a.atttypid, \
         pg_catalog.format_type(a.atttypid, a.atttypmod), tn.nspname, t.typname, t.typtype, \
         CASE WHEN t.typlen = -1 AND t.typelem <> 0 THEN t.typelem END, \
         CASE WHEN t.typtype = 'd' THEN t.typbasetype END, \
         a.attndims, a.attnotnull, pg_catalog.pg_get_expr(ad.adbin, ad.adrelid), \
         a.attidentity, a.attgenerated, \
         CASE WHEN a.attcollation <> t.typcollation THEN co.collname END, \
         a.attinhcount, d.description \
         FROM pg_catalog.pg_attribute a \
         JOIN pg_catalog.pg_class c ON c.oid = a.attrelid \
         JOIN pg_catalog.pg_type t ON t.oid = a.atttypid \
         JOIN pg_catalog.pg_namespace tn ON tn.oid = t.typnamespace \
         LEFT JOIN pg_catalog.pg_attrdef ad ON ad.adrelid = a.attrelid AND ad.adnum = a.attnum \
         LEFT JOIN pg_catalog.pg_collation co ON co.oid = a.attcollation \
         LEFT JOIN pg_catalog.pg_description d ON d.classoid = 'pg_catalog.pg_class'::pg_catalog.regclass \
         AND d.objoid = a.attrelid AND d.objsubid = a.attnum \
         WHERE {} AND c.relkind IN {REL_KINDS} AND a.attnum > 0 AND NOT a.attisdropped",
        f.on("c", "relnamespace")
    )
}

/// Indexes of the filtered relations: one row per index, keys as ordered arrays
/// (HV-INTRO-13); expression keys deparsed in the same row.
pub fn indexes(f: &Filter<'_>, v: PgVersion, sources: bool) -> String {
    let nnd = if v.major() >= 15 {
        "i.indnullsnotdistinct"
    } else {
        "false"
    };
    let def = if sources {
        "pg_catalog.pg_get_indexdef(i.indexrelid)"
    } else {
        "NULL"
    };
    format!(
        "/* hv:pg/RetrieveIndexes */ SELECT i.indrelid, i.indexrelid, ic.relname, am.amname, \
         i.indisunique, i.indisprimary, i.indisexclusion, i.indimmediate, i.indisvalid, {nnd}, \
         i.indnkeyatts, i.indkey::text, i.indoption::text, \
         ARRAY(SELECT CASE WHEN i.indkey[k] = 0 \
         THEN pg_catalog.pg_get_indexdef(i.indexrelid, k + 1, true) END \
         FROM pg_catalog.generate_series(0, i.indnatts - 1) AS k ORDER BY k)::text, \
         pg_catalog.pg_get_expr(i.indpred, i.indrelid), {def}, d.description \
         FROM pg_catalog.pg_index i \
         JOIN pg_catalog.pg_class c ON c.oid = i.indrelid \
         JOIN pg_catalog.pg_class ic ON ic.oid = i.indexrelid \
         JOIN pg_catalog.pg_am am ON am.oid = ic.relam \
         LEFT JOIN pg_catalog.pg_description d ON d.classoid = 'pg_catalog.pg_class'::pg_catalog.regclass \
         AND d.objoid = i.indexrelid AND d.objsubid = 0 \
         WHERE {} AND c.relkind IN {REL_KINDS}",
        f.on("c", "relnamespace")
    )
}

/// Table constraints of the filtered relations.
pub fn constraints(f: &Filter<'_>, v: PgVersion) -> String {
    let enforced = if v.major() >= 18 {
        "k.conenforced"
    } else {
        "true"
    };
    format!(
        "/* hv:pg/RetrieveConstraints */ SELECT k.conrelid, k.oid, k.conname, k.contype, \
         k.condeferrable, k.condeferred, k.convalidated, {enforced}, k.conislocal, \
         k.connoinherit, k.conkey::text, NULLIF(k.conindid, 0), \
         NULLIF(k.confrelid, 0), fn.nspname, fc.relname, k.confkey::text, k.confupdtype, \
         k.confdeltype, k.confmatchtype, pg_catalog.pg_get_constraintdef(k.oid, true), \
         d.description \
         FROM pg_catalog.pg_constraint k \
         JOIN pg_catalog.pg_class c ON c.oid = k.conrelid \
         LEFT JOIN pg_catalog.pg_class fc ON fc.oid = k.confrelid \
         LEFT JOIN pg_catalog.pg_namespace fn ON fn.oid = fc.relnamespace \
         LEFT JOIN pg_catalog.pg_description d ON d.classoid = 'pg_catalog.pg_constraint'::pg_catalog.regclass \
         AND d.objoid = k.oid AND d.objsubid = 0 \
         WHERE {} AND c.relkind IN {REL_KINDS}",
        f.on("c", "relnamespace")
    )
}

/// User triggers of the filtered relations.
pub fn triggers(f: &Filter<'_>) -> String {
    format!(
        "/* hv:pg/RetrieveTriggers */ SELECT t.tgrelid, t.oid, t.tgname, t.tgtype, t.tgenabled, \
         pn.nspname, p.proname, pg_catalog.pg_get_triggerdef(t.oid, true), \
         t.tgconstraint <> 0, d.description \
         FROM pg_catalog.pg_trigger t \
         JOIN pg_catalog.pg_class c ON c.oid = t.tgrelid \
         JOIN pg_catalog.pg_proc p ON p.oid = t.tgfoid \
         JOIN pg_catalog.pg_namespace pn ON pn.oid = p.pronamespace \
         LEFT JOIN pg_catalog.pg_description d ON d.classoid = 'pg_catalog.pg_trigger'::pg_catalog.regclass \
         AND d.objoid = t.oid AND d.objsubid = 0 \
         WHERE {} AND c.relkind IN {REL_KINDS} AND NOT t.tgisinternal",
        f.on("c", "relnamespace")
    )
}

/// A relation's own and its columns' comments (the incremental comments facet).
pub fn rel_comments(oids: &[Oid]) -> String {
    format!(
        "/* hv:pg/Comments */ SELECT d.objoid, d.objsubid, d.description \
         FROM pg_catalog.pg_description d \
         WHERE d.classoid = 'pg_catalog.pg_class'::pg_catalog.regclass AND d.objoid = ANY({})",
        oid_array(oids)
    )
}

/// Sequences.
pub fn sequences(f: &Filter<'_>) -> String {
    format!(
        "/* hv:pg/RetrieveSequences */ SELECT c.oid, c.relname, pg_catalog.pg_get_userbyid(c.relowner), \
         pg_catalog.format_type(s.seqtypid, NULL), s.seqstart, s.seqincrement, s.seqmin, \
         s.seqmax, s.seqcache, s.seqcycle, dep.refobjid, dep.refobjsubid, dep.deptype, \
         d.description \
         FROM pg_catalog.pg_class c \
         JOIN pg_catalog.pg_sequence s ON s.seqrelid = c.oid \
         LEFT JOIN pg_catalog.pg_depend dep ON dep.classid = 'pg_catalog.pg_class'::pg_catalog.regclass \
         AND dep.objid = c.oid AND dep.refclassid = 'pg_catalog.pg_class'::pg_catalog.regclass \
         AND dep.deptype IN ('a','i') \
         LEFT JOIN pg_catalog.pg_description d ON d.classoid = 'pg_catalog.pg_class'::pg_catalog.regclass \
         AND d.objoid = c.oid AND d.objsubid = 0 \
         WHERE {} AND c.relkind = 'S'",
        f.on("c", "relnamespace")
    )
}

/// Routines.
pub fn routines(f: &Filter<'_>, sources: bool) -> String {
    let def = if sources {
        "CASE WHEN p.prokind IN ('f','p') THEN pg_catalog.pg_get_functiondef(p.oid) END"
    } else {
        "NULL"
    };
    format!(
        "/* hv:pg/RetrieveRoutines */ SELECT p.oid, p.proname, p.prokind, \
         pg_catalog.pg_get_function_identity_arguments(p.oid), \
         CASE WHEN p.prokind <> 'p' THEN pg_catalog.pg_get_function_result(p.oid) END, \
         l.lanname, p.provolatile, p.proisstrict, p.prosecdef, p.proretset, p.proparallel, \
         pg_catalog.pg_get_userbyid(p.proowner), \
         EXISTS (SELECT 1 FROM pg_catalog.pg_depend dep \
         WHERE dep.classid = 'pg_catalog.pg_proc'::pg_catalog.regclass AND dep.objid = p.oid \
         AND dep.deptype = 'e'), \
         {def}, d.description \
         FROM pg_catalog.pg_proc p \
         JOIN pg_catalog.pg_language l ON l.oid = p.prolang \
         LEFT JOIN pg_catalog.pg_description d ON d.classoid = 'pg_catalog.pg_proc'::pg_catalog.regclass \
         AND d.objoid = p.oid AND d.objsubid = 0 \
         WHERE {}",
        f.on("p", "pronamespace")
    )
}

/// User types.
pub fn types(f: &Filter<'_>) -> String {
    format!(
        "/* hv:pg/RetrieveTypes */ SELECT t.oid, t.typname, t.typtype, \
         pg_catalog.pg_get_userbyid(t.typowner), d.description, \
         CASE WHEN t.typtype = 'd' THEN t.typbasetype END, \
         CASE WHEN t.typtype = 'd' THEN pg_catalog.format_type(t.typbasetype, t.typtypmod) END, \
         t.typnotnull, t.typdefault, t.typrelid, \
         CASE WHEN t.typtype = 'e' THEN ARRAY(SELECT e.enumlabel FROM pg_catalog.pg_enum e \
         WHERE e.enumtypid = t.oid ORDER BY e.enumsortorder)::text END, \
         CASE WHEN t.typtype = 'r' THEN pg_catalog.format_type(r.rngsubtype, NULL) END \
         FROM pg_catalog.pg_type t \
         LEFT JOIN pg_catalog.pg_range r ON r.rngtypid = t.oid \
         LEFT JOIN pg_catalog.pg_description d ON d.classoid = 'pg_catalog.pg_type'::pg_catalog.regclass \
         AND d.objoid = t.oid AND d.objsubid = 0 \
         WHERE {} AND {TYPE_KINDS}",
        f.on("t", "typnamespace")
    )
}

/// Domain constraints of the filtered types.
pub fn domain_constraints(f: &Filter<'_>) -> String {
    format!(
        "/* hv:pg/RetrieveDomainConstraints */ SELECT k.contypid, k.oid, k.conname, k.contype, \
         pg_catalog.pg_get_constraintdef(k.oid, true), k.convalidated \
         FROM pg_catalog.pg_constraint k \
         JOIN pg_catalog.pg_type t ON t.oid = k.contypid \
         WHERE {} AND t.typtype = 'd'",
        f.on("t", "typnamespace")
    )
}

/// The existence query (HV-INTRO-05 step 4a): every object id of the namespace by kind tag
/// (`r` relation, `S` sequence, `x` child: index, constraint, trigger; `p` routine, `y` type).
pub fn alive(ns: Oid) -> String {
    format!(
        "/* hv:pg/ListExistent */ \
         SELECT 'r', c.oid FROM pg_catalog.pg_class c WHERE c.relnamespace = {ns} AND c.relkind IN {REL_KINDS} \
         UNION ALL SELECT 'S', c.oid FROM pg_catalog.pg_class c WHERE c.relnamespace = {ns} AND c.relkind = 'S' \
         UNION ALL SELECT 'x', i.indexrelid FROM pg_catalog.pg_index i \
         JOIN pg_catalog.pg_class c ON c.oid = i.indrelid WHERE c.relnamespace = {ns} \
         UNION ALL SELECT 'x', k.oid FROM pg_catalog.pg_constraint k WHERE k.connamespace = {ns} \
         UNION ALL SELECT 'x', t.oid FROM pg_catalog.pg_trigger t \
         JOIN pg_catalog.pg_class c ON c.oid = t.tgrelid WHERE c.relnamespace = {ns} AND NOT t.tgisinternal \
         UNION ALL SELECT 'p', p.oid FROM pg_catalog.pg_proc p WHERE p.pronamespace = {ns} \
         UNION ALL SELECT 'y', t.oid FROM pg_catalog.pg_type t WHERE t.typnamespace = {ns} AND {TYPE_KINDS}"
    )
}

/// The comment keys of the namespace (`pg_description` rows of its objects). A `COMMENT …
/// IS NULL` deletes the row and leaves no young xmin behind, so a comment that disappeared is
/// found only by comparing these keys with the previous snapshot's comments.
///
/// Tag: `c` pg_class (relation, column by `objsubid`, index, sequence), `k` constraint,
/// `t` trigger, `p` routine, `y` type, `n` the schema.
pub fn alive_comments(ns: Oid) -> String {
    format!(
        "/* hv:pg/ListExistentComments */ SELECT CASE d.classoid \
         WHEN 'pg_catalog.pg_class'::pg_catalog.regclass THEN 'c' \
         WHEN 'pg_catalog.pg_constraint'::pg_catalog.regclass THEN 'k' \
         WHEN 'pg_catalog.pg_trigger'::pg_catalog.regclass THEN 't' \
         WHEN 'pg_catalog.pg_proc'::pg_catalog.regclass THEN 'p' \
         WHEN 'pg_catalog.pg_type'::pg_catalog.regclass THEN 'y' \
         ELSE 'n' END, d.objoid, d.objsubid \
         FROM pg_catalog.pg_description d WHERE \
         (d.classoid = 'pg_catalog.pg_class'::pg_catalog.regclass AND d.objoid IN \
         (SELECT c.oid FROM pg_catalog.pg_class c WHERE c.relnamespace = {ns})) \
         OR (d.classoid = 'pg_catalog.pg_constraint'::pg_catalog.regclass AND d.objoid IN \
         (SELECT k.oid FROM pg_catalog.pg_constraint k WHERE k.connamespace = {ns})) \
         OR (d.classoid = 'pg_catalog.pg_trigger'::pg_catalog.regclass AND d.objoid IN \
         (SELECT t.oid FROM pg_catalog.pg_trigger t JOIN pg_catalog.pg_class c ON c.oid = t.tgrelid \
         WHERE c.relnamespace = {ns})) \
         OR (d.classoid = 'pg_catalog.pg_proc'::pg_catalog.regclass AND d.objoid IN \
         (SELECT p.oid FROM pg_catalog.pg_proc p WHERE p.pronamespace = {ns})) \
         OR (d.classoid = 'pg_catalog.pg_type'::pg_catalog.regclass AND d.objoid IN \
         (SELECT t.oid FROM pg_catalog.pg_type t WHERE t.typnamespace = {ns})) \
         OR (d.classoid = 'pg_catalog.pg_namespace'::pg_catalog.regclass AND d.objoid = {ns})"
    )
}

/// `alias`'s row was written at or after `since`.
fn young(alias: &str, since: u32) -> String {
    format!("pg_catalog.age({alias}.xmin) BETWEEN 0 AND pg_catalog.age('{since}'::pg_catalog.xid)")
}

/// The change query (HV-INTRO-05 step 4b): `(facet, owner oid)` for every catalog row of the
/// namespace written by a transaction at or after the watermark `since` (the previous pass's
/// snapshot xmin, low 32 bits), by the verified per-DDL matrix (ver/01 #pg-xmin).
///
/// The predicate is `age(xmin) BETWEEN 0 AND age(since)`. Both ages count from the same
/// reference XID (`age()` is stable within a transaction), so the bound is exact: no margin
/// for XIDs consumed between the probe and this query is needed, and none can be missed on a
/// busy server (HV-INTRO-05's client-side `txage + 64` could). A frozen row older
/// than 2^31 transactions has a negative age and is not re-read (#pg-xmin-freeze); bootstrap
/// rows (`age = INT_MAX`) never match.
///
/// Facets: `rel` (the relation's own row, its view rule, inheritance, partition key), `columns`
/// (`pg_attribute`, `pg_attrdef`), `indexes` (`pg_index`, the index's `pg_class` row and
/// comment), `constraints`, `triggers` (rows and comments), `comments` (relation and column
/// comments), `sequence`, `routine`, `type` (`pg_type`, `pg_enum`, domain constraints,
/// `pg_range`, comments) and `schema`.
pub fn changed(ns: Oid, since: u32) -> String {
    let y = |a: &str| young(a, since);
    let class = "'pg_catalog.pg_class'::pg_catalog.regclass";
    format!(
        "/* hv:pg/ChangedParents */ WITH rels AS MATERIALIZED (SELECT c.oid FROM pg_catalog.pg_class c \
         WHERE c.relnamespace = {ns} AND c.relkind IN {REL_KINDS}) \
         SELECT DISTINCT f, o FROM ( \
         SELECT 'rel' AS f, c.oid AS o FROM pg_catalog.pg_class c \
         WHERE c.relnamespace = {ns} AND c.relkind IN {REL_KINDS} AND {yc} \
         UNION ALL SELECT 'rel', r.ev_class FROM pg_catalog.pg_rewrite r JOIN rels ON rels.oid = r.ev_class WHERE {yr} \
         UNION ALL SELECT 'rel', i.inhrelid FROM pg_catalog.pg_inherits i JOIN rels ON rels.oid = i.inhrelid WHERE {yi} \
         UNION ALL SELECT 'rel', p.partrelid FROM pg_catalog.pg_partitioned_table p JOIN rels ON rels.oid = p.partrelid WHERE {yp} \
         UNION ALL SELECT 'columns', a.attrelid FROM pg_catalog.pg_attribute a JOIN rels ON rels.oid = a.attrelid WHERE {ya} \
         UNION ALL SELECT 'columns', ad.adrelid FROM pg_catalog.pg_attrdef ad JOIN rels ON rels.oid = ad.adrelid WHERE {yad} \
         UNION ALL SELECT 'indexes', i.indrelid FROM pg_catalog.pg_index i JOIN rels ON rels.oid = i.indrelid WHERE {yi} \
         UNION ALL SELECT 'indexes', i.indrelid FROM pg_catalog.pg_index i JOIN rels ON rels.oid = i.indrelid \
         JOIN pg_catalog.pg_class ic ON ic.oid = i.indexrelid WHERE {yic} \
         UNION ALL SELECT 'constraints', k.conrelid FROM pg_catalog.pg_constraint k JOIN rels ON rels.oid = k.conrelid WHERE {yk} \
         UNION ALL SELECT 'triggers', t.tgrelid FROM pg_catalog.pg_trigger t JOIN rels ON rels.oid = t.tgrelid \
         WHERE NOT t.tgisinternal AND {yt} \
         UNION ALL SELECT 'comments', d.objoid FROM pg_catalog.pg_description d JOIN rels ON rels.oid = d.objoid \
         WHERE d.classoid = {class} AND {yd} \
         UNION ALL SELECT 'indexes', i.indrelid FROM pg_catalog.pg_description d \
         JOIN pg_catalog.pg_index i ON i.indexrelid = d.objoid JOIN rels ON rels.oid = i.indrelid \
         WHERE d.classoid = {class} AND {yd} \
         UNION ALL SELECT 'constraints', k.conrelid FROM pg_catalog.pg_description d \
         JOIN pg_catalog.pg_constraint k ON k.oid = d.objoid JOIN rels ON rels.oid = k.conrelid \
         WHERE d.classoid = 'pg_catalog.pg_constraint'::pg_catalog.regclass AND {yd} \
         UNION ALL SELECT 'triggers', t.tgrelid FROM pg_catalog.pg_description d \
         JOIN pg_catalog.pg_trigger t ON t.oid = d.objoid JOIN rels ON rels.oid = t.tgrelid \
         WHERE d.classoid = 'pg_catalog.pg_trigger'::pg_catalog.regclass AND {yd} \
         UNION ALL SELECT 'sequence', c.oid FROM pg_catalog.pg_class c \
         WHERE c.relnamespace = {ns} AND c.relkind = 'S' AND {yc} \
         UNION ALL SELECT 'sequence', s.seqrelid FROM pg_catalog.pg_sequence s \
         JOIN pg_catalog.pg_class c ON c.oid = s.seqrelid WHERE c.relnamespace = {ns} AND {ys} \
         UNION ALL SELECT 'sequence', c.oid FROM pg_catalog.pg_description d \
         JOIN pg_catalog.pg_class c ON c.oid = d.objoid \
         WHERE d.classoid = {class} AND c.relnamespace = {ns} AND c.relkind = 'S' AND {yd} \
         UNION ALL SELECT 'sequence', c.oid FROM pg_catalog.pg_depend dep \
         JOIN pg_catalog.pg_class c ON c.oid = dep.objid \
         WHERE dep.classid = {class} AND c.relnamespace = {ns} AND c.relkind = 'S' AND {ydep} \
         UNION ALL SELECT 'routine', p.oid FROM pg_catalog.pg_proc p WHERE p.pronamespace = {ns} AND {yp} \
         UNION ALL SELECT 'routine', p.oid FROM pg_catalog.pg_description d \
         JOIN pg_catalog.pg_proc p ON p.oid = d.objoid \
         WHERE d.classoid = 'pg_catalog.pg_proc'::pg_catalog.regclass AND p.pronamespace = {ns} AND {yd} \
         UNION ALL SELECT 'type', t.oid FROM pg_catalog.pg_type t WHERE t.typnamespace = {ns} AND {yty} \
         UNION ALL SELECT 'type', e.enumtypid FROM pg_catalog.pg_enum e \
         JOIN pg_catalog.pg_type t ON t.oid = e.enumtypid WHERE t.typnamespace = {ns} AND {ye} \
         UNION ALL SELECT 'type', k.contypid FROM pg_catalog.pg_constraint k \
         JOIN pg_catalog.pg_type t ON t.oid = k.contypid WHERE t.typnamespace = {ns} AND {yk} \
         UNION ALL SELECT 'type', r.rngtypid FROM pg_catalog.pg_range r \
         JOIN pg_catalog.pg_type t ON t.oid = r.rngtypid WHERE t.typnamespace = {ns} AND {yrg} \
         UNION ALL SELECT 'type', t.oid FROM pg_catalog.pg_description d \
         JOIN pg_catalog.pg_type t ON t.oid = d.objoid \
         WHERE d.classoid = 'pg_catalog.pg_type'::pg_catalog.regclass AND t.typnamespace = {ns} AND {yd} \
         UNION ALL SELECT 'schema', n.oid FROM pg_catalog.pg_namespace n WHERE n.oid = {ns} AND {yn} \
         UNION ALL SELECT 'schema', d.objoid FROM pg_catalog.pg_description d \
         WHERE d.classoid = 'pg_catalog.pg_namespace'::pg_catalog.regclass AND d.objoid = {ns} AND {yd} \
         ) AS young",
        yc = y("c"),
        yr = y("r"),
        yi = y("i"),
        yp = y("p"),
        ya = y("a"),
        yad = y("ad"),
        yic = y("ic"),
        yk = y("k"),
        yt = y("t"),
        yd = y("d"),
        ys = y("s"),
        ydep = y("dep"),
        yty = y("t"),
        ye = y("e"),
        yrg = y("r"),
        yn = y("n"),
    )
}

/// The dependency half of the change query: deparsed text (`format_type`, `pg_get_*def`,
/// `pg_get_expr`, a foreign key's referenced table name) embeds the names of *other* objects,
/// whose renames leave the dependent's own catalog rows untouched. So every young `pg_class`,
/// `pg_attribute`, `pg_type` or `pg_proc` row of the **database** (any namespace) is a source,
/// and the namespace's objects that depend on a source (`pg_depend`, one level) are re-read:
/// a renamed type re-reads the columns that use it, a renamed table the views and foreign
/// keys that reference it, a renamed function the triggers and defaults that call it.
pub fn changed_deps(ns: Oid, since: u32) -> String {
    let y = |a: &str| young(a, since);
    let class = "'pg_catalog.pg_class'::pg_catalog.regclass";
    format!(
        "/* hv:pg/ChangedDependents */ WITH src AS MATERIALIZED ( \
         SELECT {class} AS cls, c.oid FROM pg_catalog.pg_class c WHERE {yc} \
         UNION SELECT {class}, a.attrelid FROM pg_catalog.pg_attribute a WHERE {ya} \
         UNION SELECT 'pg_catalog.pg_type'::pg_catalog.regclass, t.oid FROM pg_catalog.pg_type t WHERE {yt} \
         UNION SELECT 'pg_catalog.pg_proc'::pg_catalog.regclass, p.oid FROM pg_catalog.pg_proc p WHERE {yp} \
         ), dep AS MATERIALIZED (SELECT DISTINCT d.classid, d.objid, d.objsubid \
         FROM pg_catalog.pg_depend d JOIN src ON d.refclassid = src.cls AND d.refobjid = src.oid \
         WHERE d.deptype NOT IN ('e','x','p')) \
         SELECT DISTINCT f, o FROM ( \
         SELECT CASE WHEN c.relkind IN ('i','I') THEN 'indexes' WHEN c.relkind = 'S' THEN 'sequence' \
         WHEN dep.objsubid <> 0 THEN 'columns' ELSE 'rel' END AS f, \
         CASE WHEN c.relkind IN ('i','I') THEN (SELECT i.indrelid FROM pg_catalog.pg_index i \
         WHERE i.indexrelid = c.oid) ELSE c.oid END AS o \
         FROM dep JOIN pg_catalog.pg_class c ON dep.classid = {class} AND c.oid = dep.objid \
         WHERE c.relnamespace = {ns} AND c.relkind IN ('r','p','v','m','f','c','i','I','S') \
         UNION ALL SELECT 'rel', r.ev_class FROM dep \
         JOIN pg_catalog.pg_rewrite r ON dep.classid = 'pg_catalog.pg_rewrite'::pg_catalog.regclass AND r.oid = dep.objid \
         JOIN pg_catalog.pg_class c ON c.oid = r.ev_class WHERE c.relnamespace = {ns} \
         UNION ALL SELECT CASE WHEN k.conrelid <> 0 THEN 'constraints' ELSE 'type' END, \
         CASE WHEN k.conrelid <> 0 THEN k.conrelid ELSE k.contypid END FROM dep \
         JOIN pg_catalog.pg_constraint k ON dep.classid = 'pg_catalog.pg_constraint'::pg_catalog.regclass AND k.oid = dep.objid \
         WHERE k.connamespace = {ns} \
         UNION ALL SELECT 'triggers', t.tgrelid FROM dep \
         JOIN pg_catalog.pg_trigger t ON dep.classid = 'pg_catalog.pg_trigger'::pg_catalog.regclass AND t.oid = dep.objid \
         JOIN pg_catalog.pg_class c ON c.oid = t.tgrelid WHERE c.relnamespace = {ns} \
         UNION ALL SELECT 'columns', ad.adrelid FROM dep \
         JOIN pg_catalog.pg_attrdef ad ON dep.classid = 'pg_catalog.pg_attrdef'::pg_catalog.regclass AND ad.oid = dep.objid \
         JOIN pg_catalog.pg_class c ON c.oid = ad.adrelid WHERE c.relnamespace = {ns} \
         UNION ALL SELECT 'type', t.oid FROM dep \
         JOIN pg_catalog.pg_type t ON dep.classid = 'pg_catalog.pg_type'::pg_catalog.regclass AND t.oid = dep.objid \
         WHERE t.typnamespace = {ns} \
         UNION ALL SELECT 'routine', p.oid FROM dep \
         JOIN pg_catalog.pg_proc p ON dep.classid = 'pg_catalog.pg_proc'::pg_catalog.regclass AND p.oid = dep.objid \
         WHERE p.pronamespace = {ns} \
         ) AS deps WHERE o IS NOT NULL",
        yc = y("c"),
        ya = y("a"),
        yt = y("t"),
        yp = y("p"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oid_arrays_hold_digits_and_commas_only() {
        assert_eq!(oid_array(&[]), "'{}'::pg_catalog.oid[]");
        assert_eq!(oid_array(&[16384, 7]), "'{16384,7}'::pg_catalog.oid[]");
    }

    #[test]
    fn filters_select_by_namespace_or_oid() {
        assert!(relations(&Filter::Namespace(2200), false).contains("c.relnamespace = 2200"));
        let oids = [1, 2];
        let q = columns(&Filter::Oids(&oids));
        assert!(q.contains("c.oid = ANY('{1,2}'::pg_catalog.oid[])"), "{q}");
        assert!(routines(&Filter::Namespace(9), false).contains("p.pronamespace = 9"));
        assert!(types(&Filter::Namespace(9)).contains("t.typnamespace = 9"));
    }

    #[test]
    fn version_gates() {
        let f = Filter::Namespace(1);
        assert!(!indexes(&f, PgVersion(130_023), false).contains("indnullsnotdistinct"));
        assert!(indexes(&f, PgVersion(150_000), false).contains("i.indnullsnotdistinct"));
        assert!(!constraints(&f, PgVersion(170_012),).contains("conenforced"));
        assert!(constraints(&f, PgVersion(180_007)).contains("k.conenforced"));
    }

    #[test]
    fn sources_level_adds_definitions() {
        let f = Filter::Namespace(1);
        assert!(!relations(&f, false).contains("pg_get_viewdef"));
        assert!(relations(&f, true).contains("pg_get_viewdef"));
        assert!(!routines(&f, false).contains("pg_get_functiondef"));
        assert!(routines(&f, true).contains("pg_get_functiondef"));
        assert!(!indexes(&f, PgVersion(170_000), false).contains("pg_get_indexdef(i.indexrelid)"));
    }

    #[test]
    fn change_query_uses_the_between_predicate_and_never_assigns_an_xid() {
        let q = changed(2200, 1234);
        assert!(
            q.contains(
                "pg_catalog.age(c.xmin) BETWEEN 0 AND pg_catalog.age('1234'::pg_catalog.xid)"
            ),
            "{q}"
        );
        assert!(
            !q.contains("<="),
            "the BETWEEN fix (ver/01 #pg-xmin-freeze)"
        );
        let d = changed_deps(2200, 1234);
        assert!(
            d.contains("pg_catalog.age(a.xmin) BETWEEN 0 AND pg_catalog.age('1234'"),
            "{d}"
        );
        for q in [q, d, probe(2200), alive(2200), alive_comments(2200)] {
            assert!(!q.contains("txid_current()"), "{q}");
            assert!(!q.contains("pg_current_xact_id()"), "{q}");
        }
    }

    #[test]
    fn every_section_names_itself() {
        let f = Filter::Namespace(1);
        let v = PgVersion(170_000);
        for (q, name) in [
            (probe(1), "pg/Identity"),
            (schema(1), "pg/RetrieveNamespace"),
            (relations(&f, true), "pg/RetrieveRelations"),
            (columns(&f), "pg/RetrieveColumns"),
            (indexes(&f, v, true), "pg/RetrieveIndexes"),
            (constraints(&f, v), "pg/RetrieveConstraints"),
            (triggers(&f), "pg/RetrieveTriggers"),
            (sequences(&f), "pg/RetrieveSequences"),
            (routines(&f, true), "pg/RetrieveRoutines"),
            (types(&f), "pg/RetrieveTypes"),
            (domain_constraints(&f), "pg/RetrieveDomainConstraints"),
            (alive(1), "pg/ListExistent"),
            (alive_comments(1), "pg/ListExistentComments"),
            (changed(1, 1), "pg/ChangedParents"),
            (changed_deps(1, 1), "pg/ChangedDependents"),
            (rel_comments(&[1]), "pg/Comments"),
        ] {
            assert!(q.starts_with(&format!("/* hv:{name} */ ")), "{q}");
        }
    }
}
