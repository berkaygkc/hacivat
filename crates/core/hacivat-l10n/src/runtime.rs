//! The localization runtime and its atomic swap (I18N-04, I18N-07): per locale of the UI chain,
//! two bundles over the same parsed resources (UI isolating, machine not), plus the format
//! context. Rendering with per-message fallback lives in `render.rs`.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use arc_swap::ArcSwap;
use fluent_bundle::FluentResource;
use unic_langid::LanguageIdentifier;

use crate::args::{ArgValue, IntoFluentArgs, MsgRef};
use crate::catalog::{Catalogs, Layer, LoadDiag};
use crate::format::{FormatCtx, install_functions};
use crate::negotiate::Negotiated;
use crate::pseudo::{PseudoMode, transform_resource};
use crate::render::{Sink, render, render_english, render_missing};

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

/// One built localization state: the UI chain's bundles and the format context.
#[derive(Debug, Clone, Default)]
pub struct Runtime {
    pub generation: u64,
    pub ui_chain: Vec<LanguageIdentifier>,
    pub format: FormatCtx,
    pub diags: Vec<LoadDiag>,
    /// The bundles of `ui_chain` (none by default).
    pub bundles: Bundles,
}

pub(crate) type Bundle = fluent_bundle::concurrent::FluentBundle<Arc<FluentResource>>;

/// The two bundles of one locale of the chain.
pub(crate) struct LocaleBundles {
    pub(crate) lang: LanguageIdentifier,
    /// Placeables wrapped in U+2068/U+2069.
    pub(crate) ui: Bundle,
    /// No isolation marks (clipboard, logs, OS surfaces).
    pub(crate) machine: Bundle,
}

/// The built bundles of a [`Runtime`], shared between its clones.
#[derive(Clone, Default)]
pub struct Bundles(pub(crate) Arc<BundleSet>);

#[derive(Default)]
pub(crate) struct BundleSet {
    /// In chain order; a chain locale without catalog files has no entry.
    pub(crate) locales: Vec<LocaleBundles>,
    /// Set when `chain[0]` is a pseudo-locale: UI results are bracketed.
    pub(crate) pseudo: Option<PseudoMode>,
}

impl fmt::Debug for Bundles {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let locales: Vec<String> = self.0.locales.iter().map(|l| l.lang.to_string()).collect();
        f.debug_struct("Bundles")
            .field("locales", &locales)
            .field("pseudo", &self.0.pseudo)
            .finish()
    }
}

/// Builds the runtime for `neg` from `cat`. When `PseudoMode::for_tag(&neg.chain[0])` is set,
/// that locale's bundles are built from the `en` resources through `transform_resource`.
pub fn build_runtime(
    cat: &Catalogs,
    neg: &Negotiated,
    fmt: &FormatInputs,
    generation: u64,
) -> Runtime {
    let pseudo = neg.chain.first().and_then(PseudoMode::for_tag);
    let mut locales = Vec::with_capacity(neg.chain.len());
    for (i, tag) in neg.chain.iter().enumerate() {
        let resources = match pseudo {
            Some(mode) if i == 0 => pseudo_resources(cat, mode),
            _ => {
                let tag = tag.to_string();
                cat.files_of(&tag)
                    .map(|f| (Arc::clone(&f.resource), f.layer))
                    .collect()
            }
        };
        if resources.is_empty() {
            continue;
        }
        locales.push(LocaleBundles {
            lang: tag.clone(),
            ui: bundle(tag, &resources, true),
            machine: bundle(tag, &resources, false),
        });
    }
    Runtime {
        generation,
        ui_chain: neg.chain.clone(),
        format: FormatCtx {
            format_locale: fmt.format_locale.clone(),
            time_zone: fmt.time_zone.clone(),
            generation,
        },
        diags: cat.diags().to_vec(),
        bundles: Bundles(Arc::new(BundleSet { locales, pseudo })),
    }
}

fn bundle(
    tag: &LanguageIdentifier,
    resources: &[(Arc<FluentResource>, Layer)],
    isolating: bool,
) -> Bundle {
    let mut b = Bundle::new_concurrent(vec![tag.clone()]);
    b.set_use_isolating(isolating);
    install_functions(&mut b);
    for (res, layer) in resources {
        match layer {
            // Within a layer the first definition of an ID wins (I18N-02 edge 12).
            Layer::Embedded => {
                let _ = b.add_resource(Arc::clone(res));
            }
            Layer::Overlay => b.add_resource_overriding(Arc::clone(res)),
        }
    }
    b
}

/// The `en` resources, transformed for a pseudo-locale (I18N-16).
fn pseudo_resources(cat: &Catalogs, mode: PseudoMode) -> Vec<(Arc<FluentResource>, Layer)> {
    cat.files_of("en")
        .map(|f| {
            let mut ast = match fluent_syntax::parser::parse(f.resource.source().to_string()) {
                Ok(ast) | Err((ast, _)) => ast,
            };
            transform_resource(&mut ast, mode);
            let text = fluent_syntax::serializer::serialize(&ast);
            let res = match FluentResource::try_new(text) {
                Ok(res) | Err((res, _)) => res,
            };
            (Arc::new(res), f.layer)
        })
        .collect()
}

/// The installed runtime, swapped atomically on a language switch.
pub struct L10n {
    current: ArcSwap<Runtime>,
    reserved: AtomicU64,
}

impl fmt::Debug for L10n {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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

    /// Renders `msg` for the UI sink (placeables isolated with U+2068/U+2069), falling back per
    /// message along the chain.
    pub fn t<A: IntoFluentArgs>(&self, msg: MsgRef<A>, args: A) -> String {
        self.text(msg.id, args.into_args(), Sink::Ui)
    }

    /// Renders attribute `attr` of `msg` for the UI sink; `None` when no language has it.
    pub fn t_attr<A: IntoFluentArgs>(&self, msg: MsgRef<A>, attr: &str, args: A) -> Option<String> {
        let rt = self.current.load();
        render(&rt, msg.id, Some(attr), args.into_args(), Sink::Ui).map(|r| r.text)
    }

    /// Renders `msg` for a machine sink (no isolation marks).
    pub fn t_machine<A: IntoFluentArgs>(&self, msg: MsgRef<A>, args: A) -> String {
        self.text(msg.id, args.into_args(), Sink::Machine)
    }

    /// Renders a dynamic ID (`hui-*`, `hv-cmd-*`) for the UI sink.
    pub fn t_dynamic(&self, id: &str, args: Vec<(&'static str, ArgValue)>) -> String {
        self.text(id, args, Sink::Ui)
    }

    /// Renders `id` from the `en` bundle only, without isolation marks (the English search
    /// layer).
    pub fn t_english(&self, id: &str) -> String {
        let rt = self.current.load();
        render_english(&rt, id).unwrap_or_else(|| render_missing(&rt, id, Sink::Machine))
    }

    fn text(&self, id: &str, args: Vec<(&'static str, ArgValue)>, sink: Sink) -> String {
        let rt = self.current.load();
        match render(&rt, id, None, args, sink) {
            Some(r) => r.text,
            None => render_missing(&rt, id, sink),
        }
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

    #[test]
    fn each_chain_locale_with_files_gets_both_bundles() {
        let cat = Catalogs::from_memory(&[
            ("en/hacivat-ui.ftl", "hv-a = A"),
            ("tr/hacivat-ui.ftl", "hv-a = A-tr"),
        ]);
        let neg = Negotiated {
            chain: vec![
                unic_langid::langid!("de"),
                unic_langid::langid!("tr"),
                unic_langid::langid!("en"),
            ],
            format_locale: "tr-TR".into(),
            diags: Vec::new(),
        };
        let r = build_runtime(&cat, &neg, &FormatInputs::new(&neg, "UTC"), 1);
        assert_eq!(
            format!("{:?}", r.bundles),
            "Bundles { locales: [\"tr\", \"en\"], pseudo: None }"
        );
        assert_eq!(r.ui_chain.len(), 3);
    }
}
