//! The localization runtime and its atomic swap (I18N-04, I18N-07). Bundles and rendering in
//! Task 6; the generation bookkeeping (F8) is complete.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use arc_swap::ArcSwap;
use unic_langid::LanguageIdentifier;

use crate::args::{ArgValue, IntoFluentArgs, MsgRef};
use crate::catalog::{Catalogs, LoadDiag};
use crate::format::FormatCtx;
use crate::negotiate::Negotiated;

/// What the formatters need besides the UI chain (F7).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FormatInputs {
    pub format_locale: String,
    /// An IANA zone such as `Europe/Istanbul`.
    pub time_zone: String,
}

impl FormatInputs {
    pub fn new(neg: &Negotiated, time_zone: &str) -> FormatInputs {
        FormatInputs {
            format_locale: neg.format_locale.clone(),
            time_zone: time_zone.to_string(),
        }
    }
}

/// One built localization state: the UI chain's bundles and the format context. Task 6 adds the
/// bundles (two sets per locale: UI isolating, machine not); hence no `PartialEq`.
#[derive(Debug, Clone, Default)]
pub struct Runtime {
    pub generation: u64,
    pub ui_chain: Vec<LanguageIdentifier>,
    pub format: FormatCtx,
    pub diags: Vec<LoadDiag>,
}

/// Builds the runtime for `neg` from `cat`. The pseudo mode comes from
/// `PseudoMode::for_tag(&neg.chain[0])`. Stub: no bundles.
pub fn build_runtime(
    cat: &Catalogs,
    neg: &Negotiated,
    fmt: &FormatInputs,
    generation: u64,
) -> Runtime {
    Runtime {
        generation,
        ui_chain: neg.chain.clone(),
        format: FormatCtx {
            format_locale: fmt.format_locale.clone(),
            time_zone: fmt.time_zone.clone(),
            generation,
        },
        diags: cat.diags().to_vec(),
    }
}

/// The installed runtime, swapped atomically on a language switch.
pub struct L10n {
    current: ArcSwap<Runtime>,
    reserved: AtomicU64,
}

impl std::fmt::Debug for L10n {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("L10n")
            .field("generation", &self.generation())
            .finish_non_exhaustive()
    }
}

impl L10n {
    pub fn new(rt: Runtime) -> L10n {
        let reserved = AtomicU64::new(rt.generation);
        L10n {
            current: ArcSwap::from_pointee(rt),
            reserved,
        }
    }

    /// Renders `msg` for the UI sink (placeables isolated with U+2068/U+2069). Stub: the ID.
    pub fn t<A: IntoFluentArgs>(&self, msg: MsgRef<A>, args: A) -> String {
        let _ = args.into_args();
        msg.id.to_string()
    }

    /// Renders attribute `attr` of `msg`. Stub: `None`.
    pub fn t_attr<A: IntoFluentArgs>(&self, msg: MsgRef<A>, attr: &str, args: A) -> Option<String> {
        let _ = (msg, attr, args.into_args());
        None
    }

    /// Renders `msg` for a machine sink (no isolation marks). Stub: the ID.
    pub fn t_machine<A: IntoFluentArgs>(&self, msg: MsgRef<A>, args: A) -> String {
        let _ = args.into_args();
        msg.id.to_string()
    }

    /// Renders a dynamic ID (`hui-*`, `hv-cmd-*`) for the UI sink. Stub: the ID.
    pub fn t_dynamic(&self, id: &str, args: Vec<(&'static str, ArgValue)>) -> String {
        let _ = args;
        id.to_string()
    }

    /// Renders `id` from the `en` bundle only (the English search layer). Stub: the ID.
    pub fn t_english(&self, id: &str) -> String {
        id.to_string()
    }

    /// The installed runtime.
    pub fn snapshot(&self) -> Arc<Runtime> {
        self.current.load_full()
    }

    /// Installs `rt` unless a runtime of the same or a newer generation is installed.
    pub fn install_if_newer(&self, rt: Runtime) -> bool {
        let new = Arc::new(rt);
        let mut cur = self.current.load_full();
        loop {
            if new.generation <= cur.generation {
                return false;
            }
            let prev = self.current.compare_and_swap(&cur, Arc::clone(&new));
            if Arc::ptr_eq(&prev, &cur) {
                return true;
            }
            cur = arc_swap::Guard::into_inner(prev);
        }
    }

    /// The installed runtime's generation.
    pub fn generation(&self) -> u64 {
        self.current.load().generation
    }

    /// A generation greater than every earlier reservation and than the installed one. A
    /// language switch reserves at call time, so the last switch requested wins (F8).
    pub fn reserve_generation(&self) -> u64 {
        self.reserved.fetch_max(self.generation(), Ordering::SeqCst);
        self.reserved.fetch_add(1, Ordering::SeqCst) + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rt(generation: u64) -> Runtime {
        Runtime {
            generation,
            ..Runtime::default()
        }
    }

    #[test]
    fn l10n_can_be_shared_across_threads() {
        fn send_sync<T: Send + Sync>() {}
        send_sync::<L10n>();
        send_sync::<Runtime>();
    }

    #[test]
    fn older_or_equal_generation_is_not_installed() {
        let l = L10n::new(rt(1));
        assert!(l.install_if_newer(rt(3)));
        assert!(!l.install_if_newer(rt(2)));
        assert!(!l.install_if_newer(rt(3)));
        assert_eq!(l.generation(), 3);
        assert_eq!(l.snapshot().generation, 3);
    }

    #[test]
    fn reservations_increase_past_the_installed_generation() {
        let l = L10n::new(rt(5));
        let a = l.reserve_generation();
        let b = l.reserve_generation();
        assert!(a > 5 && b > a, "{a} {b}");
        assert!(l.install_if_newer(rt(b)));
        assert!(!l.install_if_newer(rt(a)));
        assert!(l.install_if_newer(rt(b + 10)));
        assert!(l.reserve_generation() > b + 10);
    }

    #[test]
    fn format_inputs_take_the_negotiated_format_locale() {
        let neg = Negotiated {
            chain: vec![unic_langid::langid!("en")],
            format_locale: "tr-TR".into(),
            diags: Vec::new(),
        };
        let f = FormatInputs::new(&neg, "Europe/Istanbul");
        assert_eq!(
            f,
            FormatInputs {
                format_locale: "tr-TR".into(),
                time_zone: "Europe/Istanbul".into()
            }
        );
        let r = build_runtime(&Catalogs::default(), &neg, &f, 7);
        assert_eq!((r.generation, r.format.generation), (7, 7));
        assert_eq!(r.format.time_zone, "Europe/Istanbul");
    }
}
