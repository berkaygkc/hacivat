//! `--check-update <manifest-url>`: minisign-verified self-update through `cargo-packager-updater`.

use base64::Engine as _;
use cargo_packager_updater::{Config, UpdaterBuilder, semver::Version, url::Url};

/// The packaged version (`HACIVAT_SPIKE_VERSION` is set by `packaging/package-macos.sh`).
pub const VERSION: &str = match option_env!("HACIVAT_SPIKE_VERSION") {
    Some(v) => v,
    None => env!("CARGO_PKG_VERSION"),
};

/// The minisign public key, as the updater wants it: the base64 of the `.pub` file's text.
fn pubkey() -> String {
    base64::engine::general_purpose::STANDARD.encode(include_str!("../../../packaging/keys/update.pub"))
}

/// Check the manifest; on a newer, correctly signed version install it and relaunch (the
/// process exits). Returns when there is no update. A signature or download failure exits 5.
pub fn check_and_install(manifest_url: &str) {
    match run(manifest_url) {
        Ok(()) => eprintln!("spike-harman: no update (running {VERSION})"),
        Err(e) => {
            eprintln!("spike-harman: update refused: {e}");
            std::process::exit(5);
        }
    }
}

fn run(manifest_url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let config = Config { endpoints: vec![manifest_url.parse::<Url>()?], pubkey: pubkey(), windows: None };
    let updater = UpdaterBuilder::new(Version::parse(VERSION)?, config).build()?;
    let Some(update) = updater.check()? else { return Ok(()) };
    eprintln!("spike-harman: update {} -> {} available", update.current_version, update.version);
    let exe = std::env::current_exe()?;
    // Download verifies the minisign signature before anything is installed.
    let bytes = update.download()?;
    eprintln!("spike-harman: signature verified ({} bytes)", bytes.len());
    update.install(bytes)?;
    eprintln!("spike-harman: installed {}; relaunching", update.version);
    std::process::Command::new(exe).spawn()?;
    std::process::exit(0);
}
