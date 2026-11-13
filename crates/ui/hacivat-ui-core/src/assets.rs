//! Bundled SVG icons, served to GPUI by `app::run_app` (`Application::with_assets`).

use std::borrow::Cow;

use anyhow::Result;
use gpui::{AssetSource, SharedString};

macro_rules! lucide {
    ($($n:literal),* $(,)?) => {
        &[$((
            concat!("icons/lucide/", $n, ".svg"),
            include_bytes!(concat!("../assets/icons/lucide/", $n, ".svg")) as &[u8],
        )),*]
    };
}

/// Lucide v0.460.0 (ISC) icons: the harman mock set, then the M1 shell set (window controls,
/// menus, toasts, settings, keymap, panes).
const ICONS: &[(&str, &[u8])] = lucide![
    "lock",
    "arrow-left-right",
    "chevrons-up-down",
    "search",
    "layout-panel-left",
    "moon",
    "sun",
    "database",
    "cylinder",
    "layers",
    "table-properties",
    "table-2",
    "eye",
    "square-function",
    "list-ordered",
    "radio",
    "chevron-right",
    "chevron-down",
    "chevron-left",
    "file-code-2",
    "x",
    "play",
    "list-tree",
    "align-left",
    "columns-2",
    "calendar",
    "brackets",
    "key-round",
    "check",
    "filter",
    "arrow-down",
    "sigma",
    "copy",
    "download",
    "trash-2",
    "file-diff",
    "undo-2",
    "panel-right",
    "shield-check",
    "languages",
    "git-commit-horizontal",
    // M1 shell.
    "menu",
    "minus",
    "square",
    "plus",
    "settings",
    "info",
    "triangle-alert",
    "circle-check",
    "ellipsis",
    "keyboard",
    "rows-2",
    "panel-left",
    // M2a connections (Task 1).
    "lock-open",
    "rotate-cw",
    "clipboard-paste",
    "plug",
    "unplug",
    "pencil",
    "external-link",
    "route",
    "circle-alert",
    "circle-x",
    "folder-open",
    "file-key",
    // ## m2b-t24-shots: the M2b connection icons (`key-round` is above).
    "file-up",
    "arrow-right",
];
const HV_MARK: &[u8] = include_bytes!("../assets/icons/hacivat/hv-mark.svg");

/// Serves the bundled icons under `icons/lucide/<name>.svg` and `icons/hacivat/hv-mark.svg`.
pub struct BundledAssets;

impl AssetSource for BundledAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path == "icons/hacivat/hv-mark.svg" {
            return Ok(Some(Cow::Borrowed(HV_MARK)));
        }
        Ok(ICONS
            .iter()
            .find(|(p, _)| *p == path)
            .map(|(_, b)| Cow::Borrowed(*b)))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(ICONS
            .iter()
            .filter(|(p, _)| p.starts_with(path))
            .map(|(p, _)| SharedString::from(*p))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_icons_include_m1_set() {
        for n in [
            "menu", "minus", "square", "copy", "settings", "ellipsis", "keyboard",
        ] {
            assert!(
                BundledAssets
                    .load(&format!("icons/lucide/{n}.svg"))
                    .unwrap()
                    .is_some(),
                "{n}"
            );
        }
    }
}
