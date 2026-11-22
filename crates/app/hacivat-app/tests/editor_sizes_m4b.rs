// ## m4b-t11-editor (fix round 1)
//! M4b Task 11: the sizes the editor's T2 banner and paste question show are formatted by the
//! app's `SizeText` in the format locale, which follows the OS region (spec L15(d)), not the
//! UI language: a tr-TR region writes "42,6 MB" even in English. (The mock's `nf()` follows
//! the language toggle; that is a mock simplification, noted in design/m4b-scripts NOTES §9.)

mod support;

use gpui::{TestAppContext, VisualTestContext};
use hacivat_ui_editor::clipboard::paste_confirm_dialog;
use support::*;

/// Fluent's bidi isolation marks removed.
fn plain(s: &str) -> String {
    s.replace(['\u{2068}', '\u{2069}'], "")
}

fn question(vcx: &VisualTestContext) -> (String, String) {
    let spec = paste_confirm_dialog(44_669_747, 63_963_136);
    vcx.cx
        .read(|cx| (plain(&(spec.title)(cx)), plain(&(spec.body)(cx))))
}

#[gpui::test]
fn the_paste_question_writes_sizes_in_the_format_locale(cx: &mut TestAppContext) {
    let vcx = shell(cx, "en");
    vcx.cx.update(|cx| {
        let settings = hacivat_app::shell::update_settings(cx, |s| {
            s.ui.regional_format = hacivat_settings::LanguagePref::Tag("en-US".to_string());
        });
        hacivat_app::l10n_global::switch_locale(cx, &settings).detach();
    });
    wait_until(vcx, "the en-US region", |vcx| {
        question(vcx).0 == "Paste 42.6 MB?"
    });
    vcx.cx.update(|cx| {
        let settings = hacivat_app::shell::update_settings(cx, |s| {
            s.ui.regional_format = hacivat_settings::LanguagePref::Tag("tr-TR".to_string());
        });
        hacivat_app::l10n_global::switch_locale(cx, &settings).detach();
    });
    wait_until(vcx, "the tr-TR region", |vcx| {
        question(vcx).0 == "Paste 42,6 MB?"
    });
    assert_eq!(
        question(vcx).1,
        "That makes this document 61,0 MB, too large to edit. Paste it as a read-only preview, \
         or cancel."
    );
    // The T2 banner's sizes come from the same formatter.
    let size = vcx
        .cx
        .read(|cx| plain(&hacivat_app::console::view::size_text(cx, 148 << 20)));
    assert_eq!(size, "148,0 MB");
    stop_core_runtime_test(cx);
}
