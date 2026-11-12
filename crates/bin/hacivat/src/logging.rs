//! Logs: JSON lines in `<cache>/logs/hacivat.<date>.log`, rotated daily, the 7 newest kept.
//! The level comes from `HACIVAT_LOG` (an `EnvFilter` directive), `info` by default. Log lines
//! are machine text: English, no localization.

use std::io;
use std::path::Path;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::EnvFilter;

/// How many daily files are kept.
pub const KEEP_FILES: usize = 7;
/// The file name prefix (`hacivat.2026-11-12.log`).
const PREFIX: &str = "hacivat";
const SUFFIX: &str = "log";

/// The rolling appender writing into `dir`, which is created `0700` (Unix).
pub fn appender(dir: &Path) -> io::Result<RollingFileAppender> {
    hacivat_settings::fsio::create_dir_all_private(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix(PREFIX)
        .filename_suffix(SUFFIX)
        .max_log_files(KEEP_FILES)
        .build(dir)
        .map_err(io::Error::other)?;
    restrict_files(dir);
    Ok(appender)
}

/// Makes the log files in `dir` owner-only (`0600`); the appender creates them with the
/// default mode.
fn restrict_files(dir: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
            }
        }
    }
    #[cfg(not(unix))]
    let _ = dir;
}

/// Installs the global subscriber. Keep the guard alive for the whole run: dropping it flushes
/// and stops the writer thread. `None` when the directory cannot be used; the app then runs
/// without file logs.
pub fn init(dir: &Path) -> Option<WorkerGuard> {
    let appender = match appender(dir) {
        Ok(a) => a,
        Err(err) => {
            eprintln!("logging disabled: {}: {err}", dir.display());
            return None;
        }
    };
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let filter = EnvFilter::try_from_env("HACIVAT_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(filter)
        .with_writer(writer)
        .with_ansi(false)
        .try_init()
        .ok()?;
    Some(guard)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    #[test]
    fn appender_writes_dated_files_in_a_private_directory() {
        let root =
            std::env::temp_dir().join(format!("hacivat-logging-test-{}", std::process::id()));
        let dir = root.join("logs");
        let mut appender = appender(&dir).unwrap();
        writeln!(appender, "{{\"msg\":\"x\"}}").unwrap();
        appender.flush().unwrap();
        let names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names.len(), 1, "{names:?}");
        assert!(
            names[0].starts_with("hacivat.") && names[0].ends_with(".log"),
            "{names:?}"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&dir), 0o700);
            assert_eq!(mode(&dir.join(&names[0])), 0o600);
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}
