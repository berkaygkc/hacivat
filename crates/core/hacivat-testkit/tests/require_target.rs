//! `require_target_in_set`, the pure core of `require_target` (Q6). No Docker, no environment
//! mutation: the active set is passed in, as `targets_in_set` takes it.

use hacivat_testkit::{all_targets_unfiltered, ids_in_set, require_target_in_set};

#[test]
fn require_target_is_none_outside_the_active_set() {
    let outside = all_targets_unfiltered()
        .into_iter()
        .find(|t| !t.sets.iter().any(|s| s == "pr"))
        .expect("a target outside the pr set");
    assert!(require_target_in_set(Some("pr"), &outside.id).is_none());
    assert!(require_target_in_set(None, "no-such-id").is_none());
    assert!(require_target_in_set(Some("pr"), "no-such-id").is_none());
}

#[test]
fn require_target_returns_a_target_of_the_active_set() {
    for id in ids_in_set("pr") {
        let t = require_target_in_set(Some("pr"), &id).expect("in the pr set");
        assert_eq!(t.id, id);
        assert!(t.sets.iter().any(|s| s == "pr"), "{t:?}");
    }
    // Without a set every target is active (as `targets_in_set(None, ..)`).
    let first = &all_targets_unfiltered()[0];
    assert_eq!(
        require_target_in_set(None, &first.id).map(|t| t.id),
        Some(first.id.clone())
    );
}
