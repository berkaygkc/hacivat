//! Key semantics (HV-GRID-05 step 4; Task 17): what each [`GridCommand`] does to the selection.
//!
//! - The focus moves in view coordinates; the selection collapses to the new focus unless the
//!   command extends it from the anchor.
//! - Jumps go to the edge of the grid, never to the "next non-empty" cell: the data may not be
//!   loaded.
//! - The last row is the last *loaded* row while rows stream in; Last row then follows the
//!   tail.
//! - Tab and Shift+Tab move to the next and previous cell, wrapping rows inside the grid, and
//!   leave the grid from its last (first) cell, so keyboard users always have an exit (digest 04
//!   R10). Tab collapses the selection.
//! - A focus on a column hidden since sits between its visible neighbours: Right and Left step
//!   to them, every other key snaps it to the next visible column.

use crate::model::selection::{CopyRefusal, CopyRegion, Selection, ViewMap};
use crate::source::{ColId, ModelCell, ModelRow, ViewRow};
use crate::view::GridCommand;

/// A grid direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

/// What a command runs against.
#[derive(Debug, Clone, Copy)]
pub struct KeyCtx<'a> {
    /// The loaded rows, in view order.
    pub map: &'a ViewMap,
    /// The visible columns, in view order.
    pub order: &'a [ColId],
    /// Rows per page (PageUp/PageDown): the body's whole rows.
    pub page_rows: u64,
}

/// What the view does after a command.
#[derive(Debug, Clone, PartialEq)]
pub enum KeyOutcome {
    /// Nothing changed (an edge, an empty grid, a repeated select-all).
    Unchanged,
    /// The selection changed and the focus stayed.
    Changed,
    /// The focus moved: scroll it into view. `follow_tail`: the view goes to the end and follows
    /// rows streaming in (Last row), even when the focus was already there.
    Moved { focus: ModelCell, follow_tail: bool },
    /// Tab from the last cell (Shift+Tab from the first): the keyboard focus leaves the grid.
    Leave { back: bool },
    /// Copy this region (or say why not).
    Copy {
        region: Result<CopyRegion, CopyRefusal>,
        headers: bool,
    },
}

/// Runs `cmd` on `sel`.
pub fn apply_command(cmd: GridCommand, sel: &mut Selection, ctx: &KeyCtx<'_>) -> KeyOutcome {
    match cmd {
        GridCommand::Copy { headers } => KeyOutcome::Copy {
            region: sel.normalize_for_copy(ctx.map, ctx.order),
            headers,
        },
        GridCommand::Escape => changed(sel.collapse(ctx.map)),
        GridCommand::SelectAll => select(sel, ctx, |s, _| s.select_all()),
        GridCommand::SelectRow => select(sel, ctx, |s, focus| s.extend_rows_to(focus.row)),
        GridCommand::SelectColumn => select(sel, ctx, |s, focus| s.extend_columns_to(focus.col)),
        nav => navigate(nav, sel, ctx),
    }
}

/// Where a navigation command goes.
enum Nav {
    To {
        row: u64,
        col: i64,
        extend: bool,
        follow_tail: bool,
    },
    Leave {
        back: bool,
    },
}

fn navigate(cmd: GridCommand, sel: &mut Selection, ctx: &KeyCtx<'_>) -> KeyOutcome {
    let Some(grid) = Grid::of(ctx) else {
        return match cmd {
            GridCommand::NextCell { back } => KeyOutcome::Leave { back },
            _ => KeyOutcome::Unchanged,
        };
    };
    let Some(focus) = sel.focus() else {
        // Nothing focused yet: the first key focuses the first cell.
        let first = grid.cell(ctx, 0, 0);
        sel.select_cell(first, ctx.map);
        return KeyOutcome::Moved {
            focus: first,
            follow_tail: false,
        };
    };
    let row = ctx
        .map
        .model_to_view(focus.row)
        .map_or(focus.row.0, |v| v.0)
        .min(grid.last_row);
    let half = half_position(ctx.order, focus.col);
    let Some(nav) = grid.target(cmd, row, half, ctx.page_rows) else {
        return KeyOutcome::Unchanged;
    };
    match nav {
        Nav::Leave { back } => KeyOutcome::Leave { back },
        Nav::To {
            row,
            col,
            extend,
            follow_tail,
        } => {
            let before = sel.clone();
            let target = grid.cell(ctx, row, col);
            if extend {
                sel.extend_to(target, ctx.map);
            } else {
                sel.select_cell(target, ctx.map);
            }
            if follow_tail || *sel != before {
                KeyOutcome::Moved {
                    focus: target,
                    follow_tail,
                }
            } else {
                KeyOutcome::Unchanged
            }
        }
    }
}

/// Select-all, row or column: from the focus, or from the first cell when nothing is focused.
fn select(
    sel: &mut Selection,
    ctx: &KeyCtx<'_>,
    apply: impl FnOnce(&mut Selection, ModelCell),
) -> KeyOutcome {
    let Some(grid) = Grid::of(ctx) else {
        return KeyOutcome::Unchanged;
    };
    let before = sel.clone();
    let focus = match sel.focus() {
        Some(focus) => focus,
        None => {
            let first = grid.cell(ctx, 0, 0);
            sel.select_cell(first, ctx.map);
            first
        }
    };
    apply(sel, focus);
    changed(*sel != before)
}

fn changed(changed: bool) -> KeyOutcome {
    if changed {
        KeyOutcome::Changed
    } else {
        KeyOutcome::Unchanged
    }
}

/// A grid with at least one loaded row and one visible column.
#[derive(Debug, Clone, Copy)]
struct Grid {
    last_row: u64,
    last_col: i64,
}

impl Grid {
    fn of(ctx: &KeyCtx<'_>) -> Option<Grid> {
        let rows = ctx.map.len();
        (rows > 0 && !ctx.order.is_empty()).then(|| Grid {
            last_row: rows - 1,
            last_col: ctx.order.len() as i64 - 1,
        })
    }

    fn cell(&self, ctx: &KeyCtx<'_>, row: u64, col: i64) -> ModelCell {
        let col = col.clamp(0, self.last_col) as usize;
        ModelCell {
            row: ctx.map.view_to_model(ViewRow(row)).unwrap_or(ModelRow(row)),
            col: ctx.order[col],
        }
    }

    /// The destination of `cmd` from view row `row` and the doubled column position `half`
    /// (see [`half_position`]); `None` for a command that does not navigate.
    fn target(&self, cmd: GridCommand, row: u64, half: i64, page_rows: u64) -> Option<Nav> {
        let snap = (half + 1).div_euclid(2);
        let to = |row: u64, col: i64, extend: bool| Nav::To {
            row,
            col,
            extend,
            follow_tail: false,
        };
        Some(match cmd {
            GridCommand::Move { dir, extend } => match dir {
                Dir::Up => to(row.saturating_sub(1), snap, extend),
                Dir::Down => to(row.saturating_add(1).min(self.last_row), snap, extend),
                Dir::Left => to(row, (half + 1).div_euclid(2) - 1, extend),
                Dir::Right => to(row, half.div_euclid(2) + 1, extend),
            },
            GridCommand::JumpEdge { dir, extend } => match dir {
                Dir::Up => to(0, snap, extend),
                Dir::Down => to(self.last_row, snap, extend),
                Dir::Left => to(row, 0, extend),
                Dir::Right => to(row, self.last_col, extend),
            },
            GridCommand::Home { extend } => to(row, 0, extend),
            GridCommand::End { extend } => to(row, self.last_col, extend),
            GridCommand::FirstRow { extend } => to(0, snap, extend),
            GridCommand::LastRow { extend } => Nav::To {
                row: self.last_row,
                col: snap,
                extend,
                follow_tail: true,
            },
            GridCommand::Page { up, extend } => {
                let n = page_rows.max(1);
                let row = if up {
                    row.saturating_sub(n)
                } else {
                    row.saturating_add(n).min(self.last_row)
                };
                to(row, snap, extend)
            }
            GridCommand::NextCell { back: false } => {
                let next = half.div_euclid(2) + 1;
                if next <= self.last_col {
                    to(row, next, false)
                } else if row < self.last_row {
                    to(row + 1, 0, false)
                } else {
                    Nav::Leave { back: false }
                }
            }
            GridCommand::NextCell { back: true } => {
                let prev = (half + 1).div_euclid(2) - 1;
                if prev >= 0 {
                    to(row, prev, false)
                } else if row > 0 {
                    to(row - 1, self.last_col, false)
                } else {
                    Nav::Leave { back: true }
                }
            }
            GridCommand::SelectAll
            | GridCommand::SelectRow
            | GridCommand::SelectColumn
            | GridCommand::Copy { .. }
            | GridCommand::Escape => return None,
        })
    }
}

/// The focus column's place among the visible columns, doubled so a hidden column has a place
/// too: `2p` at visible position `p`; `2n − 1` just before the first visible column with a
/// higher ordinal `n`; `2·len − 1` after the last.
fn half_position(order: &[ColId], col: ColId) -> i64 {
    let exact = match order.get(col.0 as usize) {
        Some(c) if *c == col => Some(col.0 as usize),
        _ => order.iter().position(|c| *c == col),
    };
    match exact {
        Some(p) => 2 * p as i64,
        None => {
            let next = order
                .iter()
                .position(|c| c.0 > col.0)
                .unwrap_or(order.len());
            2 * next as i64 - 1
        }
    }
}
