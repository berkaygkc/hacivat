//! The explorer's budgets (plan-m4 Task 14; release, `HACIVAT_PERF=1 cargo test --release -p
//! hacivat-app --test explorer_perf -- --ignored`), at the HV-MODEL-03 scale: one PostgreSQL
//! schema of 15,000 tables and 150,000 columns (165,000 names).
//!
//! - **Filter** (D-074): the UI thread's share of a keystroke (making the job from the model)
//!   stays under 1 ms, and the job, which runs off the UI thread at that size, gives its result
//!   in under 100 ms (median of the queries below).
//! - **Rows**: a new model generation, expanded to the schema's "Tables" group (15,000 rows,
//!   sorted by the UI collation, keyed, flattened) and the rows of one screen built, stays
//!   under one 16 ms frame; the sort is measured cold (nothing cached).

mod support;

use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hacivat_app::explorer::adapter::{
    ExplorerCache, ExplorerInputs, ExplorerModel, ProfileInput, SYNC_FILTER_NAMES, database_key,
    group_key, namespace_key, profile_key,
};
use hacivat_app::explorer::rows::DisplayGroup;
use hacivat_meta::{CatalogSnapshot, NsKey};
use hacivat_proto::DbmsKind;
use hacivat_session::profile::Profile;
use hacivat_ui::tree::flat::Flat;
use hacivat_ui::tree::{KeySet, TreeModel};
use support::catalogs::{catalog, wide_pg_schema};

fn perf_enabled() -> bool {
    std::env::var_os("HACIVAT_PERF").is_some_and(|v| v == "1")
}

/// 15,000 tables of 10 columns in `app.public`.
fn big() -> (Arc<Profile>, Arc<CatalogSnapshot>) {
    let profile = Arc::new(Profile {
        name: "big".to_string(),
        engine: DbmsKind::Postgres,
        database: Some("app".to_string()),
        ..Profile::default()
    });
    let ns = wide_pg_schema("app", "public", 15_000, 10);
    (profile, catalog(DbmsKind::Postgres, vec![ns]))
}

fn model(profile: &Arc<Profile>, cat: &Arc<CatalogSnapshot>, generation: u64) -> ExplorerModel {
    let inputs = ExplorerInputs {
        profiles: vec![ProfileInput::new(profile.clone(), Some(cat.clone()))],
        lang: "en".to_string(),
        ..ExplorerInputs::default()
    };
    ExplorerModel::build(inputs, Rc::new(ExplorerCache::default()), generation)
}

fn median(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v[v.len() / 2]
}

#[test]
#[ignore = "perf"]
fn filter_160k_names_off_ui_thread() {
    if !perf_enabled() {
        return;
    }
    let (profile, cat) = big();
    let m = model(&profile, &cat, 1);
    let queries = ["t1234", "c05", "zz", "t0", "public", "t14999", "C0", "x"];
    let mut ui = Vec::new();
    let mut job = Vec::new();
    for q in queries {
        let t0 = Instant::now();
        let j = m.filter_job(q, true);
        ui.push(t0.elapsed());
        assert!(j.names() > SYNC_FILTER_NAMES, "{} names", j.names());
        // The job runs on another thread (the background executor's), as the explorer does.
        let (result, took) = std::thread::spawn(move || {
            let t0 = Instant::now();
            let r = j.run();
            (r, t0.elapsed())
        })
        .join()
        .expect("the filter job");
        assert!(
            result.count > 0 || q == "zz" || q == "x",
            "{q}: {}",
            result.count
        );
        job.push(took);
    }
    let (ui_p50, job_p50) = (median(ui.clone()), median(job.clone()));
    eprintln!(
        "explorer filter over 165,000 names: UI share median {ui_p50:?} (max {:?}), result median {job_p50:?} (max {:?})",
        ui.iter().max().unwrap(),
        job.iter().max().unwrap()
    );
    assert!(ui_p50 <= Duration::from_millis(1), "UI share {ui_p50:?}");
    assert!(job_p50 <= Duration::from_millis(100), "result {job_p50:?}");
}

#[test]
#[ignore = "perf"]
fn build_rows_for_15k_tables_under_16ms() {
    if !perf_enabled() {
        return;
    }
    let (profile, cat) = big();
    let id = profile.id;
    let root = profile_key(id);
    let db = database_key(id, "app");
    let schema = namespace_key(id, &NsKey::schema(["app", "public"]));
    let tables = group_key(schema, DisplayGroup::Tables);
    let expanded: KeySet = [root, db, schema, tables].into_iter().collect();
    let mut took = Vec::new();
    for generation in 1..=7 {
        let t0 = Instant::now();
        // A cold cache every time: the sort and the keys are measured.
        let m = model(&profile, &cat, generation);
        let mut not_loaded = Vec::new();
        let flat = Flat::build(&m, &expanded, &mut not_loaded);
        let shown: usize = flat
            .rows()
            .iter()
            .take(40)
            .filter_map(|r| m.row(r.key))
            .count();
        took.push(t0.elapsed());
        assert_eq!(
            flat.len(),
            5 + 15_000,
            "a header, the root, app, public, TABLES and every table"
        );
        assert_eq!(shown, 40);
        assert!(not_loaded.is_empty());
    }
    let p50 = median(took.clone());
    eprintln!(
        "explorer rows for 15,000 tables: median {p50:?} (max {:?})",
        took.iter().max().unwrap()
    );
    assert!(p50 <= Duration::from_millis(16), "{p50:?}");
}
