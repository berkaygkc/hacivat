//! `GridView` (Task 20): the grid's view over a source, its commands and events.
//!
//! Task 1b: the view keeps its source, configuration, selection and scroll; `dispatch` and
//! `scroll_px` do nothing; it renders an empty focusable area.

use std::ops::Range;
use std::rc::Rc;

use hacivat_ui::text::TextFn;
use hacivat_ui_core::gpui_surface::{
    App, Context, EventEmitter, FocusHandle, Focusable, InteractiveElement, IntoElement, Render,
    Styled, Window, div,
};

use crate::model::{self, ScrollState};
use crate::source::{ColId, GridDataSource, ModelCell};

/// The grid's own text (the app passes Fluent text).
#[derive(Clone)]
pub struct GridLabels {
    /// A placeholder row's label.
    pub loading: TextFn,
    /// More than 2,000 columns.
    pub too_many_columns: TextFn,
}

impl std::fmt::Debug for GridLabels {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GridLabels").finish_non_exhaustive()
    }
}

/// How a grid looks and is announced.
#[derive(Clone)]
pub struct GridConfig {
    /// 21 px.
    pub row_height: f32,
    /// Off in M3a.
    pub zebra: bool,
    pub labels: GridLabels,
    /// Role `Grid` with this label (R-M3-32).
    pub a11y_label: TextFn,
}

impl std::fmt::Debug for GridConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GridConfig")
            .field("row_height", &self.row_height)
            .field("zebra", &self.zebra)
            .finish_non_exhaustive()
    }
}

/// A grid command (keymap rows `grid.*`, the app's commands).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GridCommand {
    Move { dir: model::Dir, extend: bool },
    JumpEdge { dir: model::Dir, extend: bool },
    Home { extend: bool },
    End { extend: bool },
    FirstRow { extend: bool },
    LastRow { extend: bool },
    Page { up: bool, extend: bool },
    NextCell { back: bool },
    SelectAll,
    SelectRow,
    SelectColumn,
    Copy { headers: bool },
    Escape,
}

/// What a grid tells its host.
#[derive(Debug, Clone, PartialEq)]
pub enum GridEvent {
    SelectionChanged,
    FocusChanged(Option<ModelCell>),
    VisibleRegion {
        rows: Range<u64>,
        cols: Range<usize>,
    },
    CopyRequested {
        region: model::CopyRegion,
        headers: bool,
    },
    CopyRefused(model::CopyRefusal),
    ColumnResized {
        col: ColId,
        width: f32,
    },
}

/// One grid.
pub struct GridView {
    source: Rc<dyn GridDataSource>,
    config: GridConfig,
    selection: model::Selection,
    scroll: ScrollState,
    focus: FocusHandle,
}

impl std::fmt::Debug for GridView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GridView")
            .field("config", &self.config)
            .field("selection", &self.selection)
            .field("scroll", &self.scroll)
            .finish_non_exhaustive()
    }
}

impl GridView {
    pub fn new(source: Rc<dyn GridDataSource>, config: GridConfig, cx: &mut Context<Self>) -> Self {
        GridView {
            source,
            config,
            selection: model::Selection::default(),
            scroll: ScrollState::default(),
            focus: cx.focus_handle(),
        }
    }

    /// Replaces the source (a re-run): the selection clears; widths stay when `keep_widths`.
    pub fn set_source(
        &mut self,
        source: Rc<dyn GridDataSource>,
        keep_widths: bool,
        cx: &mut Context<Self>,
    ) {
        let _ = keep_widths;
        self.source = source;
        self.selection = model::Selection::default();
        cx.notify();
    }

    /// Runs `cmd` (Task 20).
    pub fn dispatch(&mut self, cmd: GridCommand, window: &mut Window, cx: &mut Context<Self>) {
        let _ = (cmd, window, cx);
    }

    pub fn selection(&self) -> &model::Selection {
        &self.selection
    }

    /// Scrolls by pixels (Task 20).
    pub fn scroll_px(&mut self, dy: f64, dx: f64, cx: &mut Context<Self>) {
        let _ = (dy, dx, cx);
    }

    /// The grid's source.
    pub fn source(&self) -> &Rc<dyn GridDataSource> {
        &self.source
    }
}

impl Focusable for GridView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl EventEmitter<GridEvent> for GridView {}

impl Render for GridView {
    /// Task 1b: an empty focusable area; Task 20 paints the grid.
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().track_focus(&self.focus).size_full()
    }
}
