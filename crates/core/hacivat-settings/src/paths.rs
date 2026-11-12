//! The §5.6 local state locations.

use std::io;
use std::path::{Path, PathBuf};

/// The four base directories; every file the app owns lives under one of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalPaths {
    pub config: PathBuf,
    pub data: PathBuf,
    pub cache: PathBuf,
    /// The runtime directory, where the platform has one (none on Windows, nor on macOS).
    pub runtime: Option<PathBuf>,
}

impl LocalPaths {
    /// The platform locations from `directories::ProjectDirs`.
    pub fn from_os() -> io::Result<Self> {
        let dirs =
            directories::ProjectDirs::from("dev", "hacivat", "Hacivat").ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "no home directory could be determined",
                )
            })?;
        Ok(Self {
            config: dirs.config_dir().to_path_buf(),
            data: dirs.data_dir().to_path_buf(),
            cache: dirs.cache_dir().to_path_buf(),
            runtime: dirs.runtime_dir().map(Path::to_path_buf),
        })
    }

    /// All locations under `root` (for tests).
    pub fn under(root: &Path) -> Self {
        Self {
            config: root.join("config"),
            data: root.join("data"),
            cache: root.join("cache"),
            runtime: Some(root.join("runtime")),
        }
    }

    /// Creates the base directories (and `<runtime>/hacivat`) with mode `0700` on Unix.
    pub fn ensure_dirs(&self) -> io::Result<()> {
        let runtime_app = self.runtime.as_ref().map(|r| r.join("hacivat"));
        for dir in [&self.config, &self.data, &self.cache]
            .into_iter()
            .chain(runtime_app.as_ref())
        {
            create_private_dir(dir)?;
        }
        Ok(())
    }

    pub fn settings_toml(&self) -> PathBuf {
        self.config.join("settings.toml")
    }
    pub fn connections_toml(&self) -> PathBuf {
        self.config.join("connections.toml")
    }
    pub fn keymaps_dir(&self) -> PathBuf {
        self.config.join("keymaps")
    }
    pub fn profile_toml(&self, uuid: &str) -> PathBuf {
        self.config.join("profiles").join(format!("{uuid}.toml"))
    }
    pub fn vault_bin(&self) -> PathBuf {
        self.data.join("vault.bin")
    }
    pub fn catalog_db(&self, uuid: &str, major: u32) -> PathBuf {
        self.data
            .join("catalog")
            .join(format!("{uuid}.v{major}.sqlite"))
    }
    pub fn history_db(&self) -> PathBuf {
        self.data.join("history.sqlite")
    }
    pub fn consoles_dir(&self, uuid: &str) -> PathBuf {
        self.data.join("consoles").join(uuid)
    }
    pub fn workspace_json(&self) -> PathBuf {
        self.data.join("workspace.json")
    }
    pub fn ai_broker_db(&self) -> PathBuf {
        self.data.join("ai").join("broker.sqlite")
    }
    pub fn ai_audit_db(&self) -> PathBuf {
        self.data.join("ai").join("audit.sqlite")
    }
    /// `\\.\pipe\hacivat-<user-sid>` on Windows; `<runtime>/hacivat/broker.sock` elsewhere
    /// (`<cache>/hacivat/broker.sock` when the platform has no runtime directory).
    pub fn ipc_endpoint(&self, user_sid: Option<&str>) -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(format!(r"\\.\pipe\hacivat-{}", user_sid.unwrap_or("user")))
        } else {
            let base = self.runtime.as_ref().unwrap_or(&self.cache);
            base.join("hacivat").join("broker.sock")
        }
    }
    pub fn logs_dir(&self) -> PathBuf {
        self.cache.join("logs")
    }
    pub fn locales_overlay_dir(&self) -> PathBuf {
        self.config.join("locales")
    }
}

fn create_private_dir(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(dir)
    }
}
