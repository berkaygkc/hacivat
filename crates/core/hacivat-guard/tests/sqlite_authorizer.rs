//! SQLite hardening values and the default-deny authorizer (B21, Q16).

use hacivat_guard::{Authorization, SqliteAction, sqlite_authorize, sqlite_hardening};

#[test]
fn sqlite_hardening_values() {
    let h = sqlite_hardening();
    assert!(h.query_only && h.defensive && !h.attach_create && !h.attach_write);
    assert_eq!(h.limit_attached, 0);
}

#[test]
fn sqlite_hardening_limits_are_exact() {
    let h = sqlite_hardening();
    assert_eq!(h.limit_length, 100_000_000);
    assert_eq!(h.limit_sql_length, 1 << 20);
    assert_eq!(h.limit_variable_number, 32_766);
}

#[test]
fn sqlite_authorizer_default_deny() {
    use SqliteAction::*;
    for (a, want) in [
        (
            Read {
                table: "t",
                column: "a",
            },
            Authorization::Allow,
        ),
        (Select, Authorization::Allow),
        (Function { name: "count" }, Authorization::Allow),
        (
            Function {
                name: "load_extension",
            },
            Authorization::Deny,
        ),
        (Function { name: "writefile" }, Authorization::Deny),
        (
            Pragma {
                name: "table_info",
                arg: None,
            },
            Authorization::Allow,
        ),
        (
            Pragma {
                name: "query_only",
                arg: None,
            },
            Authorization::Allow,
        ),
        (
            Pragma {
                name: "journal_mode",
                arg: None,
            },
            Authorization::Allow,
        ),
        (
            Pragma {
                name: "query_only",
                arg: Some("0"),
            },
            Authorization::Deny,
        ),
        (
            Pragma {
                name: "journal_mode",
                arg: Some("delete"),
            },
            Authorization::Deny,
        ),
        (
            Pragma {
                name: "writable_schema",
                arg: None,
            },
            Authorization::Deny,
        ),
        (Attach { file: "new.db" }, Authorization::Deny),
        (Detach, Authorization::Deny),
        (Insert { table: "temp_t" }, Authorization::Deny),
        (Transaction, Authorization::Deny),
        (Other(99), Authorization::Deny),
    ] {
        assert_eq!(sqlite_authorize(&a), want, "{a:?}");
    }
}

const READ_PRAGMAS: [&str; 20] = [
    "table_info",
    "table_xinfo",
    "index_list",
    "index_info",
    "index_xinfo",
    "foreign_key_list",
    "database_list",
    "collation_list",
    "function_list",
    "pragma_list",
    "schema_version",
    "user_version",
    "page_count",
    "page_size",
    "encoding",
    "compile_options",
    "query_only",
    "journal_mode",
    "trusted_schema",
    "foreign_keys",
];

#[test]
fn every_read_pragma_is_allowed_without_argument_in_any_case() {
    for name in READ_PRAGMAS {
        let upper = name.to_ascii_uppercase();
        for n in [name, upper.as_str()] {
            assert_eq!(
                sqlite_authorize(&SqliteAction::Pragma { name: n, arg: None }),
                Authorization::Allow,
                "{n}"
            );
        }
    }
}

#[test]
fn every_pragma_with_an_argument_is_denied() {
    for name in READ_PRAGMAS {
        for arg in ["", "0", "1", "t", "main", "OFF"] {
            assert_eq!(
                sqlite_authorize(&SqliteAction::Pragma {
                    name,
                    arg: Some(arg)
                }),
                Authorization::Deny,
                "{name}({arg:?})"
            );
        }
    }
}

#[test]
fn unlisted_and_malformed_pragma_names_are_denied() {
    for name in [
        "writable_schema",
        "cell_size_check",
        "ignore_check_constraints",
        "schema.query_only",
        "main.query_only",
        "query_only ",
        " query_only",
        "query_onl",
        "query_only\0",
        "",
        "wal_checkpoint",
        "optimize",
        "incremental_vacuum",
        "integrity_check",
        "temp_store_directory",
        "data_store_directory",
        "mmap_size",
        "locking_mode",
        "secure_delete",
    ] {
        assert_eq!(
            sqlite_authorize(&SqliteAction::Pragma { name, arg: None }),
            Authorization::Deny,
            "{name:?}"
        );
    }
}

#[test]
fn dangerous_functions_are_denied_in_any_case() {
    for name in [
        "load_extension",
        "LOAD_EXTENSION",
        "Load_Extension",
        "readfile",
        "ReadFile",
        "writefile",
        "WRITEFILE",
        "edit",
        "EDIT",
        "fts3_tokenizer",
        "FTS3_Tokenizer",
    ] {
        assert_eq!(
            sqlite_authorize(&SqliteAction::Function { name }),
            Authorization::Deny,
            "{name}"
        );
    }
    for name in ["count", "lower", "json_extract", "editdist3", "readfiles"] {
        assert_eq!(
            sqlite_authorize(&SqliteAction::Function { name }),
            Authorization::Allow,
            "{name}"
        );
    }
}

#[test]
fn every_writing_or_attaching_action_is_denied() {
    use SqliteAction::*;
    for a in [
        Attach { file: "" },
        Attach { file: ":memory:" },
        Detach,
        Insert { table: "t" },
        Insert {
            table: "sqlite_master",
        },
        Update { table: "t" },
        Update {
            table: "sqlite_temp_master",
        },
        Delete { table: "t" },
        Delete { table: "temp_t" },
        Create,
        Drop,
        Alter,
        Transaction,
        Savepoint,
        Analyze,
    ] {
        assert_eq!(sqlite_authorize(&a), Authorization::Deny, "{a:?}");
    }
}

#[test]
fn every_raw_action_code_is_denied() {
    // Whatever code Task 11's adapter cannot name falls through as `Other`; the SQLite action
    // codes run 0..=33.
    for code in (-1..=40).chain([i32::MIN, i32::MAX]) {
        assert_eq!(
            sqlite_authorize(&SqliteAction::Other(code)),
            Authorization::Deny,
            "{code}"
        );
    }
}

#[test]
fn reads_and_recursion_are_allowed() {
    use SqliteAction::*;
    for a in [
        Read {
            table: "sqlite_master",
            column: "sql",
        },
        Read {
            table: "",
            column: "",
        },
        Select,
        Recursive,
    ] {
        assert_eq!(sqlite_authorize(&a), Authorization::Allow, "{a:?}");
    }
}
