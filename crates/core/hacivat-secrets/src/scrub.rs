//! Scrubbing: removes passwords from URLs and option lists before they reach a log, the
//! clipboard or a file (HV-SEC-05). Bodies in Task 4.

/// `url` without its password, wherever it appears (userinfo or a password option). The stub
/// returns an empty string, so nothing unscrubbed leaks before Task 4.
pub fn scrub_url(_url: &str) -> String {
    String::new()
}

/// `opts` without password-bearing options. The stub returns nothing.
pub fn scrub_options(_opts: &[(String, String)]) -> Vec<(String, String)> {
    Vec::new()
}
