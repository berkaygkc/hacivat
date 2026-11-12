//! The localization globals and the runtime language switch (R4, F7, F8).
//!
//! [`L10nGlobal`] holds the one [`L10n`] of the app (its runtime is swapped atomically) and the
//! catalogs it was built from. [`install_l10n`] also installs the `hui-*` text provider, so the
//! widgets of `hacivat-ui` resolve through the same chain. [`switch_locale`] negotiates on the UI
//! thread, builds the new runtime on the background executor and installs it if no later
//! switch has won (F8), then repaints every window.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use hacivat_l10n::{
    ArgValue, Catalogs, FormatInputs, IntoFluentArgs, L10n, LanguageIdentifier, MsgRef, NoArgs,
    OsLocale, build_runtime, effective, negotiate,
};
use hacivat_settings::Settings;
use hacivat_ui::accesskey::AccessKeyLocale;
use hacivat_ui::l10n::{UiArg, UiText, UiTextProvider};
use hacivat_ui::text::TextFn;
use hacivat_ui_core::gpui_surface::{App, AppContext as _, Global, SharedString, Task};

/// The window title: the brand, which is never translated and is not chrome text.
pub const WINDOW_TITLE: &str = "Hacivat";

/// The app's localization: the swapped runtime and the catalogs it is built from.
#[derive(Debug, Clone)]
pub struct L10nGlobal {
    pub l10n: Arc<L10n>,
    pub catalogs: Arc<Catalogs>,
}

impl Global for L10nGlobal {}

/// What the OS reports about the user's locale, detected once at startup (empty in tests, so
/// they never depend on the machine).
#[derive(Debug, Clone, Default)]
pub struct HostLocale(pub OsLocale);

impl Global for HostLocale {}

/// Whether pseudo-locales (`en-XA`, `tr-XT`) may be negotiated: builds with `pseudo` only.
pub fn allow_pseudo() -> bool {
    cfg!(feature = "pseudo")
}

/// Builds the startup runtime synchronously, before any window opens.
pub fn build_initial(catalogs: &Catalogs, settings: &Settings, os: &OsLocale) -> L10n {
    let eff = effective(settings, os, allow_pseudo());
    let neg = negotiate(&eff.ctx, catalogs.locales());
    let fmt = FormatInputs::new(&neg, &eff.time_zone);
    L10n::new(build_runtime(catalogs, &neg, &fmt, 1))
}

/// Installs `l10n` and `catalogs` as the app's localization, the `hui-*` text provider over the
/// same runtime, and the access-key locale of its UI language.
pub fn install_l10n(cx: &mut App, l10n: Arc<L10n>, catalogs: Arc<Catalogs>) {
    cx.set_global(UiText(Arc::new(L10nUiProvider::new(l10n.clone()))));
    cx.set_global(L10nGlobal { l10n, catalogs });
    set_access_key_locale(cx);
}

/// Switches the UI language and formats to what `settings` say. The generation is reserved now,
/// so the last switch requested wins even if an earlier build finishes later (F8); the runtime
/// is built on the background executor (F7). Once installed, every window repaints, window
/// titles are set again and the menus are rebuilt (F15).
pub fn switch_locale(cx: &mut App, settings: &Settings) -> Task<()> {
    let Some(global) = cx.try_global::<L10nGlobal>().cloned() else {
        return Task::ready(());
    };
    let generation = global.l10n.reserve_generation();
    let os = cx
        .try_global::<HostLocale>()
        .map(|h| h.0.clone())
        .unwrap_or_default();
    let eff = effective(settings, &os, allow_pseudo());
    let neg = negotiate(&eff.ctx, global.catalogs.locales());
    let catalogs = global.catalogs.clone();
    let build = cx.background_spawn(async move {
        let fmt = FormatInputs::new(&neg, &eff.time_zone);
        build_runtime(&catalogs, &neg, &fmt, generation)
    });
    let l10n = global.l10n;
    cx.spawn(async move |cx| {
        let runtime = build.await;
        cx.update(|cx| {
            if l10n.install_if_newer(runtime) {
                after_switch(cx);
            }
        });
    })
}

/// Repaints after a new runtime is installed.
fn after_switch(cx: &mut App) {
    set_access_key_locale(cx);
    cx.refresh_windows();
    for window in cx.windows() {
        let _ = window.update(cx, |_, window, _| window.set_window_title(WINDOW_TITLE));
    }
    crate::menus::rebuild(cx);
}

/// Sets `AccessKeyLocale` from the installed UI language (dotted and dotless `i`).
fn set_access_key_locale(cx: &mut App) {
    let turkic = cx
        .try_global::<L10nGlobal>()
        .and_then(|g| g.l10n.snapshot().ui_chain.first().map(is_turkic))
        .unwrap_or(false);
    cx.set_global(AccessKeyLocale { turkic });
}

/// Whether `lang` is a Turkic language with the dotted/dotless `i` pairs: Turkish, Azerbaijani,
/// Crimean Tatar, Kazakh and Tatar. Decided by the language subtag, never by casing.
pub fn is_turkic(lang: &LanguageIdentifier) -> bool {
    matches!(lang.language.as_str(), "tr" | "az" | "crh" | "kk" | "tt")
}

/// The UI language: the head of the installed chain (`en` before any runtime is installed).
pub fn ui_language(cx: &App) -> Option<LanguageIdentifier> {
    cx.try_global::<L10nGlobal>()
        .and_then(|g| g.l10n.snapshot().ui_chain.first().cloned())
}

/// Renders `msg` for the UI; the message ID when no localization is installed.
pub fn tr<A: IntoFluentArgs>(cx: &App, msg: MsgRef<A>, args: A) -> SharedString {
    match cx.try_global::<L10nGlobal>() {
        Some(g) => g.l10n.t(msg, args).into(),
        None => SharedString::new_static(msg.id),
    }
}

/// Renders the message `id` (no arguments) for the UI; the ID when no localization is installed.
pub fn tr_id(cx: &App, id: &str) -> SharedString {
    match cx.try_global::<L10nGlobal>() {
        Some(g) => g.l10n.t_dynamic(id, Vec::new()).into(),
        None => SharedString::from(id.to_string()),
    }
}

/// A label closure for `msg`, re-read on every render (F16).
pub fn text_fn(msg: MsgRef<NoArgs>) -> TextFn {
    Rc::new(move |cx: &App| tr(cx, msg, NoArgs))
}

/// A label closure for the message `id`, re-read on every render (F16).
pub fn text_fn_id(id: &'static str) -> TextFn {
    Rc::new(move |cx: &App| tr_id(cx, id))
}

/// Resolves `hui-*` texts through the installed runtime's chain (R4).
pub struct L10nUiProvider {
    l10n: Arc<L10n>,
    /// Attribute lookups need `'static` IDs (`MsgRef`); the widget IDs are a small closed set,
    /// so each distinct one is interned once.
    interned: Mutex<HashMap<String, &'static str>>,
}

impl L10nUiProvider {
    pub fn new(l10n: Arc<L10n>) -> Self {
        Self {
            l10n,
            interned: Mutex::default(),
        }
    }

    fn intern(&self, id: &str) -> &'static str {
        let mut map = self
            .interned
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(found) = map.get(id) {
            return found;
        }
        let leaked: &'static str = Box::leak(id.to_string().into_boxed_str());
        map.insert(id.to_string(), leaked);
        leaked
    }
}

impl UiTextProvider for L10nUiProvider {
    fn text(&self, id: &str, args: &[(&'static str, UiArg)]) -> SharedString {
        let args = args
            .iter()
            .map(|(name, arg)| {
                let value = match arg {
                    UiArg::Count(n) => ArgValue::Count(*n),
                    UiArg::Text(t) => ArgValue::Text(t.to_string()),
                };
                (*name, value)
            })
            .collect();
        self.l10n.t_dynamic(id, args).into()
    }

    fn attr(&self, id: &str, attr: &str) -> Option<SharedString> {
        let msg: MsgRef<NoArgs> = MsgRef::new(self.intern(id));
        self.l10n.t_attr(msg, attr, NoArgs).map(Into::into)
    }

    fn generation(&self) -> u64 {
        self.l10n.generation()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turkic_languages_are_decided_by_the_language_subtag() {
        for tag in ["tr", "tr-TR", "az", "crh", "kk", "tt", "tr-XT"] {
            let lang: LanguageIdentifier = tag.parse().unwrap();
            assert!(is_turkic(&lang), "{tag}");
        }
        for tag in ["en", "de", "en-TR", "fr"] {
            let lang: LanguageIdentifier = tag.parse().unwrap();
            assert!(!is_turkic(&lang), "{tag}");
        }
    }
}
