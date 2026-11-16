//! Pasting connection text into the dialog through the input paste filter (R21, P16, Task 17;
//! NOTES surface 2).
//!
//! - **Where.** Every text field that is not masked installs a filter. The URL field's filter
//!   takes every paste; the other fields' filters take a paste only when
//!   `looks_like_connection` matches it, and leave any other text to the field. A masked field
//!   (the password, the key passphrase) never calls its filter: its paste is always literal.
//! - **How.** The filter runs while its field is being updated, so it never touches the field:
//!   it carries the text as a secret (it may hold a password) into a deferred step, which parses
//!   it and applies it with `apply_parse` (T12).
//! - **What the user sees.** The filled fields keep the `blue-tint` for 1.6 s, then fade over
//!   240 ms. An inline notice lists exactly what happened (the password moved out, parameters
//!   mapped, unknown ones kept) with Undo; a URL for another engine asks before switching a form
//!   that already has input; text that is no connection changes nothing and lists the formats.
//! - **Trust (M2a final review I5).** A profile created or filled from pasted text is
//!   `Origin::Imported { source: Url }` (unless it already has an untrusted or managed
//!   origin), so DRV-05 checks its options and its TLS files must lie under the approved roots
//!   (the home directory, or a file picked in the dialog). Text typed into the URL row is the
//!   user's own and keeps the origin.

use std::collections::BTreeSet;
use std::rc::Rc;
use std::time::{Duration, Instant};

use hacivat_driver::url::{
    FieldId, MappedParam, MappedTo, ParseOutcome, ParseResult, ParseWarning, looks_like_connection,
    parse,
};
use hacivat_driver::{EngineCatalog, SanitizeInput, ViolationReason, sanitize};
use hacivat_l10n::ids::hv::HvConnUrlLeftOutReason as LeftOut;
use hacivat_l10n::{Count, NoArgs, Text, ids};
use hacivat_proto::{DbmsKind, ImportSource, Origin};
use hacivat_secrets::Retention;
use hacivat_session::profile::{EngineChange, FoldedOption, FoldedTo, Profile, apply_parse};
use hacivat_ui::button::{Button, ButtonKind};
use hacivat_ui::icon::IconName;
use hacivat_ui::input::PasteFilter;
use hacivat_ui::notice::{InlineNotice, NoticeIcon};
use hacivat_ui_core::gpui_surface::{
    AnyElement, App, Context, IntoElement, SharedString, Task, WeakEntity, Window,
};
use hacivat_ui_core::theme::Theme;
use secrecy::{ExposeSecret, SecretString};

use super::ConnectionDialog;
use super::form::{Field, tls_label};
use crate::l10n_global::tr;

/// What the Display URL shows where a secret is (`hacivat_driver::url::render`).
const DISPLAY_MASK: &str = "****";

/// How long pasted fields keep the tint before it fades (NOTES cross-cutting #7).
pub(crate) const TINT_HOLD: Duration = Duration::from_millis(1600);
/// How long the tint takes to fade.
pub(crate) const TINT_FADE: Duration = Duration::from_millis(240);

/// The paste filter of a field (P16): `parse_all` (the URL row) takes every paste; any other
/// field takes only text that looks like a connection.
pub(crate) fn paste_filter(parse_all: bool, dialog: WeakEntity<ConnectionDialog>) -> PasteFilter {
    Rc::new(move |text: &str, window: &mut Window, cx: &mut App| {
        if !parse_all && !looks_like_connection(text) {
            return false;
        }
        // The text may carry a password: it travels as a secret, and the field (leased while
        // its filter runs) is left alone; the edit lands in a deferred step.
        let text = SecretString::from(text.to_string());
        let dialog = dialog.clone();
        window.defer(cx, move |window, cx| {
            let _ = dialog.update(cx, |d, cx| d.apply_text(text.expose_secret(), window, cx));
        });
        true
    })
}

/// The form before a paste, for Undo. Holds secrets, so `Debug` is not derived.
pub(crate) struct Snapshot {
    profile: Profile,
    password: Option<SecretString>,
    passphrase: Option<SecretString>,
    touched: BTreeSet<FieldId>,
    env_touched: bool,
    password_from_url: bool,
}

/// The inline notice under the URL row.
pub(crate) enum Notice {
    /// The form was filled from a URL.
    Filled {
        engine_set: Option<DbmsKind>,
        secret: bool,
        retention: Retention,
        mapped: Vec<MappedParam>,
        /// Extras with a typed setting, which went to it (I2).
        folded: Vec<FoldedOption>,
        extras: Vec<(String, String)>,
        // ## m2a-demo-extra
        /// Parameters DRV-05 refuses for this origin and engine: left out, never a row.
        left_out: Vec<(String, String, ViolationReason)>,
        /// The host used and how many the URL named.
        multihost: Option<(String, usize)>,
    },
    /// A URL for another engine, held until the user decides.
    Switch {
        url: DbmsKind,
        form: DbmsKind,
        outcome: Box<ParseOutcome>,
        /// The text was pasted (not typed into the URL row): it fills as an import (I5).
        pasted: bool,
    },
    /// Text that is not a connection: nothing changed.
    Unrecognised,
}

/// The paste tint on the filled fields.
pub(crate) struct Tint {
    pub(crate) fields: Vec<Field>,
    /// When the fade began (after [`TINT_HOLD`]).
    fade_from: Option<Instant>,
    _timers: [Task<()>; 2],
}

impl Tint {
    pub(crate) fn fading(&self) -> bool {
        self.fade_from.is_some()
    }

    /// How far the fade is, 0 (tinted) to 1 (back to the well).
    pub(crate) fn progress(&self, cx: &App) -> f32 {
        match self.fade_from {
            None => 0.0,
            Some(from) => {
                let gone = cx
                    .background_executor()
                    .now()
                    .saturating_duration_since(from);
                (gone.as_secs_f32() / TINT_FADE.as_secs_f32()).clamp(0.0, 1.0)
            }
        }
    }
}

impl ConnectionDialog {
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            profile: self.draft.profile.clone(),
            password: self.draft.password.clone(),
            passphrase: self.draft.tls_key_passphrase.clone(),
            touched: self.draft.touched.clone(),
            env_touched: self.env_touched,
            password_from_url: self.password_from_url,
        }
    }

    /// Applies pasted (or typed) connection text: fills the form, asks before switching the
    /// engine of a form with input, or says the text is not a connection.
    pub(crate) fn apply_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        match parse(text, Some(self.draft.profile.engine)) {
            ParseResult::Unrecognised(_) => {
                self.notice = Some(Notice::Unrecognised);
                cx.notify();
            }
            ParseResult::Recognised(outcome) => self.apply_outcome(outcome, true, window, cx),
        }
    }

    /// The URL row's own text, edited: the row shows `****` where a password is, so a secret
    /// whose value is that mask is the current secret, never a new one (it is dropped and the
    /// draft's password or passphrase stays).
    pub(crate) fn apply_row_text(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match parse(text, Some(self.draft.profile.engine)) {
            ParseResult::Unrecognised(_) => {
                self.notice = Some(Notice::Unrecognised);
                cx.notify();
            }
            ParseResult::Recognised(mut outcome) => {
                outcome
                    .secrets
                    .retain(|(_, s)| s.expose_secret() != DISPLAY_MASK);
                if outcome
                    .tls_key_passphrase
                    .as_ref()
                    .is_some_and(|s| s.expose_secret() == DISPLAY_MASK)
                {
                    outcome.tls_key_passphrase = None;
                }
                self.apply_outcome(outcome, false, window, cx);
            }
        }
    }

    /// Applies a recognised outcome: fills the form, or asks before switching the engine of a
    /// form with input. `pasted`: the text came from a paste, not the URL row's own text.
    fn apply_outcome(
        &mut self,
        outcome: ParseOutcome,
        pasted: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let form = self.draft.profile.engine;
        if outcome.kind != form && !self.draft.touched.is_empty() {
            // HV-DRV-04: never switch a form with input silently.
            self.notice = Some(Notice::Switch {
                url: outcome.kind,
                form,
                outcome: Box::new(outcome),
                pasted,
            });
        } else {
            self.fill(outcome, false, pasted, window, cx);
        }
        cx.notify();
    }

    /// Fills the form from `outcome` (`force`: the user accepted another engine; `pasted`:
    /// the text was pasted, so the profile becomes an import from a URL, I5).
    fn fill(
        &mut self,
        outcome: ParseOutcome,
        force: bool,
        pasted: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let before = self.snapshot();
        let mapped = outcome.mapped.clone();
        let extras = outcome.extra.clone();
        let multihost = outcome.warnings.iter().find_map(|w| match w {
            ParseWarning::MultipleHosts { used, dropped } => Some((used.clone(), dropped + 1)),
            _ => None,
        });
        let read_only_before = self.draft.profile.read_only;
        let report = apply_parse(&mut self.draft, outcome, force);
        // ## m2a-finalfix (I5): pasted text is an untrusted origin. A profile that already
        // has one (an import, a managed profile) keeps it.
        if pasted && self.draft.profile.origin == Origin::UserTyped {
            self.draft.profile.origin = Origin::Imported {
                source: ImportSource::Url,
            };
        }
        // Extras with a typed setting went to it (I2): listed as mapped, not as kept.
        let folded = report.folded.clone();
        // ## m2a-demo-extra: a parameter DRV-05 refuses for this origin and engine would only
        // block the save: it is left out, and the notice says so (never "kept").
        let left_out = refused_extras(&self.draft.profile);
        self.draft
            .profile
            .extra
            .retain(|(k, _)| !left_out.iter().any(|(r, _, _)| r == k));
        let extras: Vec<(String, String)> = extras
            .into_iter()
            .filter(|(k, v)| !folded.iter().any(|f| &f.key == k && &f.value == v))
            .filter(|(k, _)| !left_out.iter().any(|(r, _, _)| r == k))
            .collect();
        // SQLite `mode=ro` is the file option "Open the file read-only", not the Hacivat guard
        // (ruling A): the guard keeps its value, the option is set.
        if self.draft.profile.engine == hacivat_proto::DbmsKind::Sqlite
            && mapped
                .iter()
                .any(|m| matches!(m.to, MappedTo::ReadOnly(true)))
        {
            self.draft.profile.read_only = read_only_before;
            super::form::set_sqlite_mode_ro(&mut self.draft.profile, true);
        }
        let engine_set = match report.engine {
            Some(EngineChange::Switched { to, .. }) => Some(to),
            _ => None,
        };
        if report.password_moved {
            self.password_from_url = true;
        }
        if let Some(kind) = engine_set {
            let options = super::form::tls_options(kind);
            self.tls_mode.update(cx, |s, cx| s.set_options(options, cx));
        }
        self.undo = Some(before);
        self.issues.clear();
        self.write_fields(cx);
        self.rebuild_extra_rows(window, cx);
        self.start_tint(
            report.filled.iter().filter_map(|f| Field::of(*f)).collect(),
            cx,
        );
        self.notice = Some(Notice::Filled {
            engine_set,
            secret: report.password_moved,
            retention: self.draft.profile.retention,
            mapped,
            folded,
            extras,
            left_out,
            multihost,
        });
        self.after_edit(window, cx);
        // The URL row shows the result even when the paste went into it.
        self.write_url(cx);
    }

    /// "Switch to <engine> and fill": the held URL replaces what the form had.
    pub(crate) fn accept_switch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Notice::Switch {
            outcome, pasted, ..
        }) = self.notice.take()
        else {
            return;
        };
        let before = self.snapshot();
        self.draft.touched.clear();
        self.fill(*outcome, true, pasted, window, cx);
        // Undo goes back to the form as it was before the switch.
        self.undo = Some(before);
    }

    /// Undo of a paste: the form as it was before.
    pub(crate) fn undo_paste(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(s) = self.undo.take() else {
            return;
        };
        let engine_changed = s.profile.engine != self.draft.profile.engine;
        self.draft.profile = s.profile;
        self.draft.password = s.password;
        self.draft.tls_key_passphrase = s.passphrase;
        self.draft.touched = s.touched;
        self.env_touched = s.env_touched;
        self.password_from_url = s.password_from_url;
        if engine_changed {
            let options = super::form::tls_options(self.draft.profile.engine);
            self.tls_mode
                .update(cx, |sel, cx| sel.set_options(options, cx));
        }
        self.notice = None;
        self.tint = None;
        self.issues.clear();
        self.write_fields(cx);
        self.rebuild_extra_rows(window, cx);
        self.after_edit(window, cx);
        self.write_url(cx);
    }

    pub(crate) fn dismiss_notice(&mut self, cx: &mut Context<Self>) {
        self.notice = None;
        cx.notify();
    }

    /// Tints `fields` for [`TINT_HOLD`], then fades them over [`TINT_FADE`]. Both timers start
    /// now, so the tint is gone exactly 1.84 s after the paste.
    fn start_tint(&mut self, fields: Vec<Field>, cx: &mut Context<Self>) {
        if fields.is_empty() {
            self.tint = None;
            return;
        }
        let fade = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(TINT_HOLD).await;
            let _ = this.update(cx, |d, cx| {
                let now = cx.background_executor().now();
                if let Some(tint) = &mut d.tint {
                    tint.fade_from = Some(now);
                }
                cx.notify();
            });
        });
        let clear = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(TINT_HOLD + TINT_FADE).await;
            let _ = this.update(cx, |d, cx| {
                d.tint = None;
                cx.notify();
            });
        });
        self.tint = Some(Tint {
            fields,
            fade_from: None,
            _timers: [fade, clear],
        });
    }

    /// The notice element (NOTES surface 2), with its buttons.
    pub(crate) fn render_notice(
        &self,
        _theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let notice = self.notice.as_ref()?;
        let engine = |k: &DbmsKind| Text(k.display_name().to_string());
        let el = match notice {
            Notice::Filled {
                engine_set,
                secret,
                retention,
                mapped,
                folded,
                extras,
                left_out,
                multihost,
            } => {
                let mut n = InlineNotice::new(NoticeIcon::Done).line(tr(
                    cx,
                    ids::hv::CONN_URL_FILLED,
                    NoArgs,
                ));
                if let Some(kind) = engine_set {
                    n = n.secondary(tr(
                        cx,
                        ids::hv::CONN_URL_ENGINE_SET,
                        ids::hv::HvConnUrlEngineSet {
                            engine: engine(kind),
                        },
                    ));
                }
                if *secret {
                    n = n.secondary(secret_line(cx, *retention));
                }
                if let Some((host, count)) = multihost {
                    n = n.secondary(tr(
                        cx,
                        ids::hv::CONN_URL_MULTIHOST,
                        ids::hv::HvConnUrlMultihost {
                            count: Count(*count as u64),
                            host: Text(host.clone()),
                        },
                    ));
                }
                for m in mapped {
                    if let Some(setting) = mapped_setting(cx, m) {
                        n = n.param(
                            param_text(&m.key, &m.value),
                            tr(
                                cx,
                                ids::hv::CONN_URL_MAPPED,
                                ids::hv::HvConnUrlMapped {
                                    setting: Text(setting.to_string()),
                                },
                            ),
                        );
                    }
                }
                for f in folded {
                    n = n.param(
                        param_text(&f.key, &f.value),
                        tr(
                            cx,
                            ids::hv::CONN_URL_MAPPED,
                            ids::hv::HvConnUrlMapped {
                                setting: Text(folded_setting(cx, &f.to).to_string()),
                            },
                        ),
                    );
                }
                for (k, v) in extras {
                    n = n.param(param_text(k, v), tr(cx, ids::hv::CONN_URL_EXTRA, NoArgs));
                }
                // ## m2a-demo-extra
                let form_engine = self.draft.profile.engine;
                for (k, v, reason) in left_out {
                    let reason = match reason {
                        ViolationReason::UnknownOption => LeftOut::Unknown,
                        _ => LeftOut::Refused,
                    };
                    n = n.param(
                        param_text(k, v),
                        tr(
                            cx,
                            ids::hv::CONN_URL_LEFT_OUT,
                            ids::hv::HvConnUrlLeftOut {
                                reason,
                                engine: engine(&form_engine),
                            },
                        ),
                    );
                }
                n.trailing(
                    Button::new("notice-undo", tr(cx, ids::hv::CONN_URL_UNDO, NoArgs))
                        .kind(ButtonKind::Ghost)
                        .focus_handle(self.focus.notice_primary.clone())
                        .on_click(cx.listener(|this, _, window, cx| this.undo_paste(window, cx))),
                )
            }
            Notice::Switch { url, form, .. } => InlineNotice::new(NoticeIcon::Info)
                .line(tr(
                    cx,
                    ids::hv::CONN_URL_SWITCH,
                    ids::hv::HvConnUrlSwitch {
                        engine: engine(url),
                        current: engine(form),
                    },
                ))
                .secondary(tr(cx, ids::hv::CONN_URL_SWITCH_L2, NoArgs))
                .action(
                    Button::new(
                        "notice-switch",
                        tr(
                            cx,
                            ids::hv::CONN_URL_SWITCH_BTN,
                            ids::hv::HvConnUrlSwitchBtn {
                                engine: engine(url),
                            },
                        ),
                    )
                    .focus_handle(self.focus.notice_primary.clone())
                    .on_raised(true)
                    .on_click(cx.listener(|this, _, window, cx| this.accept_switch(window, cx))),
                )
                .action(
                    Button::new(
                        "notice-keep",
                        tr(
                            cx,
                            ids::hv::CONN_URL_KEEP,
                            ids::hv::HvConnUrlKeep {
                                current: engine(form),
                            },
                        ),
                    )
                    .kind(ButtonKind::Ghost)
                    .focus_handle(self.focus.notice_secondary.clone())
                    .on_click(cx.listener(|this, _, _, cx| this.dismiss_notice(cx))),
                ),
            Notice::Unrecognised => InlineNotice::new(NoticeIcon::Info)
                .line(tr(cx, ids::hv::CONN_URL_UNREC, NoArgs))
                .secondary(tr(cx, ids::hv::CONN_URL_FORMATS, NoArgs))
                .trailing(
                    Button::new("notice-dismiss", tr(cx, ids::hv::CONN_URL_DISMISS, NoArgs))
                        .kind(ButtonKind::Icon)
                        .icon(IconName::X)
                        .focus_handle(self.focus.notice_primary.clone())
                        .on_click(cx.listener(|this, _, _, cx| this.dismiss_notice(cx))),
                ),
        };
        Some(el.into_any_element())
    }
}

/// `key=value` as the URL had it (machine text, shown in mono). The value of a password-like
/// key is never shown, only the mask (M2a final-fix round 2, N1).
fn param_text(key: &str, value: &str) -> SharedString {
    let value = if hacivat_proto::is_secret_param(key) {
        DISPLAY_MASK
    } else {
        value
    };
    [key, value].join("=").into()
}

/// The password line: honest that the Keychain is used only on save, or how long it is kept.
fn secret_line(cx: &App, retention: Retention) -> SharedString {
    use ids::hv::HvConnUrlSecretOtherRet as Ret;
    let ret = match retention {
        Retention::Forever => return tr(cx, ids::hv::CONN_URL_SECRET, NoArgs),
        Retention::Never => Ret::Never,
        Retention::UntilRestart => Ret::UntilRestart,
        Retention::ForSession => Ret::ForSession,
    };
    tr(
        cx,
        ids::hv::CONN_URL_SECRET_OTHER,
        ids::hv::HvConnUrlSecretOther { ret },
    )
}

// ## m2a-finalfix
/// The form setting a folded extra became (I2).
fn folded_setting(cx: &App, to: &FoldedTo) -> SharedString {
    match to {
        FoldedTo::TlsMode(mode) => tr(
            cx,
            ids::hv::CONN_URL_SETTING_TLS,
            ids::hv::HvConnUrlSettingTls {
                mode: Text(tr(cx, tls_label(*mode), NoArgs).to_string()),
            },
        ),
        FoldedTo::CaFile(_) => tr(cx, ids::hv::CONN_TLS_CA, NoArgs),
        FoldedTo::ClientCert => tr(cx, ids::hv::CONN_DLG_CERT, NoArgs),
        FoldedTo::ClientKey => tr(cx, ids::hv::CONN_DLG_KEY, NoArgs),
        FoldedTo::ConnectTimeout(_) => tr(cx, ids::hv::CONN_ADV_TIMEOUT, NoArgs),
        FoldedTo::ApplicationName => tr(cx, ids::hv::CONN_ADV_APP, NoArgs),
    }
}

/// The form setting a mapped parameter became (`None` for `options`, which also stays an extra
/// and is listed there).
fn mapped_setting(cx: &App, m: &MappedParam) -> Option<SharedString> {
    Some(match &m.to {
        MappedTo::TlsMode(mode) => tr(
            cx,
            ids::hv::CONN_URL_SETTING_TLS,
            ids::hv::HvConnUrlSettingTls {
                mode: Text(tr(cx, tls_label(*mode), NoArgs).to_string()),
            },
        ),
        MappedTo::ApplicationName => tr(cx, ids::hv::CONN_ADV_APP, NoArgs),
        MappedTo::ConnectTimeout(_) => tr(cx, ids::hv::CONN_ADV_TIMEOUT, NoArgs),
        MappedTo::ReadOnly(_) => tr(cx, ids::hv::CONN_DLG_SQLITE_RO, NoArgs),
        MappedTo::PgOptions => return None,
    })
}

// ## m2a-demo-extra
/// The extras of `p` that DRV-05 refuses for its origin and engine (an unknown option from an
/// untrusted origin, an always-denied one, ...), each with the first reason. A file outside
/// the approved roots is not among them: the user can still pick that file in the dialog.
fn refused_extras(p: &Profile) -> Vec<(String, String, ViolationReason)> {
    let Err(violations) = sanitize(
        &SanitizeInput {
            kind: p.engine,
            origin: &p.origin,
            options: &p.extra,
            paths: &[],
            unsafe_toggle: false,
            approved_roots: &[],
        },
        EngineCatalog::embedded(),
    ) else {
        return Vec::new();
    };
    p.extra
        .iter()
        .filter_map(|(k, v)| {
            violations
                .iter()
                .find(|x| &x.key == k && x.reason != ViolationReason::PathOutsideRoots)
                .map(|x| (k.clone(), v.clone(), x.reason))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ## m2a-finalfix (round 2, N1)
    #[test]
    fn the_notice_never_shows_the_value_of_a_password_like_key() {
        for key in [
            "trustCertificateKeyStorePassword",
            "keyStorePassword",
            "password1",
            "ssl_passphrase",
            "db_password",
        ] {
            let shown = param_text(key, "hunter2");
            assert!(!shown.contains("hunter2"), "{key}");
            assert_eq!(shown.as_ref(), format!("{key}=****"));
        }
        assert_eq!(param_text("foo", "bar").as_ref(), "foo=bar");
    }
}
