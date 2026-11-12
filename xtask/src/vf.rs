//! `xtask vf`: visual-fidelity comparison between a Chromium reference capture of the harman mock
//! and a GPUI candidate capture. Colour samples are compared with CIEDE2000, scanlines compare
//! edge positions (±1 logical px). `--side-by-side` writes reference|candidate for PO sign-off.
//!
//! Before grading the candidate the expectations themselves are validated against the
//! reference: both images must be 1440x900 x scale, every reference sample pixel must match its
//! labelled DESIGN.md token (dE2000 <= 1, translucent tokens composited over their `over`
//! backdrop), and every scanline must find exactly its expected edges in the reference. Any of
//! these failing is an error (exit 2: "reference/token mismatch" / "bad expectation"), not a
//! candidate FAIL.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use image::{GenericImage, Rgba, RgbaImage};
use serde_json::Value;

/// A colour sample passes when ΔE2000 between reference and candidate is at most this.
pub const SAMPLE_TOLERANCE_DE: f64 = 1.0;
/// Edge detector threshold: ΔE between neighbouring pixels above which a pixel starts an edge.
pub const EDGE_TOLERANCE_DE: f64 = 2.0;
/// Allowed deviation (logical px) between expected and found edges.
pub const EDGE_SLACK_PX: f64 = 1.0;

// ───────────────────────── Colour science ─────────────────────────

fn srgb_to_linear(c: f64) -> f64 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

/// sRGB (D65) to CIE L*a*b* (D65 white point).
pub fn srgb_to_lab(rgb: [u8; 3]) -> [f64; 3] {
    let [r, g, b] = rgb.map(|v| srgb_to_linear(v as f64 / 255.0));
    let x = 0.4124564 * r + 0.3575761 * g + 0.1804375 * b;
    let y = 0.2126729 * r + 0.7151522 * g + 0.0721750 * b;
    let z = 0.0193339 * r + 0.1191920 * g + 0.9503041 * b;
    const WHITE: [f64; 3] = [0.95047, 1.0, 1.08883];
    let f = |t: f64| {
        const D: f64 = 6.0 / 29.0;
        if t > D * D * D { t.cbrt() } else { t / (3.0 * D * D) + 4.0 / 29.0 }
    };
    let (fx, fy, fz) = (f(x / WHITE[0]), f(y / WHITE[1]), f(z / WHITE[2]));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

/// CIEDE2000 colour difference (Sharma, Wu, Dalal 2005), kL = kC = kH = 1.
pub fn delta_e2000(a: [f64; 3], b: [f64; 3]) -> f64 {
    use std::f64::consts::PI;
    let (l1, a1, b1) = (a[0], a[1], a[2]);
    let (l2, a2, b2) = (b[0], b[1], b[2]);
    let c1 = a1.hypot(b1);
    let c2 = a2.hypot(b2);
    let cbar = (c1 + c2) / 2.0;
    let cbar7 = cbar.powi(7);
    let g = 0.5 * (1.0 - (cbar7 / (cbar7 + 25f64.powi(7))).sqrt());
    let (ap1, ap2) = ((1.0 + g) * a1, (1.0 + g) * a2);
    let (cp1, cp2) = (ap1.hypot(b1), ap2.hypot(b2));
    let hue = |bb: f64, aa: f64| {
        if bb == 0.0 && aa == 0.0 {
            0.0
        } else {
            let h = bb.atan2(aa).to_degrees();
            if h < 0.0 { h + 360.0 } else { h }
        }
    };
    let (hp1, hp2) = (hue(b1, ap1), hue(b2, ap2));
    let dl = l2 - l1;
    let dc = cp2 - cp1;
    let dhp = if cp1 * cp2 == 0.0 {
        0.0
    } else if (hp2 - hp1).abs() <= 180.0 {
        hp2 - hp1
    } else if hp2 - hp1 > 180.0 {
        hp2 - hp1 - 360.0
    } else {
        hp2 - hp1 + 360.0
    };
    let dh = 2.0 * (cp1 * cp2).sqrt() * (dhp.to_radians() / 2.0).sin();
    let lbar = (l1 + l2) / 2.0;
    let cpbar = (cp1 + cp2) / 2.0;
    let hpbar = if cp1 * cp2 == 0.0 {
        hp1 + hp2
    } else if (hp1 - hp2).abs() <= 180.0 {
        (hp1 + hp2) / 2.0
    } else if hp1 + hp2 < 360.0 {
        (hp1 + hp2 + 360.0) / 2.0
    } else {
        (hp1 + hp2 - 360.0) / 2.0
    };
    let t = 1.0 - 0.17 * (hpbar - 30.0).to_radians().cos()
        + 0.24 * (2.0 * hpbar).to_radians().cos()
        + 0.32 * (3.0 * hpbar + 6.0).to_radians().cos()
        - 0.20 * (4.0 * hpbar - 63.0).to_radians().cos();
    let dtheta = 30.0 * (-((hpbar - 275.0) / 25.0).powi(2)).exp();
    let cpbar7 = cpbar.powi(7);
    let rc = 2.0 * (cpbar7 / (cpbar7 + 25f64.powi(7))).sqrt();
    let sl = 1.0 + 0.015 * (lbar - 50.0).powi(2) / (20.0 + (lbar - 50.0).powi(2)).sqrt();
    let sc = 1.0 + 0.045 * cpbar;
    let sh = 1.0 + 0.015 * cpbar * t;
    let rt = -(2.0 * dtheta * PI / 180.0).sin() * rc;
    let (tl, tc, th) = (dl / sl, dc / sc, dh / sh);
    (tl * tl + tc * tc + th * th + rt * tc * th).sqrt()
}

fn de_pixels(a: &Rgba<u8>, b: &Rgba<u8>) -> f64 {
    delta_e2000(srgb_to_lab([a[0], a[1], a[2]]), srgb_to_lab([b[0], b[1], b[2]]))
}

// ───────────────────────── Scanlines ─────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    /// Scan along x at a fixed y (`at`).
    X,
    /// Scan along y at a fixed x (`at`).
    Y,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Scanline {
    pub name: String,
    pub axis: Axis,
    pub at: u32,
    pub from: u32,
    pub to: u32,
    pub expect_edges_px: Vec<u32>,
    /// Edge detector threshold override (low-contrast seams such as `seam-soft` on `panel`).
    pub tol_de: Option<f64>,
}

#[cfg(test)]
impl Scanline {
    /// Scans along x, at row `y`, from `from` to `to` (exclusive).
    pub fn x(y: u32, from: u32, to: u32) -> Self {
        Self { name: String::new(), axis: Axis::X, at: y, from, to, expect_edges_px: vec![], tol_de: None }
    }
}

/// Positions (along the scan axis, in image pixels) where ΔE to the previous pixel exceeds `tol_de`.
pub fn find_edges(img: &RgbaImage, line: &Scanline, tol_de: f64) -> Vec<u32> {
    let (w, h) = img.dimensions();
    let px = |p: u32| match line.axis {
        Axis::X => (p, line.at),
        Axis::Y => (line.at, p),
    };
    let max = match line.axis {
        Axis::X => w,
        Axis::Y => h,
    };
    let mut out = vec![];
    for p in (line.from + 1)..line.to.min(max) {
        let (x0, y0) = px(p - 1);
        let (x1, y1) = px(p);
        if x1 >= w || y1 >= h {
            break;
        }
        if de_pixels(img.get_pixel(x0, y0), img.get_pixel(x1, y1)) > tol_de {
            out.push(p);
        }
    }
    out
}

/// Collapses device-pixel edges into logical positions, merging neighbours closer than 1 logical px.
fn logical_edges(edges: &[u32], scale: f64) -> Vec<f64> {
    let mut out: Vec<f64> = vec![];
    for e in edges {
        let l = *e as f64 / scale;
        // Several device pixels can belong to one logical edge; keep the first of each.
        if out.last().is_none_or(|last| l - last >= 1.0 - 1e-9) {
            out.push(l);
        }
    }
    out
}

/// Match expected logical edges against found ones, ±`EDGE_SLACK_PX`.
/// Returns (missing expected, unexpected found).
pub fn match_edges(expected: &[u32], found: &[f64]) -> (Vec<u32>, Vec<f64>) {
    let missing = expected
        .iter()
        .copied()
        .filter(|e| !found.iter().any(|f| (f - *e as f64).abs() <= EDGE_SLACK_PX + 1e-9))
        .collect();
    let extra = found
        .iter()
        .copied()
        .filter(|f| !expected.iter().any(|e| (f - *e as f64).abs() <= EDGE_SLACK_PX + 1e-9))
        .collect();
    (missing, extra)
}

// ───────────────────────── Samples ─────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct ColorSample {
    pub name: String,
    pub x: f64,
    pub y: f64,
    pub token: String,
    pub theme: String,
    /// Opaque token a translucent `token` is composited over.
    pub over: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Samples {
    pub colors: Vec<ColorSample>,
    pub scanlines: Vec<Scanline>,
}

pub fn parse_samples(json: &str) -> Result<Samples> {
    let v: Value = serde_json::from_str(json)?;
    let s = |o: &Value, k: &str| -> Result<String> {
        Ok(o.get(k).and_then(Value::as_str).with_context(|| format!("missing string `{k}`"))?.to_string())
    };
    let n = |o: &Value, k: &str| -> Result<f64> {
        o.get(k).and_then(Value::as_f64).with_context(|| format!("missing number `{k}`"))
    };
    let mut colors = vec![];
    for c in v["colors"].as_array().context("colors[]")? {
        colors.push(ColorSample {
            name: s(c, "name")?,
            x: n(c, "x")?,
            y: n(c, "y")?,
            token: s(c, "token")?,
            theme: s(c, "theme")?,
            over: c.get("over").and_then(Value::as_str).map(str::to_string),
        });
    }
    let mut scanlines = vec![];
    for l in v["scanlines"].as_array().context("scanlines[]")? {
        let axis = match s(l, "axis")?.as_str() {
            "x" => Axis::X,
            "y" => Axis::Y,
            other => bail!("bad axis {other}"),
        };
        scanlines.push(Scanline {
            name: s(l, "name")?,
            axis,
            at: n(l, "at")? as u32,
            from: n(l, "from")? as u32,
            to: n(l, "to")? as u32,
            expect_edges_px: l["expect_edges_px"]
                .as_array()
                .context("expect_edges_px")?
                .iter()
                .filter_map(Value::as_u64)
                .map(|v| v as u32)
                .collect(),
            tol_de: l.get("tol_de").and_then(Value::as_f64),
        });
    }
    Ok(Samples { colors, scanlines })
}

fn sample_pixel(img: &RgbaImage, x: f64, y: f64, scale: f64) -> Rgba<u8> {
    let (w, h) = img.dimensions();
    let px = (((x + 0.5) * scale).floor() as u32).min(w - 1);
    let py = (((y + 0.5) * scale).floor() as u32).min(h - 1);
    *img.get_pixel(px, py)
}

/// Scales a logical scanline into device pixels.
fn scale_line(l: &Scanline, scale: f64) -> Scanline {
    let at = ((l.at as f64 + 0.5) * scale).floor() as u32;
    Scanline {
        name: l.name.clone(),
        axis: l.axis,
        at,
        from: (l.from as f64 * scale).floor() as u32,
        to: (l.to as f64 * scale).ceil() as u32,
        expect_edges_px: l.expect_edges_px.clone(),
        tol_de: l.tol_de,
    }
}

/// The logical frame every capture must show (DESIGN.md reference window).
pub const FRAME: (u32, u32) = (1440, 900);

/// Fails unless `img` is exactly `FRAME` x `scale` device pixels.
pub fn check_size(img: &RgbaImage, scale: f64, what: &str) -> Result<()> {
    let want = (
        (FRAME.0 as f64 * scale).round() as u32,
        (FRAME.1 as f64 * scale).round() as u32,
    );
    if img.dimensions() != want {
        bail!(
            "{what} is {}x{}, expected {}x{} ({}x{} at scale {scale})",
            img.width(),
            img.height(),
            want.0,
            want.1,
            FRAME.0,
            FRAME.1
        );
    }
    Ok(())
}

fn token_rgba(tokens: &crate::tokens::Tokens, theme: &str, token: &str) -> Result<u32> {
    let key = token.replace('_', "-");
    let set = match theme {
        "dark" => &tokens.dark,
        "light" => &tokens.light,
        other => bail!("unknown theme {other}"),
    };
    set.color(&key)
        .or_else(|| tokens.shared.color(&key))
        .with_context(|| format!("token `{token}` is not in DESIGN.md colors ({theme})"))
}

/// The sRGB colour a sample labelled `token` (optionally `token@opacity`) must show in `theme`:
/// the DESIGN.md value, or for a translucent token its source-over composite on the opaque
/// `over` token (8-bit, as Chromium blends).
pub fn expected_rgb(
    tokens: &crate::tokens::Tokens,
    theme: &str,
    token: &str,
    over: Option<&str>,
) -> Result<[u8; 3]> {
    let (name, opacity) = match token.split_once('@') {
        Some((n, op)) => (n, op.parse::<f64>().with_context(|| format!("bad opacity in `{token}`"))?),
        None => (token, 1.0),
    };
    let fg = token_rgba(tokens, theme, name)?;
    let ch = |c: u32, i: u32| f64::from((c >> (24 - 8 * i)) & 0xFF);
    let alpha = ch(fg, 3) / 255.0 * opacity;
    if alpha >= 1.0 {
        return Ok([0, 1, 2].map(|i| ch(fg, i) as u8));
    }
    let over = over.with_context(|| format!("translucent token `{token}` needs an `over` backdrop"))?;
    let bg = token_rgba(tokens, theme, over)?;
    if bg & 0xFF != 0xFF {
        bail!("backdrop `{over}` of `{token}` is not opaque");
    }
    Ok([0, 1, 2].map(|i| (ch(fg, i) * alpha + ch(bg, i) * (1.0 - alpha)).round() as u8))
}

struct Args {
    reference: Option<PathBuf>,
    candidate: Option<PathBuf>,
    scale: f64,
    theme: String,
    samples: PathBuf,
    side_by_side: Option<PathBuf>,
}

fn parse_args(args: &[String]) -> Result<Args> {
    let mut a = Args {
        reference: None,
        candidate: None,
        scale: 2.0,
        theme: "dark".into(),
        samples: PathBuf::from("design/vf/samples.json"),
        side_by_side: None,
    };
    let mut it = args.iter();
    while let Some(k) = it.next() {
        let mut val = || it.next().cloned().with_context(|| format!("{k} needs a value"));
        match k.as_str() {
            "--reference" => a.reference = Some(val()?.into()),
            "--candidate" => a.candidate = Some(val()?.into()),
            "--scale" => a.scale = val()?.parse()?,
            "--theme" => {
                a.theme = val()?;
                if a.theme != "dark" && a.theme != "light" {
                    bail!("--theme must be dark or light, got {:?}", a.theme);
                }
            }
            "--samples" => a.samples = val()?.into(),
            "--side-by-side" => a.side_by_side = Some(val()?.into()),
            other => bail!("unknown flag {other}"),
        }
    }
    Ok(a)
}

/// Whether a PNG carries an embedded ICC profile (an `iCCP` chunk before the pixel data).
pub fn png_has_icc_profile(bytes: &[u8]) -> bool {
    const SIG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if !bytes.starts_with(SIG) {
        return false;
    }
    let mut i = SIG.len();
    while i + 8 <= bytes.len() {
        let len = u32::from_be_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]) as usize;
        match &bytes[i + 4..i + 8] {
            b"iCCP" => return true,
            b"IDAT" | b"IEND" => return false,
            _ => {}
        }
        i += 12 + len;
    }
    false
}

fn load(p: &Path) -> Result<RgbaImage> {
    Ok(image::open(p).with_context(|| format!("open {}", p.display()))?.to_rgba8())
}

/// Reference left, candidate right, 8 device px gap. Heights are padded to the taller one.
pub fn side_by_side(reference: &RgbaImage, candidate: &RgbaImage) -> RgbaImage {
    let gap = 8;
    let w = reference.width() + gap + candidate.width();
    let h = reference.height().max(candidate.height());
    let mut out = RgbaImage::from_pixel(w, h, Rgba([128, 128, 128, 255]));
    out.copy_from(reference, 0, 0).expect("fits");
    out.copy_from(candidate, reference.width() + gap, 0).expect("fits");
    out
}

pub fn run() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(2).collect();
    match run_inner(&args) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            eprintln!("vf: {e:#}");
            ExitCode::from(2)
        }
    }
}

fn run_inner(args: &[String]) -> Result<bool> {
    let a = parse_args(args)?;
    let cand_path = a.candidate.as_deref().context("--candidate required")?;
    if png_has_icc_profile(&std::fs::read(cand_path)?) {
        bail!(
            "candidate {} carries an embedded ICC profile (iCCP chunk); the comparison assumes \
             untagged sRGB. Convert and strip it first (design/vf/capture.sh does this with sips)",
            cand_path.display()
        );
    }

    let rf = load(a.reference.as_deref().context("--reference required")?)?;
    let cd = load(cand_path)?;
    if let Some(out) = &a.side_by_side {
        side_by_side(&rf, &cd).save(out)?;
        println!("wrote {}", out.display());
    }
    let samples = parse_samples(&std::fs::read_to_string(&a.samples)?)?;
    let tokens = crate::tokens::load_repo_sources()?;
    compare(&rf, &cd, &samples, &a.theme, a.scale, &tokens)
}

/// Validates the expectations against the reference (errors), then grades the candidate:
/// `Ok(false)` when any candidate sample or scanline fails.
fn compare(
    rf: &RgbaImage,
    cd: &RgbaImage,
    samples: &Samples,
    theme: &str,
    scale: f64,
    tokens: &crate::tokens::Tokens,
) -> Result<bool> {
    check_size(rf, scale, "reference")?;
    check_size(cd, scale, "candidate")?;
    println!(
        "reference {}x{}  candidate {}x{}  scale {}",
        rf.width(), rf.height(), cd.width(), cd.height(), scale
    );
    let colors: Vec<&ColorSample> = samples.colors.iter().filter(|c| c.theme == theme).collect();

    // 1. Every reference sample shows its labelled token.
    let mut mismatches = vec![];
    for c in &colors {
        let want = expected_rgb(tokens, theme, &c.token, c.over.as_deref())
            .with_context(|| format!("sample {}", c.name))?;
        let pr = sample_pixel(rf, c.x, c.y, scale);
        let de = delta_e2000(srgb_to_lab(want), srgb_to_lab([pr[0], pr[1], pr[2]]));
        if de > SAMPLE_TOLERANCE_DE {
            mismatches.push(format!(
                "{} ({}, {}) {}{}: reference #{:02X}{:02X}{:02X}, token #{:02X}{:02X}{:02X}, dE {de:.2}",
                c.name, c.x, c.y, c.token,
                c.over.as_deref().map(|o| format!(" over {o}")).unwrap_or_default(),
                pr[0], pr[1], pr[2], want[0], want[1], want[2]
            ));
        }
    }
    if !mismatches.is_empty() {
        bail!("reference/token mismatch ({theme}):\n  {}", mismatches.join("\n  "));
    }

    // 2. Every scanline finds exactly its expected edges in the reference.
    let edges = |img: &RgbaImage, l: &Scanline| {
        let found = logical_edges(
            &find_edges(img, &scale_line(l, scale), l.tol_de.unwrap_or(EDGE_TOLERANCE_DE)),
            scale,
        );
        let (missing, extra) = match_edges(&l.expect_edges_px, &found);
        (found, missing, extra)
    };
    let mut bad = vec![];
    for l in &samples.scanlines {
        let (found, missing, extra) = edges(rf, l);
        if !missing.is_empty() || !extra.is_empty() {
            bad.push(format!(
                "{}: expected {:?}, reference has {found:?} (missing {missing:?}, extra {extra:?})",
                l.name, l.expect_edges_px
            ));
        }
    }
    if !bad.is_empty() {
        bail!("bad expectation: scanlines disagree with the reference:\n  {}", bad.join("\n  "));
    }

    // 3. Grade the candidate.
    let (mut pass, mut fail) = (0, 0);
    for c in &colors {
        let (pr, pc) = (sample_pixel(rf, c.x, c.y, scale), sample_pixel(cd, c.x, c.y, scale));
        let de = de_pixels(&pr, &pc);
        let ok = de <= SAMPLE_TOLERANCE_DE;
        println!(
            "{} color {:<28} ({:>6.1},{:>6.1}) {:<18} ref #{:02X}{:02X}{:02X} cand #{:02X}{:02X}{:02X} dE {:.2}",
            if ok { "ok  " } else { "FAIL" }, c.name, c.x, c.y, c.token,
            pr[0], pr[1], pr[2], pc[0], pc[1], pc[2], de
        );
        if ok { pass += 1 } else { fail += 1 }
    }
    for l in &samples.scanlines {
        let (found, missing, extra) = edges(cd, l);
        let ok = missing.is_empty() && extra.is_empty();
        let fmt: Vec<String> = found.iter().map(|f| format!("{f:.1}")).collect();
        println!(
            "{} scan  {:<28} expected {:?} found [{}]{}{}",
            if ok { "ok  " } else { "FAIL" }, l.name, l.expect_edges_px, fmt.join(", "),
            if missing.is_empty() { String::new() } else { format!(" missing {missing:?}") },
            if extra.is_empty() { String::new() } else { format!(" extra {extra:?}") },
        );
        if ok { pass += 1 } else { fail += 1 }
    }
    println!("vf: {pass} ok, {fail} FAIL");
    Ok(fail == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Sharma, Wu, Dalal (2005) CIEDE2000 test data, pairs 1, 7, 17 and 34
    #[test]
    fn de2000_sharma_pair_1() {
        assert!((delta_e2000([50.0, 2.6772, -79.7751], [50.0, 0.0, -82.7485]) - 2.0425).abs() < 1e-4);
    }
    #[test]
    fn de2000_sharma_pair_7() {
        assert!((delta_e2000([50.0, 0.0, 0.0], [50.0, -1.0, 2.0]) - 2.3669).abs() < 1e-4);
    }
    #[test]
    fn de2000_sharma_pair_17() {
        assert!((delta_e2000([50.0, 2.5, 0.0], [73.0, 25.0, -18.0]) - 27.1492).abs() < 1e-4);
    }
    #[test]
    fn de2000_sharma_pair_34() {
        assert!((delta_e2000([2.0776, 0.0795, -1.1350], [0.9033, -0.0636, -0.5514]) - 0.9082).abs() < 1e-4);
    }
    #[test]
    fn srgb_white_is_l100() {
        let l = srgb_to_lab([255, 255, 255]);
        assert!((l[0] - 100.0).abs() < 1e-3);
    }

    fn rgb(hex: u32) -> Rgba<u8> {
        Rgba([(hex >> 16) as u8, (hex >> 8) as u8, hex as u8, 255])
    }
    fn synthetic_with_vertical_seam(seam_x: u32, seam: u32, fill: u32) -> RgbaImage {
        RgbaImage::from_fn(600, 600, |x, _| if x == seam_x { rgb(seam) } else { rgb(fill) })
    }

    #[test]
    fn finds_one_px_seam() {
        let img = synthetic_with_vertical_seam(276, 0x2A2E36, 0x1E2127);
        assert_eq!(find_edges(&img, &Scanline::x(450, 200, 400), 2.0), vec![276, 277]);
    }

    #[test]
    fn scaled_seam_collapses_to_logical_edges() {
        // 2x capture: the 1 logical px seam is 2 device px wide.
        let img = RgbaImage::from_fn(800, 100, |x, _| {
            if (552..554).contains(&x) { rgb(0x2A2E36) } else { rgb(0x1E2127) }
        });
        let sl = scale_line(&Scanline::x(10, 200, 400), 2.0);
        let found = logical_edges(&find_edges(&img, &sl, 2.0), 2.0);
        assert_eq!(match_edges(&[276, 277], &found), (vec![], vec![]));
    }

    #[test]
    fn edge_matching_reports_missing_and_extra() {
        let (missing, extra) = match_edges(&[276, 400], &[276.0, 310.0]);
        assert_eq!(missing, vec![400]);
        assert_eq!(extra, vec![310.0]);
    }

    #[test]
    fn parses_samples_json_shape() {
        let s = parse_samples(
            r#"{"colors":[{"name":"a","x":1,"y":2,"token":"ground","theme":"dark"}],
                "scanlines":[{"name":"s","axis":"y","at":5,"from":0,"to":9,"expect_edges_px":[3]}]}"#,
        )
        .unwrap();
        assert_eq!(s.colors[0].token, "ground");
        assert_eq!(s.scanlines[0].axis, Axis::Y);
        assert_eq!(s.scanlines[0].expect_edges_px, vec![3]);
    }

    #[test]
    fn side_by_side_concatenates_horizontally() {
        let a = RgbaImage::from_pixel(10, 4, rgb(0xFF0000));
        let b = RgbaImage::from_pixel(6, 4, rgb(0x00FF00));
        let o = side_by_side(&a, &b);
        assert_eq!(o.dimensions(), (24, 4));
        assert_eq!(*o.get_pixel(0, 0), rgb(0xFF0000));
        assert_eq!(*o.get_pixel(18, 0), rgb(0x00FF00));
    }

    fn fake_png(chunks: &[&[u8; 4]]) -> Vec<u8> {
        let mut v = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        for c in chunks {
            v.extend_from_slice(&2u32.to_be_bytes());
            v.extend_from_slice(*c);
            v.extend_from_slice(&[0, 0]);
            v.extend_from_slice(&[0, 0, 0, 0]); // crc (not checked)
        }
        v
    }

    #[test]
    fn detects_embedded_icc_profile() {
        assert!(png_has_icc_profile(&fake_png(&[b"IHDR", b"iCCP", b"IDAT", b"IEND"])));
        assert!(!png_has_icc_profile(&fake_png(&[b"IHDR", b"IDAT", b"IEND"])));
        // iCCP after IDAT is not a valid position and is ignored.
        assert!(!png_has_icc_profile(&fake_png(&[b"IHDR", b"IDAT", b"iCCP"])));
    }

    #[test]
    fn candidate_with_icc_profile_is_refused() {
        let dir = std::env::temp_dir().join("vf-icc-test");
        std::fs::create_dir_all(&dir).unwrap();
        let png = dir.join("tagged.png");
        std::fs::write(&png, fake_png(&[b"IHDR", b"iCCP", b"IDAT", b"IEND"])).unwrap();
        let args: Vec<String> = ["--reference", png.to_str().unwrap(), "--candidate", png.to_str().unwrap()]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let err = run_inner(&args).unwrap_err().to_string();
        assert!(err.contains("ICC profile"), "{err}");
    }

    #[test]
    fn samples_cover_required_tokens_and_seams() {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../design/vf/samples.json");
        let s = parse_samples(&std::fs::read_to_string(p).unwrap()).unwrap();
        for theme in ["dark", "light"] {
            let tokens: Vec<&str> = s
                .colors
                .iter()
                .filter(|c| c.theme == theme)
                .map(|c| c.token.split('@').next().unwrap())
                .collect();
            for want in [
                "cp_fill", "ground", "panel", "side", "head", "gutter", "raised", "seam", "seam_strong",
                "seam_soft", "blue", "blue_tint", "blue_tint_strong", "red", "red_tint", "amber",
                "green_fill", "text_2", "text_3",
            ] {
                assert!(tokens.contains(&want), "{theme}: no sample for token {want}");
            }
        }
        let has_edge = |name: &str, e: u32| {
            s.scanlines.iter().any(|l| l.name.contains(name) && l.expect_edges_px.contains(&e))
        };
        assert!(has_edge("explorer-editor-seam", 276), "x = 276 seam");
        assert!(has_edge("titlebar", 39), "title-bar seam");
        let rows = s.scanlines.iter().find(|l| l.name.contains("grid-rows")).expect("grid rows scanline");
        for e in [483, 504, 525, 546, 567] {
            assert!(rows.expect_edges_px.contains(&e), "row border {e}");
        }
    }

    fn dark_rgb(tokens: &crate::tokens::Tokens, token: &str) -> Rgba<u8> {
        let [r, g, b] = expected_rgb(tokens, "dark", token, None).unwrap();
        Rgba([r, g, b, 255])
    }

    /// A 1440x900 (scale 1) dark frame of `ground` with a 1 px `seam` column at x = 276.
    fn frame(tokens: &crate::tokens::Tokens) -> RgbaImage {
        let (ground, seam) = (dark_rgb(tokens, "ground"), dark_rgb(tokens, "seam"));
        RgbaImage::from_fn(1440, 900, |x, _| if x == 276 { seam } else { ground })
    }

    fn frame_samples(token: &str, expect: Vec<u32>) -> Samples {
        Samples {
            colors: vec![ColorSample {
                name: "bg".into(),
                x: 100.0,
                y: 100.0,
                token: token.into(),
                theme: "dark".into(),
                over: None,
            }],
            scanlines: vec![Scanline {
                name: "seam".into(),
                expect_edges_px: expect,
                ..Scanline::x(450, 250, 300)
            }],
        }
    }

    #[test]
    fn rejects_themes_other_than_dark_and_light() {
        let args: Vec<String> = ["--theme", "blue", "--reference", "r.png", "--candidate", "c.png"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let err = run_inner(&args).unwrap_err().to_string();
        assert!(err.contains("dark or light"), "{err}");
    }

    #[test]
    fn rejects_images_that_are_not_1440x900_at_scale() {
        let t = crate::tokens::load_repo_sources().unwrap();
        let good = frame(&t);
        let small = RgbaImage::new(1440, 899);
        let s = frame_samples("ground", vec![276, 277]);
        for (rf, cd) in [(&small, &good), (&good, &small)] {
            let err = compare(rf, cd, &s, "dark", 1.0, &t).unwrap_err().to_string();
            assert!(err.contains("1440x900"), "{err}");
        }
        // The same frame is too small at scale 2.
        let err = compare(&good, &good, &s, "dark", 2.0, &t).unwrap_err().to_string();
        assert!(err.contains("2880x1800"), "{err}");
    }

    #[test]
    fn reference_pixels_must_match_their_tokens() {
        let t = crate::tokens::load_repo_sources().unwrap();
        let img = frame(&t);
        let err = compare(&img, &img, &frame_samples("raised", vec![276, 277]), "dark", 1.0, &t)
            .unwrap_err()
            .to_string();
        assert!(err.contains("reference/token mismatch"), "{err}");
    }

    #[test]
    fn reference_scanlines_must_match_expectations() {
        let t = crate::tokens::load_repo_sources().unwrap();
        let img = frame(&t);
        let err = compare(&img, &img, &frame_samples("ground", vec![300, 301]), "dark", 1.0, &t)
            .unwrap_err()
            .to_string();
        assert!(err.contains("bad expectation"), "{err}");
    }

    #[test]
    fn valid_reference_grades_the_candidate() {
        let t = crate::tokens::load_repo_sources().unwrap();
        let rf = frame(&t);
        let s = frame_samples("ground", vec![276, 277]);
        assert!(compare(&rf, &rf, &s, "dark", 1.0, &t).unwrap());
        // Candidate off by a whole token at the sample: graded FAIL, not an error.
        let mut cd = rf.clone();
        cd.put_pixel(100, 100, dark_rgb(&t, "raised"));
        assert!(!compare(&rf, &cd, &s, "dark", 1.0, &t).unwrap());
    }

    #[test]
    fn translucent_tokens_composite_over_their_backdrop() {
        let t = crate::tokens::load_repo_sources().unwrap();
        // Chromium renders .row.sel (--blue-a over --panel) as #293140 in dark.
        assert_eq!(expected_rgb(&t, "dark", "blue_tint", Some("panel")).unwrap(), [41, 49, 64]);
        // `@.6` multiplies the alpha (CSS opacity): text_3 at 60 % over light gutter.
        assert_eq!(expected_rgb(&t, "light", "text_3@.6", Some("gutter")).unwrap(), [165, 165, 171]);
        assert!(expected_rgb(&t, "dark", "blue_tint", None).is_err(), "needs a backdrop");
        assert!(expected_rgb(&t, "dark", "no_such_token", None).is_err());
    }

    /// Regions of the 1440x900 (scale 1) M1 shell that the harman reference and the M1 shell
    /// draw alike (Task 18): everything else in harman (tree, editor, grid, inspector) is not in M1.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Zone {
        TitleBar,
        StatusBar,
        PanelHeader,
        Seam,
    }

    /// The zone a colour sample lies in: seams at x ∈ {276, 1188} or y ∈ {362, 671} (±1 px), the
    /// title bar (y < 40), the status bar (y ≥ 876), the explorer and connection panel headers
    /// (y ∈ [40, 70) or [671, 701), x < 276).
    fn zone_of(p: &ColorSample) -> Option<Zone> {
        let near = |v: f64, at: f64| (v - at).abs() <= 1.0;
        if [276.0, 1188.0].iter().any(|&x| near(p.x, x))
            || [362.0, 671.0].iter().any(|&y| near(p.y, y))
        {
            Some(Zone::Seam)
        } else if p.y < 40.0 {
            Some(Zone::TitleBar)
        } else if p.y >= 876.0 {
            Some(Zone::StatusBar)
        } else if p.x < 276.0 && ((40.0..70.0).contains(&p.y) || (671.0..701.0).contains(&p.y)) {
            Some(Zone::PanelHeader)
        } else {
            None
        }
    }

    #[test]
    fn m1_shell_samples_validate_against_reference() {
        let s = parse_samples(
            &std::fs::read_to_string(
                crate::workspace_root().join("design/vf/samples-m1-shell.json"),
            )
            .unwrap(),
        )
        .unwrap();
        for p in &s.colors {
            assert!(zone_of(p).is_some(), "{p:?} lies outside every zone");
        }
        for z in [
            Zone::TitleBar,
            Zone::StatusBar,
            Zone::PanelHeader,
            Zone::Seam,
        ] {
            assert!(
                s.colors.iter().any(|p| zone_of(p) == Some(z)),
                "no sample in {z:?}"
            );
        }
        // Both themes are sampled alike, and every scanline expects edges.
        let count = |theme: &str| s.colors.iter().filter(|c| c.theme == theme).count();
        assert_eq!(count("dark"), count("light"));
        assert!(!s.scanlines.is_empty());
        for l in &s.scanlines {
            assert!(!l.expect_edges_px.is_empty(), "{}", l.name);
        }
        // Every reference token resolves in DESIGN.md (the self-check `xtask vf` runs first).
        let t = crate::tokens::load_repo_sources().unwrap();
        for c in &s.colors {
            expected_rgb(&t, &c.theme, &c.token, c.over.as_deref())
                .unwrap_or_else(|e| panic!("{}: {e:#}", c.name));
        }
    }

    #[test]
    fn shipped_samples_json_is_valid_and_large_enough() {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../design/vf/samples.json");
        let s = parse_samples(&std::fs::read_to_string(p).unwrap()).unwrap();
        assert!(s.colors.iter().filter(|c| c.theme == "dark").count() >= 40);
        assert!(s.scanlines.len() >= 8);
    }
}
