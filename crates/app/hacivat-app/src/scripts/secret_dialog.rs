//! The save-time secret warning (HV-SCRIPT-10, R-M4b-17; NOTES surface 8): one dialog listing
//! the lines and kinds (never the values), Save anyway / **Cancel** (the default and Escape).
//!
//! ## m4b-t01b-seams: nothing yet (Task 17).
//!
//! ## m4b-t17-secrets-ui
//! - **Where it asks.** Every save of a library script goes through `save::save`, which asks
//!   here first ([`confirm`]): ⌘S, Save all, the unsaved-changes prompt's Save (close, quit and
//!   restart), the recovery notice's Save and "Deleted on disk"'s Save. Consoles never ask
//!   (they stay in Hacivat's own folder, Q7). M4b has no "Save as… into a library" and no
//!   "Move console to library…"; a later one calls [`confirm`] the same way.
//! - **The dialog** (480 alertdialog): "3 lines in create_reader.sql look like secrets", the
//!   tab's environment · connection · database under it, "Hacivat found text that looks like a
//!   password or a token. The values aren't shown here.", then a field well listing **Line n ·
//!   kind** (one row per line and kind, the first [`SHOWN_ROWS`], then "… and N more lines"),
//!   the git sentence when the library is in a repository (or its status is not known yet:
//!   fail toward the warning), the `hacivat:allow-secret` hint, and "Save anyway is remembered
//!   for this tab until you quit." Footer: **Save anyway** (Default) · **Cancel** (Primary,
//!   focused); Escape and the close button answer Cancel; Enter presses the focused button.
//!   When the dialog closes the focus goes back where it was (the code).
//! - **Unscanned** (a scan that could not run): "orders_report.sql couldn't be checked for
//!   secrets", no rows, no allow hint and no "remembered" line (it is never remembered).
//! - Nothing here holds or shows a value: the rows have only a line number and a kind.

use std::collections::BTreeSet;
use std::rc::Rc;

use hacivat_l10n::ArgValue;
use hacivat_scripts::secrets::SecretKind;
use hacivat_scripts::service::GitSummary;
use hacivat_ui::button::ButtonKind;
use hacivat_ui::dialog::{Dialog, DialogButton, DialogEvent, DialogSize, DialogSpec};
use hacivat_ui::icon::{Icon, IconName, IconSize};
use hacivat_ui::text::{Label, TextFn, TextSource, paragraph};
use hacivat_ui_core::gpui_surface::{
    App, AppContext as _, Context, Entity, Global, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Role, SharedString, Styled as _, WeakEntity, Window, div, px,
};
use hacivat_ui_core::theme::{Theme, ThemeMode};

use crate::scripts::secret_marks::{ALLOW_MARKER, Scan, kind_label};
use crate::scripts::tabs::{self, ScriptDoc, msg};

/// The rows the well lists before "… and N more lines".
pub const SHOWN_ROWS: usize = 8;

/// The answer to the warning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveDecision {
    /// "Save anyway".
    Save,
    /// Cancel, Escape, the close button, or no window to ask in: the tab stays unsaved.
    Cancel,
}

/// What an answer runs.
pub(crate) type Decided = Box<dyn FnOnce(SaveDecision, &mut Window, &mut App)>;

/// What a save of `scan`'s text needs: `Save` at once when nothing asks (no finding, or every
/// one remembered for this tab), else `None` (ask).
pub fn decide(scan: &Scan, doc: &ScriptDoc) -> Option<SaveDecision> {
    (!scan.asks(&doc.secret_ok)).then_some(SaveDecision::Save)
}

/// Asks about `scan`'s findings in `doc`'s window (the dialog above), then runs `decided`.
pub(crate) fn confirm(
    doc: &Entity<ScriptDoc>,
    scan: &Scan,
    window: &mut Window,
    cx: &mut App,
    decided: Decided,
) {
    let Some(ws) = window.root::<crate::shell::Workspace>().flatten() else {
        decided(SaveDecision::Cancel, window, cx);
        return;
    };
    let unscanned = scan.unscanned();
    let mut rows: Vec<(u32, SecretKind)> = scan.findings.iter().map(|f| (f.line, f.kind)).collect();
    rows.sort_by_key(|(line, kind)| (*line, *kind as u8));
    rows.dedup();
    let lines = rows.iter().map(|r| r.0).collect::<BTreeSet<_>>().len() as u64;
    let file = tabs::file_label(doc.read(cx), cx);
    let title: TextFn = if unscanned {
        Rc::new(move |cx: &App| {
            msg(
                cx,
                "hv-scripts-secret-unscanned-title",
                vec![("file", ArgValue::Text(file.clone()))],
            )
        })
    } else {
        Rc::new(move |cx: &App| {
            msg(
                cx,
                "hv-scripts-secret-title",
                vec![
                    ("n", ArgValue::Count(lines)),
                    ("file", ArgValue::Text(file.clone())),
                ],
            )
        })
    };
    let body = crate::l10n_global::text_fn_id(if unscanned {
        "hv-scripts-secret-unscanned-body"
    } else {
        "hv-scripts-secret-body"
    });
    let subtitle = crate::scripts::unsaved_prompt::subtitle(doc.read(cx), cx);
    let content = SecretList {
        rows: if unscanned { Vec::new() } else { rows },
        git: in_repository(doc.read(cx), cx),
        unscanned,
    };
    let button = |label: TextFn, kind: ButtonKind| DialogButton {
        label,
        kind,
        accesskey: None,
        english_accesskey: None,
    };
    let spec = DialogSpec {
        title,
        body,
        buttons: vec![
            button(
                crate::l10n_global::text_fn_id("hv-scripts-secret-save"),
                ButtonKind::Default,
            ),
            button(
                crate::l10n_global::text_fn_id("hv-tx-cancel"),
                ButtonKind::Primary,
            ),
        ],
        cancel: 1,
        primary: 1,
        size: DialogSize::Wide,
    };
    // The answer's subscription is made before the layer is pushed and holds the dialog until
    // an event is delivered (`open_dialog`).
    let dialog = ws.update(cx, |ws, cx| ws.open_dialog(spec, window, cx));
    let list = cx.new(|_| content);
    dialog.update(cx, |d, _| {
        d.set_role(Role::AlertDialog);
        d.set_subtitle(Rc::new(move |_: &App| SharedString::from(subtitle.clone())));
        d.set_content(list.into());
    });
    cx.set_global(OpenWarning(dialog.downgrade()));
    let decided = std::cell::RefCell::new(Some(decided));
    cx.subscribe(&dialog, move |_, event: &DialogEvent, cx| {
        let answer = match event {
            DialogEvent::Pressed(0) => SaveDecision::Save,
            _ => SaveDecision::Cancel,
        };
        if let Some(decided) = decided.borrow_mut().take() {
            crate::exec_ui::in_workspace_window(cx, move |_, window, cx| {
                decided(answer, window, cx)
            });
        }
    })
    .detach();
}

/// The warning last opened (a weak handle: gone once it closed).
struct OpenWarning(WeakEntity<Dialog>);

impl Global for OpenWarning {}

/// The open secret warning's dialog, while it is open (tests and shots read its focus).
pub fn open_warning(cx: &App) -> Option<Entity<Dialog>> {
    cx.try_global::<OpenWarning>()?.0.upgrade()
}

/// Whether `doc`'s library is in a git repository, or its status is not known yet (the
/// sentence errs toward the warning).
fn in_repository(doc: &ScriptDoc, cx: &App) -> bool {
    let (Some(tab), Some(service)) = (doc.spec(cx), crate::scripts::open::service(cx)) else {
        return true;
    };
    let snap = service.snapshot();
    let git = snap.view(doc.env(cx)).library(tab.library).map(|l| l.git());
    !matches!(git, Some(GitSummary::NotARepo))
}

/// The dialog's content: the well of lines and kinds, the git sentence and the hints.
struct SecretList {
    rows: Vec<(u32, SecretKind)>,
    git: bool,
    unscanned: bool,
}

impl Render for SecretList {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        // Inside a dialog the dim ink is text-2 (the Raised Ink Rule); the well's own fill.
        let (ink, dim) = (c.text.to_hsla(), c.text_2.to_hsla());
        let well_fill = match theme.mode {
            ThemeMode::Dark => c.panel,
            ThemeMode::Light => c.ground,
        };
        let mut col = div()
            .debug_selector(|| "scripts-secret-list".to_string())
            .flex()
            .flex_col()
            .gap(px(t.spacing.lg));
        if !self.rows.is_empty() {
            let mut well = div()
                .debug_selector(|| "scripts-secret-well".to_string())
                .flex()
                .flex_col()
                .gap(px(3.0))
                .py(px(6.0))
                .px(px(10.0))
                .border_1()
                .border_color(c.seam_strong.to_hsla())
                .rounded(px(t.radii.sm))
                .bg(well_fill.to_hsla());
            for (i, (line, kind)) in self.rows.iter().take(SHOWN_ROWS).enumerate() {
                let number = msg(
                    cx,
                    "hv-scripts-secret-line",
                    vec![("line", ArgValue::Count(u64::from(*line) + 1))],
                );
                well = well.child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .min_w_0()
                        .child(
                            Icon::new(IconName::KeyRound)
                                .size(IconSize::Chrome)
                                .color(dim),
                        )
                        .child(
                            div().flex_none().min_w(px(64.0)).child(
                                Label::new(number, TextSource::Fluent, t.type_ramp.mono_data, dim)
                                    .id(SharedString::from(format!("scripts-secret-line-{i}"))),
                            ),
                        )
                        .child(
                            div().flex_1().min_w_0().child(
                                paragraph(
                                    kind_label(*kind)(cx),
                                    TextSource::Fluent,
                                    t.type_ramp.body_sm,
                                    ink,
                                )
                                .id("scripts-secret-kind"),
                            ),
                        ),
                );
            }
            if self.rows.len() > SHOWN_ROWS {
                let more = u64::try_from(self.rows.len() - SHOWN_ROWS).unwrap_or(u64::MAX);
                well = well.child(
                    Label::new(
                        msg(
                            cx,
                            "hv-scripts-attach-more-lines",
                            vec![("n", ArgValue::Count(more))],
                        ),
                        TextSource::Fluent,
                        t.type_ramp.caption,
                        dim,
                    )
                    .id("scripts-secret-more"),
                );
            }
            col = col.child(well);
        }
        if self.git {
            col = col.child(
                paragraph(
                    msg(cx, "hv-scripts-secret-git", Vec::new()),
                    TextSource::Fluent,
                    t.type_ramp.body_strong,
                    ink,
                )
                .id("scripts-secret-git"),
            );
        }
        if !self.unscanned {
            let hint = |icon: IconName, text: SharedString, id: &'static str| {
                div()
                    .flex()
                    .flex_row()
                    .items_start()
                    .gap(px(6.0))
                    .child(
                        div()
                            .flex_none()
                            .pt(px(2.0))
                            .child(Icon::new(icon).size(IconSize::Chrome).color(dim)),
                    )
                    .child(div().flex_1().min_w_0().child(
                        paragraph(text, TextSource::Fluent, t.type_ramp.body_sm, dim).id(id),
                    ))
            };
            col = col
                .child(hint(
                    IconName::Info,
                    msg(
                        cx,
                        "hv-scripts-secret-allow",
                        vec![("marker", ArgValue::Text(ALLOW_MARKER.to_string()))],
                    ),
                    "scripts-secret-allow",
                ))
                .child(hint(
                    IconName::History,
                    msg(cx, "hv-scripts-secret-remember", Vec::new()),
                    "scripts-secret-remember",
                ));
        }
        col
    }
}
