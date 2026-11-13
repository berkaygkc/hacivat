//! Booting the connections: the core runtime, the profile store, the secret store and the
//! drivers, prepared before GPUI starts (Task 12).

use std::sync::Arc;

use hacivat_driver::{DriverRegistry, EngineCatalog};
use hacivat_net::ssh::{KnownHosts, TunnelConfig, TunnelManager};
use hacivat_secrets::{
    CredentialFiles, FallbackBackend, FileVault, KeyringBackend, MemoryBackend, SecretBackend,
    SecretStore,
};
use hacivat_session::pipeline::{ApprovedRoots, ConnectDeps};
use hacivat_session::profile::{LoadedProfiles, ProfileStore};
use hacivat_session::{ConnectionManager, CoreRuntime, ManagerConfig, default_drivers};
use hacivat_settings::LocalPaths;
use hacivat_ui_core::gpui_surface::App;

use crate::connections::model::{ConnectionsModel, push_store_toasts};

/// Everything the connections need, built before the window opens (M2a, P17).
pub struct ConnectionsBoot {
    pub runtime: Arc<CoreRuntime>,
    pub store: ProfileStore,
    pub loaded: LoadedProfiles,
    pub secrets: Arc<SecretStore>,
    pub drivers: Arc<DriverRegistry>,
    /// The credential files connects read (`.pgpass`, `pg_service.conf`); none in tests.
    pub files: CredentialFiles,
    // ## m2b-t01-interfaces
    /// The known-host stores: Hacivat's `known_hosts` in the data folder and the user's
    /// OpenSSH files (read only): `~/.ssh/known_hosts{,2}` in `load`, files under the temp
    /// root in `for_tests`, so a test never reads the developer's `~/.ssh` (fix round 1, M6).
    /// Replaceable before boot. Loading does no I/O (Q42).
    pub known_hosts: Arc<KnownHosts>,
}

impl ConnectionsBoot {
    /// The app's boot: `connections.toml` under `paths`, the OS keyring (`MemoryBackend` when
    /// `HACIVAT_SECRETS=memory`), the default drivers and the credential files the environment
    /// names. The keyring is not touched here.
    pub fn load(paths: &LocalPaths) -> ConnectionsBoot {
        let choice = std::env::var("HACIVAT_SECRETS").ok();
        let backend = secret_backend_from(choice.as_deref());
        // ## m2b-t22-live: on Linux the keyring falls back to the vault (B13, Q23).
        #[cfg(target_os = "linux")]
        let backend = if backend.name() == "keyring" {
            vault_fallback(backend, paths, Box::new(KeyringBackend::available))
        } else {
            backend
        };
        boot_with(
            paths,
            default_drivers(),
            backend,
            CredentialFiles::from_env(),
            openssh_known_hosts(),
        )
    }

    /// A boot for tests: a memory secret backend, `drivers` and no credential files.
    pub fn for_tests(paths: &LocalPaths, drivers: DriverRegistry) -> ConnectionsBoot {
        boot_with(
            paths,
            drivers,
            Arc::new(MemoryBackend::new()),
            CredentialFiles {
                pgpass: None,
                pg_service: None,
            },
            vec![paths.config.join("ssh").join("known_hosts")],
        )
    }
}

/// The secret backend for `HACIVAT_SECRETS`: `memory` gives a `MemoryBackend` (dev and test
/// runs that must not touch the keychain); anything else the OS keyring.
pub fn secret_backend_from(choice: Option<&str>) -> Arc<dyn SecretBackend> {
    match choice {
        Some(c) if c.eq_ignore_ascii_case("memory") => Arc::new(MemoryBackend::new()),
        _ => Arc::new(KeyringBackend::new(None)),
    }
}

// ## m2b-t22-live
/// Linux without a usable system keyring (B13, Q23): `keyring` behind a `FallbackBackend`
/// whose other half is the encrypted vault at `<data>/vault.bin`. Nothing is probed or read
/// here: `probe` (a Secret Service reachability check, never prompting, P36) runs once, on the
/// core runtime, the first time the store is used, and the vault file is read only when it is
/// unlocked. Built on every platform so it is compiled and tested everywhere; only Linux boots
/// with it.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn vault_fallback(
    keyring: Arc<dyn SecretBackend>,
    paths: &LocalPaths,
    probe: Box<dyn Fn() -> bool + Send + Sync>,
) -> Arc<dyn SecretBackend> {
    let vault = Arc::new(FileVault::open(paths.data.join("vault.bin")));
    Arc::new(FallbackBackend::new(keyring, vault, probe))
}

fn boot_with(
    paths: &LocalPaths,
    drivers: DriverRegistry,
    backend: Arc<dyn SecretBackend>,
    files: CredentialFiles,
    openssh_known_hosts: Vec<std::path::PathBuf>,
) -> ConnectionsBoot {
    let runtime = CoreRuntime::new().expect("the core runtime starts");
    let (store, loaded) = ProfileStore::open(paths.connections_toml());
    ConnectionsBoot {
        runtime: Arc::new(runtime),
        store,
        loaded,
        secrets: Arc::new(SecretStore::new(Some(backend))),
        drivers: Arc::new(drivers),
        files,
        known_hosts: KnownHosts::load(openssh_known_hosts, paths.data.join("known_hosts")),
    }
}

// ## m2b-t01-interfaces
/// The user's OpenSSH `known_hosts` files (`~/.ssh/known_hosts`, `~/.ssh/known_hosts2`), read
/// only; none without a home directory. Paths only: nothing is read here.
fn openssh_known_hosts() -> Vec<std::path::PathBuf> {
    std::env::home_dir()
        .map(|home| {
            let ssh = home.join(".ssh");
            vec![ssh.join("known_hosts"), ssh.join("known_hosts2")]
        })
        .unwrap_or_default()
}

/// Sets the `ConnectionsModel` Global from `boot` (with its `ConnectionManager`) and shows the
/// store diagnostics as toasts, one per kind (P29): the quarantined file; a file that cannot
/// be read or set aside; the skipped entries (invalid, duplicate ID, unknown engine) as one
/// count; the ignored values as one count; an ignored password (never echoed); a newer file
/// (read-only). It never touches the keyring. A pending profile write is flushed
/// at quit.
pub fn install(cx: &mut App, boot: ConnectionsBoot) {
    let ConnectionsBoot {
        runtime,
        store,
        loaded,
        secrets,
        drivers,
        files,
        known_hosts,
    } = boot;
    // ## m2b-t01-interfaces: the SSH chains, shared by every connect (Q23).
    let tunnels = TunnelManager::new(known_hosts.clone(), TunnelConfig::default());
    // ## m2a-t17-dialog: the dialog's file pickers approve into the roots connects check.
    let approved_roots = ApprovedRoots::for_user();
    cx.set_global(crate::connections::dialog::DialogEnv {
        approved_roots: approved_roots.clone(),
        pgpass: files.pgpass.clone(),
    });
    let deps = ConnectDeps {
        drivers,
        catalog: EngineCatalog::embedded(),
        secrets: secrets.clone(),
        files,
        transport_wrap: None,
        extra_steps: Vec::new(),
        // Home plus paths the user picks in the connection dialog (T17 adds them).
        approved_roots,
        timeout_override: None,
        tunnels: tunnels.clone(),
        known_hosts: known_hosts.clone(),
    };
    // ## m2b-t22-live: B18 liveness presets and B19 reconnects (T17; `default()` keeps M2a's).
    let manager = ConnectionManager::new(runtime.handle(), Arc::new(deps), ManagerConfig::m2b());
    let LoadedProfiles {
        profiles,
        diags,
        quarantined,
    } = loaded;
    cx.set_global(ConnectionsModel::new(
        runtime,
        store,
        profiles,
        secrets,
        manager,
        known_hosts,
        tunnels,
    ));
    cx.on_app_quit(|cx| {
        if let Some(m) = cx.try_global::<ConnectionsModel>() {
            m.flush_writes();
        }
        async {}
    })
    .detach();

    push_store_toasts(cx, &diags, quarantined.as_deref());
}

// ## m2b-t22-live
#[cfg(test)]
mod tests {
    use super::*;
    use hacivat_secrets::BackendChoice;

    #[test]
    fn the_linux_fallback_keeps_the_vault_in_the_data_folder_and_probes_lazily() {
        let root = tempfile::tempdir().expect("temp dir");
        let paths = LocalPaths::under(root.path());
        let probed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let seen = probed.clone();
        let backend = vault_fallback(
            Arc::new(MemoryBackend::new()),
            &paths,
            Box::new(move || {
                seen.store(true, std::sync::atomic::Ordering::SeqCst);
                false
            }),
        );
        // Boot never probes (P36): the choice is made at the first use.
        assert!(!probed.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(backend.choice(), None);
        assert!(backend.probe_available());
        assert!(probed.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(backend.choice(), Some(BackendChoice::Vault));
        let vault = backend.as_vault().expect("the vault backs Forever");
        assert!(!vault.exists(), "nothing is created before the user asks");
        assert!(!paths.data.join("vault.bin").exists());
    }
}
