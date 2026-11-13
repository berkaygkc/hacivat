//! UI text provider and the `Label` text registry (Task 11).

use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _,
    TestAppContext, Window, div, point, px, size,
};
use hacivat_ui::dialog::{DialogSize, dialog_frame};
use hacivat_ui::l10n::{EnglishDefaults, UiTextProvider};
use hacivat_ui::text::{Label, RenderedText, TextRegistry, TextSource, paragraph};
use hacivat_ui_core::theme::{DARK, Theme, ThemeMode, TypeStyle};

#[test]
fn english_defaults_render_hui() {
    assert_eq!(
        EnglishDefaults::new().text("hui-palette-no-results", &[]),
        "No matching commands"
    );
}

#[test]
fn english_defaults_render_every_task_11_message() {
    let e = EnglishDefaults::new();
    assert_eq!(e.text("hui-input-clear", &[]), "Clear");
    assert_eq!(e.text("hui-palette-hint-navigate", &[]), "Navigate");
    assert_eq!(e.text("hui-palette-hint-run", &[]), "Run");
    assert_eq!(e.text("hui-palette-hint-close", &[]), "Close");
    assert_eq!(e.generation(), 0);
}

#[cfg(debug_assertions)]
#[test]
fn english_defaults_render_unknown_ids_in_brackets() {
    assert_eq!(
        EnglishDefaults::new().text("hui-no-such", &[]),
        "⟦hui-no-such⟧"
    );
    assert_eq!(EnglishDefaults::new().attr("hui-no-such", "title"), None);
}

/// Draws one label in a `width`-px box with the registry enabled; returns the registry.
fn draw_label_in(
    cx: &mut TestAppContext,
    text: &'static str,
    width: f32,
    truncate: bool,
) -> TextRegistry {
    draw_style_in(cx, text, width, truncate, DARK.type_ramp.body)
}

/// A view holding one label in a fixed-width box (labels always render inside a view).
struct LabelHost {
    text: &'static str,
    width: f32,
    truncate: bool,
    style: TypeStyle,
}

impl Render for LabelHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(self.width)).child(
            Label::new(
                self.text,
                TextSource::Data,
                self.style,
                DARK.colors.text.to_hsla(),
            )
            .truncate(self.truncate),
        )
    }
}

fn draw_style_in(
    cx: &mut TestAppContext,
    text: &'static str,
    width: f32,
    truncate: bool,
    style: TypeStyle,
) -> TextRegistry {
    cx.update(|cx| {
        Theme::install(cx, ThemeMode::Dark);
        cx.set_global(TextRegistry {
            enabled: true,
            frame: Vec::new(),
        });
    });
    let (_, vcx) = cx.add_window_view(|_, _| LabelHost {
        text,
        width,
        truncate,
        style,
    });
    vcx.run_until_parked();
    vcx.read(|cx| cx.global::<TextRegistry>().clone())
}

#[gpui::test]
fn clipped_label_is_recorded(cx: &mut TestAppContext) {
    let reg = draw_label_in(cx, "A very long label", 40.0, false);
    assert!(reg.frame[0].clipped);
    let reg = draw_label_in(cx, "A very long label", 40.0, true);
    assert!(reg.frame[0].truncated && !reg.frame[0].clipped);
}

#[gpui::test]
fn registry_records_full_text_source_and_size_once_per_frame(cx: &mut TestAppContext) {
    let reg = draw_label_in(cx, "A very long label", 40.0, true);
    assert_eq!(reg.frame.len(), 1);
    let r: &RenderedText = &reg.frame[0];
    // The full text, not the ellipsized one.
    assert_eq!(r.text, "A very long label");
    assert_eq!(r.source, TextSource::Data);
    assert_eq!(r.size_px, DARK.type_ramp.body.size_px);
    // `/` separators on every platform (Windows' `Location::file` uses `\`).
    assert!(r.path.contains("hacivat-ui/tests/text.rs:"), "{}", r.path);
    assert!(!r.path.contains('\\'), "{}", r.path);
}

#[gpui::test]
fn label_that_fits_is_neither_clipped_nor_truncated(cx: &mut TestAppContext) {
    let reg = draw_label_in(cx, "Run", 200.0, true);
    assert!(!reg.frame[0].clipped && !reg.frame[0].truncated);
    let reg = draw_label_in(cx, "Run", 200.0, false);
    assert!(!reg.frame[0].clipped && !reg.frame[0].truncated);
}

#[gpui::test]
fn small_caps_labels_are_recorded_and_clip_like_plain_ones(cx: &mut TestAppContext) {
    let style = DARK.type_ramp.label_sc;
    let reg = draw_style_in(cx, "GEZGİN BAĞLANTILARI", 30.0, false, style);
    assert_eq!(reg.frame.len(), 1);
    assert_eq!(reg.frame[0].text, "GEZGİN BAĞLANTILARI");
    assert!(reg.frame[0].clipped);
    let reg = draw_style_in(cx, "GEZGİN BAĞLANTILARI", 30.0, true, style);
    assert!(reg.frame[0].truncated && !reg.frame[0].clipped);
    let reg = draw_style_in(cx, "GEZGİN", 300.0, false, style);
    assert!(!reg.frame[0].clipped && !reg.frame[0].truncated);
}

#[gpui::test]
fn disabled_registry_records_nothing(cx: &mut TestAppContext) {
    let vcx = cx.add_empty_window();
    vcx.update(|_, cx| cx.set_global(TextRegistry::default()));
    vcx.draw(
        point(px(0.0), px(0.0)),
        size(px(400.0), px(100.0)),
        |_, _| {
            div().child(Label::new(
                "Run",
                TextSource::Fluent,
                DARK.type_ramp.body,
                DARK.colors.text.to_hsla(),
            ))
        },
    );
    assert!(vcx.read(|cx| cx.global::<TextRegistry>().frame.is_empty()));
}

#[gpui::test]
fn each_frame_replaces_the_previous_one(cx: &mut TestAppContext) {
    let vcx = cx.add_empty_window();
    vcx.update(|_, cx| {
        cx.set_global(TextRegistry {
            enabled: true,
            frame: Vec::new(),
        })
    });
    let label = |t: &'static str| {
        Label::new(
            t,
            TextSource::Fluent,
            DARK.type_ramp.body,
            DARK.colors.text.to_hsla(),
        )
    };
    for _ in 0..3 {
        vcx.draw(
            point(px(0.0), px(0.0)),
            size(px(400.0), px(100.0)),
            move |_, _| div().child(label("Run")).child(label("Stop")),
        );
    }
    let texts: Vec<_> = vcx.read(|cx| {
        cx.global::<TextRegistry>()
            .frame
            .iter()
            .map(|r| r.text.to_string())
            .collect()
    });
    assert_eq!(texts, ["Run", "Stop"]);
}

/// A shrink-wrapped small-caps label in a row, with a selector on its wrapper.
struct NaturalHost {
    text: &'static str,
    style: TypeStyle,
    /// `Some(w)`: a fixed `w`-px box; `None`: natural width.
    width: Option<f32>,
}

impl Render for NaturalHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let label = Label::new(
            self.text,
            TextSource::Data,
            self.style,
            DARK.colors.text.to_hsla(),
        )
        .truncate(true);
        let mut row = div().flex().flex_row().debug_selector(|| "natural".into());
        if let Some(w) = self.width {
            row = row.w(px(w));
        }
        div().flex().child(row.child(label))
    }
}

fn draw_natural(
    cx: &mut TestAppContext,
    text: &'static str,
    style: TypeStyle,
    width: Option<f32>,
) -> (f32, f32, TextRegistry) {
    cx.update(|cx| {
        Theme::install(cx, ThemeMode::Dark);
        hacivat_ui_core::fonts::register_fonts(cx).unwrap();
        cx.set_global(TextRegistry {
            enabled: true,
            frame: Vec::new(),
        });
    });
    let (_, vcx) = cx.add_window_view(|_, _| NaturalHost { text, style, width });
    vcx.run_until_parked();
    let w = f32::from(vcx.debug_bounds("natural").unwrap().size.width);
    // The expected ink: every glyph shaped on its own at its small-caps size, plus the
    // tracking between glyphs (not after the last one).
    let expected = vcx.update(|window, _| {
        let font = hacivat_ui_core::fonts::font_for(&style);
        let gap = hacivat_ui_core::fonts::small_caps_tracking_px(&style);
        let glyphs = hacivat_ui_core::fonts::small_caps_glyphs(text, style.size_px);
        let n = glyphs.len();
        let sum: f32 = glyphs
            .into_iter()
            .map(|(ch, size)| {
                let run = gpui::TextRun {
                    len: ch.len_utf8(),
                    font: font.clone(),
                    color: DARK.colors.text.to_hsla(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                let line =
                    window
                        .text_system()
                        .shape_line(ch.to_string().into(), px(size), &[run], None);
                f32::from(line.width)
            })
            .sum();
        sum + gap * (n as f32 - 1.0)
    });
    (
        w,
        expected,
        vcx.read(|cx| cx.global::<TextRegistry>().clone()),
    )
}

#[gpui::test]
fn small_caps_paint_exactly_their_measured_run(cx: &mut TestAppContext) {
    for style in [
        DARK.type_ramp.label_sc,
        DARK.type_ramp.panel_title,
        DARK.type_ramp.capsule_env,
    ] {
        for text in ["HACIVAT", "PROD", "GEZGİN", "BAĞLANTI"] {
            let (natural, expected, reg) = draw_natural(cx, text, style, None);
            let painted = reg.frame[0].painted_px;
            // The painted run is the sum of its glyph advances plus tracking (the old flex row
            // painted ~0.5 px wider per glyph), and fits the box it was measured for.
            assert!(
                (painted - expected).abs() < 0.01,
                "{text}: painted {painted} vs glyph sum {expected}"
            );
            assert!(
                painted <= natural + 0.5,
                "{text}: {painted} > box {natural}"
            );
            // Handing the label exactly its measured width neither ellipsizes nor clips it.
            let (_, _, reg) = draw_natural(cx, text, style, Some(natural));
            let r = &reg.frame[0];
            assert!(!r.truncated && !r.clipped, "{text} at {natural}px: {r:?}");
            // A box sized to the ink (no trailing tracking) is not flagged either.
            let (_, _, reg) = draw_natural(cx, text, style, Some(expected.ceil()));
            assert!(!reg.frame[0].clipped, "{text} at ink width");
        }
    }
}

// ## m2b-t24-shots (follow-up, R1): check (f)'s detection, through a real dialog card.

/// A sentence that cannot fit a 420 px card on one line.
const TOO_WIDE: &str =
    "A label with no room to wrap that runs well past the right side of its dialog card";

/// A 420 px dialog card holding a label that fits, a label wider than the card, a paragraph
/// laid out in a column wider than the card and one that wraps inside it; then a wide label
/// outside any card.
struct CardHost;

impl Render for CardHost {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        let ink = DARK.colors.text.to_hsla();
        let body = DARK.type_ramp.body;
        let card = dialog_frame(&theme, DialogSize::Standard)
            .child(Label::new("Fits", TextSource::Data, body, ink).id("fits"))
            .child(
                div()
                    .flex_none()
                    .child(Label::new(TOO_WIDE, TextSource::Data, body, ink).id("too-wide")),
            )
            .child(
                div()
                    .flex_none()
                    .w(px(700.0))
                    .child(paragraph(TOO_WIDE, TextSource::Data, body, ink).id("wide-paragraph")),
            )
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .child(paragraph(TOO_WIDE, TextSource::Data, body, ink).id("wrapped")),
            );
        div()
            .flex()
            .flex_col()
            .child(card)
            .child(Label::new(TOO_WIDE, TextSource::Data, body, ink).id("no-card"))
    }
}

#[gpui::test]
fn text_past_its_dialog_card_is_recorded_as_such(cx: &mut TestAppContext) {
    cx.update(|cx| {
        Theme::install(cx, ThemeMode::Dark);
        cx.set_global(TextRegistry {
            enabled: true,
            frame: Vec::new(),
        });
    });
    let (_, vcx) = cx.add_window_view(|_, _| CardHost);
    vcx.simulate_resize(size(px(1400.0), px(900.0)));
    vcx.run_until_parked();
    let frame = vcx.read(|cx| cx.global::<TextRegistry>().frame.clone());
    let outside = |id: &str| {
        let suffix = format!("#{id}");
        frame
            .iter()
            .find(|r| r.path.ends_with(&suffix))
            .unwrap_or_else(|| panic!("{id} was recorded: {frame:?}"))
            .outside_card
    };
    assert!(!outside("fits"), "a label inside its card");
    assert!(outside("too-wide"), "a label past the card's right side");
    assert!(
        outside("wide-paragraph"),
        "a paragraph past the card's right side"
    );
    assert!(
        !outside("wrapped"),
        "a paragraph that wraps inside the card"
    );
    // The scope closes with the card: text after it is never measured against it.
    assert!(!outside("no-card"), "a label outside any card");
}
