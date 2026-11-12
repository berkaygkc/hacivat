//! Guard: only `hacivat-ui-core` may depend directly on `gpui` / `gpui_platform`.
//!
//! F1 exception: a dev-dependency that enables `test-support` is allowed, because
//! `#[gpui::test]` expands to hard-coded `gpui::` paths. Task 2 replaces this M0 check.

use std::process::ExitCode;

use cargo_metadata::{Dependency, DependencyKind, Metadata, MetadataCommand};

const ADAPTER: &str = "hacivat-ui-core";
const GUARDED: [&str; 2] = ["gpui", "gpui_platform"];

/// A dev-dependency with `test-support` (F1) is the only allowed non-adapter GPUI edge.
fn is_test_support_dev_edge(d: &Dependency) -> bool {
    d.kind == DependencyKind::Development && d.features.iter().any(|f| f == "test-support")
}

/// Names of workspace crates other than the adapter that depend directly on GPUI.
pub fn offending_crates(metadata: &Metadata) -> Vec<String> {
    metadata
        .workspace_packages()
        .into_iter()
        .filter(|p| p.name.as_str() != ADAPTER)
        .filter(|p| {
            p.dependencies
                .iter()
                .any(|d| GUARDED.contains(&d.name.as_str()) && !is_test_support_dev_edge(d))
        })
        .map(|p| p.name.to_string())
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
    let offenders = offending_crates(&metadata);
    if offenders.is_empty() {
        println!("check-deps: ok");
        ExitCode::SUCCESS
    } else {
        eprintln!("check-deps: only {ADAPTER} may depend on gpui/gpui_platform; offending crates:");
        for name in &offenders {
            eprintln!("  {name}");
        }
        ExitCode::FAILURE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace_root;
    use cargo_metadata::DependencyKind as DepKind;
    use serde_json::json;

    /// `(dependency name, kind, enabled features)`.
    type DepSpec<'a> = (&'a str, DepKind, &'a [&'a str]);

    /// Builds a `Metadata` where each `(name, deps)` is a workspace member with normal deps.
    fn fixture_metadata(crates: &[(&str, &[&str])]) -> Metadata {
        let owned: Vec<(&str, Vec<DepSpec>)> = crates
            .iter()
            .map(|(name, deps)| {
                (
                    *name,
                    deps.iter()
                        .map(|d| (*d, DepKind::Normal, &[][..]))
                        .collect(),
                )
            })
            .collect();
        let borrowed: Vec<(&str, &[DepSpec])> =
            owned.iter().map(|(n, d)| (*n, d.as_slice())).collect();
        fixture_metadata_kinds(&borrowed)
    }

    /// Builds a `Metadata` where each `(name, [(dep, kind, features)])` is a workspace member.
    fn fixture_metadata_kinds(crates: &[(&str, &[DepSpec])]) -> Metadata {
        let kind_str = |k: &DepKind| match k {
            DepKind::Development => json!("dev"),
            DepKind::Build => json!("build"),
            _ => json!(null),
        };
        let packages: Vec<_> = crates
            .iter()
            .map(|(name, deps)| {
                json!({
                    "name": name,
                    "version": "0.0.0",
                    "id": format!("path+file:///ws/{name}#0.0.0"),
                    "source": null,
                    "dependencies": deps.iter().map(|(d, kind, features)| json!({
                        "name": d, "source": null, "req": "*", "kind": kind_str(kind),
                        "rename": null, "optional": false, "uses_default_features": true,
                        "features": features, "target": null, "registry": null
                    })).collect::<Vec<_>>(),
                    "targets": [], "features": {},
                    "manifest_path": format!("/ws/{name}/Cargo.toml"),
                    "metadata": null, "publish": null, "authors": [], "categories": [],
                    "keywords": [], "readme": null, "repository": null, "homepage": null,
                    "documentation": null, "edition": "2024", "links": null,
                    "description": null, "license": null, "license_file": null
                })
            })
            .collect();
        let ids: Vec<_> = crates
            .iter()
            .map(|(n, _)| format!("path+file:///ws/{n}#0.0.0"))
            .collect();
        serde_json::from_value(json!({
            "packages": packages,
            "workspace_members": ids,
            "workspace_default_members": ids,
            "resolve": null,
            "target_directory": "/ws/target",
            "version": 1,
            "workspace_root": "/ws",
            "metadata": null
        }))
        .unwrap()
    }

    #[test]
    fn flags_spike_that_depends_on_gpui() {
        let md = fixture_metadata(&[
            ("spike-x", &["gpui"]),
            ("hacivat-ui-core", &["gpui", "gpui_platform"]),
        ]);
        assert_eq!(offending_crates(&md), vec!["spike-x".to_string()]);
    }

    #[test]
    fn adapter_itself_is_allowed() {
        let md = fixture_metadata(&[("hacivat-ui-core", &["gpui"])]);
        assert!(offending_crates(&md).is_empty());
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

    #[test]
    fn test_support_dev_dependency_is_not_an_offender() {
        let md = fixture_metadata_kinds(&[(
            "hacivat-ui",
            &[("gpui", DepKind::Development, &["test-support"])],
        )]);
        assert!(offending_crates(&md).is_empty());
        let md = fixture_metadata_kinds(&[(
            "hacivat-ui",
            &[("gpui", DepKind::Normal, &["test-support"])],
        )]);
        assert_eq!(offending_crates(&md), ["hacivat-ui"]);
    }
}
