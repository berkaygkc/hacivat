//! The keys of a single-line text field (inputnav-1, PO demo finding F1): the `TextInput` key
//! context of the keymap data and the commands it binds.
//!
//! A focused [`TextInput`](super::TextInput) resolves its own key presses against
//! [`InputKeymap`] in the context [`TEXT_INPUT_CONTEXT`], in its key-down listener: after the
//! workspace dispatcher, which sees every key first with the workspace's context stack. So the
//! rows live in the keymap files (users remap them like any other) and never become GPUI key
//! bindings (`menus::rebuild` owns those). The app keeps `InputKeymap` equal to its active
//! keymap; without it (a widget test, a window outside the app) the host's default keymap
//! applies.

use std::sync::Arc;

use hacivat_ui_core::gpui_surface::{App, Global, Keystroke};
use hacivat_ui_core::keymap::{
    ContextStack, KeyboardLayoutId, Keymap, KeymapPlatform, Resolution, normalize,
};

/// The key context of a focused single-line text field.
pub const TEXT_INPUT_CONTEXT: &str = "TextInput";

/// What a text field does for a bound key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputCommand {
    MoveLeft,
    MoveRight,
    SelectLeft,
    SelectRight,
    MoveWordLeft,
    MoveWordRight,
    SelectWordLeft,
    SelectWordRight,
    MoveLineStart,
    MoveLineEnd,
    SelectLineStart,
    SelectLineEnd,
    DeleteBackward,
    DeleteForward,
    DeleteWordBackward,
    DeleteWordForward,
    DeleteToLineStart,
    DeleteToLineEnd,
    SelectAll,
    Copy,
    Cut,
    Paste,
}

/// Every text-field command: its keymap ID, its title's Fluent ID (`hv-cmd-<id with . and _ as
/// ->`, R5) and what it does. The app registers each ID so the keymap's rows name known
/// commands; the field runs them, never the palette.
pub const INPUT_COMMANDS: [(&str, &str, InputCommand); 22] = [
    (
        "input.move_left",
        "hv-cmd-input-move-left",
        InputCommand::MoveLeft,
    ),
    (
        "input.move_right",
        "hv-cmd-input-move-right",
        InputCommand::MoveRight,
    ),
    (
        "input.select_left",
        "hv-cmd-input-select-left",
        InputCommand::SelectLeft,
    ),
    (
        "input.select_right",
        "hv-cmd-input-select-right",
        InputCommand::SelectRight,
    ),
    (
        "input.move_word_left",
        "hv-cmd-input-move-word-left",
        InputCommand::MoveWordLeft,
    ),
    (
        "input.move_word_right",
        "hv-cmd-input-move-word-right",
        InputCommand::MoveWordRight,
    ),
    (
        "input.select_word_left",
        "hv-cmd-input-select-word-left",
        InputCommand::SelectWordLeft,
    ),
    (
        "input.select_word_right",
        "hv-cmd-input-select-word-right",
        InputCommand::SelectWordRight,
    ),
    (
        "input.move_line_start",
        "hv-cmd-input-move-line-start",
        InputCommand::MoveLineStart,
    ),
    (
        "input.move_line_end",
        "hv-cmd-input-move-line-end",
        InputCommand::MoveLineEnd,
    ),
    (
        "input.select_line_start",
        "hv-cmd-input-select-line-start",
        InputCommand::SelectLineStart,
    ),
    (
        "input.select_line_end",
        "hv-cmd-input-select-line-end",
        InputCommand::SelectLineEnd,
    ),
    (
        "input.delete_backward",
        "hv-cmd-input-delete-backward",
        InputCommand::DeleteBackward,
    ),
    (
        "input.delete_forward",
        "hv-cmd-input-delete-forward",
        InputCommand::DeleteForward,
    ),
    (
        "input.delete_word_backward",
        "hv-cmd-input-delete-word-backward",
        InputCommand::DeleteWordBackward,
    ),
    (
        "input.delete_word_forward",
        "hv-cmd-input-delete-word-forward",
        InputCommand::DeleteWordForward,
    ),
    (
        "input.delete_to_line_start",
        "hv-cmd-input-delete-to-line-start",
        InputCommand::DeleteToLineStart,
    ),
    (
        "input.delete_to_line_end",
        "hv-cmd-input-delete-to-line-end",
        InputCommand::DeleteToLineEnd,
    ),
    (
        "input.select_all",
        "hv-cmd-input-select-all",
        InputCommand::SelectAll,
    ),
    ("input.copy", "hv-cmd-input-copy", InputCommand::Copy),
    ("input.cut", "hv-cmd-input-cut", InputCommand::Cut),
    ("input.paste", "hv-cmd-input-paste", InputCommand::Paste),
];

impl InputCommand {
    /// The command behind keymap ID `id`.
    pub fn from_id(id: &str) -> Option<Self> {
        INPUT_COMMANDS
            .iter()
            .find(|(name, _, _)| *name == id)
            .map(|(_, _, cmd)| *cmd)
    }

    /// Whether the command changes the text (refused by a read-only field).
    pub fn edits(self) -> bool {
        matches!(
            self,
            Self::DeleteBackward
                | Self::DeleteForward
                | Self::DeleteWordBackward
                | Self::DeleteWordForward
                | Self::DeleteToLineStart
                | Self::DeleteToLineEnd
                | Self::Cut
                | Self::Paste
        )
    }
}

/// The keymap text fields resolve against, and the keyboard layout it applies on.
#[derive(Debug, Clone)]
pub struct InputKeymap {
    pub keymap: Arc<Keymap>,
    pub layout: KeyboardLayoutId,
}

impl Global for InputKeymap {}

impl InputKeymap {
    /// The default keymap of `platform` (preset `hacivat`) on a US layout.
    pub fn defaults(platform: KeymapPlatform) -> Self {
        InputKeymap {
            keymap: Arc::new(crate::keymaps::load(platform, "hacivat", None).0),
            layout: KeyboardLayoutId("us".to_string()),
        }
    }

    /// The command `keystroke` runs in a text field; `None` leaves the key to the platform
    /// text input (typing, AltGr text, a dead key) or to the field's parents.
    pub fn command_for(&self, keystroke: &Keystroke) -> Option<InputCommand> {
        let press = normalize(keystroke)?;
        let context = ContextStack(vec![TEXT_INPUT_CONTEXT]);
        match self
            .keymap
            .resolve(&context, &[], &press, &self.layout, false)
        {
            Resolution::Command(id) => InputCommand::from_id(&id),
            Resolution::Pending | Resolution::PassThrough => None,
        }
    }
}

/// The keymap platform of the host.
pub fn host_platform() -> KeymapPlatform {
    if cfg!(target_os = "macos") {
        KeymapPlatform::MacOs
    } else if cfg!(target_os = "windows") {
        KeymapPlatform::Windows
    } else {
        KeymapPlatform::Linux
    }
}

/// Makes text fields follow `keymap` on `layout` (the app's active keymap: a preset or user
/// remap, a layout change).
pub fn set_input_keymap(cx: &mut App, keymap: Arc<Keymap>, layout: KeyboardLayoutId) {
    cx.set_global(InputKeymap { keymap, layout });
}

/// The command `keystroke` runs in a focused text field (the host's defaults until the app
/// sets [`InputKeymap`]).
pub(super) fn command_for(keystroke: &Keystroke, cx: &mut App) -> Option<InputCommand> {
    if !cx.has_global::<InputKeymap>() {
        cx.set_global(InputKeymap::defaults(host_platform()));
    }
    cx.global::<InputKeymap>().command_for(keystroke)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_has_a_unique_id_and_a_matching_title() {
        for (i, (id, title, cmd)) in INPUT_COMMANDS.iter().enumerate() {
            assert_eq!(InputCommand::from_id(id), Some(*cmd));
            let expected = format!("hv-cmd-{}", id.replace(['.', '_'], "-"));
            assert_eq!(*title, expected);
            assert!(
                INPUT_COMMANDS[i + 1..]
                    .iter()
                    .all(|(o, _, c)| o != id && c != cmd),
                "{id}"
            );
        }
        assert_eq!(InputCommand::from_id("input.fly"), None);
    }

    /// A typo in a keymap row's command fails here (the editor's and the grid's table tests).
    #[test]
    fn every_bound_input_command_is_in_the_table() {
        for p in [
            KeymapPlatform::MacOs,
            KeymapPlatform::Windows,
            KeymapPlatform::Linux,
        ] {
            for preset in crate::keymaps::PRESETS {
                let (km, _) = crate::keymaps::load(p, preset, None);
                let mut rows = 0;
                for b in km.bindings() {
                    if b.command.starts_with("input.") || b.context == TEXT_INPUT_CONTEXT {
                        assert_eq!(b.context, TEXT_INPUT_CONTEXT, "{} {p:?}", b.command);
                        assert!(InputCommand::from_id(&b.command).is_some(), "{}", b.command);
                        rows += 1;
                    }
                }
                assert!(rows >= 21, "{p:?} {preset}: {rows}");
            }
        }
    }
}
