//! M4 Task 1a seams of `hacivat-ddl`: the renderers refuse until Task 7, the producers until
//! Task 19, and the IR declares every alg/03 HV-DDL-07 variant.

use hacivat_ddl::ir::{AlterAction, DdlOp};
use hacivat_ddl::options::GenOptions;
use hacivat_ddl::render::{RenderCtx, RenderError, renderer};
use hacivat_ddl::{DdlError, DdlTarget};
use sql_dialect::{DialectId, ServerVersion};

fn ctx(d: DialectId) -> RenderCtx {
    RenderCtx::new(
        DdlTarget::new(d, ServerVersion::new(17, 0, 0)),
        GenOptions::default(),
    )
}

#[test]
fn stub_renderers_refuse() {
    for d in [
        DialectId::Postgres,
        DialectId::MySql,
        DialectId::MariaDb,
        DialectId::Sqlite,
    ] {
        for op in DdlOp::samples() {
            assert!(
                matches!(
                    renderer(d).render(&op, &ctx(d)),
                    Err(RenderError::Unsupported { .. })
                ),
                "{d:?} {}",
                op.variant()
            );
        }
        assert!(matches!(
            renderer(d).render(&DdlOp::sample_create_table(), &ctx(d)),
            Err(RenderError::Unsupported { .. })
        ));
    }
}

/// Every alg/03 HV-DDL-07 `DdlOp` variant is declared: the exhaustive `match` below fails to
/// compile when one is missing or renamed, and `samples()` builds each one once.
#[test]
fn every_alg03_variant_is_declared() {
    fn name(op: &DdlOp) -> &'static str {
        match op {
            DdlOp::CreateSchema(_) => "CreateSchema",
            DdlOp::CreateTable(_) => "CreateTable",
            DdlOp::AlterTable { .. } => "AlterTable",
            DdlOp::DropObject { .. } => "DropObject",
            DdlOp::CreateIndex(_) => "CreateIndex",
            DdlOp::CreateView(_) => "CreateView",
            DdlOp::CreateRoutine(_) => "CreateRoutine",
            DdlOp::CreateTrigger(_) => "CreateTrigger",
            DdlOp::CreateSequence(_) => "CreateSequence",
            DdlOp::CreateType(_) => "CreateType",
            DdlOp::Rename { .. } => "Rename",
            DdlOp::SetSchema { .. } => "SetSchema",
            DdlOp::Comment { .. } => "Comment",
            DdlOp::Grant(_) => "Grant",
            DdlOp::Revoke(_) => "Revoke",
            DdlOp::Owner { .. } => "Owner",
            DdlOp::RowSecurity { .. } => "RowSecurity",
            DdlOp::TableRebuild(_) => "TableRebuild",
            DdlOp::Exchange { .. } => "Exchange",
            DdlOp::Raw { .. } => "Raw",
            DdlOp::Data(_) => "Data",
        }
    }
    let samples = DdlOp::samples();
    let names: Vec<&str> = samples.iter().map(name).collect();
    assert_eq!(
        names,
        [
            "CreateSchema",
            "CreateTable",
            "AlterTable",
            "DropObject",
            "CreateIndex",
            "CreateView",
            "CreateRoutine",
            "CreateTrigger",
            "CreateSequence",
            "CreateType",
            "Rename",
            "SetSchema",
            "Comment",
            "Grant",
            "Revoke",
            "Owner",
            "RowSecurity",
            "TableRebuild",
            "Exchange",
            "Raw",
            "Data",
        ]
    );
    for op in &samples {
        assert_eq!(op.variant(), name(op));
    }
    // The HV-DDL-05 alter actions of v1 (column add/drop/rename/type/nullability/default/
    // comment; constraint add/drop/validate).
    assert_eq!(AlterAction::samples().len(), 10);
}

/// The view-only producer refuses until Task 19 (no script that is not the object's).
#[test]
fn stub_create_script_refuses() {
    use hacivat_ddl::produce::create_script;
    use hacivat_meta::ids::ObjectKey;
    use hacivat_meta::name::IdentCtx;
    use hacivat_meta::snapshot::{Catalog, ServerFacts};
    use hacivat_meta::view::SnapshotView;
    use std::sync::Arc;

    let catalog = Catalog::new(Arc::new(ServerFacts::new(
        hacivat_proto::DbmsKind::Postgres,
        IdentCtx::new(DialectId::Postgres),
    )));
    let view = SnapshotView::new(catalog.current());
    let key = ObjectKey {
        kind: hacivat_meta::kind::ObjectKind::Table,
        path: smallvec::smallvec![Arc::from("db"), Arc::from("public"), Arc::from("t")],
        server_id: None,
        overload_sig: None,
        raw_kind: None,
    };
    let target = DdlTarget::new(DialectId::Postgres, ServerVersion::new(17, 0, 0));
    assert!(matches!(
        create_script(&view, &[key], &target, &GenOptions::default()),
        Err(DdlError::NotImplemented)
    ));
}
