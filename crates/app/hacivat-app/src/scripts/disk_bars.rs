//! Changes on disk (HV-SCRIPT-08, R-M4b-16, Q17; NOTES surface 6): a clean tab reloads in
//! place (the editor's `replace_from_disk`, held during a composition); a dirty tab shows the
//! "Changed on disk" bar (Reload after a confirm, Keep mine); "Deleted on disk"; the "no longer
//! listed under this connection" notice; the "session is on another database" Run refusal.
//!
//! ## m4b-t01b-seams: nothing yet.
//!
//! ## m4b-t14-tabs
//! The notices under a script tab's toolbar (NOTES surfaces 5, 6, 10, 12) and the states of a
//! tab whose file is not shown (opening, not opened, the encoding question, refused). Every
//! notice sits in the console's focus home, whose primary is the code area (R-M4b-43): a
//! pressed button that removes its notice gives the focus to the code; the not-shown states
//! sit in a first-stop home of their own. Every button is a Tab stop (R-T33-1).
//! "Open under another connection…" is disabled, with the reason as its label's neighbour,
//! when no other connection shows the file (N2).

use hacivat_l10n::ArgValue;
use hacivat_ui::button::{Button, ButtonKind};
use hacivat_ui::focus_home::{HomeSlot, first_stop_home};
use hacivat_ui::notice::{InlineNotice, NoticeIcon};
use hacivat_ui_core::gpui_surface::{
    AnyElement, App, Entity, IntoElement, ParentElement as _, SharedString, Styled as _, Window,
    div, px,
};
use hacivat_ui_core::theme::Theme;

use crate::scripts::open;
use crate::scripts::save::{self, ConfirmSpec};
use crate::scripts::tabs::{
    self, Load, Notice, RunRefused, ScriptDoc, Unlisted, display_name, msg,
};

/// The notices of script tab `doc`, under its toolbar (`None`: none).
pub(crate) fn render(doc: &Entity<ScriptDoc>, cx: &mut App) -> Option<AnyElement> {
    let notices = doc.read(cx).notices.clone();
    let shown: Vec<AnyElement> = notices.iter().filter_map(|n| notice(doc, n, cx)).collect();
    if shown.is_empty() {
        return None;
    }
    let t = Theme::get(cx).tokens;
    Some(
        div()
            .flex()
            .flex_col()
            .gap(px(t.spacing.sm))
            .px(px(t.spacing.lg))
            .py(px(t.spacing.sm))
            .children(shown)
            .into_any_element(),
    )
}

/// The state of a tab whose text is not shown yet (or will not be), in its own home.
pub(crate) fn render_unloaded(
    doc: &Entity<ScriptDoc>,
    home: &HomeSlot,
    cx: &mut App,
) -> AnyElement {
    let load = doc.read(cx).load.clone();
    let file = tabs::file_label(doc.read(cx), cx);
    let conn = tabs::conn_label(doc.read(cx), cx);
    let theme = *Theme::get(cx);
    let t = theme.tokens;
    let tab = doc.read(cx).tab.0;
    let card = match load {
        Load::Ready => InlineNotice::new(NoticeIcon::Info),
        Load::Opening => InlineNotice::new(NoticeIcon::Info).line(msg(
            cx,
            "hv-scripts-opening",
            vec![("file", ArgValue::Text(file))],
        )),
        Load::Failed(reason) => InlineNotice::new(NoticeIcon::Info)
            .line(msg(
                cx,
                "hv-scripts-open-failed",
                vec![("file", ArgValue::Text(file))],
            ))
            .secondary(msg(
                cx,
                "hv-scripts-open-failed-body",
                vec![
                    ("reason", ArgValue::Text(reason.to_string())),
                    ("conn", ArgValue::Text(conn)),
                ],
            )),
        Load::NeedsEncoding { .. } => {
            let mut n = InlineNotice::new(NoticeIcon::Info)
                .line(msg(
                    cx,
                    "hv-scripts-enc-title",
                    vec![("file", ArgValue::Text(file))],
                ))
                .secondary(msg(cx, "hv-scripts-enc-body", Vec::new()));
            for (i, label) in ENCODINGS.iter().enumerate() {
                let weak = doc.downgrade();
                let label: &'static str = label;
                n = n.action(
                    Button::new(
                        ("script-enc", tab as usize * 8 + i),
                        msg(
                            cx,
                            "hv-scripts-enc-open-as",
                            vec![("encoding", ArgValue::Text(label.to_string()))],
                        ),
                    )
                    .on_click(move |_, _, cx| {
                        let _ = weak.update(cx, |d, cx| d.start_load(Some(label), cx));
                    }),
                );
            }
            n
        }
        Load::Refused(refusal) => {
            let database = doc
                .read(cx)
                .spec(cx)
                .and_then(|s| s.database)
                .map(|d| d.as_str().to_string())
                .unwrap_or_default();
            let text = open::refusal_text(&refusal, &file, &conn, &database, cx);
            let n = InlineNotice::new(NoticeIcon::Info).line(text);
            with_open_under(n, doc, cx)
        }
    };
    let body = div()
        .size_full()
        .bg(theme.tokens.colors.panel.to_hsla())
        .p(px(t.spacing.lg))
        .child(card);
    first_stop_home(home, body).into_any_element()
}

/// The encodings the question offers (NOTES surface 10; the Turkish ones first).
const ENCODINGS: [&str; 4] = ["windows-1254", "ISO-8859-9", "windows-1252", "UTF-16LE"];

fn notice(doc: &Entity<ScriptDoc>, n: &Notice, cx: &mut App) -> Option<AnyElement> {
    let file = tabs::file_label(doc.read(cx), cx);
    let conn = tabs::conn_label(doc.read(cx), cx);
    let tab = doc.read(cx).tab.0;
    let id = |name: &'static str| (name, tab as usize);
    let weak = doc.downgrade();
    let card = match n {
        Notice::Changed => {
            let (a, b) = (weak.clone(), weak.clone());
            InlineNotice::new(NoticeIcon::Info)
                .line(msg(
                    cx,
                    "hv-scripts-disk-changed",
                    vec![("file", ArgValue::Text(file))],
                ))
                .secondary(msg(cx, "hv-scripts-disk-changed-body", Vec::new()))
                .action(
                    Button::new(
                        id("script-reload"),
                        msg(cx, "hv-scripts-disk-reload", Vec::new()),
                    )
                    .on_click(move |_, window, cx| {
                        if let Some(doc) = a.upgrade() {
                            confirm_reload(&doc, window, cx);
                        }
                    }),
                )
                .action(
                    Button::new(
                        id("script-keep"),
                        msg(cx, "hv-scripts-disk-keep", Vec::new()),
                    )
                    .on_click(move |_, _, cx| {
                        let _ = b.update(cx, |d, cx| {
                            d.keep_mine = true;
                            d.hide(|n| matches!(n, Notice::Changed), cx);
                        });
                    }),
                )
        }
        Notice::Deleted => {
            let (a, b) = (weak.clone(), weak.clone());
            InlineNotice::new(NoticeIcon::Info)
                .line(msg(
                    cx,
                    "hv-scripts-disk-deleted",
                    vec![("file", ArgValue::Text(file))],
                ))
                .secondary(msg(cx, "hv-scripts-disk-deleted-body", Vec::new()))
                .action(
                    Button::new(
                        id("script-recreate"),
                        msg(cx, "hv-scripts-tab-save", Vec::new()),
                    )
                    .on_click(move |_, window, cx| {
                        if let Some(doc) = a.upgrade() {
                            save::save(&doc, window, cx, None);
                        }
                    }),
                )
                .action(
                    Button::new(
                        id("script-close"),
                        msg(cx, "hv-scripts-disk-close", Vec::new()),
                    )
                    .on_click(move |_, window, cx| {
                        if b.upgrade().is_some()
                            && let Some(ws) = window.root::<crate::shell::Workspace>().flatten()
                        {
                            crate::shell::Workspace::request_close_active_tab(&ws, window, cx);
                        }
                    }),
                )
        }
        // ## m4b-t14-tabs fix round 1 (I6, R-T14-4): Run and Save are off; Close, and
        // "Open under another connection…" where the file is listed.
        Notice::Unlisted(why) => {
            let body = match why {
                Unlisted::Moved => msg(cx, "hv-scripts-unlisted-gone", Vec::new()),
                Unlisted::Env { folder } => msg(
                    cx,
                    "hv-scripts-unlisted-env",
                    vec![("folder", ArgValue::Text(folder.clone()))],
                ),
            };
            let b = weak.clone();
            let n = InlineNotice::new(NoticeIcon::Info)
                .line(msg(
                    cx,
                    "hv-scripts-unlisted",
                    vec![
                        ("file", ArgValue::Text(file)),
                        ("conn", ArgValue::Text(conn)),
                    ],
                ))
                .secondary(body)
                .secondary(msg(cx, "hv-scripts-unlisted-off", Vec::new()))
                .action(
                    Button::new(
                        id("script-unlisted-close"),
                        msg(cx, "hv-scripts-disk-close", Vec::new()),
                    )
                    .on_click(move |_, window, cx| {
                        if b.upgrade().is_some()
                            && let Some(ws) = window.root::<crate::shell::Workspace>().flatten()
                        {
                            crate::shell::Workspace::request_close_active_tab(&ws, window, cx);
                        }
                    }),
                );
            with_open_under(n, doc, cx)
        }
        Notice::Moved { path } => {
            let a = weak.clone();
            InlineNotice::new(NoticeIcon::Info)
                .line(msg(
                    cx,
                    "hv-scripts-moved",
                    vec![("path", ArgValue::Text(path.clone()))],
                ))
                .secondary(msg(
                    cx,
                    "hv-scripts-moved-body",
                    vec![("conn", ArgValue::Text(conn))],
                ))
                .action(
                    Button::new(
                        id("script-moved-ok"),
                        msg(cx, "hv-scripts-recover-dismiss", Vec::new()),
                    )
                    .on_click(move |_, _, cx| {
                        let _ = a.update(cx, |d, cx| {
                            d.hide(|n| matches!(n, Notice::Moved { .. }), cx)
                        });
                    }),
                )
        }
        Notice::RunRefused(why) => {
            let (line, body) = match why {
                RunRefused::Unknown { bound } => (
                    msg(
                        cx,
                        "hv-scripts-run-refused-unknown",
                        vec![("bound", ArgValue::Text(display_name(bound)))],
                    ),
                    msg(cx, "hv-scripts-run-refused-unknown-body", Vec::new()),
                ),
                RunRefused::Mysql { bound, current } | RunRefused::Edited { bound, current } => {
                    let reason = if matches!(why, RunRefused::Mysql { .. }) {
                        "mysql"
                    } else {
                        "edited"
                    };
                    (
                        msg(
                            cx,
                            "hv-scripts-run-refused",
                            vec![
                                ("bound", ArgValue::Text(display_name(bound))),
                                ("current", ArgValue::Text(display_name(current))),
                            ],
                        ),
                        msg(
                            cx,
                            "hv-scripts-run-refused-body",
                            vec![
                                ("reason", ArgValue::Text(reason.to_string())),
                                ("conn", ArgValue::Text(conn.clone())),
                                ("bound", ArgValue::Text(display_name(bound))),
                                ("current", ArgValue::Text(display_name(current))),
                            ],
                        ),
                    )
                }
            };
            let mut n = InlineNotice::new(NoticeIcon::Info)
                .line(line)
                .secondary(body);
            if let RunRefused::Mysql { bound, .. } = why {
                let a = weak.clone();
                n = n.action(
                    Button::new(
                        id("script-retry"),
                        msg(
                            cx,
                            "hv-scripts-run-refused-retry",
                            vec![("bound", ArgValue::Text(display_name(bound)))],
                        ),
                    )
                    .on_click(move |_, window, cx| {
                        let editor = a
                            .update(cx, |d, cx| {
                                d.hide(|n| matches!(n, Notice::RunRefused(_)), cx);
                                d.editor().cloned()
                            })
                            .ok()
                            .flatten();
                        // ## m4b-t18-run-as-script fix round 1 (named exception; m3): a
                        // preview retries Run as script, never plain Run.
                        let console = a.upgrade().and_then(|d| d.read(cx).console());
                        if let Some(console) =
                            console.filter(|c| crate::scripts::run_as_script::offered(*c, cx))
                        {
                            crate::scripts::run_as_script::start(console, cx);
                        } else if let Some(editor) = editor {
                            editor.update(cx, |e, cx| {
                                e.run_command("editor.run", window, cx);
                            });
                        }
                    }),
                );
            }
            with_open_under(n, doc, cx)
        }
        Notice::SwitchFailed { database } => {
            let n = InlineNotice::new(NoticeIcon::Info)
                .line(msg(
                    cx,
                    "hv-scripts-switch-failed",
                    vec![("database", ArgValue::Text(display_name(database)))],
                ))
                .secondary(msg(cx, "hv-scripts-switch-failed-body", Vec::new()));
            with_open_under(n, doc, cx)
        }
        Notice::Removed => {
            let n = InlineNotice::new(NoticeIcon::Info)
                .line(msg(cx, "hv-scripts-unbound-title", Vec::new()))
                .secondary(msg(cx, "hv-scripts-unbound-body", Vec::new()));
            with_open_under(n, doc, cx)
        }
        // ## m4b-t17-secrets-ui (named lines): `secret` adds the R-T5-4 line.
        Notice::Recovered {
            at,
            unknown,
            secret,
        } => {
            let unix_ms = at
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
                .unwrap_or_default();
            let mut n = InlineNotice::new(NoticeIcon::Info)
                .line(msg(
                    cx,
                    "hv-scripts-recover-tab",
                    vec![(
                        "date",
                        ArgValue::When(hacivat_l10n::WhenValue::Instant { unix_ms }),
                    )],
                ))
                .secondary(msg(cx, "hv-scripts-recover-tab-body", Vec::new()));
            if *unknown {
                n = n.secondary(msg(cx, "hv-scripts-recover-unknown", Vec::new()));
            }
            // ## m4b-t17-secrets-ui (R-T5-4): the copy holds secret-like text (never which).
            if *secret {
                n = n.secondary(msg(cx, "hv-scripts-recover-secret", Vec::new()));
            }
            let (a, b) = (weak.clone(), weak.clone());
            n.action(
                Button::new(
                    id("script-rec-save"),
                    msg(cx, "hv-scripts-tab-save", Vec::new()),
                )
                .on_click(move |_, window, cx| {
                    if let Some(doc) = a.upgrade() {
                        save::save(&doc, window, cx, None);
                    }
                }),
            )
            .action(
                Button::new(
                    id("script-rec-discard"),
                    msg(cx, "hv-scripts-recover-discard", Vec::new()),
                )
                .on_click(move |_, window, cx| {
                    if let Some(doc) = b.upgrade() {
                        confirm_discard_recovered(&doc, window, cx);
                    }
                }),
            )
        }
        Notice::Markdown => {
            InlineNotice::new(NoticeIcon::Info).line(msg(cx, "hv-scripts-run-md", Vec::new()))
        }
        // The editor's own T2 banner says it (Task 11); the status bar has the segment.
        Notice::Preview => return None,
        // ## m4b-t14-tabs fix round 1 (I3, R-T14-3)
        Notice::OneSession => {
            InlineNotice::new(NoticeIcon::Info).line(msg(cx, "hv-scripts-one-session", Vec::new()))
        }
    };
    Some(card.into_any_element())
}

/// Adds "Open under another connection…", disabled with its reason when no other connection
/// shows the file (N2).
fn with_open_under(n: InlineNotice, doc: &Entity<ScriptDoc>, cx: &mut App) -> InlineNotice {
    let Some(spec) = doc.read(cx).spec(cx) else {
        return n;
    };
    let places = open::other_places(&spec, cx);
    let tab = doc.read(cx).tab.0;
    // ## m4b-t14-tabs fix round 1 (minors 4, 11): disabled with its reason (a sentence) when no
    // other connection shows the file, or when only the chooser (Task 15) could pick one.
    let reason = if places.is_empty() {
        Some("hv-scripts-open-under-none-reason")
    } else if !open::can_open_under(&places, cx) {
        Some("hv-scripts-open-under-chooser")
    } else {
        None
    };
    if let Some(reason) = reason {
        return n
            .action(
                Button::new(
                    ("script-open-under", tab as usize),
                    msg(cx, "hv-scripts-menu-open-under", Vec::new()),
                )
                .disabled(true),
            )
            .secondary(msg(cx, reason, Vec::new()));
    }
    n.action(
        Button::new(
            ("script-open-under", tab as usize),
            msg(cx, "hv-scripts-menu-open-under", Vec::new()),
        )
        .on_click(move |_, _, cx| open::open_under_another(&spec, cx)),
    )
}

/// "Reload" asks first: the edits are discarded (Danger), Cancel is the default.
fn confirm_reload(doc: &Entity<ScriptDoc>, window: &mut Window, cx: &mut App) {
    let file = tabs::file_label(doc.read(cx), cx);
    // ## m4b-t21-shots fix round 1 (m4): the tab's connection as the subtitle.
    let subtitle = Some(crate::scripts::unsaved_prompt::subtitle(doc.read(cx), cx));
    let target = doc.clone();
    save::confirm(
        window,
        cx,
        ConfirmSpec {
            subtitle,
            title: std::rc::Rc::new(move |cx: &App| {
                msg(
                    cx,
                    "hv-scripts-disk-reload-title",
                    vec![("file", ArgValue::Text(file.clone()))],
                )
            }),
            body: crate::l10n_global::text_fn_id("hv-scripts-disk-reload-body"),
            yes: crate::l10n_global::text_fn_id("hv-scripts-disk-reload-yes"),
            yes_kind: ButtonKind::Danger,
        },
        Box::new(move |yes, _, cx| {
            if yes {
                discard_and_reload(&target, cx);
            }
        }),
    );
}

/// The recovered tab's "Discard…": the same confirmation, then the copy goes and the tab goes
/// back to the file on disk.
fn confirm_discard_recovered(doc: &Entity<ScriptDoc>, window: &mut Window, cx: &mut App) {
    let file = tabs::file_label(doc.read(cx), cx);
    // ## m4b-t21-shots fix round 1 (m4): the tab's connection as the subtitle.
    let subtitle = Some(crate::scripts::unsaved_prompt::subtitle(doc.read(cx), cx));
    let target = doc.clone();
    save::confirm(
        window,
        cx,
        ConfirmSpec {
            subtitle,
            title: std::rc::Rc::new(move |cx: &App| {
                msg(
                    cx,
                    "hv-scripts-disk-reload-title",
                    vec![("file", ArgValue::Text(file.clone()))],
                )
            }),
            body: crate::l10n_global::text_fn_id("hv-scripts-disk-reload-body"),
            yes: crate::l10n_global::text_fn_id("hv-scripts-disk-reload-yes"),
            yes_kind: ButtonKind::Danger,
        },
        Box::new(move |yes, _, cx| {
            if yes {
                target.update(cx, |d, cx| d.discard_recovered(cx));
                discard_and_reload(&target, cx);
            }
        }),
    );
}

/// Replaces a dirty tab's text with the file on disk (the user confirmed).
fn discard_and_reload(doc: &Entity<ScriptDoc>, cx: &mut App) {
    let Some(editor) = doc.read(cx).editor().cloned() else {
        return;
    };
    let (Some(tab), Some(service)) = (doc.read(cx).spec(cx), open::service(cx)) else {
        return;
    };
    let env = doc.read(cx).env(cx);
    // ## m4b-t14-tabs fix round 1 (minor 1): in the file's own encoding, as `reload` reads.
    let meta = doc.read(cx).meta;
    let tier = doc.read(cx).tier;
    let label = (meta.encoding != hacivat_scripts::fileio::TextEncoding::UTF_8 && !meta.bom)
        .then_some(meta.encoding.name());
    let (lib, rel) = (tab.library, tab.rel.clone());
    let read = tabs::on_core(cx, async move {
        match label {
            Some(label) => service.read_with(lib, &rel, env, label).await,
            None => service.read(lib, &rel, env).await,
        }
    });
    let doc = doc.clone();
    cx.spawn(async move |cx| {
        let read = read.await;
        cx.update(|cx| match read {
            // A file that is now a T2 preview (or a preview tab) is never loaded into this
            // editor in place (minor 1); an open composition holds the replacement, so the
            // clean version would mark the user's composed text clean (minor 2): the bar
            // stays, and the user answers again.
            Some(Ok(decoded))
                if decoded.tier == hacivat_scripts::fileio::SizeTier::Preview
                    || tier == hacivat_scripts::fileio::SizeTier::Preview
                    || editor.read(cx).status().composing =>
            {
                doc.update(cx, |d, cx| d.show(Notice::Changed, cx));
            }
            Some(Ok(decoded)) => {
                editor.update(cx, |e, cx| e.set_text(&decoded.text, cx));
                let v = editor.read(cx).snapshot().version;
                doc.update(cx, |d, cx| {
                    d.meta = decoded.meta;
                    d.clean_at(v, Some(decoded.stamp), cx);
                });
            }
            _ => doc.update(cx, |d, cx| d.show(Notice::Changed, cx)),
        });
    })
    .detach();
}

/// The "Read-only preview" status segment's text, for a preview tab (carries-w1 T14/T18).
pub fn preview_segment(tab: hacivat_ui::panes::TabId, cx: &App) -> Option<SharedString> {
    let doc = tabs::doc_of_tab(tab, cx)?;
    (doc.read(cx).tier == hacivat_scripts::fileio::SizeTier::Preview)
        .then(|| msg(cx, "hv-scripts-preview-segment", Vec::new()))
}

// ## m4b-t21-shots
/// Shot fixtures (`tests/shots/m4b.rs`).
pub mod fixtures {
    use super::*;

    /// The question "Reload" asks on `doc`'s "Changed on disk" bar.
    pub fn confirm_reload(doc: &Entity<ScriptDoc>, window: &mut Window, cx: &mut App) {
        super::confirm_reload(doc, window, cx);
    }
}
