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
const PRODUCT_CRATES: [&str; 7] = [
    "hacivat",
    "hacivat-app",
    // ## m3a-t01b-seams (P22): the new Ui and App crates ship in the product too.
    "hacivat-ui-editor",
    "hacivat-ui-grid",
    "hacivat-sql-editor",
    "hacivat-text",
    "hacivat-ui",
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
    /// A crate other than `sql-oracle-pg` depends on `pg_query` directly.
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
/// The one crate that may depend on `pg_query` (R-M3-1).
const PG_BACKEND: (&str, &str) = ("pg_query", "sql-oracle-pg");
/// The headless editor core (`TextCoreNotHeadless`).
const TEXT_CORE: &str = "hacivat-text";
/// The rope under `hacivat-text`, and the features that change its line breaks.
const ROPE: &str = "ropey";
const ROPE_LINE_FEATURES: [&str; 2] = ["unicode_lines", "default"];

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
        if name != PG_BACKEND.1 {
            for (dep_id, _) in edges.get(&root.id).into_iter().flatten() {
                if pkgs
                    .get(dep_id)
                    .is_some_and(|d| d.name.as_str() == PG_BACKEND.0)
                {
                    add(name, Rule::BackendOutsideOwner, vec![PG_BACKEND.0.into()]);
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

pub fn run() -> ExitCode {
    let metadata = match MetadataCommand::new().exec() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("cargo metadata failed: {e}");
            return ExitCode::from(2);
        }
    };
    let found = violations(&metadata, &|p| {
        std::fs::read_to_string(p).unwrap_or_default()
    });
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
    }

    struct Dep {
        name: String,
        kind: DepKind,
        features: Vec<String>,
        source: Option<String>,
        /// `default-features` (true unless a fixture turns it off, as the workspace's `ropey`).
        default_features: bool,
    }

    fn dep(name: &str, kind: DepKind, features: &[&str], source: Option<&str>) -> Dep {
        Dep {
            name: name.into(),
            kind,
            features: features.iter().map(|f| (*f).into()).collect(),
            source: source.map(|s| format!("git+{s}?rev=abc#abc")),
            default_features: true,
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
                "rename": null, "optional": false, "uses_default_features": d.default_features,
                "features": d.features, "target": null, "registry": null
            })
        };
        let package = |name: &str, source: Option<String>, dir: String, deps: &[Dep]| {
            json!({
                "name": name, "version": "0.0.0", "id": pkg_id(name), "source": source,
                "dependencies": deps.iter().map(dependency).collect::<Vec<_>>(),
                "targets": [], "features": {},
                "manifest_path": format!("/ws/{dir}/Cargo.toml"),
                "metadata": null, "publish": null, "authors": [], "categories": [],
                "keywords": [], "readme": null, "repository": null, "homepage": null,
                "documentation": null, "edition": "2024", "links": null,
                "description": null, "license": null, "license_file": null
            })
        };
        let mut packages: Vec<_> = members
            .iter()
            .map(|m| package(&m.name, None, format!("{}/{}", m.dir, m.name), &m.deps))
            .collect();
        let mut externals: Vec<(String, Option<String>)> = Vec::new();
        for d in members.iter().flat_map(|m| &m.deps) {
            if !is_member(&d.name) && !externals.iter().any(|(n, _)| *n == d.name) {
                externals.push((d.name.clone(), d.source.clone()));
            }
        }
        for (name, source) in externals {
            packages.push(package(&name, source, format!("ext/{name}"), &[]));
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
}
