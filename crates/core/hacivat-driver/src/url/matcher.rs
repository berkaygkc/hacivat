//! Matching typed text against a template (HV-DRV-03). Body in Task 3.

use super::{FieldMap, Template};

/// The fields and extra parameters of `input`, if it matches `t` (Task 3).
pub fn match_template(_t: &Template, _input: &str) -> Option<(FieldMap, Vec<(String, String)>)> {
    None
}
