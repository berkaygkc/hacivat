//! A console's result tabs (Task 26): Output first, then one tab per unit result keyed
//! `TabKey`; a re-run of the same key reuses its tab; titles `Result N`.
//!
//! Task 1b: the types, an empty set (Output only) and its accessors.

use std::sync::Arc;

use hacivat_exec::{ResultTab, SessionLog};

/// The tab a console's results show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ActiveResult {
    /// The output log.
    #[default]
    Output,
    /// The result tab at this index (after Output).
    Tab(usize),
}

/// A console's results: its output log, its result tabs, the active one and whether the
/// output has lines the user has not seen.
#[derive(Debug)]
pub struct ConsoleResults {
    output: Arc<SessionLog>,
    tabs: Vec<ResultTab>,
    active: ActiveResult,
    unread_output: bool,
}

impl ConsoleResults {
    /// No result tabs yet; the Output tab shows `output`.
    pub fn new(output: Arc<SessionLog>) -> Self {
        ConsoleResults {
            output,
            tabs: Vec::new(),
            active: ActiveResult::Output,
            unread_output: false,
        }
    }

    pub fn output(&self) -> &Arc<SessionLog> {
        &self.output
    }

    pub fn tabs(&self) -> &[ResultTab] {
        &self.tabs
    }

    pub fn active(&self) -> ActiveResult {
        self.active
    }

    pub fn unread_output(&self) -> bool {
        self.unread_output
    }
}
