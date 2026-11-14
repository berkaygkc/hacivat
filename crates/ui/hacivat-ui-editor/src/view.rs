//! `EditorView` (UI-EDIT-01 view; Task 19): the model, the IME core, the language service and
//! its decoration sources, the module-owned state (P11), the shape cache, the scroll position,
//! the caret blink, the probe, and the public API the console host and the execution glue use.
//!
//! Every committed transaction notifies the service, the marks, the tier, the preview and the
//! input handler, and emits `EditorEvent::Changed` (or `SelectionChanged`). Task 19 adds to
//! that one fan-out ([`EditorView::apply_committed`]): the probe's input mark, the scroll
//! anchor's rebase (an edit above the view keeps the content still), the caret reveal (only
//! when the commit moved the main caret or edited at it), the longest line for the tier, and
//! the blink's restart. Edits that are not the IME's own (`set_text`, `select_range`, a click,
//! every `commit`) go through `ImeCore::dispatch`, which commits an open composition first, so
//! they never join its undo group (UI-EDIT-12 step 9). Transactions are stamped with the GPUI
//! executor's clock. The view is embeddable as `AnyView::cached(..)`: nothing outside it
//! re-renders when it is notified, and its layout, hit targets, cursor style, pointer
//! listeners and input handler survive frames where it is reused (`embeds_as_a_cached_view`).
//! **Its accessibility node may not:** the GPUI fork's `reuse_prepaint`/`reuse_paint` carry no
//! accessibility state, so on a frame where the cached editor is not prepainted again its node
//! may be missing from the tree. Task 29 (the node's bodies) and Task 32 (the console host's
//! `cached()` embedding) own the test and the fix (review round 1).
//!
//! Call points every module seam already has (review rounds 1 and 2, P11), so Tasks 25 and
//! 27–29 never edit this file: focus and blur (`ime::on_focus`/`on_blur`, `FocusedEditor`);
//! every `Committed`, whatever produced it ([`EditorView::apply_committed`]: a transaction, an
//! undo or redo, an IME mark or commit) → `marks::on_commit`, `LanguageService::change`,
//! `large_file::update`, `preview::on_update`, `ime::after_commit` and the events; a fresh
//! analysis (`LanguageService::on_update` → `preview::on_update`); a composition opening or
//! closing ([`EditorView::composition_changed`] → `preview::on_composition`); the banner
//! (`large_file::banner`) and the commands' overlay (`commands::overlay`); each frame
//! (`element::EditorElement`: the a11y hooks, `prepaint_frame`, `paint_frame`); the commands'
//! state (`commands::CommandState`).

use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use hacivat_text::display::HeightMap;
use hacivat_text::{
    Annotations, Bias, ChangeSet, Committed, EditorModel, ImeCore, MapMode, ModelConfig, Origin,
    SelectionRange, SelectionSet, Snapshot, TextBuffer, Transaction, UserEvent,
};
use hacivat_ui::text::TextFn;
use hacivat_ui_core::gpui_surface::{
    App, Bounds, Context, EventEmitter, FocusHandle, Focusable, Global, InteractiveElement,
    IntoElement, ParentElement, Pixels, Point, Render, SharedString, Styled, Subscription,
    WeakEntity, Window, div,
};
use smallvec::SmallVec;

use crate::a11y::{self, A11yFrame, A11yState};
use crate::clipboard::PasteState;
use crate::commands::{self, CommandState, command_by_id};
use crate::config::{DocContext, DocId, EditorConfig, Tier};
use crate::element::{EditorElement, FrameLayout};
use crate::ime::{self, InputState};
use crate::large_file::{self, TierState};
use crate::layout_cache::{LayoutCache, LayoutStats};
use crate::marks::{self, ExecMarkState, MarkKey, MarkStore};
use crate::mouse::DragState;
use crate::paint::{BLINK_HALF, Blink};
use crate::preview::{self, BandState, Refresh};
use crate::probe::FrameProbe;
use crate::scroll::ScrollState;
use crate::service::{DecorationSource, LanguageService};

/// What the status bar's editor segment shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EditorStatus {
    /// 1-based.
    pub line: u32,
    /// 1-based, in graphemes.
    pub column: u32,
    pub selected_chars: u64,
    pub tier: Tier,
    pub composing: bool,
}

/// What an editor tells its host.
#[derive(Debug, Clone)]
pub enum EditorEvent {
    Changed {
        version: u64,
    },
    SelectionChanged {
        version: u64,
    },
    /// Run, Run 2 or Run 3 (Task 25 emits it; the app resolves and runs it).
    ExecuteRequested(crate::service::ExecRequest),
    CompositionChanged(bool),
    StatusChanged,
    TierChanged(Tier),
    /// A paste would take the document past 20 MB (R-M3-49).
    PasteRefused {
        bytes: u64,
    },
    /// The context menu opened (`editor.context_menu`, Shift+F10 at the caret, or a right click
    /// at the pointer; Task 25 emits it and draws the menu in `commands::overlay`). The host
    /// closes its own popovers (Task 32; nothing in Task 1b).
    ContextMenuOpened {
        at: Point<Pixels>,
    },
}

/// The focused editor, for the app's commands (`enabled` = an editor is focused).
#[derive(Debug, Clone, Default)]
pub struct FocusedEditor(pub Option<WeakEntity<EditorView>>);

impl Global for FocusedEditor {}

/// One editor.
pub struct EditorView {
    pub(crate) doc: DocId,
    pub(crate) model: EditorModel,
    pub(crate) ime: ImeCore,
    pub(crate) config: EditorConfig,
    pub(crate) service: Rc<dyn LanguageService>,
    pub(crate) a11y_label: TextFn,
    pub(crate) focus: FocusHandle,
    pub(crate) ctx: DocContext,
    pub(crate) marks: MarkStore,
    pub(crate) band: BandState,
    pub(crate) a11y: A11yState,
    pub(crate) tier: TierState,
    pub(crate) paste: PasteState,
    pub(crate) probe: Option<Rc<dyn FrameProbe>>,
    pub(crate) layout: Option<FrameLayout>,
    // ## m3a-t01b-seams (fix round 1): module-owned state of the input handler (Task 27) and
    // the commands (Task 25), and the focus subscriptions.
    pub(crate) input: InputState,
    pub(crate) commands: CommandState,
    _subscriptions: Vec<Subscription>,
    // ## m3a-t01b-seams (round 2): the label as last evaluated, with the UI-text generation it
    // was evaluated at (re-read only when the language changes).
    a11y_label_cache: Option<(u64, SharedString)>,
    /// The text version the fan-out last saw: a `Committed` at the same version changed the
    /// selection only (Task 5's model does not bump the version for those).
    fanned_version: u64,
    // ## m3a-t19-editor-view: the element's state.
    /// The service's decoration sources (highlighting), queried per frame for the visible
    /// bytes only.
    pub(crate) sources: SmallVec<[Rc<dyn DecorationSource>; 2]>,
    pub(crate) cache: LayoutCache,
    pub(crate) scroll: ScrollState,
    pub(crate) blink: Blink,
    pub(crate) drag: Option<DragState>,
    /// The editor has the focus (the blink runs only then).
    has_focus: bool,
    /// The longest line seen, in bytes (the tier's long-line rule). Edits only grow it (a
    /// shortened longest line is not re-measured per keystroke, which would be O(document));
    /// `set_text` measures it again.
    longest_line: u64,
    /// The snapshot the fan-out last saw (the scroll rebase and the reveal rule read it).
    fanned: Snapshot,
    /// `commit` marked the probe's input before its dispatch; the fan-out does not mark again.
    input_marked: bool,
    /// The pointer of the right click whose `editor.context_menu` is running (`None` for the
    /// keyboard's Shift+F10 or the menu key): Task 25 opens the menu there, else at the caret.
    context_point: Option<Point<Pixels>>,
}

impl std::fmt::Debug for EditorView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EditorView")
            .field("doc", &self.doc)
            .field("model", &self.model)
            .field("config", &self.config)
            .field("ctx", &self.ctx)
            .field("marks", &self.marks)
            .field("band", &self.band)
            .field("a11y", &self.a11y)
            .field("tier", &self.tier)
            .field("paste", &self.paste)
            .field("input", &self.input)
            .field("commands", &self.commands)
            .finish_non_exhaustive()
    }
}

/// The longest line of `text`, in bytes (the tier's long-line rule).
fn longest_line(text: &str) -> u64 {
    text.split(['\n', '\r']).map(str::len).max().unwrap_or(0) as u64
}

impl EditorView {
    /// An editor over `text` for document `doc`, opened with `service` (disconnected until the
    /// host sets the context).
    pub fn new(
        doc: DocId,
        text: &str,
        cfg: EditorConfig,
        service: Rc<dyn LanguageService>,
        a11y_label: TextFn,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let tier = large_file::tier_for(text.len() as u64, longest_line(text), &cfg.tiers);
        let ctx = DocContext {
            connected: false,
            tier,
        };
        let model = EditorModel::new(text, ModelConfig::default());
        let opened_version = model.snapshot().version;
        service.open(doc, &model.snapshot(), ctx, cx);
        let sources = service.decoration_sources(doc);
        let fanned = model.snapshot();
        // A fresh analysis lands: the preview may change, and the decorations repaint.
        let weak = cx.entity().downgrade();
        service.on_update(
            doc,
            Rc::new(move |cx: &mut App| {
                let _ = weak.update(cx, |v, cx| {
                    preview::on_update(v, Refresh::AnalysisLanded, cx);
                    cx.notify();
                });
            }),
        );
        let focus = cx.focus_handle();
        let subscriptions = vec![
            cx.on_focus(&focus, window, |v, window, cx| v.focused(window, cx)),
            cx.on_blur(&focus, window, |v, window, cx| v.blurred(window, cx)),
        ];
        EditorView {
            doc,
            model,
            ime: ImeCore::default(),
            config: cfg,
            service,
            a11y_label,
            focus,
            ctx,
            marks: MarkStore::default(),
            band: BandState::default(),
            a11y: A11yState::default(),
            tier: TierState { tier },
            paste: PasteState::default(),
            probe: None,
            layout: None,
            input: InputState::default(),
            commands: CommandState::default(),
            _subscriptions: subscriptions,
            a11y_label_cache: None,
            fanned_version: opened_version,
            sources,
            cache: LayoutCache::default(),
            scroll: ScrollState::default(),
            blink: Blink::default(),
            drag: None,
            has_focus: false,
            longest_line: longest_line(text),
            fanned,
            input_marked: false,
            context_point: None,
        }
    }

    /// The editor took the focus: it becomes the `FocusedEditor` the app's commands run on,
    /// and the input handler hears of it (`ime::on_focus`).
    fn focused(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let me = cx.entity().downgrade();
        cx.set_global(FocusedEditor(Some(me)));
        self.has_focus = true;
        self.restart_blink(cx);
        ime::on_focus(self, window, cx);
    }

    /// The editor lost the focus: it stops being the `FocusedEditor` (unless another editor
    /// already took that), and the input handler hears of it (`ime::on_blur`).
    fn blurred(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let me = cx.entity().downgrade();
        if cx
            .try_global::<FocusedEditor>()
            .and_then(|f| f.0.as_ref())
            .is_some_and(|f| *f == me)
        {
            cx.set_global(FocusedEditor(None));
        }
        self.has_focus = false;
        self.blink.stop();
        self.drag = None;
        ime::on_blur(self, window, cx);
    }

    /// The document's ID.
    pub fn doc(&self) -> DocId {
        self.doc
    }

    /// The current version, O(1).
    pub fn snapshot(&self) -> Snapshot {
        self.model.snapshot()
    }

    /// Replaces the whole text (`Origin::Program`, one undo group): a restored console. An
    /// open composition is committed first (`ImeCore::dispatch`).
    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        let len = self.model.snapshot().buffer.len_bytes();
        let Ok(changes) = ChangeSet::from_edits(len, [(0..len, Arc::from(text))]) else {
            return;
        };
        self.longest_line = longest_line(text);
        self.scroll.known_width = 0.0;
        let ann = self.annotations(UserEvent::Command("set_text"), Origin::Program, cx);
        self.commit(
            Transaction {
                changes,
                selection: Some(SelectionSet::cursor(0)),
                ann,
            },
            cx,
        );
    }

    pub fn set_config(&mut self, cfg: EditorConfig, cx: &mut Context<Self>) {
        self.config = cfg;
        if self.has_focus {
            self.restart_blink(cx);
        }
        cx.notify();
    }

    /// The editor's configuration.
    pub fn config(&self) -> &EditorConfig {
        &self.config
    }

    /// The connection and tier the language service sees.
    pub fn set_doc_context(&mut self, ctx: DocContext, cx: &mut Context<Self>) {
        self.ctx = ctx;
        self.service.set_context(self.doc, ctx, cx);
        preview::on_update(self, Refresh::ContextChanged, cx);
        cx.notify();
    }

    /// The commands' state (Ctrl+M's tab-moves-focus mode, the open context menu; Task 25).
    pub fn command_state(&self) -> &CommandState {
        &self.commands
    }

    /// A polite announcement for assistive technology ("Run would execute lines 3–7", an
    /// execution status), read out by the next frame's live region (`a11y::announce`, Task 29).
    pub fn announce(&self, text: SharedString) {
        a11y::announce(&self.a11y, text);
    }

    /// What the element's accessibility hooks declare this frame (called from `request_layout`
    /// only while assistive technology is active). The label is evaluated again only when the
    /// UI-text generation changes (a language switch).
    pub(crate) fn a11y_frame(&mut self, focused: bool, cx: &App) -> A11yFrame {
        let generation = cx
            .try_global::<hacivat_ui::l10n::UiText>()
            .map_or(0, |t| t.0.generation());
        let label = match &self.a11y_label_cache {
            Some((at, label)) if *at == generation => label.clone(),
            _ => {
                let label = (self.a11y_label)(cx);
                self.a11y_label_cache = Some((generation, label.clone()));
                label
            }
        };
        A11yFrame {
            snapshot: self.model.snapshot(),
            label,
            config: self.config,
            tier: self.tier.tier,
            focused,
        }
    }

    /// An IME composition opened or closed (Task 27 calls it on mark and unmark; `commit` when
    /// an edit ends one): the preview freezes while composing (`preview::on_composition`), and
    /// the host hears of it.
    pub(crate) fn composition_changed(&mut self, composing: bool, cx: &mut Context<Self>) {
        preview::on_composition(&mut self.band, composing);
        if !composing {
            preview::on_update(self, Refresh::SelectionMoved, cx);
        }
        cx.emit(EditorEvent::CompositionChanged(composing));
        cx.emit(EditorEvent::StatusChanged);
        cx.notify();
    }

    /// Runs the editor command `id`; `false` when there is no such command.
    pub fn run_command(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(command) = command_by_id(id) else {
            return false;
        };
        if command.seals_history {
            self.model.seal_history();
        }
        (command.run)(self, window, cx);
        true
    }

    /// Selects the bytes `r` ("go to error"), each end clamped to the text and snapped back to
    /// a character boundary (a server's byte offset may fall inside a character).
    pub fn select_range(&mut self, r: Range<usize>, cx: &mut Context<Self>) {
        let snap = self.model.snapshot();
        let len = snap.buffer.len_bytes();
        let start = snap
            .buffer
            .snap(r.start.min(len), hacivat_text::Bias::Before);
        let end = snap
            .buffer
            .snap(r.end.min(len), hacivat_text::Bias::Before)
            .max(start);
        let Ok(changes) = ChangeSet::from_edits(len, []) else {
            return;
        };
        let ann = self.annotations(UserEvent::Select, Origin::Program, cx);
        self.commit(
            Transaction {
                changes,
                selection: Some(SelectionSet::single(SelectionRange {
                    anchor: start,
                    head: end,
                    goal_x: None,
                    assoc: hacivat_text::Bias::After,
                })),
                ann,
            },
            cx,
        );
    }

    /// The main caret's bounds in window coordinates (the chooser's anchor): 2 px wide, the
    /// row's height less 2 px. `None` before the first painted frame, or when the last frame
    /// did not paint the caret's line at the current version.
    pub fn caret_bounds(&self) -> Option<Bounds<Pixels>> {
        let layout = self.layout.as_ref()?;
        let snap = self.model.snapshot();
        if layout.version() != snap.version {
            return None;
        }
        crate::paint::caret_rect(layout, snap.selection.main().head)
    }

    /// The caret is drawn in the next frame (focused, and in the visible half of its blink).
    pub fn caret_visible(&self) -> bool {
        self.has_focus && self.blink.visible()
    }

    /// The shape cache's counters (the layout-cache tests and the B3 bench read them).
    pub fn layout_stats(&self) -> LayoutStats {
        self.cache.stats()
    }

    /// The chooser's hover override of the preview band (`None` clears it).
    pub fn set_preview_override(
        &mut self,
        ranges: Option<SmallVec<[Range<usize>; 2]>>,
        cx: &mut Context<Self>,
    ) {
        preview::set_override(&mut self.band, ranges);
        cx.notify();
    }

    /// The status bar's editor segment.
    pub fn status(&self) -> EditorStatus {
        let snap = self.model.snapshot();
        let main = snap.selection.main();
        let line = snap.buffer.byte_to_line(main.head);
        // Bounded on a long line (exact within its first 64 KB, Task 10).
        let column = hacivat_text::units::grapheme_column(&snap.buffer, main.head);
        let selected = snap.buffer.slice(main.range()).chars().count();
        EditorStatus {
            line: u32::try_from(line + 1).unwrap_or(u32::MAX),
            column: u32::try_from(column + 1).unwrap_or(u32::MAX),
            selected_chars: selected as u64,
            tier: self.tier.tier,
            composing: self.ime.is_composing(),
        }
    }

    /// The B3 probe (`None` removes it).
    pub fn set_probe(&mut self, probe: Option<Rc<dyn FrameProbe>>) {
        self.probe = probe;
    }

    /// Adds execution marks over byte ranges of `version`.
    pub fn add_exec_marks(
        &mut self,
        version: u64,
        marks: Vec<(MarkKey, Range<usize>)>,
        cx: &mut Context<Self>,
    ) {
        let snap = self.model.snapshot();
        marks::add(
            &mut self.marks,
            version,
            marks,
            self.model.changelog(),
            &snap,
        );
        cx.notify();
    }

    /// Sets a mark's state, its gutter label and its end-of-line status.
    pub fn set_exec_state(
        &mut self,
        key: MarkKey,
        state: ExecMarkState,
        label: Option<TextFn>,
        eol: Option<TextFn>,
        cx: &mut Context<Self>,
    ) {
        marks::set_state(&mut self.marks, key, state, label, eol);
        cx.notify();
    }

    pub fn clear_exec_marks(&mut self, cx: &mut Context<Self>) {
        marks::clear(&mut self.marks);
        cx.notify();
    }

    /// The marks (the gutter's state).
    pub fn marks(&self) -> &MarkStore {
        &self.marks
    }

    /// The last painted frame (Task 27's `bounds_for_range`, `character_index_for_point`).
    pub fn last_layout(&self) -> Option<&FrameLayout> {
        self.layout.as_ref()
    }

    /// The editor's accessible label.
    pub fn a11y_label(&self, cx: &App) -> SharedString {
        (self.a11y_label)(cx)
    }

    /// Applies `tr`, an edit that is not the IME's own, through `ImeCore::dispatch` (an open
    /// composition is committed first, so the edit never joins its undo group: UI-EDIT-12
    /// step 9), then [`EditorView::apply_committed`] (Task 19 owns this path). A composition the
    /// dispatch ended is reported (`composition_changed`) even when the edit is vetoed.
    pub(crate) fn commit(&mut self, tr: Transaction, cx: &mut Context<Self>) -> Option<Committed> {
        let event = tr.ann.user_event;
        // The probe's input mark comes before the dispatch, so B3 counts it (a paste's apply
        // is not small).
        self.mark_input(cx);
        self.input_marked = true;
        let was_composing = self.ime.is_composing();
        let result = self.ime.dispatch(&mut self.model, tr);
        if was_composing && !self.ime.is_composing() {
            self.composition_changed(false, cx);
        }
        let Ok(committed) = result else {
            self.input_marked = false;
            return None;
        };
        Some(self.apply_committed(committed, event, cx))
    }

    /// Runs `editor.context_menu` for a right click at `at` (window coordinates): the pointer
    /// is readable through [`EditorView::context_menu_point`] while the command runs, so the
    /// menu opens there even when the click fell inside the selection, far from the caret.
    pub(crate) fn open_context_menu_at(
        &mut self,
        at: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.context_point = Some(at);
        self.run_command("editor.context_menu", window, cx);
        self.context_point = None;
    }

    /// Where the running `editor.context_menu` should open: the right click's pointer, or
    /// `None` when the keyboard asked (Task 25 then opens it at `caret_bounds()`).
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "Task 25's editor.context_menu reads it")
    )]
    pub(crate) fn context_menu_point(&self) -> Option<Point<Pixels>> {
        self.context_point
    }

    /// Annotations for an edit made now (the GPUI executor's clock, so a test on the fake
    /// clock drives the typing window).
    pub(crate) fn annotations(
        &self,
        user_event: UserEvent,
        origin: Origin,
        cx: &App,
    ) -> Annotations {
        Annotations {
            user_event,
            add_to_history: true,
            time: cx.background_executor().now(),
            origin,
        }
    }

    /// The probe's input mark: once per input (a committed transaction, a wheel step).
    pub(crate) fn mark_input(&self, cx: &App) {
        if let Some(probe) = &self.probe {
            probe.input(cx.background_executor().now());
        }
    }

    /// Scrolls by `dy` pixels with the last frame's metrics (nothing before the first frame).
    pub(crate) fn scroll_by(&mut self, dy: f64) {
        let Some(layout) = self.layout.as_ref() else {
            return;
        };
        let rows = layout.rows();
        let lines = u32::try_from(self.model.buffer().len_lines()).unwrap_or(u32::MAX);
        let map = HeightMap::uniform(lines, f64::from(f32::from(rows.line_height)));
        let view_h = f64::from(f32::from(rows.bounds.size.height));
        self.scroll.scroll_by(&map, dy, view_h);
    }

    /// Restarts the caret blink after an input (or a focus, or a configuration change):
    /// solid now, blinking after a half-period of no input, solid and stopped after 10 s idle;
    /// never under reduced motion or with the blink off (NOTES open question 3).
    fn restart_blink(&mut self, cx: &mut Context<Self>) {
        let epoch = self.blink.restart(cx.background_executor().now());
        let blinks = self.has_focus
            && self.config.caret_blink == crate::config::CaretBlink::System
            && !self.config.reduced_motion
            && !cx.reduce_motion();
        if !blinks {
            self.blink.stop();
            return;
        }
        // The running timer (if any) belongs to an older epoch; a fresh one counts the
        // half-period from this input.
        self.blink.task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(BLINK_HALF).await;
                let go = this
                    .update(cx, |v, cx| {
                        let now = cx.background_executor().now();
                        let go = v.blink.tick(epoch, now);
                        cx.notify();
                        go
                    })
                    .unwrap_or(false);
                if !go {
                    break;
                }
            }
        }));
    }

    /// The one entry for every `Committed`, whatever produced it (review round 2): a
    /// transaction (`commit`), `EditorModel::undo`/`redo` (Task 25: `UserEvent::Undo`/`Redo`),
    /// `ImeCore::mark`/`commit` (Task 27: `UserEvent::InputTypeCompose`/`InputType`). It lets
    /// the marks, the language service, the tier, the preview, the input handler and the host
    /// know, in that order.
    ///
    /// It reads `version`, `buffer_changed` and `desc` only (never a literal or an exhaustive
    /// destructure, so Task 5's extra fields compile here), and a `Committed` whose version did
    /// not move is selection-only whatever its other fields say.
    pub(crate) fn apply_committed(
        &mut self,
        committed: Committed,
        event: UserEvent,
        cx: &mut Context<Self>,
    ) -> Committed {
        // A commit that did not come through `commit` (undo, redo, an IME mark) marks here.
        if !std::mem::take(&mut self.input_marked) {
            self.mark_input(cx);
        }
        let snap = self.model.snapshot();
        let text_changed = committed.buffer_changed && committed.version != self.fanned_version;
        self.fanned_version = committed.version;
        let prev = std::mem::replace(&mut self.fanned, snap.clone());
        if text_changed {
            self.scroll
                .rebase(&prev.buffer, &snap.buffer, &committed.desc);
        }
        if reveals(&prev, &snap, &committed, text_changed) {
            self.scroll.reveal = true;
        }
        marks::on_commit(&mut self.marks, &committed, self.model.changelog(), &snap);
        if text_changed {
            self.service.change(self.doc, &snap, &committed.desc, cx);
            let text_len = snap.buffer.len_bytes() as u64;
            // Nothing per keystroke is O(document): the longest line grows with the lines an
            // edit touched (bounded by the edit, not the document).
            if let Some(dirty) = &committed.dirty {
                self.longest_line = self
                    .longest_line
                    .max(longest_touched(&snap.buffer, dirty.clone()));
            }
            if let Some(tier) = large_file::update(
                &mut self.tier,
                text_len,
                self.longest_line,
                &self.config.tiers,
            ) {
                cx.emit(EditorEvent::TierChanged(tier));
            }
            cx.emit(EditorEvent::Changed {
                version: committed.version,
            });
        } else {
            cx.emit(EditorEvent::SelectionChanged {
                version: committed.version,
            });
        }
        let why = if text_changed {
            Refresh::Changed
        } else {
            Refresh::SelectionMoved
        };
        preview::on_update(self, why, cx);
        ime::after_commit(self, &committed, event, cx);
        if self.has_focus {
            self.restart_blink(cx);
        }
        cx.emit(EditorEvent::StatusChanged);
        cx.notify();
        committed
    }
}

/// Whether a commit from `prev` to `snap` reveals the main caret: it changed the main selection
/// (a selection-only commit), moved the caret beyond where mapping through the change puts it,
/// or edited at it. An edit elsewhere that leaves the caret where it was (an edit above the
/// view) keeps the scroll position.
fn reveals(prev: &Snapshot, snap: &Snapshot, committed: &Committed, text_changed: bool) -> bool {
    let head = snap.selection.main().head;
    let old = prev.selection.main().head;
    if !text_changed {
        return snap.selection.main().range() != prev.selection.main().range() || head != old;
    }
    if let Some(d) = &committed.dirty
        && d.start <= head
        && head <= d.end
    {
        return true;
    }
    let fits = committed.desc.len_old() == prev.buffer.len_bytes();
    let mapped = fits
        .then(|| committed.desc.map_pos(old, Bias::After, MapMode::Simple))
        .flatten();
    mapped != Some(head)
}

/// The longest line, in bytes, among the lines `dirty` touches (O(touched lines)).
fn longest_touched(buf: &TextBuffer, dirty: Range<usize>) -> u64 {
    let first = buf.byte_to_line(dirty.start);
    let last = buf.byte_to_line(dirty.end);
    (first..=last)
        .map(|l| buf.line_range(l, false).len() as u64)
        .max()
        .unwrap_or(0)
}

impl Focusable for EditorView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl EventEmitter<EditorEvent> for EditorView {}

impl Render for EditorView {
    /// A focusable column with the large-file banner's slot, the editor element (the text area
    /// with its gutter, its input handler and accessibility node, every module seam) and the
    /// commands' overlay.
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let banner = large_file::banner(&self.tier, window, cx);
        let overlay = commands::overlay(self, window, cx);
        let text = EditorElement::new(
            cx.entity(),
            self.focus.clone(),
            ("editor-text", self.doc.0).into(),
            self.a11y.clone(),
        );
        div()
            .id(("editor", self.doc.0))
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .children(banner)
            .child(text)
            .children(overlay)
    }
}

// ## m3a-t19-editor-view: tests of the view's crate-private paths (an edit with a mapped
// selection, the IME core's state).
#[cfg(test)]
mod t19_tests {
    use std::cell::RefCell;

    use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext, div, px};

    use super::*;
    use crate::service::FailClosedService;

    struct Host {
        editor: Entity<EditorView>,
    }

    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().w(px(800.0)).h(px(600.0)).child(self.editor.clone())
        }
    }

    fn open(cx: &mut TestAppContext, text: String) -> (Entity<EditorView>, &mut VisualTestContext) {
        let slot: Rc<RefCell<Option<Entity<EditorView>>>> = Rc::default();
        let out = slot.clone();
        let (_, vcx) = cx.add_window_view(move |window, cx| {
            let editor = cx.new(|cx| {
                EditorView::new(
                    DocId(5),
                    &text,
                    EditorConfig::default(),
                    Rc::new(FailClosedService),
                    Rc::new(|_: &App| "Console 1".into()),
                    window,
                    cx,
                )
            });
            *out.borrow_mut() = Some(editor.clone());
            Host { editor }
        });
        let editor = slot.borrow().clone().unwrap();
        draw(vcx);
        (editor, vcx)
    }

    fn draw(vcx: &mut VisualTestContext) {
        vcx.update(|window, _| window.refresh());
        vcx.run_until_parked();
    }

    /// The first painted line and its text.
    fn first_row(e: &Entity<EditorView>, vcx: &VisualTestContext) -> (u32, String) {
        vcx.read(|cx| {
            let v = e.read(cx);
            let line = v.last_layout().unwrap().rows().first_line;
            let buf = v.snapshot().buffer;
            let r = buf.line_range(line as usize, false);
            (line, buf.slice(r).into_owned())
        })
    }

    #[gpui::test]
    fn scroll_keeps_the_anchor_across_edits_above(cx: &mut TestAppContext) {
        let text: String = (0..200).map(|i| format!("line {i}\n")).collect();
        let (e, vcx) = open(cx, text);
        // Go to line 120: the view scrolls just enough to show it.
        let at = vcx.read(|cx| e.read(cx).snapshot().buffer.line_to_byte(120));
        e.update(vcx, |v, cx| v.select_range(at..at, cx));
        draw(vcx);
        let (line, shown) = first_row(&e, vcx);
        assert!(line > 80, "scrolled to line 120: first {line}");
        // Three lines inserted at the top, the caret mapped: the content stays still.
        e.update(vcx, |v, cx| {
            let len = v.model.buffer().len_bytes();
            let ann = v.annotations(UserEvent::InputPaste, Origin::User, cx);
            v.commit(
                Transaction {
                    changes: ChangeSet::from_edits(len, [(0..0, Arc::from("a\nb\nc\n"))]).unwrap(),
                    selection: None,
                    ann,
                },
                cx,
            );
        });
        draw(vcx);
        let (moved, still) = first_row(&e, vcx);
        assert_eq!((moved, still.as_str()), (line + 3, shown.as_str()));
        // Typing at a caret scrolled out of view brings it back.
        e.update(vcx, |v, _| {
            v.scroll = ScrollState::default();
        });
        draw(vcx);
        assert_eq!(first_row(&e, vcx).0, 0);
        e.update_in(vcx, |v, window, cx| {
            hacivat_ui_core::gpui_surface::EntityInputHandler::replace_text_in_range(
                v, None, "x", window, cx,
            );
        });
        draw(vcx);
        let caret_line = vcx.read(|cx| {
            let v = e.read(cx);
            let s = v.snapshot();
            s.buffer.byte_to_line(s.selection.main().head) as u32
        });
        let rows = vcx.read(|cx| *e.read(cx).last_layout().unwrap().rows());
        assert!(
            rows.first_line <= caret_line && caret_line <= rows.last_line,
            "{caret_line} in {rows:?}"
        );
    }

    /// `set_text` and `select_range` are not the IME's edits: they commit an open composition
    /// first, so neither joins its undo group (UI-EDIT-12 step 9), and the host hears that the
    /// composition ended.
    #[gpui::test]
    fn programmatic_edits_commit_the_composition_first(cx: &mut TestAppContext) {
        let (e, vcx) = open(cx, "SELECT ".to_string());
        let events: Rc<RefCell<Vec<String>>> = Rc::default();
        let seen = events.clone();
        vcx.update(|_, cx| {
            cx.subscribe(&e, move |_, ev: &EditorEvent, _| {
                seen.borrow_mut().push(format!("{ev:?}"));
            })
            .detach();
        });
        let mark = |e: &Entity<EditorView>, vcx: &mut VisualTestContext| {
            e.update(vcx, |v, cx| {
                let now = cx.background_executor().now();
                let c = v
                    .ime
                    .mark(&mut v.model, None, "ş", None, now)
                    .expect("the mark applies");
                v.apply_committed(c, UserEvent::InputTypeCompose, cx);
                assert!(v.ime.is_composing());
            });
        };
        mark(&e, vcx);
        e.update(vcx, |v, cx| v.set_text("SELECT 2", cx));
        vcx.read(|cx| {
            let v = e.read(cx);
            assert!(!v.ime.is_composing());
            assert_eq!(v.snapshot().buffer.to_string(), "SELECT 2");
            assert_eq!(
                v.model.undo_depth(),
                2,
                "the composition and set_text: two groups"
            );
        });
        assert!(
            events
                .borrow()
                .iter()
                .any(|e| e == "CompositionChanged(false)"),
            "{:?}",
            events.borrow()
        );
        mark(&e, vcx);
        e.update(vcx, |v, cx| v.select_range(0..6, cx));
        vcx.read(|cx| {
            let v = e.read(cx);
            assert!(!v.ime.is_composing());
            assert_eq!(v.model.undo_depth(), 3);
        });
    }

    // ## m3a-t19-editor-view (review round 1)

    /// Counts the probe's input marks.
    #[derive(Default)]
    struct Inputs(std::cell::Cell<u32>);

    impl FrameProbe for Inputs {
        fn input(&self, _: std::time::Instant) {
            self.0.set(self.0.get() + 1);
        }
        fn painted(&self, _: &mut App) {}
    }

    /// The input mark comes before the dispatch: a vetoed edit still counts as an input, a
    /// commit through `commit` counts once, and a `Committed` made elsewhere (undo, an IME
    /// mark) is marked by the fan-out.
    #[gpui::test]
    fn the_probe_marks_input_before_the_dispatch(cx: &mut TestAppContext) {
        let (e, vcx) = open(cx, "SELECT 1".to_string());
        let probe = Rc::new(Inputs::default());
        let p: Rc<dyn FrameProbe> = probe.clone();
        e.update(vcx, |v, cx| {
            v.set_probe(Some(p));
            let ann = v.annotations(UserEvent::InputType, Origin::User, cx);
            // A change built for another text length: vetoed by the model.
            let stale = Transaction {
                changes: ChangeSet::from_edits(3, [(0..0, Arc::from("x"))]).unwrap(),
                selection: None,
                ann,
            };
            assert!(v.commit(stale, cx).is_none());
            assert_eq!(probe.0.get(), 1, "the attempt was an input");
            let ok = Transaction {
                changes: ChangeSet::from_edits(8, [(8..8, Arc::from(";"))]).unwrap(),
                selection: None,
                ann,
            };
            assert!(v.commit(ok, cx).is_some());
            assert_eq!(probe.0.get(), 2, "once per commit");
            let now = cx.background_executor().now();
            let c = v
                .ime
                .mark(&mut v.model, None, "ş", None, now)
                .expect("the mark applies");
            v.apply_committed(c, UserEvent::InputTypeCompose, cx);
            assert_eq!(probe.0.get(), 3, "a Committed made outside `commit`");
        });
    }

    /// A composition is underlined 1 px in `text` ink under its whole range, and its active
    /// clause (the selection inside it) 2 px, on one baseline.
    #[gpui::test]
    fn the_composition_is_underlined_and_its_clause_thicker(cx: &mut TestAppContext) {
        let (e, vcx) = open(cx, "SELECT ".to_string());
        e.update(vcx, |v, cx| {
            let len = v.model.buffer().len_bytes();
            v.select_range(len..len, cx);
            let now = cx.background_executor().now();
            let c = v
                .ime
                .mark(&mut v.model, None, "nihao", Some(0..2), now)
                .expect("the mark applies");
            v.apply_committed(c, UserEvent::InputTypeCompose, cx);
        });
        draw(vcx);
        let ink = vcx.read(|cx| crate::theme::EditorTheme::get(cx).ime_underline);
        let (x, quads) = vcx.update(|window, cx| {
            let v = e.read(cx);
            let l = v.last_layout().unwrap();
            let x = |b: usize| f32::from(l.x_for_offset(b).unwrap());
            let scale = window.scale_factor();
            let quads: Vec<(f32, f32, f32, f32)> = window
                .painted_quads()
                .iter()
                .filter(|q| q.background.as_solid() == Some(ink))
                .map(|q| {
                    let b = q.bounds;
                    (
                        b.origin.x.0 / scale,
                        b.origin.y.0 / scale,
                        b.size.width.0 / scale,
                        b.size.height.0 / scale,
                    )
                })
                .collect();
            ((x(7), x(9), x(12)), quads)
        });
        let close = |a: f32, b: f32| (a - b).abs() < 0.05;
        let whole = quads
            .iter()
            .find(|q| close(q.3, 1.0))
            .expect("the 1 px underline");
        let clause = quads
            .iter()
            .find(|q| close(q.3, 2.0))
            .expect("the 2 px clause");
        assert!(
            close(whole.0, x.0) && close(whole.2, x.2 - x.0),
            "{quads:?}"
        );
        assert!(
            close(clause.0, x.0) && close(clause.2, x.1 - x.0),
            "{quads:?}"
        );
        assert!(close(whole.1, clause.1), "one baseline");
        // 3 px under GPUI's own baseline, so below the row's middle (the test text system's
        // metrics put the baseline near the row's bottom).
        assert!(whole.1 > 6.0 + 9.0, "under the text: {whole:?}");
    }

    /// The right click's pointer is readable only while its `editor.context_menu` runs, so a
    /// later Shift+F10 opens at the caret, never at an old pointer.
    #[gpui::test]
    fn the_context_menu_pointer_is_scoped_to_the_right_click(cx: &mut TestAppContext) {
        let (e, vcx) = open(cx, "SELECT 1".to_string());
        e.update_in(vcx, |v, window, cx| {
            assert_eq!(v.context_menu_point(), None);
            v.open_context_menu_at(gpui::point(px(120.0), px(14.0)), window, cx);
            assert_eq!(v.context_menu_point(), None);
        });
    }
}
