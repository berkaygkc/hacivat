//! M0 non-gating probes (X row) plus font and Turkish casing checks.
//!
//! Each probe prints `probe <name>: ok|FAIL <detail>` and the process exits non-zero when any
//! probe FAILs. Recorded-only values are printed as `probe <name>: record ...`.
//!
//! The `a11y` probe follows `crates/gpui/examples/a11y.rs` at the GPUI pin (Apache-2.0).

#![allow(unsafe_code, reason = "M0 probes call platform APIs directly; spike only")]

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use hacivat_ui_core::app::{Boot, open_window, run_app};
use hacivat_ui_core::fonts::font_for;
use hacivat_ui_core::gpui_surface::{
    App, AppContext, Bounds, ClipboardItem, Context, Element, ElementId, Font, FontFeatures,
    FontWeight, GlobalElementId, InspectorElementId, IntoElement, LayoutId, ParentElement, Pixels,
    Render, Role, Style, Styled, TextRun, Window, WindowHandle, accesskit, div, hsla, px,
};
use hacivat_ui_core::theme::DARK;

static START: OnceLock<Instant> = OnceLock::new();

const ALL: [&str; 7] = [
    "fonts",
    "turkish_small_caps",
    "fallback",
    "bidi",
    "clipboard",
    "a11y",
    "startup",
];

fn main() {
    START.get_or_init(Instant::now);
    let arg = std::env::args().nth(1).unwrap_or_default();
    let probes: Vec<&'static str> = if arg == "all" {
        ALL.to_vec()
    } else if let Some(p) = ALL.iter().find(|p| **p == arg) {
        vec![*p]
    } else {
        eprintln!("usage: spike-probes <all|{}>", ALL.join("|"));
        std::process::exit(2);
    };
    run_app(Boot::default(), move |cx| {
        let first_frame_ms: Rc<RefCell<Option<f64>>> = Rc::new(RefCell::new(None));
        let ff = first_frame_ms.clone();
        // Shaping needs a window's text system, so every probe runs with a window open.
        let handle = match open_window(cx, "Hacivat probes", (480., 200.), |window, cx| {
            window.on_next_frame(move |_w, _cx| {
                *ff.borrow_mut() =
                    Some(START.get().map_or(0., |s| s.elapsed().as_secs_f64() * 1e3));
            });
            cx.new(|_| ProbeView)
        }) {
            Ok(h) => h,
            Err(e) => {
                println!("probe window: FAIL could not open window: {e:#}");
                std::process::exit(1);
            }
        };
        cx.activate(true);
        let mut failed = false;
        for p in &probes {
            let r = handle.update(cx, |_, window, cx| match *p {
                "fonts" => Some(fonts(window)),
                "turkish_small_caps" => Some(turkish_small_caps(window)),
                "fallback" => Some(fallback(window)),
                "bidi" => Some(bidi(window)),
                "clipboard" => Some(clipboard(cx)),
                _ => None,
            });
            match r {
                Ok(Some(r)) => failed |= report(p, r),
                Ok(None) => {}
                Err(e) => {
                    println!("probe {p}: FAIL window update failed: {e:#}");
                    failed = true;
                }
            }
        }
        let want_a11y = probes.contains(&"a11y");
        let want_startup = probes.contains(&"startup");
        if want_a11y || want_startup {
            window_probes(cx, handle, first_frame_ms, want_a11y, want_startup, failed);
        } else {
            std::process::exit(i32::from(failed));
        }
    });
}

/// Prints the probe line; returns true on FAIL.
fn report(name: &str, r: Result<String, String>) -> bool {
    match r {
        Ok(d) => {
            println!("probe {name}: ok {d}");
            false
        }
        Err(d) => {
            println!("probe {name}: FAIL {d}");
            true
        }
    }
}

fn record(name: &str, msg: impl AsRef<str>) {
    println!("probe {name}: record {}", msg.as_ref());
}

// ---------------------------------------------------------------- shaping helpers

struct Shaped {
    width: f32,
    /// (glyph id, byte index in source text), in run order.
    glyphs: Vec<(u32, usize)>,
    font_ids: Vec<usize>,
}

fn shape(cx: &Window, font: &Font, text: &str) -> Shaped {
    let run = TextRun {
        len: text.len(),
        font: font.clone(),
        color: hsla(0., 0., 0., 1.),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let layout = cx.text_system().layout_line(text, px(16.), &[run], None);
    let mut glyphs = Vec::new();
    let mut font_ids = Vec::new();
    for r in &layout.runs {
        font_ids.push(r.font_id.0);
        for g in &r.glyphs {
            glyphs.push((g.id.0, g.index));
        }
    }
    Shaped {
        width: f32::from(layout.width),
        glyphs,
        font_ids,
    }
}

fn ids(s: &Shaped) -> Vec<u32> {
    s.glyphs.iter().map(|g| g.0).collect()
}

fn with_features(base: &Font, feats: &[(&str, u32)]) -> Font {
    let mut f = base.clone();
    f.features = FontFeatures(std::sync::Arc::new(
        feats.iter().map(|(t, v)| (t.to_string(), *v)).collect(),
    ));
    f
}

fn plain(font: &Font) -> Font {
    with_features(font, &[])
}

// ---------------------------------------------------------------- fonts

fn fonts(cx: &Window) -> Result<String, String> {
    let body = font_for(&DARK.type_ramp.body);
    let code = font_for(&DARK.type_ramp.code);
    let mut notes = Vec::new();

    // Tabular digits: Source Sans 3 has no `tnum`; digits are tabular by default. Assert on
    // advances, with the body token's (inert) tnum request in place.
    let w0 = shape(cx, &body, "0000").width;
    let w1 = shape(cx, &body, "1111").width;
    if (w0 - w1).abs() > 0.001 {
        return Err(format!(
            "tabular digits: width(0000)={w0} != width(1111)={w1}"
        ));
    }
    notes.push(format!("digits 0000/1111 width={w0:.3}px"));

    // smcp changes glyph ids.
    let base = plain(&body);
    let off = ids(&shape(cx, &base, "Ab"));
    let on = ids(&shape(cx, &with_features(&base, &[("smcp", 1)]), "Ab"));
    if off == on {
        return Err(format!("smcp did not change glyph ids for \"Ab\": {off:?}"));
    }
    notes.push(format!("smcp Ab {off:?} -> {on:?}"));

    // Fragment Mono is fixed-pitch.
    let wi = shape(cx, &code, "iiii").width;
    let ww = shape(cx, &code, "WWWW").width;
    let code_id = cx.text_system().resolve_font(&code);
    let adv_i = f32::from(
        cx.text_system()
            .advance(code_id, px(16.), 'i')
            .map_err(|e| e.to_string())?
            .width,
    );
    let adv_w = f32::from(
        cx.text_system()
            .advance(code_id, px(16.), 'W')
            .map_err(|e| e.to_string())?
            .width,
    );
    if (wi - ww).abs() > 0.001 || (adv_i - adv_w).abs() > 0.001 {
        return Err(format!(
            "Fragment Mono not fixed-pitch: iiii={wi} WWWW={ww} adv(i)={adv_i} adv(W)={adv_w}"
        ));
    }
    notes.push(format!("mono i/W advance={adv_i:.3}px"));

    // Weight resolution through family "Source Sans 3": record only (verified on macOS only).
    let sample = "Hamburgefonstiv 0123";
    let at_weight = |w: f32| {
        let mut f = plain(&body);
        f.weight = FontWeight(w);
        f
    };
    for w in [400., 500., 600., 700.] {
        let f = at_weight(w);
        let id = cx.text_system().resolve_font(&f);
        let s = shape(cx, &f, sample);
        record(
            "fonts",
            format!(
                "weight {w} -> FontId({}) sample width={:.3}px glyph-run-font-ids={:?}",
                id.0, s.width, s.font_ids
            ),
        );
    }
    let fid = |w: f32| cx.text_system().resolve_font(&at_weight(w)).0;
    let width = |w: f32| shape(cx, &at_weight(w), sample).width;
    record(
        "fonts",
        format!(
            "distinct faces: 400!=500 {}, 500!=600 {}, 600!=700 {}; sample widths 400/500/600/700 = \
             {:.3}/{:.3}/{:.3}/{:.3}",
            fid(400.) != fid(500.),
            fid(500.) != fid(600.),
            fid(600.) != fid(700.),
            width(400.),
            width(500.),
            width(600.),
            width(700.)
        ),
    );
    Ok(notes.join("; "))
}

// ---------------------------------------------------------------- turkish_small_caps

fn turkish_small_caps(cx: &Window) -> Result<String, String> {
    let label_sc = font_for(&DARK.type_ramp.label_sc);

    // 1. Pre-cased "GEZGİN" (as Fluent will deliver it) with label-sc.
    let cased = shape(cx, &label_sc, "GEZGİN");
    let undotted = shape(cx, &label_sc, "GEZGIN");
    let dot_idx = "GEZG".len();
    let g_dotted = cased.glyphs.iter().find(|g| g.1 == dot_idx).map(|g| g.0);
    let g_plain = undotted.glyphs.iter().find(|g| g.1 == dot_idx).map(|g| g.0);
    let (Some(gd), Some(gp)) = (g_dotted, g_plain) else {
        return Err(format!(
            "no glyph at the I position: GEZGİN={:?} GEZGIN={:?}",
            cased.glyphs, undotted.glyphs
        ));
    };
    if gd == 0 {
        return Err("İ resolved to .notdef".into());
    }
    if gd == gp {
        return Err(format!("İ glyph {gd} equals I glyph {gp}: dot lost"));
    }
    if cased.glyphs.len() != 6 {
        return Err(format!(
            "GEZGİN shaped into {} glyphs (expected 6, decomposed?): {:?}",
            cased.glyphs.len(),
            cased.glyphs
        ));
    }

    // 2. Lower-case "gezgin" with smcp: record whether locl TRK applies. GPUI exposes no
    // language attribute, so a dotted small-cap i could only come from the platform shaper
    // choosing the font's locl lookup on its own.
    let smcp = with_features(&plain(&label_sc), &[("smcp", 1)]);
    let c2sc = with_features(&plain(&label_sc), &[("c2sc", 1)]);
    let lower = shape(cx, &smcp, "gezgin");
    let i_pos = "gezg".len();
    let g_i_smcp = lower.glyphs.iter().find(|g| g.1 == i_pos).map(|g| g.0);
    let latin_i_sc = shape(cx, &c2sc, "I").glyphs.first().map(|g| g.0);
    let dotted_i_sc = shape(cx, &c2sc, "İ").glyphs.first().map(|g| g.0);
    let trk_applied = g_i_smcp.is_some() && g_i_smcp == dotted_i_sc && g_i_smcp != latin_i_sc;
    record(
        "turkish_small_caps",
        format!(
            "locl TRK under GPUI shaping: {} (glyph of 'i' under smcp={g_i_smcp:?}, \
             'I' under c2sc={latin_i_sc:?}, 'İ' under c2sc={dotted_i_sc:?}, 'gezgin' smcp={:?}). {}",
            if trk_applied {
                "APPLIES"
            } else {
                "does NOT apply"
            },
            ids(&lower),
            if trk_applied {
                "Font-level casing works here, but only via platform locale; do not rely on it."
            } else {
                "Casing must always happen upstream (pre-cased text), never via font smcp."
            }
        ),
    );
    Ok(format!(
        "label-sc \"GEZGİN\" -> {} glyphs, İ glyph={gd} (I glyph={gp})",
        cased.glyphs.len()
    ))
}

// ---------------------------------------------------------------- fallback

fn fallback(cx: &Window) -> Result<String, String> {
    let text = "İıŞşĞğÜüÖöÇç ✓ → ≈";
    let body = font_for(&DARK.type_ramp.body);
    let s = shape(cx, &body, text);
    let missing: Vec<char> = s
        .glyphs
        .iter()
        .filter(|g| g.0 == 0)
        .filter_map(|g| text[g.1..].chars().next())
        .collect();
    if !missing.is_empty() {
        return Err(format!("`.notdef` glyphs for {missing:?}"));
    }
    // Record which face each segment landed on, to tell bundled-font glyphs from OS fallback.
    let turkish = shape(cx, &body, "İıŞşĞğÜüÖöÇç");
    let symbols = shape(cx, &body, "✓ → ≈");
    record(
        "fallback",
        format!(
            "run font ids: turkish={:?} symbols={:?} full={:?}",
            turkish.font_ids, symbols.font_ids, s.font_ids
        ),
    );
    Ok(format!("{} glyphs, no .notdef", s.glyphs.len()))
}

// ---------------------------------------------------------------- bidi

fn bidi(cx: &Window) -> Result<String, String> {
    let body = font_for(&DARK.type_ramp.body);
    let isolated = shape(cx, &body, "\u{2068}Ada\u{2069}");
    let bare = shape(cx, &body, "Ada");
    if (isolated.width - bare.width).abs() > 0.001 {
        return Err(format!(
            "FSI/PDI add width: isolated={} bare={} (glyph ids {:?})",
            isolated.width,
            bare.width,
            ids(&isolated)
        ));
    }
    Ok(format!("width {:.3}px both", bare.width))
}

// ---------------------------------------------------------------- clipboard

fn clipboard(cx: &App) -> Result<String, String> {
    let text = "Ada Lovelace İıŞ";
    let meta = r#"{"source":"spike-probes"}"#;
    cx.write_to_clipboard(ClipboardItem::new_string_with_metadata(
        text.to_string(),
        meta.to_string(),
    ));
    let item = cx
        .read_from_clipboard()
        .ok_or("read_from_clipboard returned None")?;
    let got_text = item.text().ok_or("no text in clipboard item")?;
    let got_meta_raw = item.metadata().cloned();
    if got_text != text {
        return Err(format!("text mismatch: wrote {text:?} read {got_text:?}"));
    }
    let meta_ok = got_meta_raw.as_deref() == Some(meta);
    #[allow(unused_mut)]
    let mut detail =
        format!("text round-trip ok; metadata round-trip {meta_ok} ({got_meta_raw:?})");
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        cx.write_to_primary(ClipboardItem::new_string("primary-selection".into()));
        let p = cx.read_from_primary().and_then(|i| i.text());
        detail.push_str(&format!("; primary selection read-back {p:?}"));
        if p.as_deref() != Some("primary-selection") {
            return Err(format!("primary selection mismatch: {p:?}"));
        }
    }
    if !meta_ok {
        return Err(format!(
            "metadata mismatch: wrote {meta:?} read {got_meta_raw:?}"
        ));
    }
    Ok(detail)
}

// ---------------------------------------------------------------- a11y + startup (windowed)

/// Custom element exposing one AccessKit node (role `Grid`, label "Result 1").
struct ResultGrid;

impl Element for ResultGrid {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(ElementId::Name("result-grid".into()))
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = px(240.).into();
        style.size.height = px(40.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _state: &mut (),
        _window: &mut Window,
        _cx: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        _prepaint: &mut (),
        _window: &mut Window,
        _cx: &mut App,
    ) {
    }

    fn a11y_role(&self) -> Option<Role> {
        Some(Role::Grid)
    }

    fn write_a11y_info(&self, node: &mut accesskit::Node) {
        node.set_label("Result 1");
    }
}

struct ProbeView;

impl Render for ProbeView {
    fn render(&mut self, _w: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().p_4().child(ResultGrid)
    }
}

fn window_probes(
    cx: &mut App,
    handle: WindowHandle<ProbeView>,
    first_frame_ms: Rc<RefCell<Option<f64>>>,
    want_a11y: bool,
    want_startup: bool,
    failed_so_far: bool,
) {
    let idle = if want_startup {
        Duration::from_secs(5)
    } else {
        Duration::from_millis(1500)
    };
    cx.spawn(async move |cx| {
        cx.background_executor().timer(idle).await;
        let mut failed = failed_so_far;
        cx.update(|cx| {
            if want_startup {
                match *first_frame_ms.borrow() {
                    Some(ms) => {
                        println!("probe startup: ok main()->first on_next_frame = {ms:.1} ms")
                    }
                    None => {
                        println!("probe startup: FAIL on_next_frame never fired");
                        failed = true;
                    }
                }
                match rss_bytes() {
                    Some(b) => println!(
                        "probe startup: ok RSS after {}s idle = {:.1} MiB",
                        idle.as_secs(),
                        b as f64 / 1048576.
                    ),
                    None => {
                        println!("probe startup: FAIL could not read RSS");
                        failed = true;
                    }
                }
            }
            if want_a11y {
                let tree = handle
                    .update(cx, |_, window, _| {
                        (window.is_a11y_active(), window.debug_a11y_tree_json())
                    })
                    .ok();
                match tree {
                    Some((active, json)) => {
                        let (line, fail) = a11y_line(active, json.as_deref());
                        println!("{line}");
                        failed |= fail;
                    }
                    None => {
                        println!("probe a11y: FAIL window update failed");
                        failed = true;
                    }
                }
            }
            std::process::exit(i32::from(failed));
        });
    })
    .detach();
}

/// The a11y probe line and whether it fails. GPUI's debug tree alone proves only that GPUI built
/// the nodes; the platform bridge is verified only while assistive technology is attached.
fn a11y_line(active: bool, json: Option<&str>) -> (String, bool) {
    let Some(json) = json else {
        return (
            format!(
                "probe a11y: record not verified (no assistive technology attached, \
                 is_a11y_active={active}). Turn on VoiceOver (Cmd+F5), Narrator or Orca \
                 and re-run to dump the node tree and verify manually."
            ),
            false,
        );
    };
    if !(json.contains("Result 1") && json.contains("Grid")) {
        return (
            format!("probe a11y: FAIL tree lacks Grid / \"Result 1\":\n{json}"),
            true,
        );
    }
    let line = if active {
        format!(
            "probe a11y: ok Grid \"Result 1\" present (assistive tech active); node tree:\n{json}"
        )
    } else {
        format!(
            "probe a11y: record GPUI tree ok; platform AT unverified (Grid \"Result 1\" in GPUI's \
             debug tree, is_a11y_active=false). Attach VoiceOver, Narrator or Orca to verify the \
             platform tree; node tree:\n{json}"
        )
    };
    (line, false)
}

// ---------------------------------------------------------------- RSS

#[cfg(target_os = "macos")]
fn rss_bytes() -> Option<u64> {
    // SAFETY: proc_pid_rusage fills the provided rusage_info_v2 buffer for our own pid.
    unsafe {
        let mut info: libc::rusage_info_v2 = std::mem::zeroed();
        let rc = libc::proc_pid_rusage(
            libc::getpid(),
            libc::RUSAGE_INFO_V2,
            (&mut info as *mut libc::rusage_info_v2).cast(),
        );
        (rc == 0).then_some(info.ri_resident_size)
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn rss_bytes() -> Option<u64> {
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    // SAFETY: sysconf has no preconditions.
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    Some(pages * u64::try_from(page).ok()?)
}

#[cfg(windows)]
fn rss_bytes() -> Option<u64> {
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    // SAFETY: the counters struct is sized and zero-initialised before the call.
    unsafe {
        let mut c: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
        c.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        (GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) != 0)
            .then_some(c.WorkingSetSize as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TREE: &str = r#"{"role":"Grid","label":"Result 1"}"#;

    #[test]
    fn a11y_with_assistive_tech_is_ok() {
        let (line, failed) = a11y_line(true, Some(TREE));
        assert!(line.starts_with("probe a11y: ok "), "{line}");
        assert!(!failed);
    }

    #[test]
    fn a11y_without_assistive_tech_does_not_claim_platform_ok() {
        let (line, failed) = a11y_line(false, Some(TREE));
        assert!(
            line.contains("GPUI tree ok; platform AT unverified"),
            "{line}"
        );
        assert!(!line.starts_with("probe a11y: ok"), "{line}");
        assert!(!failed);
    }

    #[test]
    fn a11y_tree_without_the_grid_fails() {
        for active in [true, false] {
            let (line, failed) = a11y_line(active, Some("{}"));
            assert!(line.starts_with("probe a11y: FAIL"), "{line}");
            assert!(failed);
        }
    }

    #[test]
    fn a11y_without_a_tree_is_recorded_unverified() {
        let (line, failed) = a11y_line(false, None);
        assert!(line.starts_with("probe a11y: record "), "{line}");
        assert!(!failed);
    }
}
