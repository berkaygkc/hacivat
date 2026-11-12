//! Logs: JSON lines in `<cache>/logs/hacivat.<YYYY-MM-DD>.log` (UTC date), a new file each day,
//! the 7 newest kept. The directory is `0700` and every file is created `0600` on Unix. The
//! level comes from `HACIVAT_LOG` (an `EnvFilter` directive), `info` by default. Log lines are
//! machine text: English, no localization.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;

/// How many daily files are kept.
pub const KEEP_FILES: usize = 7;
const PREFIX: &str = "hacivat.";
const SUFFIX: &str = ".log";

/// The current day, as days since the Unix epoch (UTC).
pub type Clock = fn() -> i64;

fn today() -> i64 {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    i64::try_from(secs / 86_400).unwrap_or(0)
}

/// `YYYY-MM-DD` of `days` since 1970-01-01 (proleptic Gregorian, H. Hinnant's algorithm).
fn civil_date(days: i64) -> String {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// A writer that appends to the current day's file, opening a new owner-only file when the day
/// changes and deleting all but the [`KEEP_FILES`] newest.
pub struct DailyLog {
    dir: PathBuf,
    clock: Clock,
    current: Option<(i64, File)>,
}

impl DailyLog {
    /// A log in `dir`, which is created `0700` (Unix).
    pub fn new(dir: &Path) -> io::Result<DailyLog> {
        DailyLog::with_clock(dir, today)
    }

    pub fn with_clock(dir: &Path, clock: Clock) -> io::Result<DailyLog> {
        hacivat_settings::fsio::create_dir_all_private(dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
        let mut log = DailyLog {
            dir: dir.to_path_buf(),
            clock,
            current: None,
        };
        log.file()?;
        Ok(log)
    }

    /// The file of `day`.
    pub fn path_of(&self, day: i64) -> PathBuf {
        self.dir
            .join(format!("{PREFIX}{}{SUFFIX}", civil_date(day)))
    }

    /// The current day's file, opened (and older files pruned) when the day changed.
    fn file(&mut self) -> io::Result<&mut File> {
        let day = (self.clock)();
        if self.current.as_ref().is_none_or(|(d, _)| *d != day) {
            let mut options = OpenOptions::new();
            options.create(true).append(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let file = options.open(self.path_of(day))?;
            self.current = Some((day, file));
            self.prune();
        }
        match &mut self.current {
            Some((_, file)) => Ok(file),
            None => Err(io::Error::other("no log file")),
        }
    }

    /// Deletes all but the newest [`KEEP_FILES`] log files (names sort by date).
    fn prune(&self) {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return;
        };
        let mut logs: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(PREFIX) && n.ends_with(SUFFIX))
            })
            .collect();
        logs.sort();
        let excess = logs.len().saturating_sub(KEEP_FILES);
        for old in &logs[..excess] {
            let _ = std::fs::remove_file(old);
        }
    }
}

impl Write for DailyLog {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.file()?.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        match &mut self.current {
            Some((_, file)) => file.flush(),
            None => Ok(()),
        }
    }
}

/// Installs the global subscriber. Keep the guard alive for the whole run and drop it before
/// exiting: dropping it flushes and stops the writer thread. `None` when the directory cannot be
/// used; the app then runs without file logs.
pub fn init(dir: &Path) -> Option<WorkerGuard> {
    let log = match DailyLog::new(dir) {
        Ok(log) => log,
        Err(err) => {
            eprintln!("logging disabled: {}: {err}", dir.display());
            return None;
        }
    };
    let (writer, guard) = tracing_appender::non_blocking(log);
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
    use std::sync::atomic::{AtomicI64, Ordering};

    static DAY: AtomicI64 = AtomicI64::new(0);

    fn test_clock() -> i64 {
        DAY.load(Ordering::SeqCst)
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hacivat-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn dates_are_civil_utc() {
        assert_eq!(civil_date(0), "1970-01-01");
        assert_eq!(civil_date(20_769), "2026-11-12");
        assert_eq!(civil_date(11_016), "2000-02-29");
    }

    #[test]
    fn every_day_gets_an_owner_only_file_and_seven_are_kept() {
        let root = scratch("logging");
        let dir = root.join("logs");
        DAY.store(20_760, Ordering::SeqCst);
        let mut log = DailyLog::with_clock(&dir, test_clock).unwrap();
        for day in 20_760..20_770 {
            DAY.store(day, Ordering::SeqCst);
            writeln!(log, "{{\"day\":{day}}}").unwrap();
        }
        log.flush().unwrap();
        let names = names(&dir);
        assert_eq!(names.len(), KEEP_FILES, "{names:?}");
        assert_eq!(
            names.first().map(String::as_str),
            Some("hacivat.2026-11-06.log")
        );
        assert_eq!(
            names.last().map(String::as_str),
            Some("hacivat.2026-11-12.log")
        );
        let today = std::fs::read_to_string(dir.join("hacivat.2026-11-12.log")).unwrap();
        assert_eq!(today, "{\"day\":20769}\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&dir), 0o700);
            for name in &names {
                assert_eq!(mode(&dir.join(name)), 0o600, "{name}");
            }
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}
