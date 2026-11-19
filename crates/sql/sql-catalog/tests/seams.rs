//! M4 Task 1a seams of `sql_catalog`: the fail-closed `EmptyCatalog`, the reason codes and the
//! "name only" function signature (M4 review I5, X-8).

use std::sync::Arc;

use sql_catalog::empty::EmptyCatalog;
use sql_catalog::lookup::{Lookup, MissingReason, UnknownReason};
use sql_catalog::types::{FnArg, FnKind, FnSig, RelRef};
use sql_catalog::view::CatalogView;
use sql_ident::key::{DialectId, IdentCtx};

fn unknown_not_loaded<T: std::fmt::Debug>(l: Lookup<T>) -> bool {
    matches!(l, Lookup::Unknown(UnknownReason::NotLoaded))
}

/// M5's test, moved (R-M4-27): every lookup of the empty catalog is `Unknown(NotLoaded)`, never
/// `Found` and never `Missing`; virtual FKs are empty and row estimates absent.
#[test]
fn stub_catalog_is_unknown() {
    for d in [
        DialectId::Postgres,
        DialectId::MySql,
        DialectId::MariaDb,
        DialectId::Sqlite,
    ] {
        let cat = EmptyCatalog::new(IdentCtx::new(d));
        let view: &dyn CatalogView = &cat;
        // No `IdentKey` can be built until Task 3 (`match_key` is the only builder and its stub
        // answers `ServerDependent`), so the key-taking methods are unreachable from callers
        // until then; Task 26's `sql_view` tests cover them. The key-free ones are asserted here.
        assert_eq!(view.generation(), 0);
        assert_eq!(view.ident_ctx().dialect, d);
        assert!(unknown_not_loaded(view.schemas()));
        assert!(unknown_not_loaded(view.databases()));
        assert!(unknown_not_loaded(view.settings()));
        let rel = RelRef {
            schema: Arc::from("public"),
            name: Arc::from("t"),
        };
        assert!(unknown_not_loaded(view.columns(&rel)));
        assert!(unknown_not_loaded(view.foreign_keys(&rel)));
        assert!(view.virtual_fks(&rel).is_empty());
        assert_eq!(view.row_estimate(&rel), None);
    }
}

/// Reasons are machine codes (Fluent ids are built from them in the UI layer), never English
/// text: lower-case ASCII words joined by `-`, unique per enum.
#[test]
fn lookup_reasons_are_codes_not_text() {
    let unknown: Vec<&str> = UnknownReason::ALL.iter().map(|r| r.code()).collect();
    let missing: Vec<&str> = MissingReason::ALL.iter().map(|r| r.code()).collect();
    assert_eq!(
        unknown,
        [
            "not-loaded",
            "loading",
            "out-of-scope",
            "failed",
            "server-dependent"
        ]
    );
    assert_eq!(missing, ["not-found"]);
    for code in unknown.iter().chain(&missing) {
        assert!(!code.is_empty());
        assert!(
            code.bytes().all(|b| b.is_ascii_lowercase() || b == b'-'),
            "{code}"
        );
        assert!(!code.starts_with('-') && !code.ends_with('-'), "{code}");
    }
    let mut all = unknown.clone();
    all.sort_unstable();
    all.dedup();
    assert_eq!(all.len(), unknown.len());
}

/// `FnSig.args = None` means "name only" (MySQL/MariaDB built-ins from the snapshots), which is
/// not the same as "takes no arguments" (`Some(vec![])`).
#[test]
fn fn_sig_args_none_means_name_only() {
    let name_only = FnSig::name_only(Arc::from("mysql"), Arc::from("concat"), FnKind::Function);
    assert!(name_only.args.is_none());
    assert!(name_only.is_name_only());
    let no_args = FnSig {
        args: Some(Vec::new()),
        ..name_only.clone()
    };
    assert!(!no_args.is_name_only());
    assert_ne!(name_only, no_args);
    let one = FnSig {
        args: Some(vec![FnArg::input(Some(Arc::from("x")), Arc::from("text"))]),
        ..name_only
    };
    assert!(!one.is_name_only());
    assert_eq!(one.args.as_ref().map(Vec::len), Some(1));
}
