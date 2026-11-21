//! Coverage of the generated PostgreSQL visitor and of the statement table (M3b Task 9;
//! R-M3-2, CVE-2026-85620 lesson): over a Hacivat-authored corpus of every statement family
//! and expression form, an independent walk of each parse tree (its JSON, through serde)
//! counts the `Node` kinds, and the generated walk must visit exactly as many of each; no
//! kind seen may fall through to the classifier's `Unhandled` role. Also: the statement table
//! names every statement message, the generated file matches the vendored schema, and the
//! classifier never uses `pg_query`'s own walker (R9).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde_json::Value;
use sql_dialect::{ClientFlavor, DialectCtx, DialectId, ServerVersion};
use sql_oracle_pg::tree::parse_tree;
use sql_safety::pg_backend_hooks as hooks;
use sql_safety::{ClassifyCtx, CoarseClass, Expect};
use twox_hash::XxHash3_64;

/// Hacivat-authored statements, one family per block. Every one parses with the vendored
/// PostgreSQL 18 grammar.
const CORPUS: &[&str] = &[
    // Reads: joins, subqueries, CTEs, windows, grouping
    "SELECT a, b AS c, t.* FROM s.t AS t(a, b) INNER JOIN u USING (a) LEFT JOIN v ON v.a = t.a CROSS JOIN w NATURAL FULL JOIN x",
    "SELECT DISTINCT ON (a) a, b FROM t WHERE a > $1 ORDER BY a DESC NULLS LAST, b USING < LIMIT 10 OFFSET 5",
    "SELECT * FROM t ORDER BY a FETCH FIRST 3 ROWS WITH TIES",
    "WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 10) SEARCH DEPTH FIRST BY n SET ord CYCLE n SET is_cycle USING path SELECT * FROM r",
    "WITH c AS MATERIALIZED (SELECT 1 AS x), d AS NOT MATERIALIZED (SELECT 2) SELECT * FROM c, d",
    "SELECT a, sum(b) OVER w, rank() OVER (PARTITION BY a ORDER BY b ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW EXCLUDE TIES) FROM t WINDOW w AS (PARTITION BY a)",
    "SELECT a, b, count(*) FILTER (WHERE c > 0), GROUPING(a, b) FROM t GROUP BY GROUPING SETS ((a), (b), ()) HAVING count(*) > 1",
    "SELECT a FROM t GROUP BY ROLLUP (a, b), CUBE (c)",
    "SELECT percentile_cont(0.5) WITHIN GROUP (ORDER BY a), string_agg(DISTINCT b, ',' ORDER BY b) FROM t",
    "SELECT * FROM t WHERE a IN (1, 2) AND b NOT IN (SELECT b FROM u) AND c = ANY (ARRAY[1, 2]) AND d > ALL (SELECT d FROM u) AND EXISTS (SELECT 1)",
    "SELECT CASE WHEN a THEN 1 WHEN b THEN 2 ELSE 3 END, CASE x WHEN 1 THEN 'a' END, COALESCE(a, b), NULLIF(a, b), GREATEST(a, b), LEAST(a, b) FROM t",
    "SELECT a IS NULL, b IS NOT NULL, c IS TRUE, d IS NOT FALSE, e IS UNKNOWN, f IS DISTINCT FROM g, h IS NOT DISTINCT FROM i FROM t",
    "SELECT a BETWEEN 1 AND 2, b NOT BETWEEN SYMMETRIC 2 AND 1, c LIKE 'x%' ESCAPE '!', d ILIKE 'y', e SIMILAR TO 'z', f NOT LIKE 'w' FROM t",
    "SELECT (ARRAY[1, 2])[1], (ARRAY[[1, 2], [3, 4]])[1:2][1], (row(1, 2)).f1, ROW(1, 'a'), (t.c).x, t.arr[2:] FROM t",
    "SELECT ARRAY(SELECT a FROM t), (SELECT max(a) FROM t), a::int, CAST(b AS text), c::numeric(10, 2), d::timestamptz, e COLLATE \"C\" FROM t",
    "SELECT CURRENT_DATE, CURRENT_TIME, CURRENT_TIMESTAMP(3), LOCALTIME, LOCALTIMESTAMP, CURRENT_ROLE, CURRENT_USER, SESSION_USER, CURRENT_CATALOG, CURRENT_SCHEMA",
    "SELECT extract(epoch FROM now()), substring('abc' FROM 2), substring('abc' SIMILAR 'b' ESCAPE '#'), trim(LEADING 'x' FROM 'xa'), position('b' IN 'abc'), overlay('abc' PLACING 'z' FROM 2), now() AT TIME ZONE 'UTC', now() AT LOCAL, COLLATION FOR ('a')",
    "SELECT xmlelement(name e, xmlattributes(1 AS a), 'x'), xmlforest(a, b), xmlparse(document '<a/>'), xmlserialize(content '<a/>' AS text), xmlconcat('<a/>', '<b/>'), xmlpi(name p), xmlroot('<a/>', version '1.0'), '<a/>' IS DOCUMENT FROM t",
    "SELECT * FROM XMLTABLE('/r' PASSING '<r/>' COLUMNS a int PATH 'a', b text DEFAULT 'x', o FOR ORDINALITY)",
    "SELECT json_object('a' VALUE 1), json_array(1, 2 RETURNING jsonb), json_objectagg(k VALUE v), json_arrayagg(v ORDER BY v), json('{}'), json_scalar(1), json_serialize('{}' RETURNING text), '{}' IS JSON OBJECT WITH UNIQUE KEYS FROM t",
    "SELECT json_query(j, '$.a' WITH WRAPPER), json_value(j, '$.b' RETURNING int DEFAULT 0 ON ERROR), json_exists(j, '$.c' PASSING 1 AS x) FROM t",
    "SELECT * FROM JSON_TABLE('[1,2]', '$[*]' COLUMNS (id FOR ORDINALITY, v int PATH '$', NESTED PATH '$.n' COLUMNS (w text PATH '$'))) jt",
    "SELECT * FROM t, LATERAL (SELECT * FROM u WHERE u.a = t.a) l, ROWS FROM (generate_series(1, 3), unnest(ARRAY[1])) WITH ORDINALITY r(a, b, o)",
    "SELECT * FROM t TABLESAMPLE BERNOULLI (10) REPEATABLE (42)",
    "SELECT f(a => 1, b => 2), g(VARIADIC ARRAY[1, 2]), -a, +b, @c, ~d, a || b, a + b * c / d % e ^ f FROM t",
    "VALUES (1, 'a'), (2, DEFAULT)",
    "TABLE t",
    "SELECT 1 UNION SELECT 2 INTERSECT SELECT 3 EXCEPT ALL SELECT 4",
    "SELECT * FROM t WHERE a = 1 FOR UPDATE OF t NOWAIT",
    "SELECT 'a'::text, E'b\\n', U&'d\\0061t', B'101', X'1F', 1.5e3, $$x$$, true, false, NULL",
    "SELECT * FROM t WHERE CURRENT_DATE - INTERVAL '1 day' < ts AND a <-> b < 3",
    // Data changes
    "INSERT INTO t (a, b) VALUES (1, 2) ON CONFLICT (a) WHERE b > 0 DO UPDATE SET b = EXCLUDED.b WHERE t.b < 5 RETURNING *",
    "INSERT INTO t DEFAULT VALUES",
    "INSERT INTO t OVERRIDING SYSTEM VALUE SELECT * FROM u ON CONFLICT ON CONSTRAINT k DO NOTHING",
    "INSERT INTO t AS x VALUES (1) RETURNING OLD.a, NEW.a",
    "UPDATE t SET (a, b) = (SELECT 1, 2), c = DEFAULT, d[1] = 3 FROM u WHERE t.id = u.id RETURNING t.*",
    "DELETE FROM t USING u WHERE t.id = u.id RETURNING *",
    "DELETE FROM t WHERE CURRENT OF c",
    "MERGE INTO t USING u ON t.id = u.id WHEN MATCHED AND u.x THEN UPDATE SET a = u.a WHEN NOT MATCHED BY TARGET THEN INSERT (id) VALUES (u.id) WHEN NOT MATCHED BY SOURCE THEN DELETE RETURNING merge_action(), t.*",
    "TRUNCATE ONLY t, u RESTART IDENTITY CASCADE",
    "COPY t (a, b) FROM STDIN WITH (FORMAT csv, HEADER true) WHERE a > 0",
    "COPY (SELECT * FROM t) TO STDOUT",
    // Schema changes
    "CREATE TABLE IF NOT EXISTS s.t (id int GENERATED ALWAYS AS IDENTITY PRIMARY KEY, a text NOT NULL DEFAULT 'x' CHECK (a <> ''), b int REFERENCES u (id) ON DELETE CASCADE, c numeric GENERATED ALWAYS AS (b * 2) STORED, UNIQUE (a, b), EXCLUDE USING gist (c WITH =), LIKE v INCLUDING ALL) PARTITION BY RANGE (id)",
    "CREATE TABLE p1 PARTITION OF t FOR VALUES FROM (1) TO (10)",
    "CREATE TABLE p2 PARTITION OF t FOR VALUES IN (1, 2)",
    "CREATE TABLE p3 PARTITION OF t FOR VALUES WITH (MODULUS 2, REMAINDER 0)",
    "CREATE UNLOGGED TABLE t2 (a int) WITH (fillfactor = 70) TABLESPACE ts",
    "CREATE TABLE t3 AS SELECT * FROM t WITH NO DATA",
    "CREATE MATERIALIZED VIEW mv AS SELECT 1",
    "CREATE OR REPLACE VIEW v (a) WITH (security_barrier) AS SELECT a FROM t WITH CASCADED CHECK OPTION",
    "CREATE RECURSIVE VIEW rv (n) AS SELECT 1 UNION ALL SELECT n + 1 FROM rv WHERE n < 3",
    "CREATE UNIQUE INDEX CONCURRENTLY IF NOT EXISTS i ON t USING btree (lower(a) DESC NULLS FIRST, b) INCLUDE (c) WHERE a IS NOT NULL",
    "CREATE FUNCTION f(a int, OUT b int, VARIADIC c int[] DEFAULT '{}') RETURNS SETOF record LANGUAGE sql STABLE STRICT SECURITY DEFINER SET search_path = pg_catalog AS $$ SELECT 1 $$",
    "CREATE FUNCTION g(int) RETURNS int LANGUAGE sql IMMUTABLE RETURN $1 + 1",
    "CREATE PROCEDURE p(INOUT x int) LANGUAGE sql BEGIN ATOMIC SELECT 1; INSERT INTO t VALUES (x); END",
    "CREATE TRIGGER tr AFTER INSERT OR UPDATE OF a ON t REFERENCING NEW TABLE AS nt FOR EACH STATEMENT WHEN (true) EXECUTE FUNCTION f()",
    "CREATE EVENT TRIGGER et ON ddl_command_start WHEN TAG IN ('CREATE TABLE') EXECUTE FUNCTION f()",
    "CREATE RULE r AS ON UPDATE TO t WHERE OLD.a <> NEW.a DO INSTEAD (INSERT INTO log VALUES (OLD.a); NOTIFY t)",
    "CREATE POLICY pol ON t AS PERMISSIVE FOR SELECT TO PUBLIC USING (owner = current_user) WITH CHECK (true)",
    "CREATE DOMAIN d AS int CONSTRAINT pos CHECK (VALUE > 0) NOT NULL",
    "CREATE TYPE ct AS (a int, b text)",
    "CREATE TYPE en AS ENUM ('a', 'b')",
    "CREATE TYPE rg AS RANGE (subtype = int)",
    "CREATE AGGREGATE agg (int) (sfunc = int4pl, stype = int)",
    "CREATE OPERATOR === (leftarg = int, rightarg = int, function = int4eq)",
    "CREATE CAST (int AS text) WITH FUNCTION f(int) AS ASSIGNMENT",
    "CREATE SEQUENCE IF NOT EXISTS sq AS bigint INCREMENT 2 START 10 OWNED BY t.a",
    "CREATE SCHEMA app AUTHORIZATION bob CREATE TABLE x (a int) CREATE VIEW y AS SELECT 1",
    "CREATE EXTENSION IF NOT EXISTS hstore WITH SCHEMA app VERSION '1.0' CASCADE",
    "CREATE FOREIGN DATA WRAPPER fdw HANDLER h VALIDATOR v",
    "CREATE SERVER srv FOREIGN DATA WRAPPER fdw OPTIONS (host 'x')",
    "CREATE USER MAPPING FOR CURRENT_USER SERVER srv OPTIONS (user 'u')",
    "CREATE FOREIGN TABLE ft (a int) SERVER srv OPTIONS (table_name 't')",
    "IMPORT FOREIGN SCHEMA remote LIMIT TO (a, b) FROM SERVER srv INTO app",
    "CREATE PUBLICATION pub FOR TABLE t (a, b) WHERE (a > 0), TABLES IN SCHEMA app",
    "CREATE SUBSCRIPTION sub CONNECTION 'host=x' PUBLICATION pub WITH (enabled = false)",
    "CREATE STATISTICS st (dependencies) ON a, (b + 1) FROM t",
    "CREATE ACCESS METHOD am TYPE TABLE HANDLER heap_tableam_handler",
    "CREATE OPERATOR CLASS oc FOR TYPE int USING btree AS OPERATOR 1 <, FUNCTION 1 btint4cmp(int, int)",
    "CREATE OPERATOR FAMILY of USING btree",
    "CREATE CONVERSION cv FOR 'LATIN1' TO 'UTF8' FROM iso8859_1_to_utf8",
    "CREATE TRANSFORM FOR int LANGUAGE plpgsql (FROM SQL WITH FUNCTION f(internal), TO SQL WITH FUNCTION g(internal))",
    "CREATE COLLATION co (provider = icu, locale = 'tr-TR')",
    "CREATE TEXT SEARCH DICTIONARY td (TEMPLATE = simple)",
    "CREATE LANGUAGE plsample HANDLER h",
    "CREATE TABLESPACE ts LOCATION '/x'",
    "CREATE DATABASE db WITH OWNER bob TEMPLATE template0 ENCODING 'UTF8'",
    "ALTER TABLE ONLY t ADD COLUMN IF NOT EXISTS z int, DROP COLUMN y CASCADE, ALTER COLUMN a SET DEFAULT 1, ALTER COLUMN b TYPE bigint USING b::bigint, ALTER COLUMN c SET NOT NULL, ADD CONSTRAINT k CHECK (a > 0) NOT VALID, VALIDATE CONSTRAINT k, DROP CONSTRAINT IF EXISTS k2, ENABLE ROW LEVEL SECURITY, SET (fillfactor = 50), OWNER TO bob, REPLICA IDENTITY FULL",
    "ALTER TABLE t ALTER CONSTRAINT fk DEFERRABLE INITIALLY DEFERRED",
    "ALTER TABLE t ATTACH PARTITION p FOR VALUES IN (3)",
    "ALTER TABLE t DETACH PARTITION p CONCURRENTLY",
    "ALTER TABLE t RENAME COLUMN a TO b",
    "ALTER TABLE t SET SCHEMA app",
    "ALTER TABLE ALL IN TABLESPACE a SET TABLESPACE b",
    "ALTER INDEX i ATTACH PARTITION j",
    "ALTER INDEX i DEPENDS ON EXTENSION e",
    "ALTER TABLESPACE ts SET (seq_page_cost = 1)",
    "ALTER DOMAIN d ADD CONSTRAINT c CHECK (VALUE < 10)",
    "ALTER COLLATION co REFRESH VERSION",
    "ALTER EXTENSION hstore UPDATE TO '2.0'",
    "ALTER EXTENSION hstore ADD FUNCTION f(int)",
    "ALTER FOREIGN DATA WRAPPER fdw OPTIONS (ADD x 'y')",
    "ALTER SERVER srv OPTIONS (SET host 'y')",
    "ALTER USER MAPPING FOR bob SERVER srv OPTIONS (DROP user)",
    "ALTER POLICY pol ON t USING (false)",
    "ALTER EVENT TRIGGER et DISABLE",
    "ALTER SEQUENCE sq RESTART WITH 1",
    "ALTER OPERATOR FAMILY of USING btree ADD OPERATOR 1 < (int, int)",
    "ALTER STATISTICS st SET STATISTICS 100",
    "ALTER FUNCTION f(int) IMMUTABLE",
    "ALTER OPERATOR === (int, int) SET (restrict = eqsel)",
    "ALTER TYPE ct ADD ATTRIBUTE c int",
    "ALTER TYPE en ADD VALUE IF NOT EXISTS 'c' BEFORE 'a'",
    "ALTER TYPE rg SET (subscript = f)",
    "ALTER DATABASE db SET work_mem = '1MB'",
    "ALTER DATABASE db WITH CONNECTION LIMIT 5",
    "ALTER DATABASE db REFRESH COLLATION VERSION",
    "ALTER TEXT SEARCH DICTIONARY td (StopWords = english)",
    "ALTER TEXT SEARCH CONFIGURATION tc ADD MAPPING FOR word WITH simple",
    "ALTER PUBLICATION pub ADD TABLE u",
    "ALTER SUBSCRIPTION sub REFRESH PUBLICATION",
    "COMMENT ON TABLE t IS 'x'",
    "SECURITY LABEL FOR p ON TABLE t IS 'x'",
    "DROP TABLE IF EXISTS t, u CASCADE",
    "DROP FUNCTION f(int), g",
    "DROP TYPE ct",
    "DROP INDEX CONCURRENTLY i",
    "DROP TABLESPACE ts",
    "DROP USER MAPPING FOR bob SERVER srv",
    "DROP DATABASE IF EXISTS db WITH (FORCE)",
    "DROP SUBSCRIPTION sub",
    "DROP OWNED BY bob CASCADE",
    "REASSIGN OWNED BY bob TO alice",
    "VACUUM (FULL, ANALYZE) t (a)",
    "ANALYZE VERBOSE t",
    "CLUSTER t USING i",
    "REINDEX (VERBOSE) TABLE CONCURRENTLY t",
    "REFRESH MATERIALIZED VIEW CONCURRENTLY mv WITH DATA",
    // Privileges
    "GRANT SELECT (a), UPDATE ON TABLE t, u TO bob, GROUP g WITH GRANT OPTION GRANTED BY CURRENT_USER",
    "REVOKE ALL PRIVILEGES ON ALL TABLES IN SCHEMA app FROM PUBLIC CASCADE",
    "GRANT r TO bob WITH ADMIN OPTION",
    "REVOKE r FROM bob",
    "ALTER DEFAULT PRIVILEGES IN SCHEMA app GRANT SELECT ON TABLES TO bob",
    "CREATE ROLE r WITH LOGIN PASSWORD NULL VALID UNTIL 'infinity' IN ROLE g",
    "ALTER ROLE r SET search_path = app",
    "ALTER ROLE r NOSUPERUSER",
    "DROP ROLE IF EXISTS r",
    "ALTER TABLE t OWNER TO bob",
    "ALTER FUNCTION f(int) OWNER TO bob",
    "ALTER SCHEMA app OWNER TO CURRENT_ROLE",
    // Node-wrapped forms of otherwise direct messages
    "CREATE TABLE p4 PARTITION OF t FOR VALUES FROM (MINVALUE) TO (MAXVALUE)",
    "ALTER TABLE t SET (fillfactor = 70.5)",
    "UPDATE t SET a = 1 WHERE b = 2 RETURNING WITH (OLD AS o, NEW AS n) o.a, n.a",
    "SELECT json_array(SELECT a FROM t RETURNING jsonb)",
    "SELECT json_arrayagg(a RETURNING jsonb), json_objectagg(k : v ABSENT ON NULL WITH UNIQUE KEYS) FROM t",
    // Transactions and session state
    "BEGIN ISOLATION LEVEL SERIALIZABLE, READ ONLY, DEFERRABLE",
    "SAVEPOINT s",
    "RELEASE s",
    "ROLLBACK TO s",
    "COMMIT AND CHAIN",
    "PREPARE TRANSACTION 'tx'",
    "COMMIT PREPARED 'tx'",
    "ROLLBACK PREPARED 'tx'",
    "SET CONSTRAINTS ALL DEFERRED",
    "SET LOCAL search_path TO app, public",
    "SET TIME ZONE INTERVAL '+00:00' HOUR TO MINUTE",
    "SET SESSION CHARACTERISTICS AS TRANSACTION READ ONLY",
    "SET TRANSACTION SNAPSHOT '000003A1-1'",
    "SET XML OPTION DOCUMENT",
    "SET search_path FROM CURRENT",
    "RESET ALL",
    "SHOW search_path",
    "DISCARD PLANS",
    "PREPARE q (int, text) AS SELECT * FROM t WHERE a = $1 AND b = $2",
    "EXECUTE q (1, 'x')",
    "DEALLOCATE PREPARE q",
    "DECLARE c SCROLL CURSOR WITH HOLD FOR SELECT * FROM t",
    "FETCH FORWARD 10 FROM c",
    "MOVE LAST IN c",
    "CLOSE ALL",
    "LISTEN ch",
    "UNLISTEN ch",
    "NOTIFY ch, 'payload'",
    "LOCK TABLE t, u IN SHARE ROW EXCLUSIVE MODE",
    // Code and administration
    "DO LANGUAGE plpgsql $$ BEGIN NULL; END $$",
    "CALL p(1, x => 2)",
    "LOAD 'auto_explain'",
    "CHECKPOINT",
    "ALTER SYSTEM SET work_mem = '1GB'",
    "ALTER SYSTEM RESET ALL",
    "EXPLAIN (ANALYZE, VERBOSE, COSTS false, BUFFERS, FORMAT YAML, SETTINGS, WAL, TIMING, SUMMARY, MEMORY, SERIALIZE TEXT) SELECT 1",
    "EXPLAIN CREATE TABLE x AS SELECT 1",
];

/// `Node` variants a raw (unanalysed) parse tree never holds inside a `Node`: messages the
/// grammar puts only in typed fields (the walk reaches them there, and the `Node`s under them
/// are counted), and the ones only analysis or the PL/pgSQL grammar builds.
const NOT_NODE_WRAPPED_IN_A_RAW_TREE: [&str; 24] = [
    // Typed fields only
    "Alias",
    "BitString",
    "CtecycleClause",
    "CtesearchClause",
    "InferClause",
    "IntoClause",
    "JsonAggConstructor",
    "JsonBehavior",
    "JsonFormat",
    "JsonOutput",
    "JsonReturning",
    "JsonTablePathSpec",
    "OnConflictClause",
    "PartitionBoundSpec",
    "PartitionSpec",
    "PublicationTable",
    "RawStmt",
    "ReturningClause",
    "WithClause",
    // Built by analysis (raw bounds are expressions) or by the PL/pgSQL grammar
    "IntList",
    "OidList",
    "PartitionRangeDatum",
    "PlassignStmt",
    "SetOperationStmt",
];

/// The `Node` oneof variants of a JSON parse tree: every object holding a `"node"` key whose
/// value is an object with one key in UpperCamelCase (prost's variant name). `SortBy.node`
/// is itself a `Node` (`{"node": {"node": …}}`), so a lower-case key is not a variant.
/// The messages of a JSON parse tree: every object but the `oneof` wrappers (prost serializes
/// a oneof as `{"Variant": {…}}`; struct field names are snake_case).
fn json_messages(v: &Value) -> usize {
    match v {
        Value::Object(map) => {
            let wrapper = map.len() == 1
                && map
                    .keys()
                    .next()
                    .is_some_and(|k| k.starts_with(|c: char| c.is_ascii_uppercase()));
            usize::from(!wrapper) + map.values().map(json_messages).sum::<usize>()
        }
        Value::Array(items) => items.iter().map(json_messages).sum(),
        _ => 0,
    }
}

fn json_counts(v: &Value, out: &mut BTreeMap<String, usize>) {
    match v {
        Value::Object(map) => {
            if let Some(Value::Object(inner)) = map.get("node")
                && inner.len() == 1
                && let Some(k) = inner.keys().next()
                && k.starts_with(|c: char| c.is_ascii_uppercase())
            {
                *out.entry(k.clone()).or_default() += 1;
            }
            for v in map.values() {
                json_counts(v, out);
            }
        }
        Value::Array(items) => items.iter().for_each(|v| json_counts(v, out)),
        _ => {}
    }
}

/// The corpus, plus every PostgreSQL statement of the splitter corpus that parses.
fn corpus() -> Vec<String> {
    let mut out: Vec<String> = CORPUS.iter().map(|s| (*s).to_string()).collect();
    let cases = sql_testkit::split::load(&sql_testkit::split::corpus_file("pg"));
    for c in cases {
        for s in c.editor {
            if parse_tree(&s).is_ok() {
                out.push(s);
            }
        }
    }
    out
}

#[test]
fn pg_visitor_reaches_every_node_kind_in_the_corpus() {
    let unhandled = hooks::unhandled_node_variants();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let texts = corpus();
    assert!(
        texts.len() > CORPUS.len(),
        "the splitter corpus adds statements"
    );
    for sql in &texts {
        let tree = parse_tree(sql).unwrap_or_else(|e| panic!("{sql}: {e}"));
        let mut independent = BTreeMap::new();
        let json = serde_json::to_value(&tree).unwrap();
        json_counts(&json, &mut independent);
        // Every message, typed fields included (round 1, M8): each JSON object is a message,
        // except a `oneof` wrapper (one key in UpperCamelCase, the variant's name).
        assert_eq!(
            hooks::visit_total(sql).unwrap(),
            json_messages(&json),
            "{sql}"
        );
        let visited: BTreeMap<String, usize> = hooks::visit_counts(sql)
            .unwrap()
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
        assert_eq!(visited, independent, "{sql}");
        let falls_through = hooks::unhandled_kinds(sql).unwrap();
        assert!(falls_through.is_empty(), "{sql}: {falls_through:?}");
        for k in independent.keys() {
            assert!(!unhandled.contains(k.as_str()), "{sql}: {k}");
        }
        // Classified: one report per statement, none Unknown for lack of a rule.
        let ctx = ClassifyCtx::new(
            DialectCtx::new(DialectId::Postgres, Some(ServerVersion::new(18, 0, 0))),
            ClientFlavor::Psql,
            Expect::Script,
        );
        let r = hooks::classify_pg(sql, &ctx);
        assert!(!r.items.is_empty(), "{sql}");
        for s in &r.items {
            assert!(
                !s.reasons
                    .iter()
                    .any(|x| x.code == sql_safety::ReasonCode::EffectsUnknown),
                "{sql}: {s:?}"
            );
        }
        seen.extend(independent.into_keys());
    }
    // The corpus exercises every node kind a raw parse tree can hold.
    let raw: BTreeSet<String> = hooks::node_variants()
        .difference(&unhandled)
        .map(|v| (*v).to_string())
        .collect();
    let missing: Vec<&str> = raw
        .difference(&seen)
        .map(String::as_str)
        .filter(|k| !NOT_NODE_WRAPPED_IN_A_RAW_TREE.contains(k))
        .collect();
    assert!(
        missing.is_empty(),
        "{} of {} kinds; missing {missing:?}",
        seen.len(),
        raw.len()
    );
    assert!(seen.len() >= 197, "{}", seen.len());
}

#[test]
fn reads_in_the_corpus_classify_as_reads() {
    let ctx = ClassifyCtx::new(
        DialectCtx::new(DialectId::Postgres, Some(ServerVersion::new(18, 0, 0))),
        ClientFlavor::Psql,
        Expect::SingleStatement,
    );
    // The read block of the corpus (up to the first data change), minus the forms that lock,
    // name a non-builtin or a user operator.
    for sql in CORPUS.iter().take_while(|s| !s.starts_with("INSERT")) {
        let r = hooks::classify_pg(sql, &ctx);
        let expect_read =
            !sql.contains("FOR UPDATE") && !sql.contains(" f(") && !sql.contains("g(VARIADIC");
        if expect_read {
            assert_eq!(r.request_class, CoarseClass::Read, "{sql}: {r:?}");
        } else {
            assert_ne!(r.request_class, CoarseClass::Read, "{sql}");
        }
    }
}

#[test]
fn the_statement_table_covers_every_statement_message() {
    let rows = hooks::statement_rows().expect("data/pg/statements.toml loads");
    let messages = hooks::statement_messages();
    assert_eq!(rows, messages);
    let by_name: BTreeSet<&str> = hooks::message_names()
        .into_iter()
        .filter(|n| n.ends_with("Stmt") && *n != "RawStmt")
        .collect();
    assert_eq!(messages, by_name);
}

#[test]
fn the_statement_table_rejects_bad_rows() {
    let ok = "rows = [ { node = \"SelectStmt\", kind = \"Select\", effects = [\"READ\"] } ]";
    assert_eq!(hooks::load_table(ok), Ok(1));
    for bad in [
        "rows = [ { node = \"SelectStmt\", kind = \"Select\", effects = [\"READS\"] } ]",
        "rows = [ { node = \"SelectStmt\", kind = \"Picks\", effects = [] } ]",
        "rows = [ { node = \"FuncCall\", kind = \"Select\", effects = [] } ]",
        "rows = [ { node = \"NoSuchStmt\", kind = \"Select\", effects = [] } ]",
        "rows = [ { node = \"SelectStmt\", kind = \"Select\", effects = [], reasons = [\"Nope\"] } ]",
        "rows = [ { node = \"SelectStmt\", kind = \"Select\", effects = [], flags = [\"NOPE\"] } ]",
        "rows = [ { node = \"SelectStmt\", kind = \"Select\", effects = [], extra = 1 } ]",
        "rows = [ { node = \"SelectStmt\", kind = \"Select\", effects = [] }, { node = \"SelectStmt\", kind = \"Select\", effects = [] } ]",
    ] {
        assert!(hooks::load_table(bad).is_err(), "{bad}");
    }
}

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

/// The generated visitor's first line records the digest of the schema it was generated from
/// (`cargo xtask gen-pg-visitor`; CI also runs `--check`, which regenerates in full).
#[test]
fn generated_visitor_is_fresh() {
    let proto =
        std::fs::read(workspace().join("third_party/pg_query/libpg_query/protobuf/pg_query.proto"))
            .unwrap();
    let text = String::from_utf8(proto).unwrap().replace("\r\n", "\n");
    let want = format!(
        "// proto-digest: xxh3-64:{:016x}",
        XxHash3_64::oneshot(text.as_bytes())
    );
    assert_eq!(hooks::generated_digest_line(), want);
    // Every `Node` arm of the schema is a variant the generated walk names.
    let arms = text
        .split("message Node")
        .nth(1)
        .and_then(|rest| rest.split("\n}\n").next())
        .unwrap()
        .lines()
        .filter(|l| l.contains("[json_name="))
        .count();
    assert_eq!(arms, 271);
}

/// Every `.rs` file under `dir`.
fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// R9: the classifier walks the generated visitor only, never `pg_query`'s own walker or
/// summary (`nodes()`, `NodeRef`, the parse result's `tables`/`functions`/…).
#[test]
fn pg_query_walkers_are_never_used() {
    let mut files = Vec::new();
    rust_files(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    files.push(workspace().join("crates/sql/sql-oracle-pg/src/tree.rs"));
    let forbidden = [
        ".nodes()",
        ".nodes_mut()",
        "NodeRef",
        "NodeMut",
        "pg_query::summary",
        ".tables",
        ".cte_names",
        ".filter_columns",
        ".aliases",
        "parsed.functions",
        "deparse",
    ];
    for f in files {
        if f.ends_with("visit/pg_generated.rs") {
            // Generated from the schema: its field names (`m.tables`, ...) are the tree's.
            continue;
        }
        if f.ends_with("backend/interim.rs") {
            // m3b merge (T9 + T10): the interim backend walks sqlparser's AST (its `.tables`
            // are sqlparser fields); it never reaches pg_query (R-M3-54, check-deps).
            continue;
        }
        let text = std::fs::read_to_string(&f).unwrap();
        for needle in forbidden {
            assert!(!text.contains(needle), "{}: {needle}", f.display());
        }
    }
}
