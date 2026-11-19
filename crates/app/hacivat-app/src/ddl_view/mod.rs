//! The DDL view tab (Task 25; design/m4-explorer surfaces 10 and 11): a read-only editor with the
//! object's script (Task 19's `create_script`), the options strip, Copy, Open in console and
//! Refresh, and the stale, gone, unsupported-kind and executable-comment notices.
//!
//! ## m4-t01b-seams: stub. The tab kind ([`DDL_VIEW_KIND`]) and its key context
//! ([`DDL_VIEW_CONTEXT`]) are declared, and `ddl_view.copy` is registered **disabled**; nothing
//! opens the tab until Task 25 registers its view and fills `open_ddl_view`.

use std::rc::Rc;

use hacivat_ui::actions::{ActionDef, ActionRegistry};
use hacivat_ui_core::gpui_surface::App;

pub use crate::shell::tab_content::DDL_VIEW_KIND;

/// The key context while a DDL view tab is active (`ddl_view.copy`'s row).
pub const DDL_VIEW_CONTEXT: &str = "DdlView";

/// The DDL view commands that are still stubs (registered disabled; Task 25 fills them).
pub const STUB_COMMANDS: [&str; 1] = ["ddl_view.copy"];

/// Why a DDL view did not open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DdlViewError {
    /// The DDL view is not built yet (Task 25).
    NotYet,
}

fn not_yet(_: &App) -> bool {
    false
}

/// Registers the DDL view's commands (one line in `crate::init`).
pub fn install(cx: &mut App) {
    let _ = cx.default_global::<ActionRegistry>().register(ActionDef {
        id: "ddl_view.copy",
        title_id: "hv-cmd-ddl-view-copy",
        context: DDL_VIEW_CONTEXT,
        run: Rc::new(|_, _| {}),
        enabled: not_yet,
    });
}
