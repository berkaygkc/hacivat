//! The certificate review (NOTES M2a surface 5; M2b new surface 5, Task 19; B5, B6, Q26, Q34).
//!
//! **One 480 px layer `cert-dialog`, two variants.**
//! - *Untrusted* (`review.pinned` is `None`): a certificate no CA trusted by this Mac has
//!   signed. Who it was issued to and by, the valid-until date (the region format), the SHA-256
//!   fingerprint (two mono lines of 16 bytes, with Copy), the chain with each certificate's role,
//!   and the pin note. Buttons: Cancel (left), Import CA… and Pin this certificate (primary).
//! - *Pin changed* (`review.pinned` is `Some`; a hard stop): the Pinned and Now fingerprints, the
//!   new certificate's facts and chain inside the Now inset, the ask and the rule. Buttons:
//!   Replace pin (outline, far left) and Cancel (primary).
//!
//! **The safe answer is the default (Q34; PO question: Cancel until the PO says otherwise).**
//! Both variants open with Cancel focused. Enter acts on key down only, never held or with a
//! modifier, and its key up is suppressed (so the focused button's keyboard click never runs a
//! second time). In the untrusted variant Enter presses the focused button; in the pin-changed
//! variant it always cancels, whatever has the focus. Replace pin runs only on its own click
//! (the pointer, or Space on the focused button). Escape cancels both.
//!
//! **Dates (B5).** A pin never extends a certificate's dates, and a CA cannot either: when today
//! is outside the validity window, neither Pin, Replace pin nor Import CA is offered, and a line
//! says why.
//!
//! **Where the answer goes.**
//! - [`open`]: a pipeline question (`Question::Certificate`). Pin answers `Answer::Pin`; Import
//!   CA answers `Answer::ImportCa` with the path the platform file picker returned (never a
//!   typed one). A question never offers Replace pin: the pipeline never asks on a changed pin.
//! - [`review`]: the "Review certificate" fix of a failed connect. It builds the
//!   `ProfileChange` itself (Q26) — `AddPin`, `ReplacePin { old, new }` or `SetCaFile` — applies
//!   it through `ConnectionsModel::apply_changes` and connects again (a new gesture). Nothing
//!   else builds `ReplacePin`: every pin replacement is an explicit, traceable click.
//! - [`review_for_draft`]: the same review from the connection dialog's test strip; the change
//!   goes to the dialog's draft (a Test never writes the saved profile).
//!
//! Certificate names are server text: RFC 4514 strings whose controls and bidi marks arrive
//! escaped (Task 3). They are drawn verbatim in mono, never inside a translated sentence.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use futures::future::Either;
use hacivat_l10n::{NoArgs, Text, WhenValue, ids};
use hacivat_net::{CertPin, CertReview, ChainRole, fingerprint_lines, pin_endpoint};
use hacivat_proto::{EnvTag, ProfileId};
use hacivat_session::pipeline::{Answer, ConnectError, ProfileChange, Question};
use hacivat_session::profile::Profile;
use hacivat_ui::button::{Button, ButtonKind};
use hacivat_ui::capsule::env_swatch_on_raised;
use hacivat_ui::dialog::{DialogSize, dialog_frame};
use hacivat_ui::icon::{Icon, IconName, IconSize};
use hacivat_ui::popover::A11yNode;
use hacivat_ui::text::{Label, TextSource, paragraph, scroll_viewport};
use hacivat_ui::toast::{Toast, ToastKind, ToastQueue};
use hacivat_ui_core::gpui_surface::{
    AnyElement, AnyWindowHandle, App, AppContext as _, ClipboardItem, Context, Div, Entity,
    EntityId, FocusHandle, Focusable, Global, InteractiveElement, IntoElement, KeyDownEvent,
    MouseButton, ParentElement, PathPromptOptions, Render, Role, SharedString,
    StatefulInteractiveElement as _, Styled, WeakEntity, Window, div, px,
};
use hacivat_ui_core::theme::{Theme, ThemeMode, TypeStyle};
use tokio::sync::oneshot;

use crate::connections::dialog::DialogEnv;
use crate::connections::error_view::env_kind;
use crate::connections::model::ConnectionsModel;
use crate::l10n_global::tr;
use crate::shell::Workspace;
use crate::shell::layers::{DIALOG_CONTEXT, Layer};

/// The review's layer ID.
pub const CERT_LAYER_ID: &str = "cert-dialog";

/// The digest named next to a fingerprint (machine text, never translated).
pub(crate) const SHA256_LABEL: &str = "SHA-256";

/// The certificate facts' label column (the mock's `.certkv` 120 px).
const FACT_LABEL_PX: f32 = 120.0;
/// Fingerprints and names: Fragment Mono 11.5 on a 17 px line (the mock's `.fp`).
const FP_LINE_PX: f32 = 17.0;
/// A chain row's height (the mock's `.chain li`).
const CHAIN_ROW_PX: f32 = 22.0;
/// The chain's indent per level, and the tick that joins a row to the one above it.
const CHAIN_INDENT_PX: [f32; 3] = [0.0, 8.0, 24.0];
const TICK_W_PX: f32 = 9.0;
const TICK_H_PX: f32 = 11.0;

/// What the user chose in the review.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum Choice {
    Pin,
    /// The CA file the platform file picker returned.
    ImportCa(PathBuf),
    /// Replace the changed pin (only ever from the Replace pin button's own click).
    ReplacePin,
    Cancel,
}

type OnChoice = Box<dyn FnOnce(Choice, &mut Window, &mut App)>;

/// Whether today lies inside the reviewed certificate's validity window (B5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Validity {
    Valid,
    Expired,
    NotYetValid,
}

/// The validity of `review` at `now` (seconds since the Unix epoch).
pub fn validity_at(review: &CertReview, now: i64) -> Validity {
    if now < review.not_before_unix {
        Validity::NotYetValid
    } else if now > review.not_after_unix {
        Validity::Expired
    } else {
        Validity::Valid
    }
}

/// The wall clock in seconds since the Unix epoch (a certificate's dates are wall-clock dates).
fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// The change a choice makes for `review` (Q26): Pin adds the pin of the reviewed endpoint (only
/// where none was pinned), Replace pin replaces the pinned fingerprint by the presented one,
/// Import CA sets the CA file. Cancel changes nothing.
pub(crate) fn change_for(choice: &Choice, review: &CertReview) -> Option<ProfileChange> {
    let presented = || CertPin {
        endpoint: review.endpoint.clone(),
        sha256: review.sha256,
    };
    match choice {
        Choice::Pin if review.pinned.is_none() => Some(ProfileChange::AddPin(presented())),
        Choice::ReplacePin => review.pinned.map(|old| ProfileChange::ReplacePin {
            old,
            new: presented(),
        }),
        Choice::ImportCa(path) => Some(ProfileChange::SetCaFile(path.clone())),
        Choice::Pin | Choice::Cancel => None,
    }
}

/// Whether two pin endpoints name one endpoint, compared in `pin_endpoint`'s normal form (case,
/// a trailing dot, IPv6 brackets and canonical IP literals).
pub(crate) fn same_endpoint(a: &str, b: &str) -> bool {
    normal(a) == normal(b)
}

fn normal(endpoint: &str) -> String {
    match endpoint.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() => match port.parse::<u16>() {
            Ok(port) => pin_endpoint(host, port),
            Err(_) => endpoint.to_owned(),
        },
        _ => endpoint.to_owned(),
    }
}

/// Applies `change` to `profile` (the saved profile through `ConnectionsModel::apply_changes`,
/// or the connection dialog's draft after a Test):
/// - `AddPin` adds the pin only when its endpoint has none: replacing a pin is `ReplacePin`'s,
///   an explicit choice (Q26), so a stale or racing `AddPin` never overrides one;
/// - `ReplacePin { old, new }` replaces the endpoint's pin only while it still is `old` (the
///   fingerprint the user compared); otherwise it changes nothing;
/// - `SetCaFile` sets the CA file; `AllowPublicPlaintext` remembers the consent (B7).
///
/// Returns whether the profile changed.
pub(crate) fn apply_change(profile: &mut Profile, change: &ProfileChange) -> bool {
    let tls = &mut profile.tls;
    match change {
        ProfileChange::AddPin(pin) => {
            if tls
                .pins
                .iter()
                .any(|p| same_endpoint(&p.endpoint, &pin.endpoint))
            {
                return false;
            }
            tls.pins.push(pin.clone());
            true
        }
        ProfileChange::ReplacePin { old, new } => {
            let at = tls
                .pins
                .iter()
                .position(|p| same_endpoint(&p.endpoint, &new.endpoint) && p.sha256 == *old);
            let Some(at) = at else {
                return false;
            };
            tls.pins[at] = new.clone();
            // One pin per endpoint: any other pin of it would shadow or contradict the new one.
            let mut seen = false;
            tls.pins.retain(|p| {
                if !same_endpoint(&p.endpoint, &new.endpoint) {
                    return true;
                }
                let keep = !seen && p == new;
                seen |= keep;
                keep
            });
            true
        }
        ProfileChange::SetCaFile(path) => {
            let changed = tls.ca_file.as_deref() != Some(path.as_path());
            tls.ca_file = Some(path.clone());
            changed
        }
        ProfileChange::AllowPublicPlaintext => {
            !std::mem::replace(&mut tls.allow_public_plaintext, true)
        }
        // ## m2b-t21-prompts: `apply_changes` saves the saved profile's SSH retention through
        // `ConnectionsModel::save` (it moves the stored secrets); this applies it to an open
        // dialog's draft, so the dialog's Save never writes the old retention back.
        ProfileChange::SshRetention(retention) => {
            !std::mem::replace(&mut profile.ssh_retention, *retention).eq(retention)
        }
    }
}

/// The host and port of a pin endpoint as the review's sentences name them (`db:5432`,
/// `[::1]:5432`: an IPv6 host keeps its brackets).
fn host_and_port(endpoint: &str) -> (String, String) {
    match endpoint.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() => (host.to_string(), port.to_string()),
        _ => (endpoint.to_string(), String::new()),
    }
}

/// The most telling part of an RFC 4514 name for a chain row: its common name's value, still
/// escaped as received; the whole name when it has none.
pub(crate) fn display_name(name: &str) -> &str {
    let mut start = 0;
    let mut escaped = false;
    let mut boundaries = Vec::new();
    for (i, c) in name.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' => escaped = true,
            ',' | '+' => {
                boundaries.push((start, i));
                start = i + 1;
            }
            _ => {}
        }
    }
    boundaries.push((start, name.len()));
    boundaries
        .into_iter()
        .map(|(a, b)| &name[a..b])
        .find_map(|atv| {
            let (key, value) = atv.split_once('=')?;
            key.eq_ignore_ascii_case("CN").then_some(value)
        })
        .filter(|cn| !cn.is_empty())
        .unwrap_or(name)
}

// ## the dialog

/// The review (NOTES surface 5 and new surface 5).
pub struct CertDialog {
    name: String,
    env: Option<EnvTag>,
    review: CertReview,
    validity: Validity,
    /// Replace pin is offered (a review from an error, never a pipeline question).
    replace_offered: bool,
    on_choice: Option<OnChoice>,
    root: FocusHandle,
    copy_now: FocusHandle,
    copy_old: FocusHandle,
    cancel: FocusHandle,
    import: FocusHandle,
    pin: FocusHandle,
    replace: FocusHandle,
}

impl Focusable for CertDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.root.clone()
    }
}

/// The review open in the app, for tests and the shots harness.
struct OpenReview(WeakEntity<CertDialog>);

impl Global for OpenReview {}

/// The review open now, if any.
pub fn open_view(cx: &App) -> Option<Entity<CertDialog>> {
    cx.try_global::<OpenReview>().and_then(|o| o.0.upgrade())
}

impl CertDialog {
    fn new(
        name: String,
        env: Option<EnvTag>,
        review: CertReview,
        replace_offered: bool,
        on_choice: OnChoice,
        cx: &mut Context<Self>,
    ) -> Self {
        let stop = |cx: &mut Context<Self>| cx.focus_handle().tab_stop(true);
        CertDialog {
            name,
            env,
            validity: validity_at(&review, now_unix()),
            review,
            replace_offered,
            on_choice: Some(on_choice),
            root: cx.focus_handle(),
            copy_now: stop(cx),
            copy_old: stop(cx),
            cancel: stop(cx),
            import: stop(cx),
            pin: stop(cx),
            replace: stop(cx),
        }
    }

    // ---- reading (tests) --------------------------------------------------------------------

    /// Whether this is the pin-changed hard stop.
    pub fn is_pin_changed(&self) -> bool {
        self.review.pinned.is_some()
    }

    /// The certificate's validity today.
    pub fn validity(&self) -> Validity {
        self.validity
    }

    /// Whether trust can be widened at all: today is inside the validity window (B5).
    fn trust_offered(&self) -> bool {
        self.validity == Validity::Valid
    }

    /// The buttons drawn, by ID (`cert-copy-old`, `cert-copy`, `cert-cancel`, `cert-import`,
    /// `cert-pin`, `cert-replace-pin`), in Tab order.
    pub fn buttons(&self) -> Vec<&'static str> {
        self.order().into_iter().map(|(id, _)| id).collect()
    }

    /// The primary button's ID: Pin this certificate when it is offered, else Cancel.
    pub fn primary_button(&self) -> &'static str {
        if !self.is_pin_changed() && self.trust_offered() {
            "cert-pin"
        } else {
            "cert-cancel"
        }
    }

    /// The ID of the focused button, if one is.
    pub fn focused_button(&self, window: &Window) -> Option<&'static str> {
        self.order()
            .into_iter()
            .find(|(_, h)| h.is_focused(window))
            .map(|(id, _)| id)
    }

    /// The Tab order: the Copy buttons, then the footer from left to right.
    fn order(&self) -> Vec<(&'static str, FocusHandle)> {
        let trust = self.trust_offered();
        let mut order = Vec::new();
        if self.is_pin_changed() {
            order.push(("cert-copy-old", self.copy_old.clone()));
            order.push(("cert-copy", self.copy_now.clone()));
            if trust && self.replace_offered {
                order.push(("cert-replace-pin", self.replace.clone()));
            }
            order.push(("cert-cancel", self.cancel.clone()));
        } else {
            order.push(("cert-copy", self.copy_now.clone()));
            order.push(("cert-cancel", self.cancel.clone()));
            if trust {
                order.push(("cert-import", self.import.clone()));
                order.push(("cert-pin", self.pin.clone()));
            }
        }
        order
    }

    // ---- choosing ---------------------------------------------------------------------------

    /// Closes the review, then hands `choice` to its owner (once).
    fn choose(&mut self, choice: Choice, window: &mut Window, cx: &mut Context<Self>) {
        let Some(on_choice) = self.on_choice.take() else {
            return;
        };
        let view = cx.entity_id();
        window.defer(cx, move |window, cx| {
            close_view(view, window, cx);
            on_choice(choice, window, cx);
        });
    }

    fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.choose(Choice::Cancel, window, cx);
    }

    fn pin(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_pin_changed() || !self.trust_offered() {
            return;
        }
        self.choose(Choice::Pin, window, cx);
    }

    /// Replace pin: from its own click only (never Enter, Q34).
    fn replace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_pin_changed() || !self.replace_offered || !self.trust_offered() {
            return;
        }
        self.choose(Choice::ReplacePin, window, cx);
    }

    /// Import CA…: the platform file picker; the path it returns answers (a cancelled picker
    /// leaves the review open).
    fn import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_pin_changed() || !self.trust_offered() {
            return;
        }
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(path) = paths.into_iter().next().filter(|p| p.is_absolute()) else {
                return;
            };
            let _ = this.update_in(cx, |d, window, cx| {
                d.choose(Choice::ImportCa(path), window, cx);
            });
        })
        .detach();
    }

    /// Copies exactly the fingerprint: 32 colon-separated uppercase hex bytes.
    fn copy(sha256: &[u8; 32], cx: &mut App) {
        cx.write_to_clipboard(ClipboardItem::new_string(
            fingerprint_lines(sha256).join(":"),
        ));
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let stroke = &event.keystroke;
        let m = &stroke.modifiers;
        let plain = !(m.control || m.platform || m.alt || m.shift || m.function);
        match stroke.key.as_str() {
            "escape" => self.cancel(window, cx),
            "enter" if !plain || event.is_held => {}
            // A hard stop: Enter cancels, whatever has the focus (Q34).
            "enter" if self.is_pin_changed() => self.cancel(window, cx),
            "enter" => match self.focused_button(window) {
                Some("cert-copy") => Self::copy(&self.review.sha256, cx),
                Some("cert-import") => self.import(window, cx),
                Some("cert-pin") => self.pin(window, cx),
                _ => self.cancel(window, cx),
            },
            "tab" if !(m.control || m.platform || m.alt) => {
                let order = self.order();
                let n = order.len();
                let at = order.iter().position(|(_, h)| h.is_focused(window));
                let next = match (at, m.shift) {
                    (Some(i), false) => (i + 1) % n,
                    (Some(i), true) => (i + n - 1) % n,
                    (None, false) => 0,
                    (None, true) => n - 1,
                };
                window.focus(&order[next].1, cx);
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    // ---- drawing ----------------------------------------------------------------------------

    fn header(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let t = theme.tokens;
        let c = &t.colors;
        let (host, port) = host_and_port(&self.review.endpoint);
        let title = if self.is_pin_changed() {
            tr(
                cx,
                ids::hv::CONN_TLS_PIN_CHANGED_TITLE,
                ids::hv::HvConnTlsPinChangedTitle {
                    host: Text(host),
                    port: Text(port),
                },
            )
        } else {
            tr(cx, ids::hv::CONN_TLS_TITLE, NoArgs)
        };
        div()
            .flex()
            .flex_col()
            .flex_none()
            .gap(px(t.spacing.s_2xs))
            .child(
                paragraph(
                    title,
                    TextSource::Fluent,
                    t.type_ramp.body_strong,
                    c.text.to_hsla(),
                )
                .id("cert-title"),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(t.spacing.sm))
                    .min_w_0()
                    .children(self.env.map(|e| env_swatch_on_raised(env_kind(e), theme)))
                    .child(
                        Label::new(
                            self.name.clone(),
                            TextSource::Data,
                            t.type_ramp.mono_data,
                            c.text_2.to_hsla(),
                        )
                        .truncate(true)
                        .id("cert-connection"),
                    ),
            )
    }

    /// A sentence of the body: 12.5 or 13 px `text-2` on the raised card (Raised Ink Rule).
    fn sentence(
        text: SharedString,
        style: TypeStyle,
        theme: &Theme,
        id: &'static str,
    ) -> AnyElement {
        paragraph(
            text,
            TextSource::Fluent,
            style,
            theme.tokens.colors.text_2.to_hsla(),
        )
        .id(id)
        .into_any_element()
    }

    /// Issued to, Issued by and Valid until (the mock's `.certkv`).
    fn facts(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let t = theme.tokens;
        let c = &t.colors;
        let mono = TypeStyle {
            line_height: FP_LINE_PX,
            ..t.type_ramp.mono_data
        };
        let row = |label: SharedString, value: AnyElement| {
            div()
                .flex()
                .flex_row()
                .items_start()
                .gap(px(t.spacing.lg))
                // A long locale's label wraps inside its column; it never clips.
                .child(div().flex_none().w(px(FACT_LABEL_PX)).child(paragraph(
                    label,
                    TextSource::Fluent,
                    t.type_ramp.body_sm,
                    c.text_2.to_hsla(),
                )))
                .child(div().flex_1().min_w_0().child(value))
        };
        let name = |text: &str, id: &'static str| {
            paragraph(
                SharedString::from(text.to_string()),
                TextSource::Data,
                mono,
                c.text.to_hsla(),
            )
            .id(id)
            .into_any_element()
        };
        let until = tr(
            cx,
            ids::hv::CONN_TLS_VALID_DATE,
            ids::hv::HvConnTlsValidDate {
                when: WhenValue::Instant {
                    unix_ms: self.review.not_after_unix.saturating_mul(1000),
                },
            },
        );
        div()
            .flex()
            .flex_col()
            .gap(px(5.0))
            .child(row(
                tr(cx, ids::hv::CONN_TLS_SUBJECT, NoArgs),
                name(&self.review.subject, "cert-subject"),
            ))
            .child(row(
                tr(cx, ids::hv::CONN_TLS_ISSUER, NoArgs),
                name(&self.review.issuer, "cert-issuer"),
            ))
            .child(row(
                tr(cx, ids::hv::CONN_TLS_VALID, NoArgs),
                Label::new(
                    until,
                    TextSource::Fluent,
                    t.type_ramp.body_sm,
                    c.text.to_hsla(),
                )
                .id("cert-valid")
                .into_any_element(),
            ))
    }

    /// The chain: one row per certificate, leaf first, with its role (the mock's `.chain`).
    fn chain(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let t = theme.tokens;
        let c = &t.colors;
        let last = self.review.chain.len().saturating_sub(1);
        let mut rows = div().flex().flex_col();
        for (i, entry) in self.review.chain.iter().enumerate() {
            let role = match entry.role {
                ChainRole::Leaf => ids::hv::CONN_TLS_LEAF,
                ChainRole::Intermediate => ids::hv::CONN_TLS_INTER,
                ChainRole::Root => ids::hv::CONN_TLS_ROOT,
            };
            let level = i.min(last).min(CHAIN_INDENT_PX.len() - 1);
            let mark = if i == 0 {
                Icon::new(IconName::FileKey)
                    .size(IconSize::Chrome)
                    .color(c.text_2.to_hsla())
                    .into_any_element()
            } else {
                div()
                    .flex_none()
                    .w(px(TICK_W_PX))
                    .h(px(TICK_H_PX))
                    .mt(px(-8.0))
                    .border_l_1()
                    .border_b_1()
                    .border_color(c.seam_strong.to_hsla())
                    .into_any_element()
            };
            rows = rows.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(7.0))
                    .min_h(px(CHAIN_ROW_PX))
                    .pl(px(CHAIN_INDENT_PX[level]))
                    .child(mark)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(t.spacing.xs))
                            .child(div().min_w_0().child(paragraph(
                                SharedString::from(display_name(&entry.subject).to_string()),
                                TextSource::Data,
                                TypeStyle {
                                    line_height: FP_LINE_PX,
                                    ..t.type_ramp.mono_data
                                },
                                c.text.to_hsla(),
                            )))
                            .child(div().flex_none().child(Label::new(
                                SharedString::from(ROLE_SEPARATOR),
                                TextSource::Data,
                                t.type_ramp.body_sm,
                                c.text_2.to_hsla(),
                            )))
                            .child(div().flex_none().child(Label::new(
                                tr(cx, role, NoArgs),
                                TextSource::Fluent,
                                t.type_ramp.body_sm,
                                c.text_2.to_hsla(),
                            ))),
                    ),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap(px(t.spacing.s_2xs))
            .child(Label::new(
                tr(cx, ids::hv::CONN_TLS_CHAIN, NoArgs),
                TextSource::Fluent,
                t.type_ramp.body_sm,
                c.text_2.to_hsla(),
            ))
            .child(rows)
    }

    /// A fingerprint inset (the NOTES key-comparison primitive): the label, the digest's name
    /// (pin-changed only), Copy, then the two lines of 16 bytes; `extra` closes the inset.
    #[allow(clippy::too_many_arguments, reason = "one inset, drawn twice")]
    fn inset(
        &self,
        label: SharedString,
        sha256: [u8; 32],
        with_alg: bool,
        copy_id: &'static str,
        focus: FocusHandle,
        line_ids: [&'static str; 2],
        extra: Option<AnyElement>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = theme.tokens;
        let c = &t.colors;
        let well = match theme.mode {
            ThemeMode::Dark => c.panel,
            ThemeMode::Light => c.ground,
        };
        let mono = TypeStyle {
            line_height: FP_LINE_PX,
            ..t.type_ramp.mono_data
        };
        let lines = fingerprint_lines(&sha256);
        let copy = Button::new(copy_id, tr(cx, ids::hv::CONN_TLS_COPY_FP, NoArgs))
            .kind(ButtonKind::Icon)
            .icon(IconName::Copy)
            .focus_handle(focus)
            .on_click(cx.listener(move |_, _, _, cx| Self::copy(&sha256, cx)));
        // Raised Ink (Q34): the inset's labels are text-2; the values are text.
        let head = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(t.spacing.sm))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(t.spacing.sm))
                    .child(Label::new(
                        label,
                        TextSource::Fluent,
                        t.type_ramp.body_sm,
                        c.text_2.to_hsla(),
                    ))
                    .children(with_alg.then(|| {
                        Label::new(
                            SharedString::from(SHA256_LABEL),
                            TextSource::Data,
                            t.type_ramp.mono_data,
                            c.text.to_hsla(),
                        )
                    })),
            )
            .child(div().flex_none().child(copy));
        let mut inset = div()
            .flex()
            .flex_col()
            .gap(px(t.spacing.xs))
            .py(px(t.spacing.md))
            .px(px(t.spacing.lg))
            .border_1()
            .border_color(c.seam_strong.to_hsla())
            .rounded(px(t.radii.sm))
            .bg(well.to_hsla())
            .child(head);
        let mut value = div().flex().flex_col();
        for (line, id) in lines.into_iter().zip(line_ids) {
            value = value.child(
                Label::new(
                    SharedString::from(line),
                    TextSource::Data,
                    mono,
                    c.text.to_hsla(),
                )
                .id(id),
            );
        }
        inset = inset.child(value);
        if let Some(extra) = extra {
            inset = inset.child(extra);
        }
        inset
    }

    /// Why nothing is offered for a certificate outside its dates (B5).
    fn dates_note(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let state = match self.validity {
            Validity::Valid => return None,
            Validity::Expired => ids::hv::HvConnTlsDatesNoteState::Expired,
            Validity::NotYetValid => ids::hv::HvConnTlsDatesNoteState::Early,
        };
        let t = theme.tokens;
        let c = &t.colors;
        Some(
            div()
                .flex()
                .flex_row()
                .items_start()
                .gap(px(t.spacing.sm))
                .child(
                    div().pt(px(1.0)).flex_none().child(
                        Icon::new(IconName::TriangleAlert)
                            .size(IconSize::Chrome)
                            .color(c.text.to_hsla()),
                    ),
                )
                .child(
                    div().flex_1().min_w_0().child(
                        paragraph(
                            tr(
                                cx,
                                ids::hv::CONN_TLS_DATES_NOTE,
                                ids::hv::HvConnTlsDatesNote { state },
                            ),
                            TextSource::Fluent,
                            t.type_ramp.body_sm,
                            c.text.to_hsla(),
                        )
                        .id("cert-dates-note"),
                    ),
                )
                .into_any_element(),
        )
    }

    fn untrusted_body(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let t = theme.tokens;
        let (host, port) = host_and_port(&self.review.endpoint);
        let body = tr(
            cx,
            ids::hv::CONN_TLS_BODY,
            ids::hv::HvConnTlsBody {
                host: Text(host.clone()),
                port: Text(port.clone()),
            },
        );
        let note = tr(
            cx,
            ids::hv::CONN_TLS_PIN_NOTE,
            ids::hv::HvConnTlsPinNote {
                host: Text(host),
                port: Text(port),
            },
        );
        let inset = self.inset(
            tr(cx, ids::hv::CONN_TLS_FP, NoArgs),
            self.review.sha256,
            false,
            "cert-copy",
            self.copy_now.clone(),
            ["cert-fingerprint-line-1", "cert-fingerprint-line-2"],
            None,
            theme,
            cx,
        );
        div()
            .flex()
            .flex_col()
            .gap(px(t.spacing.lg))
            .child(Self::sentence(body, t.type_ramp.body, theme, "cert-body"))
            .child(self.facts(theme, cx))
            .child(inset)
            .child(self.chain(theme, cx))
            .children(self.dates_note(theme, cx))
            .children(
                self.trust_offered()
                    .then(|| Self::sentence(note, t.type_ramp.body_sm, theme, "cert-pin-note")),
            )
    }

    fn pin_changed_body(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let t = theme.tokens;
        let (host, port) = host_and_port(&self.review.endpoint);
        let body = tr(cx, ids::hv::CONN_TLS_PIN_CHANGED_BODY, NoArgs);
        let ask = tr(cx, ids::hv::CONN_TLS_PIN_CHANGED_ASK, NoArgs);
        let note = tr(
            cx,
            ids::hv::CONN_TLS_PIN_CHANGED_NOTE,
            ids::hv::HvConnTlsPinChangedNote {
                host: Text(host),
                port: Text(port),
            },
        );
        let old = self.review.pinned.unwrap_or_default();
        let pinned = self.inset(
            tr(cx, ids::hv::CONN_TLS_PIN_CHANGED_OLD, NoArgs),
            old,
            true,
            "cert-copy-old",
            self.copy_old.clone(),
            ["cert-fingerprint-old-line-1", "cert-fingerprint-old-line-2"],
            None,
            theme,
            cx,
        );
        let facts = div()
            .flex()
            .flex_col()
            .gap(px(t.spacing.xs))
            .mt(px(t.spacing.md))
            .child(self.facts(theme, cx))
            .child(self.chain(theme, cx))
            .into_any_element();
        let now = self.inset(
            tr(cx, ids::hv::CONN_TLS_PIN_CHANGED_NEW, NoArgs),
            self.review.sha256,
            true,
            "cert-copy",
            self.copy_now.clone(),
            ["cert-fingerprint-line-1", "cert-fingerprint-line-2"],
            Some(facts),
            theme,
            cx,
        );
        div()
            .flex()
            .flex_col()
            .gap(px(t.spacing.lg))
            .child(Self::sentence(body, t.type_ramp.body, theme, "cert-body"))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(t.spacing.md))
                    .child(pinned)
                    .child(now),
            )
            .children(self.dates_note(theme, cx))
            // Both speak of replacing the pin: only where Replace pin is offered (fix round 1,
            // M3).
            .children((self.trust_offered() && self.replace_offered).then(|| {
                div()
                    .flex()
                    .flex_col()
                    .gap(px(t.spacing.lg))
                    .child(Self::sentence(ask, t.type_ramp.body_sm, theme, "cert-ask"))
                    .child(Self::sentence(
                        note,
                        t.type_ramp.body_sm,
                        theme,
                        "cert-pin-note",
                    ))
            }))
    }

    fn footer(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let ids_now: Vec<&'static str> = self.buttons();
        let offered = |id: &str| ids_now.contains(&id);
        let primary = self.primary_button();
        let kind = |id: &str, otherwise: ButtonKind| {
            if id == primary {
                ButtonKind::Primary
            } else {
                otherwise
            }
        };
        let mut cancel = Button::new("cert-cancel", tr(cx, ids::hv::CONN_DLG_CANCEL, NoArgs))
            .kind(kind("cert-cancel", ButtonKind::Ghost))
            .focus_handle(self.cancel.clone())
            .on_raised(true)
            .on_click(cx.listener(|this, _, window, cx| this.cancel(window, cx)));
        if self.is_pin_changed()
            && let Some(hint) = escape_label(cx)
        {
            cancel = cancel.key_hint(hint);
        }
        // The buttons left to right; the first sits on the left.
        let mut buttons: Vec<AnyElement> = Vec::new();
        if self.is_pin_changed() {
            if offered("cert-replace-pin") {
                // An outline button, far from the default (NOTES ruling 4): it widens trust.
                buttons.push(
                    Button::new(
                        "cert-replace-pin",
                        tr(cx, ids::hv::CONN_TLS_PIN_CHANGED_REPLACE, NoArgs),
                    )
                    .kind(ButtonKind::Default)
                    .focus_handle(self.replace.clone())
                    .on_raised(true)
                    .on_click(cx.listener(|this, _, window, cx| this.replace(window, cx)))
                    .into_any_element(),
                );
            }
            buttons.push(cancel.into_any_element());
        } else {
            buttons.push(cancel.into_any_element());
            if offered("cert-import") {
                buttons.push(
                    Button::new("cert-import", tr(cx, ids::hv::CONN_TLS_IMPORT, NoArgs))
                        .kind(ButtonKind::Default)
                        .icon(IconName::FileKey)
                        .focus_handle(self.import.clone())
                        .on_raised(true)
                        .on_click(cx.listener(|this, _, window, cx| this.import(window, cx)))
                        .into_any_element(),
                );
            }
            if offered("cert-pin") {
                buttons.push(
                    Button::new("cert-pin", tr(cx, ids::hv::CONN_TLS_PIN_BTN, NoArgs))
                        .kind(kind("cert-pin", ButtonKind::Default))
                        .focus_handle(self.pin.clone())
                        .on_raised(true)
                        .on_click(cx.listener(|this, _, window, cx| this.pin(window, cx)))
                        .into_any_element(),
                );
            }
        }
        footer_row(buttons, theme)
    }
}

/// A prompt's footer: `buttons` left to right, the first on the left and the rest on the
/// right. When they do not fit one line (a long locale), they wrap one by one, right-aligned,
/// so no button runs past the card.
pub(crate) fn footer_row(buttons: Vec<AnyElement>, theme: &Theme) -> Div {
    let t = theme.tokens;
    let mut row = div()
        .flex()
        .flex_row()
        .flex_wrap()
        .flex_none()
        .items_center()
        .justify_end()
        .gap(px(t.spacing.md))
        .w_full();
    for (i, button) in buttons.into_iter().enumerate() {
        let cell = div().flex_none().child(button);
        row = row.child(if i == 0 { cell.mr_auto() } else { cell });
    }
    row
}

/// The separator between a chain row's name and its role (a symbol, not text).
const ROLE_SEPARATOR: &str = "·";

/// The Escape key as the active keymap writes it (the hard stop's Cancel hint).
fn escape_label(cx: &App) -> Option<SharedString> {
    let active = cx.try_global::<crate::shell::ActiveKeymap>()?;
    let chord = hacivat_ui_core::keymap::parse_chord("escape").ok()?;
    Some(hacivat_ui_core::keymap::display_chord(&chord, active.platform).into())
}

impl Render for CertDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        let header = self.header(&theme, cx);
        let body = if self.is_pin_changed() {
            self.pin_changed_body(&theme, cx)
        } else {
            self.untrusted_body(&theme, cx)
        };
        let footer = self.footer(&theme, cx);
        let title = if self.is_pin_changed() {
            let (host, port) = host_and_port(&self.review.endpoint);
            tr(
                cx,
                ids::hv::CONN_TLS_PIN_CHANGED_TITLE,
                ids::hv::HvConnTlsPinChangedTitle {
                    host: Text(host),
                    port: Text(port),
                },
            )
        } else {
            tr(cx, ids::hv::CONN_TLS_TITLE, NoArgs)
        };
        let card = dialog_frame(&theme, DialogSize::Wide)
            .id(CERT_LAYER_ID)
            .debug_selector(|| CERT_LAYER_ID.to_string())
            .track_focus(&self.root)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.on_key(event, window, cx);
            }))
            // Enter acts on key down only; the focused button's key-up click is suppressed.
            .capture_key_up(|event, window, _| {
                let stroke = &event.keystroke;
                if stroke.key == "enter" && !stroke.modifiers.modified() {
                    window.prevent_default();
                }
            })
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(header)
            // The body scrolls inside the window's height (the 960 × 600 minimum included);
            // the header and the buttons stay put.
            .child(
                div()
                    .id("cert-dialog-body")
                    .flex()
                    .flex_col()
                    .flex_shrink(1.0)
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(scroll_viewport())
                    .child(body),
            )
            .child(footer);
        // A hard stop is an alert dialog (NOTES surface 5).
        let role = if self.is_pin_changed() {
            Role::AlertDialog
        } else {
            Role::Dialog
        };
        A11yNode::new("cert-dialog-node", role, title, true, card)
    }
}

// ## opening

/// Removes the layer showing `view`, wherever it is in the stack.
pub(crate) fn close_view(view: EntityId, window: &mut Window, cx: &mut App) {
    if let Some(ws) = window.root::<Workspace>().flatten() {
        ws.update(cx, |ws, cx| ws.remove_layer(view, window, cx));
    }
}

/// Forwards the answer of the prompt `view` to `reply`, the pipeline's channel. When the asker
/// goes away first (the connect or test was cancelled), the orphaned prompt closes; a prompt
/// closed without an answer drops `reply`, which the port reads as `Cancelled`.
pub(crate) fn forward_answer(
    mut reply: oneshot::Sender<Answer>,
    answer: oneshot::Receiver<Answer>,
    view: EntityId,
    window: AnyWindowHandle,
    cx: &mut App,
) {
    cx.spawn(async move |cx| {
        let answered = {
            let closed = reply.closed();
            futures::pin_mut!(closed);
            match futures::future::select(closed, answer).await {
                Either::Left(_) => None,
                Either::Right((answer, _)) => Some(answer),
            }
        };
        match answered {
            Some(Ok(answer)) => {
                let _ = reply.send(answer);
            }
            Some(Err(_)) => {}
            None => {
                cx.update(|cx| {
                    let _ = window.update(cx, |_, window, cx| close_view(view, window, cx));
                });
            }
        }
    })
    .detach();
}

/// Pushes the review as a modal layer over `window`'s workspace and focuses Cancel. `None`
/// without a workspace.
fn push(
    name: String,
    env: Option<EnvTag>,
    review: CertReview,
    replace_offered: bool,
    on_choice: OnChoice,
    window: &mut Window,
    cx: &mut App,
) -> Option<Entity<CertDialog>> {
    let ws = window.root::<Workspace>().flatten()?;
    let view = cx.new(|cx| CertDialog::new(name, env, review, replace_offered, on_choice, cx));
    let cancel = view.read(cx).cancel.clone();
    ws.update(cx, |ws, cx| {
        ws.push_layer(
            Layer {
                id: CERT_LAYER_ID,
                context: DIALOG_CONTEXT,
                view: view.clone().into(),
                restore_focus: None,
                modal: true,
            },
            window,
            cx,
        );
    });
    // The safe answer has the focus (Q34; PO question, default Cancel).
    window.focus(&cancel, cx);
    cx.set_global(OpenReview(view.downgrade()));
    Some(view)
}

/// The environment of the saved profile `id`, if any.
fn environment_of(id: ProfileId, cx: &App) -> Option<EnvTag> {
    cx.try_global::<ConnectionsModel>()
        .and_then(|m| m.get(id))
        .and_then(|p| p.environment)
}

/// Asks `q` (`Question::Certificate`); the answer goes to `reply`. Pin answers `Pin`, Import
/// CA the picked path; Cancel, Escape, Enter on Cancel or a closed layer answer `Cancelled`.
/// Replace pin is never offered here (the pipeline never asks on a changed pin, Q26).
pub(crate) fn open(q: Question, reply: oneshot::Sender<Answer>, window: &mut Window, cx: &mut App) {
    let Question::Certificate {
        profile,
        name,
        review,
    } = q
    else {
        let _ = reply.send(Answer::Cancelled);
        return;
    };
    let env = environment_of(profile, cx);
    let (tx, rx) = oneshot::channel::<Answer>();
    let on_choice: OnChoice = Box::new(move |choice, _, _| {
        let answer = match choice {
            Choice::Pin => Answer::Pin,
            Choice::ImportCa(path) => Answer::ImportCa(path),
            Choice::ReplacePin | Choice::Cancel => Answer::Cancelled,
        };
        let _ = tx.send(answer);
    });
    let Some(view) = push(name, env, review, false, on_choice, window, cx) else {
        // Dropping `reply` answers `Cancelled`.
        return;
    };
    forward_answer(reply, rx, view.entity_id(), window.window_handle(), cx);
}

/// The "Review certificate" fix of `profile`'s failed connect `err`: the review of
/// `err.certificate`, whose choice builds the `ProfileChange` here (Q26) — "Replace pin" →
/// `ReplacePin { old: review.pinned, new: { review.endpoint, review.sha256 } }`, "Pin" →
/// `AddPin`, Import CA → `SetCaFile` — applies it through `apply_changes` and connects again.
pub(crate) fn review(
    err: Arc<ConnectError>,
    profile: ProfileId,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(review) = err.certificate.clone() else {
        return;
    };
    let name = cx
        .try_global::<ConnectionsModel>()
        .and_then(|m| m.get(profile))
        .map_or_else(|| err.args.name.clone(), |p| p.name.clone());
    let env = environment_of(profile, cx);
    let reviewed = review.clone();
    let on_choice: OnChoice = Box::new(move |choice, _, cx| {
        if let Some(change) = change_for(&choice, &reviewed) {
            apply_and_connect(profile, change, &reviewed.endpoint, cx);
        }
    });
    push(name, env, review, true, on_choice, window, cx);
}

/// The same review from the connection dialog's test strip: `on_change` receives the change a
/// choice makes (the dialog applies it to its draft and tests again).
pub(crate) fn review_for_draft(
    review: CertReview,
    name: String,
    env: Option<EnvTag>,
    on_change: impl FnOnce(ProfileChange, &mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let reviewed = review.clone();
    let on_choice: OnChoice = Box::new(move |choice, window, cx| {
        if let Some(change) = change_for(&choice, &reviewed) {
            on_change(change, window, cx);
        }
    });
    push(name, env, review, true, on_choice, window, cx);
}

/// The "Import CA" fix of `profile`'s failed connect: the platform file picker; the picked
/// file becomes the profile's CA file and the profile connects again.
pub(crate) fn import_ca(profile: ProfileId, cx: &mut App) {
    pick_ca(cx, move |path, cx| {
        apply_and_connect(profile, ProfileChange::SetCaFile(path), "", cx);
    });
}

/// Opens the platform file picker for a CA file; `then` gets the picked (absolute) path.
pub(crate) fn pick_ca(cx: &mut App, then: impl FnOnce(PathBuf, &mut App) + 'static) {
    let picked = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: false,
        multiple: false,
        prompt: None,
    });
    cx.spawn(async move |cx| {
        let Ok(Ok(Some(paths))) = picked.await else {
            return;
        };
        if let Some(path) = paths.into_iter().next().filter(|p| p.is_absolute()) {
            cx.update(|cx| then(path, cx));
        }
    })
    .detach();
}

/// Whether `profile` holds what `change` asks for (the pin, the CA file, the consent).
fn holds(profile: &Profile, change: &ProfileChange) -> bool {
    let tls = &profile.tls;
    match change {
        ProfileChange::AddPin(pin) | ProfileChange::ReplacePin { new: pin, .. } => {
            tls.pins.contains(pin)
        }
        ProfileChange::SetCaFile(path) => tls.ca_file.as_deref() == Some(path.as_path()),
        ProfileChange::AllowPublicPlaintext => tls.allow_public_plaintext,
        // ## m2b-t21-prompts
        ProfileChange::SshRetention(retention) => profile.ssh_retention == *retention,
    }
}

/// Applies `change` to the saved profile `id` and connects it again (a new gesture). A CA file
/// the user picked joins the approved roots first (T16 ruling A); a replaced pin is confirmed
/// with a toast naming the endpoint's host. A change the saved profile did not take (a stale
/// `ReplacePin`, a read-only store, a profile deleted meanwhile) is neither confirmed nor
/// followed by a connect (fix round 1, M1).
fn apply_and_connect(id: ProfileId, change: ProfileChange, endpoint: &str, cx: &mut App) {
    if let ProfileChange::SetCaFile(path) = &change
        && let Some(env) = cx.try_global::<DialogEnv>()
    {
        env.approved_roots.add(path.clone());
    }
    let replaced = matches!(change, ProfileChange::ReplacePin { .. });
    // The model's list changes at once; the write follows on the profile writer.
    let saved = ConnectionsModel::apply_changes(cx, id, vec![change.clone()]);
    cx.spawn(async move |_| {
        if let Err(e) = saved.await {
            tracing::debug!(error = ?e, "connections: the reviewed change was not saved");
        }
    })
    .detach();
    let taken = cx
        .try_global::<ConnectionsModel>()
        .and_then(|m| m.get(id))
        .is_some_and(|p| holds(&p, &change));
    if !taken {
        tracing::debug!("connections: the reviewed change no longer applies; not connecting");
        return;
    }
    if replaced {
        let host = host_and_port(endpoint).0;
        let text = std::rc::Rc::new(move |cx: &App| {
            tr(
                cx,
                ids::hv::CONN_TLS_PIN_CHANGED_DONE,
                ids::hv::HvConnTlsPinChangedDone {
                    host: Text(host.clone()),
                },
            )
        });
        ToastQueue::push(cx, Toast::new(ToastKind::Info, text));
    }
    ConnectionsModel::connect(cx, id, None);
}

// ## shots fixtures (R28, P19)

#[cfg(feature = "shots")]
mod shots {
    use hacivat_net::{CertReview, ChainEntry, ChainRole};
    use hacivat_ui_core::gpui_surface::{App, Window};

    use super::push;

    /// The mock's review: `db-qa.acme.io:5432`, signed by Acme's own CA.
    pub(crate) fn sample(pinned: Option<[u8; 32]>) -> CertReview {
        CertReview {
            endpoint: "db-qa.acme.io:5432".to_string(),
            subject: "CN=db-qa.acme.io".to_string(),
            issuer: "CN=Acme Internal CA G2,O=Acme".to_string(),
            not_before_unix: 1_735_689_600,
            // 2027-03-14, noon UTC (the same date in every time zone the shots run in).
            not_after_unix: 1_805_025_600,
            sha256: [
                0x3F, 0x9A, 0xC2, 0x7E, 0x15, 0xB0, 0x4D, 0x88, 0xE1, 0x2C, 0x6F, 0x93, 0x0A, 0xD7,
                0x5B, 0x31, 0xC4, 0x68, 0x9E, 0xF2, 0x07, 0xBA, 0x43, 0x1D, 0x8C, 0xE5, 0x72, 0x39,
                0xA6, 0x0F, 0xD4, 0x71,
            ],
            chain: vec![
                ChainEntry {
                    subject: "CN=db-qa.acme.io".to_string(),
                    role: ChainRole::Leaf,
                },
                ChainEntry {
                    subject: "CN=Acme Internal CA G2,O=Acme".to_string(),
                    role: ChainRole::Intermediate,
                },
                ChainEntry {
                    subject: "CN=Acme Root CA 2021,O=Acme".to_string(),
                    role: ChainRole::Root,
                },
            ],
            pinned,
        }
    }

    /// The untrusted-certificate review (NOTES M2a surface 5).
    pub fn untrusted_fixture(window: &mut Window, cx: &mut App) {
        push(
            "reports-qa".to_string(),
            Some(hacivat_proto::EnvTag::Test),
            sample(None),
            false,
            Box::new(|_, _, _| {}),
            window,
            cx,
        );
    }

    /// The pin-changed hard stop (NOTES M2b surface 5).
    pub fn pin_changed_fixture(window: &mut Window, cx: &mut App) {
        push(
            "reports-test".to_string(),
            Some(hacivat_proto::EnvTag::Test),
            sample(Some([0x5C; 32])),
            true,
            Box::new(|_, _, _| {}),
            window,
            cx,
        );
    }
}

#[cfg(feature = "shots")]
pub use shots::{pin_changed_fixture, untrusted_fixture};

#[cfg(feature = "shots")]
pub(crate) use shots::sample as sample_review;

#[cfg(test)]
mod tests {
    use super::*;
    use hacivat_net::ChainEntry;

    fn review(pinned: Option<[u8; 32]>) -> CertReview {
        CertReview {
            endpoint: "db:5432".into(),
            subject: "CN=db".into(),
            issuer: "CN=ca".into(),
            not_before_unix: 100,
            not_after_unix: 200,
            sha256: [2; 32],
            chain: vec![ChainEntry {
                subject: "CN=db".into(),
                role: ChainRole::Leaf,
            }],
            pinned,
        }
    }

    #[test]
    fn validity_follows_the_window() {
        let r = review(None);
        assert_eq!(validity_at(&r, 99), Validity::NotYetValid);
        assert_eq!(validity_at(&r, 100), Validity::Valid);
        assert_eq!(validity_at(&r, 200), Validity::Valid);
        assert_eq!(validity_at(&r, 201), Validity::Expired);
    }

    #[test]
    fn only_replace_pin_builds_replace_pin() {
        let pinned = review(Some([1; 32]));
        let new = CertPin {
            endpoint: "db:5432".into(),
            sha256: [2; 32],
        };
        assert_eq!(
            change_for(&Choice::ReplacePin, &pinned),
            Some(ProfileChange::ReplacePin { old: [1; 32], new })
        );
        // Pin on a changed pin, Cancel, and Replace pin without a pin build nothing.
        assert_eq!(change_for(&Choice::Pin, &pinned), None);
        assert_eq!(change_for(&Choice::Cancel, &pinned), None);
        assert_eq!(change_for(&Choice::ReplacePin, &review(None)), None);
        assert!(matches!(
            change_for(&Choice::Pin, &review(None)),
            Some(ProfileChange::AddPin(p)) if p.sha256 == [2; 32] && p.endpoint == "db:5432"
        ));
    }

    #[test]
    fn add_pin_never_replaces_and_replace_pin_needs_the_old_one() {
        let mut p = Profile::default();
        let pin = |sha: u8| CertPin {
            endpoint: "db:5432".into(),
            sha256: [sha; 32],
        };
        assert!(apply_change(&mut p, &ProfileChange::AddPin(pin(1))));
        // A second AddPin for the endpoint (any spelling) changes nothing.
        let other = CertPin {
            endpoint: "DB.:5432".into(),
            sha256: [9; 32],
        };
        assert!(!apply_change(&mut p, &ProfileChange::AddPin(other)));
        assert_eq!(p.tls.pins, [pin(1)]);
        // ReplacePin against a stale fingerprint changes nothing.
        let stale = ProfileChange::ReplacePin {
            old: [7; 32],
            new: pin(2),
        };
        assert!(!apply_change(&mut p, &stale));
        assert_eq!(p.tls.pins, [pin(1)]);
        let replace = ProfileChange::ReplacePin {
            old: [1; 32],
            new: pin(2),
        };
        assert!(apply_change(&mut p, &replace));
        assert_eq!(p.tls.pins, [pin(2)]);
        // Another endpoint's pin is left alone.
        let elsewhere = CertPin {
            endpoint: "db:5433".into(),
            sha256: [3; 32],
        };
        assert!(apply_change(
            &mut p,
            &ProfileChange::AddPin(elsewhere.clone())
        ));
        assert_eq!(p.tls.pins, [pin(2), elsewhere]);
    }

    #[test]
    fn chain_rows_show_the_common_name_as_received() {
        assert_eq!(
            display_name("CN=Acme Root CA 2021,O=Acme"),
            "Acme Root CA 2021"
        );
        assert_eq!(display_name("O=Acme,CN=db\\,qa"), "db\\,qa");
        assert_eq!(display_name("O=Acme+CN=x"), "x");
        assert_eq!(display_name("O=Acme"), "O=Acme");
        assert_eq!(display_name("CN=a\\E2\\80\\AEb"), "a\\E2\\80\\AEb");
    }

    #[test]
    fn host_and_port_keep_ipv6_brackets() {
        assert_eq!(
            host_and_port("[::1]:5432"),
            ("[::1]".to_string(), "5432".to_string())
        );
        assert_eq!(
            host_and_port("db:5432"),
            ("db".to_string(), "5432".to_string())
        );
    }
}
