//! `scripts/dev-run.sh`, the macOS cargo runner (R7, P35), driven with stand-in `security` and
//! `codesign` executables on `PATH`, so no real identity or keychain is touched.
#![cfg(target_os = "macos")]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

fn script() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../scripts/dev-run.sh")
}

fn write_exe(path: &Path, body: &str) {
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
    std::fs::write(path, format!("#!/bin/sh\n{body}\n")).expect("write");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
}

struct Run {
    stdout: String,
    stderr: String,
    signed: bool,
}

/// Runs `dev-run.sh <dir>/<build>/app arg` with stand-ins: `security` lists the identity when
/// `identity`, `codesign` exits `codesign_status` and leaves a marker; `ci` sets `CI=true`.
fn run(build: &str, identity: bool, codesign_status: i32, ci: bool) -> Run {
    let dir = tempfile::tempdir().expect("tempdir");
    let fake = dir.path().join("fake-bin");
    let marker = dir.path().join("codesign-ran");
    let listing = if identity {
        r#"echo '  1) 0000000000000000000000000000000000000000 "Hacivat Dev"'"#
    } else {
        "exit 0"
    };
    write_exe(&fake.join("security"), listing);
    write_exe(
        &fake.join("codesign"),
        &format!("touch '{}'\nexit {codesign_status}", marker.display()),
    );
    let app = dir.path().join("target").join(build).join("app");
    write_exe(&app, r#"echo "ran $1""#);

    let path = format!("{}:/usr/bin:/bin", fake.display());
    let mut cmd = Command::new("/bin/sh");
    cmd.arg(script()).arg(&app).arg("x").env("PATH", path);
    if ci {
        cmd.env("CI", "true");
    } else {
        cmd.env_remove("CI");
    }
    let out = cmd.output().expect("run dev-run.sh");
    assert!(out.status.success(), "dev-run.sh never fails");
    Run {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        signed: marker.exists(),
    }
}

#[test]
fn signs_a_debug_binary_quietly() {
    let r = run("debug", true, 0, false);
    assert_eq!(r.stdout, "ran x\n");
    assert!(r.signed);
    assert_eq!(r.stderr, "");
}

#[test]
fn never_signs_a_release_binary() {
    let r = run("release", true, 0, false);
    assert_eq!(r.stdout, "ran x\n");
    assert!(!r.signed, "a release binary must not get the dev identity");
    assert_eq!(r.stderr, "");
}

#[test]
fn a_missing_identity_hints_locally_and_is_silent_on_ci() {
    let local = run("debug", false, 0, false);
    assert_eq!(local.stdout, "ran x\n");
    assert!(!local.signed);
    assert_eq!(local.stderr.lines().count(), 1);
    assert!(local.stderr.contains("no \"Hacivat Dev\""));

    let ci = run("debug", false, 0, true);
    assert_eq!(ci.stdout, "ran x\n");
    assert_eq!(ci.stderr, "");
}

#[test]
fn a_codesign_failure_hints_even_on_ci() {
    let r = run("debug", true, 1, true);
    assert_eq!(r.stdout, "ran x\n");
    assert!(r.signed, "codesign was tried");
    assert_eq!(r.stderr.lines().count(), 1);
    assert!(r.stderr.contains("codesign with \"Hacivat Dev\" failed"));
}
