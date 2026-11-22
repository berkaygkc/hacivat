//! Reload from disk and the T2 preview tier (HV-SCRIPT-08, -12; UI-EDIT-04 step 9, UI-EDIT-12,
//! UI-EDIT-23; R-M4b-16, R-M4b-38).
//!
//! ## m4b-t11-editor
//!
//! **Reload** ([`EditorView::replace_from_disk`]): the host read the file again and the tab is
//! clean (the editor has no notion of "dirty": the host compares versions and never calls it
//! on a tab with unsaved edits).
//! - **One change.** The disk text replaces the bytes between the common prefix and the
//!   common suffix ([`disk_edit`]), as one `Origin::Program` transaction outside the history,
//!   so carets, selections and anchors map through it (a line added above the caret moves it
//!   down, at its column); then the undo history is cleared (UI-EDIT-04 step 9). The change
//!   never starts or ends inside a UTF-8 sequence or between a CR and its LF, in either text.
//!   The same text again changes nothing (no version, the history kept).
//! - **Off the UI thread.** The prefix and suffix scan, the slice of new text and the longest
//!   line run on the background executor; the UI thread clones the rope (O(1)) and applies the
//!   one change. The answer applies only to the version it was computed for: an edit that
//!   landed first makes the tab dirty, and a dirty tab never reloads, so nothing is replaced
//!   (`EditorEvent::Reloaded { version: None }`; the host shows its "Changed on disk" bar). A
//!   newer reload replaces an older one still running.
//! - **Composition.** While an IME composition is open (or held app edits wait) the reload is
//!   held (`ime::AppEdit::ReplaceFromDisk`, like `set_text`) and starts when the composition
//!   ends, against the text as the composition left it; held reloads coalesce to the newest.
//!   `Ok(())` means accepted, so the host never retries. A diff that comes back while a
//!   composition is open is held the same way.
//! - **Refusals.** A T2 preview is reopened by the host, never replaced
//!   ([`ReloadRefused::PreviewTier`]); a disk text past T1's 20 MB is refused
//!   ([`ReloadRefused::TooLarge`]: nothing above 20 MB enters the editor, R-M3-49; the host
//!   opens such a file as a preview).
//! - **The host hears** `EditorEvent::Changed` (from the commit) and then
//!   `EditorEvent::Reloaded { version }`, the version at which the text is the disk text: its
//!   new clean version. The editor notifies itself (the commit's fan-out does), so a cached
//!   embedding (`a11y::embed`) draws the new text on the next frame.
//!
//! **T2 preview** ([`EditorView::set_preview`]): the editor holds the first part of a larger
//! file (UI-EDIT-23 step 1, cut by [`large_file::preview_cut`](crate::large_file::preview_cut)).
//! It becomes `Tier::Preview` for good: read-only (`set_config` cannot undo it), the T2 banner
//! with the file's and the preview's sizes (design/m4b-scripts surface 9) whose text is also
//! the accessible description, the status's tier, and find marked "preview only". The host
//! hears `TierChanged(Preview)`. A pending paste question and a running reload are dropped.

use std::ops::Range;
use std::sync::Arc;

use hacivat_text::{ChangeSet, Origin, TextBuffer, Transaction, UserEvent};
use hacivat_ui_core::gpui_surface::Context;

use crate::config::Tier;
use crate::ime::{self, AppEdit};
use crate::view::{EditorEvent, EditorView};

/// Why [`EditorView::replace_from_disk`] did not replace the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReloadRefused {
    /// A T2 preview is reopened by the host, never replaced in place.
    PreviewTier,
    /// The disk text is past T1's 20 MB (`TierLimits::reduced_max_bytes`): the host opens it
    /// as a preview (R-M3-49).
    TooLarge { bytes: u64 },
}

/// A T2 read-only preview (UI-EDIT-23 step 1): the first `shown_bytes` of a file of
/// `total_bytes`, cut at a line break.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PreviewSpec {
    pub shown_bytes: u64,
    pub total_bytes: u64,
}

/// The one change a reload makes: the bytes `old` of the editor's text become the bytes `new`
/// of the disk text. Both start at the same offset (the common prefix's end), and what follows
/// them is the common suffix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskEdit {
    pub old: Range<usize>,
    pub new: Range<usize>,
}

/// Bytes compared at a time before the byte-by-byte tail (a slice compare is a `memcmp`).
const BLOCK: usize = 256;

/// How many leading bytes `a` and `b` share.
fn common_prefix(a: &[u8], b: &[u8]) -> usize {
    let n = a.len().min(b.len());
    let mut i = 0;
    while i + BLOCK <= n && a[i..i + BLOCK] == b[i..i + BLOCK] {
        i += BLOCK;
    }
    while i < n && a[i] == b[i] {
        i += 1;
    }
    i
}

/// How many trailing bytes `a` and `b` share.
fn common_suffix(a: &[u8], b: &[u8]) -> usize {
    let n = a.len().min(b.len());
    let (la, lb) = (a.len(), b.len());
    let mut i = 0;
    while i + BLOCK <= n && a[la - i - BLOCK..la - i] == b[lb - i - BLOCK..lb - i] {
        i += BLOCK;
    }
    while i < n && a[la - i - 1] == b[lb - i - 1] {
        i += 1;
    }
    i
}

/// The change that turns `old` into `new` (`None` when they are equal): the bytes between
/// their common prefix and their common suffix. Neither end falls inside a UTF-8 sequence or
/// between a CR and its LF, in either text, so a line break is never split by the edit
/// (UI-EDIT-02 step 2). O(length), so it runs off the UI thread.
pub fn disk_edit(old: &str, new: &str) -> Option<DiskEdit> {
    if old == new {
        return None;
    }
    let (a, b) = (old.as_bytes(), new.as_bytes());
    let mut p = common_prefix(a, b);
    // The bytes before `p` are the same in both, so a boundary in one is a boundary in both.
    while !old.is_char_boundary(p) {
        p -= 1;
    }
    if p > 0 && a[p - 1] == b'\r' && (a.get(p) == Some(&b'\n') || b.get(p) == Some(&b'\n')) {
        p -= 1;
    }
    let mut s = common_suffix(&a[p..], &b[p..]);
    // The suffix's bytes are the same in both, so is its first byte's boundary.
    while !old.is_char_boundary(a.len() - s) {
        s -= 1;
    }
    let cr_before = |t: &[u8]| t.len() > s && t[t.len() - s - 1] == b'\r';
    if s > 0 && a[a.len() - s] == b'\n' && (cr_before(a) || cr_before(b)) {
        s -= 1;
    }
    Some(DiskEdit {
        old: p..a.len() - s,
        new: p..b.len() - s,
    })
}

/// What the background half hands back.
struct Landed {
    /// The whole disk text (kept to hold the reload again if a composition opened meanwhile).
    text: Arc<str>,
    /// The change and its new bytes; `None` when the texts are equal.
    edit: Option<(Range<usize>, Arc<str>)>,
    /// The disk text's longest line, in bytes (the tier's long-line rule).
    longest: u64,
}

/// The longest line of `text`, in bytes (as `EditorView::set_text` measures it).
fn longest_line(text: &str) -> u64 {
    text.split(['\n', '\r']).map(str::len).max().unwrap_or(0) as u64
}

/// The background half: `old` (the editor's text when the reload started) against `text`.
fn diff(old: &TextBuffer, text: Arc<str>) -> Landed {
    let current = old.slice(0..old.len_bytes());
    let edit = disk_edit(&current, &text).map(|e| (e.old, Arc::from(&text[e.new])));
    Landed {
        longest: longest_line(&text),
        edit,
        text,
    }
}

/// Starts the background diff of `text` against the current version.
fn start(v: &mut EditorView, text: Arc<str>, cx: &mut Context<EditorView>) {
    let version = v.model.version();
    let old = v.model.buffer().clone();
    let work = cx
        .background_executor()
        .spawn(async move { diff(&old, text) });
    v.reload = Some(cx.spawn(async move |this, cx| {
        let landed = work.await;
        let _ = this.update(cx, |v, cx| {
            v.reload = None;
            land(v, version, landed, cx);
        });
    }));
}

/// The UI half: applies `landed`, computed at `version`, when it is still current.
fn land(v: &mut EditorView, version: u64, landed: Landed, cx: &mut Context<EditorView>) {
    if v.tier.tier == Tier::Preview {
        cx.emit(EditorEvent::Reloaded { version: None });
        return;
    }
    // A composition opened while the diff ran: held like any app edit, then started again.
    if ime::defer_while_composing(v, AppEdit::ReplaceFromDisk(landed.text.to_string())) {
        return;
    }
    if v.model.version() != version {
        cx.emit(EditorEvent::Reloaded { version: None });
        return;
    }
    let Some((range, insert)) = landed.edit else {
        cx.emit(EditorEvent::Reloaded {
            version: Some(version),
        });
        return;
    };
    let len = v.model.buffer().len_bytes();
    let Ok(changes) = ChangeSet::from_edits(len, [(range, insert)]) else {
        cx.emit(EditorEvent::Reloaded { version: None });
        return;
    };
    // As `set_text` does: the longest line is the new text's, and the width is measured again.
    v.longest_line = landed.longest;
    v.scroll.known_width = 0.0;
    let mut ann = v.annotations(
        UserEvent::Command("reload_from_disk"),
        Origin::Program,
        cx,
    );
    // Outside the history (the stored groups are rebased, then dropped below).
    ann.add_to_history = false;
    let committed = v.commit(
        Transaction {
            changes,
            // The old selection mapped through the change.
            selection: None,
            ann,
        },
        cx,
    );
    v.model.clear_history();
    cx.emit(EditorEvent::Reloaded {
        version: committed.map(|c| c.version),
    });
    cx.notify();
}

impl EditorView {
    /// Replaces the text with `text`, read from disk again (R-M4b-38; the module docs): one
    /// change from the common prefix and suffix, computed off the UI thread, carets and
    /// selections mapped through it, the undo history cleared. Held while an IME composition is
    /// open and started when it ends (`Ok(())`: accepted). The host hears
    /// `EditorEvent::Reloaded` when it is done.
    pub fn replace_from_disk(
        &mut self,
        text: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), ReloadRefused> {
        if self.tier.tier == Tier::Preview {
            return Err(ReloadRefused::PreviewTier);
        }
        let bytes = text.len() as u64;
        if bytes > self.config.tiers.reduced_max_bytes {
            return Err(ReloadRefused::TooLarge { bytes });
        }
        // A newer reload replaces one still running.
        self.reload = None;
        if ime::defer_while_composing(self, AppEdit::ReplaceFromDisk(text.to_string())) {
            return Ok(());
        }
        start(self, Arc::from(text), cx);
        Ok(())
    }

    /// Shows the text as a T2 preview of a larger file (the module docs): read-only for good,
    /// the T2 banner and description, the tier in the status, find limited to the preview.
    pub fn set_preview(&mut self, spec: PreviewSpec, cx: &mut Context<Self>) {
        if self.reload.take().is_some() {
            cx.emit(EditorEvent::Reloaded { version: None });
        }
        self.paste.pending = None;
        self.config.read_only = true;
        // The host passes the tier on to the language service when it hears it, as for T1.
        let moved = crate::large_file::enter_preview(&mut self.tier, spec);
        if moved {
            cx.emit(EditorEvent::TierChanged(Tier::Preview));
        }
        cx.emit(EditorEvent::StatusChanged);
        cx.notify();
    }

    /// The preview this editor shows (`None` unless `set_preview` made it one).
    pub fn preview(&self) -> Option<PreviewSpec> {
        self.tier.preview
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;
    use std::time::Instant;

    use gpui::{
        AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _,
        TestAppContext, VisualTestContext, Window, div,
    };

    use super::*;
    use crate::config::{DocId, EditorConfig};
    use crate::service::FailClosedService;

    struct Host {
        editor: Entity<EditorView>,
    }

    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(self.editor.clone())
        }
    }

    fn editor_with(
        text: String,
        cx: &mut TestAppContext,
    ) -> (Entity<EditorView>, &mut VisualTestContext) {
        let slot: Rc<std::cell::RefCell<Option<Entity<EditorView>>>> = Rc::default();
        let out = slot.clone();
        let (_, vcx) = cx.add_window_view(move |window, cx| {
            let editor = cx.new(|cx| {
                EditorView::new(
                    DocId(1),
                    &text,
                    EditorConfig::default(),
                    Rc::new(FailClosedService),
                    Rc::new(|_: &gpui::App| "orders.sql".into()),
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

    /// 2 MB of short SQL lines.
    fn two_mb() -> String {
        let line = "SELECT id, total FROM orders WHERE id = 1;\n";
        line.repeat((2 << 20) / line.len() + 1)
    }

    /// The UI thread's share of a reload of a 2 MB script with one line changed in its middle:
    /// the call (a rope clone and a spawn) and the apply of the landed diff (one change and its
    /// fan-out); the diff itself is the background executor's. Under 1 ms (perf-gated: a
    /// wall-clock check, `HACIVAT_PERF=1`, release build).
    #[gpui::test]
    #[ignore = "perf"]
    fn reload_ui_share_under_1ms_on_2mb(cx: &mut TestAppContext) {
        if std::env::var_os("HACIVAT_PERF").is_none() {
            return;
        }
        let text = two_mb();
        let (e, vcx) = editor_with(text.clone(), cx);
        vcx.run_until_parked();
        let mid = text.len() / 2;
        let line = text[..mid].rfind('\n').unwrap() + 1;
        let mut disk = text.clone();
        disk.replace_range(line..line + 6, "UPDATE");
        let mut worst = std::time::Duration::ZERO;
        for round in 0..20 {
            let disk = if round % 2 == 0 { &disk } else { &text };
            let t = Instant::now();
            e.update(vcx, |v, cx| v.replace_from_disk(disk, cx)).unwrap();
            let call = t.elapsed();
            // The background half, outside the measure.
            let (version, landed) = vcx.read(|cx| {
                let v = e.read(cx);
                (v.model.version(), diff(v.model.buffer(), Arc::from(disk.as_str())))
            });
            e.update(vcx, |v, _| v.reload = None);
            let t = Instant::now();
            e.update(vcx, |v, cx| land(v, version, landed, cx));
            let apply = t.elapsed();
            vcx.run_until_parked();
            worst = worst.max(call + apply);
            assert_eq!(
                vcx.read(|cx| e.read(cx).snapshot().buffer.len_bytes()),
                disk.len()
            );
        }
        println!("HV-FACT m4b-t11 reload_ui_share_2mb_worst_us={}", worst.as_micros());
        assert!(worst.as_micros() < 1_000, "{worst:?}");
    }

    #[test]
    fn the_scans_agree_with_a_byte_by_byte_compare() {
        let samples = [
            ("", ""),
            ("a", ""),
            ("abc", "abd"),
            ("xabc", "abc"),
            (&"q".repeat(1_000)[..], &"q".repeat(999)[..]),
        ];
        for (a, b) in samples {
            let (a, b) = (a.as_bytes(), b.as_bytes());
            let p = a.iter().zip(b).take_while(|(x, y)| x == y).count();
            let s = a.iter().rev().zip(b.iter().rev()).take_while(|(x, y)| x == y).count();
            assert_eq!(common_prefix(a, b), p);
            assert_eq!(common_suffix(a, b), s);
        }
        // Across a block edge: a difference at byte 300 of 600.
        let a = "x".repeat(600);
        let mut b = a.clone();
        b.replace_range(300..301, "y");
        assert_eq!(common_prefix(a.as_bytes(), b.as_bytes()), 300);
        assert_eq!(common_suffix(a.as_bytes(), b.as_bytes()), 299);
    }
}
