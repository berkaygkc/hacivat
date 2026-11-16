//! The dialog's URL row: display, edit and apply (NOTES surface 2, Task 17).
//!
//! The row mirrors the form live (the form and the URL are one object seen two ways): it shows
//! the profile's Display URL, with a mask where a password goes and never the password itself.
//! While the user edits the row it is left alone; Enter, or leaving the row with a changed text,
//! applies the text as a paste. "Copy as URL" lives in the dialog (`copy_as_url`, ⌘⇧C).

use hacivat_secrets::AuthProviderId;
use hacivat_session::profile::display_url;
use hacivat_ui_core::gpui_surface::{Context, Focusable as _, Window};

use super::ConnectionDialog;
use super::form::Field;

impl ConnectionDialog {
    /// The URL row's mirror of the form: the Display URL, masked where a password is (typed,
    /// pasted, or kept in the keyring); empty while the profile cannot be written as a URL.
    pub(crate) fn mirrored_url(&self) -> String {
        let p = &self.draft.profile;
        let has_password = self.draft.password.is_some()
            || (self.has_stored_password && p.auth == AuthProviderId::Password);
        display_url(p, has_password).unwrap_or_default()
    }

    /// Mirrors the form into the URL row, unless the user is editing the row.
    pub(crate) fn sync_url(&mut self, window: &Window, cx: &mut Context<Self>) {
        let focused = self.inputs[&Field::Url]
            .read(cx)
            .focus_handle(cx)
            .is_focused(window);
        if !focused {
            self.write_url(cx);
        }
    }

    /// Writes the mirror into the URL row.
    pub(crate) fn write_url(&mut self, cx: &mut Context<Self>) {
        let text = self.mirrored_url();
        self.inputs[&Field::Url].update(cx, |input, cx| {
            if input.value() != text {
                input.set_value(text);
                cx.notify();
            }
        });
    }

    /// The row's text was submitted, or the row lost the focus: a text that differs from the
    /// mirror is applied as a paste (the mock's `change`), keeping the secrets the mask stands
    /// for.
    pub(crate) fn apply_url_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.inputs[&Field::Url].read(cx).value().to_string();
        if text.trim().is_empty() || text == self.mirrored_url() {
            return;
        }
        // The row's `****` stands for the current secret, never a new one.
        self.apply_row_text(&text, window, cx);
    }
}
