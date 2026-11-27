//! The `include_bytes!` table of `assets/system-catalogs/` (feature `embedded-system`; only
//! `hacivat-app` enables it, `EmbeddedSystemOnlyFromApp`): the manifest, every snapshot file
//! it lists and the shipped virtual foreign keys, as committed. Nothing here is decoded until
//! [`super::SystemCatalogs::for_server`] asks for a line (M4 Task 26).
//!
//! When the committed set changes (a new line), add its row here; the test
//! `embedded_snapshots_decode_lazily_once_per_version` fails until the table lists exactly
//! the manifest's files.

macro_rules! asset_path {
    ($file:literal) => {
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../assets/system-catalogs/",
            $file
        )
    };
}

macro_rules! snapshot {
    ($file:literal) => {
        ($file, include_bytes!(asset_path!($file)) as &[u8])
    };
}

/// The committed `manifest.toml`.
pub static MANIFEST: &str = include_str!(asset_path!("manifest.toml"));

/// The committed `virtual-fks.toml`.
pub static VIRTUAL_FKS: &str = include_str!(asset_path!("virtual-fks.toml"));

/// The shipped snapshot files: `(file name, bytes)`, one per `manifest.toml` entry.
pub static SNAPSHOTS: &[(&str, &[u8])] = &[
    snapshot!("mariadb-10.11.hvsnap.zst"),
    snapshot!("mariadb-10.6.hvsnap.zst"),
    snapshot!("mariadb-11.4.hvsnap.zst"),
    snapshot!("mysql-8.0.hvsnap.zst"),
    snapshot!("mysql-8.4.hvsnap.zst"),
    snapshot!("postgres-14.hvsnap.zst"),
    snapshot!("postgres-15.hvsnap.zst"),
    snapshot!("postgres-16.hvsnap.zst"),
    snapshot!("postgres-17.hvsnap.zst"),
    snapshot!("postgres-18.hvsnap.zst"),
];
