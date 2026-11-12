//! Guard: only `hacivat-ui-core` may depend directly on `gpui` / `gpui_platform`.

use std::process::ExitCode;

use cargo_metadata::{Metadata, MetadataCommand};

const ADAPTER: &str = "hacivat-ui-core";
const GUARDED: [&str; 2] = ["gpui", "gpui_platform"];

/// Names of workspace crates other than the adapter that depend directly on GPUI.
pub fn offending_crates(metadata: &Metadata) -> Vec<String> {
    metadata
        .workspace_packages()
        .into_iter()
        .filter(|p| p.name.as_str() != ADAPTER)
        .filter(|p| {
            p.dependencies
                .iter()
                .any(|d| GUARDED.contains(&d.name.as_str()))
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
    use serde_json::json;

    /// Builds a `Metadata` where each `(name, deps)` is a workspace member.
    fn fixture_metadata(crates: &[(&str, &[&str])]) -> Metadata {
        let packages: Vec<_> = crates
            .iter()
            .map(|(name, deps)| {
                json!({
                    "name": name,
                    "version": "0.0.0",
                    "id": format!("path+file:///ws/{name}#0.0.0"),
                    "source": null,
                    "dependencies": deps.iter().map(|d| json!({
                        "name": d, "source": null, "req": "*", "kind": null,
                        "rename": null, "optional": false, "uses_default_features": true,
                        "features": [], "target": null, "registry": null
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
}
