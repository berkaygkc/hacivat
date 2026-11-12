//! `xtask tokens`: generate `theme/generated.rs` from `DESIGN.md` + `.impeccable/design.json`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};

use anyhow::{Context, Result, anyhow, bail};
use yaml_rust2::{Yaml, YamlLoader};

/// Line height (as a multiple of font size) used when a typography entry has none.
const DEFAULT_LINE_HEIGHT: f32 = 1.35;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct ColorSet(BTreeMap<String, u32>);

impl ColorSet {
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn color(&self, key: &str) -> Option<u32> {
        self.0.get(key).copied()
    }

    pub fn keys(&self) -> Vec<&str> {
        self.0.keys().map(String::as_str).collect()
    }
}

/// `fontVariantCaps` in DESIGN.md typography. Small caps are synthesized (capitals at 0.7x with
/// tracking, like the approved mock), never the font's `smcp`/`c2sc` (PO decision, M0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Caps {
    Normal,
    SynthesizedSmallCaps,
}

impl Caps {
    fn parse(s: Option<&str>) -> Result<Self> {
        match s {
            None | Some("normal") => Ok(Self::Normal),
            Some("synthesized") => Ok(Self::SynthesizedSmallCaps),
            Some(other) => bail!("fontVariantCaps must be `normal` or `synthesized`, got `{other}`"),
        }
    }

    fn rust(self) -> &'static str {
        match self {
            Self::Normal => "Caps::Normal",
            Self::SynthesizedSmallCaps => "Caps::SynthesizedSmallCaps",
        }
    }
}

/// An OpenType feature tag is exactly four printable ASCII characters (0x20..=0x7E).
fn check_feature_tag(tag: &str) -> Result<()> {
    if tag.len() == 4 && tag.bytes().all(|b| (0x20..=0x7E).contains(&b)) {
        Ok(())
    } else {
        bail!("`{tag}` is not an OpenType feature tag (exactly 4 ASCII characters)")
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypeSpec {
    pub name: String,
    pub family: String,
    pub size_px: f32,
    pub weight: u16,
    /// Resolved to logical pixels (unitless values are multiplied by the size).
    pub line_height: f32,
    /// Tracking in logical pixels (`em` values are multiplied by the size); 0.0 when absent.
    pub letter_spacing_px: f32,
    pub features: Vec<String>,
    pub caps: Caps,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub x: f32,
    pub y: f32,
    pub blur: f32,
    pub spread: f32,
    pub color: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tokens {
    pub dark: ColorSet,
    pub light: ColorSet,
    pub shared: ColorSet,
    pub typography: Vec<TypeSpec>,
    pub radii: Vec<(String, f32)>,
    pub spacing: Vec<(String, f32)>,
    pub durations: Vec<(String, u32)>,
    pub easings: Vec<(String, [f32; 4])>,
    pub shadow_dark: Vec<Layer>,
    pub shadow_light: Vec<Layer>,
}

fn parse_hex(s: &str) -> Result<u32> {
    u32::from_str_radix(s, 16).with_context(|| format!("bad hex `{s}`"))
}

/// `#RRGGBB`, `#RRGGBBAA` or `rgba(r,g,b,a)` -> `0xRRGGBBAA`.
pub fn parse_color(s: &str) -> Result<u32> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#') {
        return match hex.len() {
            6 => Ok(parse_hex(hex)? << 8 | 0xFF),
            8 => parse_hex(hex),
            _ => bail!("bad hex colour `{s}`"),
        };
    }
    if let Some(inner) = s.strip_prefix("rgba(").and_then(|r| r.strip_suffix(')')) {
        let p: Vec<&str> = inner.split(',').map(str::trim).collect();
        if p.len() != 4 {
            bail!("bad rgba colour `{s}`");
        }
        let ch = |v: &str| -> Result<u32> {
            let n: u32 = v.parse().with_context(|| format!("bad channel in `{s}`"))?;
            if n > 255 {
                bail!("channel out of range in `{s}`");
            }
            Ok(n)
        };
        let a: f64 = p[3]
            .parse()
            .with_context(|| format!("bad alpha in `{s}`"))?;
        if !(0.0..=1.0).contains(&a) {
            bail!("alpha out of range in `{s}`");
        }
        // Positive values only, so `round` (half away from zero) is round-half-up.
        let a = (a * 255.0).round() as u32;
        return Ok(ch(p[0])? << 24 | ch(p[1])? << 16 | ch(p[2])? << 8 | a);
    }
    bail!("unsupported colour `{s}`")
}

/// `"13.5px"` -> 13.5. Anything but `px` is rejected.
pub fn parse_px(s: &str) -> Result<f32> {
    let n = s
        .trim()
        .strip_suffix("px")
        .ok_or_else(|| anyhow!("expected a px value, got `{s}`"))?;
    n.trim()
        .parse()
        .with_context(|| format!("bad px value `{s}`"))
}

fn parse_cubic_bezier(s: &str) -> Result<[f32; 4]> {
    let inner = s
        .trim()
        .strip_prefix("cubic-bezier(")
        .and_then(|r| r.strip_suffix(')'))
        .ok_or_else(|| anyhow!("expected cubic-bezier(..), got `{s}`"))?;
    let v: Vec<f32> = inner
        .split(',')
        .map(|p| p.trim().parse::<f32>())
        .collect::<Result<_, _>>()
        .with_context(|| format!("bad cubic-bezier `{s}`"))?;
    v.try_into()
        .map_err(|_| anyhow!("cubic-bezier needs 4 numbers: `{s}`"))
}

fn frontmatter(md: &str) -> Result<&str> {
    let rest = md
        .strip_prefix("---\n")
        .ok_or_else(|| anyhow!("DESIGN.md has no YAML frontmatter"))?;
    let end = rest
        .find("\n---")
        .ok_or_else(|| anyhow!("DESIGN.md frontmatter is not closed"))?;
    Ok(&rest[..end])
}

fn yaml_str(y: &Yaml) -> Option<String> {
    match y {
        Yaml::String(s) => Some(s.clone()),
        Yaml::Integer(i) => Some(i.to_string()),
        Yaml::Real(r) => Some(r.clone()),
        _ => None,
    }
}

fn yaml_f64(y: &Yaml) -> Option<f64> {
    match y {
        Yaml::Integer(i) => Some(*i as f64),
        Yaml::Real(r) => r.parse().ok(),
        _ => None,
    }
}

/// A YAML mapping as sorted `(key, value)` pairs.
fn section<'a>(doc: &'a Yaml, name: &str) -> Result<BTreeMap<String, &'a Yaml>> {
    let hash = doc[name]
        .as_hash()
        .ok_or_else(|| anyhow!("frontmatter `{name}` is missing or not a mapping"))?;
    hash.iter()
        .map(|(k, v)| {
            let k = yaml_str(k).ok_or_else(|| anyhow!("non-string key in `{name}`"))?;
            Ok((k, v))
        })
        .collect()
}

fn px_section(doc: &Yaml, name: &str) -> Result<Vec<(String, f32)>> {
    section(doc, name)?
        .into_iter()
        .map(|(k, v)| {
            let s = yaml_str(v).ok_or_else(|| anyhow!("`{name}.{k}` is not a string"))?;
            let px = parse_px(&s).with_context(|| format!("`{name}.{k}`"))?;
            Ok((k, px))
        })
        .collect()
}

fn parse_type(name: &str, v: &Yaml) -> Result<TypeSpec> {
    let field = |f: &str| -> Result<String> {
        yaml_str(&v[f]).ok_or_else(|| anyhow!("typography.{name}.{f} is missing"))
    };
    let size_px = parse_px(&field("fontSize")?).with_context(|| format!("typography.{name}"))?;
    let line_height = match &v["lineHeight"] {
        Yaml::BadValue | Yaml::Null => size_px * DEFAULT_LINE_HEIGHT,
        Yaml::String(s) => parse_px(s).with_context(|| format!("typography.{name}.lineHeight"))?,
        other => {
            let m = yaml_f64(other)
                .ok_or_else(|| anyhow!("typography.{name}.lineHeight is not a number"))?;
            size_px * m as f32
        }
    };
    let line_height = (line_height * 100.0).round() / 100.0;
    let letter_spacing_px = match yaml_str(&v["letterSpacing"]) {
        None => 0.0,
        Some(s) => {
            let ctx = || format!("typography.{name}.letterSpacing");
            let t = s.trim();
            let px = if let Some(em) = t.strip_suffix("em") {
                size_px * em.trim().parse::<f32>().with_context(ctx)?
            } else {
                parse_px(t).with_context(ctx)?
            };
            (px * 1000.0).round() / 1000.0
        }
    };
    let weight = yaml_f64(&v["fontWeight"])
        .ok_or_else(|| anyhow!("typography.{name}.fontWeight is missing"))? as u16;
    let features: Vec<String> = match yaml_str(&v["fontFeature"]) {
        Some(s) => s
            .split(',')
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect(),
        None => Vec::new(),
    };
    for f in &features {
        check_feature_tag(f).with_context(|| format!("typography.{name}.fontFeature"))?;
    }
    let caps = match &v["fontVariantCaps"] {
        Yaml::BadValue | Yaml::Null => Caps::parse(None),
        other => Caps::parse(Some(yaml_str(other).as_deref().unwrap_or("<not a string>"))),
    }
    .with_context(|| format!("typography.{name}"))?;
    Ok(TypeSpec {
        name: name.to_string(),
        family: field("fontFamily")?,
        size_px,
        weight,
        line_height,
        letter_spacing_px,
        features,
        caps,
    })
}

fn parse_layers(shadows: &serde_json::Value, name: &str) -> Result<Vec<Layer>> {
    let entry = shadows
        .as_array()
        .and_then(|a| a.iter().find(|s| s["name"] == name))
        .ok_or_else(|| anyhow!("design.json shadow `{name}` is missing"))?;
    let layers = entry["layers"]
        .as_array()
        .ok_or_else(|| anyhow!("shadow `{name}` has no layers"))?;
    layers
        .iter()
        .map(|l| {
            let n = |k: &str| -> Result<f32> {
                l[k].as_f64()
                    .map(|v| v as f32)
                    .ok_or_else(|| anyhow!("shadow `{name}` layer lacks `{k}`"))
            };
            let color = l["color"]
                .as_str()
                .ok_or_else(|| anyhow!("shadow `{name}` layer lacks `color`"))?;
            Ok(Layer {
                x: n("x")?,
                y: n("y")?,
                blur: n("blur")?,
                spread: n("spread")?,
                color: parse_color(color)?,
            })
        })
        .collect()
}

pub fn load(design_md: &str, design_json: &str) -> Result<Tokens> {
    let design_md = normalize_eol(design_md);
    let docs = YamlLoader::load_from_str(frontmatter(&design_md)?)
        .context("DESIGN.md frontmatter is not valid YAML")?;
    let doc = docs.first().ok_or_else(|| anyhow!("empty frontmatter"))?;

    let (mut dark, mut light, mut shared) = (
        ColorSet::default(),
        ColorSet::default(),
        ColorSet::default(),
    );
    for (k, v) in section(doc, "colors")? {
        let s = yaml_str(v).ok_or_else(|| anyhow!("colors.{k} is not a string"))?;
        let c = parse_color(&s).with_context(|| format!("colors.{k}"))?;
        let (set, key) = if let Some(r) = k.strip_prefix("dark-") {
            (&mut dark, r)
        } else if let Some(r) = k.strip_prefix("light-") {
            (&mut light, r)
        } else {
            (&mut shared, k.as_str())
        };
        set.0.insert(key.to_string(), c);
    }
    if dark.keys() != light.keys() {
        let d: BTreeSet<_> = dark.keys().into_iter().collect();
        let l: BTreeSet<_> = light.keys().into_iter().collect();
        bail!(
            "dark/light colour keysets differ: only dark {:?}, only light {:?}",
            d.difference(&l).collect::<Vec<_>>(),
            l.difference(&d).collect::<Vec<_>>()
        );
    }

    let typography = section(doc, "typography")?
        .into_iter()
        .map(|(k, v)| parse_type(&k, v))
        .collect::<Result<Vec<_>>>()?;

    let json: serde_json::Value =
        serde_json::from_str(design_json).context("design.json is not valid JSON")?;
    let ext = &json["extensions"];
    let mut durations = Vec::new();
    let mut easings = Vec::new();
    for m in ext["motion"]
        .as_array()
        .ok_or_else(|| anyhow!("extensions.motion missing"))?
    {
        let name = m["name"]
            .as_str()
            .ok_or_else(|| anyhow!("motion entry without name"))?;
        if let Some(ms) = m["durationMs"].as_u64() {
            durations.push((name.to_string(), ms as u32));
        } else if let Some(arr) = m["easing"].as_array() {
            let v: Vec<f32> = arr
                .iter()
                .filter_map(|n| n.as_f64())
                .map(|n| n as f32)
                .collect();
            let v: [f32; 4] = v
                .try_into()
                .map_err(|_| anyhow!("motion `{name}` easing needs 4 numbers"))?;
            easings.push((name.to_string(), v));
        } else if let Some(s) = m["value"].as_str() {
            easings.push((name.to_string(), parse_cubic_bezier(s)?));
        } else {
            bail!("motion `{name}` has no durationMs, easing or value");
        }
    }
    durations.sort();
    easings.sort_by(|a, b| a.0.cmp(&b.0));

    Ok(Tokens {
        dark,
        light,
        shared,
        typography,
        radii: px_section(doc, "rounded")?,
        spacing: px_section(doc, "spacing")?,
        durations,
        easings,
        shadow_dark: parse_layers(&ext["shadows"], "popover-dark")?,
        shadow_light: parse_layers(&ext["shadows"], "popover-light")?,
    })
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

pub fn generated_path() -> PathBuf {
    repo_root().join("crates/ui/hacivat-ui-core/src/theme/generated.rs")
}

pub fn load_repo_sources() -> Result<Tokens> {
    let md = std::fs::read_to_string(repo_root().join("DESIGN.md")).context("reading DESIGN.md")?;
    let json = std::fs::read_to_string(repo_root().join(".impeccable/design.json"))
        .context("reading .impeccable/design.json")?;
    load(&normalize_eol(&md), &normalize_eol(&json))
}

/// kebab -> snake; keys that start with a digit get an `s_` prefix.
fn ident(key: &str) -> String {
    let s = key.replace('-', "_");
    if s.starts_with(|c: char| c.is_ascii_digit()) {
        format!("s_{s}")
    } else {
        s
    }
}

fn idents<'a>(keys: impl Iterator<Item = &'a str>) -> Result<Vec<String>> {
    let v: Vec<String> = keys.map(ident).collect();
    let mut seen = BTreeSet::new();
    for i in &v {
        if !seen.insert(i) {
            bail!("token name collision after snake_case: `{i}`");
        }
    }
    Ok(v)
}

fn color_lit(c: u32) -> String {
    format!("Color({c:#010X})")
}

fn layers_lit(layers: &[Layer]) -> String {
    let items: Vec<String> = layers
        .iter()
        .map(|l| {
            format!(
                "ShadowLayer {{ x: {:?}, y: {:?}, blur: {:?}, spread: {:?}, color: {} }}",
                l.x,
                l.y,
                l.blur,
                l.spread,
                color_lit(l.color)
            )
        })
        .collect();
    format!("&[{}]", items.join(", "))
}

fn color_struct(out: &mut String, name: &str, set: &ColorSet) -> Result<()> {
    let ids = idents(set.0.keys().map(String::as_str))?;
    writeln!(
        out,
        "#[derive(Debug, Clone, Copy, PartialEq)]\npub struct {name} {{"
    )?;
    for i in &ids {
        writeln!(out, "pub {i}: Color,")?;
    }
    writeln!(out, "}}")?;
    Ok(())
}

fn color_const(out: &mut String, cname: &str, sname: &str, set: &ColorSet) -> Result<()> {
    writeln!(out, "const {cname}: {sname} = {sname} {{")?;
    for (k, v) in &set.0 {
        writeln!(out, "{}: {},", ident(k), color_lit(*v))?;
    }
    writeln!(out, "}};")?;
    Ok(())
}

fn rustfmt(src: &str) -> Result<String> {
    let mut child = Command::new("rustfmt")
        .args(["--edition", "2024"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("spawning rustfmt")?;
    child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("no rustfmt stdin"))?
        .write_all(src.as_bytes())?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        bail!("rustfmt failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    Ok(String::from_utf8(out.stdout)?)
}

fn simple_struct(o: &mut String, name: &str, fields: Vec<(String, &str)>) -> Result<()> {
    writeln!(
        o,
        "#[derive(Debug, Clone, Copy, PartialEq)]\npub struct {name} {{"
    )?;
    for (f, ty) in fields {
        writeln!(o, "pub {f}: {ty},")?;
    }
    writeln!(o, "}}")?;
    Ok(())
}

pub fn render(t: &Tokens) -> Result<String> {
    let mut o = String::new();
    writeln!(
        o,
        "// @generated by `cargo xtask tokens` from DESIGN.md and .impeccable/design.json.\n\
         // Do not edit by hand; run `cargo xtask tokens` to regenerate.\n\
         use super::{{Caps, Color, ShadowLayer, ThemeTokens, TypeStyle}};\n"
    )?;

    color_struct(&mut o, "Colors", &t.dark)?;
    color_struct(&mut o, "SharedColors", &t.shared)?;

    let ty_ids = idents(t.typography.iter().map(|s| s.name.as_str()))?;
    writeln!(
        o,
        "#[derive(Debug, Clone, Copy, PartialEq)]\npub struct TypeRamp {{"
    )?;
    for i in &ty_ids {
        writeln!(o, "pub {i}: TypeStyle,")?;
    }
    writeln!(
        o,
        "}}\nimpl TypeRamp {{\npub fn all(&self) -> Vec<&TypeStyle> {{\nvec!["
    )?;
    for i in &ty_ids {
        writeln!(o, "&self.{i},")?;
    }
    writeln!(o, "]\n}}\n}}")?;

    let radii_ids = idents(t.radii.iter().map(|r| r.0.as_str()))?;
    simple_struct(
        &mut o,
        "Radii",
        radii_ids.iter().map(|i| (i.clone(), "f32")).collect(),
    )?;
    let sp_ids = idents(t.spacing.iter().map(|r| r.0.as_str()))?;
    simple_struct(
        &mut o,
        "Spacing",
        sp_ids.iter().map(|i| (i.clone(), "f32")).collect(),
    )?;
    let dur_ids = idents(t.durations.iter().map(|r| r.0.as_str()))?;
    let ease_ids = idents(t.easings.iter().map(|r| r.0.as_str()))?;
    let mut motion_fields: Vec<(String, &str)> =
        dur_ids.iter().map(|i| (i.clone(), "u32")).collect();
    motion_fields.extend(ease_ids.iter().map(|i| (i.clone(), "[f32; 4]")));
    simple_struct(&mut o, "Motion", motion_fields)?;

    writeln!(o, "pub const SHARED: SharedColors = SharedColors {{")?;
    for (k, v) in &t.shared.0 {
        writeln!(o, "{}: {},", ident(k), color_lit(*v))?;
    }
    writeln!(o, "}};")?;

    writeln!(o, "const TYPE_RAMP: TypeRamp = TypeRamp {{")?;
    for (id, s) in ty_ids.iter().zip(&t.typography) {
        let feats: Vec<String> = s.features.iter().map(|f| format!("{f:?}")).collect();
        writeln!(
            o,
            "{id}: TypeStyle {{ family: {:?}, size_px: {:?}, weight: {}, line_height: {:?}, letter_spacing_px: {:?}, features: &[{}], caps: {} }},",
            s.family,
            s.size_px,
            s.weight,
            s.line_height,
            s.letter_spacing_px,
            feats.join(", "),
            s.caps.rust()
        )?;
    }
    writeln!(o, "}};")?;

    writeln!(o, "const RADII: Radii = Radii {{")?;
    for (id, (_, v)) in radii_ids.iter().zip(&t.radii) {
        writeln!(o, "{id}: {v:?},")?;
    }
    writeln!(o, "}};\nconst SPACING: Spacing = Spacing {{")?;
    for (id, (_, v)) in sp_ids.iter().zip(&t.spacing) {
        writeln!(o, "{id}: {v:?},")?;
    }
    writeln!(o, "}};\nconst MOTION: Motion = Motion {{")?;
    for (id, (_, v)) in dur_ids.iter().zip(&t.durations) {
        writeln!(o, "{id}: {v},")?;
    }
    for (id, (_, v)) in ease_ids.iter().zip(&t.easings) {
        writeln!(o, "{id}: {v:?},")?;
    }
    writeln!(o, "}};")?;

    color_const(&mut o, "DARK_COLORS", "Colors", &t.dark)?;
    color_const(&mut o, "LIGHT_COLORS", "Colors", &t.light)?;
    for (name, colors, shadow) in [
        ("DARK", "DARK_COLORS", &t.shadow_dark),
        ("LIGHT", "LIGHT_COLORS", &t.shadow_light),
    ] {
        writeln!(
            o,
            "pub const {name}: ThemeTokens = ThemeTokens {{\ncolors: {colors},\ntype_ramp: TYPE_RAMP,\nradii: RADII,\nspacing: SPACING,\npopover_shadow: {},\nmotion: MOTION,\n}};",
            layers_lit(shadow)
        )?;
    }
    rustfmt(&o)
}

fn unified_diff(expected: &str, actual: &str) -> String {
    let dir = std::env::temp_dir();
    let (a, b) = (
        dir.join("tokens-current.rs"),
        dir.join("tokens-expected.rs"),
    );
    let _ = std::fs::write(&a, actual);
    let _ = std::fs::write(&b, expected);
    Command::new("diff")
        .arg("-u")
        .arg(&a)
        .arg(&b)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_else(|_| "(diff unavailable)".into())
}

/// `--check` compare that ignores CRLF vs LF (Windows checkouts without `.gitattributes`).
fn same_text(a: &str, b: &str) -> bool {
    normalize_eol(a) == normalize_eol(b)
}

/// CRLF -> LF, so a Windows checkout parses and compares like a Unix one.
fn normalize_eol(s: &str) -> std::borrow::Cow<'_, str> {
    if s.contains("\r\n") {
        std::borrow::Cow::Owned(s.replace("\r\n", "\n"))
    } else {
        std::borrow::Cow::Borrowed(s)
    }
}

fn run_inner(check: bool) -> Result<bool> {
    let rendered = render(&load_repo_sources()?)?;
    let path = generated_path();
    if check {
        let current = std::fs::read_to_string(&path).unwrap_or_default();
        if same_text(&current, &rendered) {
            return Ok(true);
        }
        eprint!("{}", unified_diff(&rendered, &current));
        eprintln!("generated.rs is stale; run `cargo xtask tokens`");
        return Ok(false);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, rendered)?;
    println!("wrote {}", path.display());
    Ok(true)
}

pub fn run() -> ExitCode {
    let check = std::env::args().skip(2).any(|a| a == "--check");
    match run_inner(check) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            eprintln!("tokens: {e:#}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MD: &str = r##"---
name: T
colors:
  dark-ground: "#16181D"
  dark-blue-tint: "rgba(122,162,247,0.12)"
  light-ground: "#F7F7F8"
  light-blue-tint: "rgba(59,95,217,0.08)"
  traffic-close: "#FF5F57"
typography:
  body:
    fontFamily: "Source Sans 3, system-ui, sans-serif"
    fontSize: "13px"
    fontWeight: 400
    lineHeight: 1.35
    fontFeature: "tnum"
  label-sc:
    fontFamily: "Source Sans 3"
    fontSize: "13px"
    fontWeight: 600
    fontVariantCaps: synthesized
  inlay:
    fontFamily: "Source Sans 3"
    fontSize: "11px"
    fontWeight: 500
    lineHeight: "14px"
rounded:
  none: "0px"
  xs: "3px"
  sm: "4px"
  md: "6px"
  full: "9999px"
spacing:
  2xs: "2px"
  titlebar-height: "40px"
---
body text
"##;

    const MD_MISSING_LIGHT_GROUND: &str = r##"---
colors:
  dark-ground: "#16181D"
  dark-blue-tint: "rgba(122,162,247,0.12)"
  light-blue-tint: "rgba(59,95,217,0.08)"
typography: {}
rounded: {none: "0px", xs: "3px", sm: "4px", md: "6px", full: "9999px"}
spacing: {}
---
"##;

    const JSON: &str = r#"{"extensions":{
      "shadows":[
        {"name":"popover-dark","layers":[{"x":0,"y":10,"blur":28,"spread":0,"color":"rgba(0,0,0,0.34)"}]},
        {"name":"popover-light","layers":[{"x":0,"y":12,"blur":32,"spread":-6,"color":"rgba(20,20,40,0.20)"}]}],
      "motion":[
        {"name":"m-dur-1","durationMs":140},
        {"name":"m-ease-out","easing":[0.16,1,0.3,1]},
        {"name":"m-ease-str","value":"cubic-bezier(.65,0,.35,1)"}]}}"#;

    #[test]
    fn parses_dark_ground() {
        let t = load(MD, JSON).unwrap();
        assert_eq!(t.dark.color("ground"), Some(0x16181DFF));
    }

    #[test]
    fn parses_rgba_tint() {
        let t = load(MD, JSON).unwrap();
        // 0.12 * 255 = 30.6 -> round = 31 = 0x1F
        assert_eq!(t.dark.color("blue-tint"), Some(0x7AA2F71F));
    }

    #[test]
    fn dark_and_light_keysets_match() {
        let t = load(MD, JSON).unwrap();
        assert_eq!(t.dark.keys(), t.light.keys());
    }

    #[test]
    fn rejects_mismatched_keysets() {
        assert!(load(MD_MISSING_LIGHT_GROUND, JSON).is_err());
    }

    #[test]
    fn unprefixed_colors_are_shared() {
        let t = load(MD, JSON).unwrap();
        assert_eq!(t.shared.color("traffic-close"), Some(0xFF5F57FF));
    }

    #[test]
    fn parses_type_motion_and_spacing() {
        let t = load(MD, JSON).unwrap();
        let body = &t.typography[0];
        assert_eq!(body.features, ["tnum"]);
        assert_eq!(body.line_height, 17.55);
        assert_eq!(body.letter_spacing_px, 0.0);
        assert_eq!(t.typography[1].line_height, 14.0);
        assert_eq!(t.spacing[0].0, "2xs");
        assert_eq!(t.durations, [("m-dur-1".to_string(), 140)]);
        assert_eq!(t.easings[1].1, [0.65, 0.0, 0.35, 1.0]);
        assert_eq!(t.shadow_dark[0].blur, 28.0);
    }

    #[test]
    fn parses_crlf_frontmatter() {
        let md = MD.replace('\n', "\r\n");
        assert_eq!(load(&md, JSON).unwrap(), load(MD, JSON).unwrap());
    }

    #[test]
    fn check_compare_ignores_crlf() {
        assert!(same_text("a\r\nb\r\n", "a\nb\n"));
        assert!(!same_text("a\nb\n", "a\nc\n"));
    }

    #[test]
    fn font_variant_caps_is_typed() {
        let t = load(MD, JSON).unwrap();
        let ty = |n: &str| t.typography.iter().find(|s| s.name == n).unwrap();
        assert_eq!(ty("body").caps, Caps::Normal);
        assert_eq!(ty("label-sc").caps, Caps::SynthesizedSmallCaps);
        assert!(ty("label-sc").features.is_empty());
        let normal = MD.replace("fontVariantCaps: synthesized", "fontVariantCaps: normal");
        let t = load(&normal, JSON).unwrap();
        assert_eq!(t.typography.iter().find(|s| s.name == "label-sc").unwrap().caps, Caps::Normal);
    }

    #[test]
    fn rejects_unknown_font_variant_caps() {
        // Real smcp/c2sc small caps are not an option (PO decision): only `synthesized`.
        for bad in ["all-small-caps", "smcp", "\"\""] {
            let md = MD.replace("fontVariantCaps: synthesized", &format!("fontVariantCaps: {bad}"));
            assert!(load(&md, JSON).is_err(), "{bad}");
        }
    }

    #[test]
    fn feature_tags_must_be_four_ascii_chars() {
        for bad in ["synthesized small caps", "smc", "tnum1", "smcp, c2", "çsmc"] {
            let md = MD.replace("fontFeature: \"tnum\"", &format!("fontFeature: \"{bad}\""));
            let err = load(&md, JSON).unwrap_err();
            assert!(format!("{err:#}").contains("feature tag"), "{bad}: {err:#}");
        }
        let ok = MD.replace("fontFeature: \"tnum\"", "fontFeature: \"ss01, cv11\"");
        assert!(load(&ok, JSON).is_ok());
    }

    #[test]
    fn repo_small_caps_are_synthesized_without_smcp() {
        let t = load_repo_sources().unwrap();
        for name in ["label-sc", "panel-title", "capsule-env"] {
            let s = t.typography.iter().find(|s| s.name == name).unwrap();
            assert_eq!(s.caps, Caps::SynthesizedSmallCaps, "{name}");
            assert!(
                !s.features.iter().any(|f| f == "smcp" || f == "c2sc"),
                "{name}: {:?}",
                s.features
            );
        }
        assert!(t.typography.iter().all(|s| s.features.iter().all(|f| f != "smcp" && f != "c2sc")));
    }

    #[test]
    fn rejects_non_px_units() {
        let md = MD.replace("\"13px\"", "\"13em\"");
        assert!(load(&md, JSON).is_err());
    }

    #[test]
    fn generated_file_is_current() {
        let current = std::fs::read_to_string(generated_path()).unwrap();
        assert_eq!(
            render(&load_repo_sources().unwrap()).unwrap(),
            normalize_eol(&current)
        );
    }
}
