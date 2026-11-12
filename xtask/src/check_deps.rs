//! Dependency guard: transitive GPUI/zed reach, crate layers and workspace lints.
//!
//! Only `hacivat-ui-core` (the adapter) may reach `gpui`, `gpui_platform` or any zed-sourced
//! crate. F1 exception: a direct dev-dependency of a Ui or App crate on `gpui` with
//! `test-support`, because `#[gpui::test]` expands to hard-coded `gpui::` paths.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::path::Path;
use std::process::ExitCode;

use cargo_metadata::{DependencyKind, Metadata, MetadataCommand, Package, PackageId};

const ADAPTER: &str = "hacivat-ui-core";
const GUARDED: [&str; 2] = ["gpui", "gpui_platform"];
/// Adapter features that must never be enabled by a product build (F33, R12).
const ADAPTER_ONLY_FEATURES: [&str; 4] = ["spike-support", "bench", "scene-check", "test-support"];
const PRODUCT_CRATES: [&str; 2] = ["hacivat", "hacivat-app"];

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

fn layer_of(md: &Metadata, p: &Package) -> Layer {
    let abs = p.manifest_path.as_std_path();
    let rel = abs
        .strip_prefix(md.workspace_root.as_std_path())
        .unwrap_or(abs);
    let rel = rel.to_string_lossy().replace('\\', "/");
    if rel.starts_with("crates/ui/") {
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
    } else {
        Layer::Tool
    }
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

    for root in &members {
        let name = root.name.as_str();
        let layer = layer_of(md, root);

        if !has_workspace_lints(&manifest(root.manifest_path.as_std_path())) {
            add(name, Rule::MissingWorkspaceLints, vec![]);
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

        if layer == Layer::Ui && name != ADAPTER {
            for d in &root.dependencies {
                if members
                    .iter()
                    .any(|w| w.name.as_str() == d.name && layer_of(md, w) == Layer::Core)
                {
                    add(name, Rule::UiDependsOnCore, vec![d.name.clone()]);
                }
            }
        }

        // The adapter is the one crate allowed to reach GPUI.
        if name == ADAPTER {
            continue;
        }

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
                if matches!(layer_of(md, from), Layer::Ui | Layer::App)
                    && is_test_support_dev_edge(from, dep, kinds)
                {
                    continue;
                }
                if dep.name.as_str() == ADAPTER {
                    if layer == Layer::Core {
                        add(name, Rule::CoreReachesUi, via);
                    }
                    continue;
                }
                if is_guarded(dep) {
                    add(name, Rule::ZedOutsideAdapter, via);
                    continue;
                }
                if seen.insert(dep_id) {
                    queue.push_back((dep_id, via));
                }
            }
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
    }

    fn dep(name: &str, kind: DepKind, features: &[&str], source: Option<&str>) -> Dep {
        Dep {
            name: name.into(),
            kind,
            features: features.iter().map(|f| (*f).into()).collect(),
            source: source.map(|s| format!("git+{s}?rev=abc#abc")),
        }
    }

    fn dir_for(name: &str) -> &'static str {
        match name {
            "hacivat" => "crates/bin",
            "hacivat-app" => "crates/app",
            "hacivat-ui" | "hacivat-ui-core" => "crates/ui",
            "hacivat-l10n" => "crates/core",
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
                "rename": null, "optional": false, "uses_default_features": true,
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
