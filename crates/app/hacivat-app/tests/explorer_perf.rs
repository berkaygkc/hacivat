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

/// Fix round 1 (I3): the UI thread's share of applying a broad filter result. `c0` matches all
/// 150,000 columns (every table and every Columns group is kept); the explorer swaps in a new
/// model generation and flattens it twice (the old expansion, then the revealed one) before a
/// frame builds its rows. Measured cold (a new cache each time): p95 within a 16 ms frame.
#[test]
#[ignore = "perf"]
fn apply_a_broad_filter_within_a_frame() {
    if !perf_enabled() {
        return;
    }
    let (profile, cat) = big();
    let base = model(&profile, &cat, 1);
    let result = Arc::new(base.filter_job("c0", false).run());
    assert_eq!(result.count, 150_000);
    let mut took = Vec::new();
    let mut rows = 0;
    for generation in 2..=22 {
        let t0 = Instant::now();
        let inputs = ExplorerInputs {
            profiles: vec![ProfileInput::new(profile.clone(), Some(cat.clone()))],
            lang: "en".to_string(),
            filter: Some(result.clone()),
            ..ExplorerInputs::default()
        };
        let m = ExplorerModel::build(inputs, Rc::new(ExplorerCache::default()), generation);
        let mut not_loaded = Vec::new();
        let _old = Flat::build(&m, &KeySet::default(), &mut not_loaded);
        let flat = Flat::build(&m, &result.ancestors, &mut not_loaded);
        let shown = flat
            .rows()
            .iter()
            .take(40)
            .filter_map(|r| m.row(r.key))
            .count();
        took.push(t0.elapsed());
        rows = flat.len();
        assert_eq!(shown, 40);
    }
    took.sort();
    let p95 = took[took.len() * 95 / 100 - 1];
    eprintln!(
        "explorer apply of `c0` (150,000 matches, {rows} rows revealed): median {:?}, p95 {p95:?}, max {:?}",
        took[took.len() / 2],
        took.last().unwrap()
    );
    assert!(p95 <= Duration::from_millis(16), "p95 {p95:?}");
}

/// Fix round 1 (I3): a job a newer query took over stops instead of running to its end.
#[test]
#[ignore = "perf"]
fn a_superseded_filter_job_stops_early() {
    if !perf_enabled() {
        return;
    }
    use std::sync::atomic::{AtomicU64, Ordering};
    let (profile, cat) = big();
    let m = model(&profile, &cat, 1);
    let latest = Arc::new(AtomicU64::new(1));
    let job = m.filter_job("c0", false).with_cancel(latest.clone(), 1);
    latest.store(2, Ordering::Relaxed);
    let t0 = Instant::now();
    assert!(
        job.run_cancellable().is_none(),
        "a superseded job gives no result"
    );
    let stopped = t0.elapsed();
    let t1 = Instant::now();
    let _ = m.filter_job("c0", false).run();
    let whole = t1.elapsed();
    eprintln!(
        "explorer filter: a superseded job stopped after {stopped:?} (a whole run {whole:?})"
    );
    assert!(stopped * 10 < whole, "{stopped:?} vs {whole:?}");
}
