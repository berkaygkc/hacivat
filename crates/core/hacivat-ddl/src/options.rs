//! Generator options (HV-DDL-11 v1 core subset; R-M4-23 defaults). Defaults favour safety over
//! DataGrip's: nothing is replaced, nothing is skipped, constraints inside the table,
//! qualification `Auto` (display; executed scripts are always qualified, HV-DDL-10).

/// Qualification and placement are the DDL view's remembered options too, so they are
/// declared once in `hacivat_meta::userdata`.
pub use hacivat_meta::userdata::{Placement, Qualify};

/// The typed option set of one generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GenOptions {
    pub qualify: Qualify,
    pub placement: Placement,
    pub or_replace: bool,
    pub if_not_exists: bool,
    pub skip_grants: bool,
    pub skip_owner: bool,
    /// MySQL/MariaDB only.
    pub skip_definer: bool,
    pub surrogate_names: bool,
    /// "Definitions provided by server" (R-M4-24).
    pub server_definitions: bool,
}

impl Default for GenOptions {
    fn default() -> Self {
        GenOptions {
            qualify: Qualify::Auto,
            placement: Placement::InsideTable,
            or_replace: false,
            if_not_exists: false,
            skip_grants: false,
            skip_owner: false,
            skip_definer: false,
            surrogate_names: false,
            server_definitions: false,
        }
    }
}
