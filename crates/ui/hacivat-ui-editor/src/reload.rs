//! Reload from disk and the T2 preview tier (HV-SCRIPT-08, -12; UI-EDIT-04 step 9, UI-EDIT-12,
//! UI-EDIT-23; R-M4b-16, R-M4b-38 as amended by ruling R-T11-1).
//!
//! ## m4b-t11-editor
//!
//! **Reload** ([`EditorView::replace_from_disk`]): the host read the file again. It passes the
//! version it last loaded, reloaded or saved (its clean version); the editor itself refuses
//! when its text is at any other version (`ReloadRefused::Edited`), so no host can overwrite
//! unsaved edits through this API (R-T11-1).
//! - **One change.** The disk text replaces the bytes between the common prefix and the
//!   common suffix ([`disk_edit`]), as one `Origin::Program` transaction outside the history,
//!   after the history is cleared (UI-EDIT-04 step 9). Anchors and marks map through it (a line
//!   added above the caret moves it down, at its column); a caret or selection end inside the
//!   change keeps its offset, clamped into the new bytes (so a file changed at both ends keeps
//!   the caret where the user was). The change never starts or ends inside a UTF-8 sequence or
//!   between a CR and its LF, in either text. The same text again changes nothing (no version,
//!   the history kept).
//! - **Off the UI thread.** The prefix and suffix scan, the new text (the rope clone with the
//!   change applied) and the longest line are the background executor's; the UI thread moves an
//!   `Arc`, clones the rope (O(1)) and, at the landing, swaps the new text in
//!   (`EditorModel::dispatch_prebuilt`): its work does not grow with the change (review I2;
//!   `EditorView::ui_work` counts it).
//! - **Never over unsaved text (review C1).** The landing checks the version first: anything
//!   that changed the text since the call (typing, a composition's marked text, a held paste)
//!   drops the reload (`EditorEvent::Reloaded { version: None }`; the host shows its "Changed on
//!   disk" bar). Only while the text is still clean, an empty composition or held app edits make
//!   it wait (`ime::AppEdit::ReplaceFromDisk`, with the version), and the flush drops it if they
//!   changed the text. A newer reload replaces an older one.
//! - **Refusals.** A T2 preview is reopened by the host, never replaced
//!   ([`ReloadRefused::PreviewTier`]); a disk text past T1's 20 MB is refused
//!   ([`ReloadRefused::TooLarge`]: nothing above 20 MB enters the editor, R-M3-49); a text not at
//!   the clean version is refused ([`ReloadRefused::Edited`]).
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
//! hears `TierChanged(Preview)`. A pending paste question and a running reload are dropped, and
//! an open composition is discarded. Its text changes only through
//! [`EditorView::reload_preview`]: `set_text` leaves a preview alone.

use std::ops::Range;
use std::sync::Arc;

use hacivat_text::{
    ChangeSet, Origin, SelectionRange, SelectionSet, TextBuffer, Transaction, UserEvent,
};
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
    /// as a preview (R-M3-49). For `reload_preview`: past `TierLimits::preview_bytes`.
    TooLarge { bytes: u64 },
    // ## m4b-t11-editor (fix round 1, R-T11-1)
    /// The text is at `version`, not the host's clean version: it has unsaved edits (a
    /// composition's text included), which a reload never overwrites. The host shows its
    /// "Changed on disk" bar.
    Edited { version: u64 },
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

/// The change a reload makes, built off the UI thread.
struct Plan {
    edit: DiskEdit,
    changes: ChangeSet,
    /// The editor's text with `changes` applied: swapped in at the landing
    /// (`EditorModel::dispatch_prebuilt`), so the UI thread never edits the rope.
    after: TextBuffer,
}

/// What the background half hands back.
struct Landed {
    /// The whole disk text (kept to hold the reload if an empty composition is open).
    text: Arc<str>,
    /// `None` with `identical`: the texts are equal; `None` without: no change could be built
    /// (fail closed).
    plan: Option<Plan>,
    identical: bool,
    /// The disk text's longest line, in bytes (the tier's long-line rule).
    longest: u64,
}

/// The longest line of `text`, in bytes (as `EditorView::set_text` measures it).
fn longest_line(text: &str) -> u64 {
    text.split(['\n', '\r']).map(str::len).max().unwrap_or(0) as u64
}

/// The background half: `old` (the editor's text when the reload started) against `text`: the
/// edit, its change set and the text it makes, and the longest line. O(document), so never on
/// the UI thread.
fn diff(old: TextBuffer, text: Arc<str>) -> Landed {
    let longest = longest_line(&text);
    let current = old.slice(0..old.len_bytes());
    let Some(edit) = disk_edit(&current, &text) else {
        return Landed {
            text,
            plan: None,
            identical: true,
            longest,
        };
    };
    drop(current);
    let insert: Arc<str> = Arc::from(&text[edit.new.clone()]);
    let plan = ChangeSet::from_edits(old.len_bytes(), [(edit.old.clone(), insert)])
        .ok()
        .map(|changes| {
            let mut after = old;
            changes.apply(&mut after);
            Plan {
                edit,
                changes,
                after,
            }
        });
    Landed {
        text,
        plan,
        identical: false,
        longest,
    }
}

/// Starts the background diff of `text` against the current version.
fn start(v: &mut EditorView, text: Arc<str>, cx: &mut Context<EditorView>) {
    let version = v.model.version();
    let old = v.model.buffer().clone();
    let work = cx
        .background_executor()
        .spawn(async move { diff(old, text) });
    v.reload = Some(cx.spawn(async move |this, cx| {
        let landed = work.await;
        let _ = this.update(cx, |v, cx| {
            v.reload = None;
            land(v, version, landed, cx);
        });
    }));
}

/// Where byte `at` of the old text lands in the new one (m5): before the change it stays,
/// after it it moves with the change's end, inside it it keeps its offset, clamped into the
/// new bytes (the model then snaps it onto a character boundary, out of any CRLF), so a caret
/// in the middle of a file whose both ends changed stays where the user was reading.
fn map_into(at: usize, e: &DiskEdit) -> usize {
    if at <= e.old.start {
        at
    } else if at >= e.old.end {
        at - e.old.end + e.new.end
    } else {
        at.min(e.new.end)
    }
}

/// The UI half: applies `landed`, computed at `version`, only while the text is still that
/// version. O(sections + carets): the new text was built off the UI thread and is swapped in.
fn land(v: &mut EditorView, version: u64, landed: Landed, cx: &mut Context<EditorView>) {
    if v.tier.tier == Tier::Preview {
        cx.emit(EditorEvent::Reloaded { version: None });
        return;
    }
    // ## m4b-t11-editor (fix round 1, C1): the version first. Anything that changed the text
    // since the call (typing, a composition's marked text, a held paste) made the tab dirty,
    // and a dirty tab never reloads.
    if v.model.version() != version {
        cx.emit(EditorEvent::Reloaded { version: None });
        return;
    }
    // The text is the clean one, but a composition (with no text yet) or held edits wait: the
    // reload waits behind them and is dropped at the flush if they changed the text.
    if ime::defer_while_composing(
        v,
        AppEdit::ReplaceFromDisk {
            text: landed.text.clone(),
            version,
        },
    ) {
        return;
    }
    let plan = match landed.plan {
        Some(plan) => plan,
        None => {
            let version = landed.identical.then_some(version);
            cx.emit(EditorEvent::Reloaded { version });
            return;
        }
    };
    // ## m4b-t11-editor (fix round 1, m4): the history goes first, so the commit has no group
    // to rebase.
    v.model.clear_history();
    // As `set_text` does: the longest line is the new text's (measured off the UI thread, so
    // the fan-out does not scan the change, I2), and the width is measured again.
    v.longest_line = landed.longest;
    v.longest_measured = true;
    v.scroll.known_width = 0.0;
    let sel = v.model.selection();
    let ranges: Vec<SelectionRange> = sel
        .ranges()
        .iter()
        .map(|r| SelectionRange {
            anchor: map_into(r.anchor, &plan.edit),
            head: map_into(r.head, &plan.edit),
            goal_x: None,
            assoc: r.assoc,
        })
        .collect();
    let selection = SelectionSet::normalized(ranges, sel.main_index());
    let mut ann = v.annotations(UserEvent::Command("reload_from_disk"), Origin::Program, cx);
    ann.add_to_history = false;
    let committed = v.commit_prebuilt(
        Transaction {
            changes: plan.changes,
            selection: Some(selection),
            ann,
        },
        plan.after,
        cx,
    );
    v.longest_measured = false;
    cx.emit(EditorEvent::Reloaded {
        version: committed.map(|c| c.version),
    });
    cx.notify();
}

impl EditorView {
    /// Replaces the text with `text`, read from disk again (R-M4b-38, ruling R-T11-1; the module
    /// docs), when the text is still the host's clean version `clean_version` (the version
    /// after its load, reload or save): one change from the common prefix and suffix, built off
    /// the UI thread, carets and selections mapped through it, the undo history cleared.
    /// Refused (`Edited`) when the text is at another version, so unsaved edits, a composition's
    /// text included, are never overwritten. Held while an empty composition or held edits wait,
    /// and dropped when they change the text. `Ok(())`: accepted; the host hears
    /// `EditorEvent::Reloaded` when it is done.
    pub fn replace_from_disk(
        &mut self,
        text: impl Into<Arc<str>>,
        clean_version: u64,
        cx: &mut Context<Self>,
    ) -> Result<(), ReloadRefused> {
        if self.tier.tier == Tier::Preview {
            return Err(ReloadRefused::PreviewTier);
        }
        let text: Arc<str> = text.into();
        let bytes = text.len() as u64;
        if bytes > self.config.tiers.reduced_max_bytes {
            return Err(ReloadRefused::TooLarge { bytes });
        }
        let version = self.model.version();
        if version != clean_version {
            return Err(ReloadRefused::Edited { version });
        }
        // A newer reload replaces one still running.
        self.reload = None;
        if ime::defer_while_composing(
            self,
            AppEdit::ReplaceFromDisk {
                text: text.clone(),
                version,
            },
        ) {
            return Ok(());
        }
        start(self, text, cx);
        Ok(())
    }

    /// Shows the text as a T2 preview of a larger file (the module docs): read-only for good,
    /// the T2 banner and description, the tier in the status, find limited to the preview. An
    /// open composition is discarded, not committed (m6): its text is not the file's.
    pub fn set_preview(&mut self, spec: PreviewSpec, cx: &mut Context<Self>) {
        if self.reload.take().is_some() {
            cx.emit(EditorEvent::Reloaded { version: None });
        }
        self.paste.pending = None;
        ime::discard_composition(self, cx);
        self.config.read_only = true;
        // The host passes the tier on to the language service when it hears it, as for T1.
        let moved = crate::large_file::enter_preview(&mut self.tier, spec);
        if moved {
            cx.emit(EditorEvent::TierChanged(Tier::Preview));
        }
        cx.emit(EditorEvent::StatusChanged);
        cx.notify();
    }

    /// Replaces a preview's text and sizes (the host read the file's first part again): the one
    /// way to change a preview's text (`set_text` leaves a preview alone, m1). Refused past
    /// `TierLimits::preview_bytes`; it makes the editor a preview when it was not one.
    pub fn reload_preview(
        &mut self,
        text: &str,
        spec: PreviewSpec,
        cx: &mut Context<Self>,
    ) -> Result<(), ReloadRefused> {
        let bytes = text.len() as u64;
        if bytes > self.config.tiers.preview_bytes {
            return Err(ReloadRefused::TooLarge { bytes });
        }
        self.preview_text_allowed = true;
        self.set_text(text, cx);
        self.preview_text_allowed = false;
        self.model.clear_history();
        self.set_preview(spec, cx);
        Ok(())
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

    /// The UI thread's share of reloading a 2 MB script: the call (an `Arc` move, a rope clone
    /// and a spawn) and the landing (the swap and its fan-out); the diff and the new text are
    /// the background executor's. Two cases: one line changed in the middle, and the first and
    /// last bytes changed (one change over the whole file, review I2). Under 1 ms each
    /// (a wall-clock check: `--ignored`, `HACIVAT_PERF=1`, release build; without the variable
    /// it says so and checks nothing, review m3). The bound on the work itself is
    /// `the_landing_never_edits_or_scans_the_change` (always on).
    #[gpui::test]
    #[ignore = "perf"]
    fn reload_ui_share_under_1ms_on_2mb(cx: &mut TestAppContext) {
        if std::env::var_os("HACIVAT_PERF").is_none() {
            println!("HV-FACT m4b-t11 reload_ui_share=SKIP reason=HACIVAT_PERF_unset");
            return;
        }
        let text = two_mb();
        let (e, vcx) = editor_with(text.clone(), cx);
        vcx.run_until_parked();
        let mid = text.len() / 2;
        let line = text[..mid].rfind('\n').unwrap() + 1;
        let mut middle = text.clone();
        middle.replace_range(line..line + 6, "UPDATE");
        let mut ends = text.clone();
        ends.replace_range(0..6, "UPDATE");
        let n = ends.len();
        ends.replace_range(n - 2..n - 1, ")");
        for (case, disk) in [("middle", middle), ("both_ends", ends)] {
            let mut worst = std::time::Duration::ZERO;
            for round in 0..20 {
                let disk: Arc<str> = Arc::from(if round % 2 == 0 { &disk } else { &text }.as_str());
                let clean = vcx.read(|cx| e.read(cx).model.version());
                let t = Instant::now();
                e.update(vcx, |v, cx| v.replace_from_disk(disk.clone(), clean, cx))
                    .unwrap();
                let call = t.elapsed();
                // The background half, outside the measure.
                let landed = vcx.read(|cx| diff(e.read(cx).model.buffer().clone(), disk.clone()));
                e.update(vcx, |v, _| v.reload = None);
                let t = Instant::now();
                e.update(vcx, |v, cx| land(v, clean, landed, cx));
                let apply = t.elapsed();
                vcx.run_until_parked();
                worst = worst.max(call + apply);
                assert_eq!(
                    vcx.read(|cx| e.read(cx).snapshot().buffer.len_bytes()),
                    disk.len()
                );
            }
            println!(
                "HV-FACT m4b-t11 reload_ui_share_2mb case={case} worst_us={}",
                worst.as_micros()
            );
            assert!(worst.as_micros() < 1_000, "{case}: {worst:?}");
        }
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
            let s = a
                .iter()
                .rev()
                .zip(b.iter().rev())
                .take_while(|(x, y)| x == y)
                .count();
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

    #[test]
    fn a_position_inside_the_change_keeps_its_offset() {
        let e = DiskEdit {
            old: 10..100,
            new: 10..50,
        };
        assert_eq!(map_into(5, &e), 5);
        assert_eq!(map_into(10, &e), 10);
        assert_eq!(map_into(30, &e), 30);
        assert_eq!(map_into(70, &e), 50, "clamped into the new bytes");
        assert_eq!(map_into(100, &e), 50);
        assert_eq!(map_into(120, &e), 70);
    }
}
