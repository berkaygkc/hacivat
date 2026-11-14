//! Splitter profiles: block openers and closers, statement starts, client commands
//! (SQL-SPLIT-02; Task 8, from `data/split/*.toml`).
//!
//! Task 1b stub: one empty placeholder profile for every dialect.

use crate::id::DialectCtx;

/// One dialect's splitter tables (opaque; Task 8 fills it).
#[derive(Debug)]
pub struct SplitProfile {
    name: &'static str,
}

impl SplitProfile {
    /// The profile's name, for diagnostics.
    pub fn name(&self) -> &'static str {
        self.name
    }
}

static PLACEHOLDER: SplitProfile = SplitProfile {
    name: "placeholder",
};

/// The splitter profile for `ctx` (Task 8).
pub fn split_profile(ctx: &DialectCtx) -> &'static SplitProfile {
    let _ = ctx;
    &PLACEHOLDER
}
