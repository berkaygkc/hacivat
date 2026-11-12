//! Rendering a template for display, copy and the driver, and rewriting its endpoint
//! (HV-DRV-03). Bodies in Task 3.

use secrecy::SecretString;

use super::{FieldId, FieldMap, Template};

/// Who a rendered URL is for (HV-DRV-03). Display and Copy never contain the password.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RenderTarget {
    /// The URL row; `has_password` shows a mask where the password goes.
    Display {
        has_password: bool,
    },
    Copy,
}

/// Why a template could not be rendered (HV-DRV-03).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RenderError {
    MissingField(FieldId),
    FieldNotAllowed(FieldId),
    /// The endpoint cannot be rewritten (a socket or file template).
    CannotRedirect(String),
}

/// Renders `t` with `fields` and `extra` parameters for `target` (Task 3).
pub fn render(
    _t: &Template,
    _fields: &FieldMap,
    _extra: &[(String, String)],
    _target: RenderTarget,
) -> Result<String, RenderError> {
    Err(RenderError::MissingField(FieldId::Host))
}

/// Renders the driver URL, password included, as a secret (Task 3).
pub fn render_driver(
    _t: &Template,
    _fields: &FieldMap,
    _extra: &[(String, String)],
    _password: &SecretString,
) -> Result<SecretString, RenderError> {
    Err(RenderError::MissingField(FieldId::Host))
}

/// `url` pointed at `host:port` instead (a forwarder) (Task 3).
pub fn rewrite_endpoint(
    _t: &Template,
    _url: &str,
    _host: &str,
    _port: u16,
) -> Result<String, RenderError> {
    Err(RenderError::MissingField(FieldId::Host))
}
