//! `EditorView` (UI-EDIT-01 view; Task 19): the model, the IME core, the language service, the
//! module-owned state (P11), and the public API the console host and the execution glue use.
//!
//! Task 1b: a working minimal view. Every committed transaction notifies the service, the
//! marks, the tier, the preview and the input handler, and emits `EditorEvent::Changed` (or
//! `SelectionChanged`); the input handler (`ime.rs`) inserts plain text. The element paints
//! nothing yet (Task 19), so `caret_bounds` and `last_layout` are `None`.
//!
//! Call points every module seam already has (fix round 1, P11), so Tasks 25 and 27–29 never
//! edit this file: focus and blur (`ime::on_focus`/`on_blur`, `FocusedEditor`), every commit
//! (`marks::on_commit`, `large_file::update`, `preview::on_update`, `ime::after_commit`), a
//! fresh analysis (`LanguageService::on_update` → `preview::on_update`), a composition opening or
//! closing ([`EditorView::composition_changed`] → `preview::on_composition`), the banner
//! (`large_file::banner`), each frame (`element::paint_frame`), and the commands' state
//! (`commands::CommandState`).

use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use hacivat_text::{
    Annotations, ChangeSet, Committed, EditorModel, ImeCore, ModelConfig, Origin, SelectionRange,
    SelectionSet, Snapshot, Transaction, UserEvent,
};
use hacivat_ui::text::TextFn;
use hacivat_ui_core::gpui_surface::{
    App, Bounds, Context, ElementInputHandler, EventEmitter, FocusHandle, Focusable, Global,
    InteractiveElement, IntoElement, ParentElement, Pixels, Render, SharedString, Styled,
    Subscription, WeakEntity, Window, canvas, div,
};
use smallvec::SmallVec;
use unicode_segmentation::UnicodeSegmentation;

use crate::a11y::A11yState;
use crate::clipboard::PasteState;
use crate::commands::{CommandState, command_by_id};
use crate::config::{DocContext, DocId, EditorConfig, Tier};
use crate::element::{self, FrameLayout};
use crate::ime::{self, InputState};
use crate::large_file::{self, TierState};
use crate::marks::{self, ExecMarkState, MarkKey, MarkStore};
use crate::preview::{self, BandState, Refresh};
use crate::probe::FrameProbe;
use crate::service::LanguageService;

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

fn annotations(user_event: UserEvent, origin: Origin) -> Annotations {
    Annotations {
        user_event,
        add_to_history: true,
        time: Instant::now(),
        origin,
    }
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
        service.open(doc, &model.snapshot(), ctx, cx);
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
        }
    }

    /// The editor took the focus: it becomes the `FocusedEditor` the app's commands run on,
    /// and the input handler hears of it (`ime::on_focus`).
    fn focused(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let me = cx.entity().downgrade();
        cx.set_global(FocusedEditor(Some(me)));
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

    /// Replaces the whole text (`Origin::Program`, one undo group): a restored console.
    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        let len = self.model.snapshot().buffer.len_bytes();
        let Ok(changes) = ChangeSet::from_edits(len, [(0..len, Arc::from(text))]) else {
            return;
        };
        self.commit(
            Transaction {
                changes,
                selection: Some(SelectionSet::cursor(0)),
                ann: annotations(UserEvent::Command("set_text"), Origin::Program),
            },
            cx,
        );
    }

    pub fn set_config(&mut self, cfg: EditorConfig, cx: &mut Context<Self>) {
        self.config = cfg;
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

    /// The commands' state (Ctrl+M's tab-moves-focus mode, Task 25).
    pub fn command_state(&self) -> CommandState {
        self.commands
    }

    /// An IME composition opened or closed (Task 27 calls it on mark and unmark): the preview
    /// freezes while composing (`preview::on_composition`), and the host hears of it.
    #[allow(dead_code, reason = "Task 27 calls it from ime.rs on mark and unmark")]
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
        self.commit(
            Transaction {
                changes,
                selection: Some(SelectionSet::single(SelectionRange {
                    anchor: start,
                    head: end,
                    goal_x: None,
                    assoc: hacivat_text::Bias::After,
                })),
                ann: annotations(UserEvent::Select, Origin::Program),
            },
            cx,
        );
    }

    /// The caret's bounds in window coordinates (the chooser's anchor); `None` before the
    /// first painted frame (Task 19).
    pub fn caret_bounds(&self) -> Option<Bounds<Pixels>> {
        None
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
        let line_start = snap.buffer.line_to_byte(line);
        let before = snap.buffer.slice(line_start..main.head.max(line_start));
        let column = before.graphemes(true).count();
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

    /// Applies `tr` and lets everything that follows a commit know (Task 19 owns this path).
    pub(crate) fn commit(&mut self, tr: Transaction, cx: &mut Context<Self>) -> Option<Committed> {
        let event = tr.ann.user_event;
        let committed = self.model.dispatch(tr).ok()?;
        let snap = self.model.snapshot();
        marks::on_commit(&mut self.marks, &committed, self.model.changelog(), &snap);
        if committed.buffer_changed {
            self.service.change(self.doc, &snap, &committed.desc, cx);
            let text_len = snap.buffer.len_bytes() as u64;
            // Nothing per keystroke is O(document): Task 19 keeps the longest line as edits
            // land; until then the size alone decides (Task 1b's `tier_for` ignores it).
            if let Some(tier) = large_file::update(&mut self.tier, text_len, 0, &self.config.tiers)
            {
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
        let why = if committed.buffer_changed {
            Refresh::Changed
        } else {
            Refresh::SelectionMoved
        };
        preview::on_update(self, why, cx);
        ime::after_commit(self, &committed, event, cx);
        cx.emit(EditorEvent::StatusChanged);
        cx.notify();
        Some(committed)
    }
}

impl Focusable for EditorView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl EventEmitter<EditorEvent> for EditorView {}

impl Render for EditorView {
    /// Task 1b: a focusable area that takes text input, the large-file banner's slot above it,
    /// and the paint path that calls every module seam (`element::paint_frame`); Task 19 paints
    /// the text.
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let focus = self.focus.clone();
        let banner = large_file::banner(&self.tier, window, cx);
        div()
            .id(("editor", self.doc.0))
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .children(banner)
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, (), window, cx| {
                        let focused = focus.is_focused(window);
                        window.handle_input(
                            &focus,
                            ElementInputHandler::new(bounds, entity.clone()),
                            cx,
                        );
                        entity.update(cx, |v, cx| {
                            element::paint_frame(v, bounds, focused, window, cx);
                        });
                    },
                )
                .flex_1()
                .w_full(),
            )
    }
}
