//! Large-file tiers (UI-EDIT-23 T0/T1; Task 29): T0 up to 4 MB, T1 ("Reduced mode") up to
//! 20 MB; back to T0 only below 80 % of its limit (no flapping). Nothing above 20 MB enters the
//! editor in M3a (R-M3-49).
//!
//! Task 1b: the tier follows the size alone (no long-line rule, no hysteresis); no banner. The
//! view renders `banner` above the text since fix round 1.

use hacivat_ui_core::gpui_surface::{AnyElement, App, Window};

use crate::config::{Tier, TierLimits};

/// The module-owned state (P11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct TierState {
    pub tier: Tier,
}

/// The tier of a document of `bytes` whose longest line has `longest_line` bytes (Task 29).
pub fn tier_for(bytes: u64, longest_line: u64, limits: &TierLimits) -> Tier {
    let _ = longest_line;
    if bytes <= limits.full_max_bytes {
        Tier::Full
    } else {
        Tier::Reduced
    }
}

/// The tier after a change; `Some` when it moved (Task 29 adds the 80 % rule).
pub fn update(
    s: &mut TierState,
    bytes: u64,
    longest_line: u64,
    limits: &TierLimits,
) -> Option<Tier> {
    let tier = tier_for(bytes, longest_line, limits);
    (tier != s.tier).then(|| {
        s.tier = tier;
        tier
    })
}

/// The inline "Reduced mode" banner the view shows above the text in T1 (Task 29, its copy in
/// `hui.ftl`); `None` hides it. Task 1b: none.
pub fn banner(s: &TierState, window: &mut Window, cx: &mut App) -> Option<AnyElement> {
    let _ = (s, window, cx);
    None
}
