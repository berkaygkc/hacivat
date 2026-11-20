//! Dependency guard: transitive GPUI/zed reach, crate layers and workspace lints.
//!
//! Only `hacivat-ui-core` (the adapter) may reach `gpui`, `gpui_platform` or any zed-sourced
//! crate. F1 exception: a dev-only dependency on `gpui` with `test-support`, owned by a Ui or
//! App workspace crate (at any depth of the walk, since that crate is checked on its own),
//! because `#[gpui::test]` expands to hard-coded `gpui::` paths. Spike, Tool and Core owners
//! get no exception. Layers fail closed: a member outside the known directories is flagged.
//! Core crates may reach no Ui crate; Ui crates (adapter included) may reach no Core crate.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::path::Path;
use std::process::ExitCode;

use cargo_metadata::{DependencyKind, Metadata, MetadataCommand, Package, PackageId};

const ADAPTER: &str = "hacivat-ui-core";
const GUARDED: [&str; 2] = ["gpui", "gpui_platform"];
/// Adapter features that must never be enabled by a product build (F33, R12).
const ADAPTER_ONLY_FEATURES: [&str; 4] = ["spike-support", "bench", "scene-check", "test-support"];
const PRODUCT_CRATES: [&str; 11] = [
    "hacivat",
    "hacivat-app",
    // ## m3a-t01b-seams (P22): the new Ui and App crates ship in the product too.
    "hacivat-ui-editor",
    "hacivat-ui-grid",
    "hacivat-sql-editor",
    "hacivat-text",
    "hacivat-ui",
    // ## m4-t01a-model: the M4 core and SQL crates.
    "hacivat-meta",
    "hacivat-introspect",
    "hacivat-ddl",
    "sql-catalog",
];

/// Sources of the zed workspace; any crate fetched from one counts as zed-sourced.
pub const ZED_SOURCES: [&str; 2] = [
    "https://github.com/zed-industries/zed",
    "https://github.com/berkaygkc/zed",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Rule {
    ZedOutsideAdapter,
    CoreReachesUi,
    UiDependsOnCore,
    AdapterFeatureInProduct,
    MissingWorkspaceLints,
    /// The manifest lives outside every known layer directory.
    UnknownLayer,
    // ## m2b-t01-interfaces
    /// `hacivat-guard` reaches a runtime crate through its normal dependencies (§5.1 rule 3).
    GuardNotRuntimeFree,
    // ## m3a-t01b-seams
    /// A `crates/sql/*` crate (but the dev-only `sql-testkit`) reaches a runtime, `salsa`,
    /// `sql-sema`, `hacivat-l10n` or GPUI through its normal dependencies.
    SqlNotRuntimeFree,
    /// A `crates/sql/*` crate depends on a `crates/core/*` crate (any edge, dev included).
    SqlReachesCore,
    /// A crate other than `sql-oracle-pg` depends on `pg_query` directly, or (M3b) a crate
    /// other than `sql-safety` depends on `sqlparser` directly; any edge kind counts.
    BackendOutsideOwner,
    /// `hacivat-text` (the headless editor core) reaches `hacivat-ui-core`, GPUI or a zed
    /// crate through its normal dependencies.
    TextCoreNotHeadless,
    /// A member depends on `ropey` with its default features or `unicode_lines`, a member's
    /// `[features]` can turn `ropey/unicode_lines` (or `ropey/default`) on, or the ropey
    /// `hacivat-text` resolves to has `unicode_lines` in the workspace's resolve: feature
    /// unification would then make U+2028, U+2029, NEL, VT and FF line breaks in `hacivat-text`
    /// whenever that member is built too (fix round 1; CI run 43545681877). Every member uses
    /// `ropey.workspace = true`.
    ///
    /// Limits (review round 2): the resolve is `cargo metadata`'s, i.e. the members' default
    /// features over the whole workspace, which is what `cargo build`/`test --workspace` build;
    /// a feature that reaches ropey only through a non-member's optional feature is not seen,
    /// and the declaration check matches the crate name (another ropey major with its defaults
    /// is flagged too: fail closed).
    RopeLineBreaks,
    // ## m4-t01a-model
    /// `hacivat-ddl`'s graph on its **default features** (feature-resolved from the manifests,
    /// not the workspace's unified resolve) reaches a runtime, a session, a driver,
    /// introspection or l10n (Karagöz links it; alg/03 "pure functions over snapshots").
    DdlIsPure,
    /// A member other than `hacivat-app` (and `hacivat-ddl` itself) enables `hacivat-ddl/apply`
    /// through a dependency of any kind or one of its `[features]` (R-M4-35).
    DdlApplyOnlyFromApp,
    /// `hacivat-meta` depends on introspection, a session, a driver, exec or l10n: directly
    /// through any edge kind (dev and build included), or transitively through normal edges.
    MetaBelowIntrospect,
    /// A member other than `hacivat-app` (and `hacivat-meta` itself) enables
    /// `hacivat-meta/embedded-system` (the ~0.7 MB of snapshots lands in the app only).
    EmbeddedSystemOnlyFromApp,
    /// `hacivat-introspect` depends on `hacivat-exec` (any edge kind directly, or transitively
    /// through normal edges) or directly on `hacivat-results`: catalog SQL never travels the
    /// execution path (R-M4-7). `hacivat-session`'s own result store is not an edge of
    /// introspection's.
    IntrospectNotExec,
    /// A source line names `MetaQuery::new` outside `hacivat-introspect/src/pack/**` and
    /// `hacivat-session/src/meta.rs` (tests under a crate's `tests/` are exempt; R-M4-7, I13).
    MetaQueryOnlyFromPacks,
    /// A `src/` file other than its definition (`hacivat-ui-editor/src/find.rs`) names
    /// `hold_counts(`: the find counter hold is a shots-harness seam (M3c carry, P-M4-17).
    HoldCountsTestOnly,
    // ## m4-t04-packs
    /// A source file names `Pack::parse` or `Pack::parse_all` (the constructors of a pack
    /// whose queries become a `MetaQuery`) outside `hacivat-introspect/src/pack/**`, the
    /// introspector modules (`src/{pg,mysql,sqlite}/**`), a crate's `tests/` and
    /// `fuzz/meta/`; or aliases `Pack`, names it in an `impl` header or a macro invocation
    /// (R-M4-7, T4 fix round 1 I2). `Pack::builtin` stays legal everywhere.
    PackParseOnlyFromPacks,
    // ## m4-hyg-tmp
    /// A source file (any crate, `src/` or `tests/`, `fuzz/`, `xtask/`) lets a `tempfile` guard
    /// outlive its test, so the directory is never removed (R-W5-15; ~58k leaked a day): it
    /// calls `.keep()`, `.into_path()` or `.disable_cleanup(…)`; leaks or forgets a statement
    /// that names a guard (`Box::leak`, `mem::forget`); holds a guard in a process-wide
    /// `static` (never dropped; a `thread_local!` is dropped when the test's thread ends); or
    /// holds one in a type that is a GPUI `Global` (the test harness never drops the app).
    /// See [`leaks_a_temp_dir`] for the token forms.
    TempDirOutlivesTest,
}

// ## m2b-t01-interfaces
/// The sans-I/O read-only guard.
const GUARD: &str = "hacivat-guard";
/// What the guard's normal-dependency closure may not contain (§5.1 rule 3).
const GUARD_FORBIDDEN: [&str; 6] = [
    "tokio",
    "futures",
    "salsa",
    "hacivat-l10n",
    "sql-sema",
    "gpui",
];

/// The guard rule: a breadth-first walk over normal edges only (dev and build dependencies may
/// use a runtime); the first path to a forbidden crate is reported.
fn guard_violation(
    root: &PackageId,
    edges: &Edges<'_>,
    pkgs: &HashMap<&PackageId, &Package>,
) -> Option<Vec<String>> {
    normal_path_to(root, edges, pkgs, |p| {
        GUARD_FORBIDDEN.contains(&p.name.as_str())
    })
}

/// A breadth-first walk from `root` over normal edges only: the path to the first package
/// `bad` accepts (excluding `root`), if any.
fn normal_path_to(
    root: &PackageId,
    edges: &Edges<'_>,
    pkgs: &HashMap<&PackageId, &Package>,
    bad: impl Fn(&Package) -> bool,
) -> Option<Vec<String>> {
    let mut seen: BTreeSet<&PackageId> = BTreeSet::from([root]);
    let mut queue: VecDeque<(&PackageId, Vec<String>)> = VecDeque::from([(root, vec![])]);
    while let Some((id, path)) = queue.pop_front() {
        for (dep_id, kinds) in edges.get(id).into_iter().flatten() {
            if !kinds.contains(&DependencyKind::Normal) {
                continue;
            }
            let Some(dep) = pkgs.get(dep_id) else {
                continue;
            };
            let mut via = path.clone();
            via.push(dep.name.to_string());
            if bad(dep) {
                return Some(via);
            }
            if seen.insert(dep_id) {
                queue.push_back((dep_id, via));
            }
        }
    }
    None
}

// ## m3a-t01b-seams
/// What a `crates/sql/*` crate's normal-dependency closure may not contain: the guard list
/// plus the other async runtimes (§5.1; P22 keeps `salsa` and `sql-sema` for every M3a crate,
/// ADR 0005 names M5's exemptions).
const SQL_FORBIDDEN: [&str; 8] = [
    "tokio",
    "futures",
    "salsa",
    "hacivat-l10n",
    "sql-sema",
    "gpui",
    "async-std",
    "smol",
];
/// The dev-only SQL corpus crate: `SqlNotRuntimeFree` does not apply to it.
const SQL_TESTKIT: &str = "sql-testkit";
/// Each parser backend and the one crate that may depend on it directly: `pg_query` only in
/// `sql-oracle-pg` (R-M3-1); ## m3b-t01a-seams: `sqlparser` only in `sql-safety` (R-M3-54).
const BACKENDS: [(&str, &str); 2] = [("pg_query", "sql-oracle-pg"), ("sqlparser", "sql-safety")];
/// The headless editor core (`TextCoreNotHeadless`).
const TEXT_CORE: &str = "hacivat-text";
/// The rope under `hacivat-text`, and the features that change its line breaks.
const ROPE: &str = "ropey";
const ROPE_LINE_FEATURES: [&str; 2] = ["unicode_lines", "default"];

// ## m4-t01a-model
/// The pure DDL crate (`DdlIsPure`), and the one feature that may take it out of pure code.
const DDL: &str = "hacivat-ddl";
const DDL_APPLY: &str = "apply";
/// The Hacivat crates `hacivat-ddl` may never reach on any non-`apply` feature.
const DDL_FORBIDDEN: [&str; 3] = ["hacivat-session", "hacivat-introspect", "hacivat-l10n"];
/// Async runtimes and executors outside the `tokio*` / `futures*` families (fix round 1, m6).
const ASYNC_RUNTIMES: [&str; 6] = [
    "async-std",
    "smol",
    "async-executor",
    "async-global-executor",
    "async-io",
    "mio",
];
/// The metadata model and what it may never depend on (`MetaBelowIntrospect`).
const META: &str = "hacivat-meta";
const META_FORBIDDEN: [&str; 4] = [
    "hacivat-introspect",
    "hacivat-session",
    "hacivat-exec",
    "hacivat-l10n",
];
const EMBEDDED_SYSTEM: &str = "embedded-system";
/// Introspection and the execution crates it never depends on (`IntrospectNotExec`).
const INTROSPECT: &str = "hacivat-introspect";
const EXEC: &str = "hacivat-exec";
const RESULTS: &str = "hacivat-results";
/// The one crate (besides the feature's owner) that may enable `apply` / `embedded-system`.
const APP: &str = "hacivat-app";
/// Every dependency kind (a root's dev and build edges count for the M4 rules).
const ALL_KINDS: [DependencyKind; 3] = [
    DependencyKind::Normal,
    DependencyKind::Development,
    DependencyKind::Build,
];

/// Whether `name` is a driver crate (`hacivat-driver`, `hacivat-driver-pg`, …).
fn is_driver(name: &str) -> bool {
    name == "hacivat-driver" || name.starts_with("hacivat-driver-")
}

/// What `hacivat-ddl` may not reach: any `tokio*` or `futures*` crate (runtimes, executors and
/// their pieces), the other async runtimes, sessions, drivers, introspection and l10n
/// (fix round 1, m6: a rule, not a list of exact names).
fn ddl_forbidden(name: &str) -> bool {
    name.starts_with("tokio")
        || name == "futures"
        || name.starts_with("futures-")
        || ASYNC_RUNTIMES.contains(&name)
        || DDL_FORBIDDEN.contains(&name)
        || is_driver(name)
}

fn meta_forbidden(name: &str) -> bool {
    META_FORBIDDEN.contains(&name) || is_driver(name)
}

/// A breadth-first walk that follows every edge kind out of `root` and only normal edges
/// beyond it: the path to the first package `bad` accepts (excluding `root`), if any.
fn any_then_normal_path_to(
    root: &PackageId,
    edges: &Edges<'_>,
    pkgs: &HashMap<&PackageId, &Package>,
    bad: impl Fn(&Package) -> bool,
) -> Option<Vec<String>> {
    let mut seen: BTreeSet<&PackageId> = BTreeSet::from([root]);
    let mut queue: VecDeque<(&PackageId, Vec<String>)> = VecDeque::from([(root, vec![])]);
    while let Some((id, path)) = queue.pop_front() {
        for (dep_id, kinds) in edges.get(id).into_iter().flatten() {
            if id != root && !kinds.contains(&DependencyKind::Normal) {
                continue;
            }
            let Some(dep) = pkgs.get(dep_id) else {
                continue;
            };
            let mut via = path.clone();
            via.push(dep.name.to_string());
            if bad(dep) {
                return Some(via);
            }
            if seen.insert(dep_id) {
                queue.push_back((dep_id, via));
            }
        }
    }
    None
}

/// How a feature walk starts.
struct WalkStart<'s> {
    /// The features turned on at the root (`default` is not implied).
    features: BTreeSet<String>,
    /// The edge kinds followed out of the root; beyond it, normal edges only.
    root_kinds: &'s [DependencyKind],
    /// A root feature that is never turned on, even when another root feature names it
    /// (`DdlIsPure`'s `apply`).
    blocked: Option<&'s str>,
}

impl<'s> WalkStart<'s> {
    /// The root with `default` and every feature it declares, through `kinds`.
    fn all_features(root: &Package, kinds: &'s [DependencyKind], blocked: Option<&'s str>) -> Self {
        let mut features: BTreeSet<String> = root.features.keys().cloned().collect();
        features.insert("default".into());
        if let Some(b) = blocked {
            features.remove(b);
        }
        WalkStart {
            features,
            root_kinds: kinds,
            blocked,
        }
    }
}

/// What a feature walk found.
struct Walked<'a> {
    /// The closed feature set of every package reached (its own `[features]` applied).
    features: HashMap<&'a PackageId, BTreeSet<String>>,
    /// The path to the first dependency whose package name `bad` accepts, if any.
    bad_path: Option<Vec<String>>,
}

/// The M4 rules' walk: features resolved from the manifests, not from the workspace's
/// unified resolve.
///
/// The unified resolve merges every member's features (`hacivat-introspect` turns on
/// `hacivat-meta`'s defaults, so the resolved `hacivat-meta` node has its `tokio` edge), and it
/// has no edge at all for an optional dependency that no member turns on. This walk instead
/// starts from `start`'s features and follows a dependency when it is not optional or an
/// enabled feature of its owner activates it (`dep:x`, `x`, `x/f`), with the features its edge
/// and the enabling features ask (plus `default` unless the edge turns default features off).
/// Renamed dependencies are matched by their key in `[features]`; target-specific edges count
/// (fail closed). `bad` is asked of every activated dependency **by name**, so a dependency the
/// resolve never built is still seen; packages are found through the resolve's edges, or by
/// name when the resolve has no edge for them. `skip` names packages the walk does not enter.
fn feature_walk<'a>(
    root: &'a PackageId,
    edges: &Edges<'a>,
    pkgs: &HashMap<&'a PackageId, &'a Package>,
    start: WalkStart<'_>,
    skip: impl Fn(&str) -> bool,
    bad: impl Fn(&str) -> bool,
) -> Walked<'a> {
    let by_name = |n: &str| -> Option<&'a PackageId> {
        pkgs.iter()
            .find(|(_, p)| p.name.as_str() == n)
            .map(|(id, _)| *id)
    };
    let mut asked_of: HashMap<&'a PackageId, BTreeSet<String>> = HashMap::new();
    let mut closed: HashMap<&'a PackageId, BTreeSet<String>> = HashMap::new();
    let mut paths: HashMap<&'a PackageId, Vec<String>> = HashMap::new();
    let mut bad_path: Option<Vec<String>> = None;
    asked_of.insert(root, start.features.clone());
    paths.insert(root, Vec::new());
    let mut queue: VecDeque<&'a PackageId> = VecDeque::from([root]);
    while let Some(id) = queue.pop_front() {
        let Some(pkg) = pkgs.get(id) else {
            continue;
        };
        let at_root = id == root;
        let blocked = if at_root { start.blocked } else { None };
        // The feature closure of `pkg`, the optional dependencies it activates and the
        // features it asks of each dependency.
        let mut on: BTreeSet<String> = asked_of.get(id).cloned().unwrap_or_default();
        if let Some(b) = blocked {
            on.remove(b);
        }
        let mut todo: Vec<String> = on.iter().cloned().collect();
        let mut active: BTreeSet<String> = BTreeSet::new();
        let mut asked: HashMap<String, BTreeSet<String>> = HashMap::new();
        let mut weak: Vec<(String, String)> = Vec::new();
        let is_optional_dep = |n: &str| {
            pkg.dependencies
                .iter()
                .any(|d| d.optional && dep_key(d) == n)
        };
        while let Some(f) = todo.pop() {
            let Some(items) = pkg.features.get(&f) else {
                if is_optional_dep(&f) {
                    active.insert(f);
                }
                continue;
            };
            for item in items {
                if let Some(d) = item.strip_prefix("dep:") {
                    active.insert(d.to_string());
                } else if let Some((d, feat)) = item.split_once('/') {
                    if let Some(d) = d.strip_suffix('?') {
                        weak.push((d.to_string(), feat.to_string()));
                    } else {
                        active.insert(d.to_string());
                        asked
                            .entry(d.to_string())
                            .or_default()
                            .insert(feat.to_string());
                    }
                } else if pkg.features.contains_key(item) {
                    if Some(item.as_str()) != blocked && on.insert(item.clone()) {
                        todo.push(item.clone());
                    }
                } else if is_optional_dep(item) {
                    active.insert(item.clone());
                }
            }
        }
        for (d, feat) in weak {
            if active.contains(&d)
                || pkg
                    .dependencies
                    .iter()
                    .any(|x| !x.optional && dep_key(x) == d)
            {
                asked.entry(d).or_default().insert(feat);
            }
        }
        closed.insert(id, on);
        let path = paths.get(id).cloned().unwrap_or_default();
        let kinds: &[DependencyKind] = if at_root {
            start.root_kinds
        } else {
            &[DependencyKind::Normal]
        };
        for d in &pkg.dependencies {
            if !kinds.contains(&d.kind) || (d.optional && !active.contains(dep_key(d))) {
                continue;
            }
            let mut via = path.clone();
            via.push(d.name.clone());
            if bad_path.is_none() && bad(&d.name) {
                bad_path = Some(via.clone());
            }
            if skip(&d.name) {
                continue;
            }
            let resolved = edges
                .get(id)
                .into_iter()
                .flatten()
                .map(|(dep_id, _)| *dep_id)
                .find(|dep_id| pkgs.get(dep_id).is_some_and(|p| p.name == d.name));
            let Some(dep_id) = resolved.or_else(|| by_name(&d.name)) else {
                continue;
            };
            let mut want: BTreeSet<String> = d.features.iter().cloned().collect();
            if d.uses_default_features {
                want.insert("default".into());
            }
            if let Some(extra) = asked.get(dep_key(d)) {
                want.extend(extra.iter().cloned());
            }
            let have = asked_of.entry(dep_id).or_default();
            let before = have.len();
            have.extend(want);
            let first = !paths.contains_key(dep_id);
            if first {
                paths.insert(dep_id, via);
            }
            if first || have.len() != before {
                queue.push_back(dep_id);
            }
        }
    }
    Walked {
        features: closed,
        bad_path,
    }
}

/// Whether `root`, built with every feature it declares (dev and build edges included, and not
/// through `hacivat-app`, the one crate allowed to), turns on `owner`'s `feature`: directly,
/// through a renamed dependency, through one of its own features (`default` included), or
/// through an alias feature `owner` declares (fix round 1, I1).
fn turns_on(
    root: &Package,
    edges: &Edges<'_>,
    pkgs: &HashMap<&PackageId, &Package>,
    owner: &str,
    feature: &str,
) -> bool {
    let walked = feature_walk(
        &root.id,
        edges,
        pkgs,
        WalkStart::all_features(root, &ALL_KINDS, None),
        |n| n == APP,
        |_| false,
    );
    walked
        .features
        .iter()
        .any(|(id, on)| pkgs.get(id).is_some_and(|p| p.name.as_str() == owner) && on.contains(feature))
        // A declared edge that asks for the feature by name, even one the resolve never built
        // (an owner that is not in this metadata, or a dormant optional edge).
        || root
            .dependencies
            .iter()
            .any(|d| d.name == owner && d.features.iter().any(|f| f == feature))
}

/// The features of `pkg` that `feature` turns on through `pkg`'s own `[features]` table
/// (plain feature names only; `dep:` and `x/f` items name other packages).
fn own_closure(pkg: &Package, feature: &str) -> BTreeSet<String> {
    let mut on = BTreeSet::from([feature.to_string()]);
    let mut todo = vec![feature.to_string()];
    while let Some(f) = todo.pop() {
        for item in pkg.features.get(&f).into_iter().flatten() {
            if pkg.features.contains_key(item) && on.insert(item.clone()) {
                todo.push(item.clone());
            }
        }
    }
    on
}

/// The name a dependency is known by inside its owner's `[features]` (its rename, if any).
fn dep_key(d: &cargo_metadata::Dependency) -> &str {
    d.rename.as_deref().unwrap_or(&d.name)
}

/// The workspace-relative directory of `p`'s manifest, with `/` separators.
fn rel_dir(md: &Metadata, p: &Package) -> String {
    let abs = p.manifest_path.as_std_path();
    let rel = abs
        .strip_prefix(md.workspace_root.as_std_path())
        .unwrap_or(abs);
    rel.to_string_lossy().replace('\\', "/")
}

/// Whether `dep` is a Core crate for `SqlReachesCore`: a member under `crates/core/`, or (failing
/// closed) any `hacivat-*` package that is not a Ui or App member.
fn is_core_crate(md: &Metadata, dep: &Package, layers: &HashMap<&PackageId, Layer>) -> bool {
    if rel_dir(md, dep).starts_with("crates/core/") {
        return true;
    }
    dep.name.starts_with("hacivat-") && !matches!(layers.get(&dep.id), Some(Layer::Ui | Layer::App))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub krate: String,
    pub rule: Rule,
    /// Packages walked from the crate to the offender, excluding the crate itself.
    pub via: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Layer {
    Ui,
    Core,
    App,
    Spike,
    Tool,
}

fn layer_of(md: &Metadata, p: &Package) -> Option<Layer> {
    let abs = p.manifest_path.as_std_path();
    let rel = abs
        .strip_prefix(md.workspace_root.as_std_path())
        .unwrap_or(abs);
    let rel = rel.to_string_lossy().replace('\\', "/");
    Some(if rel.starts_with("crates/ui/") {
        Layer::Ui
    } else if ["crates/core/", "crates/sql/", "crates/ai/"]
        .iter()
        .any(|d| rel.starts_with(d))
    {
        Layer::Core
    } else if rel.starts_with("crates/app/") || rel.starts_with("crates/bin/") {
        Layer::App
    } else if rel.starts_with("spikes/") {
        Layer::Spike
    } else if rel.starts_with("xtask/") {
        Layer::Tool
    } else {
        return None;
    })
}

fn is_zed_sourced(p: &Package) -> bool {
    let Some(src) = p.source.as_ref().and_then(|s| s.repr.strip_prefix("git+")) else {
        return false;
    };
    ZED_SOURCES.iter().any(|z| {
        src.strip_prefix(z)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(['?', '#', '/', '.']))
    })
}

fn is_guarded(p: &Package) -> bool {
    GUARDED.contains(&p.name.as_str()) || is_zed_sourced(p)
}

/// F1: a direct dev-only edge on `gpui` that enables `test-support`.
fn is_test_support_dev_edge(from: &Package, dep: &Package, kinds: &[DependencyKind]) -> bool {
    dep.name.as_str() == "gpui"
        && !kinds.is_empty()
        && kinds.iter().all(|k| *k == DependencyKind::Development)
        && from.dependencies.iter().any(|d| {
            d.name == dep.name.as_str()
                && d.kind == DependencyKind::Development
                && d.features.iter().any(|f| f == "test-support")
        })
}

fn has_workspace_lints(manifest: &str) -> bool {
    manifest
        .parse::<toml::Table>()
        .ok()
        .and_then(|t| t.get("lints")?.get("workspace")?.as_bool())
        .unwrap_or(false)
}

type Edges<'a> = HashMap<&'a PackageId, Vec<(&'a PackageId, Vec<DependencyKind>)>>;

fn edges_of(md: &Metadata) -> Edges<'_> {
    md.resolve
        .as_ref()
        .map(|r| {
            r.nodes
                .iter()
                .map(|n| {
                    let deps = n
                        .deps
                        .iter()
                        .map(|d| (&d.pkg, d.dep_kinds.iter().map(|k| k.kind).collect()))
                        .collect();
                    (&n.id, deps)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// All guard violations in the workspace, sorted by crate then rule.
pub fn violations(md: &Metadata, manifest: &dyn Fn(&Path) -> String) -> Vec<Violation> {
    let pkgs: HashMap<&PackageId, &Package> = md.packages.iter().map(|p| (&p.id, p)).collect();
    let edges = edges_of(md);
    let members = md.workspace_packages();

    let mut out: BTreeSet<(String, Rule, Vec<String>)> = BTreeSet::new();
    let mut add = |krate: &str, rule: Rule, via: Vec<String>| {
        out.insert((krate.to_string(), rule, via));
    };

    let layers: HashMap<&PackageId, Layer> = members
        .iter()
        .filter_map(|m| layer_of(md, m).map(|l| (&m.id, l)))
        .collect();

    for root in &members {
        let name = root.name.as_str();

        if !has_workspace_lints(&manifest(root.manifest_path.as_std_path())) {
            add(name, Rule::MissingWorkspaceLints, vec![]);
        }

        let Some(&layer) = layers.get(&root.id) else {
            add(name, Rule::UnknownLayer, vec![]);
            continue;
        };

        // ## m2b-t01-interfaces
        if name == GUARD
            && let Some(via) = guard_violation(&root.id, &edges, &pkgs)
        {
            add(name, Rule::GuardNotRuntimeFree, via);
        }

        // ## m3a-t01b-seams
        let in_sql = rel_dir(md, root).starts_with("crates/sql/");
        if in_sql
            && name != SQL_TESTKIT
            && let Some(via) = normal_path_to(&root.id, &edges, &pkgs, |p| {
                SQL_FORBIDDEN.contains(&p.name.as_str())
            })
        {
            add(name, Rule::SqlNotRuntimeFree, via);
        }
        if in_sql {
            for (dep_id, _) in edges.get(&root.id).into_iter().flatten() {
                if let Some(dep) = pkgs.get(dep_id)
                    && is_core_crate(md, dep, &layers)
                {
                    add(name, Rule::SqlReachesCore, vec![dep.name.to_string()]);
                }
            }
        }
        for (backend, owner) in BACKENDS {
            if name == owner {
                continue;
            }
            for (dep_id, _) in edges.get(&root.id).into_iter().flatten() {
                if pkgs.get(dep_id).is_some_and(|d| d.name.as_str() == backend) {
                    add(name, Rule::BackendOutsideOwner, vec![backend.into()]);
                }
            }
            // ## m3b-t01a-seams (fix round 1, M4): a declared edge too, so an optional
            // dependency that no feature turns on in this resolve is still seen.
            if root.dependencies.iter().any(|d| d.name == backend) {
                add(name, Rule::BackendOutsideOwner, vec![backend.into()]);
            }
        }
        if name == TEXT_CORE
            && let Some(via) = normal_path_to(&root.id, &edges, &pkgs, |p| {
                p.name.as_str() == ADAPTER || is_guarded(p)
            })
        {
            add(name, Rule::TextCoreNotHeadless, via);
        }

        if root.dependencies.iter().any(|d| {
            d.name == ROPE
                && (d.uses_default_features
                    || d.features
                        .iter()
                        .any(|f| ROPE_LINE_FEATURES.contains(&f.as_str())))
        }) {
            add(name, Rule::RopeLineBreaks, vec![ROPE.into()]);
        }
        // Round 2: a member feature that would turn the line features on when it is built.
        if root.features.values().flatten().any(|f| {
            ROPE_LINE_FEATURES
                .iter()
                .any(|line| *f == format!("{ROPE}/{line}") || *f == format!("{ROPE}?/{line}"))
        }) {
            add(
                name,
                Rule::RopeLineBreaks,
                vec![format!("[features] {ROPE}")],
            );
        }

        // ## m4-t01a-model (fix round 1, I1 and I2: feature-resolved from the manifests)
        if name == DDL
            && let Some(via) = feature_walk(
                &root.id,
                &edges,
                &pkgs,
                WalkStart::all_features(root, &[DependencyKind::Normal], Some(DDL_APPLY)),
                |_| false,
                ddl_forbidden,
            )
            .bad_path
        {
            add(name, Rule::DdlIsPure, via);
        }
        // Fix round 2 (I-R1): `apply` is excluded from the walk above, so `default` must not
        // turn it on (every user that keeps default features would get it).
        if name == DDL && own_closure(root, "default").contains(DDL_APPLY) {
            add(
                name,
                Rule::DdlIsPure,
                vec![format!("[features] default -> {DDL_APPLY}")],
            );
        }
        if name != APP && name != DDL && turns_on(root, &edges, &pkgs, DDL, DDL_APPLY) {
            add(
                name,
                Rule::DdlApplyOnlyFromApp,
                vec![format!("{DDL}/{DDL_APPLY}")],
            );
        }
        if name == META {
            // Any declared edge, optional or not, of any kind; then the all-features graph.
            for d in root.dependencies.iter().filter(|d| meta_forbidden(&d.name)) {
                add(name, Rule::MetaBelowIntrospect, vec![d.name.clone()]);
            }
            if let Some(via) = feature_walk(
                &root.id,
                &edges,
                &pkgs,
                WalkStart::all_features(root, &ALL_KINDS, None),
                |_| false,
                meta_forbidden,
            )
            .bad_path
            {
                add(name, Rule::MetaBelowIntrospect, via);
            }
        }
        if name != APP && name != META && turns_on(root, &edges, &pkgs, META, EMBEDDED_SYSTEM) {
            add(
                name,
                Rule::EmbeddedSystemOnlyFromApp,
                vec![format!("{META}/{EMBEDDED_SYSTEM}")],
            );
        }
        if name == INTROSPECT {
            if let Some(via) =
                any_then_normal_path_to(&root.id, &edges, &pkgs, |p| p.name.as_str() == EXEC)
            {
                add(name, Rule::IntrospectNotExec, via);
            }
            for (dep_id, _) in edges.get(&root.id).into_iter().flatten() {
                if pkgs.get(dep_id).is_some_and(|d| d.name.as_str() == RESULTS) {
                    add(name, Rule::IntrospectNotExec, vec![RESULTS.into()]);
                }
            }
            // Fix round 3 (m-R2-1): any declared edge too, optional or not, of any kind, so a
            // dependency behind a feature nothing turns on (no resolve edge) is seen.
            for d in root
                .dependencies
                .iter()
                .filter(|d| d.name == RESULTS || d.name == EXEC)
            {
                add(name, Rule::IntrospectNotExec, vec![d.name.clone()]);
            }
        }

        if PRODUCT_CRATES.contains(&name)
            && let Some(bad) = root.dependencies.iter().find(|d| {
                d.name == ADAPTER
                    && d.kind != DependencyKind::Development
                    && d.features
                        .iter()
                        .any(|f| ADAPTER_ONLY_FEATURES.contains(&f.as_str()))
            })
        {
            add(name, Rule::AdapterFeatureInProduct, vec![bad.name.clone()]);
        }

        // The adapter is the one crate allowed to reach GPUI.
        let is_adapter = name == ADAPTER;

        // Breadth-first walk over normal, build and dev edges; the first path found is reported.
        let mut seen: BTreeSet<&PackageId> = BTreeSet::from([&root.id]);
        let mut queue: VecDeque<(&PackageId, Vec<String>)> = VecDeque::from([(&root.id, vec![])]);
        while let Some((id, path)) = queue.pop_front() {
            for (dep_id, kinds) in edges.get(id).into_iter().flatten() {
                let Some(dep) = pkgs.get(dep_id) else {
                    continue;
                };
                let mut via = path.clone();
                via.push(dep.name.to_string());
                // The exception applies to the Ui/App package that owns the edge, at any depth:
                // that package is checked in its own right.
                let from = pkgs[id];
                if !is_adapter
                    && matches!(layers.get(id), Some(Layer::Ui | Layer::App))
                    && is_test_support_dev_edge(from, dep, kinds)
                {
                    continue;
                }
                let dep_layer = layers.get(dep_id).copied();
                if layer == Layer::Core && dep_layer == Some(Layer::Ui) {
                    add(name, Rule::CoreReachesUi, via);
                    continue;
                }
                if layer == Layer::Ui && dep_layer == Some(Layer::Core) {
                    add(name, Rule::UiDependsOnCore, via);
                    continue;
                }
                if dep.name.as_str() == ADAPTER {
                    // Not expanded: the adapter's own edges are checked on the adapter.
                    if layer == Layer::Core {
                        add(name, Rule::CoreReachesUi, via);
                    }
                    continue;
                }
                if is_guarded(dep) {
                    if !is_adapter {
                        add(name, Rule::ZedOutsideAdapter, via);
                    }
                    continue;
                }
                if seen.insert(dep_id) {
                    queue.push_back((dep_id, via));
                }
            }
        }
    }
    // ## m3a-t01b-seams (fix round 1): the unified features, whoever enables them; round 2:
    // of the ropey `hacivat-text` resolves to (any ropey when it is not in the graph).
    let text_rope: Vec<&PackageId> = members
        .iter()
        .filter(|m| m.name.as_str() == TEXT_CORE)
        .flat_map(|m| edges.get(&m.id).into_iter().flatten())
        .filter(|(id, _)| pkgs.get(id).is_some_and(|p| p.name.as_str() == ROPE))
        .map(|(id, _)| *id)
        .collect();
    for node in md.resolve.iter().flat_map(|r| &r.nodes) {
        let is_rope = pkgs.get(&node.id).is_some_and(|p| p.name.as_str() == ROPE);
        let relevant = text_rope.is_empty() || text_rope.contains(&&node.id);
        if is_rope && relevant && node.features.iter().any(|f| f.as_str() == "unicode_lines") {
            add(ROPE, Rule::RopeLineBreaks, vec!["unicode_lines".into()]);
        }
    }
    out.into_iter()
        .map(|(krate, rule, via)| Violation { krate, rule, via })
        .collect()
}

// ## m3b-t01a-seams (fix round 1, M2)
/// The crates that may name `sql_oracle_pg::tree` (its `pb` re-export of `pg_query`'s
/// protobuf types): the oracle itself and the PG classifier backend.
const TREE_OWNERS: [&str; 2] = ["sql-oracle-pg", "sql-safety"];

/// `BackendOutsideOwner` at the source level: a non-comment line naming `sql_oracle_pg` and
/// `tree` in any `crates/<layer>/<crate>/` outside [`TREE_OWNERS`] (tests included), because
/// `tree::pb` is reachable through every crate that depends on the oracle.
pub fn tree_violations(workspace: &Path) -> Vec<Violation> {
    let walk = rs_files;
    let mut found = BTreeSet::new();
    let Ok(layers) = std::fs::read_dir(workspace.join("crates")) else {
        return Vec::new();
    };
    for layer in layers.flatten() {
        let Ok(crates) = std::fs::read_dir(layer.path()) else {
            continue;
        };
        for krate in crates.flatten() {
            let name = krate.file_name().to_string_lossy().into_owned();
            if TREE_OWNERS.contains(&name.as_str()) {
                continue;
            }
            let mut files = Vec::new();
            walk(&krate.path(), &mut files);
            let names_tree = files.iter().any(|f| {
                std::fs::read_to_string(f).is_ok_and(|src| {
                    src.lines().any(|l| {
                        let l = l.trim_start();
                        !l.starts_with("//") && l.contains("sql_oracle_pg") && l.contains("tree")
                    })
                })
            });
            if names_tree {
                found.insert(name);
            }
        }
    }
    found
        .into_iter()
        .map(|krate| Violation {
            krate,
            rule: Rule::BackendOutsideOwner,
            via: vec!["sql_oracle_pg::tree".into()],
        })
        .collect()
}

/// Every `.rs` file under `dir`, recursively (the source rules' walker). `target/` directories
/// (build output, e.g. `fuzz/target`) are skipped.
fn rs_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            if e.file_name() != "target" {
                rs_files(&p, out);
            }
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

// ## m4-t01a-model
/// Where `MetaQuery::new` may be named (`MetaQueryOnlyFromPacks`).
const META_QUERY: &str = "MetaQuery";
const META_QUERY_DIR: &str = "crates/core/hacivat-introspect/src/pack/";
const META_QUERY_DEF: &str = "crates/core/hacivat-session/src/meta.rs";
/// The shots-only find hold (`HoldCountsTestOnly`) and its definition.
const HOLD_COUNTS: &str = "hold_counts";
const HOLD_COUNTS_DEF: &str = "crates/ui/hacivat-ui-editor/src/find.rs";
// ## m4-t04-packs
/// `PackParseOnlyFromPacks`: the type, the one associated item every file may name, and the
/// places that may name the rest.
const PACK: &str = "Pack";
const PACK_ITEMS_OPEN: [&str; 1] = ["builtin"];
const PACK_PARSE_DIRS: [&str; 4] = [
    "crates/core/hacivat-introspect/src/pack/",
    "crates/core/hacivat-introspect/src/pg/",
    "crates/core/hacivat-introspect/src/mysql/",
    "crates/core/hacivat-introspect/src/sqlite/",
];
const PACK_PARSE_FUZZ: &str = "fuzz/meta/";

/// How deep `#[path]` / `include!` targets are followed.
const MAX_INCLUDE_DEPTH: usize = 4;

/// One token of Rust source as the source rules see it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum SrcTok {
    Ident(String),
    Punct(char),
    /// A string literal's text (raw), kept only for `#[path]` and `include!` targets.
    Str(String),
}

/// `src` as tokens, with comments (line, nested block, doc), whitespace, char literals,
/// lifetimes and numbers dropped, so `MetaQuery :: new`, a path split over lines or a comment
/// in the middle collapse to the same tokens; string contents never match an identifier.
fn src_tokens(src: &str) -> Vec<SrcTok> {
    let c: Vec<char> = src.chars().collect();
    let at = |i: usize| c.get(i).copied().unwrap_or('\0');
    let mut out = Vec::new();
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        if ch.is_whitespace() {
            i += 1;
        } else if ch == '/' && at(i + 1) == '/' {
            while i < c.len() && c[i] != '\n' {
                i += 1;
            }
        } else if ch == '/' && at(i + 1) == '*' {
            let mut depth = 1;
            i += 2;
            while i < c.len() && depth > 0 {
                if c[i] == '/' && at(i + 1) == '*' {
                    depth += 1;
                    i += 2;
                } else if c[i] == '*' && at(i + 1) == '/' {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
        } else if let Some(r) = raw_string_r(&c, i) {
            // `r"…"`, `r#"…"#`, `br#"…"#`, `cr"…"` (fix round 2: prefixed raw strings).
            let mut j = r + 1;
            let mut hashes = 0;
            while at(j) == '#' {
                hashes += 1;
                j += 1;
            }
            j += 1;
            let begin = j;
            loop {
                if j >= c.len() {
                    break;
                }
                if c[j] == '"' && (0..hashes).all(|k| at(j + 1 + k) == '#') {
                    break;
                }
                j += 1;
            }
            out.push(SrcTok::Str(c[begin..j.min(c.len())].iter().collect()));
            i = j + 1 + hashes;
        } else if ch == '"' {
            let mut j = i + 1;
            let begin = j;
            while j < c.len() && c[j] != '"' {
                j += if c[j] == '\\' { 2 } else { 1 };
            }
            out.push(SrcTok::Str(c[begin..j.min(c.len())].iter().collect()));
            i = j + 1;
        } else if ch == '\'' {
            if at(i + 1) == '\\' {
                let mut j = i + 2;
                while j < c.len() && c[j] != '\'' {
                    j += 1;
                }
                i = j + 1;
            } else if at(i + 2) == '\'' {
                i += 3;
            } else {
                i += 1;
            }
        } else if ch == 'r' && at(i + 1) == '#' && (at(i + 2).is_alphabetic() || at(i + 2) == '_') {
            // A raw identifier `r#new` is the identifier `new` (fix round 2, I-R2 c).
            i += 2;
        } else if ch.is_alphabetic() || ch == '_' {
            let begin = i;
            while i < c.len() && (c[i].is_alphanumeric() || c[i] == '_') {
                i += 1;
            }
            out.push(SrcTok::Ident(c[begin..i].iter().collect()));
        } else if ch.is_ascii_digit() {
            while i < c.len() && (c[i].is_alphanumeric() || c[i] == '_' || c[i] == '.') {
                i += 1;
            }
        } else {
            out.push(SrcTok::Punct(ch));
            i += 1;
        }
    }
    out
}

/// The index of the `r` that opens a raw string at `i` (`r`, `br` or `cr` followed by `"` or by
/// `#`s and `"`), if one does.
fn raw_string_r(c: &[char], i: usize) -> Option<usize> {
    let at = |k: usize| c.get(k).copied().unwrap_or('\0');
    let r = match c[i] {
        'r' => i,
        'b' | 'c' if at(i + 1) == 'r' => i + 1,
        _ => return None,
    };
    if i > 0 && (c[i - 1].is_alphanumeric() || c[i - 1] == '_') {
        return None;
    }
    let mut j = r + 1;
    while at(j) == '#' {
        j += 1;
    }
    (at(j) == '"').then_some(r)
}

fn tok_ident(t: &[SrcTok], i: usize, name: &str) -> bool {
    matches!(t.get(i), Some(SrcTok::Ident(s)) if s == name)
}

fn tok_punct(t: &[SrcTok], i: usize, p: char) -> bool {
    t.get(i) == Some(&SrcTok::Punct(p))
}

/// The index just past the balanced `<…>` group that opens at `open` (`t[open]` is `<`), or
/// `t.len()` when it never closes. A `>` of `->` or `=>` is not a bracket.
fn skip_angle(t: &[SrcTok], open: usize) -> usize {
    let mut depth = 0usize;
    for k in open..t.len() {
        if tok_punct(t, k, '<') {
            depth += 1;
        } else if tok_punct(t, k, '>')
            && !(k > 0 && (tok_punct(t, k - 1, '-') || tok_punct(t, k - 1, '=')))
        {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return k + 1;
            }
        }
    }
    t.len()
}

/// Whether the `<` at `open` opens a qualified path (`<T>::item`, `<T as Tr>::item`), not
/// generic arguments: it follows no path segment (an identifier, `::`) and no closing `>` of
/// generic arguments. After `->` or `=>` (a return type, a match arm) it is qualified.
fn qualified_open(t: &[SrcTok], open: usize) -> bool {
    open == 0
        || !(matches!(t.get(open - 1), Some(SrcTok::Ident(_)))
            || tok_punct(t, open - 1, ':')
            || (tok_punct(t, open - 1, '>')
                && !(open >= 2 && (tok_punct(t, open - 2, '-') || tok_punct(t, open - 2, '=')))))
}

/// Whether the `MetaQuery` segment at `i` is followed by an associated-item access (fix rounds
/// 3 and 4, I-R4/I-R5):
/// - directly: `MetaQuery`, optional generic arguments (`<…>` or `::<…>`), then `::`;
/// - through a qualified path: `MetaQuery` anywhere inside the `<…>` of a qualified path that
///   is followed by `::` — whatever wraps it there (parentheses, a tuple, `&`, `&mut`, `dyn`,
///   its own generic arguments): `<(MetaQuery)>::new`, `<((MetaQuery<'_>))>::new`,
///   `<(MetaQuery,)>::default`, `<&MetaQuery>::clone`. In doubt this refuses: an access that
///   resolves on a type merely containing `MetaQuery` (`<Vec<MetaQuery>>::new`) is refused too;
///   `Vec::<MetaQuery>::new()` is the spelling that stays legal.
fn meta_query_item_access(t: &[SrcTok], i: usize) -> bool {
    let mut j = i + 1;
    if tok_punct(t, j, ':') && tok_punct(t, j + 1, ':') && tok_punct(t, j + 2, '<') {
        j = skip_angle(t, j + 2);
    } else if tok_punct(t, j, '<') {
        j = skip_angle(t, j);
    }
    if tok_punct(t, j, ':') && tok_punct(t, j + 1, ':') {
        return true;
    }
    // Every qualified path whose `<…>` holds this `MetaQuery` and is followed by `::`.
    let mut k = i;
    while k > 0 {
        k -= 1;
        // A path expression never spans a statement or block boundary.
        if tok_punct(t, k, ';') || tok_punct(t, k, '{') || tok_punct(t, k, '}') {
            break;
        }
        if !tok_punct(t, k, '<') || !qualified_open(t, k) {
            continue;
        }
        let end = skip_angle(t, k);
        if end > i && tok_punct(t, end, ':') && tok_punct(t, end + 1, ':') {
            return true;
        }
    }
    false
}

/// Whether the `impl` at `i` starts an impl block (an item), not `impl Trait` in a type
/// position (`-> impl Iterator<…>`, `q: impl Into<…>`; fix round 3, m-R2-2): the previous
/// token is none, `}`, `;`, `]`, `{` or `pub`/`unsafe`/`default`.
fn impl_item_header(t: &[SrcTok], i: usize) -> bool {
    i == 0
        || ['}', ';', ']', '{'].iter().any(|p| tok_punct(t, i - 1, *p))
        || ["pub", "unsafe", "default"]
            .iter()
            .any(|k| tok_ident(t, i - 1, k))
}

/// Whether the tokens build a `MetaQuery` or open a way to:
/// - any associated-item access on `MetaQuery` (`MetaQuery::new`, `MetaQuery::r#new`,
///   `MetaQuery::<'_>::new`, `<…MetaQuery<'_>>::new`, `MetaQuery::OTHER`; I-R4);
/// - an alias (`MetaQuery as X`, `type X = …MetaQuery…;`);
/// - an impl block whose header names `MetaQuery` (inherent, or a local trait whose methods may
///   call `Self::new`; `impl Trait` in a type position is not a header, m-R2-2);
/// - `MetaQuery` inside a macro invocation's brackets (`mk!(MetaQuery)` with `$t::new`).
///
/// Type positions stay legal: `&MetaQuery`, `Vec<MetaQuery>`, `q: MetaQuery`,
/// `-> impl Iterator<Item = &MetaQuery>`, `impl Into<MetaQuery>`.
fn builds_meta_query(t: &[SrcTok]) -> bool {
    let names_it = |range: &[SrcTok]| range.iter().any(|x| *x == SrcTok::Ident(META_QUERY.into()));
    (0..t.len()).any(|i| {
        if tok_ident(t, i, META_QUERY) {
            meta_query_item_access(t, i) || tok_ident(t, i + 1, "as")
        } else if tok_ident(t, i, "type") || (tok_ident(t, i, "impl") && impl_item_header(t, i)) {
            let header: Vec<SrcTok> = t[i + 1..]
                .iter()
                .take_while(|x| **x != SrcTok::Punct(';') && **x != SrcTok::Punct('{'))
                .cloned()
                .collect();
            names_it(&header)
        } else if matches!(t.get(i), Some(SrcTok::Ident(_)))
            && tok_punct(t, i + 1, '!')
            && let Some(SrcTok::Punct(open @ ('(' | '[' | '{'))) = t.get(i + 2)
        {
            let close = match open {
                '(' => ')',
                '[' => ']',
                _ => '}',
            };
            let mut depth = 0usize;
            let mut end = t.len();
            for (k, x) in t.iter().enumerate().skip(i + 2) {
                if *x == SrcTok::Punct(*open) {
                    depth += 1;
                } else if *x == SrcTok::Punct(close) {
                    depth -= 1;
                    if depth == 0 {
                        end = k;
                        break;
                    }
                }
            }
            names_it(&t[i + 3..end.max(i + 3)])
        } else {
            false
        }
    })
}

// ## m4-t04-packs
/// The associated item named after the `Pack` at `i`, if the segment is followed by an
/// associated-item access: directly (`Pack::item`, `Pack::<…>::item`) or through a qualified
/// path whose `<…>` holds it (`<Pack>::item`, `<(Pack)>::item`). `Some("")` when an access is
/// there but its item cannot be read (refused in doubt).
fn pack_item_access(t: &[SrcTok], i: usize) -> Option<String> {
    let item_at = |k: usize| match t.get(k) {
        Some(SrcTok::Ident(s)) => s.clone(),
        _ => String::new(),
    };
    let mut j = i + 1;
    if tok_punct(t, j, ':') && tok_punct(t, j + 1, ':') && tok_punct(t, j + 2, '<') {
        j = skip_angle(t, j + 2);
    } else if tok_punct(t, j, '<') {
        j = skip_angle(t, j);
    }
    if tok_punct(t, j, ':') && tok_punct(t, j + 1, ':') {
        return Some(item_at(j + 2));
    }
    let mut k = i;
    while k > 0 {
        k -= 1;
        if tok_punct(t, k, ';') || tok_punct(t, k, '{') || tok_punct(t, k, '}') {
            break;
        }
        if !tok_punct(t, k, '<') || !qualified_open(t, k) {
            continue;
        }
        let end = skip_angle(t, k);
        if end > i && tok_punct(t, end, ':') && tok_punct(t, end + 1, ':') {
            return Some(item_at(end + 2));
        }
    }
    None
}

/// Whether the tokens name a renderable pack constructor (`PackParseOnlyFromPacks`): an
/// associated item of `Pack` other than `builtin` (`Pack::parse`, `Pack::parse_all`,
/// `<Pack>::parse`, a function pointer `Pack::parse`), an alias of `Pack` (`Pack as X`,
/// `type X = …Pack…;`), an `impl` header naming it (a local trait whose methods may call
/// `Self::parse`), or `Pack` in a macro invocation's brackets without an open item after it
/// (`mk!(Pack)` with `$t::parse`). Type positions stay legal (`&Pack`, `Option<Pack>`).
fn parses_a_pack(t: &[SrcTok]) -> bool {
    let open = |item: &str| PACK_ITEMS_OPEN.contains(&item);
    (0..t.len()).any(|i| {
        if tok_ident(t, i, PACK) {
            tok_ident(t, i + 1, "as") || pack_item_access(t, i).is_some_and(|item| !open(&item))
        } else if tok_ident(t, i, "type") || (tok_ident(t, i, "impl") && impl_item_header(t, i)) {
            t[i + 1..]
                .iter()
                .take_while(|x| **x != SrcTok::Punct(';') && **x != SrcTok::Punct('{'))
                .any(|x| *x == SrcTok::Ident(PACK.into()))
        } else if matches!(t.get(i), Some(SrcTok::Ident(_)))
            && tok_punct(t, i + 1, '!')
            && let Some(SrcTok::Punct(open_p @ ('(' | '[' | '{'))) = t.get(i + 2)
        {
            let close = match open_p {
                '(' => ')',
                '[' => ']',
                _ => '}',
            };
            let mut depth = 0usize;
            let mut end = t.len();
            for (k, x) in t.iter().enumerate().skip(i + 2) {
                if *x == SrcTok::Punct(*open_p) {
                    depth += 1;
                } else if *x == SrcTok::Punct(close) {
                    depth -= 1;
                    if depth == 0 {
                        end = k;
                        break;
                    }
                }
            }
            (i + 3..end.max(i + 3)).any(|k| {
                tok_ident(t, k, PACK) && !pack_item_access(t, k).is_some_and(|item| open(&item))
            })
        } else {
            false
        }
    })
}

/// Whether the tokens name `hold_counts` at all (a call, an alias, a function pointer).
fn names_hold_counts(t: &[SrcTok]) -> bool {
    t.iter().any(|x| *x == SrcTok::Ident(HOLD_COUNTS.into()))
}

// ## m4-hyg-tmp
/// The `tempfile` names a `TempDirOutlivesTest` statement, static or global is checked for:
/// the guard types and the functions that make a temp directory.
const TEMP_GUARDS: [&str; 5] = [
    "TempDir",
    "NamedTempFile",
    "TempPath",
    "tempdir",
    "tempdir_in",
];

/// Whether `t[i]` is one of [`TEMP_GUARDS`].
fn names_temp_guard(t: &[SrcTok], i: usize) -> bool {
    TEMP_GUARDS.iter().any(|g| tok_ident(t, i, g))
}

/// The index past the item that starts at `from`: the first `;` outside braces, or the brace
/// that closes the first `{` (a `struct`'s fields, a `thread_local!` block).
fn item_end(t: &[SrcTok], from: usize) -> usize {
    let mut depth = 0usize;
    for (k, tok) in t.iter().enumerate().skip(from) {
        match tok {
            SrcTok::Punct('{') => depth += 1,
            SrcTok::Punct('}') => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return k + 1;
                }
            }
            SrcTok::Punct(';') if depth == 0 => return k + 1,
            _ => {}
        }
    }
    t.len()
}

/// Whether the tokens let a `tempfile` guard outlive its test (`TempDirOutlivesTest`):
///
/// - `. keep ( )`, `. into_path ( )` (the guard gives up its directory) or
///   `. disable_cleanup (`, whatever the receiver (`self.keep(x)` with arguments is not one);
/// - a statement (between `;`, `{` and `}`) naming `leak` or `forget` and a guard;
/// - a `static` item outside `thread_local!` whose type or initialiser names a guard;
/// - `impl … Global for X` where the definition of `X` in the same file names a guard.
///
/// Limits: a leak of a guard whose statement never names a guard type or constructor
/// (`Box::leak(Box::new(x))` alone), a `static` holding a type defined elsewhere that holds a
/// guard, and a `Global` defined in another file are not seen.
fn leaks_a_temp_dir(t: &[SrcTok]) -> bool {
    let call = |i: usize, name: &str, empty: bool| {
        tok_punct(t, i, '.')
            && tok_ident(t, i + 1, name)
            && tok_punct(t, i + 2, '(')
            && (!empty || tok_punct(t, i + 3, ')'))
    };
    if (0..t.len()).any(|i| {
        call(i, "keep", true) || call(i, "into_path", true) || call(i, "disable_cleanup", false)
    }) {
        return true;
    }
    // Statements that leak or forget a guard.
    let mut start = 0;
    for i in 0..=t.len() {
        let boundary = i == t.len() || [';', '{', '}'].iter().any(|p| tok_punct(t, i, *p));
        if boundary {
            let stmt = start..i;
            let leaks = stmt
                .clone()
                .any(|k| tok_ident(t, k, "leak") || tok_ident(t, k, "forget"));
            if leaks && stmt.clone().any(|k| names_temp_guard(t, k)) {
                return true;
            }
            start = i + 1;
        }
    }
    // `thread_local!` blocks: their statics are dropped when the thread ends.
    let mut local = vec![false; t.len()];
    for i in 0..t.len() {
        if tok_ident(t, i, "thread_local") && tok_punct(t, i + 1, '!') {
            let end = item_end(t, i);
            local[i..end].iter_mut().for_each(|x| *x = true);
        }
    }
    // A process-wide `static` item (not `&'static`: the lifetime's tick is dropped, so the
    // item position is checked).
    let item_start = |i: usize| {
        i == 0
            || [';', '{', '}', ']', ')']
                .iter()
                .any(|p| tok_punct(t, i - 1, *p))
            || tok_ident(t, i - 1, "pub")
    };
    for (i, in_local) in local.iter().enumerate() {
        if tok_ident(t, i, "static") && !in_local && item_start(i) {
            let end = item_end(t, i);
            if (i..end).any(|k| names_temp_guard(t, k)) {
                return true;
            }
        }
    }
    // A type holding a guard that is a GPUI `Global`.
    for i in 0..t.len() {
        if tok_ident(t, i, "Global")
            && tok_ident(t, i + 1, "for")
            && let Some(SrcTok::Ident(name)) = t.get(i + 2)
        {
            let holds_guard = (0..t.len()).any(|k| {
                (tok_ident(t, k, "struct") || tok_ident(t, k, "enum"))
                    && tok_ident(t, k + 1, name)
                    && (k..item_end(t, k)).any(|m| names_temp_guard(t, m))
            });
            if holds_guard {
                return true;
            }
        }
    }
    false
}

/// The `#[path = "…"]` targets (also `path = "…"` inside `#[cfg_attr(…)]` at any nesting, fix
/// round 3, I-R3) and the literal `include!("…")` targets of `t`.
fn module_targets(t: &[SrcTok]) -> (Vec<String>, Vec<String>) {
    let mut paths = Vec::new();
    let mut includes = Vec::new();
    for i in 0..t.len() {
        if tok_punct(t, i, '#')
            && tok_punct(t, i + 1, '[')
            && tok_ident(t, i + 2, "path")
            && tok_punct(t, i + 3, '=')
            && let Some(SrcTok::Str(p)) = t.get(i + 4)
        {
            paths.push(p.clone());
        }
        // `#[cfg_attr(…)]` or `#![cfg_attr(…)]`: every `path = "…"` inside its parentheses.
        let at = if tok_punct(t, i + 1, '!') {
            i + 2
        } else {
            i + 1
        };
        if tok_punct(t, i, '#')
            && tok_punct(t, at, '[')
            && tok_ident(t, at + 1, "cfg_attr")
            && tok_punct(t, at + 2, '(')
        {
            let mut depth = 0usize;
            for k in at + 2..t.len() {
                if tok_punct(t, k, '(') {
                    depth += 1;
                } else if tok_punct(t, k, ')') {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        break;
                    }
                } else if tok_ident(t, k, "path")
                    && tok_punct(t, k + 1, '=')
                    && let Some(SrcTok::Str(p)) = t.get(k + 2)
                {
                    paths.push(p.clone());
                }
            }
        }
        if tok_ident(t, i, "include")
            && tok_punct(t, i + 1, '!')
            && tok_punct(t, i + 2, '(')
            && let Some(SrcTok::Str(p)) = t.get(i + 3)
        {
            includes.push(p.clone());
        }
    }
    (paths, includes)
}

/// `rel` joined to `dir` lexically (`..` and `.` folded), with `/` separators.
fn join_lexical(dir: &Path, rel: &str) -> std::path::PathBuf {
    let mut out = std::path::PathBuf::new();
    for comp in dir.join(rel).components() {
        match comp {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// The tokens a file contributes to its crate: its own, plus (recursively) those of the files
/// it pulls in with `#[path]` or a literal `include!`. Also returns the `include!` targets that
/// are not `.rs` files and the `#[path]` targets, both resolved.
fn effective_tokens(
    file: &Path,
    src: &str,
) -> (
    Vec<SrcTok>,
    Vec<std::path::PathBuf>,
    Vec<std::path::PathBuf>,
) {
    let mut tokens = Vec::new();
    let mut non_rs = Vec::new();
    let mut path_targets = Vec::new();
    let mut seen: BTreeSet<std::path::PathBuf> = BTreeSet::new();
    let mut todo: Vec<(std::path::PathBuf, String, usize)> =
        vec![(file.to_path_buf(), src.to_string(), 0)];
    while let Some((f, text, depth)) = todo.pop() {
        if !seen.insert(f.clone()) {
            continue;
        }
        let t = src_tokens(&text);
        let (paths, includes) = module_targets(&t);
        tokens.extend(t);
        let dir = f.parent().unwrap_or(Path::new("")).to_path_buf();
        for (target, is_include) in paths
            .iter()
            .map(|p| (p, false))
            .chain(includes.iter().map(|p| (p, true)))
        {
            let resolved = join_lexical(&dir, target);
            if is_include && resolved.extension().is_none_or(|x| x != "rs") {
                non_rs.push(resolved.clone());
            }
            if !is_include && depth == 0 {
                path_targets.push(resolved.clone());
            }
            if depth < MAX_INCLUDE_DEPTH
                && let Ok(more) = std::fs::read_to_string(&resolved)
            {
                todo.push((resolved, more, depth + 1));
            }
        }
    }
    (tokens, non_rs, path_targets)
}

/// The roots the source rules walk: every `crates/<layer>/<crate>/` (as `(name, dir, prefix)`
/// with `prefix` its workspace-relative directory), plus `fuzz/` and `xtask/` (fix round 1, I3:
/// the fuzz workspace links `hacivat-introspect`, so `hacivat-session`).
fn source_roots(workspace: &Path) -> Vec<(String, std::path::PathBuf, String)> {
    let mut roots = Vec::new();
    if let Ok(layers) = std::fs::read_dir(workspace.join("crates")) {
        for layer in layers.flatten() {
            let Ok(crates) = std::fs::read_dir(layer.path()) else {
                continue;
            };
            for krate in crates.flatten() {
                let name = krate.file_name().to_string_lossy().into_owned();
                let prefix = format!("crates/{}/{name}/", layer.file_name().to_string_lossy());
                roots.push((name, krate.path(), prefix));
            }
        }
    }
    for extra in ["fuzz", "xtask"] {
        if workspace.join(extra).is_dir() {
            roots.push((
                extra.to_string(),
                workspace.join(extra),
                format!("{extra}/"),
            ));
        }
    }
    roots
}

/// The M4 source rules (fix round 1, I3: token-based, not a line grep):
///
/// - `MetaQueryOnlyFromPacks` over every crate, `fuzz/` and `xtask/`: no file outside
///   `hacivat-introspect/src/pack/**`, `hacivat-session/src/meta.rs` and a crate's `tests/`
///   builds a `MetaQuery` ([`builds_meta_query`]), counting the files it pulls in with
///   `#[path]` or `include!`; no `src/` file has a `#[path]` that leaves its crate's `src/`, and
///   no such file `include!`s a file that is not `.rs`.
/// - `HoldCountsTestOnly` over every crate's `src/`: no file but the definition names
///   `hold_counts` (same inclusion rule).
/// - `PackParseOnlyFromPacks` (`## m4-t04-packs`) over every crate, `fuzz/` and `xtask/`: no
///   file outside `hacivat-introspect/src/{pack,pg,mysql,sqlite}/**`, a crate's `tests/` and
///   `fuzz/meta/` names a renderable pack constructor ([`parses_a_pack`]).
/// - `TempDirOutlivesTest` (`## m4-hyg-tmp`) over every file of every crate, `fuzz/` and
///   `xtask/`: no `tempfile` guard outlives its test ([`leaks_a_temp_dir`]).
///
/// Comments, whitespace and string contents never match. One violation per finding, `via` the
/// workspace-relative file (and what it did, for the `#[path]`/`include!` refusals).
pub fn source_violations(workspace: &Path) -> Vec<Violation> {
    let mut found: BTreeSet<(String, Rule, String)> = BTreeSet::new();
    let rel_of = |p: &Path| {
        p.strip_prefix(workspace)
            .unwrap_or(p)
            .to_string_lossy()
            .replace('\\', "/")
    };
    for (name, dir, prefix) in source_roots(workspace) {
        let is_crate = prefix.starts_with("crates/");
        let mut files = Vec::new();
        rs_files(&dir, &mut files);
        for f in files {
            let rel = rel_of(&f);
            let in_crate = rel.strip_prefix(&prefix).unwrap_or("").to_string();
            let in_src = is_crate && in_crate.starts_with("src/");
            let meta_query_ok = is_crate
                && (rel.starts_with(META_QUERY_DIR)
                    || rel == META_QUERY_DEF
                    || in_crate.starts_with("tests/"));
            let hold_counts_checked = in_src && rel != HOLD_COUNTS_DEF;
            // ## m4-t04-packs
            let pack_parse_ok = (is_crate
                && (PACK_PARSE_DIRS.iter().any(|d| rel.starts_with(d))
                    || in_crate.starts_with("tests/")))
                || rel.starts_with(PACK_PARSE_FUZZ);
            let Ok(src) = std::fs::read_to_string(&f) else {
                continue;
            };
            // ## m4-hyg-tmp: every file, `src/` and `tests/` alike.
            if leaks_a_temp_dir(&src_tokens(&src)) {
                found.insert((name.clone(), Rule::TempDirOutlivesTest, rel.clone()));
            }
            if meta_query_ok && !hold_counts_checked && pack_parse_ok {
                continue;
            }
            let (tokens, non_rs, path_targets) = effective_tokens(&f, &src);
            if !meta_query_ok {
                if builds_meta_query(&tokens) {
                    found.insert((name.clone(), Rule::MetaQueryOnlyFromPacks, rel.clone()));
                }
                for t in &non_rs {
                    found.insert((
                        name.clone(),
                        Rule::MetaQueryOnlyFromPacks,
                        format!("{rel}: include! of a non-.rs file {}", rel_of(t)),
                    ));
                }
                if in_src {
                    let src_dir = dir.join("src");
                    for t in path_targets.iter().filter(|t| !t.starts_with(&src_dir)) {
                        found.insert((
                            name.clone(),
                            Rule::MetaQueryOnlyFromPacks,
                            format!("{rel}: #[path] outside src/ {}", rel_of(t)),
                        ));
                    }
                }
            }
            if hold_counts_checked && names_hold_counts(&tokens) {
                found.insert((name.clone(), Rule::HoldCountsTestOnly, rel.clone()));
            }
            // ## m4-t04-packs
            if !pack_parse_ok && parses_a_pack(&tokens) {
                found.insert((name.clone(), Rule::PackParseOnlyFromPacks, rel.clone()));
            }
        }
    }
    found
        .into_iter()
        .map(|(krate, rule, path)| Violation {
            krate,
            rule,
            via: vec![path],
        })
        .collect()
}

pub fn run() -> ExitCode {
    let metadata = match MetadataCommand::new().exec() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("cargo metadata failed: {e}");
            return ExitCode::from(2);
        }
    };
    let mut found = violations(&metadata, &|p| {
        std::fs::read_to_string(p).unwrap_or_default()
    });
    // ## m3b-t01a-seams (fix round 1, M2)
    found.extend(tree_violations(metadata.workspace_root.as_std_path()));
    // ## m4-t01a-model
    found.extend(source_violations(metadata.workspace_root.as_std_path()));
    if found.is_empty() {
        println!("check-deps: ok");
        return ExitCode::SUCCESS;
    }
    for v in &found {
        if v.via.is_empty() {
            eprintln!("{}: {:?}", v.krate, v.rule);
        } else {
            eprintln!("{}: {:?} via {}", v.krate, v.rule, v.via.join(" -> "));
        }
    }
    ExitCode::FAILURE
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace_root;
    use cargo_metadata::DependencyKind as DepKind;
    use serde_json::json;

    /// One workspace member: `(name, manifest dir under the workspace, dependencies)`.
    struct Member {
        name: String,
        dir: String,
        deps: Vec<Dep>,
        // ## m4-t01a-model: the member's `[features]` table (DdlIsPure fixtures).
        features: Vec<(String, Vec<String>)>,
    }

    struct Dep {
        name: String,
        kind: DepKind,
        features: Vec<String>,
        source: Option<String>,
        /// `default-features` (true unless a fixture turns it off, as the workspace's `ropey`).
        default_features: bool,
        // ## m4-t01a-model
        optional: bool,
        /// `package = …` renames (fix round 1, I1): the dependency's key in `[features]`.
        rename: Option<String>,
    }

    fn dep(name: &str, kind: DepKind, features: &[&str], source: Option<&str>) -> Dep {
        Dep {
            name: name.into(),
            kind,
            features: features.iter().map(|f| (*f).into()).collect(),
            source: source.map(|s| format!("git+{s}?rev=abc#abc")),
            default_features: true,
            optional: false,
            rename: None,
        }
    }

    fn dir_for(name: &str) -> &'static str {
        match name {
            "hacivat" => "crates/bin",
            "hacivat-app" => "crates/app",
            "hacivat-ui" | "hacivat-ui-core" => "crates/ui",
            "hacivat-l10n" => "crates/core",
            // ## m3a-t01b-seams
            "hacivat-text" | "hacivat-ui-grid" => "crates/ui",
            _ => "spikes",
        }
    }

    /// Builds a `Metadata` (with a resolve graph) from workspace members; every dependency
    /// name that is not a member gets a synthesized external package without dependencies.
    fn metadata(members: Vec<Member>) -> Metadata {
        let id = |n: &str| format!("path+file:///ws/{n}#0.0.0");
        let ext_id = |n: &str| format!("registry+https://ext/{n}#0.0.0");
        let is_member = |n: &str| members.iter().any(|m| m.name == n);
        let pkg_id = |n: &str| if is_member(n) { id(n) } else { ext_id(n) };
        let kind_json = |k: &DepKind| match k {
            DepKind::Development => json!("dev"),
            DepKind::Build => json!("build"),
            _ => json!(null),
        };
        let dependency = |d: &Dep| {
            json!({
                "name": d.name, "source": d.source, "req": "*", "kind": kind_json(&d.kind),
                "rename": d.rename, "optional": d.optional, "uses_default_features": d.default_features,
                "features": d.features, "target": null, "registry": null
            })
        };
        let package = |name: &str,
                       source: Option<String>,
                       dir: String,
                       deps: &[Dep],
                       features: &[(String, Vec<String>)]| {
            let features: serde_json::Map<String, serde_json::Value> = features
                .iter()
                .map(|(f, list)| (f.clone(), json!(list)))
                .collect();
            json!({
                "name": name, "version": "0.0.0", "id": pkg_id(name), "source": source,
                "dependencies": deps.iter().map(dependency).collect::<Vec<_>>(),
                "targets": [], "features": features,
                "manifest_path": format!("/ws/{dir}/Cargo.toml"),
                "metadata": null, "publish": null, "authors": [], "categories": [],
                "keywords": [], "readme": null, "repository": null, "homepage": null,
                "documentation": null, "edition": "2024", "links": null,
                "description": null, "license": null, "license_file": null
            })
        };
        let mut packages: Vec<_> = members
            .iter()
            .map(|m| {
                package(
                    &m.name,
                    None,
                    format!("{}/{}", m.dir, m.name),
                    &m.deps,
                    &m.features,
                )
            })
            .collect();
        let mut externals: Vec<(String, Option<String>)> = Vec::new();
        for d in members.iter().flat_map(|m| &m.deps) {
            if !is_member(&d.name) && !externals.iter().any(|(n, _)| *n == d.name) {
                externals.push((d.name.clone(), d.source.clone()));
            }
        }
        for (name, source) in externals {
            packages.push(package(&name, source, format!("ext/{name}"), &[], &[]));
        }
        let nodes: Vec<_> = members
            .iter()
            .map(|m| {
                json!({
                    "id": id(&m.name),
                    "dependencies": m.deps.iter().map(|d| pkg_id(&d.name)).collect::<Vec<_>>(),
                    "deps": m.deps.iter().map(|d| json!({
                        "name": d.name.replace('-', "_"), "pkg": pkg_id(&d.name),
                        "dep_kinds": [{"kind": kind_json(&d.kind), "target": null}]
                    })).collect::<Vec<_>>(),
                    "features": []
                })
            })
            .collect();
        let ids: Vec<_> = members.iter().map(|m| id(&m.name)).collect();
        serde_json::from_value(json!({
            "packages": packages,
            "workspace_members": ids,
            "workspace_default_members": ids,
            "resolve": {"nodes": nodes, "root": null},
            "target_directory": "/ws/target",
            "version": 1,
            "workspace_root": "/ws",
            "metadata": null
        }))
        .unwrap()
    }

    fn member(name: &str, dir: &str, deps: Vec<Dep>) -> Member {
        Member {
            name: name.into(),
            dir: dir.into(),
            deps,
            features: Vec::new(),
        }
    }

    fn normal(names: &[&str]) -> Vec<Dep> {
        names
            .iter()
            .map(|n| dep(n, DepKind::Normal, &[], None))
            .collect()
    }

    fn fx(list: &[(&str, &[&str])]) -> Metadata {
        metadata(
            list.iter()
                .map(|(n, d)| member(n, dir_for(n), normal(d)))
                .collect(),
        )
    }

    fn fx_at(list: &[(&str, &str, &[&str])]) -> Metadata {
        metadata(
            list.iter()
                .map(|(n, dir, d)| member(n, dir, normal(d)))
                .collect(),
        )
    }

    fn fx_kind_at(name: &str, dir: &str, d: &str, kind: DepKind, features: &[&str]) -> Metadata {
        metadata(vec![member(name, dir, vec![dep(d, kind, features, None)])])
    }

    fn fx_src(list: &[(&str, &[(&str, &str)])]) -> Metadata {
        metadata(
            list.iter()
                .map(|(n, deps)| {
                    let deps = deps
                        .iter()
                        .map(|(d, src)| dep(d, DepKind::Normal, &[], Some(src)))
                        .collect();
                    member(n, dir_for(n), deps)
                })
                .collect(),
        )
    }

    fn fx_feat_kind(name: &str, d: &str, kind: DepKind, features: &[&str]) -> Metadata {
        metadata(vec![member(
            name,
            dir_for(name),
            vec![dep(d, kind, features, None)],
        )])
    }

    fn fx_feat(name: &str, d: &str, features: &[&str]) -> Metadata {
        fx_feat_kind(name, d, DepKind::Normal, features)
    }

    fn lints_ok(_: &Path) -> String {
        "[package]\nname = \"x\"\n\n[lints]\nworkspace = true\n".into()
    }

    fn names(v: &[Violation]) -> Vec<String> {
        v.iter().map(|v| v.krate.clone()).collect()
    }

    fn rule_of(md: &Metadata, krate: &str) -> Rule {
        let v: Vec<_> = violations(md, &lints_ok)
            .into_iter()
            .filter(|v| v.krate == krate)
            .collect();
        assert_eq!(v.len(), 1, "{v:?}");
        v[0].rule
    }

    #[test]
    fn flags_transitive_gpui_through_helper() {
        let md = fx(&[("spike-x", &["helper"]), ("helper", &["gpui"])]);
        let v = violations(&md, &lints_ok);
        assert_eq!(names(&v), ["helper", "spike-x"]);
        assert_eq!(v[1].via, ["helper", "gpui"]);
    }

    #[test]
    fn gpui_through_adapter_is_allowed() {
        let md = fx(&[
            ("spike-x", &["hacivat-ui-core"]),
            ("hacivat-ui-core", &["gpui", "gpui_platform"]),
        ]);
        assert!(violations(&md, &lints_ok).is_empty());
    }

    #[test]
    fn allows_test_support_dev_dependency_on_gpui() {
        let md = fx_kind_at(
            "hacivat-ui",
            "crates/ui",
            "gpui",
            DepKind::Development,
            &["test-support"],
        );
        assert!(violations(&md, &lints_ok).is_empty());
        let md = fx_kind_at(
            "hacivat-app",
            "crates/app",
            "gpui",
            DepKind::Development,
            &["test-support"],
        );
        assert!(violations(&md, &lints_ok).is_empty());
    }

    #[test]
    fn flags_dev_dependency_on_gpui_without_test_support() {
        let md = fx_kind_at("hacivat-ui", "crates/ui", "gpui", DepKind::Development, &[]);
        assert_eq!(rule_of(&md, "hacivat-ui"), Rule::ZedOutsideAdapter);
    }

    #[test]
    fn flags_normal_dependency_on_gpui_even_with_test_support() {
        let md = fx_kind_at(
            "hacivat-ui",
            "crates/ui",
            "gpui",
            DepKind::Normal,
            &["test-support"],
        );
        assert_eq!(rule_of(&md, "hacivat-ui"), Rule::ZedOutsideAdapter);
    }

    #[test]
    fn flags_build_dependency_on_gpui() {
        let md = fx_kind_at(
            "hacivat-app",
            "crates/app",
            "gpui",
            DepKind::Build,
            &["test-support"],
        );
        assert_eq!(rule_of(&md, "hacivat-app"), Rule::ZedOutsideAdapter);
    }

    #[test]
    fn test_support_exception_is_limited_to_ui_and_app() {
        let md = fx_kind_at(
            "hacivat-l10n",
            "crates/core",
            "gpui",
            DepKind::Development,
            &["test-support"],
        );
        assert_eq!(rule_of(&md, "hacivat-l10n"), Rule::ZedOutsideAdapter);
    }

    #[test]
    fn flags_gpui_platform_only() {
        let md = fx(&[("hacivat-app", &["gpui_platform"])]);
        assert_eq!(rule_of(&md, "hacivat-app"), Rule::ZedOutsideAdapter);
    }

    #[test]
    fn flags_any_zed_sourced_crate() {
        let md = fx_src(&[("hacivat-ui", &[("collections", ZED_SOURCES[0])])]);
        assert_eq!(rule_of(&md, "hacivat-ui"), Rule::ZedOutsideAdapter);
    }

    #[test]
    fn flags_fork_sourced_crate() {
        let md = fx_src(&[("hacivat-ui", &[("gpui_util", ZED_SOURCES[1])])]);
        assert_eq!(rule_of(&md, "hacivat-ui"), Rule::ZedOutsideAdapter);
    }

    #[test]
    fn flags_core_reaching_adapter() {
        let md = fx_at(&[("hacivat-l10n", "crates/core", &["hacivat-ui-core"])]);
        assert_eq!(rule_of(&md, "hacivat-l10n"), Rule::CoreReachesUi);
    }

    #[test]
    fn flags_ui_depending_on_core() {
        let md = fx_at(&[
            ("hacivat-ui", "crates/ui", &["hacivat-l10n"]),
            ("hacivat-l10n", "crates/core", &[]),
        ]);
        assert_eq!(rule_of(&md, "hacivat-ui"), Rule::UiDependsOnCore);
    }

    #[test]
    fn app_may_use_core_and_ui() {
        let md = fx_at(&[("hacivat-app", "crates/app", &["hacivat-ui", "hacivat-l10n"])]);
        assert!(violations(&md, &lints_ok).is_empty());
    }

    #[test]
    fn flags_product_enabling_adapter_only_features() {
        for f in ["spike-support", "bench", "scene-check", "test-support"] {
            let md = fx_feat("hacivat", "hacivat-ui-core", &[f]);
            assert_eq!(
                rule_of(&md, "hacivat"),
                Rule::AdapterFeatureInProduct,
                "{f}"
            );
        }
    }

    #[test]
    fn app_dev_dependency_may_enable_adapter_test_support() {
        let md = fx_feat_kind(
            "hacivat-app",
            "hacivat-ui-core",
            DepKind::Development,
            &["test-support"],
        );
        assert!(violations(&md, &lints_ok).is_empty());
    }

    #[test]
    fn flags_member_without_workspace_lints() {
        let md = fx(&[("hacivat-ui", &[])]);
        let v = violations(&md, &|_| "[package]\nname=\"hacivat-ui\"\n".into());
        assert_eq!(v[0].rule, Rule::MissingWorkspaceLints);
    }

    #[test]
    fn flags_core_reaching_any_ui_crate() {
        let md = fx_at(&[
            ("hacivat-l10n", "crates/core", &["hacivat-bench"]),
            ("hacivat-bench", "crates/ui", &[]),
        ]);
        assert_eq!(rule_of(&md, "hacivat-l10n"), Rule::CoreReachesUi);
    }

    #[test]
    fn flags_adapter_depending_on_core() {
        let md = fx_at(&[
            ("hacivat-ui-core", "crates/ui", &["hacivat-l10n"]),
            ("hacivat-l10n", "crates/core", &[]),
        ]);
        assert_eq!(rule_of(&md, "hacivat-ui-core"), Rule::UiDependsOnCore);
    }

    #[test]
    fn flags_ui_reaching_core_through_app_or_spike() {
        for mid in [("spike-y", "spikes"), ("hacivat-app", "crates/app")] {
            let md = fx_at(&[
                ("hacivat-ui", "crates/ui", &[mid.0]),
                (mid.0, mid.1, &["hacivat-l10n"]),
                ("hacivat-l10n", "crates/core", &[]),
            ]);
            let v = violations(&md, &lints_ok);
            let ui: Vec<_> = v.iter().filter(|v| v.krate == "hacivat-ui").collect();
            assert_eq!(ui.len(), 1, "{v:?}");
            assert_eq!(ui[0].rule, Rule::UiDependsOnCore);
            assert_eq!(ui[0].via, [mid.0, "hacivat-l10n"]);
        }
    }

    #[test]
    fn test_support_exception_applies_at_depth_for_app_owner() {
        let md = metadata(vec![
            member("hacivat", "crates/bin", normal(&["hacivat-app"])),
            member(
                "hacivat-app",
                "crates/app",
                vec![dep("gpui", DepKind::Development, &["test-support"], None)],
            ),
        ]);
        assert!(violations(&md, &lints_ok).is_empty());
    }

    #[test]
    fn test_support_exception_is_denied_to_spike_and_tool_owners() {
        for dir in ["spikes", "xtask"] {
            let md = metadata(vec![
                member("root-x", "spikes", normal(&["owner-x"])),
                member(
                    "owner-x",
                    dir,
                    vec![dep("gpui", DepKind::Development, &["test-support"], None)],
                ),
            ]);
            assert_eq!(rule_of(&md, "owner-x"), Rule::ZedOutsideAdapter, "{dir}");
            assert_eq!(rule_of(&md, "root-x"), Rule::ZedOutsideAdapter, "{dir}");
        }
    }

    /// M2b Task 1 (§5.1 rule 3, Q23): `hacivat-guard` stays sans-I/O and runtime-free.
    #[test]
    fn guard_with_tokio_is_flagged() {
        let md = fx_at(&[("hacivat-guard", "crates/core", &["tokio"])]);
        assert_eq!(rule_of(&md, "hacivat-guard"), Rule::GuardNotRuntimeFree);
    }

    #[test]
    fn guard_reaching_a_runtime_crate_transitively_is_flagged() {
        let md = fx_at(&[
            ("hacivat-guard", "crates/core", &["helper"]),
            ("helper", "crates/core", &["futures"]),
        ]);
        let v = violations(&md, &lints_ok);
        assert_eq!(names(&v), ["hacivat-guard"], "{v:?}");
        assert_eq!(v[0].rule, Rule::GuardNotRuntimeFree);
        assert_eq!(v[0].via, ["helper", "futures"]);
    }

    #[test]
    fn guard_dev_dependency_on_tokio_is_allowed() {
        let md = fx_kind_at(
            "hacivat-guard",
            "crates/core",
            "tokio",
            DepKind::Development,
            &[],
        );
        assert!(violations(&md, &lints_ok).is_empty());
        let clean = fx_at(&[("hacivat-guard", "crates/core", &["hacivat-proto"])]);
        assert!(violations(&clean, &lints_ok).is_empty());
    }

    // ## m3a-t01b-seams

    // ## m3a-t01b-seams (fix round 1)
    #[test]
    fn ropey_with_default_features_is_flagged() {
        // The fixture's dependencies keep their default features, as `ropey = "1.6"` does.
        let md = fx_at(&[("spike-editor", "spikes", &["ropey"])]);
        assert_eq!(rule_of(&md, "spike-editor"), Rule::RopeLineBreaks);
        let md = fx_kind_at(
            "hacivat-text",
            "crates/ui",
            "ropey",
            DepKind::Normal,
            &["unicode_lines"],
        );
        assert_eq!(rule_of(&md, "hacivat-text"), Rule::RopeLineBreaks);
    }

    #[test]
    fn a_member_feature_that_turns_on_unicode_lines_is_flagged() {
        let mut md = metadata(vec![member(
            "spike-editor",
            "spikes",
            vec![Dep {
                default_features: false,
                ..dep("ropey", DepKind::Normal, &["cr_lines"], None)
            }],
        )]);
        assert!(violations(&md, &lints_ok).is_empty());
        let spike = md
            .packages
            .iter_mut()
            .find(|p| p.name.as_str() == "spike-editor")
            .unwrap();
        spike
            .features
            .insert("lines".into(), vec!["ropey/unicode_lines".into()]);
        assert_eq!(rule_of(&md, "spike-editor"), Rule::RopeLineBreaks);
    }

    #[test]
    fn the_workspace_ropey_has_no_unicode_line_breaks() {
        let md = cargo_metadata::MetadataCommand::new()
            .current_dir(crate::workspace_root())
            .exec()
            .unwrap();
        let v = violations(&md, &|p| std::fs::read_to_string(p).unwrap_or_default());
        let rope: Vec<_> = v
            .iter()
            .filter(|v| v.rule == Rule::RopeLineBreaks)
            .collect();
        assert!(rope.is_empty(), "{rope:?}");
    }

    #[test]
    fn sql_crate_with_tokio_is_flagged() {
        let md = fx_at(&[("sql-lexer", "crates/sql", &["tokio"])]);
        assert_eq!(rule_of(&md, "sql-lexer"), Rule::SqlNotRuntimeFree);
    }

    #[test]
    fn sql_crate_reaching_a_runtime_transitively_is_flagged() {
        for forbidden in ["futures", "salsa", "hacivat-l10n", "async-std", "smol"] {
            let md = fx_at(&[
                ("sql-split", "crates/sql", &["sql-lexer"]),
                ("sql-lexer", "crates/sql", &[forbidden]),
            ]);
            let v: Vec<_> = violations(&md, &lints_ok)
                .into_iter()
                .filter(|v| v.krate == "sql-split" && v.rule == Rule::SqlNotRuntimeFree)
                .collect();
            assert_eq!(v.len(), 1, "{forbidden}: {v:?}");
            assert_eq!(v[0].via, ["sql-lexer", forbidden]);
        }
    }

    #[test]
    fn sql_testkit_and_sql_dev_dependencies_may_use_a_runtime() {
        let md = fx_at(&[("sql-testkit", "crates/sql", &["tokio"])]);
        assert!(violations(&md, &lints_ok).is_empty());
        let md = fx_kind_at(
            "sql-split",
            "crates/sql",
            "tokio",
            DepKind::Development,
            &[],
        );
        assert!(violations(&md, &lints_ok).is_empty());
    }

    #[test]
    fn sql_crate_depending_on_core_is_flagged() {
        let md = fx_at(&[("sql-split", "crates/sql", &["hacivat-proto"])]);
        assert_eq!(rule_of(&md, "sql-split"), Rule::SqlReachesCore);
    }

    #[test]
    fn sql_crate_dev_depending_on_a_core_member_is_flagged() {
        let md = metadata(vec![
            member(
                "sql-split",
                "crates/sql",
                vec![dep("hacivat-values", DepKind::Development, &[], None)],
            ),
            member("hacivat-values", "crates/core", vec![]),
        ]);
        assert_eq!(rule_of(&md, "sql-split"), Rule::SqlReachesCore);
        // A core crate depending on a SQL crate is the allowed direction.
        let md = fx_at(&[
            ("hacivat-exec", "crates/core", &["sql-split"]),
            ("sql-split", "crates/sql", &[]),
        ]);
        assert!(violations(&md, &lints_ok).is_empty());
    }

    #[test]
    fn pg_query_outside_the_oracle_is_flagged() {
        let md = fx_at(&[("sql-split", "crates/sql", &["pg_query"])]);
        assert_eq!(rule_of(&md, "sql-split"), Rule::BackendOutsideOwner);
        let md = fx_at(&[("sql-oracle-pg", "crates/sql", &["pg_query"])]);
        assert!(violations(&md, &lints_ok).is_empty());
        // Through the oracle is fine; a dev-dependency on pg_query is still direct.
        let md = fx_kind_at(
            "hacivat-exec",
            "crates/core",
            "pg_query",
            DepKind::Development,
            &[],
        );
        assert_eq!(rule_of(&md, "hacivat-exec"), Rule::BackendOutsideOwner);
    }

    // ## m3b-t01a-seams
    #[test]
    fn sqlparser_outside_safety_is_flagged() {
        let md = fx_at(&[("sql-split", "crates/sql", &["sqlparser"])]);
        assert_eq!(rule_of(&md, "sql-split"), Rule::BackendOutsideOwner);
        // A core crate, the oracle and a dev-dependency are flagged alike.
        let md = fx_at(&[("hacivat-exec", "crates/core", &["sqlparser"])]);
        assert_eq!(rule_of(&md, "hacivat-exec"), Rule::BackendOutsideOwner);
        let md = fx_at(&[("sql-oracle-pg", "crates/sql", &["sqlparser"])]);
        assert_eq!(rule_of(&md, "sql-oracle-pg"), Rule::BackendOutsideOwner);
        let md = fx_kind_at(
            "sql-testkit",
            "crates/sql",
            "sqlparser",
            DepKind::Development,
            &[],
        );
        assert_eq!(rule_of(&md, "sql-testkit"), Rule::BackendOutsideOwner);
        // Each backend has its own owner: sql-safety may not take pg_query directly.
        let md = fx_at(&[("sql-safety", "crates/sql", &["pg_query"])]);
        assert_eq!(rule_of(&md, "sql-safety"), Rule::BackendOutsideOwner);
    }

    /// Fix round 1 (M2): `sql_oracle_pg::tree` outside the oracle and `sql-safety` is flagged;
    /// comments are not; the workspace itself is clean.
    #[test]
    fn tree_outside_its_owners_is_flagged() {
        let dir = tempfile::tempdir().unwrap();
        let write = |rel: &str, src: &str| {
            let p = dir.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, src).unwrap();
        };
        write(
            "crates/core/hacivat-exec/tests/t.rs",
            "use sql_oracle_pg::tree::pb::Node;\n",
        );
        write(
            "crates/sql/sql-split/src/a.rs",
            "// sql_oracle_pg::tree is the oracle's\n",
        );
        write(
            "crates/sql/sql-safety/src/backend/pg.rs",
            "use sql_oracle_pg::tree::pb;\n",
        );
        let v = tree_violations(dir.path());
        assert_eq!(names(&v), ["hacivat-exec"]);
        assert_eq!(v[0].rule, Rule::BackendOutsideOwner);
        assert!(tree_violations(&workspace_root()).is_empty());
    }

    /// Fix round 1 (M4): an optional backend dependency that no feature turns on has no edge
    /// in the resolve, but its declaration is still flagged.
    #[test]
    fn a_declared_backend_without_a_resolved_edge_is_flagged() {
        for backend in ["sqlparser", "pg_query"] {
            let mut md = fx_at(&[("sql-split", "crates/sql", &[backend])]);
            for node in &mut md.resolve.as_mut().unwrap().nodes {
                node.deps.clear();
                node.dependencies.clear();
            }
            assert_eq!(
                rule_of(&md, "sql-split"),
                Rule::BackendOutsideOwner,
                "{backend}"
            );
        }
    }

    #[test]
    fn sqlparser_in_safety_is_allowed() {
        let md = fx_at(&[("sql-safety", "crates/sql", &["sqlparser"])]);
        assert!(violations(&md, &lints_ok).is_empty());
        // Through sql-safety is fine (only direct edges count).
        let md = fx_at(&[
            ("hacivat-exec", "crates/core", &["sql-safety"]),
            ("sql-safety", "crates/sql", &["sqlparser"]),
        ]);
        assert!(violations(&md, &lints_ok).is_empty());
    }

    /// The M3b crates are Core-layer members with no violation; `fuzz/` is its own workspace.
    #[test]
    fn m3b_crates_sit_in_their_layers_and_are_clean() {
        let md = cargo_metadata::MetadataCommand::new()
            .current_dir(crate::workspace_root())
            .exec()
            .unwrap();
        let v = violations(&md, &|p| std::fs::read_to_string(p).unwrap_or_default());
        for c in ["sql-ident", "sql-safety"] {
            let member = md
                .workspace_packages()
                .into_iter()
                .find(|p| p.name.as_str() == c)
                .unwrap_or_else(|| panic!("{c} is not a workspace member"));
            assert_eq!(layer_of(&md, member), Some(Layer::Core), "{c}");
            assert!(v.iter().all(|x| x.krate != c), "{c}: {v:?}");
        }
        assert!(
            md.workspace_packages()
                .iter()
                .all(|p| p.name.as_str() != "hacivat-fuzz"),
            "fuzz/ must stay excluded"
        );
        // Only sql-safety reaches sqlparser directly.
        let direct: Vec<&str> = md
            .workspace_packages()
            .iter()
            .filter(|p| p.dependencies.iter().any(|d| d.name == "sqlparser"))
            .map(|p| p.name.as_str())
            .collect();
        assert_eq!(direct, ["sql-safety"]);
    }

    #[test]
    fn text_core_with_the_adapter_is_flagged() {
        let md = fx_at(&[("hacivat-text", "crates/ui", &["hacivat-ui-core"])]);
        assert_eq!(rule_of(&md, "hacivat-text"), Rule::TextCoreNotHeadless);
        let md = fx_src(&[("hacivat-text", &[("gpui_util", ZED_SOURCES[1])])]);
        let v = violations(&md, &lints_ok);
        assert!(
            v.iter().any(|v| v.rule == Rule::TextCoreNotHeadless),
            "{v:?}"
        );
        // The workspace's `ropey` (no default features, `cr_lines`) is clean (fix round 1).
        let clean = metadata(vec![member(
            "hacivat-text",
            "crates/ui",
            vec![Dep {
                default_features: false,
                ..dep("ropey", DepKind::Normal, &["cr_lines", "simd"], None)
            }],
        )]);
        assert!(violations(&clean, &lints_ok).is_empty());
    }

    #[test]
    fn new_ui_and_app_crates_are_product_crates() {
        for c in [
            "hacivat-ui-editor",
            "hacivat-ui-grid",
            "hacivat-sql-editor",
            "hacivat-text",
            "hacivat-ui",
        ] {
            assert!(PRODUCT_CRATES.contains(&c), "{c}");
        }
        let md = fx_feat("hacivat-ui-grid", "hacivat-ui-core", &["test-support"]);
        assert_eq!(
            rule_of(&md, "hacivat-ui-grid"),
            Rule::AdapterFeatureInProduct
        );
    }

    #[test]
    fn unknown_member_directory_fails_closed() {
        let md = fx_at(&[("mystery", "tools", &[])]);
        assert_eq!(rule_of(&md, "mystery"), Rule::UnknownLayer);
    }

    /// M2a Task 1: the nine new crates are workspace members in the Core layer, with no
    /// violation (no UI or GPUI reach, workspace lints on).
    #[test]
    fn new_core_crates_are_core_layer_and_clean() {
        let md = cargo_metadata::MetadataCommand::new()
            .current_dir(crate::workspace_root())
            .exec()
            .unwrap();
        let v = violations(&md, &|p| std::fs::read_to_string(p).unwrap_or_default());
        for c in [
            "hacivat-proto",
            "hacivat-net",
            "hacivat-secrets",
            "hacivat-driver",
            "hacivat-driver-pg",
            "hacivat-driver-mysql",
            "hacivat-driver-sqlite",
            "hacivat-session",
            "hacivat-testkit",
            // M2b Task 1.
            "hacivat-guard",
        ] {
            let member = md
                .workspace_packages()
                .into_iter()
                .find(|p| p.name.as_str() == c);
            let member = member.unwrap_or_else(|| panic!("{c} is not a workspace member"));
            assert_eq!(layer_of(&md, member), Some(Layer::Core), "{c}");
            assert!(v.iter().all(|x| x.krate != c), "{c}: {v:?}");
        }
    }

    // ## m3a-t01b-seams
    /// The M3a crates are members in their layers, with no violation.
    #[test]
    fn m3a_crates_sit_in_their_layers_and_are_clean() {
        let md = cargo_metadata::MetadataCommand::new()
            .current_dir(crate::workspace_root())
            .exec()
            .unwrap();
        let v = violations(&md, &|p| std::fs::read_to_string(p).unwrap_or_default());
        for (c, layer) in [
            ("sql-text", Layer::Core),
            ("sql-dialect", Layer::Core),
            ("sql-lexer", Layer::Core),
            ("sql-split", Layer::Core),
            ("sql-oracle-pg", Layer::Core),
            ("sql-testkit", Layer::Core),
            ("hacivat-values", Layer::Core),
            ("hacivat-results", Layer::Core),
            ("hacivat-exec", Layer::Core),
            ("hacivat-text", Layer::Ui),
            ("hacivat-ui-editor", Layer::Ui),
            ("hacivat-ui-grid", Layer::Ui),
            ("hacivat-sql-editor", Layer::App),
        ] {
            let member = md
                .workspace_packages()
                .into_iter()
                .find(|p| p.name.as_str() == c)
                .unwrap_or_else(|| panic!("{c} is not a workspace member"));
            assert_eq!(layer_of(&md, member), Some(layer), "{c}");
            assert!(v.iter().all(|x| x.krate != c), "{c}: {v:?}");
        }
        // The vendored copies are path patches, never members.
        for vendored in ["pg_query", "tokio-postgres"] {
            assert!(
                md.workspace_packages()
                    .iter()
                    .all(|p| p.name.as_str() != vendored),
                "{vendored}"
            );
        }
    }

    #[test]
    fn clippy_toml_bans_locale_blind_casing() {
        let t = std::fs::read_to_string(workspace_root().join("clippy.toml")).unwrap();
        for m in [
            "str::to_lowercase",
            "str::to_uppercase",
            "char::to_lowercase",
            "char::to_uppercase",
        ] {
            assert!(t.contains(m), "{m}");
        }
    }

    // ## m4-t01a-model: the M4 layering rules (Global Constraints "Layering").

    fn optional(name: &str) -> Dep {
        Dep {
            optional: true,
            ..dep(name, DepKind::Normal, &[], None)
        }
    }

    fn no_defaults(name: &str, features: &[&str]) -> Dep {
        Dep {
            default_features: false,
            ..dep(name, DepKind::Normal, features, None)
        }
    }

    fn feats(list: &[(&str, &[&str])]) -> Vec<(String, Vec<String>)> {
        list.iter()
            .map(|(f, items)| ((*f).into(), items.iter().map(|i| (*i).into()).collect()))
            .collect()
    }

    fn with_features(mut m: Member, list: &[(&str, &[&str])]) -> Member {
        m.features = feats(list);
        m
    }

    /// `hacivat-meta` as M4 declares it: `tokio` behind the default feature `catalog`.
    fn meta_member() -> Member {
        with_features(
            member(
                "hacivat-meta",
                "crates/core",
                vec![
                    optional("tokio"),
                    dep("smallvec", DepKind::Normal, &[], None),
                ],
            ),
            &[("default", &["catalog"]), ("catalog", &["dep:tokio"])],
        )
    }

    fn for_crate(md: &Metadata, krate: &str) -> Vec<Violation> {
        violations(md, &lints_ok)
            .into_iter()
            .filter(|v| v.krate == krate)
            .collect()
    }

    /// `DdlIsPure` reads `hacivat-ddl`'s default-feature graph: `hacivat-meta` with its
    /// defaults reaches `tokio`, without them it does not, although the workspace's unified
    /// resolve keeps the `hacivat-meta -> tokio` edge either way.
    #[test]
    fn ddl_reaching_tokio_is_refused() {
        let md = metadata(vec![
            member(
                "hacivat-ddl",
                "crates/core",
                vec![dep("hacivat-meta", DepKind::Normal, &[], None)],
            ),
            meta_member(),
        ]);
        let v = for_crate(&md, "hacivat-ddl");
        assert_eq!(v.len(), 1, "{v:?}");
        assert_eq!(v[0].rule, Rule::DdlIsPure);
        assert_eq!(v[0].via, ["hacivat-meta", "tokio"]);

        let pure = metadata(vec![
            member(
                "hacivat-ddl",
                "crates/core",
                vec![no_defaults("hacivat-meta", &[])],
            ),
            meta_member(),
        ]);
        assert!(violations(&pure, &lints_ok).is_empty());

        // Turning `catalog` back on by name is the same edge.
        let named = metadata(vec![
            member(
                "hacivat-ddl",
                "crates/core",
                vec![no_defaults("hacivat-meta", &["catalog"])],
            ),
            meta_member(),
        ]);
        assert_eq!(rule_of(&named, "hacivat-ddl"), Rule::DdlIsPure);

        for bad in [
            "tokio",
            "futures",
            "hacivat-session",
            "hacivat-driver",
            "hacivat-driver-pg",
            "hacivat-introspect",
            "hacivat-l10n",
        ] {
            let md = fx_at(&[("hacivat-ddl", "crates/core", &[bad])]);
            assert_eq!(rule_of(&md, "hacivat-ddl"), Rule::DdlIsPure, "{bad}");
        }
    }

    #[test]
    fn ddl_apply_feature_outside_app_is_refused() {
        let md = metadata(vec![
            member(
                "hacivat-introspect",
                "crates/core",
                vec![dep("hacivat-ddl", DepKind::Normal, &["apply"], None)],
            ),
            member(
                "hacivat-app",
                "crates/app",
                vec![dep("hacivat-ddl", DepKind::Normal, &["apply"], None)],
            ),
            member("hacivat-ddl", "crates/core", vec![]),
        ]);
        assert_eq!(
            rule_of(&md, "hacivat-introspect"),
            Rule::DdlApplyOnlyFromApp
        );
        assert!(for_crate(&md, "hacivat-app").is_empty());

        // A dev-dependency counts, and so does a member feature that turns it on.
        let dev = fx_kind_at(
            "hacivat-session",
            "crates/core",
            "hacivat-ddl",
            DepKind::Development,
            &["apply"],
        );
        assert_eq!(rule_of(&dev, "hacivat-session"), Rule::DdlApplyOnlyFromApp);
        let feature = metadata(vec![with_features(
            member(
                "hacivat-exec",
                "crates/core",
                vec![dep("hacivat-ddl", DepKind::Normal, &[], None)],
            ),
            &[("full", &["hacivat-ddl/apply"])],
        )]);
        assert_eq!(rule_of(&feature, "hacivat-exec"), Rule::DdlApplyOnlyFromApp);
    }

    #[test]
    fn meta_reaching_session_is_refused() {
        for bad in [
            "hacivat-session",
            "hacivat-introspect",
            "hacivat-driver",
            "hacivat-driver-mysql",
            "hacivat-exec",
            "hacivat-l10n",
        ] {
            let md = fx_at(&[("hacivat-meta", "crates/core", &[bad])]);
            assert_eq!(
                rule_of(&md, "hacivat-meta"),
                Rule::MetaBelowIntrospect,
                "{bad}"
            );
        }
        // Transitively, through a helper member.
        let md = fx_at(&[
            ("hacivat-meta", "crates/core", &["helper"]),
            ("helper", "crates/core", &["hacivat-session"]),
        ]);
        let v = for_crate(&md, "hacivat-meta");
        assert_eq!(v.len(), 1, "{v:?}");
        assert_eq!(v[0].rule, Rule::MetaBelowIntrospect);
        assert_eq!(v[0].via, ["helper", "hacivat-session"]);
        // Core to SQL is allowed.
        let ok = fx_at(&[(
            "hacivat-meta",
            "crates/core",
            &["sql-catalog", "sql-ident", "hacivat-proto"],
        )]);
        assert!(violations(&ok, &lints_ok).is_empty());
    }

    #[test]
    fn meta_dev_dependency_on_introspect_is_refused() {
        for kind in [DepKind::Development, DepKind::Build] {
            let md = fx_kind_at(
                "hacivat-meta",
                "crates/core",
                "hacivat-introspect",
                kind,
                &[],
            );
            assert_eq!(
                rule_of(&md, "hacivat-meta"),
                Rule::MetaBelowIntrospect,
                "{kind:?}"
            );
        }
    }

    #[test]
    fn embedded_system_outside_app_is_refused() {
        let md = metadata(vec![
            member(
                "hacivat-introspect",
                "crates/core",
                vec![dep(
                    "hacivat-meta",
                    DepKind::Normal,
                    &["embedded-system"],
                    None,
                )],
            ),
            member(
                "hacivat-app",
                "crates/app",
                vec![dep(
                    "hacivat-meta",
                    DepKind::Normal,
                    &["embedded-system"],
                    None,
                )],
            ),
        ]);
        assert_eq!(
            rule_of(&md, "hacivat-introspect"),
            Rule::EmbeddedSystemOnlyFromApp
        );
        assert!(for_crate(&md, "hacivat-app").is_empty());
        let feature = metadata(vec![with_features(
            member(
                "hacivat-ddl",
                "crates/core",
                vec![no_defaults("hacivat-meta", &[])],
            ),
            &[("snapshots", &["hacivat-meta?/embedded-system"])],
        )]);
        assert_eq!(
            rule_of(&feature, "hacivat-ddl"),
            Rule::EmbeddedSystemOnlyFromApp
        );
    }

    /// `hacivat-exec` is refused at any depth; `hacivat-results` as a direct edge (any kind):
    /// `hacivat-session`, which introspection needs, keeps its result store on it.
    #[test]
    fn introspect_reaching_exec_is_refused() {
        for bad in ["hacivat-exec", "hacivat-results"] {
            let md = fx_at(&[("hacivat-introspect", "crates/core", &[bad])]);
            assert_eq!(
                rule_of(&md, "hacivat-introspect"),
                Rule::IntrospectNotExec,
                "{bad}"
            );
            let dev = fx_kind_at(
                "hacivat-introspect",
                "crates/core",
                bad,
                DepKind::Development,
                &[],
            );
            assert_eq!(
                rule_of(&dev, "hacivat-introspect"),
                Rule::IntrospectNotExec,
                "{bad}"
            );
        }
        let deep = fx_at(&[
            ("hacivat-introspect", "crates/core", &["helper"]),
            ("helper", "crates/core", &["hacivat-exec"]),
        ]);
        assert_eq!(
            rule_of(&deep, "hacivat-introspect"),
            Rule::IntrospectNotExec
        );
        let through_session = fx_at(&[
            ("hacivat-introspect", "crates/core", &["hacivat-session"]),
            ("hacivat-session", "crates/core", &["hacivat-results"]),
        ]);
        assert!(violations(&through_session, &lints_ok).is_empty());
        // The other direction is the refresh sink (Task 21).
        let sink = fx_at(&[("hacivat-exec", "crates/core", &["hacivat-introspect"])]);
        assert!(violations(&sink, &lints_ok).is_empty());
    }

    #[test]
    fn sql_catalog_reaching_core_is_refused() {
        let md = fx_at(&[
            ("sql-catalog", "crates/sql", &["hacivat-meta"]),
            ("hacivat-meta", "crates/core", &[]),
        ]);
        assert_eq!(rule_of(&md, "sql-catalog"), Rule::SqlReachesCore);
        let runtime = fx_at(&[("sql-catalog", "crates/sql", &["tokio"])]);
        assert_eq!(rule_of(&runtime, "sql-catalog"), Rule::SqlNotRuntimeFree);
        let ok = fx_at(&[("sql-catalog", "crates/sql", &["sql-ident", "smallvec"])]);
        assert!(violations(&ok, &lints_ok).is_empty());
    }

    fn source_tree(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (rel, src) in files {
            let p = dir.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, src).unwrap();
        }
        dir
    }

    #[test]
    fn meta_query_new_outside_packs_is_refused() {
        let dir = source_tree(&[
            (
                "crates/core/hacivat-introspect/src/pack/render.rs",
                "let q = MetaQuery::new(text, binds);\n",
            ),
            (
                "crates/core/hacivat-session/src/meta.rs",
                "pub fn x() { MetaQuery::new(t, b) }\n",
            ),
            (
                "crates/core/hacivat-introspect/tests/pack.rs",
                "let q = MetaQuery::new(t, b);\n",
            ),
            (
                "crates/core/hacivat-introspect/src/engine/mod.rs",
                "// MetaQuery::new is the packs' only\n",
            ),
            (
                "crates/core/hacivat-introspect/src/pg/mod.rs",
                "    let q = MetaQuery::new(sql, vec![]);\n",
            ),
            (
                "crates/app/hacivat-app/src/explorer/live.rs",
                "MetaQuery::new(user_text, vec![])\n",
            ),
        ]);
        let v: Vec<_> = source_violations(dir.path())
            .into_iter()
            .filter(|v| v.rule == Rule::MetaQueryOnlyFromPacks)
            .collect();
        assert_eq!(names(&v), ["hacivat-app", "hacivat-introspect"]);
        assert_eq!(v[0].via, ["crates/app/hacivat-app/src/explorer/live.rs"]);
        assert_eq!(v[1].via, ["crates/core/hacivat-introspect/src/pg/mod.rs"]);
    }

    /// The M3c carry (m3c.md §8.1, P-M4-17): `hold_counts(` is named by no `src/` file except
    /// its definition; its one caller is the shots harness under `tests/`.
    #[test]
    fn hold_counts_called_from_src_is_refused() {
        let dir = source_tree(&[
            (
                "crates/ui/hacivat-ui-editor/src/find.rs",
                "pub fn hold_counts(cx: &mut App) {}\n",
            ),
            (
                "crates/app/hacivat-app/tests/shots/m3c.rs",
                "hacivat_ui_editor::find::hold_counts(cx);\n",
            ),
            (
                "crates/app/hacivat-app/src/console/mod.rs",
                "/// Never call [`hold_counts`] here.\n",
            ),
            (
                "crates/app/hacivat-app/src/exec_ui/run.rs",
                "        hacivat_ui_editor::find::hold_counts(cx);\n",
            ),
        ]);
        let v: Vec<_> = source_violations(dir.path())
            .into_iter()
            .filter(|v| v.rule == Rule::HoldCountsTestOnly)
            .collect();
        assert_eq!(names(&v), ["hacivat-app"]);
        assert_eq!(v[0].via, ["crates/app/hacivat-app/src/exec_ui/run.rs"]);
    }

    /// The M4 crates are members in their layers and product crates, with no violation; the
    /// workspace passes the M4 source rules.
    #[test]
    fn m4_crates_sit_in_their_layers_and_are_clean() {
        let md = cargo_metadata::MetadataCommand::new()
            .current_dir(crate::workspace_root())
            .exec()
            .unwrap();
        let v = violations(&md, &|p| std::fs::read_to_string(p).unwrap_or_default());
        for c in [
            "hacivat-meta",
            "hacivat-introspect",
            "hacivat-ddl",
            "sql-catalog",
        ] {
            let member = md
                .workspace_packages()
                .into_iter()
                .find(|p| p.name.as_str() == c)
                .unwrap_or_else(|| panic!("{c} is not a workspace member"));
            assert_eq!(layer_of(&md, member), Some(Layer::Core), "{c}");
            assert!(PRODUCT_CRATES.contains(&c), "{c}");
            assert!(v.iter().all(|x| x.krate != c), "{c}: {v:?}");
        }
        assert!(
            source_violations(&workspace_root()).is_empty(),
            "{:?}",
            source_violations(&workspace_root())
        );
    }

    // ## m4-t01a-model (fix round 1): the reviewer's hostile forms, one test each.

    fn renamed(name: &str, key: &str, features: &[&str]) -> Dep {
        Dep {
            rename: Some(key.into()),
            ..dep(name, DepKind::Normal, features, None)
        }
    }

    /// Drops every resolve edge, as for an optional dependency no member turns on: the cargo
    /// resolve then has no edge for it, and only the manifests show it.
    fn without_resolve_edges(mut md: Metadata) -> Metadata {
        for node in &mut md.resolve.as_mut().unwrap().nodes {
            node.deps.clear();
            node.dependencies.clear();
        }
        md
    }

    /// I1 / H4: a renamed `hacivat-meta` whose `embedded-system` comes on through `default`.
    #[test]
    fn embedded_system_through_a_renamed_dependency_and_default_is_refused() {
        let md = metadata(vec![with_features(
            member(
                "hacivat-introspect",
                "crates/core",
                vec![renamed("hacivat-meta", "meta", &[])],
            ),
            &[("default", &["snap"]), ("snap", &["meta/embedded-system"])],
        )]);
        assert_eq!(
            rule_of(&md, "hacivat-introspect"),
            Rule::EmbeddedSystemOnlyFromApp
        );
        // Weak form and `dep:` plus a renamed feature list: same.
        let weak = metadata(vec![with_features(
            member(
                "hacivat-exec",
                "crates/core",
                vec![Dep {
                    optional: true,
                    ..renamed("hacivat-meta", "m", &["embedded-system"])
                }],
            ),
            &[("snapshots", &["dep:m"])],
        )]);
        assert_eq!(
            rule_of(&weak, "hacivat-exec"),
            Rule::EmbeddedSystemOnlyFromApp
        );
    }

    /// I1 / H4b: an alias feature declared in the owner (`snapshots = ["embedded-system"]`),
    /// enabled by name by another member.
    #[test]
    fn embedded_system_through_an_owner_alias_feature_is_refused() {
        let md = metadata(vec![
            with_features(
                member("hacivat-meta", "crates/core", vec![]),
                &[
                    ("snapshots", &["embedded-system"]),
                    ("embedded-system", &[]),
                ],
            ),
            member(
                "hacivat-introspect",
                "crates/core",
                vec![no_defaults("hacivat-meta", &["snapshots"])],
            ),
        ]);
        assert_eq!(
            rule_of(&md, "hacivat-introspect"),
            Rule::EmbeddedSystemOnlyFromApp
        );
        assert!(for_crate(&md, "hacivat-meta").is_empty());
        // The owner's `default` naming the alias counts for a user that keeps defaults on.
        let by_default = metadata(vec![
            with_features(
                member("hacivat-meta", "crates/core", vec![]),
                &[
                    ("default", &["snapshots"]),
                    ("snapshots", &["embedded-system"]),
                    ("embedded-system", &[]),
                ],
            ),
            member(
                "hacivat-introspect",
                "crates/core",
                vec![dep("hacivat-meta", DepKind::Normal, &[], None)],
            ),
        ]);
        assert_eq!(
            rule_of(&by_default, "hacivat-introspect"),
            Rule::EmbeddedSystemOnlyFromApp
        );
        // A crate that reaches the feature only through `hacivat-app` (the binary) is clean.
        let bin = metadata(vec![
            member(
                "hacivat-app",
                "crates/app",
                vec![dep(
                    "hacivat-meta",
                    DepKind::Normal,
                    &["embedded-system"],
                    None,
                )],
            ),
            member("hacivat", "crates/bin", normal(&["hacivat-app"])),
        ]);
        assert!(violations(&bin, &lints_ok).is_empty());
    }

    /// I1 / H5: a renamed `hacivat-ddl` whose `apply` comes on through `default`.
    #[test]
    fn ddl_apply_through_a_renamed_dependency_and_default_is_refused() {
        let md = metadata(vec![with_features(
            member(
                "hacivat-exec",
                "crates/core",
                vec![renamed("hacivat-ddl", "d", &[])],
            ),
            &[("default", &["mig"]), ("mig", &["d/apply"])],
        )]);
        assert_eq!(rule_of(&md, "hacivat-exec"), Rule::DdlApplyOnlyFromApp);
        // Through an alias feature of the owner.
        let alias = metadata(vec![
            with_features(
                member("hacivat-ddl", "crates/core", vec![]),
                &[("migrate", &["apply"]), ("apply", &[])],
            ),
            member(
                "hacivat-session",
                "crates/core",
                vec![dep("hacivat-ddl", DepKind::Normal, &["migrate"], None)],
            ),
        ]);
        assert_eq!(
            rule_of(&alias, "hacivat-session"),
            Rule::DdlApplyOnlyFromApp
        );
    }

    /// I2 / H2d: a non-default, non-`apply` feature that pulls a runtime.
    #[test]
    fn ddl_feature_other_than_apply_reaching_tokio_is_refused() {
        let md = metadata(vec![with_features(
            member("hacivat-ddl", "crates/core", vec![optional("tokio")]),
            &[("async", &["dep:tokio"]), ("apply", &[])],
        )]);
        let v = for_crate(&md, "hacivat-ddl");
        assert_eq!(v.len(), 1, "{v:?}");
        assert_eq!(v[0].rule, Rule::DdlIsPure);
        assert_eq!(v[0].via, ["tokio"]);
        // `apply` itself may (R-M4-35), also when another feature only names `apply`.
        let apply = metadata(vec![with_features(
            member("hacivat-ddl", "crates/core", vec![optional("tokio")]),
            &[("apply", &["dep:tokio"]), ("full", &["apply"])],
        )]);
        assert!(violations(&apply, &lints_ok).is_empty());
        // Dormant in the workspace resolve (no edge) is still seen.
        assert_eq!(
            rule_of(&without_resolve_edges(md), "hacivat-ddl"),
            Rule::DdlIsPure
        );
    }

    /// m6 / H2c: executor and runtime crates by rule, not by exact name.
    #[test]
    fn ddl_reaching_an_executor_is_refused() {
        for bad in [
            "futures-executor",
            "futures-util",
            "tokio-util",
            "async-std",
            "smol",
            "async-executor",
        ] {
            let md = fx_at(&[("hacivat-ddl", "crates/core", &[bad])]);
            assert_eq!(rule_of(&md, "hacivat-ddl"), Rule::DdlIsPure, "{bad}");
        }
        let ok = fx_at(&[("hacivat-ddl", "crates/core", &["smallvec", "sql-ident"])]);
        assert!(violations(&ok, &lints_ok).is_empty());
    }

    /// I2 / H3: an optional `hacivat-session` behind a feature nothing turns on (no resolve
    /// edge), and the same two hops down.
    #[test]
    fn meta_dormant_optional_dependency_is_refused() {
        let md = without_resolve_edges(metadata(vec![with_features(
            member(
                "hacivat-meta",
                "crates/core",
                vec![optional("hacivat-session")],
            ),
            &[("live", &["dep:hacivat-session"])],
        )]));
        assert_eq!(rule_of(&md, "hacivat-meta"), Rule::MetaBelowIntrospect);
        let deep = without_resolve_edges(metadata(vec![
            with_features(
                member("hacivat-meta", "crates/core", vec![optional("helper")]),
                &[("x", &["dep:helper"])],
            ),
            with_features(
                member("helper", "crates/core", vec![optional("hacivat-driver-pg")]),
                &[("default", &["pg"]), ("pg", &["dep:hacivat-driver-pg"])],
            ),
        ]));
        let v = for_crate(&deep, "hacivat-meta");
        assert_eq!(v.len(), 1, "{v:?}");
        assert_eq!(v[0].via, ["helper", "hacivat-driver-pg"]);
    }

    fn meta_query_findings(files: &[(&str, &str)]) -> Vec<Violation> {
        let dir = source_tree(files);
        source_violations(dir.path())
            .into_iter()
            .filter(|v| v.rule == Rule::MetaQueryOnlyFromPacks)
            .collect()
    }

    const ENGINE: &str = "crates/core/hacivat-introspect/src/engine/mod.rs";

    #[test]
    fn meta_query_alias_with_use_as_is_refused() {
        let v = meta_query_findings(&[(
            ENGINE,
            "use hacivat_session::meta::MetaQuery as Q;\nfn f() { Q::new(t, b); }\n",
        )]);
        assert_eq!(names(&v), ["hacivat-introspect"]);
    }

    #[test]
    fn meta_query_qualified_path_form_is_refused() {
        let v = meta_query_findings(&[(
            ENGINE,
            "fn f() { <hacivat_session::meta::MetaQuery>::new(t, b); }\n",
        )]);
        assert_eq!(names(&v), ["hacivat-introspect"]);
    }

    #[test]
    fn meta_query_new_with_spaces_is_refused() {
        let v = meta_query_findings(&[(ENGINE, "fn f() { MetaQuery :: new(t, b); }\n")]);
        assert_eq!(names(&v), ["hacivat-introspect"]);
    }

    #[test]
    fn meta_query_new_split_over_lines_and_comments_is_refused() {
        let v =
            meta_query_findings(&[(ENGINE, "fn f() { MetaQuery::\n    /* x */ new(t, b); }\n")]);
        assert_eq!(names(&v), ["hacivat-introspect"]);
    }

    #[test]
    fn meta_query_type_alias_is_refused() {
        let v = meta_query_findings(&[(
            ENGINE,
            "type M = hacivat_session::meta::MetaQuery;\nfn f() { M::new(t, b); }\n",
        )]);
        assert_eq!(names(&v), ["hacivat-introspect"]);
    }

    /// `#[path]` from `src/` into the exempt `tests/`: refused as an escape, and the target's
    /// code counts as the library's.
    #[test]
    fn meta_query_through_path_into_tests_is_refused() {
        let v = meta_query_findings(&[
            (
                "crates/core/hacivat-introspect/src/lib.rs",
                "#[path = \"../tests/evil_impl.rs\"]\nmod evil;\n",
            ),
            (
                "crates/core/hacivat-introspect/tests/evil_impl.rs",
                "pub fn q() { MetaQuery::new(t, b) }\n",
            ),
        ]);
        let via: Vec<&str> = v.iter().map(|x| x.via[0].as_str()).collect();
        assert_eq!(
            via,
            [
                "crates/core/hacivat-introspect/src/lib.rs",
                "crates/core/hacivat-introspect/src/lib.rs: #[path] outside src/ crates/core/hacivat-introspect/tests/evil_impl.rs",
            ]
        );
    }

    #[test]
    fn meta_query_through_include_of_a_non_rs_file_is_refused() {
        let v = meta_query_findings(&[
            (ENGINE, "include!(\"raw.inc\");\n"),
            (
                "crates/core/hacivat-introspect/src/engine/raw.inc",
                "fn q() { MetaQuery::new(t, b) }\n",
            ),
        ]);
        let via: Vec<&str> = v.iter().map(|x| x.via[0].as_str()).collect();
        assert_eq!(
            via,
            [
                ENGINE,
                "crates/core/hacivat-introspect/src/engine/mod.rs: include! of a non-.rs file crates/core/hacivat-introspect/src/engine/raw.inc",
            ]
        );
    }

    // ## m4-t04-packs
    fn pack_parse_findings(files: &[(&str, &str)]) -> Vec<Violation> {
        let dir = source_tree(files);
        source_violations(dir.path())
            .into_iter()
            .filter(|v| v.rule == Rule::PackParseOnlyFromPacks)
            .collect()
    }

    /// T4 fix round 1, I2: runtime text never becomes a renderable `Pack` outside the packs.
    #[test]
    fn pack_parse_outside_packs_is_refused() {
        for src in [
            "fn f(t: &'static str) { let _ = Pack::parse(d, t); }\n",
            "fn f() { let _ = hacivat_introspect::pack::Pack::parse_all(d, &F); }\n",
            "fn f() { let _ = Pack :: /* x */ parse(d, Box::leak(s)); }\n",
            "fn f() { let _ = <Pack>::parse(d, t); }\n",
            "fn f() { let _ = <(Pack)>::parse(d, t); }\n",
            "fn f() { let g = Pack::parse; }\n",
            "use hacivat_introspect::pack::Pack as P;\nfn f() { P::parse(d, t); }\n",
            "type P = hacivat_introspect::pack::Pack;\nfn f() { P::parse(d, t); }\n",
            "trait T { fn g(); }\nimpl T for Pack { fn g() { Self::parse(d, t); } }\n",
            "macro_rules! mk { ($t:ty) => { <$t>::parse(d, t) } }\nfn f() { mk!(Pack); }\n",
            "fn f() { let _ = Pack::render(&p, s, c); }\n",
        ] {
            let v = pack_parse_findings(&[(ENGINE, src)]);
            assert_eq!(names(&v), ["hacivat-introspect"], "{src}");
        }
        let v = pack_parse_findings(&[
            (
                "crates/app/hacivat-app/src/x.rs",
                "fn f() { Pack::parse(d, t); }\n",
            ),
            (
                "fuzz/fuzz_targets/evil.rs",
                "fn f() { Pack::parse(d, t); }\n",
            ),
            ("xtask/src/evil.rs", "fn f() { Pack::parse_all(d, &f); }\n"),
        ]);
        assert_eq!(names(&v), ["fuzz", "hacivat-app", "xtask"]);
    }

    #[test]
    fn pack_builtin_types_and_allowed_places_pass() {
        let v = pack_parse_findings(&[
            (
                ENGINE,
                "use hacivat_introspect::pack::{Pack, DraftPack};\n\
                 fn f(p: &Pack) -> Option<Pack> { let b = Pack::builtin(d)?; \
                 assert!(Pack::builtin(d).is_ok()); let x = DraftPack::parse(d, &s); \
                 let s = \"Pack::parse\"; None }\n",
            ),
            (
                "crates/core/hacivat-introspect/src/pack/mod.rs",
                "fn f() { Pack::parse_all(d, &F) }\n",
            ),
            (
                "crates/core/hacivat-introspect/src/pg/mod.rs",
                "fn f() { Pack::parse(d, include_str!(\"x\")) }\n",
            ),
            (
                "crates/core/hacivat-introspect/tests/pack.rs",
                "fn f() { Pack::parse(d, S) }\n",
            ),
            (
                "fuzz/meta/fuzz_targets/pack_parse.rs",
                "fn f() { Pack::parse(d, S) }\n",
            ),
        ]);
        assert_eq!(v, Vec::new());
    }

    // ## m4-hyg-tmp
    fn temp_findings(files: &[(&str, &str)]) -> Vec<Violation> {
        let dir = source_tree(files);
        source_violations(dir.path())
            .into_iter()
            .filter(|v| v.rule == Rule::TempDirOutlivesTest)
            .collect()
    }

    /// R-W5-15: the forms that left ~58k `.tmp*` directories a day behind.
    #[test]
    fn a_temp_dir_that_outlives_its_test_is_refused() {
        for src in [
            "fn tmp() -> PathBuf { tempfile::tempdir().unwrap().keep() }\n",
            "fn tmp() -> PathBuf { tempdir().keep().join(name) }\n",
            "fn tmp() -> PathBuf { let d = tempdir(); d.into_path() }\n",
            "fn tmp() { let mut d = tempdir(); d . /* x */ disable_cleanup(true); }\n",
            "fn f() { let f = NamedTempFile::new().unwrap(); let (_, p) = f.keep().unwrap(); }\n",
            "fn f() { let root: &'static tempfile::TempDir = Box::leak(Box::new(root)); }\n",
            "fn f() { std::mem::forget(tempfile::tempdir().unwrap()); }\n",
            "fn dir() -> &'static Path {\n    static DIR: OnceLock<TempDir> = OnceLock::new();\n    \
             DIR.get_or_init(|| tempfile::tempdir().unwrap()).path()\n}\n",
            "pub(crate) static D: LazyLock<tempfile::TempDir> = LazyLock::new(make);\n",
            "#[derive(Default)]\nstruct TempRoots(Vec<tempfile::TempDir>);\nimpl Global for TempRoots {}\n",
            "struct Kept {\n    roots: Vec<TempDir>,\n}\nimpl gpui::Global for Kept {}\n",
        ] {
            let v = temp_findings(&[("crates/app/hacivat-app/tests/support/mod.rs", src)]);
            assert_eq!(names(&v), ["hacivat-app"], "{src}");
        }
        let v = temp_findings(&[
            (
                "crates/core/hacivat-settings/src/store.rs",
                "fn t() { tempdir().unwrap().keep(); }\n",
            ),
            (
                "fuzz/fuzz_targets/evil.rs",
                "fn f() { tempdir().unwrap().keep(); }\n",
            ),
            (
                "xtask/src/evil.rs",
                "fn f() { tempdir().unwrap().into_path(); }\n",
            ),
        ]);
        assert_eq!(names(&v), ["fuzz", "hacivat-settings", "xtask"]);
    }

    #[test]
    fn temp_dirs_dropped_by_the_test_pass() {
        let v = temp_findings(&[(
            "crates/app/hacivat-app/tests/support/mod.rs",
            "// tempdir().keep()\nconst S: &str = \"tempdir().unwrap().keep()\";\n\
             thread_local! {\n    static DIRS: RefCell<Vec<tempfile::TempDir>> = \
             const { RefCell::new(Vec::new()) };\n    static DIR: TempDir = tempdir().unwrap();\n}\n\
             fn tmp() -> PathBuf { let d = tempdir().unwrap(); let p = d.path().to_path_buf(); \
             DIRS.with(|x| x.borrow_mut().push(d)); p }\n\
             fn dir() -> &'static Path { static N: AtomicUsize = AtomicUsize::new(0); x() }\n\
             fn g(d: &'static TempDir) {}\n\
             async fn h(&mut self) { self.keep(wrap).await; keep(name); }\n\
             fn s() -> &'static str { Box::leak(s.into_boxed_str()) }\n\
             struct Kept { listeners: Vec<TcpListener> }\nimpl Global for Kept {}\n\
             struct Tmp { _dir: TempDir, path: PathBuf }\n",
        )]);
        assert_eq!(v, Vec::new());
    }

    #[test]
    fn meta_query_in_fuzz_and_xtask_is_refused() {
        let v = meta_query_findings(&[
            (
                "fuzz/fuzz_targets/evil.rs",
                "fuzz_target!(|d: &[u8]| { let _ = MetaQuery::new(d, vec![]); });\n",
            ),
            ("xtask/src/evil.rs", "fn f() { MetaQuery::new(t, b); }\n"),
            // Strings never match (xtask's own rule tests write them).
            (
                "xtask/src/rules.rs",
                "const T: &str = \"MetaQuery::new\";\n",
            ),
        ]);
        assert_eq!(names(&v), ["fuzz", "xtask"]);
    }

    #[test]
    fn hold_counts_alias_is_refused() {
        let dir = source_tree(&[(
            "crates/app/hacivat-app/src/exec_ui/run.rs",
            "use hacivat_ui_editor::find::hold_counts as hc;\nfn f() { hc(cx); }\n",
        )]);
        let v: Vec<_> = source_violations(dir.path())
            .into_iter()
            .filter(|v| v.rule == Rule::HoldCountsTestOnly)
            .collect();
        assert_eq!(names(&v), ["hacivat-app"]);
    }

    /// What the scan must leave alone: strings, comments, generated `include!`s, `#[path]`
    /// inside `src/`, and a fuzz `#[path]` into a clean test helper.
    #[test]
    fn source_rules_leave_strings_comments_and_clean_includes_alone() {
        let dir = source_tree(&[
            (
                ENGINE,
                "// MetaQuery::new\n/// MetaQuery::new\nconst S: &str = \"MetaQuery::new\";\n\
                 const R: &str = r#\"MetaQuery::new\"#;\nconst C: char = '\"';\n\
                 include!(concat!(env!(\"OUT_DIR\"), \"/ids.rs\"));\n\
                 #[path = \"generated/ids.rs\"]\nmod ids;\nfn f<'a>(x: &'a MetaQuery) {}\n",
            ),
            (
                "crates/core/hacivat-introspect/src/engine/generated/ids.rs",
                "pub const A: u8 = 1;\n",
            ),
            (
                "fuzz/fuzz_targets/relex.rs",
                "#[path = \"../../crates/sql/sql-lexer/tests/support/edits.rs\"]\nmod edits;\n",
            ),
            (
                "crates/sql/sql-lexer/tests/support/edits.rs",
                "pub fn edit() {}\n",
            ),
        ]);
        assert!(
            source_violations(dir.path()).is_empty(),
            "{:?}",
            source_violations(dir.path())
        );
    }

    // ## m4-t01a-model (fix round 2): the re-review's forms, one test each.

    /// I-R1 / H7b: `default` turning `apply` (and through it a runtime) on.
    #[test]
    fn ddl_default_turning_on_apply_is_refused() {
        let md = metadata(vec![with_features(
            member("hacivat-ddl", "crates/core", vec![optional("tokio")]),
            &[("default", &["apply"]), ("apply", &["dep:tokio"])],
        )]);
        let v = for_crate(&md, "hacivat-ddl");
        assert_eq!(v.len(), 1, "{v:?}");
        assert_eq!(v[0].rule, Rule::DdlIsPure);
        assert_eq!(v[0].via, ["[features] default -> apply"]);
        // Through another default feature too.
        let chain = metadata(vec![with_features(
            member("hacivat-ddl", "crates/core", vec![]),
            &[("default", &["std"]), ("std", &["apply"]), ("apply", &[])],
        )]);
        assert_eq!(rule_of(&chain, "hacivat-ddl"), Rule::DdlIsPure);
    }

    /// I-R2 (a): a local trait implemented for `MetaQuery` (its methods can call `Self::new`).
    #[test]
    fn meta_query_local_trait_impl_is_refused() {
        let v = meta_query_findings(&[(
            ENGINE,
            "trait Mk { fn mk() -> Self; }\n\
             impl Mk for hacivat_session::meta::MetaQuery { fn mk() -> Self { Self::new(t, b) } }\n",
        )]);
        assert_eq!(names(&v), ["hacivat-introspect"]);
    }

    /// I-R2 (b): `MetaQuery` handed to a macro that builds it (`$t::new`).
    #[test]
    fn meta_query_passed_to_a_macro_is_refused() {
        let v = meta_query_findings(&[(
            ENGINE,
            "macro_rules! mk { ($t:ident) => { $t::new(a, b) }; }\nfn f() { mk!(MetaQuery); }\n",
        )]);
        assert_eq!(names(&v), ["hacivat-introspect"]);
    }

    /// I-R2 (c): the raw identifier `r#new`.
    #[test]
    fn meta_query_raw_identifier_new_is_refused() {
        let v = meta_query_findings(&[(ENGINE, "fn f() { MetaQuery::r#new(t, b); }\n")]);
        assert_eq!(names(&v), ["hacivat-introspect"]);
    }

    /// Prefixed raw strings stay strings, and a raw identifier elsewhere is harmless.
    #[test]
    fn prefixed_raw_strings_and_raw_identifiers_are_not_code() {
        let dir = source_tree(&[(
            ENGINE,
            "const A: &[u8] = br#\"MetaQuery::new \" x\"#;\nconst B: &str = r\"MetaQuery::new\";\n\
             fn r#type() {}\nfn f() { println!(\"{}\", 1); }\n",
        )]);
        assert!(
            source_violations(dir.path()).is_empty(),
            "{:?}",
            source_violations(dir.path())
        );
    }

    /// Fix round 2 (CI 44514966689): `hacivat-introspect` without its default feature `session`
    /// (what `fuzz/meta` builds under the sanitizer) never reaches a session, the network or SSH
    /// stack, or the keyring; with it, it does (so the feature is what keeps them out).
    #[test]
    fn introspect_without_session_never_reaches_the_session_stack() {
        let md = cargo_metadata::MetadataCommand::new()
            .current_dir(crate::workspace_root())
            .exec()
            .unwrap();
        let pkgs: HashMap<&PackageId, &Package> = md.packages.iter().map(|p| (&p.id, p)).collect();
        let edges = edges_of(&md);
        let introspect = md
            .workspace_packages()
            .into_iter()
            .find(|p| p.name.as_str() == INTROSPECT)
            .unwrap();
        let session_stack = |n: &str| {
            matches!(
                n,
                "hacivat-session" | "hacivat-net" | "hacivat-secrets" | "hacivat-settings"
            ) || n == "russh"
                || n == "keyring"
                || is_driver(n)
        };
        let walk = |features: &[&str]| {
            feature_walk(
                &introspect.id,
                &edges,
                &pkgs,
                WalkStart {
                    features: features.iter().map(|f| (*f).to_string()).collect(),
                    root_kinds: &[DependencyKind::Normal],
                    blocked: None,
                },
                |_| false,
                session_stack,
            )
            .bad_path
        };
        assert_eq!(walk(&[]), None);
        assert!(walk(&["default"]).is_some());
    }

    // ## m4-t01a-model (fix round 3): re-review 2 and its addendum.

    /// I-R4: generic arguments between `MetaQuery` and `::new` (the turbofish and the
    /// qualified-path forms), one finding each.
    #[test]
    fn meta_query_turbofish_and_generic_forms_are_refused() {
        for form in [
            "fn f(s: String) { let _ = MetaQuery::<'_>::new(s, vec![]); }\n",
            "fn f(s: String) { let _ = MetaQuery :: < '_ > :: new(s, vec![]); }\n",
            "fn f(s: String) { let _ = MetaQuery::<>::new(s, vec![]); }\n",
            "fn f(s: String) { let _ = <MetaQuery<'_>>::new(s, vec![]); }\n",
            "fn f(s: String) { let _ = <hacivat_session::meta::MetaQuery>::joinable(s); }\n",
            "fn f() { let _ = MetaQuery::DEFAULT; }\n",
        ] {
            let v = meta_query_findings(&[(ENGINE, form)]);
            assert_eq!(names(&v), ["hacivat-introspect"], "{form}");
        }
    }

    /// I-R4 and m-R2-2: `MetaQuery` in type positions, and `impl Trait` in return and argument
    /// position, are legal.
    #[test]
    fn meta_query_in_type_positions_is_allowed() {
        let v = meta_query_findings(&[(
            ENGINE,
            "use hacivat_session::meta::MetaQuery;\n\
             fn a(q: &MetaQuery) -> usize { 0 }\n\
             fn b(v: Vec<MetaQuery>) -> Vec<MetaQuery> { v }\n\
             fn c(q: MetaQuery) -> Option<MetaQuery> { Some(q) }\n\
             fn qs(v: &[MetaQuery]) -> impl Iterator<Item = &MetaQuery> { v.iter() }\n\
             fn one(q: impl Into<MetaQuery>) -> MetaQuery { q.into() }\n\
             fn d() -> Vec<MetaQuery> { Vec::<MetaQuery>::new() }\n\
             fn e() -> Vec<MetaQuery> { Vec::new() }\n\
             struct S<'a> { q: &'a MetaQuery, m: std::collections::HashMap<String, Vec<MetaQuery>> }\n\
             fn g(x: Box<dyn Fn(&MetaQuery) -> bool>) {}\n",
        )]);
        assert!(v.is_empty(), "{v:?}");
    }

    /// I-R3 / H13: `path = "…"` inside `#[cfg_attr(…)]` (nested too) is a `#[path]` target.
    #[test]
    fn meta_query_through_cfg_attr_path_into_tests_is_refused() {
        for attr in [
            "#[cfg_attr(feature = \"session\", path = \"../tests/evil_impl.rs\")]",
            "#[cfg_attr(all(), cfg_attr(unix, path = \"../tests/evil_impl.rs\"))]",
        ] {
            let v = meta_query_findings(&[
                (
                    "crates/core/hacivat-introspect/src/filter.rs",
                    &format!("{attr}\nmod evil;\n"),
                ),
                (
                    "crates/core/hacivat-introspect/tests/evil_impl.rs",
                    "pub fn q(s: String) -> MetaQuery { MetaQuery::new(s, vec![]) }\n",
                ),
            ]);
            let via: Vec<&str> = v.iter().map(|x| x.via[0].as_str()).collect();
            assert_eq!(
                via,
                [
                    "crates/core/hacivat-introspect/src/filter.rs",
                    "crates/core/hacivat-introspect/src/filter.rs: #[path] outside src/ crates/core/hacivat-introspect/tests/evil_impl.rs",
                ],
                "{attr}"
            );
        }
    }

    /// m-R2-1 / H12: an optional `hacivat-results` or `hacivat-exec` behind a feature nothing
    /// turns on (no resolve edge).
    #[test]
    fn introspect_dormant_optional_results_or_exec_is_refused() {
        for bad in ["hacivat-results", "hacivat-exec"] {
            let item = format!("dep:{bad}");
            let md = without_resolve_edges(metadata(vec![with_features(
                member("hacivat-introspect", "crates/core", vec![optional(bad)]),
                &[("exec", &[item.as_str()])],
            )]));
            let v = for_crate(&md, "hacivat-introspect");
            assert_eq!(v.len(), 1, "{bad}: {v:?}");
            assert_eq!(v[0].rule, Rule::IntrospectNotExec);
            assert_eq!(v[0].via, [bad]);
        }
    }

    // ## m4-t01a-model (fix round 4): re-review 3, I-R5.

    /// I-R5: `MetaQuery` wrapped inside a qualified path's `<…>` (parentheses, a tuple, a
    /// reference, its own generic arguments; after `=>` too) before `::`.
    #[test]
    fn meta_query_wrapped_in_a_qualified_path_is_refused() {
        for form in [
            "fn f(s: &str) { let _ = <(hacivat_session::meta::MetaQuery)>::new(&s, &[], true); }\n",
            "fn f(s: &str) { let _ = <(hacivat_session::meta::MetaQuery<'_>)>::new(&s, &[], true); }\n",
            "fn f(s: &str) { let _ = <((MetaQuery))>::new(&s, &[], true); }\n",
            "fn f() { let _ = <(MetaQuery,)>::default(); }\n",
            "fn f(q: &MetaQuery) { let _ = <&MetaQuery>::clone(&q); }\n",
            "fn f(q: &MetaQuery) { let _ = <&mut dyn Into<MetaQuery>>::into(q); }\n",
            "fn f(x: u8, s: &str) { match x { _ => <(MetaQuery)>::new(s, &[], true) }; }\n",
            "fn f() -> u8 { let _ = <Vec<MetaQuery>>::new(); 0 }\n",
        ] {
            let v = meta_query_findings(&[(ENGINE, form)]);
            assert_eq!(names(&v), ["hacivat-introspect"], "{form}");
        }
    }

    /// I-R5: qualified paths that do not hold `MetaQuery`, and tuples of it in type positions,
    /// stay legal.
    #[test]
    fn qualified_paths_without_meta_query_and_tuple_types_are_allowed() {
        let v = meta_query_findings(&[(
            ENGINE,
            "fn a(p: (MetaQuery, u8)) -> (MetaQuery,) { (p.0,) }\n\
             fn b() -> Vec<(MetaQuery, String)> { <Vec<(u8, u8)>>::new(); Vec::new() }\n\
             fn c(x: u8) -> u8 { match x { _ => <u8>::default() } }\n\
             fn d(q: &MetaQuery) -> &MetaQuery { q }\n",
        )]);
        assert!(v.is_empty(), "{v:?}");
    }
}
