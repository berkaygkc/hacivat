//! Hashes the classifier attaches to a report (Task 14): the request's fingerprint and the
//! hash of the exact bytes classified (P-M3b-2), which the exec gate compares with the text it
//! is about to send.

use twox_hash::XxHash3_64;

/// XXH3-64 of `text`'s bytes: `SafetyReport.classified_hash`.
pub fn classified_hash(text: &str) -> u64 {
    XxHash3_64::oneshot(text.as_bytes())
}
