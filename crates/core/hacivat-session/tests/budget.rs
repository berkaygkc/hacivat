//! Connection budgets (CONN-02, D-035, R13).

mod common;

use common::*;
use hacivat_net::TlsMode;
use hacivat_session::{Budget, Priority, budget_for};
use tokio::time::timeout;

#[tokio::test]
async fn budget_of_four_queues_the_fifth_and_serves_priority_first() {
    let b = Budget::new(4);
    let mut held: Vec<_> = (0..4)
        .map(|_| b.try_acquire(Priority::Console).unwrap())
        .collect();
    assert!(b.try_acquire(Priority::Console).is_none());
    let intro = tokio::spawn(b.acquire(Priority::Introspection));
    let console = tokio::spawn(b.acquire(Priority::Console));
    // One slot comes back; the other three stay held, and so does the one Console gets.
    drop(held.remove(0));
    let served = timeout(ms(50), console).await;
    assert!(served.is_ok());
    assert!(timeout(ms(50), intro).await.is_err());
    assert_eq!((b.in_use(), held.len()), (4, 3));
}

#[tokio::test]
async fn a_given_up_wait_passes_the_slot_on() {
    let b = Budget::new(1);
    let first = b.try_acquire(Priority::Console).unwrap();
    let gone = b.acquire(Priority::Console);
    let next = tokio::spawn(b.acquire(Priority::Job));
    drop(gone);
    drop(first);
    let permit = timeout(ms(50), next).await.unwrap().unwrap();
    assert_eq!(b.in_use(), 1);
    drop(permit);
    assert_eq!(b.in_use(), 0);
    assert!(b.try_acquire(Priority::Introspection).is_some());
    // A wait that ends before its permit is read still returns the slot.
    let held = b.try_acquire(Priority::Console).unwrap();
    let pending = b.acquire(Priority::Console);
    drop(held);
    drop(pending);
    assert_eq!(b.in_use(), 0);
}

#[test]
fn sqlite_budget_is_one() {
    assert_eq!(budget_for(&sqlite_profile()).max(), 1);
    assert_eq!(budget_for(&pg_profile(TlsMode::Auto)).max(), 4);
}
