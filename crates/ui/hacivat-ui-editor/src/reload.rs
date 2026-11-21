//! Reload from disk and the T2 preview tier (HV-SCRIPT-08, -12; UI-EDIT-04 step 9, UI-EDIT-12,
//! UI-EDIT-23; R-M4b-38).
//!
//! ## m4b-t01b-seams: the stubs M4b's script tabs code against.
//! - [`EditorView::replace_from_disk`] refuses ([`ReloadRefused::NotImplemented`]) and leaves
//!   the text as it is, so a script tab shows its "Changed on disk" bar instead of reloading
//!   (Global Constraints, "No silent weakening"). Task 11 fills it: one `Origin::Program` change
//!   from the common prefix and suffix, carets and selections mapped through it, undo history
//!   cleared, held during an IME composition (`ime::AppEdit::ReplaceFromDisk`, like
//!   `set_text`); it refuses only [`ReloadRefused::PreviewTier`]. The editor has no notion of
//!   "dirty": the host never calls it on a tab with unsaved edits.
//! - [`EditorView::set_preview`] does nothing yet (Task 11: `Tier::Preview`, read-only, the
//!   banner and the tier segment).

use hacivat_ui_core::gpui_surface::Context;

use crate::view::EditorView;

/// Why [`EditorView::replace_from_disk`] did not replace the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReloadRefused {
    /// The stub (until Task 11).
    NotImplemented,
    /// A T2 preview is reopened by the host, never replaced in place.
    PreviewTier,
}

/// A T2 read-only preview (UI-EDIT-23 step 1): the first `shown_bytes` of a file of
/// `total_bytes`, cut at a line break.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreviewSpec {
    pub shown_bytes: u64,
    pub total_bytes: u64,
}

impl EditorView {
    /// Replaces the text with `text`, as read from disk again (R-M4b-38). Stub: refuses and
    /// leaves the text unchanged.
    pub fn replace_from_disk(
        &mut self,
        text: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), ReloadRefused> {
        let _ = (text, cx);
        Err(ReloadRefused::NotImplemented)
    }

    /// Shows the text as a T2 preview of a larger file. Stub: nothing changes.
    pub fn set_preview(&mut self, spec: PreviewSpec, cx: &mut Context<Self>) {
        let _ = (spec, cx);
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use gpui::{
        AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _,
        TestAppContext, VisualTestContext, Window, div,
    };

    use super::*;
    use crate::config::{DocId, EditorConfig, Tier};
    use crate::service::FailClosedService;

    struct Host {
        editor: Entity<EditorView>,
    }

    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(self.editor.clone())
        }
    }

    fn editor_with<'a>(
        text: &'static str,
        cx: &'a mut TestAppContext,
    ) -> (Entity<EditorView>, &'a mut VisualTestContext) {
        let slot: Rc<std::cell::RefCell<Option<Entity<EditorView>>>> = Rc::default();
        let out = slot.clone();
        let (_, vcx) = cx.add_window_view(move |window, cx| {
            let editor = cx.new(|cx| {
                EditorView::new(
                    DocId(1),
                    text,
                    EditorConfig::default(),
                    Rc::new(FailClosedService),
                    Rc::new(|_: &gpui::App| "Console 1".into()),
                    window,
                    cx,
                )
            });
            *out.borrow_mut() = Some(editor.clone());
            Host { editor }
        });
        let editor = slot.borrow().clone().expect("the editor");
        (editor, vcx)
    }

    fn text_of(e: &Entity<EditorView>, vcx: &VisualTestContext) -> String {
        vcx.read(|cx| e.read(cx).snapshot().buffer.to_string())
    }

    #[gpui::test]
    fn replace_from_disk_stub_refuses(cx: &mut TestAppContext) {
        let (e, vcx) = editor_with("a", cx);
        let version = vcx.read(|cx| e.read(cx).snapshot().version);
        let got = e.update(vcx, |e, cx| e.replace_from_disk("b", cx));
        assert_eq!(got, Err(ReloadRefused::NotImplemented));
        assert_eq!(text_of(&e, vcx), "a");
        assert_eq!(vcx.read(|cx| e.read(cx).snapshot().version), version);
    }

    #[gpui::test]
    fn set_preview_stub_changes_nothing(cx: &mut TestAppContext) {
        let (e, vcx) = editor_with("SELECT 1;", cx);
        e.update(vcx, |e, cx| {
            e.set_preview(
                PreviewSpec {
                    shown_bytes: 9,
                    total_bytes: 30 << 20,
                },
                cx,
            )
        });
        assert_eq!(text_of(&e, vcx), "SELECT 1;");
        assert_ne!(vcx.read(|cx| e.read(cx).status().tier), Tier::Preview);
    }
}
