//! The grid's command ids (keymap rows `grid.*`, context `Grid`): each id and the
//! [`GridCommand`] it runs. The app registers one `ActionDef` per entry (Task 30); a keymap
//! row naming an id outside this table fails `every_bound_grid_command_is_in_the_table`.
//!
//! A Task 1b table (fix round 1): an id added to a keymap is added here in the same change.

use crate::model::Dir;
use crate::view::GridCommand;

/// Every grid command id, in palette order, and its command.
pub static GRID_COMMANDS: [(&str, GridCommand); 36] = [
    ("grid.move_left", mv(Dir::Left, false)),
    ("grid.move_right", mv(Dir::Right, false)),
    ("grid.move_up", mv(Dir::Up, false)),
    ("grid.move_down", mv(Dir::Down, false)),
    ("grid.select_left", mv(Dir::Left, true)),
    ("grid.select_right", mv(Dir::Right, true)),
    ("grid.select_up", mv(Dir::Up, true)),
    ("grid.select_down", mv(Dir::Down, true)),
    ("grid.jump_left", jump(Dir::Left)),
    ("grid.jump_right", jump(Dir::Right)),
    ("grid.jump_up", jump(Dir::Up)),
    ("grid.jump_down", jump(Dir::Down)),
    ("grid.home", GridCommand::Home { extend: false }),
    ("grid.end", GridCommand::End { extend: false }),
    ("grid.first_row", GridCommand::FirstRow { extend: false }),
    ("grid.last_row", GridCommand::LastRow { extend: false }),
    (
        "grid.page_up",
        GridCommand::Page {
            up: true,
            extend: false,
        },
    ),
    (
        "grid.page_down",
        GridCommand::Page {
            up: false,
            extend: false,
        },
    ),
    ("grid.next_cell", GridCommand::NextCell { back: false }),
    ("grid.prev_cell", GridCommand::NextCell { back: true }),
    ("grid.select_all", GridCommand::SelectAll),
    ("grid.select_row", GridCommand::SelectRow),
    // Palette-only in M3a (R-M3-38); bound since M3b Task 1b (P25).
    ("grid.select_column", GridCommand::SelectColumn),
    ("grid.copy", GridCommand::Copy { headers: false }),
    (
        "grid.copy_with_headers",
        GridCommand::Copy { headers: true },
    ),
    ("grid.escape", GridCommand::Escape),
    // ## m3b-t01b-seams: the M3b rows. The sort and view rows (`grid.sort_asc`, `sort_desc`,
    // `clear_sort`, `view_table`, `view_transposed`, `view_text`, `toggle_record`) are not
    // grid commands: the app registers them over its results (`results::{sort, views}`).
    ("grid.grow_selection", GridCommand::GrowSelection),
    ("grid.shrink_selection", GridCommand::ShrinkSelection),
    ("grid.find", GridCommand::Find),
    ("grid.goto_row", GridCommand::GotoRow),
    ("grid.open_value", GridCommand::OpenValue),
    ("grid.context_menu", GridCommand::ContextMenu),
    ("grid.auto_fit", GridCommand::AutoFit),
    ("grid.freeze", GridCommand::Freeze),
    ("grid.unfreeze", GridCommand::Unfreeze),
    // ## m3c-t37 (named exception; NOTES Plan notes, Task 37): the grid's own Select all
    // occurrences row (⌃⌘G in `Grid`), one chord with the editor's, its own id.
    (
        "grid.select_all_occurrences",
        GridCommand::SelectAllOccurrences,
    ),
];

// ## m3b-t01b-seams (fix round 2, I3)
/// The grid commands that do nothing until their task fills them (Tasks 37, 38, 39): the app
/// registers them disabled, so the palette does not list them and their keys pass through.
/// The filling task removes its ids (a named exception).
///
/// ## m3c-t35 (named exception): Task 35 removed `grid.grow_selection`,
/// `grid.shrink_selection`, `grid.auto_fit`, `grid.freeze` and `grid.unfreeze`.
/// ## m3c-t37 (named exception): Task 37 removed `grid.find` and `grid.goto_row`.
/// ## m3c-t38 (named exception): Task 38 removed `grid.open_value` (the value viewer).
/// ## m3c-t39 (named exception): Task 39 removed `grid.context_menu` (the grid menu): none left.
pub static STUB_COMMANDS: [&str; 0] = [];

/// Whether grid command `id` is a stub ([`STUB_COMMANDS`]).
pub fn is_stub(id: &str) -> bool {
    STUB_COMMANDS.contains(&id)
}

// ## m3c-t40 fix round 1 (m6)
/// Grid commands that keep their keys but stay out of the command palette: Tab and Shift+Tab
/// leave the grid from any cell since R-M3-69, so their titles ("Grid: next cell") no longer
/// say what they do. A later task may retitle them in its own block.
pub static PALETTE_HIDDEN: [&str; 2] = ["grid.next_cell", "grid.prev_cell"];

/// Whether grid command `id` stays out of the palette ([`PALETTE_HIDDEN`]).
pub fn hidden_in_palette(id: &str) -> bool {
    PALETTE_HIDDEN.contains(&id)
}

const fn mv(dir: Dir, extend: bool) -> GridCommand {
    GridCommand::Move { dir, extend }
}

const fn jump(dir: Dir) -> GridCommand {
    GridCommand::JumpEdge { dir, extend: false }
}

/// The command of `id`; `None` for an id that is not a grid command.
pub fn grid_command(id: &str) -> Option<GridCommand> {
    GRID_COMMANDS
        .iter()
        .find(|(name, _)| *name == id)
        .map(|(_, command)| *command)
}

#[cfg(test)]
mod tests {
    use hacivat_ui::keymaps::{PRESETS, load};
    use hacivat_ui_core::keymap::KeymapPlatform;

    use super::*;

    /// A typo in a keymap row's grid id fails here, as the editor's table test does.
    #[test]
    fn every_bound_grid_command_is_in_the_table() {
        for p in [
            KeymapPlatform::MacOs,
            KeymapPlatform::Windows,
            KeymapPlatform::Linux,
        ] {
            for preset in PRESETS {
                for b in load(p, preset, None).0.bindings() {
                    if b.command.starts_with("grid.") {
                        assert!(grid_command(&b.command).is_some(), "{} {p:?}", b.command);
                    }
                }
            }
        }
        assert!(grid_command("grid.select_column").is_some());
        assert!(grid_command("grid.fly").is_none());
    }

    #[test]
    fn ids_are_unique() {
        let mut ids: Vec<&str> = GRID_COMMANDS.iter().map(|(id, _)| *id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), GRID_COMMANDS.len());
    }
}
