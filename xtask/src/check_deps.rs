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
const PRODUCT_CRATES: [&str; 13] = [
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
    // ## m4b-t01a-seams (R-M4b-41): the script workspace and its git status.
    "hacivat-scripts",
    "hacivat-git",
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
    // ## m4-t33a-hygiene (T23 review m-6)
    /// A workflow or composite action under `.github/` uses a third-party action by tag or
    /// branch instead of a full commit SHA (`owner/repo[/path]@<40 hex>`, with the tag as a
    /// trailing comment), or a `docker://` image without `@sha256:<64 hex>`. Local actions
    /// (`./…`) are exempt. A `uses` key counts wherever it stands on a line (block or flow
    /// mapping, quoted or not, `uses :`), and a file that cannot be read is a violation too
    /// (fix round 1, review M3). A tag can be moved to other code; a SHA cannot.
    ActionNotPinnedBySha,
    // ## m4-t33a-hygiene
    // ## m4b-t01a-seams
    /// `hacivat-scripts` reaches `hacivat-exec`, `hacivat-meta`, `hacivat-session`,
    /// `hacivat-introspect`, `hacivat-net`, `hacivat-secrets`, a driver, `russh` or `rustls`
    /// through its normal or build dependencies (declared, optional included, or resolved, at
    /// any depth; build edges since fix round 1, M2); or `hacivat-exec` depends on
    /// `hacivat-scripts` by any edge (R-M4b-28, C-M4-6: the fuzz package and every test build
    /// would compile the session stack).
    ScriptsBelowExec,
    /// A non-test file of a product crate (every `crates/<layer>/<crate>/` but the bench tool
    /// `hacivat-bench`; everything outside the crate's `tests/`, `#[path]`/`include!` targets
    /// counted; inline `#[cfg(test)]` modules under `src/` count, R-T1a-1) spells the git
    /// program at compile time: a string literal (plain, raw, byte, C; raw and with its escapes
    /// resolved), a `concat!` join (char pieces and nested macros included) or a `stringify!`
    /// holding a word whose last path component is `git` or `git.exe` (ASCII case-insensitive).
    /// In `hacivat-git` and `hacivat-scripts`, any API that starts a process is refused too
    /// (fix round 1, I1). See [`spawns_git`] and [`process_api`].
    NoGitSubprocess,
    /// A non-test file of `hacivat-scripts` (the one crate that may depend on `notify`) builds a
    /// file watcher that can follow directory links (R-T0-2, alg/28 C5), checked per call site
    /// since fix round 1 (I3). See [`watcher_follows_links`].
    WatcherFollowsLinks,
    /// A `src/` file of `hacivat-scripts` or `hacivat-git` shows a backend type in its public
    /// API (`notify`, `notify-debouncer-full`, `notify-types`, `file-id`, `inotify`, `kqueue`,
    /// `fsevent-sys`, `encoding_rs`, `trash`; `gix`, `gix-*`). See [`backend_in_public_api`].
    BackendInPublicApi,
    // ## m4b-t01a-seams
    // ## m4b-t07-git (named exception, fix round 3, p3)
    /// A file outside `hacivat-git`'s own `tests/` names `hacivat_git::testing`, the doc-hidden
    /// test hooks (dataless and stall injection), through any rename or glob. See
    /// [`names_git_testing_hooks`].
    GitTestingHookOutsideTests,
    // ## m4b-t07-git
    // ## m4b-t20-adversarial (named exception: Task 1a's file; carries T13 fr1 n1, T10)
    /// A file names `trash::DEFAULT_TRASH_CTX`, the `trash` crate's default context (on macOS
    /// it asks Finder through `osascript`; T1a review m3). `clippy.toml` bans the type and its
    /// constructors, but a constant is not a path clippy can ban, so this token rule refuses
    /// it in every crate, `fuzz/` and `xtask/` (T13 fr1 re-review n1).
    TrashDefaultContext,
    /// A file of the library service (`hacivat-scripts` `src/service.rs`, `src/service/**`,
    /// `src/index.rs`, `src/events.rs`) names the file system itself: a path into `std::fs`,
    /// `std::os::<os>::fs` or `tokio::fs` through any rename, a glob import from outside the
    /// crate, or a read, listing or removal call by name. Script content is read and written
    /// only through `fileio` (T5 mg2); this replaces T10's text tripwire. See
    /// [`names_the_file_system`].
    ScriptContentOutsideFileio,
    // ## m4b-t20-adversarial
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
/// ## m4b-t01a-seams (R-M4b-41, ADR 0009 D1): the script workspace's backends; a pattern
/// ending in `-*` matches every crate with that prefix (`gix-*`: every `gix-` crate as a
/// direct dependency, any edge kind).
const BACKENDS: [(&str, &str); 15] = [
    ("pg_query", "sql-oracle-pg"),
    ("sqlparser", "sql-safety"),
    ("notify", "hacivat-scripts"),
    ("notify-debouncer-full", "hacivat-scripts"),
    ("encoding_rs", "hacivat-scripts"),
    ("trash", "hacivat-scripts"),
    ("gix", "hacivat-git"),
    ("gix-*", "hacivat-git"),
    // ## m4b-t01a-fr1 (review M2): the watcher's lower crates.
    ("notify-types", "hacivat-scripts"),
    ("file-id", "hacivat-scripts"),
    ("inotify", "hacivat-scripts"),
    ("inotify-sys", "hacivat-scripts"),
    ("kqueue", "hacivat-scripts"),
    ("kqueue-sys", "hacivat-scripts"),
    ("fsevent-sys", "hacivat-scripts"),
];

/// Whether the package `name` is the backend `pattern` names (exact, or a `-*` prefix).
fn is_backend(pattern: &str, name: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => name.starts_with(prefix),
        None => name == pattern,
    }
}
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

// ## m4b-t01a-seams
/// The script workspace crate and the run service it never depends on (`ScriptsBelowExec`).
const SCRIPTS: &str = "hacivat-scripts";
const SCRIPTS_FORBIDDEN: [&str; 8] = [
    "hacivat-exec",
    "hacivat-meta",
    "hacivat-session",
    "hacivat-introspect",
    "russh",
    // ## m4b-t01a-fr1 (review M2): the network and secrets stack.
    "hacivat-net",
    "hacivat-secrets",
    "rustls",
];

/// What `hacivat-scripts`' normal and build closure may not contain (R-M4b-28, C-M4-6).
fn scripts_forbidden(name: &str) -> bool {
    SCRIPTS_FORBIDDEN.contains(&name) || is_driver(name)
}

/// [`normal_path_to`] over the edge `kinds` (every step), not only normal edges.
fn kinds_path_to(
    root: &PackageId,
    edges: &Edges<'_>,
    pkgs: &HashMap<&PackageId, &Package>,
    kinds: &[DependencyKind],
    bad: impl Fn(&Package) -> bool,
) -> Option<Vec<String>> {
    let mut seen: BTreeSet<&PackageId> = BTreeSet::from([root]);
    let mut queue: VecDeque<(&PackageId, Vec<String>)> = VecDeque::from([(root, vec![])]);
    while let Some((id, path)) = queue.pop_front() {
        for (dep_id, dep_kinds) in edges.get(id).into_iter().flatten() {
            if !dep_kinds.iter().any(|k| kinds.contains(k)) {
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
// ## m4b-t01a-seams

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
                // ## m4b-t01a-seams: the package's own name, through the `-*` prefix match.
                if let Some(d) = pkgs.get(dep_id).filter(|d| is_backend(backend, &d.name)) {
                    add(name, Rule::BackendOutsideOwner, vec![d.name.to_string()]);
                }
            }
            // ## m3b-t01a-seams (fix round 1, M4): a declared edge too, so an optional
            // dependency that no feature turns on in this resolve is still seen.
            for d in root
                .dependencies
                .iter()
                .filter(|d| is_backend(backend, &d.name))
            {
                add(name, Rule::BackendOutsideOwner, vec![d.name.clone()]);
            }
        }
        // ## m4b-t01a-seams (R-M4b-28, C-M4-6)
        if name == SCRIPTS {
            // Any declared normal edge, optional or not; then the resolved closure.
            // ## m4b-t01a-fr1 (review M2): build edges too (a build dependency would compile
            // the session stack into the fuzz build).
            for d in root.dependencies.iter().filter(|d| {
                matches!(d.kind, DependencyKind::Normal | DependencyKind::Build)
                    && scripts_forbidden(&d.name)
            }) {
                add(name, Rule::ScriptsBelowExec, vec![d.name.clone()]);
            }
            if let Some(via) = kinds_path_to(
                &root.id,
                &edges,
                &pkgs,
                &[DependencyKind::Normal, DependencyKind::Build],
                |p| scripts_forbidden(p.name.as_str()),
            ) {
                add(name, Rule::ScriptsBelowExec, via);
            }
        }
        if name == EXEC {
            for d in root.dependencies.iter().filter(|d| d.name == SCRIPTS) {
                add(name, Rule::ScriptsBelowExec, vec![d.name.clone()]);
            }
            for (dep_id, _) in edges.get(&root.id).into_iter().flatten() {
                if pkgs.get(dep_id).is_some_and(|d| d.name.as_str() == SCRIPTS) {
                    add(name, Rule::ScriptsBelowExec, vec![SCRIPTS.into()]);
                }
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
    // ## m4b-t01a-fr1
    /// A char literal, its escape resolved (`NoGitSubprocess`'s `concat!` joins).
    Char(char),
}

/// `src` as tokens, with comments (line, nested block, doc), whitespace, lifetimes and
/// numbers dropped (char literals are `Char` tokens since `m4b-t01a-fr1`), so `MetaQuery :: new`, a path split over lines or a comment
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
            // ## m4b-t01a-fr1 (review I2): char literals are kept (cooked) as `Char` tokens,
            // so `concat!("gi", 't')` joins to its compile-time text; the escaped character
            // itself is skipped, so `'\''` ends at its own closing quote.
            if at(i + 1) == '\\' {
                let mut j = i + 3;
                while j < c.len() && c[j] != '\'' {
                    j += 1;
                }
                let raw: String = c[i + 1..j.min(c.len())].iter().collect();
                if let Some(ch) = unescape(&raw).chars().next() {
                    out.push(SrcTok::Char(ch));
                }
                i = j + 1;
            } else if at(i + 2) == '\'' {
                out.push(SrcTok::Char(at(i + 1)));
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

// ## m4b-t01a-seams
/// Crates under `crates/` that are tools, not product code (`NoGitSubprocess`): the bench
/// crate, reachable only through the adapter's `bench` feature (`AdapterFeatureInProduct`).
const NO_GIT_EXEMPT: [&str; 1] = ["hacivat-bench"];
/// The crates that may build no subprocess at all, whatever the program.
const NO_PROCESS_CRATES: [&str; 2] = ["hacivat-git", "hacivat-scripts"];
/// The backend crates (as Rust paths) each owner keeps out of its public API; `gix_*` is a
/// prefix (`BackendInPublicApi`).
const PUBLIC_API_BACKENDS: [(&str, &[&str]); 2] = [
    (
        "hacivat-scripts",
        &[
            "notify",
            "notify_debouncer_full",
            "notify_types",
            "file_id",
            "encoding_rs",
            "trash",
            // ## m4b-t01a-fr1 (review M2)
            "inotify",
            "inotify_sys",
            "kqueue",
            "kqueue_sys",
            "fsevent_sys",
        ],
    ),
    ("hacivat-git", &["gix", "gix_*"]),
];

// ## m4b-t01a-fr1 (review I1): what starts a process, for the two crates that never do.
/// Identifiers refused anywhere in a no-process crate's non-test file: the `process` module
/// (`std::process`, `tokio::process`, `std::os::unix::process`, a renamed `process as sp`),
/// `CommandExt`, `gix`'s command crate, the `gix-path` `env` functions that build or run a
/// program, and the `exec*`/`posix_spawn*`/`popen`/`vfork` libc calls.
const PROCESS_IDENTS: [&str; 24] = [
    "process",
    "CommandExt",
    "gix_command",
    "shell_command",
    "exe_invocation",
    "core_dir_program",
    "installation_program",
    "installation_config",
    "installation_config_prefix",
    "system_prefix",
    "execv",
    "execve",
    "execvp",
    "execvpe",
    "execvP",
    "execl",
    "execlp",
    "execle",
    "fexecve",
    "posix_spawn",
    "posix_spawnp",
    "popen",
    "vfork",
    "pidfd_spawn",
];
/// Module paths refused in a no-process crate, after `use … as` and `extern crate … as`
/// renames are resolved: names too common to refuse as bare identifiers (`fork`, `system`,
/// `env`, `command`) are refused through their path.
const PROCESS_PATHS: [&[&str]; 9] = [
    &["gix", "command"],
    &["gix", "path", "env"],
    &["gix_path", "env"],
    &["libc", "fork"],
    &["libc", "system"],
    &["libc", "syscall"],
    &["libc", "daemon"],
    &["std", "process"],
    &["tokio", "process"],
];

/// `raw` with Rust's string escapes resolved: `\n \r \t \\ \0 \' \"`, `\xHH`, `\u{…}`, and a
/// `\`-newline continuation (which drops the next line's leading whitespace, as rustc does).
fn unescape(raw: &str) -> String {
    let mut out = String::new();
    let mut it = raw.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('0') => out.push('\0'),
            Some('x') => {
                let hex: String = it.by_ref().take(2).collect();
                if let Ok(b) = u8::from_str_radix(&hex, 16) {
                    out.push(char::from(b));
                }
            }
            Some('u') => {
                if it.peek() == Some(&'{') {
                    it.next();
                    let hex: String = it.by_ref().take_while(|c| *c != '}').collect();
                    if let Some(ch) = u32::from_str_radix(&hex.replace('_', ""), 16)
                        .ok()
                        .and_then(char::from_u32)
                    {
                        out.push(ch);
                    }
                }
            }
            Some('\n' | '\r') => {
                while it.peek().is_some_and(|c| c.is_whitespace()) {
                    it.next();
                }
            }
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}
// ## m4b-t01a-fr1

/// Whether a string literal names the git program: some word of it (split at whitespace,
/// quotes and shell punctuation, so `sh -c "cd x && git status"` counts) has `git` or
/// `git.exe` as its last `/` or `\` path component, ASCII case-insensitive.
fn names_git_program(lit: &str) -> bool {
    lit.split(|c: char| c.is_whitespace() || ";&|()<>`'\"$".contains(c))
        .filter_map(|w| w.rsplit(['/', '\\']).next())
        .any(|leaf| leaf.eq_ignore_ascii_case("git") || leaf.eq_ignore_ascii_case("git.exe"))
}

/// The index just past the bracket group that opens at `open` (`(`, `[` or `{`), or
/// `t.len()` when it never closes.
fn group_end(t: &[SrcTok], open: usize) -> usize {
    let (o, c) = match t.get(open) {
        Some(SrcTok::Punct('(')) => ('(', ')'),
        Some(SrcTok::Punct('[')) => ('[', ']'),
        Some(SrcTok::Punct('{')) => ('{', '}'),
        _ => return open,
    };
    let mut depth = 0usize;
    for (k, tok) in t.iter().enumerate().skip(open) {
        if *tok == SrcTok::Punct(o) {
            depth += 1;
        } else if *tok == SrcTok::Punct(c) {
            depth -= 1;
            if depth == 0 {
                return k + 1;
            }
        }
    }
    t.len()
}

// ## m4b-t01a-fr1 (review I2)
/// The tokens inside the bracket group that opens at `open` (empty when there is none).
fn group_inner(t: &[SrcTok], open: usize) -> &[SrcTok] {
    let end = group_end(t, open);
    if end <= open + 1 {
        return &[];
    }
    let close = if end <= t.len() && end > open + 1 && end - 1 > open {
        end - 1
    } else {
        t.len()
    };
    &t[open + 1..close.max(open + 1)]
}

/// Tokens as `stringify!` would print them (joined by spaces; close enough for the word
/// split of [`names_git_program`]).
fn render(t: &[SrcTok]) -> String {
    t.iter()
        .map(|x| match x {
            SrcTok::Ident(s) => s.clone(),
            SrcTok::Punct(c) | SrcTok::Char(c) => c.to_string(),
            SrcTok::Str(s) => format!("\"{}\"", unescape(s)),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The compile-time text of a `concat!` group: string pieces cooked, chars, `true`/`false`,
/// and nested `concat!`/`stringify!` text.
fn concat_text(inner: &[SrcTok]) -> String {
    let mut out = String::new();
    let mut k = 0;
    while k < inner.len() {
        match &inner[k] {
            SrcTok::Str(s) => out.push_str(&unescape(s)),
            SrcTok::Char(c) => out.push(*c),
            SrcTok::Ident(name) if tok_punct(inner, k + 1, '!') => {
                let nested = group_inner(inner, k + 2);
                match name.as_str() {
                    "concat" => out.push_str(&concat_text(nested)),
                    "stringify" => out.push_str(&render(nested).replace(' ', "")),
                    _ => {}
                }
                k = group_end(inner, k + 2).max(k + 1);
                continue;
            }
            SrcTok::Ident(s) => out.push_str(s),
            _ => {}
        }
        k += 1;
    }
    out
}

/// Every compile-time text the tokens spell: each string literal raw and cooked, each
/// `concat!(…)` joined, each `stringify!(…)` printed.
fn literal_texts(t: &[SrcTok]) -> Vec<String> {
    let mut out = Vec::new();
    for x in t {
        if let SrcTok::Str(s) = x {
            out.push(s.clone());
            out.push(unescape(s));
        }
    }
    for i in 0..t.len() {
        if !tok_punct(t, i + 1, '!') {
            continue;
        }
        if tok_ident(t, i, "concat") {
            out.push(concat_text(group_inner(t, i + 2)));
        } else if tok_ident(t, i, "stringify") {
            let inner = group_inner(t, i + 2);
            out.push(render(inner));
            out.push(render(inner).replace(' ', ""));
        }
    }
    out
}

/// One `use` tree flattened: every `(full path, local name)` it binds, and its globs.
fn flatten_use(
    t: &[SrcTok],
    mut k: usize,
    end: usize,
    prefix: &[String],
    out: &mut Vec<Import>,
    globs: &mut Vec<Vec<String>>,
) -> usize {
    let mut path = prefix.to_vec();
    while k < end && tok_punct(t, k, ':') {
        k += 1;
    }
    while k < end {
        match &t[k] {
            SrcTok::Ident(s) => {
                path.push(s.clone());
                k += 1;
                if tok_punct(t, k, ':') && tok_punct(t, k + 1, ':') {
                    k += 2;
                    continue;
                }
                if path.last().is_some_and(|l| l == "self") {
                    path.pop();
                }
                let name = if tok_ident(t, k, "as") {
                    k += 1;
                    match t.get(k) {
                        Some(SrcTok::Ident(a)) => {
                            k += 1;
                            a.clone()
                        }
                        _ => "_".into(),
                    }
                } else {
                    path.last().cloned().unwrap_or_default()
                };
                out.push((path, name));
                return k;
            }
            SrcTok::Punct('{') => {
                k += 1;
                while k < end && !tok_punct(t, k, '}') {
                    let before = k;
                    k = flatten_use(t, k, end, &path, out, globs);
                    if tok_punct(t, k, ',') {
                        k += 1;
                    }
                    if k == before {
                        k += 1;
                    }
                }
                return k + 1;
            }
            SrcTok::Punct('*') => {
                globs.push(path);
                return k + 1;
            }
            _ => return k + 1,
        }
    }
    k
}

/// An imported path and the local name it binds.
type Import = (Vec<String>, String);

/// The paths the tokens import (`use`, `extern crate`), their local names, and the globs.
fn imports(t: &[SrcTok]) -> (Vec<Import>, Vec<Vec<String>>) {
    let mut out = Vec::new();
    let mut globs = Vec::new();
    for i in 0..t.len() {
        if tok_ident(t, i, "use") {
            let end = item_end(t, i).saturating_sub(1).max(i + 1);
            flatten_use(t, i + 1, end, &[], &mut out, &mut globs);
        } else if tok_ident(t, i, "extern")
            && tok_ident(t, i + 1, "crate")
            && let Some(SrcTok::Ident(name)) = t.get(i + 2)
        {
            let local = match (tok_ident(t, i + 3, "as"), t.get(i + 4)) {
                (true, Some(SrcTok::Ident(a))) => a.clone(),
                _ => name.clone(),
            };
            out.push((vec![name.clone()], local));
        }
    }
    (out, globs)
}

/// `path` with its first segment replaced through the import renames, up to four times.
fn resolve_path(path: &[String], renames: &HashMap<String, Vec<String>>) -> Vec<String> {
    let mut p = path.to_vec();
    for _ in 0..4 {
        match p.first().and_then(|f| renames.get(f)) {
            Some(full) if full.len() > 1 || full.first() != p.first() => {
                let mut next = full.clone();
                next.extend(p.iter().skip(1).cloned());
                p = next;
            }
            _ => break,
        }
    }
    p
}

/// Every path expression of the tokens (`a::b::c`, turbofish skipped), as segments.
fn expr_paths(t: &[SrcTok]) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    for i in 0..t.len() {
        let SrcTok::Ident(first) = &t[i] else {
            continue;
        };
        let continues = i >= 2
            && tok_punct(t, i - 1, ':')
            && tok_punct(t, i - 2, ':')
            && (i >= 3 && (matches!(t[i - 3], SrcTok::Ident(_)) || tok_punct(t, i - 3, '>')));
        if continues {
            continue;
        }
        let mut segs = vec![first.clone()];
        let mut k = i + 1;
        while tok_punct(t, k, ':') && tok_punct(t, k + 1, ':') {
            match t.get(k + 2) {
                Some(SrcTok::Ident(s)) => {
                    segs.push(s.clone());
                    k += 3;
                }
                Some(SrcTok::Punct('<')) => k = skip_angle(t, k + 2),
                _ => break,
            }
        }
        out.push(segs);
    }
    out
}

// ## m4b-t07-git (named exception, fix round 3, p3)
/// The crate whose `tests/` alone may name its `testing` hooks.
const GIT_HOOKS_HOME: &str = "hacivat-git";

/// Whether the tokens name `hacivat_git::testing` (`GitTestingHookOutsideTests`): in a path
/// expression or an import, through `use … as` and `extern crate … as` renames, or hidden
/// behind a glob of the crate (`use hacivat_git::*`) or of the module.
fn names_git_testing_hooks(t: &[SrcTok]) -> bool {
    let (imported, globs) = imports(t);
    let renames: HashMap<String, Vec<String>> = imported
        .iter()
        .map(|(path, name)| (name.clone(), path.clone()))
        .collect();
    // ## m4b-t20-adversarial fix round 1 (named line, review m1): `testing` anywhere after the
    // crate (an alias of the crate root inside it: `hacivat_git::hv::testing`).
    let hooks = |p: &[String]| {
        p.len() >= 2 && p[0] == "hacivat_git" && p[1..].iter().any(|s| s == "testing")
    };
    let crate_glob = |p: &[String]| p.len() == 1 && p[0] == "hacivat_git";
    globs.iter().any(|g| {
        let r = resolve_path(g, &renames);
        hooks(&r) || crate_glob(&r)
    }) || imported
        .iter()
        .any(|(p, _)| hooks(&resolve_path(p, &renames)))
        || expr_paths(t)
            .iter()
            .any(|p| hooks(&resolve_path(p, &renames)))
}
// ## m4b-t07-git

// ## m4b-t20-adversarial (named exception: Task 1a's file; carry T7 q1)
/// Whether a `use` at `i` is public: `pub use` or `pub(…) use`.
fn is_pub_use(t: &[SrcTok], i: usize) -> bool {
    if i == 0 {
        return false;
    }
    if tok_ident(t, i - 1, "pub") {
        return true;
    }
    if !tok_punct(t, i - 1, ')') {
        return false;
    }
    let mut depth = 0usize;
    let mut k = i - 1;
    loop {
        if tok_punct(t, k, ')') {
            depth += 1;
        } else if tok_punct(t, k, '(') {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return k > 0 && tok_ident(t, k - 1, "pub");
            }
        }
        if k == 0 {
            return false;
        }
        k -= 1;
    }
}

/// Whether a `src/` file of `hacivat-git` re-exports its `testing` hooks
/// (`GitTestingHookOutsideTests`, T7 review q1): a public `use` (any `pub(…)` too) whose path,
/// through this file's import renames, goes through a `testing` segment, or a public glob of a
/// crate-local root (`pub use super::*`), which would re-export `testing` under another name.
/// Outside code could then call a hook without naming `testing`.
///
/// Fix round 1 (review m1): also a public alias of a crate-local root (`pub use crate as hv`,
/// `pub use super::super as root`, `pub extern crate self as hv`), and a public `type`,
/// `const` or `static` whose tokens name a path through `testing`.
///
/// Known limits (recorded in ADR 0009 `m4b-t20`): a path a macro assembles from fragments
/// (`macro_rules! m { ($m:ident) => { hacivat_git::$m::… } }`) is not seen, like the run-time
/// program name of `NoGitSubprocess`; nor is a public wrapper function that calls a hook.
fn reexports_git_testing(t: &[SrcTok]) -> bool {
    let (imported, _) = imports(t);
    let renames: HashMap<String, Vec<String>> = imported
        .iter()
        .map(|(path, name)| (name.clone(), path.clone()))
        .collect();
    let through = |p: &[String]| p.iter().any(|s| s == "testing");
    let local_root = |p: &[String]| {
        p.iter()
            .all(|s| ["crate", "self", "super"].contains(&s.as_str()))
    };
    (0..t.len()).any(|i| {
        // ## m4b-t20-adversarial fix round 1 (named lines, review m1): `pub extern crate self
        // as hv`, and public `type`/`const`/`static` items whose value or type goes through
        // `testing` (an alias outside code can name).
        if tok_ident(t, i, "extern")
            && is_pub_use(t, i)
            && tok_ident(t, i + 1, "crate")
            && tok_ident(t, i + 2, "self")
        {
            return true;
        }
        if ["type", "const", "static"]
            .iter()
            .any(|k| tok_ident(t, i, k))
            && is_pub_use(t, i)
        {
            let end = item_end(t, i).min(t.len());
            return expr_paths(&t[i..end])
                .iter()
                .any(|p| through(&resolve_path(p, &renames)));
        }
        if !tok_ident(t, i, "use") || !is_pub_use(t, i) {
            return false;
        }
        let end = item_end(t, i).saturating_sub(1).max(i + 1);
        let mut out = Vec::new();
        let mut globs = Vec::new();
        flatten_use(t, i + 1, end, &[], &mut out, &mut globs);
        // Fix round 1 (m1): a public alias of a crate-local root (`pub use crate as hv`,
        // `pub use super::super as root`) re-exports `testing` under another path.
        out.iter().any(|(p, _)| {
            let r = resolve_path(p, &renames);
            through(&r) || local_root(&r)
        }) || globs.iter().any(|g| {
            let r = resolve_path(g, &renames);
            through(&r) || local_root(&r)
        })
    })
}

/// The `trash` crate's default context (`TrashDefaultContext`).
const TRASH_DEFAULT_CTX: &str = "DEFAULT_TRASH_CTX";

/// Whether the tokens name `DEFAULT_TRASH_CTX` (raw identifiers included; a comment or a
/// string never matches). A constant cannot be reached without spelling its name once.
fn names_default_trash_ctx(t: &[SrcTok]) -> bool {
    t.iter()
        .any(|x| matches!(x, SrcTok::Ident(s) if s == TRASH_DEFAULT_CTX))
}

/// The library service's files, relative to `hacivat-scripts` (`ScriptContentOutsideFileio`).
const FILEIO_ONLY_FILES: [&str; 3] = ["src/service.rs", "src/index.rs", "src/events.rs"];
/// And every file under this folder.
const FILEIO_ONLY_DIR: &str = "src/service/";
/// File-system paths (`std::os::<os>::fs` is matched apart). ## m4b-t20-adversarial fix round 1
/// (review m2): and `hacivat_settings::fsio`, a second, path-based writer (`registry` and
/// `recovery` hold its uses; script content never goes through it).
const FS_PATHS: [&[&str]; 3] = [
    &["std", "fs"],
    &["tokio", "fs"],
    &["hacivat_settings", "fsio"],
];
/// Calls that open, read, list or remove by name, wherever they come from.
const FS_IDENTS: [&str; 8] = [
    "OpenOptions",
    "read_to_end",
    "read_to_string",
    "read_dir",
    "remove_file",
    "remove_dir",
    "remove_dir_all",
    "read_link",
];

/// Whether the tokens name the file system (`ScriptContentOutsideFileio`): an identifier of
/// [`FS_IDENTS`], a path of [`FS_PATHS`] or `std::os::<os>::fs` through any rename (`use
/// std::{fs as disk}`, `use std as s; s::fs`, `extern crate std as s`), or a glob import from
/// outside the crate (`use std::*`), which would hide the rest.
fn names_the_file_system(t: &[SrcTok]) -> bool {
    if t.iter()
        .any(|x| matches!(x, SrcTok::Ident(s) if FS_IDENTS.contains(&s.as_str())))
    {
        return true;
    }
    // ## m4b-t20-adversarial fix round 1 (named lines, review m2): no file of the service
    // splices in another (`include!`, `#[path]`): the rule's file set is the whole service.
    let splices = (0..t.len()).any(|i| {
        (tok_ident(t, i, "include") && tok_punct(t, i + 1, '!'))
            || (tok_punct(t, i, '#') && tok_punct(t, i + 1, '[') && tok_ident(t, i + 2, "path"))
            // Fix round 2 (re-review nit): `#[cfg_attr(…, path = "…")]`.
            || (tok_punct(t, i, '#')
                && tok_punct(t, i + 1, '[')
                && tok_ident(t, i + 2, "cfg_attr")
                && {
                    let end = (i + 3..t.len())
                        .find(|k| tok_punct(t, *k, ']'))
                        .unwrap_or(t.len());
                    (i + 3..end).any(|k| tok_ident(t, k, "path") && tok_punct(t, k + 1, '='))
                })
    });
    if splices {
        return true;
    }
    let (imported, globs) = imports(t);
    let renames: HashMap<String, Vec<String>> = imported
        .iter()
        .map(|(path, name)| (name.clone(), path.clone()))
        .collect();
    let banned = |p: &[String]| {
        FS_PATHS
            .iter()
            .any(|b| p.len() >= b.len() && p.iter().zip(b.iter()).all(|(a, b)| a == b))
            || (p.len() >= 4 && p[0] == "std" && p[1] == "os" && p[3] == "fs")
    };
    let local_root = |p: &[String]| {
        p.first()
            .is_some_and(|f| ["crate", "self", "super"].contains(&f.as_str()))
    };
    globs
        .iter()
        .any(|g| !local_root(&resolve_path(g, &renames)))
        || imported
            .iter()
            .any(|(p, _)| banned(&resolve_path(p, &renames)))
        || expr_paths(t)
            .iter()
            .any(|p| banned(&resolve_path(p, &renames)))
}
// ## m4b-t20-adversarial

/// Whether the tokens reach an API that starts a process (`NoGitSubprocess` in the
/// no-process crates): an identifier of [`PROCESS_IDENTS`], a path of [`PROCESS_PATHS`]
/// through any rename (`use gix as g; g::command`), or a glob import from outside the crate
/// (`use gix::*`, `use libc::*`), which would hide the rest.
fn process_api(t: &[SrcTok]) -> bool {
    if t.iter()
        .any(|x| matches!(x, SrcTok::Ident(s) if PROCESS_IDENTS.contains(&s.as_str())))
    {
        return true;
    }
    let (imported, globs) = imports(t);
    let renames: HashMap<String, Vec<String>> = imported
        .iter()
        .map(|(path, name)| (name.clone(), path.clone()))
        .collect();
    let banned = |p: &[String]| {
        PROCESS_PATHS
            .iter()
            .any(|b| p.len() >= b.len() && p.iter().zip(b.iter()).all(|(a, b)| a == b))
    };
    let local_root = |p: &[String]| {
        p.first()
            .is_some_and(|f| ["crate", "self", "super"].contains(&f.as_str()))
    };
    globs
        .iter()
        .any(|g| !local_root(&resolve_path(g, &renames)))
        || imported
            .iter()
            .any(|(p, _)| banned(&resolve_path(p, &renames)))
        || expr_paths(t)
            .iter()
            .any(|p| banned(&resolve_path(p, &renames)))
}
// ## m4b-t01a-fr1

/// Whether the tokens spell the git program (`NoGitSubprocess`): a compile-time text of
/// theirs ([`literal_texts`]: string literals raw and with escapes resolved, `concat!` joins
/// with char pieces, `stringify!` text) that [`names_git_program`]; with `no_process`, any
/// API that starts a process ([`process_api`]).
///
/// Limits: outside the no-process crates, a program name assembled at run time
/// (`["gi", "t"].concat()`, `format!`, `env::var`) is not seen; the price of the literal rule is
/// that a product string may not use the bare word (say "repository"; R-T1a-1: a later need
/// is met by an exact-literal allow-list reviewed with that task, never a pattern).
fn spawns_git(t: &[SrcTok], no_process: bool) -> bool {
    literal_texts(t).iter().any(|s| names_git_program(s)) || (no_process && process_api(t))
}

// ## m4b-t01a-fr1 (review I3): watcher construction, checked per call site.
/// The notify watcher types (and the trait) whose constructors take a `Config`.
const WATCHER_TYPES: [&str; 8] = [
    "RecommendedWatcher",
    "PollWatcher",
    "INotifyWatcher",
    "FsEventWatcher",
    "KqueueWatcher",
    "ReadDirectoryChangesWatcher",
    "NullWatcher",
    "Watcher",
];
/// Refused outright: they cannot be given a non-following config or cache.
const WATCHER_BANNED: [&str; 4] = [
    "new_debouncer",
    "FileIdMap",
    "RecommendedCache",
    "recommended_watcher",
];
/// The one shared config builder a constructor may take instead of an inline
/// `with_follow_symlinks(false)`.
const WATCHER_CONFIG_FN: &str = "watcher_config";

/// Whether the tokens hold `with_follow_symlinks ( false )`.
fn says_no_follow(t: &[SrcTok]) -> bool {
    (0..t.len()).any(|i| {
        tok_ident(t, i, "with_follow_symlinks")
            && tok_punct(t, i + 1, '(')
            && tok_ident(t, i + 2, "false")
            && tok_punct(t, i + 3, ')')
    })
}

/// Whether a constructor's arguments carry a non-following config: the shared
/// `watcher_config()` or an inline `with_follow_symlinks(false)`.
fn config_arg_ok(args: &[SrcTok]) -> bool {
    let shared = (0..args.len()).any(|i| {
        tok_ident(args, i, WATCHER_CONFIG_FN)
            && tok_punct(args, i + 1, '(')
            && tok_punct(args, i + 2, ')')
            // ## m4b-t09-watcher fix round 1 (m1, named exception): only the bare call, never
            // a path (`alt::watcher_config()`) or a method of the same name.
            && !(i > 0 && (tok_punct(args, i - 1, ':') || tok_punct(args, i - 1, '.')))
    });
    shared || says_no_follow(args)
}

/// The generic arguments of the `<…>` that opens at `open`, split at depth-0 commas.
fn generic_args(t: &[SrcTok], open: usize) -> Vec<&[SrcTok]> {
    let end = skip_angle(t, open);
    let inner = &t[(open + 1).min(end)..end.saturating_sub(1).max(open + 1)];
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    for (k, x) in inner.iter().enumerate() {
        match x {
            SrcTok::Punct('<' | '(' | '[') => depth += 1,
            SrcTok::Punct('>' | ')' | ']') => depth -= 1,
            SrcTok::Punct(',') if depth == 0 => {
                parts.push(&inner[start..k]);
                start = k + 1;
            }
            _ => {}
        }
    }
    parts.push(&inner[start..]);
    parts
}

/// Whether the tokens (a non-test file of the `notify` owner) build a watcher that can follow
/// directory links (`WatcherFollowsLinks`, R-T0-2), at any one call site:
///
/// - `new_debouncer`, `FileIdMap`, `RecommendedCache` or `recommended_watcher` at all;
/// - `with_follow_symlinks(…)` with anything but `false`;
/// - a watcher type or `new_debouncer_opt` renamed (`… as W`) or aliased (`type W = …`);
/// - `new_debouncer_opt` without a turbofish whose cache is `NoCache`, or without a
///   non-following config argument ([`config_arg_ok`]);
/// - `W::new(…)`/`W::with_initial_scan(…)` of a watcher type, directly or qualified
///   (`<X as Watcher>::new`), without a non-following config argument;
/// - `Config::default()` whose method chain does not set `with_follow_symlinks(false)`;
/// - a `fn watcher_config` whose body does not set `with_follow_symlinks(false)`.
///
/// Limits: a watcher built in a generic function (`W::new(h, Default::default())` with
/// `W: Watcher`) is not seen here; `clippy.toml` refuses `notify::Watcher::new` itself.
fn watcher_follows_links(t: &[SrcTok]) -> bool {
    let banned = t
        .iter()
        .any(|x| matches!(x, SrcTok::Ident(s) if WATCHER_BANNED.contains(&s.as_str())));
    let follows = (0..t.len()).any(|i| {
        tok_ident(t, i, "with_follow_symlinks")
            && !(tok_punct(t, i + 1, '(')
                && tok_ident(t, i + 2, "false")
                && tok_punct(t, i + 3, ')'))
    });
    let is_watcher = |k: usize| WATCHER_TYPES.iter().any(|w| tok_ident(t, k, w));
    let renamed = (0..t.len()).any(|i| {
        (is_watcher(i) || tok_ident(t, i, "new_debouncer_opt")) && tok_ident(t, i + 1, "as")
    }) || (0..t.len())
        .any(|i| tok_ident(t, i, "type") && (i + 1..item_end(t, i)).any(&is_watcher));
    // Imports name the function without calling it; every other mention is a call (or a
    // function pointer) and is checked.
    let mut in_use = vec![false; t.len()];
    for i in 0..t.len() {
        if tok_ident(t, i, "use") {
            let end = item_end(t, i).min(t.len());
            in_use[i..end].iter_mut().for_each(|x| *x = true);
        }
    }
    let debouncer_call = (0..t.len()).any(|i| {
        if !tok_ident(t, i, "new_debouncer_opt") || in_use[i] {
            return false;
        }
        let turbofish =
            tok_punct(t, i + 1, ':') && tok_punct(t, i + 2, ':') && tok_punct(t, i + 3, '<');
        if !turbofish {
            return true;
        }
        let args = generic_args(t, i + 3);
        let cache_ok = args
            .get(2)
            .and_then(|a| a.last())
            .is_some_and(|x| *x == SrcTok::Ident("NoCache".into()));
        let open = skip_angle(t, i + 3);
        !(cache_ok && tok_punct(t, open, '(') && config_arg_ok(group_inner(t, open)))
    });
    let ctor = |k: usize| {
        (tok_ident(t, k, "new") || tok_ident(t, k, "with_initial_scan")) && tok_punct(t, k + 1, '(')
    };
    let watcher_ctor_call = (0..t.len()).any(|i| {
        if !(tok_punct(t, i, ':') && tok_punct(t, i + 1, ':') && ctor(i + 2)) || i == 0 {
            return false;
        }
        let on_watcher = if tok_punct(t, i - 1, '>') {
            // `<X as Watcher>::new`: the qualified `<…>` that closes at `i - 1`.
            let mut depth = 0i32;
            let mut k = i - 1;
            let mut found = false;
            loop {
                if tok_punct(t, k, '>') {
                    depth += 1;
                } else if tok_punct(t, k, '<') {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                found |= is_watcher(k);
                if k == 0 {
                    break;
                }
                k -= 1;
            }
            found
        } else {
            is_watcher(i - 1)
        };
        on_watcher && !config_arg_ok(group_inner(t, i + 3))
    });
    let default_config = (0..t.len()).any(|i| {
        if !(tok_ident(t, i, "Config")
            && tok_punct(t, i + 1, ':')
            && tok_punct(t, i + 2, ':')
            && tok_ident(t, i + 3, "default")
            && tok_punct(t, i + 4, '('))
        {
            return false;
        }
        // The method chain after `Config::default()`: `.name(…)` repeated.
        let mut k = group_end(t, i + 4);
        let chain_start = k;
        while tok_punct(t, k, '.')
            && matches!(t.get(k + 1), Some(SrcTok::Ident(_)))
            && tok_punct(t, k + 2, '(')
        {
            k = group_end(t, k + 2);
        }
        !says_no_follow(&t[chain_start..k.min(t.len())])
    });
    let shared_fn = (0..t.len()).any(|i| {
        tok_ident(t, i, "fn")
            && tok_ident(t, i + 1, WATCHER_CONFIG_FN)
            && (i + 2..t.len())
                .find(|k| tok_punct(t, *k, '{'))
                .is_none_or(|open| !says_no_follow(group_inner(t, open)))
    });
    banned
        || follows
        || renamed
        || debouncer_call
        || watcher_ctor_call
        || default_config
        || shared_fn
}
// ## m4b-t01a-fr1

// ## m4b-t09-watcher (named exception in Task 1a's file; carries-w1 T9 "hardening m1")
/// The one file where `fn watcher_config` may be defined.
const WATCHER_CONFIG_HOME: &str = "crates/core/hacivat-scripts/src/watch/mod.rs";

/// Whether the tokens define a stand-in for a name [`watcher_follows_links`] trusts, so that
/// the per-call-site check would pass a following cache or config:
///
/// - `FileIdCache` named at all (a local cache, which could walk through links, needs it, by
///   any import or rename; the crate's own `NoCache` never does);
/// - any item, binding or rename named `NoCache` (`struct NoCache`, `type NoCache = …`,
///   `use … as NoCache`, `let NoCache`), so the turbofish's `NoCache` is always the crate's;
/// - `watcher_config` bound or renamed anywhere (`let`, `const`, `static`, `mut`, `as`, a
///   parameter or a field), or defined as a `fn` outside [`WATCHER_CONFIG_HOME`].
///
/// `home` says whether the tokens are that file.
fn watcher_decoys(t: &[SrcTok], home: bool) -> bool {
    let binders = [
        "struct", "enum", "union", "type", "trait", "fn", "mod", "const", "static", "let", "mut",
        "as",
    ];
    let bound = |name: &str, allow_fn: bool| {
        (0..t.len()).any(|i| {
            if !tok_ident(t, i, name) || i == 0 {
                return false;
            }
            let by_fn = tok_ident(t, i - 1, "fn");
            if by_fn {
                return !allow_fn;
            }
            binders.iter().any(|b| tok_ident(t, i - 1, b))
                // A parameter, field or pattern binding: `name:` (but not a path `name::`).
                || (tok_punct(t, i + 1, ':') && !tok_punct(t, i + 2, ':'))
        })
    };
    let local_cache = t
        .iter()
        .any(|x| matches!(x, SrcTok::Ident(s) if s == "FileIdCache"));
    // Fix round 1 (m1): exactly one definition in the home file, at brace depth 0 (a nested
    // `mod alt { fn watcher_config() … }` is refused).
    let mut depth = 0usize;
    let mut defs = 0usize;
    let mut nested = false;
    for i in 0..t.len() {
        if tok_punct(t, i, '{') {
            depth += 1;
        } else if tok_punct(t, i, '}') {
            depth = depth.saturating_sub(1);
        } else if tok_ident(t, i, "fn") && tok_ident(t, i + 1, WATCHER_CONFIG_FN) {
            defs += 1;
            nested |= depth > 0;
        }
    }
    let home_extra = home && (defs > 1 || nested);
    local_cache || bound("NoCache", false) || bound(WATCHER_CONFIG_FN, home) || home_extra
}
// ## m4b-t09-watcher

/// Whether the backend path segment `seg` is one of `backends` (`gix_*` is a prefix).
fn is_backend_segment(backends: &[&str], seg: &str) -> bool {
    backends.iter().any(|b| match b.strip_suffix('*') {
        Some(prefix) => seg.starts_with(prefix),
        None => seg == *b,
    })
}

/// The end of the item header that starts at `from` (a `pub` or an `impl`): the first `;`, `,`
/// or `{` outside `(…)`, `[…]` and `<…>`, or a closing bracket that leaves the header (a last
/// field before `}`). With `body`, a `{` takes the whole braced body in (an enum's variants, a
/// trait's methods).
fn header_end(t: &[SrcTok], from: usize, body: bool) -> usize {
    let mut depth = 0i32;
    let mut k = from;
    while k < t.len() {
        match &t[k] {
            SrcTok::Punct('(' | '[') => depth += 1,
            SrcTok::Punct('<') => depth += 1,
            SrcTok::Punct('>')
                if !(k > 0 && (tok_punct(t, k - 1, '-') || tok_punct(t, k - 1, '='))) =>
            {
                depth -= 1
            }
            SrcTok::Punct(')' | ']' | '}') if depth <= 0 => return k,
            SrcTok::Punct(')' | ']') => depth -= 1,
            SrcTok::Punct(';' | ',') if depth <= 0 => return k,
            SrcTok::Punct('{') if depth <= 0 => {
                return if body { group_end(t, k) } else { k };
            }
            _ => {}
        }
        k += 1;
    }
    t.len()
}

/// Whether a `src/` file of a backend owner shows a backend type publicly
/// (`BackendInPublicApi`):
///
/// - a `pub use` of a backend path or of a name imported from one (`use notify as n;
///   pub use n::Config;`), or a glob import of either (`use notify::*`);
/// - a `pub` item (not `pub(crate)`, `pub(super)`, `pub(in …)`) whose header names a backend
///   path segment (`notify::…`, `gix_status::…`), a name imported from a backend, or a type
///   alias of one (`type Inner = notify::Config;`, transitively); an enum's or a trait's whole
///   body counts;
/// - an `impl` item header that does the same, except a backend trait implemented for a type
///   this file declares private (`impl DebounceEventHandler for Sink` with `struct Sink;`),
///   which leaks nothing.
///
/// `pub` items of private modules count too (fail closed: use `pub(crate)` for internals). A
/// method called like a backend (`pub fn trash`) is not a path segment and passes.
fn backend_in_public_api(t: &[SrcTok], backends: &[&str]) -> bool {
    let seg_at = |k: usize| {
        matches!(t.get(k), Some(SrcTok::Ident(s)) if is_backend_segment(backends, s))
            && tok_punct(t, k + 1, ':')
            && tok_punct(t, k + 2, ':')
    };
    let (paths, globs) = imports(t);
    // ## m4b-t01a-fr1 (review M1): names bound to a backend, through renames and type
    // aliases, to a fixed point.
    let mut imported: BTreeSet<String> = BTreeSet::new();
    loop {
        let before = imported.len();
        for (p, name) in &paths {
            if p.first()
                .is_some_and(|f| is_backend_segment(backends, f) || imported.contains(f))
            {
                imported.insert(name.clone());
            }
        }
        for i in 0..t.len() {
            if tok_ident(t, i, "type")
                && let Some(SrcTok::Ident(name)) = t.get(i + 1)
            {
                let rhs = i + 2..item_end(t, i);
                if rhs.clone().any(seg_at)
                    || rhs
                        .filter_map(|k| match &t[k] {
                            SrcTok::Ident(s) => Some(s),
                            _ => None,
                        })
                        .any(|s| imported.contains(s))
                {
                    imported.insert(name.clone());
                }
            }
        }
        if imported.len() == before {
            break;
        }
    }
    let from_backend = |p: &[String]| {
        p.first()
            .is_some_and(|f| is_backend_segment(backends, f) || imported.contains(f))
    };
    if globs.iter().any(|g| from_backend(g)) {
        return true;
    }
    // A `pub use` of a backend path: the `use` items preceded by a bare `pub`.
    for i in 1..t.len() {
        if tok_ident(t, i, "use") && tok_ident(t, i - 1, "pub") {
            let end = item_end(t, i).saturating_sub(1).max(i + 1);
            let mut found = Vec::new();
            let mut g = Vec::new();
            flatten_use(t, i + 1, end, &[], &mut found, &mut g);
            if found.iter().any(|(p, _)| from_backend(p)) || g.iter().any(|p| from_backend(p)) {
                return true;
            }
        }
    }
    let shows = |range: std::ops::Range<usize>| {
        range.clone().any(seg_at)
            || range
                .filter_map(|k| match &t[k] {
                    SrcTok::Ident(s) => Some(s),
                    _ => None,
                })
                .any(|s| imported.contains(s))
    };
    // The types this file declares private (no bare `pub` before the keyword).
    let private_type = |name: &str| {
        (0..t.len()).any(|k| {
            ["struct", "enum", "union"]
                .iter()
                .any(|kw| tok_ident(t, k, kw))
                && tok_ident(t, k + 1, name)
                && !(k > 0 && tok_ident(t, k - 1, "pub"))
        })
    };
    (0..t.len()).any(|i| {
        if tok_ident(t, i, "pub") && !tok_punct(t, i + 1, '(') && !tok_ident(t, i + 1, "use") {
            let body = (i + 1..(i + 4).min(t.len()))
                .any(|k| tok_ident(t, k, "enum") || tok_ident(t, k, "trait"));
            shows(i + 1..header_end(t, i + 1, body))
        } else if tok_ident(t, i, "impl") && impl_item_header(t, i) {
            let end = header_end(t, i + 1, false);
            match (i + 1..end).find(|k| tok_ident(t, *k, "for")) {
                Some(f) => {
                    let self_ty = match t.get(f + 1) {
                        Some(SrcTok::Ident(s)) => s.as_str(),
                        _ => "",
                    };
                    shows(f + 1..end) || (shows(i + 1..f) && !private_type(self_ty))
                }
                None => shows(i + 1..end),
            }
        } else {
            false
        }
    })
}
// ## m4b-t01a-seams

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
            // ## m4b-t07-git (named exception, fix round 3, p3): every file but the hooks' own
            // tests.
            if !(name == GIT_HOOKS_HOME && in_crate.starts_with("tests/")) {
                let (tokens, _, _) = effective_tokens(&f, &src);
                if names_git_testing_hooks(&tokens) {
                    found.insert((name.clone(), Rule::GitTestingHookOutsideTests, rel.clone()));
                }
            }
            // ## m4b-t07-git
            // ## m4b-t20-adversarial (named exception, carries T7 q1, T13 fr1 n1, T10)
            {
                let (tokens, _, _) = effective_tokens(&f, &src);
                if name == GIT_HOOKS_HOME && in_src && reexports_git_testing(&tokens) {
                    found.insert((name.clone(), Rule::GitTestingHookOutsideTests, rel.clone()));
                }
                if names_default_trash_ctx(&tokens) {
                    found.insert((name.clone(), Rule::TrashDefaultContext, rel.clone()));
                }
                let fileio_only = is_crate
                    && name == SCRIPTS
                    && (FILEIO_ONLY_FILES.contains(&in_crate.as_str())
                        || in_crate.starts_with(FILEIO_ONLY_DIR));
                if fileio_only && names_the_file_system(&tokens) {
                    found.insert((name.clone(), Rule::ScriptContentOutsideFileio, rel.clone()));
                }
            }
            // ## m4b-t20-adversarial
            // ## m4b-t01a-seams: every non-test file of a product crate (the crate's `tests/`
            // may run the git CLI on its own fixtures; xtask and the fuzz packages are tools).
            let product_file = is_crate
                && !in_crate.starts_with("tests/")
                && !NO_GIT_EXEMPT.contains(&name.as_str());
            let api_backends = PUBLIC_API_BACKENDS
                .iter()
                .find(|(owner, _)| in_src && *owner == name)
                .map(|(_, b)| *b);
            if product_file || api_backends.is_some() {
                let (tokens, _, _) = effective_tokens(&f, &src);
                if product_file && spawns_git(&tokens, NO_PROCESS_CRATES.contains(&name.as_str())) {
                    found.insert((name.clone(), Rule::NoGitSubprocess, rel.clone()));
                }
                // ## m4b-t01a-fr1 (I3): the one crate that may depend on `notify`.
                if product_file && name == SCRIPTS && watcher_follows_links(&tokens) {
                    found.insert((name.clone(), Rule::WatcherFollowsLinks, rel.clone()));
                }
                // ## m4b-t09-watcher: stand-ins for the names the rule above trusts.
                if product_file
                    && name == SCRIPTS
                    && watcher_decoys(&tokens, rel == WATCHER_CONFIG_HOME)
                {
                    found.insert((name.clone(), Rule::WatcherFollowsLinks, rel.clone()));
                }
                if let Some(b) = api_backends
                    && backend_in_public_api(&tokens, b)
                {
                    found.insert((name.clone(), Rule::BackendInPublicApi, rel.clone()));
                }
            }
            // ## m4b-t01a-seams
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

// ## m4-t33a-hygiene (T23 review m-6)
/// The `uses:` values in `text` (one workflow or action file) that name a third-party action
/// without a full commit SHA, as `line N: <value>`.
pub fn unpinned_uses(text: &str) -> Vec<String> {
    let hex = |s: &str, n: usize| {
        s.len() == n
            && s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    };
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        // ## m4-t33a-hygiene (fix round 1, review M3): `uses` is a key wherever it stands
        // (a block or flow mapping, quoted or not, a space before the colon).
        let code = line.split(" #").next().unwrap_or("");
        if code.trim_start().starts_with('#') {
            continue;
        }
        for value in uses_values(code) {
            if value.starts_with("./") {
                continue;
            }
            let pinned = if let Some(image) = value.strip_prefix("docker://") {
                image
                    .rsplit_once("@sha256:")
                    .is_some_and(|(name, digest)| !name.is_empty() && hex(digest, 64))
            } else {
                value
                    .rsplit_once('@')
                    .is_some_and(|(action, sha)| !action.is_empty() && hex(sha, 40))
            };
            if !pinned {
                out.push(format!("line {}: {value}", i + 1));
            }
        }
    }
    out
}

/// The values of every `uses` key on one YAML line (comment already cut): the key stands at
/// the line's start (after `- `), after `{` or `,`, optionally quoted, then optional spaces and
/// `:`; the value runs to `,` or `}` in a flow mapping, else to the end, quotes stripped.
fn uses_values(code: &str) -> Vec<String> {
    let b = code.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(off) = code[i..].find("uses") {
        let at = i + off;
        i = at + 4;
        let (start, end) = match (at.checked_sub(1).map(|p| b[p]), b.get(at + 4)) {
            (Some(q @ (b'"' | b'\'')), Some(&e)) if e == q => (at - 1, at + 5),
            _ => (at, at + 4),
        };
        let before = code[..start].trim();
        let key_position =
            before.is_empty() || before == "-" || before.ends_with('{') || before.ends_with(',');
        let rest = code[end..].trim_start();
        let (Some(rest), true) = (rest.strip_prefix(':'), key_position) else {
            continue;
        };
        let flow = before.contains('{');
        let value = if flow {
            rest.split([',', '}']).next().unwrap_or("")
        } else {
            rest
        };
        let value = value.trim().trim_matches(|c| c == '"' || c == '\'');
        out.push(value.to_string());
    }
    out
}

/// [`Rule::ActionNotPinnedBySha`] over every `*.yml`/`*.yaml` file under `.github/`.
pub fn action_pin_violations(workspace: &Path) -> Vec<Violation> {
    fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut paths: Vec<_> = entries.flatten().map(|e| e.path()).collect();
        paths.sort();
        for p in paths {
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|e| e == "yml" || e == "yaml") {
                out.push(p);
            }
        }
    }
    let mut files = Vec::new();
    walk(&workspace.join(".github"), &mut files);
    let mut found = Vec::new();
    for f in files {
        let rel = f
            .strip_prefix(workspace)
            .unwrap_or(&f)
            .to_string_lossy()
            .replace('\\', "/");
        // ## m4-t33a-hygiene (fix round 1, review M3): an unreadable file is a violation.
        let text = match std::fs::read_to_string(&f) {
            Ok(t) => t,
            Err(e) => {
                found.push(Violation {
                    krate: rel,
                    rule: Rule::ActionNotPinnedBySha,
                    via: vec![format!("unreadable: {e}")],
                });
                continue;
            }
        };
        for u in unpinned_uses(&text) {
            found.push(Violation {
                krate: rel.clone(),
                rule: Rule::ActionNotPinnedBySha,
                via: vec![u],
            });
        }
    }
    found
}
// ## m4-t33a-hygiene

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
    // ## m4-t33a-hygiene
    found.extend(action_pin_violations(metadata.workspace_root.as_std_path()));
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

    // ## m4-t33a-hygiene (T23 review m-6)
    #[test]
    fn unpinned_actions_are_found() {
        let sha = "11d5960a326750d5838078e36cf38b85af677262";
        let text = [
            "jobs:".to_string(),
            "  a:".to_string(),
            "    steps:".to_string(),
            "      - uses: actions/checkout@v4".to_string(),
            format!("      - uses: actions/checkout@{sha} # v4.4.0"),
            "      - uses: Swatinem/rust-cache@v2.9.3".to_string(),
            "      - uses: ./.github/actions/libclang".to_string(),
            "        uses: docker://alpine:3".to_string(),
            "      - uses: \"owner/repo/sub@main\"".to_string(),
            format!("      - uses: owner/repo@{}", &sha[..39]),
            format!("      - uses: owner/repo@{}", sha.to_ascii_uppercase()),
            format!("      - uses: '@{sha}'"),
        ]
        .join("\n");
        assert_eq!(
            unpinned_uses(&text),
            vec![
                "line 4: actions/checkout@v4".to_string(),
                "line 6: Swatinem/rust-cache@v2.9.3".to_string(),
                // ## m4-t33a-hygiene (fix round 1, M3): a `docker://` tag is no pin either.
                "line 8: docker://alpine:3".to_string(),
                "line 9: owner/repo/sub@main".to_string(),
                format!("line 10: owner/repo@{}", &sha[..39]),
                format!("line 11: owner/repo@{}", sha.to_ascii_uppercase()),
                format!("line 12: @{sha}"),
            ]
        );
    }

    /// Every action this repository's workflows use is pinned by commit SHA.
    #[test]
    fn the_repository_pins_every_action_by_sha() {
        let v = action_pin_violations(&workspace_root());
        let lines: Vec<String> = v
            .iter()
            .map(|v| format!("{}: {}", v.krate, v.via.join("")))
            .collect();
        assert!(lines.is_empty(), "{lines:#?}");
    }
    // ## m4-t33a-hygiene

    // ## m4-t33a-hygiene (fix round 1, review M3)
    /// `uses` is a YAML key wherever it stands: in a flow mapping, quoted, with a space before
    /// its colon. A `docker://` image is pinned only by `@sha256:` and 64 hex digits.
    #[test]
    fn unpinned_actions_are_found_in_every_yaml_form() {
        let sha = "11d5960a326750d5838078e36cf38b85af677262";
        let digest = "a".repeat(64);
        let text = [
            "      - { uses: actions/checkout@v4 }".to_string(),
            "      - { name: x, uses: actions/checkout@v4, with: { a: 1 } }".to_string(),
            "      - \"uses\": actions/checkout@v4".to_string(),
            "      - 'uses': actions/checkout@v4".to_string(),
            "      - uses : actions/checkout@v4".to_string(),
            "        uses: docker://alpine:3".to_string(),
            format!("        uses: docker://alpine@sha256:{}", &digest[..63]),
            format!("      - {{ uses: actions/checkout@{sha} }}"),
            format!("        uses: docker://alpine@sha256:{digest}"),
            "      # uses: actions/checkout@v4 (a comment)".to_string(),
            "        run: echo reuses: nothing".to_string(),
            "      - uses: ./.github/actions/libclang".to_string(),
        ]
        .join("\n");
        assert_eq!(
            unpinned_uses(&text),
            vec![
                "line 1: actions/checkout@v4".to_string(),
                "line 2: actions/checkout@v4".to_string(),
                "line 3: actions/checkout@v4".to_string(),
                "line 4: actions/checkout@v4".to_string(),
                "line 5: actions/checkout@v4".to_string(),
                "line 6: docker://alpine:3".to_string(),
                format!("line 7: docker://alpine@sha256:{}", &digest[..63]),
            ]
        );
    }

    /// A workflow file that cannot be read is a violation, never skipped.
    #[test]
    fn an_unreadable_workflow_file_is_a_violation() {
        let dir = tempfile::tempdir().unwrap();
        let wf = dir.path().join(".github/workflows");
        std::fs::create_dir_all(&wf).unwrap();
        std::fs::write(wf.join("bad.yml"), [0xff, 0xfe, 0x00, 0x80]).unwrap();
        let v = action_pin_violations(dir.path());
        assert_eq!(v.len(), 1, "{v:?}");
        assert_eq!(v[0].krate, ".github/workflows/bad.yml");
        assert!(v[0].via[0].starts_with("unreadable"), "{v:?}");
    }
    // ## m4-t33a-hygiene

    // ## m4b-t01a-seams
    /// The (crate, file) pairs of `rule` over the source tree at `dir`.
    fn source_hits(dir: &Path, rule: Rule) -> Vec<(String, String)> {
        source_violations(dir)
            .into_iter()
            .filter(|v| v.rule == rule)
            .map(|v| (v.krate, v.via[0].clone()))
            .collect()
    }

    #[test]
    fn notify_outside_scripts_is_flagged() {
        let md = fx_at(&[("hacivat-exec", "crates/core", &["notify"])]);
        assert_eq!(rule_of(&md, "hacivat-exec"), Rule::BackendOutsideOwner);
        let md = fx_at(&[("hacivat-app", "crates/app", &["notify-debouncer-full"])]);
        assert_eq!(rule_of(&md, "hacivat-app"), Rule::BackendOutsideOwner);
        let ok = fx_at(&[(
            "hacivat-scripts",
            "crates/core",
            &["notify", "notify-debouncer-full", "encoding_rs", "trash"],
        )]);
        assert!(violations(&ok, &lints_ok).is_empty());
    }

    #[test]
    fn gix_outside_git_is_flagged() {
        let md = fx_at(&[("hacivat-scripts", "crates/core", &["gix"])]);
        assert_eq!(rule_of(&md, "hacivat-scripts"), Rule::BackendOutsideOwner);
        let ok = fx_at(&[("hacivat-git", "crates/core", &["gix", "gix-path"])]);
        assert!(violations(&ok, &lints_ok).is_empty());
    }

    #[test]
    fn gix_subcrate_outside_git_is_flagged() {
        let md = fx_at(&[("hacivat-app", "crates/app", &["gix-status"])]);
        assert_eq!(rule_of(&md, "hacivat-app"), Rule::BackendOutsideOwner);
        for sub in ["gix-config", "gix-path", "gix-command", "gix-filter"] {
            let md = fx_at(&[("hacivat-scripts", "crates/core", &[sub])]);
            assert_eq!(
                rule_of(&md, "hacivat-scripts"),
                Rule::BackendOutsideOwner,
                "{sub}"
            );
        }
        // A dev-only edge counts too, and only the `gix-` prefix matches.
        let md = fx_kind_at(
            "hacivat-app",
            "crates/app",
            "gix-index",
            DepKind::Development,
            &[],
        );
        assert_eq!(rule_of(&md, "hacivat-app"), Rule::BackendOutsideOwner);
        let ok = fx_at(&[("hacivat-scripts", "crates/core", &["gixy", "digix"])]);
        assert!(violations(&ok, &lints_ok).is_empty());
    }

    #[test]
    fn encoding_rs_outside_scripts_is_flagged() {
        let md = fx_at(&[("hacivat-exec", "crates/core", &["encoding_rs"])]);
        assert_eq!(rule_of(&md, "hacivat-exec"), Rule::BackendOutsideOwner);
        let md = fx_at(&[("hacivat-app", "crates/app", &["trash"])]);
        assert_eq!(rule_of(&md, "hacivat-app"), Rule::BackendOutsideOwner);
        // A declared optional dependency no feature turns on is seen too.
        let md = metadata(vec![member(
            "hacivat-git",
            "crates/core",
            vec![Dep {
                optional: true,
                ..dep("encoding_rs", DepKind::Normal, &[], None)
            }],
        )]);
        assert!(
            violations(&md, &lints_ok)
                .iter()
                .any(|v| v.krate == "hacivat-git" && v.rule == Rule::BackendOutsideOwner),
            "{:?}",
            violations(&md, &lints_ok)
        );
    }

    #[test]
    fn scripts_reaching_exec_or_meta_is_flagged() {
        for bad in [
            "hacivat-exec",
            "hacivat-meta",
            "hacivat-session",
            "hacivat-driver-pg",
        ] {
            let md = fx_at(&[
                ("hacivat-scripts", "crates/core", &[bad]),
                (bad, "crates/core", &[]),
            ]);
            assert_eq!(
                rule_of(&md, "hacivat-scripts"),
                Rule::ScriptsBelowExec,
                "{bad}"
            );
        }
        // Transitively, through a normal edge.
        let md = fx_at(&[
            ("hacivat-scripts", "crates/core", &["helper"]),
            ("helper", "crates/core", &["russh"]),
        ]);
        assert_eq!(rule_of(&md, "hacivat-scripts"), Rule::ScriptsBelowExec);
        // A dev-dependency is allowed (the `secret_table` test, Task 19).
        let ok = fx_kind_at(
            "hacivat-scripts",
            "crates/core",
            "hacivat-exec",
            DepKind::Development,
            &[],
        );
        assert!(violations(&ok, &lints_ok).is_empty());
        // The run service never depends on the script workspace, by any edge.
        let md = fx_kind_at(
            "hacivat-exec",
            "crates/core",
            "hacivat-scripts",
            DepKind::Development,
            &[],
        );
        assert_eq!(rule_of(&md, "hacivat-exec"), Rule::ScriptsBelowExec);
    }

    #[test]
    fn hacivat_scripts_does_not_reach_exec_or_meta() {
        let md = cargo_metadata::MetadataCommand::new()
            .current_dir(crate::workspace_root())
            .exec()
            .unwrap();
        let pkgs: HashMap<&PackageId, &Package> = md.packages.iter().map(|p| (&p.id, p)).collect();
        let edges = edges_of(&md);
        let scripts = md
            .workspace_packages()
            .into_iter()
            .find(|p| p.name.as_str() == "hacivat-scripts")
            .expect("hacivat-scripts is a member");
        let path = normal_path_to(&scripts.id, &edges, &pkgs, |p| {
            scripts_forbidden(p.name.as_str())
        });
        assert_eq!(path, None);
        let exec = md
            .workspace_packages()
            .into_iter()
            .find(|p| p.name.as_str() == "hacivat-exec")
            .unwrap();
        assert!(
            exec.dependencies
                .iter()
                .all(|d| d.name != "hacivat-scripts")
        );
        let v = violations(&md, &|p| std::fs::read_to_string(p).unwrap_or_default());
        assert!(v.iter().all(|x| x.rule != Rule::ScriptsBelowExec), "{v:?}");
    }

    /// The M4b crates are Core members, product crates, and clean.
    #[test]
    fn m4b_crates_sit_in_their_layers_and_are_clean() {
        let md = cargo_metadata::MetadataCommand::new()
            .current_dir(crate::workspace_root())
            .exec()
            .unwrap();
        let v = violations(&md, &|p| std::fs::read_to_string(p).unwrap_or_default());
        for c in ["hacivat-scripts", "hacivat-git"] {
            let member = md
                .workspace_packages()
                .into_iter()
                .find(|p| p.name.as_str() == c)
                .unwrap_or_else(|| panic!("{c} is not a workspace member"));
            assert_eq!(layer_of(&md, member), Some(Layer::Core), "{c}");
            assert!(PRODUCT_CRATES.contains(&c), "{c}");
            assert!(v.iter().all(|x| x.krate != c), "{c}: {v:?}");
        }
    }

    #[test]
    fn git_subprocess_in_product_code_is_flagged() {
        let dir = source_tree(&[(
            "crates/core/hacivat-git/src/lib.rs",
            "let c = std::process::Command::new(\"git\");",
        )]);
        assert_eq!(
            source_hits(dir.path(), Rule::NoGitSubprocess),
            [(
                "hacivat-git".to_string(),
                "crates/core/hacivat-git/src/lib.rs".to_string()
            )]
        );
    }

    /// The M4 lesson (T1a I3, I-R2, T1c I3): every spelling of the program, wherever it is
    /// built, and whatever pulls the file in.
    #[test]
    fn git_subprocess_forms_are_all_flagged() {
        let forms: [(&str, &str); 16] = [
            (
                "alias.rs",
                "use std::process::Command as C;\nfn f() { C::new(\"git\").status(); }",
            ),
            (
                "constant.rs",
                "const GIT: &str = \"git\";\nfn f() { Command::new(GIT).spawn(); }",
            ),
            ("to_string.rs", "fn p() -> String { \"git\".to_string() }"),
            ("exe.rs", "fn f() { Command::new(\"git.exe\"); }"),
            ("raw.rs", "fn f() { Command::new(r#\"git\"#); }"),
            (
                "spaced.rs",
                "fn f() { Command :: new (\n    \"git\"\n  ) ; }",
            ),
            (
                "concat.rs",
                "fn f() { Command::new(concat!(\"gi\", \"t\")); }",
            ),
            ("abs.rs", "fn f() { Command::new(\"/usr/bin/git\"); }"),
            (
                "win.rs",
                "fn f() { Command::new(\"C:\\\\Program Files\\\\Git\\\\cmd\\\\GIT.EXE\"); }",
            ),
            ("bytes.rs", "fn f() { OsStr::from_bytes(b\"git\"); }"),
            (
                "shell.rs",
                "fn f() { Command::new(\"sh\").arg(\"-c\").arg(\"cd x && git status\"); }",
            ),
            ("upper.rs", "fn f() { Command::new(\"Git\"); }"),
            ("padded.rs", "fn f() { Command::new(\" git \"); }"),
            (
                "path_mod.rs",
                "#[path = \"../tests/support/run.rs\"]\nmod run;",
            ),
            ("include.rs", "include!(\"../tests/support/run.rs\");"),
            ("crate_raw.rs", "fn f() { Command::new(br\"git\"); }"),
        ];
        let mut files: Vec<(String, String)> = forms
            .iter()
            .map(|(f, src)| (format!("crates/app/hacivat-app/src/{f}"), src.to_string()))
            .collect();
        // Only the `#[path]` and `include!` forms reach this file; `tests/` itself is allowed.
        files.push((
            "crates/app/hacivat-app/tests/support/run.rs".into(),
            "pub fn run() { std::process::Command::new(\"git\"); }".into(),
        ));
        // No subprocess at all in the two M4b crates, whatever the program.
        files.push((
            "crates/core/hacivat-scripts/src/reveal.rs".into(),
            "fn f(p: &str) { std::process::Command::new(p); }".into(),
        ));
        files.push((
            "crates/core/hacivat-git/src/probe.rs".into(),
            "use std::process;\nfn f(p: &str) { process::Command::new(p); }".into(),
        ));
        files.push((
            "crates/core/hacivat-git/src/tokio_spawn.rs".into(),
            "fn f(p: &str) { tokio::process::Command::new(p); }".into(),
        ));
        let refs: Vec<(&str, &str)> = files
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect();
        let dir = source_tree(&refs);
        let hits: Vec<String> = source_hits(dir.path(), Rule::NoGitSubprocess)
            .into_iter()
            .map(|(_, f)| f)
            .collect();
        for (f, _) in &files {
            if f.contains("/tests/") {
                assert!(!hits.contains(f), "{f} is a test file: {hits:?}");
            } else {
                assert!(hits.contains(f), "{f} not flagged: {hits:?}");
            }
        }
    }

    #[test]
    fn git_subprocess_in_tests_is_allowed() {
        let dir = source_tree(&[
            (
                "crates/core/hacivat-git/tests/status_differential.rs",
                "fn git(args: &[&str]) { std::process::Command::new(\"git\").args(args); }",
            ),
            (
                "crates/core/hacivat-scripts/tests/hostile_library.rs",
                "use std::process::Command;\nfn f() { Command::new(\"git\"); }",
            ),
            // Not product code: the tools, the fuzz packages and the bench crate.
            ("xtask/src/minicat.rs", "let out = Command::new(\"git\");"),
            ("fuzz/scripts/fuzz_targets/x.rs", "Command::new(\"git\");"),
            (
                "crates/ui/hacivat-bench/src/lib.rs",
                "std::process::Command::new(\"git\")",
            ),
            // Words and paths that only contain the name, and comments.
            (
                "crates/core/hacivat-git/src/discover.rs",
                "// Command::new(\"git\") never\n/// `git status` is not run\nconst D: &str = \".git\";\nconst I: &str = \".git/index\";\nconst G: &str = \"gitdir: \";\nconst N: &str = \"digit\";",
            ),
        ]);
        assert_eq!(source_hits(dir.path(), Rule::NoGitSubprocess), []);
    }

    #[test]
    fn watcher_that_follows_links_is_flagged() {
        let forms: [(&str, &str); 7] = [
            (
                "plain.rs",
                "fn w() { notify_debouncer_full::new_debouncer(D, None, h); }",
            ),
            (
                "alias.rs",
                "use notify_debouncer_full::new_debouncer as nd;\nfn w() { nd(D, None, h); }",
            ),
            ("cache.rs", "fn c() -> FileIdMap { FileIdMap::new() }"),
            (
                "recommended.rs",
                "type C = notify_debouncer_full::RecommendedCache;",
            ),
            (
                "follows.rs",
                "fn c() -> Config { Config::default().with_follow_symlinks(true) }",
            ),
            (
                "default_config.rs",
                "fn w() { new_debouncer_opt::<_, RecommendedWatcher, NoCache>(D, None, h, NoCache, Config::default()); }",
            ),
            (
                "poll.rs",
                "fn p() { PollWatcher::new(h, Config::default().with_poll_interval(T)); }",
            ),
        ];
        let files: Vec<(String, String)> = forms
            .iter()
            .map(|(f, src)| {
                (
                    format!("crates/core/hacivat-scripts/src/watch/{f}"),
                    src.to_string(),
                )
            })
            .collect();
        let refs: Vec<(&str, &str)> = files
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect();
        let dir = source_tree(&refs);
        let hits: Vec<String> = source_hits(dir.path(), Rule::WatcherFollowsLinks)
            .into_iter()
            .map(|(_, f)| f)
            .collect();
        for (f, _) in &files {
            assert!(hits.contains(f), "{f} not flagged: {hits:?}");
        }
    }

    #[test]
    fn nocache_watcher_that_never_follows_links_is_allowed() {
        let dir = source_tree(&[
            (
                "crates/core/hacivat-scripts/src/watch/mod.rs",
                "// never `new_debouncer`: its FileIdMap follows links\n\
                 fn watcher_config() -> notify::Config { notify::Config::default().with_follow_symlinks(false) }\n\
                 fn w() { new_debouncer_opt::<_, RecommendedWatcher, NoCache>(D, None, h, NoCache, watcher_config()); }",
            ),
            (
                "crates/core/hacivat-scripts/src/watch/poll.rs",
                "fn p() { PollWatcher::new(h, Config::default().with_follow_symlinks(false)); }",
            ),
            // A test may build the hazard on purpose (a negative control).
            (
                "crates/core/hacivat-scripts/tests/watch_real.rs",
                "fn w() { new_debouncer(D, None, h); FileIdMap::new(); }",
            ),
        ]);
        assert_eq!(source_hits(dir.path(), Rule::WatcherFollowsLinks), []);
    }

    #[test]
    fn backend_type_in_an_owners_public_api_is_flagged() {
        let forms: [(&str, &str); 8] = [
            (
                "crates/core/hacivat-scripts/src/a.rs",
                "pub fn cfg() -> notify::Config { todo!() }",
            ),
            (
                "crates/core/hacivat-scripts/src/b.rs",
                "use notify::Config;\npub fn cfg(\n    x: u8,\n    c: Config,\n) {}",
            ),
            (
                "crates/core/hacivat-scripts/src/c.rs",
                "pub struct S { pub w: notify_debouncer_full::Debouncer<W, C> }",
            ),
            (
                "crates/core/hacivat-scripts/src/d.rs",
                "pub use encoding_rs::Encoding;",
            ),
            (
                "crates/core/hacivat-scripts/src/e.rs",
                "impl From<notify::Error> for WatchError { fn from(e: notify::Error) -> Self { todo!() } }",
            ),
            (
                "crates/core/hacivat-scripts/src/f.rs",
                "pub enum E { A(trash::Error), B }",
            ),
            (
                "crates/core/hacivat-git/src/g.rs",
                "pub trait T { fn repo(&self) -> gix::Repository; }",
            ),
            (
                "crates/core/hacivat-git/src/h.rs",
                "use gix_status::*;\nfn f() {}",
            ),
        ];
        let dir = source_tree(&forms);
        let hits: Vec<String> = source_hits(dir.path(), Rule::BackendInPublicApi)
            .into_iter()
            .map(|(_, f)| f)
            .collect();
        for (f, _) in &forms {
            assert!(hits.contains(&f.to_string()), "{f} not flagged: {hits:?}");
        }
    }

    #[test]
    fn private_backend_use_is_allowed() {
        let dir = source_tree(&[
            (
                "crates/core/hacivat-scripts/src/watch/mod.rs",
                "use notify::RecommendedWatcher;\nuse notify_debouncer_full::{Debouncer, NoCache};\n\
                 pub(crate) fn cfg() -> notify::Config { notify::Config::default() }\n\
                 pub struct OsEventSource { debouncer: Debouncer<RecommendedWatcher, NoCache> }\n\
                 fn private(c: notify::Config) {}\n\
                 pub(in crate::watch) fn scoped() -> notify::Config { todo!() }",
            ),
            (
                "crates/core/hacivat-scripts/src/service.rs",
                "pub async fn trash(&self, id: LibraryId) -> Result<(), FileOpError> { trash::delete(p) }",
            ),
            (
                "crates/core/hacivat-git/src/lib.rs",
                "pub fn status(p: &Path, l: &GitLimits) -> GitState { let r = gix::open(p); todo!() }",
            ),
            // Other crates are not owners (they cannot depend on the backends at all).
            (
                "crates/core/hacivat-exec/src/x.rs",
                "pub fn f() -> notify::Config { todo!() }",
            ),
        ]);
        assert_eq!(source_hits(dir.path(), Rule::BackendInPublicApi), []);
    }

    #[test]
    fn no_backend_type_in_public_api() {
        let root = workspace_root();
        for c in ["hacivat-scripts", "hacivat-git"] {
            assert!(
                root.join(format!("crates/core/{c}/src/lib.rs")).is_file(),
                "{c}"
            );
        }
        let all = source_violations(&root);
        for rule in [
            Rule::BackendInPublicApi,
            Rule::NoGitSubprocess,
            Rule::WatcherFollowsLinks,
            // ## m4b-t07-git (named exception, fix round 3, p3)
            Rule::GitTestingHookOutsideTests,
        ] {
            let v: Vec<_> = all.iter().filter(|v| v.rule == rule).collect();
            assert!(v.is_empty(), "{rule:?}: {v:?}");
        }
    }
    // ## m4b-t01a-seams

    // ## m4b-t01a-fr1 (review I1–I3, M1, M2: every probe form is a regression test)
    /// Writes `forms` under `dir_prefix`, plus `extra` verbatim; asserts every form file is a
    /// `rule` hit and every `ok` file is not.
    fn assert_forms(rule: Rule, dir_prefix: &str, forms: &[(&str, &str)], ok: &[(&str, &str)]) {
        let mut files: Vec<(String, String)> = forms
            .iter()
            .chain(ok)
            .map(|(f, src)| (format!("{dir_prefix}{f}"), src.to_string()))
            .collect();
        files.sort();
        let refs: Vec<(&str, &str)> = files
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect();
        let dir = source_tree(&refs);
        let hits: Vec<String> = source_hits(dir.path(), rule)
            .into_iter()
            .map(|(_, f)| f)
            .collect();
        for (f, _) in forms {
            let f = format!("{dir_prefix}{f}");
            assert!(hits.contains(&f), "{rule:?}: {f} not flagged: {hits:?}");
        }
        for (f, _) in ok {
            let f = format!("{dir_prefix}{f}");
            assert!(!hits.contains(&f), "{rule:?}: {f} flagged: {hits:?}");
        }
    }

    // ## m4b-t07-git (named exception, fix round 3, p3)
    /// `hacivat_git::testing` is for `hacivat-git`'s own tests only: any other file, in any
    /// crate (its `src/` or `tests/`), `fuzz/` or `xtask/`, that names it is flagged, however.
    #[test]
    fn git_testing_hooks_outside_its_tests_are_flagged() {
        let forms: [(&str, &str); 6] = [
            (
                "direct.rs",
                "fn f() { hacivat_git::testing::status_with_stall(p, l, s, || ()); }",
            ),
            ("use_item.rs", "use hacivat_git::testing::Stall;"),
            (
                "use_module.rs",
                "use hacivat_git::testing as t;\nfn f() { t::status_treating_as_dataless(p, l, &[]); }",
            ),
            (
                "renamed_crate.rs",
                "use hacivat_git as g;\nfn f() { g::testing::status_with_stall(p, l, s, || ()); }",
            ),
            (
                "grouped.rs",
                "use hacivat_git::{status, testing::{self, Stall}};",
            ),
            (
                "glob.rs",
                "use hacivat_git::*;\nfn f() { testing::status_treating_as_dataless(p, l, &[]); }",
            ),
        ];
        let ok: [(&str, &str); 1] = [(
            "fine.rs",
            "use hacivat_git::{GitLimits, status};\nmod testing { pub fn x() {} }\n\
             fn f() { status(p, &GitLimits::default()); testing::x(); let s = \"hacivat_git::testing\"; }",
        )];
        assert_forms(
            Rule::GitTestingHookOutsideTests,
            "crates/core/hacivat-scripts/src/",
            &forms,
            &ok,
        );
        assert_forms(
            Rule::GitTestingHookOutsideTests,
            "crates/app/hacivat-app/tests/",
            &forms,
            &ok,
        );
        assert_forms(
            Rule::GitTestingHookOutsideTests,
            "crates/core/hacivat-git/src/",
            &forms[..1],
            &ok,
        );
        // `hacivat-git`'s own tests are where the hooks belong.
        let dir = source_tree(&[
            ("crates/core/hacivat-git/tests/t.rs", forms[0].1),
            ("crates/core/hacivat-git/tests/u.rs", forms[3].1),
        ]);
        assert_eq!(
            source_hits(dir.path(), Rule::GitTestingHookOutsideTests),
            []
        );
    }
    // ## m4b-t07-git

    // ## m4b-t20-adversarial (named exception: Task 1a's file; carries T7 q1, T10, T13 fr1)
    /// T7 q1: a re-export inside `hacivat-git/src` would let outside code call the hooks
    /// without ever naming `testing`. Any public `use` through `testing`, or a public glob of a
    /// crate-local root, is flagged there; private imports and plain re-exports are not.
    #[test]
    fn git_testing_reexport_inside_hacivat_git_is_flagged() {
        let forms: [(&str, &str); 15] = [
            // Fix round 1 (review m1): aliases of the crate root, and typed aliases.
            ("alias_crate.rs", "pub use crate as hv;"),
            ("alias_extern.rs", "pub extern crate self as hv;"),
            (
                "alias_super.rs",
                "pub mod m { pub use super::super as root; }",
            ),
            ("alias_type.rs", "pub type Hook = crate::testing::Stall;"),
            ("alias_const.rs", "pub const H: fn() = crate::testing::x;"),
            ("alias_static.rs", "pub static H: fn() = self::testing::x;"),
            (
                "reexp_crate.rs",
                "pub use crate::testing::status_with_stall as stall_hook;",
            ),
            ("reexp_self.rs", "pub use self::testing::Stall;"),
            (
                "reexp_super.rs",
                "pub use super::testing::{Stall, status_treating_as_dataless};",
            ),
            ("reexp_bare.rs", "pub use testing::Stall;"),
            ("reexp_vis.rs", "pub(crate) use crate::testing::Stall;"),
            ("reexp_group.rs", "pub use crate::{status, testing::Stall};"),
            ("reexp_module.rs", "pub use crate::testing as hooks;"),
            (
                "reexp_renamed.rs",
                "use crate::testing as t;\npub use t::Stall;",
            ),
            ("reexp_glob.rs", "mod inner { pub use super::*; }"),
        ];
        let ok: [(&str, &str); 3] = [
            (
                "private_use.rs",
                "use crate::testing::Stall;\nfn f() { let _ = Stall::BeforeResolve; }",
            ),
            (
                "plain_reexport.rs",
                "pub mod testing { pub fn x() {} }\npub use crate::status::GitLimits;\n\
                 pub use self::map::{testing_free, Found};",
            ),
            (
                "plain_items.rs",
                "pub type Limits = crate::GitLimits;\npub const N: usize = crate::MAX;\n\
                 extern crate self as me;\ntype Private = crate::testing::Stall;",
            ),
        ];
        assert_forms(
            Rule::GitTestingHookOutsideTests,
            "crates/core/hacivat-git/src/",
            &forms,
            &ok,
        );
        // Only `hacivat-git`'s own `src/` re-exports its hooks; elsewhere a `pub use crate::…`
        // through a module of that name is that crate's own business.
        let dir = source_tree(&[(
            "crates/core/hacivat-scripts/src/x.rs",
            "pub use crate::testing::FakeHandle;",
        )]);
        assert_eq!(
            source_hits(dir.path(), Rule::GitTestingHookOutsideTests),
            []
        );
        // Fix round 1 (m1): a consumer reaching the hooks through such an alias is flagged.
        let dir = source_tree(&[
            (
                "crates/core/hacivat-scripts/src/alias.rs",
                "fn f() { let _ = hacivat_git::hv::testing::Stall::BeforeResolve; }",
            ),
            (
                "crates/core/hacivat-scripts/src/alias_use.rs",
                "use hacivat_git::hv as g;\nfn f() { g::testing::x(); }",
            ),
        ]);
        assert_eq!(
            source_hits(dir.path(), Rule::GitTestingHookOutsideTests).len(),
            2
        );
    }

    /// T13 fr1 n1: `trash::DEFAULT_TRASH_CTX` is a constant `clippy.toml` cannot ban (it is the
    /// Finder-script context on macOS). Any file that names it is flagged, however it is
    /// spelt; a string or a comment is not.
    #[test]
    fn default_trash_context_is_flagged() {
        let forms: [(&str, &str); 5] = [
            (
                "direct.rs",
                "fn f(p: &Path) { trash::DEFAULT_TRASH_CTX.delete(p); }",
            ),
            (
                "used.rs",
                "use trash::DEFAULT_TRASH_CTX;\nfn f(p: &Path) { DEFAULT_TRASH_CTX.delete(p); }",
            ),
            (
                "renamed.rs",
                "use trash::DEFAULT_TRASH_CTX as CTX;\nfn f(p: &Path) { CTX.delete(p); }",
            ),
            (
                "raw.rs",
                "fn f(p: &Path) { trash::r#DEFAULT_TRASH_CTX.delete(p); }",
            ),
            (
                "glob.rs",
                "use trash::*;\nfn f(p: &Path) { DEFAULT_TRASH_CTX.delete(p); }",
            ),
        ];
        let ok: [(&str, &str); 1] = [(
            "fine.rs",
            "// trash::DEFAULT_TRASH_CTX is refused\nconst S: &str = \"DEFAULT_TRASH_CTX\";\n\
             fn f(p: &Path) { os_trash_context().delete(p); }",
        )];
        for prefix in [
            "crates/core/hacivat-scripts/src/",
            "crates/core/hacivat-scripts/tests/",
            "crates/app/hacivat-app/src/",
        ] {
            assert_forms(Rule::TrashDefaultContext, prefix, &forms, &ok);
        }
    }

    /// T10 review / T5 mg2: the library service's sources (`service.rs`, `service/**`,
    /// `index.rs`, `events.rs`) read and write script content only through `fileio`. The
    /// token rule replaces T10's text tripwire: it resolves `use … as` and `extern crate … as`
    /// renames, brace groups and leading `::`, and a glob from outside the crate.
    #[test]
    fn script_content_outside_fileio_is_flagged() {
        let forms: [(&str, &str); 19] = [
            // Fix round 1 (review m2): the settings crate's path-based writer, and splicing.
            (
                "fsio.rs",
                "fn f() { hacivat_settings::fsio::atomic_write(p, b); }",
            ),
            (
                "fsio_rename.rs",
                "use hacivat_settings::fsio as w;\nfn f() { w::atomic_write(p, b); }",
            ),
            ("include.rs", "include!(\"../x.rs\");"),
            ("path_attr.rs", "#[path = \"../../x.rs\"]\nmod x;"),
            (
                "cfg_attr_path.rs",
                "#[cfg_attr(unix, path = \"../../x.rs\")]\nmod x;",
            ),
            ("std_fs.rs", "fn f() { std::fs::read(\"x\"); }"),
            ("rooted.rs", "fn f() { ::std::fs::read(\"x\"); }"),
            ("use_fs.rs", "use std::fs;\nfn f() { fs::write(p, b); }"),
            (
                "braced_rename.rs",
                "use std::{fs as disk};\nfn f() { disk::read(\"x\"); }",
            ),
            (
                "std_rename.rs",
                "use std as s;\nfn f() { s::fs::read(\"x\"); }",
            ),
            (
                "extern_rename.rs",
                "extern crate std as s;\nfn f() { s::fs::read(\"x\"); }",
            ),
            (
                "file_type.rs",
                "use std::fs::File as F;\nfn f() { F::open(p); }",
            ),
            ("tokio_fs.rs", "async fn f() { tokio::fs::read(p).await; }"),
            (
                "os_fs.rs",
                "use std::os::unix::fs::FileExt;\nfn f(x: &X) { x.read_at(b, 0); }",
            ),
            (
                "open_options.rs",
                "fn f() { OpenOptions::new().read(true).open(p); }",
            ),
            ("read_to.rs", "fn f(mut r: R) { r.read_to_string(&mut s); }"),
            ("listing.rs", "fn f() { for e in read_dir(p) {} }"),
            ("remove.rs", "fn f() { remove_file(p); }"),
            ("glob.rs", "use std::*;\nfn f() { fs::read(p); }"),
        ];
        let ok: [(&str, &str); 1] = [(
            "fine.rs",
            "use std::io;\nuse crate::fileio;\nuse super::*;\n\
             fn f(fs: &dyn Fs) { let _ = fs.symlink_metadata(p); let k = Probe::File(m);\n\
             fileio::read(r, l, e); let s = \"std::fs::read\"; }",
        )];
        for prefix in [
            "crates/core/hacivat-scripts/src/service/",
            "crates/core/hacivat-scripts/src/service/deeper/",
        ] {
            assert_forms(Rule::ScriptContentOutsideFileio, prefix, &forms, &ok);
        }
        for top in ["service.rs", "index.rs", "events.rs"] {
            let path = format!("crates/core/hacivat-scripts/src/{top}");
            let dir = source_tree(&[(path.as_str(), forms[3].1)]);
            assert_eq!(
                source_hits(dir.path(), Rule::ScriptContentOutsideFileio),
                [(
                    "hacivat-scripts".to_string(),
                    format!("crates/core/hacivat-scripts/src/{top}")
                )],
                "{top}"
            );
        }
        // `fileio`, `scan`, `manifest`, `registry`, `recovery` and the crate's tests are where
        // the file system is named.
        let dir = source_tree(&[
            ("crates/core/hacivat-scripts/src/fileio.rs", forms[0].1),
            ("crates/core/hacivat-scripts/src/scan.rs", forms[0].1),
            ("crates/core/hacivat-scripts/tests/service.rs", forms[0].1),
            ("crates/core/hacivat-git/src/service.rs", forms[0].1),
        ]);
        assert_eq!(
            source_hits(dir.path(), Rule::ScriptContentOutsideFileio),
            []
        );
    }

    /// The three rules above hold on the real workspace.
    #[test]
    fn t20_source_rules_hold_on_the_workspace() {
        let all = source_violations(&workspace_root());
        for rule in [
            Rule::GitTestingHookOutsideTests,
            Rule::TrashDefaultContext,
            Rule::ScriptContentOutsideFileio,
        ] {
            let v: Vec<_> = all.iter().filter(|v| v.rule == rule).collect();
            assert!(v.is_empty(), "{rule:?}: {v:?}");
        }
    }
    // ## m4b-t20-adversarial

    /// I1: no process can start from the two M4b crates, however the API is reached.
    #[test]
    fn m4b_crates_name_no_process_api() {
        let forms: [(&str, &str); 17] = [
            (
                "shell_command.rs",
                "fn f(x: &str) { gix::path::env::shell_command().arg(\"-c\").arg(x).spawn(); }",
            ),
            (
                "gix_command.rs",
                "fn f(x: &str) { gix::command::prepare(x).spawn(); }",
            ),
            (
                "gix_command_crate.rs",
                "fn f(x: &str) { gix_command::prepare(x).spawn(); }",
            ),
            (
                "renamed.rs",
                "use std::{fs, process as sp};\nfn f(x: &str) { sp::Command::new(x); }",
            ),
            (
                "tokio.rs",
                "fn f(x: &str) { tokio::process::Command::new(x); }",
            ),
            (
                "execvp.rs",
                "fn f(x: *const i8, a: *const *const i8) { unsafe { libc::execvp(x, a); } }",
            ),
            ("fork.rs", "fn f() { unsafe { libc::fork(); } }"),
            (
                "system_alias.rs",
                "use libc::system as run;\nfn f(c: *const i8) { unsafe { run(c); } }",
            ),
            (
                "posix_spawn.rs",
                "use libc::{O_NOFOLLOW, posix_spawn};\nfn f() {}",
            ),
            ("syscall.rs", "fn f() { unsafe { libc::syscall(59); } }"),
            ("command_ext.rs", "fn f(c: &mut impl CommandExt) {}"),
            (
                "gix_alias.rs",
                "use gix as g;\nfn f(x: &str) { g::command::prepare(x).spawn(); }",
            ),
            (
                "env_group.rs",
                "use gix::path::{env};\nfn f() { env::xdg_config(\"x\", &mut |_| None); }",
            ),
            (
                "gix_path_env.rs",
                "fn f() { gix_path::env::installation_config(); }",
            ),
            (
                "gix_glob.rs",
                "use gix::*;\nfn f(x: &str) { command::prepare(x).spawn(); }",
            ),
            (
                "libc_glob.rs",
                "use libc::*;\nfn f() { unsafe { fork(); } }",
            ),
            (
                "extern_crate.rs",
                "extern crate std as s;\nfn f(x: &str) { s::process::Command::new(x); }",
            ),
        ];
        let ok: [(&str, &str); 1] = [(
            "fine.rs",
            "use libc::{AT_FDCWD, O_NOFOLLOW};\nenum Command { Scan }\n\
             fn f() { tokio::spawn(async {}); tokio::task::spawn_blocking(|| ()); std::thread::spawn(|| ());\n\
             let command = Command::Scan; let processed = 1; let o = libc::O_NOFOLLOW;\n\
             let r = gix::open::Options::isolated().with(gix::sec::Trust::Reduced); let p = gix::path::from_bstr(x); }",
        )];
        assert_forms(
            Rule::NoGitSubprocess,
            "crates/core/hacivat-git/src/",
            &forms,
            &ok,
        );
        assert_forms(
            Rule::NoGitSubprocess,
            "crates/core/hacivat-scripts/src/",
            &forms,
            &ok,
        );
    }

    /// I2: the program name spelled at compile time, in any product crate.
    #[test]
    fn compile_time_git_spellings_are_flagged() {
        let forms: [(&str, &str); 10] = [
            ("stringify.rs", "fn f() { Command::new(stringify!(git)); }"),
            (
                "concat_char.rs",
                "fn f() { Command::new(concat!(\"gi\", 't')); }",
            ),
            (
                "concat_chars.rs",
                "fn f() { Command::new(concat!('g', 'i', 't')); }",
            ),
            (
                "concat_stringify.rs",
                "fn f() { Command::new(concat!(\"g\", stringify!(it))); }",
            ),
            ("hex_escape.rs", "fn f() { Command::new(\"g\\x69t\"); }"),
            (
                "unicode_escape.rs",
                "fn f() { Command::new(\"\\u{67}it\"); }",
            ),
            (
                "bytes_escape.rs",
                "fn f() { OsStr::from_bytes(b\"\\x67\\x69\\x74\"); }",
            ),
            (
                "continuation.rs",
                "fn f() { Command::new(\"gi\\\n        t\"); }",
            ),
            (
                "stringify_path.rs",
                "fn f() { Command::new(stringify!(/usr/bin/git)); }",
            ),
            (
                "char_escape.rs",
                "fn f() { Command::new(concat!('\\x67', \"it\")); }",
            ),
        ];
        let ok: [(&str, &str); 1] = [(
            "fine.rs",
            "const A: &str = \".git\"; const B: &str = \".git/index\"; const C: &str = \"gitdir: \";\n\
             const D: &str = \"scripts-git-badge-new\"; const E: &str = \"a .git file\";\n\
             const F: &str = stringify!(digit); const G: &str = concat!(\"di\", \"g\", \"it\");\n\
             const H: char = 'g'; const I: &str = \"g\\x69\";",
        )];
        assert_forms(
            Rule::NoGitSubprocess,
            "crates/app/hacivat-app/src/",
            &forms,
            &ok,
        );
    }

    /// I3: every watcher construction is checked where it happens.
    #[test]
    fn watcher_call_sites_that_can_follow_links_are_flagged() {
        let forms: [(&str, &str); 12] = [
            (
                "recommended.rs",
                "fn w(h: H) { notify::recommended_watcher(h); }",
            ),
            (
                "new_default.rs",
                "fn w(h: H) { RecommendedWatcher::new(h, notify::Config::default()); }",
            ),
            (
                "mixed.rs",
                "use notify_debouncer_full::NoCache;\n\
                 fn ok() -> notify::Config { notify::Config::default().with_follow_symlinks(false) }\n\
                 fn w(h: H) { new_debouncer_opt::<_, RecommendedWatcher, WalkingCache>(D, None, h, WalkingCache::new(), notify::Config::default()); }",
            ),
            (
                "poll_variable.rs",
                "fn p(h: H, follow: bool) { PollWatcher::new(h, watcher_config().with_follow_symlinks(follow)); }",
            ),
            (
                "inferred.rs",
                "fn w(h: H) { new_debouncer_opt(D, None, h, NoCache, watcher_config()); }",
            ),
            (
                "use_alias.rs",
                "use notify::RecommendedWatcher as W;\nfn w(h: H) { W::new(h, watcher_config()); }",
            ),
            (
                "type_alias.rs",
                "type W = notify::PollWatcher;\nfn w(h: H) { W::new(h, watcher_config()); }",
            ),
            (
                "qualified.rs",
                "fn w(h: H, c: notify::Config) { <notify::INotifyWatcher as notify::Watcher>::new(h, c); }",
            ),
            (
                "variable_config.rs",
                "fn w(h: H, c: notify::Config) { PollWatcher::new(h, c); }",
            ),
            (
                "bad_shared_config.rs",
                "fn watcher_config() -> notify::Config { Default::default() }",
            ),
            (
                "initial_scan.rs",
                "fn w(h: H, s: S) { PollWatcher::with_initial_scan(h, notify::Config::default(), s); }",
            ),
            (
                "fn_pointer.rs",
                "fn w() { let f = new_debouncer_opt::<_, RecommendedWatcher, NoCache>; }",
            ),
        ];
        let ok: [(&str, &str); 2] = [
            (
                "mod.rs",
                "pub(crate) fn watcher_config() -> notify::Config {\n    notify::Config::default().with_follow_symlinks(false)\n}\n\
                 fn w(h: H) { new_debouncer_opt::<_, RecommendedWatcher, NoCache>(D, None, h, NoCache, watcher_config()); }",
            ),
            (
                "poll.rs",
                "use super::watcher_config;\n\
                 fn p(h: H) { PollWatcher::new(h, watcher_config().with_poll_interval(T)); }\n\
                 fn q(h: H) { PollWatcher::new(h, notify::Config::default().with_follow_symlinks(false).with_poll_interval(T)); }",
            ),
        ];
        assert_forms(
            Rule::WatcherFollowsLinks,
            "crates/core/hacivat-scripts/src/watch/",
            &forms,
            &ok,
        );
    }

    // ## m4b-t09-watcher (named exception): carries-w1 T9 "hardening m1".
    /// A local cache, or a stand-in for `NoCache` or `watcher_config`, would pass the per-call
    /// check above; each is refused where it is defined.
    #[test]
    fn watcher_cache_and_config_stand_ins_are_flagged() {
        let forms: [(&str, &str); 9] = [
            (
                "local_cache.rs",
                "struct Walk;\nimpl notify_debouncer_full::FileIdCache for Walk { fn rescan(&mut self, r: &[(PathBuf, RecursiveMode)]) {} }",
            ),
            (
                "local_cache_imported.rs",
                "use notify_debouncer_full::FileIdCache as C;\nstruct W;\nimpl<X> C for W {}",
            ),
            (
                "nocache_struct.rs",
                "struct NoCache;\nfn w(h: H) { new_debouncer_opt::<_, RecommendedWatcher, NoCache>(D, None, h, NoCache, watcher_config()); }",
            ),
            ("nocache_alias.rs", "type NoCache = crate::walk::Walking;"),
            ("nocache_rename.rs", "use crate::walk::Walking as NoCache;"),
            (
                "config_fn_elsewhere.rs",
                "fn watcher_config() -> notify::Config { notify::Config::default().with_follow_symlinks(false) }",
            ),
            (
                "config_let.rs",
                "fn w(h: H, c: C) { let watcher_config = || c; PollWatcher::new(h, watcher_config()); }",
            ),
            (
                "config_rename.rs",
                "use crate::other::cfg as watcher_config;",
            ),
            (
                "config_param.rs",
                "fn w(h: H, watcher_config: fn() -> notify::Config) { RecommendedWatcher::new(h, watcher_config()); }",
            ),
        ];
        let ok: [(&str, &str); 2] = [
            (
                "mod.rs",
                "pub(crate) fn watcher_config() -> notify::Config {\n    notify::Config::default().with_follow_symlinks(false)\n}\n\
                 use notify_debouncer_full::{NoCache, new_debouncer_opt};\n\
                 pub struct S { d: Option<notify_debouncer_full::Debouncer<RecommendedWatcher, NoCache>> }\n\
                 fn w(h: H) { new_debouncer_opt::<_, RecommendedWatcher, NoCache>(D, None, h, NoCache, watcher_config()); }\n\
                 fn t() -> &'static str { std::any::type_name::<NoCache>() }",
            ),
            (
                "poll.rs",
                "use super::watcher_config;\nfn p() { let c = watcher_config(); let d = super::watcher_config(); }",
            ),
        ];
        assert_forms(
            Rule::WatcherFollowsLinks,
            "crates/core/hacivat-scripts/src/watch/",
            &forms,
            &ok,
        );
    }

    /// Fix round 1 (review m1): the home file holds exactly one top-level `watcher_config`,
    /// and a constructor takes only the bare call, never another module's.
    #[test]
    fn watcher_config_is_one_top_level_fn_called_bare() {
        let blessed = "pub(crate) fn watcher_config() -> notify::Config {\n    notify::Config::default().with_follow_symlinks(false)\n}\n";
        let homes = [
            format!(
                "{blessed}mod alt {{ fn watcher_config() -> Config {{ let _ = Config::default().with_follow_symlinks(false); Default::default() }} }}"
            ),
            format!(
                "{blessed}#[cfg(test)]\nfn watcher_config() -> notify::Config {{ notify::Config::default().with_follow_symlinks(false) }}"
            ),
        ];
        for home in &homes {
            assert_forms(
                Rule::WatcherFollowsLinks,
                "crates/core/hacivat-scripts/src/watch/",
                &[("mod.rs", home.as_str())],
                &[],
            );
        }
        let calls: [(&str, &str); 2] = [
            (
                "path_call.rs",
                "fn w(h: H) { new_debouncer_opt::<_, RecommendedWatcher, NoCache>(D, None, h, NoCache, alt::watcher_config()); }",
            ),
            (
                "method_call.rs",
                "fn w(h: H, x: X) { RecommendedWatcher::new(h, x.watcher_config()); }",
            ),
        ];
        let ok: [(&str, &str); 1] = [(
            "mod.rs",
            "pub(crate) fn watcher_config() -> notify::Config {\n    notify::Config::default().with_follow_symlinks(false)\n}\n\
             fn w(h: H) { new_debouncer_opt::<_, RecommendedWatcher, NoCache>(D, None, h, NoCache, watcher_config()); }",
        )];
        assert_forms(
            Rule::WatcherFollowsLinks,
            "crates/core/hacivat-scripts/src/watch/",
            &calls,
            &ok,
        );
    }
    // ## m4b-t09-watcher

    /// M1: aliases and re-exports cannot hide a backend type; a backend trait implemented for
    /// a private type leaks nothing.
    #[test]
    fn backend_aliases_and_reexports_in_public_api_are_flagged() {
        let forms: [(&str, &str); 4] = [
            (
                "type_alias.rs",
                "type Inner = notify::Config;\npub fn cfg() -> Inner { todo!() }",
            ),
            ("crate_alias.rs", "use notify as n;\npub use n::Config;"),
            (
                "alias_chain.rs",
                "type A = notify::Event;\ntype B = Vec<A>;\npub struct S(pub B);",
            ),
            (
                "public_impl.rs",
                "pub struct Sink;\nimpl notify_debouncer_full::DebounceEventHandler for Sink { fn handle_event(&mut self, e: R) {} }",
            ),
        ];
        let ok: [(&str, &str); 1] = [(
            "private_impl.rs",
            "struct Sink;\n\
             impl notify_debouncer_full::DebounceEventHandler for Sink {\n\
                 fn handle_event(&mut self, e: notify_debouncer_full::DebounceEventResult) {}\n\
             }\n\
             pub(crate) struct Inner;\nimpl From<notify::Error> for Inner { fn from(e: notify::Error) -> Self { Inner } }",
        )];
        assert_forms(
            Rule::BackendInPublicApi,
            "crates/core/hacivat-scripts/src/",
            &forms,
            &ok,
        );
    }

    /// M2: the network and secrets stack, and build-dependency edges.
    #[test]
    fn scripts_reaching_net_secrets_rustls_or_by_build_edges_is_flagged() {
        for bad in ["hacivat-net", "hacivat-secrets", "rustls"] {
            let md = fx_at(&[("hacivat-scripts", "crates/core", &[bad])]);
            assert_eq!(
                rule_of(&md, "hacivat-scripts"),
                Rule::ScriptsBelowExec,
                "{bad}"
            );
        }
        let md = fx_kind_at(
            "hacivat-scripts",
            "crates/core",
            "hacivat-session",
            DepKind::Build,
            &[],
        );
        assert_eq!(rule_of(&md, "hacivat-scripts"), Rule::ScriptsBelowExec);
        // Through a build edge, then a normal one.
        let md = metadata(vec![
            member(
                "hacivat-scripts",
                "crates/core",
                vec![dep("helper", DepKind::Build, &[], None)],
            ),
            member("helper", "crates/core", normal(&["russh"])),
        ]);
        assert_eq!(rule_of(&md, "hacivat-scripts"), Rule::ScriptsBelowExec);
    }

    /// M2: the watcher's lower crates are owned by `hacivat-scripts` too.
    #[test]
    fn watcher_backend_crates_outside_scripts_are_flagged() {
        for b in [
            "notify-types",
            "file-id",
            "inotify",
            "kqueue",
            "fsevent-sys",
        ] {
            let md = fx_at(&[("hacivat-app", "crates/app", &[b])]);
            assert_eq!(
                rule_of(&md, "hacivat-app"),
                Rule::BackendOutsideOwner,
                "{b}"
            );
        }
    }
    // ## m4b-t01a-fr1
}
