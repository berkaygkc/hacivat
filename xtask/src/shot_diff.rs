//! `xtask shot-diff`: compares the screenshot harness's PNGs (`target/shots`) against a baseline
//! directory, pixel by pixel with CIEDE2000.
//!
//! ```text
//! cargo xtask shot-diff --baseline <dir> --candidate <dir> [--max-de 2.0] [--max-share 0.001]
//!                        [--superseded <file>]
//! ```
//!
//! Every `*.png` in the baseline is compared with the same-named file in the candidate
//! directory. A pixel differs when ΔE2000 between the two exceeds `--max-de`; a shot breaches
//! when the share of differing pixels exceeds `--max-share`, when its candidate is missing, or
//! when the two sizes differ. Candidate PNGs without a baseline are listed as new and do not
//! breach. Exit 0 when nothing breaches, 1 on a breach, 2 on an error (bad flags, unreadable
//! image, empty baseline).
//!
//! ## m3b-ci (controller fix): `--superseded <file>` names baselines that a later milestone
//! changed on purpose and the PO has not re-baselined yet (one name per line, `#` comments; a
//! name may end in `*` to match every baseline it starts). Those are listed as superseded and not
//! compared; every other baseline is compared as before. A line that matches no baseline is an
//! error, so the list cannot outlive what it excuses. The nightly `pseudo-screens` job compares
//! the committed M1 baselines this way (`design/baselines/m1/macos/superseded.txt`).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use image::RgbaImage;

use crate::vf::{delta_e2000, srgb_to_lab};

/// Default per-pixel threshold: ΔE2000 above this counts as a changed pixel.
pub const DEFAULT_MAX_DE: f64 = 2.0;
/// Default share of changed pixels a shot may have before it breaches.
pub const DEFAULT_MAX_SHARE: f64 = 0.001;

/// Share of pixels whose ΔE2000 against the other image exceeds `max_de` (0.0 to 1.0). Images
/// of different sizes differ everywhere (1.0). Alpha is ignored: the harness renders opaque
/// frames.
pub fn diff_share(a: &RgbaImage, b: &RgbaImage, max_de: f64) -> f64 {
    if a.dimensions() != b.dimensions() {
        return 1.0;
    }
    let total = u64::from(a.width()) * u64::from(a.height());
    if total == 0 {
        return 0.0;
    }
    let changed = a
        .pixels()
        .zip(b.pixels())
        .filter(|(pa, pb)| {
            // Identical bytes never differ; only compute ΔE for pixels that changed at all.
            pa.0[..3] != pb.0[..3]
                && delta_e2000(
                    srgb_to_lab([pa[0], pa[1], pa[2]]),
                    srgb_to_lab([pb[0], pb[1], pb[2]]),
                ) > max_de
        })
        .count() as u64;
    changed as f64 / total as f64
}

struct Args {
    baseline: PathBuf,
    candidate: PathBuf,
    max_de: f64,
    max_share: f64,
    // ## m3b-ci (controller fix)
    superseded: Option<PathBuf>,
}

fn parse_args(args: &[String]) -> Result<Args> {
    let (mut baseline, mut candidate, mut superseded) = (None, None, None);
    let (mut max_de, mut max_share) = (DEFAULT_MAX_DE, DEFAULT_MAX_SHARE);
    let mut it = args.iter();
    while let Some(k) = it.next() {
        let mut val = || {
            it.next()
                .cloned()
                .with_context(|| format!("{k} needs a value"))
        };
        match k.as_str() {
            "--baseline" => baseline = Some(PathBuf::from(val()?)),
            "--candidate" => candidate = Some(PathBuf::from(val()?)),
            "--max-de" => max_de = val()?.parse().context("--max-de")?,
            "--max-share" => max_share = val()?.parse().context("--max-share")?,
            "--superseded" => superseded = Some(PathBuf::from(val()?)),
            other => bail!("unknown flag {other}"),
        }
    }
    if !(max_de >= 0.0 && (0.0..=1.0).contains(&max_share)) {
        bail!("--max-de must be >= 0 and --max-share within [0, 1]");
    }
    Ok(Args {
        baseline: baseline.context("--baseline <dir> required")?,
        candidate: candidate.context("--candidate <dir> required")?,
        max_de,
        max_share,
        superseded,
    })
}

// ## m3b-ci (controller fix)
/// The patterns of a `--superseded` list: one per line, blank lines and `#` comments skipped.
pub fn superseded_patterns(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(String::from)
        .collect()
}

/// Whether baseline `name` matches `pattern` (the whole name, or a prefix before a final `*`).
fn matches(pattern: &str, name: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => name.starts_with(prefix),
        None => name == pattern,
    }
}

/// The `*.png` file names directly inside `dir`, sorted.
fn pngs(dir: &Path) -> Result<BTreeSet<String>> {
    let mut out = BTreeSet::new();
    for entry in std::fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
        let name = entry?.file_name().to_string_lossy().into_owned();
        if name.ends_with(".png") {
            out.insert(name);
        }
    }
    Ok(out)
}

fn load(p: &Path) -> Result<RgbaImage> {
    Ok(image::open(p)
        .with_context(|| format!("open {}", p.display()))?
        .to_rgba8())
}

/// What one comparison found.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Changed share within the limit.
    Ok(f64),
    /// Changed share above the limit.
    Changed(f64),
    /// The candidate has no file of this name.
    Missing,
    /// Baseline and candidate sizes differ.
    Size((u32, u32), (u32, u32)),
    // ## m3b-ci (controller fix)
    /// Named by the `--superseded` list: changed on purpose since the baseline, not compared.
    Superseded,
}

impl Outcome {
    pub fn breaches(&self) -> bool {
        !matches!(self, Outcome::Ok(_) | Outcome::Superseded)
    }
}

/// Per-file outcomes (by file name), and the candidate files that have no baseline.
pub type Comparison = (Vec<(String, Outcome)>, Vec<String>);

/// Compares every baseline PNG with its candidate namesake.
/// ## m3b-ci (controller fix): except the baselines `superseded` matches, reported as
/// [`Outcome::Superseded`] instead of compared. A pattern that matches no baseline is an error.
pub fn compare_dirs(
    baseline: &Path,
    candidate: &Path,
    max_de: f64,
    max_share: f64,
    superseded: &[String],
) -> Result<Comparison> {
    let base = pngs(baseline)?;
    if base.is_empty() {
        bail!("no PNG in the baseline {}", baseline.display());
    }
    if let Some(stale) = superseded
        .iter()
        .find(|p| !base.iter().any(|name| matches(p, name)))
    {
        bail!(
            "--superseded names {stale:?}, which matches no baseline in {}",
            baseline.display()
        );
    }
    let cand = pngs(candidate)?;
    let mut out = Vec::new();
    for name in &base {
        if superseded.iter().any(|p| matches(p, name)) {
            out.push((name.clone(), Outcome::Superseded));
            continue;
        }
        if !cand.contains(name) {
            out.push((name.clone(), Outcome::Missing));
            continue;
        }
        let (b, c) = (load(&baseline.join(name))?, load(&candidate.join(name))?);
        let outcome = if b.dimensions() != c.dimensions() {
            Outcome::Size(b.dimensions(), c.dimensions())
        } else {
            let share = diff_share(&b, &c, max_de);
            if share > max_share {
                Outcome::Changed(share)
            } else {
                Outcome::Ok(share)
            }
        };
        out.push((name.clone(), outcome));
    }
    let new = cand.difference(&base).cloned().collect();
    Ok((out, new))
}

pub fn run() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(2).collect();
    match run_inner(&args) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            eprintln!("shot-diff: {e:#}");
            ExitCode::from(2)
        }
    }
}

fn run_inner(args: &[String]) -> Result<bool> {
    let a = parse_args(args)?;
    // ## m3b-ci (controller fix)
    let superseded = match &a.superseded {
        Some(p) => superseded_patterns(
            &std::fs::read_to_string(p).with_context(|| format!("read {}", p.display()))?,
        ),
        None => Vec::new(),
    };
    let (results, new) = compare_dirs(
        &a.baseline,
        &a.candidate,
        a.max_de,
        a.max_share,
        &superseded,
    )?;
    let mut breaches = 0;
    let mut skipped = 0;
    for (name, outcome) in &results {
        let line = match outcome {
            Outcome::Ok(s) => format!("ok      {name}  changed {:.4}%", s * 100.0),
            Outcome::Changed(s) => format!(
                "FAIL    {name}  changed {:.4}% > {:.4}%",
                s * 100.0,
                a.max_share * 100.0
            ),
            Outcome::Missing => format!("FAIL    {name}  missing from the candidate"),
            Outcome::Size(b, c) => format!(
                "FAIL    {name}  size {}x{} vs baseline {}x{}",
                c.0, c.1, b.0, b.1
            ),
            Outcome::Superseded => {
                skipped += 1;
                format!("skip    {name}  superseded (changed on purpose; not compared)")
            }
        };
        println!("{line}");
        if outcome.breaches() {
            breaches += 1;
        }
    }
    for name in &new {
        println!("new     {name}  (no baseline)");
    }
    println!(
        "shot-diff: {} compared, {breaches} breached, {skipped} superseded, {} new \
         (max dE {}, max share {})",
        results.len() - skipped,
        new.len(),
        a.max_de,
        a.max_share
    );
    Ok(breaches == 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn rgba(hex: u32) -> Rgba<u8> {
        Rgba([(hex >> 16) as u8, (hex >> 8) as u8, hex as u8, 255])
    }

    /// A 16x16 image of one colour.
    fn img(hex: u32) -> RgbaImage {
        RgbaImage::from_pixel(16, 16, rgba(hex))
    }

    /// A 16x16 image: the left half `left`, the right half `right`.
    fn half(left: u32, right: u32) -> RgbaImage {
        RgbaImage::from_fn(16, 16, |x, _| if x < 8 { rgba(left) } else { rgba(right) })
    }

    #[test]
    fn shot_diff_identical_is_zero() {
        assert_eq!(diff_share(&img(0x16181D), &img(0x16181D), 2.0), 0.0);
    }

    #[test]
    fn shot_diff_counts_changed_pixels() {
        assert!((diff_share(&img(0x16181D), &half(0x16181D, 0xCE2C31), 2.0) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn different_sizes_differ_everywhere() {
        assert_eq!(diff_share(&img(0x16181D), &RgbaImage::new(8, 8), 2.0), 1.0);
    }

    #[test]
    fn a_change_below_the_threshold_does_not_count() {
        // One step in blue on graphite is far below dE 2.
        assert_eq!(diff_share(&img(0x16181D), &img(0x16181E), 2.0), 0.0);
    }

    #[test]
    fn directories_report_changed_missing_size_and_new() {
        let (b, c) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        img(0x16181D).save(b.path().join("same.png")).unwrap();
        img(0x16181D).save(c.path().join("same.png")).unwrap();
        img(0x16181D).save(b.path().join("changed.png")).unwrap();
        half(0x16181D, 0xCE2C31)
            .save(c.path().join("changed.png"))
            .unwrap();
        img(0x16181D).save(b.path().join("gone.png")).unwrap();
        img(0x16181D).save(b.path().join("resized.png")).unwrap();
        RgbaImage::from_pixel(8, 8, rgba(0x16181D))
            .save(c.path().join("resized.png"))
            .unwrap();
        img(0x16181D).save(c.path().join("fresh.png")).unwrap();
        let (results, new) = compare_dirs(b.path(), c.path(), 2.0, 0.001, &[]).unwrap();
        assert_eq!(
            results,
            vec![
                ("changed.png".to_string(), Outcome::Changed(0.5)),
                ("gone.png".to_string(), Outcome::Missing),
                ("resized.png".to_string(), Outcome::Size((16, 16), (8, 8))),
                ("same.png".to_string(), Outcome::Ok(0.0)),
            ]
        );
        assert_eq!(new, vec!["fresh.png".to_string()]);
        // A breach exits 1 (Ok(false)); a clean comparison exits 0 (Ok(true)).
        let args = |b: &Path, c: &Path| {
            [
                "--baseline",
                b.to_str().unwrap(),
                "--candidate",
                c.to_str().unwrap(),
            ]
            .map(String::from)
            .to_vec()
        };
        assert!(!run_inner(&args(b.path(), c.path())).unwrap());
        assert!(run_inner(&args(c.path(), c.path())).unwrap());
    }

    // ## m3b-ci (controller fix)
    #[test]
    fn superseded_baselines_are_listed_not_compared_and_a_stale_line_is_an_error() {
        let (b, c) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        for name in [
            "menu-app-dark-en.png",
            "menu-app-light-en.png",
            "shell-dark-en.png",
        ] {
            img(0x16181D).save(b.path().join(name)).unwrap();
            half(0x16181D, 0xCE2C31).save(c.path().join(name)).unwrap();
        }
        let list = superseded_patterns("# changed in M2a\n\nmenu-app-*\n  shell-dark-en.png \n");
        assert_eq!(list, ["menu-app-*", "shell-dark-en.png"]);
        let (results, _) = compare_dirs(b.path(), c.path(), 2.0, 0.001, &list).unwrap();
        assert!(
            results
                .iter()
                .all(|(_, o)| *o == Outcome::Superseded && !o.breaches())
        );
        // Only what the list names is skipped: the rest still breaches.
        let only_menus = ["menu-app-*".to_string()];
        let (results, _) = compare_dirs(b.path(), c.path(), 2.0, 0.001, &only_menus).unwrap();
        assert_eq!(
            results.last(),
            Some(&("shell-dark-en.png".to_string(), Outcome::Changed(0.5)))
        );
        // A line that excuses nothing is an error, not a silent skip.
        let stale = ["toast-info-*".to_string()];
        let err = compare_dirs(b.path(), c.path(), 2.0, 0.001, &stale).unwrap_err();
        assert!(
            format!("{err:#}").contains("matches no baseline"),
            "{err:#}"
        );
        // Through the command line: the list file makes the run pass.
        let file = b.path().join("superseded.txt");
        std::fs::write(&file, "menu-app-*\nshell-dark-en.png\n").unwrap();
        let args = |extra: &[&str]| {
            let mut v = vec![
                "--baseline",
                b.path().to_str().unwrap(),
                "--candidate",
                c.path().to_str().unwrap(),
            ];
            v.extend_from_slice(extra);
            v.into_iter().map(String::from).collect::<Vec<_>>()
        };
        assert!(!run_inner(&args(&[])).unwrap());
        assert!(run_inner(&args(&["--superseded", file.to_str().unwrap()])).unwrap());
    }

    #[test]
    fn an_empty_baseline_and_bad_flags_are_errors() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().to_str().unwrap();
        let err = run_inner(&["--baseline", p, "--candidate", p].map(String::from)).unwrap_err();
        assert!(
            format!("{err:#}").contains("no PNG in the baseline"),
            "{err:#}"
        );
        assert!(run_inner(&["--baseline".to_string()]).is_err());
        assert!(
            run_inner(&["--max-share", "2", "--baseline", p, "--candidate", p].map(String::from))
                .is_err()
        );
    }
}
