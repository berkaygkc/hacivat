//! Catalog discovery and loading (I18N-02): the embedded `locales/` tree, an optional on-disk
//! overlay layered over it per message ID, and the diagnostics of both.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use fluent_bundle::FluentResource;
use serde::Deserialize;
use unic_langid::LanguageIdentifier;

/// Release maturity of a locale (`locale.toml` `status`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Status {
    Stable,
    Beta,
    Experimental,
}

/// Writing direction (`locale.toml` `direction`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Direction {
    #[default]
    Ltr,
    Rtl,
}

/// Where a locale's files come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CatalogSource {
    Embedded,
    Overlay,
    Both,
}

/// One available locale.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CatalogLocale {
    pub tag: LanguageIdentifier,
    /// `tag` with likely subtags added.
    pub max: LanguageIdentifier,
    pub native_name: String,
    pub english_name: String,
    /// The language switcher's label (`EN`, `TR`).
    pub short_name: String,
    pub direction: Direction,
    pub status: Status,
    pub source: CatalogSource,
}

/// A problem found while loading catalogs.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LoadDiag {
    NonCanonicalTag {
        folder: String,
    },
    MissingManifest {
        tag: String,
    },
    /// `locale.toml` exists but does not parse; its defaults apply (as for a missing one).
    BadManifest {
        tag: String,
        message: String,
    },
    Junk {
        file: String,
        line: u32,
    },
    ReservedPseudo {
        folder: String,
    },
    OverlaySkipped {
        path: PathBuf,
        reason: String,
    },
}

/// Pseudo-locale tags: generated from `en` (I18N-16), never loaded from a folder.
pub(crate) const PSEUDO_TAGS: [&str; 3] = ["en-XA", "tr-XT", "ar-XB"];

/// Overlay files larger than this are skipped (I18N-02 step 5).
const MAX_OVERLAY_FILE: u64 = 1024 * 1024;

#[derive(rust_embed::Embed)]
#[folder = "../../../locales/"]
#[include = "*/*.ftl"]
#[include = "*/locale.toml"]
#[exclude = "_*/**"]
struct Embedded;

/// Which layer a file belongs to: overlay messages replace embedded ones per ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Layer {
    Embedded,
    Overlay,
}

/// One parsed FTL file.
#[derive(Clone)]
pub(crate) struct CatalogFile {
    pub(crate) tag: String,
    /// `<tag>/<domain>.ftl`.
    pub(crate) path: String,
    pub(crate) layer: Layer,
    pub(crate) resource: Arc<FluentResource>,
}

impl fmt::Debug for CatalogFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CatalogFile")
            .field("path", &self.path)
            .field("layer", &self.layer)
            .finish_non_exhaustive()
    }
}

/// The embedded catalogs plus an optional on-disk overlay.
#[derive(Debug, Clone, Default)]
pub struct Catalogs {
    locales: Vec<CatalogLocale>,
    diags: Vec<LoadDiag>,
    /// Embedded files first, then overlays in the order they were added.
    files: Vec<CatalogFile>,
}

impl Catalogs {
    /// The catalogs embedded from `locales/` (skipping `_`-prefixed folders).
    pub fn embedded() -> Catalogs {
        let raw = Embedded::iter()
            .filter_map(|path| {
                let file = Embedded::get(&path)?;
                let text = String::from_utf8_lossy(&file.data).into_owned();
                Some((path.into_owned(), text))
            })
            .collect();
        Catalogs::scan(raw, Layer::Embedded)
    }

    /// Layers the overlay directory `dir` over `self`, per message ID. New locales are at most
    /// experimental; unsafe files are skipped with a diagnostic (I18N-02 steps 2, 4, 5).
    pub fn with_overlay(mut self, dir: &Path) -> Catalogs {
        let mut diags = Vec::new();
        let raw = read_overlay(dir, &mut diags);
        let overlay = Catalogs::scan(raw, Layer::Overlay);
        self.diags.extend(diags);
        self.diags.extend(overlay.diags);
        for mut loc in overlay.locales {
            if let Some(existing) = self.locales.iter_mut().find(|l| l.tag == loc.tag) {
                existing.source = CatalogSource::Both;
            } else {
                loc.status = Status::Experimental;
                loc.source = CatalogSource::Overlay;
                self.locales.push(loc);
            }
        }
        self.locales.sort_by_key(|l| l.tag.to_string());
        self.files.extend(overlay.files);
        self
    }

    /// Catalogs from `(path, contents)` pairs such as `("en/hui.ftl", "...")`.
    pub fn from_memory(files: &[(&str, &str)]) -> Catalogs {
        let raw = files
            .iter()
            .map(|(p, t)| (p.to_string(), t.to_string()))
            .collect();
        Catalogs::scan(raw, Layer::Embedded)
    }

    pub fn locales(&self) -> &[CatalogLocale] {
        &self.locales
    }

    pub fn diags(&self) -> &[LoadDiag] {
        &self.diags
    }

    /// The files of locale `tag`, embedded before overlay.
    pub(crate) fn files_of<'a>(&'a self, tag: &'a str) -> impl Iterator<Item = &'a CatalogFile> {
        self.files.iter().filter(move |f| f.tag == tag)
    }

    /// Reads `<folder>/<file>` pairs into locales, parsed files and diagnostics.
    fn scan(raw: Vec<(String, String)>, layer: Layer) -> Catalogs {
        let mut folders: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
        for (path, text) in raw {
            let Some((folder, name)) = path.split_once('/') else {
                continue;
            };
            if folder.starts_with('_') || name.contains('/') {
                continue;
            }
            folders
                .entry(folder.to_string())
                .or_default()
                .push((name.to_string(), text));
        }

        let mut out = Catalogs::default();
        for (folder, mut files) in folders {
            if is_pseudo(&folder) {
                out.diags.push(LoadDiag::ReservedPseudo { folder });
                continue;
            }
            let Some(tag) = canonical_tag(&folder) else {
                out.diags.push(LoadDiag::NonCanonicalTag { folder });
                continue;
            };
            files.sort();
            let manifest = files
                .iter()
                .find(|(name, _)| name == "locale.toml")
                .map(|(_, text)| text.as_str());
            out.locales
                .push(locale_entry(&folder, tag, manifest, layer, &mut out.diags));
            for (name, text) in files {
                if !name.ends_with(".ftl") {
                    continue;
                }
                let path = format!("{folder}/{name}");
                let resource = parse(text, &path, &mut out.diags);
                out.files.push(CatalogFile {
                    tag: folder.clone(),
                    path,
                    layer,
                    resource: Arc::new(resource),
                });
            }
        }
        out
    }
}

/// Parses one FTL file, keeping the valid entries and reporting each Junk entry's line.
fn parse(text: String, path: &str, diags: &mut Vec<LoadDiag>) -> FluentResource {
    match FluentResource::try_new(text) {
        Ok(res) => res,
        Err((res, errors)) => {
            let src = res.source();
            let mut lines: Vec<u32> = errors
                .iter()
                .map(|e| first_non_blank_line(src, e.slice.clone().unwrap_or(e.pos.clone()).start))
                .collect();
            lines.dedup();
            for line in lines {
                diags.push(LoadDiag::Junk {
                    file: path.to_string(),
                    line,
                });
            }
            res
        }
    }
}

fn first_non_blank_line(src: &str, offset: usize) -> u32 {
    let offset = offset.min(src.len());
    let start = src[offset..]
        .find(|c: char| !c.is_whitespace())
        .map_or(offset, |i| offset + i);
    let newlines = src.as_bytes()[..start]
        .iter()
        .filter(|&&b| b == b'\n')
        .count();
    u32::try_from(newlines + 1).unwrap_or(u32::MAX)
}

/// `locale.toml`. Unknown keys (including `tag`: the folder name wins, I18N-02 edge 2) are
/// ignored at runtime; `xtask l10n check` validates the schema.
#[derive(Deserialize, Default)]
struct Manifest {
    native_name: Option<String>,
    english_name: Option<String>,
    short_name: Option<String>,
    direction: Option<ManifestDirection>,
    status: Option<ManifestStatus>,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum ManifestDirection {
    Ltr,
    Rtl,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum ManifestStatus {
    Stable,
    Beta,
    Experimental,
}

fn locale_entry(
    folder: &str,
    tag: LanguageIdentifier,
    manifest: Option<&str>,
    layer: Layer,
    diags: &mut Vec<LoadDiag>,
) -> CatalogLocale {
    let manifest = match manifest.map(toml::from_str::<Manifest>) {
        Some(Ok(m)) => Some(m),
        Some(Err(e)) => {
            diags.push(LoadDiag::BadManifest {
                tag: folder.to_string(),
                message: e.message().to_string(),
            });
            None
        }
        None => {
            diags.push(LoadDiag::MissingManifest {
                tag: folder.to_string(),
            });
            None
        }
    };
    let m = manifest.unwrap_or_default();
    CatalogLocale {
        max: maximize(&tag),
        native_name: m.native_name.unwrap_or_else(|| folder.to_string()),
        english_name: m.english_name.unwrap_or_else(|| folder.to_string()),
        short_name: m
            .short_name
            .unwrap_or_else(|| tag.language.as_str().to_ascii_uppercase()),
        direction: match m.direction {
            Some(ManifestDirection::Rtl) => Direction::Rtl,
            Some(ManifestDirection::Ltr) | None => Direction::Ltr,
        },
        status: match m.status {
            Some(ManifestStatus::Stable) => Status::Stable,
            Some(ManifestStatus::Beta) => Status::Beta,
            Some(ManifestStatus::Experimental) | None => Status::Experimental,
        },
        source: match layer {
            Layer::Embedded => CatalogSource::Embedded,
            Layer::Overlay => CatalogSource::Overlay,
        },
        tag,
    }
}

/// `folder` as a tag when it is canonical BCP 47 (`tr`, `pt-BR`, `zh-Hant`), else `None`.
fn canonical_tag(folder: &str) -> Option<LanguageIdentifier> {
    let mut locale = icu_locale::Locale::try_from_str(folder).ok()?;
    if !locale.extensions.is_empty() {
        return None;
    }
    icu_locale::LocaleCanonicalizer::new_extended().canonicalize(&mut locale);
    if locale.to_string() != folder {
        return None;
    }
    folder.parse().ok()
}

pub(crate) fn is_pseudo(tag: &str) -> bool {
    PSEUDO_TAGS.iter().any(|p| p.eq_ignore_ascii_case(tag))
}

/// `tag` with likely subtags (`tr` → `tr-Latn-TR`).
pub(crate) fn maximize(tag: &LanguageIdentifier) -> LanguageIdentifier {
    let Ok(mut id) = icu_locale::LanguageIdentifier::try_from_str(&tag.to_string()) else {
        return tag.clone();
    };
    icu_locale::LocaleExpander::new_extended().maximize(&mut id);
    id.to_string().parse().unwrap_or_else(|_| tag.clone())
}

/// Reads the overlay tree into `<folder>/<file>` pairs, enforcing the file-safety limits.
fn read_overlay(dir: &Path, diags: &mut Vec<LoadDiag>) -> Vec<(String, String)> {
    let mut skip = |path: &Path, reason: &str| {
        diags.push(LoadDiag::OverlaySkipped {
            path: path.to_path_buf(),
            reason: reason.to_string(),
        });
    };
    let root = match dir.canonicalize() {
        Ok(root) => root,
        Err(e) => {
            skip(dir, &format!("overlay directory unreadable: {e}"));
            return Vec::new();
        }
    };
    let Ok(entries) = std::fs::read_dir(&root) else {
        skip(dir, "overlay directory unreadable");
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut folders: Vec<_> = entries.filter_map(Result::ok).collect();
    folders.sort_by_key(std::fs::DirEntry::file_name);
    for folder in folders {
        let folder_name = folder.file_name().to_string_lossy().into_owned();
        let folder_path = folder.path();
        if folder_name.starts_with('_') || folder_name.starts_with('.') {
            continue;
        }
        let Some(real) = inside(&root, &folder_path) else {
            skip(&folder_path, "symlink leaves the overlay root");
            continue;
        };
        if !real.is_dir() {
            skip(&folder_path, "not a locale folder");
            continue;
        }
        let Ok(files) = std::fs::read_dir(&real) else {
            skip(&folder_path, "locale folder unreadable");
            continue;
        };
        let mut files: Vec<_> = files.filter_map(Result::ok).collect();
        files.sort_by_key(std::fs::DirEntry::file_name);
        for file in files {
            let name = file.file_name().to_string_lossy().into_owned();
            let path = file.path();
            if let Some(text) = read_overlay_file(&root, &path, &name, &mut skip) {
                out.push((format!("{folder_name}/{name}"), text));
            }
        }
    }
    out
}

fn read_overlay_file(
    root: &Path,
    path: &Path,
    name: &str,
    skip: &mut impl FnMut(&Path, &str),
) -> Option<String> {
    let Some(real) = inside(root, path) else {
        skip(path, "symlink leaves the overlay root");
        return None;
    };
    if real.is_dir() {
        skip(path, "nested folder");
        return None;
    }
    if !(name.ends_with(".ftl") || name == "locale.toml") {
        skip(path, "not a catalog file (*.ftl or locale.toml)");
        return None;
    }
    match std::fs::metadata(&real) {
        Ok(m) if m.len() > MAX_OVERLAY_FILE => {
            skip(path, "larger than 1 MB");
            return None;
        }
        Ok(_) => {}
        Err(e) => {
            skip(path, &format!("unreadable: {e}"));
            return None;
        }
    }
    let bytes = match std::fs::read(&real) {
        Ok(b) => b,
        Err(e) => {
            skip(path, &format!("unreadable: {e}"));
            return None;
        }
    };
    let text = bytes
        .strip_prefix(b"\xEF\xBB\xBF".as_slice())
        .unwrap_or(&bytes);
    match std::str::from_utf8(text) {
        Ok(text) => Some(text.to_string()),
        Err(_) => {
            skip(path, "not UTF-8");
            None
        }
    }
}

/// The real path of `path` when it stays under `root` (which is canonical).
fn inside(root: &Path, path: &Path) -> Option<PathBuf> {
    let real = path.canonicalize().ok()?;
    real.starts_with(root).then_some(real)
}
