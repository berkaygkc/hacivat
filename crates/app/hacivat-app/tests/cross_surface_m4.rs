// ## m4-t32-shots
//! M4 Task 32, cross-surface flows. Here: (d) an EN ↔ TR switch over the explorer keeps its
//! expansion and its cursor, while every M4 row text follows the language.
//!
//! The other flows of the plan's list are covered where their machinery lives (named in the
//! T32 report): (a) offline → cached → connect → stale cleared in `explorer_live.rs`
//! (`offline_profile_shows_cached_with_its_date`) and `refresh_after_ddl_ui.rs`; (b) a
//! console's `CREATE TABLE` reaching the explorer in `refresh_after_ddl_ui.rs`
//! (`create_table_in_a_console_appears_in_the_explorer`); (e) Open data on production without a
//! confirmation in `explorer_actions.rs`
//! (`open_data_on_production_is_a_read_and_needs_no_confirmation`); the offline built-in
//! group and lookups in `offline_explorer.rs`.
//!
//! ## m4-t33b-carries: (c) a user refresh after a privilege loss, the named mass-drop question
//! and Keep cached (`flow_c_privilege_loss_refresh_asks_and_keep_cached_keeps_the_tree`), and
//! (f) an encrypted production profile that reads its key only when expanded
//! (`flow_f_encrypted_production_profile_reads_its_key_only_when_expanded`) live in
//! `explorer_live.rs`, beside the fake catalog source and keyring they drive.

mod support;

use gpui::{TestAppContext, VisualTestContext};
use hacivat_app::connections::explorer;
use hacivat_app::explorer::adapter::{self, NodeInfo};
use hacivat_app::explorer::rows::DisplayGroup;
use hacivat_driver::fake::FakeDriver;
use hacivat_proto::{DbmsKind, EnvTag};
use hacivat_ui::tree::TreeKey;
use support::catalogs::*;
use support::*;

fn strip(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '\u{2068}' | '\u{2069}'))
        .collect()
}

fn key_of(vcx: &VisualTestContext, label: &str) -> TreeKey {
    vcx.run_until_parked();
    vcx.cx
        .read(explorer::shown_rows)
        .into_iter()
        .find(|r| r.status.is_none() && strip(&r.data.label) == label)
        .map(|r| r.key)
        .unwrap_or_else(|| panic!("`{label}` is shown"))
}

fn tables_key(vcx: &VisualTestContext) -> TreeKey {
    vcx.run_until_parked();
    vcx.cx.read(|cx| {
        let tree = explorer::tree(cx).expect("the tree");
        let tree = tree.read(cx);
        explorer::shown_rows(cx)
            .into_iter()
            .find(|r| {
                matches!(
                    tree.model().info(r.key),
                    Some(NodeInfo::Group {
                        group: DisplayGroup::Tables,
                        ..
                    })
                )
            })
            .map(|r| r.key)
            .expect("the TABLES group")
    })
}

fn expand(vcx: &VisualTestContext, key: TreeKey) {
    vcx.cx.update(|cx| {
        let tree = explorer::tree(cx).expect("the tree");
        tree.update(cx, |t, cx| t.set_expanded(key, true, cx));
    });
    vcx.run_until_parked();
}

/// What the switch must keep: the expanded rows shown and the cursor.
fn state(vcx: &VisualTestContext) -> (Vec<TreeKey>, Option<TreeKey>) {
    vcx.run_until_parked();
    vcx.cx.read(|cx| {
        let tree = explorer::tree(cx).expect("the tree");
        let tree = tree.read(cx);
        let expanded = explorer::shown_rows(cx)
            .into_iter()
            .filter(|r| tree.is_expanded(r.key))
            .map(|r| r.key)
            .collect();
        (expanded, tree.cursor())
    })
}

fn group_label(vcx: &VisualTestContext) -> String {
    let key = tables_key(vcx);
    vcx.cx
        .read(explorer::shown_rows)
        .into_iter()
        .find(|r| r.key == key)
        .map(|r| strip(&r.data.label))
        .expect("the TABLES row")
}

#[gpui::test]
fn a_language_switch_keeps_expansion_and_cursor_and_follows_on_every_m4_row(
    cx: &mut TestAppContext,
) {
    let vcx = shell_with_fake(cx, "en", answers_probe(FakeDriver::new(DbmsKind::Postgres)));
    let mut p = test_profile("shop", Some(EnvTag::Dev), "127.0.0.1");
    p.database = Some("app".into());
    let id = add_profile(vcx, p);
    let cat = catalog(
        DbmsKind::Postgres,
        vec![
            schema(
                DbmsKind::Postgres,
                &["app", "public"],
                false,
                vec![
                    table("orders", Some(16_400), &[("id", "bigint")]),
                    table("müşteri", Some(16_410), &[("İl", "text")]),
                ],
            ),
            schema(
                DbmsKind::Postgres,
                &["app", "pg_catalog"],
                true,
                vec![table("pg_class", Some(1259), &[])],
            ),
        ],
    );
    vcx.cx
        .update(|cx| adapter::set_catalog(id, Some(cat), None, cx));
    for label in ["shop", "app", "public"] {
        let k = key_of(vcx, label);
        expand(vcx, k);
    }
    let tables = tables_key(vcx);
    expand(vcx, tables);
    let turkish = key_of(vcx, "müşteri");
    expand(vcx, turkish);
    vcx.cx.update(|cx| {
        let tree = explorer::tree(cx).expect("the tree");
        tree.update(cx, |t, cx| t.select(turkish, cx));
    });
    let before = state(vcx);
    assert_eq!(before.1, Some(turkish));
    assert_eq!(group_label(vcx), "TABLES");
    for (lang, tables) in [("tr", "TABLOLAR"), ("en", "TABLES")] {
        switch_to(vcx, lang);
        vcx.run_until_parked();
        assert_eq!(state(vcx), before, "{lang}: expansion and cursor kept");
        assert_eq!(group_label(vcx), tables, "{lang}");
        // A catalog name stays verbatim in either language.
        assert_eq!(key_of(vcx, "müşteri"), turkish);
    }
    stop_core_runtime_test(cx);
}
