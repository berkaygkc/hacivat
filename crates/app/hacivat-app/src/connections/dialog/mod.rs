//! The connection dialog (NOTES surfaces 1 and 2): the form, the URL row, paste and copy
//! (Task 17).

pub mod form;
pub mod paste;
pub mod url_row;

use hacivat_proto::ProfileId;
use hacivat_ui_core::gpui_surface::{App, Window};

/// What the dialog opens on (NOTES surface 1).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DialogMode {
    New,
    /// A new profile prefilled from pasted connection text.
    NewFromText(String),
    Edit(ProfileId),
}

/// Opens the connection dialog as a modal layer (Task 17). The stub does nothing.
pub fn open_connection_dialog(_mode: DialogMode, _window: &mut Window, _cx: &mut App) {}
