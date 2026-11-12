//! Catalog discovery and loading (I18N-02). Bodies in Task 6.

use std::path::{Path, PathBuf};

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
    NonCanonicalTag { folder: String },
    MissingManifest { tag: String },
    Junk { file: String, line: u32 },
    ReservedPseudo { folder: String },
    OverlaySkipped { path: PathBuf, reason: String },
}

/// The embedded catalogs plus an optional on-disk overlay. (No `PartialEq`: Task 6 adds the
/// parsed resources, which do not implement it.)
#[derive(Debug, Clone, Default)]
pub struct Catalogs {
    locales: Vec<CatalogLocale>,
    diags: Vec<LoadDiag>,
}

impl Catalogs {
    /// The catalogs embedded from `locales/` (skipping `_`-prefixed folders). Stub: empty.
    pub fn embedded() -> Catalogs {
        Catalogs::default()
    }

    /// Layers the overlay directory `dir` over `self`, per message ID. Stub: `self` unchanged.
    pub fn with_overlay(self, dir: &Path) -> Catalogs {
        let _ = dir;
        self
    }

    /// Catalogs from `(path, contents)` pairs such as `("en/hui.ftl", "...")`. Stub: empty.
    pub fn from_memory(files: &[(&str, &str)]) -> Catalogs {
        let _ = files;
        Catalogs::default()
    }

    pub fn locales(&self) -> &[CatalogLocale] {
        &self.locales
    }

    pub fn diags(&self) -> &[LoadDiag] {
        &self.diags
    }
}
