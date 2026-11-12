//! Naming and environment rules (D-004, R22, P7). Bodies in Task 12.

use super::{Profile, ProfileEndpoint};

/// Where a production hint was found (R22, P7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HintSource {
    Host,
    Name,
}

/// Whether the endpoint is on this machine (loopback, a socket or a file) (Task 12).
pub fn is_local_endpoint(_e: &ProfileEndpoint) -> bool {
    false
}

/// A name for an unnamed profile (`shop-local`) (Task 12).
pub fn suggest_name(_p: &Profile) -> String {
    String::new()
}

/// The production token in `host`, then `name`: whole tokens split on `[-._/:0-9]`, compared
/// with `eq_ignore_ascii_case` against `prod`, `prd`, `live` and `production` (reported as
/// `"prod"`); never a substring (R22, P7, Task 12).
pub fn production_hint(_host: &str, _name: &str) -> Option<(&'static str, HintSource)> {
    None
}

/// `name` made unique among `existing` (`orders (2)`) (Task 12).
pub fn duplicate_name(_name: &str, _existing: &[&str]) -> String {
    String::new()
}

/// Whether connecting should first ask for an environment tag (Task 12).
pub fn needs_environment_prompt(_p: &Profile) -> bool {
    false
}
