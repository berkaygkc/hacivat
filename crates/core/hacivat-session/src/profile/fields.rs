//! Field mapping between profiles and URLs: fields for a template, applying a parsed URL to a
//! draft, display and copy URLs (HV-DRV-03, -04). Bodies in Task 12.

use hacivat_driver::url::{
    FieldId, FieldMap, MappedParam, ParseOutcome, ParseWarning, RenderError,
};
use hacivat_proto::DbmsKind;

use super::{Profile, ProfileDraft};

/// How applying a URL changed the engine (HV-DRV-04).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EngineChange {
    Switched {
        from: DbmsKind,
        to: DbmsKind,
    },
    /// The URL names another engine and the form keeps its own (not forced).
    Mismatch {
        url: DbmsKind,
        form: DbmsKind,
    },
}

/// What applying a parsed URL did to a draft, for the inline notice (HV-DRV-04).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ApplyReport {
    pub filled: Vec<FieldId>,
    pub engine: Option<EngineChange>,
    pub password_moved: bool,
    pub mapped: Vec<MappedParam>,
    pub extras_kept: Vec<String>,
    pub warnings: Vec<ParseWarning>,
}

/// A URL for the clipboard, never with the password (HV-DRV-03).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CopiedUrl {
    pub url: String,
    pub omitted_password: bool,
    pub omitted_tunnel: bool,
}

/// The profile's connection type, field values and extra parameters (Task 12).
pub fn to_fields(_p: &Profile) -> (String, FieldMap, Vec<(String, String)>) {
    (String::new(), FieldMap::default(), Vec::new())
}

/// Applies a parsed URL to `draft`; touched fields are kept (Task 12).
pub fn apply_parse(
    _draft: &mut ProfileDraft,
    _outcome: ParseOutcome,
    _force_engine: bool,
) -> ApplyReport {
    ApplyReport {
        filled: Vec::new(),
        engine: None,
        password_moved: false,
        mapped: Vec::new(),
        extras_kept: Vec::new(),
        warnings: Vec::new(),
    }
}

/// The URL row's text, a mask where the password goes (Task 12).
pub fn display_url(_p: &Profile, _has_password: bool) -> Option<String> {
    None
}

/// The URL "Copy as URL" puts on the clipboard (Task 12).
pub fn copy_url(_p: &Profile) -> Result<CopiedUrl, RenderError> {
    Err(RenderError::MissingField(FieldId::Host))
}
