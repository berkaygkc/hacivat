//! UI text without a core dependency (R4): widgets resolve `hui-*` IDs through the
//! [`UiTextProvider`] installed as the [`UiText`] global; the app installs one backed by the
//! Fluent runtime (Task 13). [`EnglishDefaults`] serves `locales/en/hui.ftl` when no app is
//! present (Task 11).

use std::sync::Arc;

use hacivat_ui_core::gpui_surface::{App, Global, SharedString};

/// Renders UI messages by ID.
pub trait UiTextProvider: Send + Sync + 'static {
    fn text(&self, id: &str, args: &[(&'static str, UiArg)]) -> SharedString;
    fn attr(&self, id: &str, attr: &str) -> Option<SharedString>;
    /// Changes whenever the rendered texts may change (a language switch).
    fn generation(&self) -> u64;
}

/// A message argument.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum UiArg {
    Count(u64),
    Text(SharedString),
}

/// The installed UI text provider.
#[derive(Clone)]
pub struct UiText(pub Arc<dyn UiTextProvider>);

impl std::fmt::Debug for UiText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UiText")
            .field("generation", &self.0.generation())
            .finish()
    }
}

impl Global for UiText {}

/// The message `id` from the installed provider, or `id` itself when none is installed.
pub fn ui_text(cx: &App, id: &str) -> SharedString {
    match cx.try_global::<UiText>() {
        Some(provider) => provider.0.text(id, &[]),
        None => SharedString::from(id.to_string()),
    }
}

/// English `hui-*` texts embedded from `locales/en/hui.ftl`. Stub until Task 11: renders the ID.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EnglishDefaults {
    _private: (),
}

impl EnglishDefaults {
    pub fn new() -> Self {
        Self::default()
    }
}

impl UiTextProvider for EnglishDefaults {
    fn text(&self, id: &str, args: &[(&'static str, UiArg)]) -> SharedString {
        let _ = args;
        SharedString::from(id.to_string())
    }

    fn attr(&self, id: &str, attr: &str) -> Option<SharedString> {
        let _ = (id, attr);
        None
    }

    fn generation(&self) -> u64 {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    struct Fixed;

    impl UiTextProvider for Fixed {
        fn text(&self, id: &str, _: &[(&'static str, UiArg)]) -> SharedString {
            SharedString::from(format!("<{id}>"))
        }
        fn attr(&self, _: &str, _: &str) -> Option<SharedString> {
            None
        }
        fn generation(&self) -> u64 {
            1
        }
    }

    #[gpui::test]
    fn ui_text_without_provider_returns_the_id(cx: &mut TestAppContext) {
        cx.update(|cx| assert_eq!(ui_text(cx, "hui-x"), "hui-x"));
    }

    #[gpui::test]
    fn ui_text_uses_the_installed_provider(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(UiText(Arc::new(Fixed)));
            assert_eq!(ui_text(cx, "hui-x"), "<hui-x>");
        });
    }
}
