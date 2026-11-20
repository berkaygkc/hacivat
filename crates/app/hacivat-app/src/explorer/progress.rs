//! Progress rows (Task 24): a static phase label plus a determinate count, repainted only when
//! the count changes (no ambient motion; mock 2, surface 14 "explorer only").
//!
//! ## m4-t24-live
//!
//! The engine reports one [`TaskProgress`] per running task (its namespace, a closed [`Phase`],
//! `done` of `total`). A namespace being read for the first time shows one child row: the run
//! marker, the phase label ("Reading names…", "Reading columns…", "Reading keys and indexes…",
//! "Reading definitions…") and the count as its own preformatted text (`3,400/15,000`), never
//! concatenated in code. The host stores a new [`Progress`] only when it differs from the last
//! one (`live::flush`), so the row repaints only when its count changes. The profile root of a
//! first connect shows "Loading…" until the first names arrive.
//!
//! Counts inside one pass come with Task 17's progress sink (R-W3-12); until then the engine
//! counts the tasks of its batch.

use hacivat_introspect::engine::{Phase, TaskProgress};
use hacivat_l10n::ArgValue;
use hacivat_meta::NsKey;

use super::live::LiveRow;

/// The phase label of `phase` (a Fluent id).
pub fn phase_message(phase: Phase) -> &'static str {
    match phase {
        Phase::ListNamespaces | Phase::Tables | Phase::Existence | Phase::Validate => {
            "hv-explorer-phase-names"
        }
        Phase::Columns => "hv-explorer-phase-columns",
        Phase::Indexes | Phase::Constraints | Phase::Triggers => "hv-explorer-phase-constraints",
        Phase::Routines
        | Phase::Types
        | Phase::Sequences
        | Phase::Comments
        | Phase::Sources
        | Phase::Facets => "hv-explorer-phase-sources",
    }
}

/// The count's message and arguments (`{ $n }/{ $total }`), when the total is known.
pub fn count_args(task: &TaskProgress) -> Option<Vec<(&'static str, ArgValue)>> {
    let total = task.total?;
    Some(vec![
        ("n", ArgValue::Count(task.done.min(total))),
        ("total", ArgValue::Count(total)),
    ])
}

/// The count's Fluent id.
pub const COUNT_MESSAGE: &str = "hv-explorer-progress-count";

/// The task reading `ns`, if any.
pub fn task_for<'a>(row: &'a LiveRow, ns: &NsKey) -> Option<&'a TaskProgress> {
    row.progress.tasks.iter().find(|t| &t.ns == ns)
}

/// The task listing the namespaces (a first connect's "Loading…" row).
pub fn listing(row: &LiveRow) -> Option<&TaskProgress> {
    row.progress
        .tasks
        .iter()
        .find(|t| t.phase == Phase::ListNamespaces)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_phase_has_a_label_and_counts_need_a_total() {
        for p in [
            Phase::ListNamespaces,
            Phase::Tables,
            Phase::Columns,
            Phase::Indexes,
            Phase::Constraints,
            Phase::Triggers,
            Phase::Routines,
            Phase::Types,
            Phase::Sequences,
            Phase::Comments,
            Phase::Sources,
            Phase::Facets,
            Phase::Existence,
            Phase::Validate,
        ] {
            assert!(phase_message(p).starts_with("hv-explorer-phase-"));
        }
        let mut t = TaskProgress {
            ns: NsKey::schema(["app", "public"]),
            phase: Phase::Columns,
            done: 3,
            total: None,
        };
        assert!(count_args(&t).is_none());
        t.total = Some(2);
        let args = count_args(&t).expect("a count");
        assert_eq!(
            args[0].1,
            ArgValue::Count(2),
            "done never exceeds the total"
        );
    }
}
