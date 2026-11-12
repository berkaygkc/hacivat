//! Step trail (NOTES surface 1 test strip): the connect steps with their state and durations.
//! Task 9 draws it; the Task 1 stub renders an empty `div`.

use hacivat_ui_core::gpui_surface::{AnyElement, IntoElement, SharedString, div};

/// A step's state; a finished step may show its duration (formatted by Fluent).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TrailState {
    Pending,
    Running,
    Done { duration: Option<SharedString> },
    Failed,
}

/// One step of a trail.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TrailItem {
    pub label: SharedString,
    pub state: TrailState,
}

/// A row of connect steps (NOTES surface 1).
pub struct StepTrail {
    #[allow(dead_code, reason = "Task 9")]
    items: Vec<TrailItem>,
}

impl StepTrail {
    pub fn new(items: Vec<TrailItem>) -> Self {
        StepTrail { items }
    }
}

impl IntoElement for StepTrail {
    type Element = AnyElement;

    fn into_element(self) -> AnyElement {
        div().into_any_element()
    }
}
