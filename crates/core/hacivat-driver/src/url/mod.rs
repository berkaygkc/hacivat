//! Connection URLs (HV-DRV-02 to -04): DataGrip-style templates, rendering for display, copy
//! and the driver, matching typed text against a template, and parsing native URL forms.
//! Task 3 fills `template`, `render` and `matcher`; Task 7 fills `parse` and `native`. This
//! file is complete since Task 1: the submodules are public, so a later task can expose a new
//! item without editing it.

pub mod matcher;
pub mod native;
pub mod parse;
pub mod render;
pub mod template;

use std::collections::BTreeMap;

pub use matcher::match_template;
pub use parse::{
    MappedParam, MappedTo, ParseOutcome, ParseResult, ParseWarning, UnrecognisedReason,
    looks_like_connection, parse,
};
pub use render::{RenderError, RenderTarget, render, render_driver, rewrite_endpoint};
pub use template::{Template, TemplateError, compile};

/// A field a URL template can carry (HV-DRV-02).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FieldId {
    Host,
    Port,
    Database,
    User,
    Password,
    File,
    Socket,
}

impl FieldId {
    pub const ALL: [FieldId; 7] = [
        FieldId::Host,
        FieldId::Port,
        FieldId::Database,
        FieldId::User,
        FieldId::Password,
        FieldId::File,
        FieldId::Socket,
    ];

    /// The template name: `host`, `port`, `database`, `user`, `password`, `file`, `socket`.
    pub fn id(self) -> &'static str {
        match self {
            FieldId::Host => "host",
            FieldId::Port => "port",
            FieldId::Database => "database",
            FieldId::User => "user",
            FieldId::Password => "password",
            FieldId::File => "file",
            FieldId::Socket => "socket",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.id() == id)
    }
}

/// Field values for a template; `hosts` keeps every host of a multi-host URL (HV-DRV-02).
/// Never holds a password: secrets travel separately as `SecretString`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct FieldMap {
    pub values: BTreeMap<FieldId, String>,
    pub hosts: Vec<(String, Option<u16>)>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_ids_round_trip() {
        for f in FieldId::ALL {
            assert_eq!(FieldId::from_id(f.id()), Some(f));
        }
        assert_eq!(FieldId::from_id("Host"), None);
        assert!(FieldMap::default().values.is_empty());
    }
}
