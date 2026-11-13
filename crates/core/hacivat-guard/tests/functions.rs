//! The text layer for calls a rollback cannot undo (Task 23 finding F1, controller ruling):
//! PostgreSQL statements that call a function of `PG_NON_TRANSACTIONAL_FUNCTIONS` are refused
//! before any I/O, in every spelling of the call the server's lexer reads as that function.

use hacivat_guard::{
    FunctionViolation, MAX_STRING_NESTING, PG_NON_TRANSACTIONAL_FUNCTIONS, check_functions,
};
use hacivat_proto::DbmsKind;

fn pg(sql: &str) -> Result<(), FunctionViolation> {
    check_functions(DbmsKind::Postgres, sql)
}

fn refused(sql: &str, name: &'static str) {
    assert_eq!(
        pg(sql),
        Err(FunctionViolation::NonTransactional(name)),
        "{sql}"
    );
}

const EMIT: &str = "pg_logical_emit_message";

#[test]
fn the_list_is_lower_case_ascii_and_distinct() {
    let mut seen = std::collections::BTreeSet::new();
    for f in PG_NON_TRANSACTIONAL_FUNCTIONS {
        assert!(f.starts_with("pg_"), "{f}");
        assert!(
            f.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'),
            "{f}"
        );
        assert!(seen.insert(f), "{f} twice");
    }
    // F1's function and the controller's named siblings are on it; pg_notify (transactional,
    // delivered on COMMIT only) is not.
    for f in [
        EMIT,
        "pg_replication_origin_advance",
        "pg_create_physical_replication_slot",
        "pg_create_logical_replication_slot",
        "pg_drop_replication_slot",
        "pg_switch_wal",
        "pg_promote",
    ] {
        assert!(seen.contains(f), "{f}");
    }
    assert!(!seen.contains("pg_notify"));
}

#[test]
fn every_listed_function_is_refused() {
    for f in PG_NON_TRANSACTIONAL_FUNCTIONS {
        refused(&format!("SELECT {f}()"), f);
        refused(&format!("SELECT * FROM pg_catalog.{f}('a', 'b')"), f);
    }
}

#[test]
fn both_overloads_and_every_arity_of_emit_message_are_refused() {
    // PostgreSQL 13–16: (boolean, text, text) and (boolean, text, bytea); 17+ adds `flush`.
    for sql in [
        "SELECT pg_logical_emit_message(false, 'hv', 'x')",
        "SELECT pg_logical_emit_message(true, 'hv', 'x')",
        "SELECT pg_logical_emit_message(false, 'hv', '\\x00'::bytea)",
        "SELECT pg_logical_emit_message(false, 'hv', 'x', true)",
        "SELECT pg_logical_emit_message(transactional => false, prefix => 'hv', message => 'x')",
        "SELECT pg_logical_emit_message(VARIADIC ARRAY[]::text[])",
    ] {
        refused(sql, EMIT);
    }
}

#[test]
fn any_qualification_quoting_case_and_trivia_is_refused() {
    for sql in [
        "SELECT PG_LOGICAL_EMIT_MESSAGE(false, 'hv', 'x')",
        "select Pg_Logical_Emit_Message(false, 'hv', 'x')",
        "SELECT pg_catalog.pg_logical_emit_message(false, 'hv', 'x')",
        "SELECT PG_CATALOG.pg_logical_emit_message(false, 'hv', 'x')",
        "SELECT \"pg_catalog\".\"pg_logical_emit_message\"(false, 'hv', 'x')",
        "SELECT \"pg_logical_emit_message\"(false, 'hv', 'x')",
        // A quoted name in another case is another function; refusing it fails closed.
        "SELECT \"PG_LOGICAL_EMIT_MESSAGE\"(false, 'hv', 'x')",
        "SELECT public.pg_logical_emit_message(false, 'hv', 'x')",
        "SELECT pg_catalog . pg_logical_emit_message (false, 'hv', 'x')",
        "SELECT pg_catalog/* a */./* b */pg_logical_emit_message/* c */(false, 'hv', 'x')",
        "SELECT pg_logical_emit_message /* a /* nested */ b */ (false, 'hv', 'x')",
        "SELECT pg_logical_emit_message -- line\n(false, 'hv', 'x')",
        "SELECT pg_logical_emit_message -- line\r(false, 'hv', 'x')",
        "SELECT pg_logical_emit_message\t\x0b\x0c\n(false, 'hv', 'x')",
        // `U&` identifiers with their escapes, the default and a declared one.
        "SELECT U&\"pg\\005flogical_emit_message\"(false, 'hv', 'x')",
        "SELECT u&\"\\0070g_logical_emit_message\"(false, 'hv', 'x')",
        "SELECT U&\"\\+000070g_logical_emit_message\"(false, 'hv', 'x')",
        "SELECT U&\"pg!005flogical_emit_message\" UESCAPE '!' (false, 'hv', 'x')",
        "SELECT U&\"pg!005flogical_emit_message\" /* c */ uescape /* c */ '!'(false, 'hv', 'x')",
        "SELECT pg_catalog.U&\"pg_logical_emit_message\"(false, 'hv', 'x')",
        // A table function, in FROM and LATERAL.
        "SELECT * FROM pg_logical_emit_message(false, 'hv', 'x')",
        "SELECT * FROM (SELECT 1) s, LATERAL pg_catalog.pg_logical_emit_message(false, 'hv', 'x') m",
        "WITH w AS (SELECT pg_logical_emit_message(false, 'hv', 'x')) SELECT * FROM w",
        "EXPLAIN ANALYZE SELECT pg_logical_emit_message(false, 'hv', 'x')",
    ] {
        refused(sql, EMIT);
    }
}

#[test]
fn calls_inside_literals_that_may_run_are_refused() {
    // A DO block, PL/pgSQL `EXECUTE` and `query_to_xml` run the text of a literal.
    for sql in [
        "DO $$ BEGIN PERFORM pg_logical_emit_message(false, 'hv', 'x'); END $$",
        "DO $body$ BEGIN PERFORM pg_catalog.pg_logical_emit_message(false, 'hv', 'x'); END $body$",
        "DO $a$ BEGIN EXECUTE $b$ SELECT pg_logical_emit_message(false, 'hv', 'x') $b$; END $a$",
        "DO LANGUAGE plpgsql 'BEGIN PERFORM pg_logical_emit_message(false, ''hv'', ''x''); END'",
        "SELECT query_to_xml('SELECT pg_logical_emit_message(false, ''hv'', ''x'')', true, true, '')",
        // Escapes a literal decodes: `E''`, `U&''` (default and declared escape).
        "SELECT query_to_xml(E'SELECT pg\\x5flogical_emit_message(false, ''hv'', ''x'')', true, true, '')",
        "SELECT query_to_xml(e'SELECT pg\\137logical_emit_message(false, \\'hv\\', \\'x\\')', true, true, '')",
        "SELECT query_to_xml(E'SELECT pg\\u005flogical_emit_message(false, ''hv'', ''x'')', true, true, '')",
        "SELECT query_to_xml(E'SELECT pg\\U0000005flogical_emit_message(false, ''hv'', ''x'')', true, true, '')",
        "SELECT query_to_xml(E'SELECT \\pg_logical_emit_message\\(false, ''hv'', ''x'')', true, true, '')",
        "SELECT query_to_xml(U&'SELECT pg\\005flogical_emit_message(false, ''hv'', ''x'')', true, true, '')",
        "SELECT query_to_xml(U&'SELECT pg#005flogical_emit_message(false, ''hv'', ''x'')' UESCAPE '#', true, true, '')",
        // A constant continued over a line break is one constant.
        "SELECT query_to_xml('SELECT pg_logical_'\n'emit_message(false, ''hv'', ''x'')', true, true, '')",
        "SELECT query_to_xml('SELECT pg_logical_' \n -- c\n 'emit_message(false, ''hv'', ''x'')', true, true, '')",
        // Nested quoting, and the reading with `standard_conforming_strings` off.
        "DO $$ BEGIN EXECUTE 'SELECT query_to_xml(''SELECT pg_logical_emit_message(false, ''''hv'''', ''''x'''')'', true, true, '''')'; END $$",
        "DO $$ BEGIN SET LOCAL standard_conforming_strings = off; EXECUTE 'SELECT query_to_xml(''SELECT pg\\x5flogical_emit_message(false, ''''hv'''', ''''x'''')'', true, true, '''')'; END $$",
        "DO $$ BEGIN EXECUTE E'SELECT query_to_xml(\\'SELECT U&\"pg\\\\005flogical_emit_message\"(false, \\'\\'hv\\'\\', \\'\\'x\\'\\')\\', true, true, \\'\\')'; END $$",
        // Unterminated: the server refuses it, and so does the check (fails closed).
        "SELECT 'pg_logical_emit_message(false, ''hv'', ''x'')",
        "SELECT $$ pg_logical_emit_message(false, 'hv', 'x')",
    ] {
        refused(sql, EMIT);
    }
}

#[test]
fn reads_names_and_unlisted_functions_pass() {
    for sql in [
        "SELECT 1",
        "SELECT * FROM pg_proc WHERE proname = 'pg_logical_emit_message'",
        "SELECT proname FROM pg_proc WHERE proname IN ('pg_switch_wal', 'pg_promote')",
        "SELECT 'pg_logical_emit_message'",
        "SELECT pg_logical_emit_message",
        "SELECT pg_logical_emit_message FROM t",
        "SELECT x.pg_logical_emit_message FROM t x",
        "/* pg_logical_emit_message(false, 'hv', 'x') */ SELECT 1",
        "-- pg_logical_emit_message(false, 'hv', 'x')\nSELECT 1",
        "SELECT pg_logical_slot_peek_changes('s', NULL, NULL)",
        "SELECT pg_replication_origin_oid('o'), pg_replication_origin_progress('o', false)",
        "SELECT pg_notify('hv', 'x')",
        "SELECT pg_advisory_lock(1)",
        "SELECT txid_current_if_assigned()",
        "SELECT pg_logical_emit_message_x(1)",
        "SELECT xpg_logical_emit_message(1)",
        "SELECT pg_logical_emit_message$(1)",
        "SELECT pg_logical_emit_message2(1)",
        "SELECT \"pg_logical_emit_message \"(1)",
        "SELECT pg_logical_emit_message.f(1)",
        "SELECT $1, $2::text",
        "SELECT a$b$c FROM t",
        "SELECT E'\\'pg_logical_emit_message'",
        "SELECT 1 FROM t WHERE name ~ 'pg_logical_emit_message'",
        "",
    ] {
        assert_eq!(pg(sql), Ok(()), "{sql}");
    }
}

#[test]
fn other_engines_pass() {
    for kind in [
        DbmsKind::MySql,
        DbmsKind::MariaDb,
        DbmsKind::Sqlite,
        DbmsKind::MsSql,
    ] {
        assert_eq!(
            check_functions(kind, "SELECT pg_logical_emit_message(false, 'hv', 'x')"),
            Ok(()),
            "{kind:?}"
        );
    }
}

/// `levels` dollar-quoted literals, one inside the other, around `inner`.
fn nested(levels: usize, inner: &str) -> String {
    let mut s = inner.to_owned();
    for k in 0..levels {
        s = format!("$t{k}${s}$t{k}$");
    }
    format!("SELECT {s}")
}

#[test]
fn literals_are_followed_to_the_nesting_bound_then_refused() {
    let call = " SELECT pg_switch_wal() ";
    assert_eq!(
        pg(&nested(MAX_STRING_NESTING, call)),
        Err(FunctionViolation::NonTransactional("pg_switch_wal"))
    );
    assert_eq!(
        pg(&nested(MAX_STRING_NESTING, " SELECT 1 ")),
        Ok(()),
        "a deep read passes"
    );
    assert_eq!(
        pg(&nested(MAX_STRING_NESTING + 1, " SELECT 1 ")),
        Err(FunctionViolation::TooComplex)
    );
}

#[test]
fn a_statement_that_would_take_too_long_to_read_is_refused() {
    // Within the 1 MiB input limit, each nested literal is read again: sixteen levels of a
    // literal just under 1 MiB exceed the check's budget, so it refuses instead of reading on.
    let filler = "x".repeat(1_048_000);
    let sql = nested(MAX_STRING_NESTING, &filler);
    assert!(sql.len() <= hacivat_guard::MAX_STATEMENT_BYTES);
    assert_eq!(pg(&sql), Err(FunctionViolation::TooComplex));
    // The same text without nesting is read in one pass.
    assert_eq!(pg(&format!("SELECT '{filler}'")), Ok(()));
}
