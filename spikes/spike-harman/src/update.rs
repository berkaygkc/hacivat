//! `--check-update <manifest-url>`: minisign-verified self-update through `cargo-packager-updater`.

use base64::Engine as _;
use minisign_verify::Signature;
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
    let manifest: Url = manifest_url.parse()?;
    check_manifest_url(&manifest)?;
    let config = Config { endpoints: vec![manifest], pubkey: pubkey(), windows: None };
    let updater = UpdaterBuilder::new(Version::parse(VERSION)?, config).build()?;
    let Some(update) = updater.check()? else { return Ok(()) };
    eprintln!("spike-harman: update {} -> {} available", update.current_version, update.version);
    let exe = std::env::current_exe()?;
    // Download verifies the minisign signature before anything is installed.
    let bytes = update.download()?;
    eprintln!("spike-harman: signature verified ({} bytes)", bytes.len());
    // The trusted comment is covered by minisign's global signature, which `download()` just
    // verified, so the version in it is bound to the artifact, unlike the manifest's claim.
    let sig_text = String::from_utf8(base64::engine::general_purpose::STANDARD.decode(&update.signature)?)?;
    let comment = Signature::decode(&sig_text)?.trusted_comment().to_string();
    check_version_binding(VERSION, &update.version, &comment)?;
    update.install(bytes)?;
    eprintln!("spike-harman: installed {}; relaunching", update.version);
    std::process::Command::new(exe).spawn()?;
    std::process::exit(0);
}

/// Plain http is only allowed for loopback (local round-trip testing).
fn check_manifest_url(url: &Url) -> Result<(), String> {
    match (url.scheme(), url.host_str()) {
        ("https", _) => Ok(()),
        ("http", Some("127.0.0.1" | "localhost" | "[::1]")) => Ok(()),
        _ => Err(format!("manifest URL must be https (or loopback http): {url}")),
    }
}

/// The signed artifact must be what the manifest claims and newer than what runs now.
/// `trusted_comment` is the verified minisign trusted comment, `hacivat-spike <version>`.
fn check_version_binding(current: &str, claimed: &str, trusted_comment: &str) -> Result<(), String> {
    let artifact = trusted_comment
        .strip_prefix("hacivat-spike ")
        .ok_or_else(|| format!("artifact signature carries no version ({trusted_comment:?})"))?;
    let parse = |v: &str| Version::parse(v).map_err(|e| format!("bad version {v:?}: {e}"));
    let (artifact_v, claimed_v, current_v) = (parse(artifact)?, parse(claimed)?, parse(current)?);
    if artifact_v != claimed_v {
        return Err(format!("manifest claims {claimed_v} but the signed artifact is {artifact_v}"));
    }
    if artifact_v <= current_v {
        return Err(format!("artifact {artifact_v} is not newer than the running {current_v}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_matching_artifact_is_accepted() {
        assert!(check_version_binding("0.0.1", "0.0.2", "hacivat-spike 0.0.2").is_ok());
        assert!(check_version_binding("0.0.1", "0.1.0", "hacivat-spike 0.1.0").is_ok());
    }

    #[test]
    fn claimed_higher_but_artifact_older_is_refused() {
        let e = check_version_binding("0.0.2", "0.0.3", "hacivat-spike 0.0.1").unwrap_err();
        assert!(e.contains("manifest claims 0.0.3"), "{e}");
    }

    #[test]
    fn same_or_older_artifact_is_refused() {
        assert!(check_version_binding("0.0.2", "0.0.2", "hacivat-spike 0.0.2").is_err());
        assert!(check_version_binding("0.0.2", "0.0.1", "hacivat-spike 0.0.1").is_err());
    }

    #[test]
    fn unversioned_signature_is_refused() {
        assert!(check_version_binding("0.0.1", "0.0.2", "timestamp:1").is_err());
    }

    #[test]
    fn non_loopback_http_is_refused() {
        let u = |s: &str| s.parse::<Url>().unwrap();
        assert!(check_manifest_url(&u("https://example.com/m.json")).is_ok());
        assert!(check_manifest_url(&u("http://127.0.0.1:8000/m.json")).is_ok());
        assert!(check_manifest_url(&u("http://example.com/m.json")).is_err());
        assert!(check_manifest_url(&u("http://127.0.0.1.evil.com/m.json")).is_err());
    }
}
