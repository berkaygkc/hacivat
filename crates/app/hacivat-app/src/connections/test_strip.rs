//! The dialog's test strip: the step trail, the clock and the result (NOTES surface 1 test,
//! Task 20).

use std::sync::Arc;

use hacivat_session::TestReport;
use hacivat_session::pipeline::Progress;
use hacivat_session::profile::ProfileDraft;
use hacivat_ui_core::gpui_surface::{Context, EventEmitter, IntoElement, Render, Window, div};

use crate::connections::fixes::FixAction;

/// Where the strip is (NOTES surface 1).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TestStripState {
    Idle,
    Running(Progress),
    Done(Arc<TestReport>),
}

/// What the strip emits (NOTES surface 1).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TestStripEvent {
    /// A quick fix was chosen.
    Fix(FixAction),
    Finished,
}

/// The test strip view (NOTES surface 1).
pub struct TestStrip {
    state: TestStripState,
}

impl EventEmitter<TestStripEvent> for TestStrip {}

impl TestStrip {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        TestStrip {
            state: TestStripState::Idle,
        }
    }

    /// Tests `draft` (Task 20).
    pub fn run(&mut self, _draft: ProfileDraft, _cx: &mut Context<Self>) {}

    /// Stops a running test; `true` if one was running (Task 20).
    pub fn stop(&mut self, _cx: &mut Context<Self>) -> bool {
        false
    }

    pub fn state(&self) -> &TestStripState {
        &self.state
    }
}

impl Render for TestStrip {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}
