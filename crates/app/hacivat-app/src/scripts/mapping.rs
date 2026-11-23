//! "Map to a script library…" (HV-SCRIPT-02, R-M4b-7, R-M4b-8, Q5; NOTES surface 3, "Show a
//! script library under {database}"): a local mapping of one database node to a library,
//! written only to `libraries.toml`, with the unchecked "also add as an alias" option.
//!
//! ## m4b-t01b-seams: nothing yet (Task 16); its menu item comes through
//! [`database_menu_items`](super::database_menu_items).
//!
//! ## m4b-t16-attach
//! - **Local by default** (Q5): "Show here" writes the mapping `(profile, database) → library`
//!   into `libraries.toml` and nothing into the library folder. Nothing about the connection
//!   is ever written into a library.
//! - **"Also add {database} as an alias for everyone who uses this folder"** is unchecked.
//!   Ticked, the dialog reads the library's manifest on the core runtime (`manifest::preview`,
//!   no link followed) and shows the edited file (`manifest::with_alias`, the added line on the
//!   selection tint); "Show here" then asks `LibraryService::map_sharing` to write that edit
//!   only over the bytes it showed, so what is written is what was shown, or nothing.
//! - Only libraries that do not show under this node already are listed; with none, the menu
//!   item is disabled ("No other library is attached").
//! - **Keys** (NOTES surface 3): Library → the box (Space) → Attach another folder… → Cancel →
//!   Show here (initial focus); Tab is trapped; Escape cancels; Enter presses the focused button.

use std::path::PathBuf;

use hacivat_l10n::ArgValue;
use hacivat_proto::ProfileId;
use hacivat_scripts::manifest::{self, ManifestPreview};
use hacivat_scripts::names::escape_untrusted;
use hacivat_scripts::service::MapError;
use hacivat_scripts::{DatabaseName, LibraryId, LocalMapping, LogicalDb};
use hacivat_ui::button::{Button, ButtonKind};
use hacivat_ui::capsule::env_swatch;
use hacivat_ui::checkbox::Checkbox;
use hacivat_ui::dialog::{DialogSize, dialog_frame};
use hacivat_ui::icon::{Icon, IconName, IconSize};
use hacivat_ui::l10n::ui_text;
use hacivat_ui::popover::{A11yNode, Declare as _};
use hacivat_ui::select::{Select, SelectEvent, SelectOption};
use hacivat_ui::text::{Label, TextSource, paragraph};
use hacivat_ui_core::gpui_surface::{
    AnyElement, App, AppContext as _, Context, Entity, FocusHandle, Focusable, Global,
    InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton, ParentElement as _, Render,
    Role, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, WeakEntity,
    Window, div, px,
};
use hacivat_ui_core::theme::Theme;

use super::attach_dialog::{self, Origin, caption, file_inset, ink_line};
use super::projection;
use crate::connections::ConnectionsModel;
use crate::l10n_global::tr_id;
use crate::shell::Workspace;
use crate::shell::layers::{DIALOG_CONTEXT, Layer};

/// The dialog's layer ID.
pub const MAP_LAYER: &str = "scripts-map";

fn text(cx: &App, id: &str, args: Vec<(&'static str, ArgValue)>) -> SharedString {
    crate::explorer::live::text(cx, id, args)
}

fn arg(s: &str) -> ArgValue {
    ArgValue::Text(s.to_string())
}

/// One library the dialog offers.
#[derive(Debug, Clone)]
struct Offer {
    id: LibraryId,
    /// Its folder name (display form).
    name: String,
    /// What it binds by name (display form).
    binds: String,
    /// Its root, canonical, and its folder's name as the manifest reader takes it.
    root: PathBuf,
    folder: String,
}

/// The libraries `(profile, database)` could show that it does not show now.
pub(crate) fn offers(profile: ProfileId, database: &str, cx: &App) -> Vec<LibraryId> {
    let Some(svc) = projection::service(cx) else {
        return Vec::new();
    };
    let here = projection::bound_at(profile, database, cx);
    svc.snapshot()
        .libraries
        .iter()
        .map(|l| l.id)
        .filter(|id| !here.contains(id))
        .collect()
}

fn offers_of(profile: ProfileId, database: &str, cx: &App) -> Vec<Offer> {
    let Some(svc) = projection::service(cx) else {
        return Vec::new();
    };
    let snap = svc.snapshot();
    let ids = offers(profile, database, cx);
    snap.libraries
        .iter()
        .filter(|l| ids.contains(&l.id))
        .map(|l| Offer {
            id: l.id,
            name: l
                .root
                .path
                .file_name()
                .map(|n| hacivat_scripts::pathsafe::display_name(&n.to_string_lossy()))
                .unwrap_or_default(),
            binds: l.db.display_escaped(),
            root: l.root.canonical.clone(),
            folder: l
                .root
                .path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string(),
        })
        .collect()
}

// ---- opening -----------------------------------------------------------------------------------

/// The mapping dialog for the node `(profile, database)` (the database menu's item).
pub fn open_for(profile: ProfileId, database: String, window: &mut Window, cx: &mut App) {
    let Some(ws) = window.root::<Workspace>().flatten() else {
        return;
    };
    if open_dialog(cx).is_some() {
        close(window, cx);
    }
    let dialog = cx.new(|cx| MapDialog::new(profile, database, window, cx));
    let root = dialog.read(cx).root.clone();
    ws.update(cx, |ws, cx| {
        ws.push_layer(
            Layer {
                id: MAP_LAYER,
                context: DIALOG_CONTEXT,
                view: dialog.clone().into(),
                restore_focus: None,
                modal: true,
            },
            window,
            cx,
        );
    });
    window.focus(&root, cx);
    cx.set_global(OpenMap(Some(dialog.downgrade())));
    dialog.update(cx, |d, cx| d.library_changed(window, cx));
}

struct OpenMap(Option<WeakEntity<MapDialog>>);

impl Global for OpenMap {}

/// The mapping dialog while it is open.
pub fn open_dialog(cx: &App) -> Option<Entity<MapDialog>> {
    cx.try_global::<OpenMap>()?.0.as_ref()?.upgrade()
}

fn close(window: &mut Window, cx: &mut App) {
    let (Some(ws), Some(d)) = (window.root::<Workspace>().flatten(), open_dialog(cx)) else {
        return;
    };
    let id = d.entity_id();
    ws.update(cx, |ws, cx| ws.remove_layer(id, window, cx));
    cx.set_global(OpenMap(None));
}

// ---- the dialog ----------------------------------------------------------------------------------

/// What the share box can do for the chosen library.
#[derive(Debug, Clone)]
enum Share {
    Reading,
    /// The manifest's bytes, and the file they become with the alias.
    Edit {
        before: Vec<u8>,
        after: String,
    },
    /// `hv-scripts-map-alias-off`'s variant.
    Off(&'static str),
}

/// The mapping dialog (NOTES surface 3).
pub struct MapDialog {
    profile: ProfileId,
    database: String,
    offers: Vec<Offer>,
    library: Entity<Select>,
    share: bool,
    can_share: Share,
    failed: Option<&'static str>,
    busy: bool,
    root: FocusHandle,
    share_focus: FocusHandle,
    other_focus: FocusHandle,
    cancel_focus: FocusHandle,
    ok_focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for MapDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.root.clone()
    }
}

impl MapDialog {
    fn new(
        profile: ProfileId,
        database: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let offers = offers_of(profile, &database, cx);
        let options = offers
            .iter()
            .map(|o| {
                let (name, binds) = (o.name.clone(), o.binds.clone());
                SelectOption {
                    label: std::rc::Rc::new(move |cx: &App| {
                        text(
                            cx,
                            "hv-scripts-map-option",
                            vec![("library", arg(&name)), ("database", arg(&binds))],
                        )
                    }),
                    hint: None,
                    disabled: false,
                }
            })
            .collect();
        let library = cx.new(|cx| {
            let mut s = Select::new(options, 0, cx);
            s.set_label(crate::l10n_global::text_fn_id("hv-scripts-map-library"));
            s
        });
        let sub = cx.subscribe_in(&library, window, |this, _, _: &SelectEvent, window, cx| {
            this.library_changed(window, cx);
        });
        MapDialog {
            profile,
            database,
            offers,
            library,
            share: false,
            can_share: Share::Reading,
            failed: None,
            busy: false,
            root: cx.focus_handle(),
            share_focus: cx.focus_handle().tab_stop(true),
            other_focus: cx.focus_handle().tab_stop(true),
            cancel_focus: cx.focus_handle().tab_stop(true),
            ok_focus: cx.focus_handle().tab_stop(true),
            _subscriptions: vec![sub],
        }
    }

    fn chosen(&self, cx: &App) -> Option<&Offer> {
        self.offers.get(self.library.read(cx).selected())
    }

    /// A library was chosen: its manifest is read for the share box (on the core runtime).
    fn library_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.failed = None;
        // ## m4b-t16-attach fix round 1 (m2): a tick was consent for the library it was given
        // for; another library starts unticked.
        self.share = false;
        let Some(offer) = self.chosen(cx).cloned() else {
            self.can_share = Share::Off("none");
            self.focus_first(window, cx);
            cx.notify();
            return;
        };
        let Ok(alias) = LogicalDb::parse(&self.database) else {
            self.can_share = Share::Off("name");
            self.focus_first(window, cx);
            cx.notify();
            return;
        };
        self.can_share = Share::Reading;
        cx.notify();
        let Some(rt) = cx.try_global::<ConnectionsModel>().map(|m| m.runtime()) else {
            return;
        };
        let (tx, rx) = tokio::sync::oneshot::channel();
        let root = offer.root.clone();
        rt.spawn_blocking(move || {
            let _ = tx.send(manifest::preview(&root));
        });
        let id = offer.id;
        cx.spawn_in(window, async move |this, cx| {
            let Ok(preview) = rx.await else {
                return;
            };
            let _ = this.update_in(cx, |d, window, cx| {
                if d.chosen(cx).map(|o| o.id) == Some(id) {
                    d.can_share = share_of(&preview, &offer.folder, &alias);
                    d.focus_first(window, cx);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Once ready, Show here has the focus (one press maps); nothing is offered: Cancel.
    fn focus_first(&self, window: &mut Window, cx: &mut Context<Self>) {
        if self.root.is_focused(window) {
            let target = if self.offers.is_empty() {
                &self.cancel_focus
            } else {
                &self.ok_focus
            };
            window.focus(target, cx);
        }
    }

    /// The libraries are listed and the chosen one's manifest was read.
    pub fn is_ready(&self) -> bool {
        !matches!(self.can_share, Share::Reading)
    }

    /// Whether the share box is ticked.
    pub fn shares_alias(&self) -> bool {
        self.share
    }

    // ## m4b-t16-attach fix round 1 (m6)
    /// The share box's and Show here's focus handles (tests assert where the focus is).
    pub fn focus_handles(&self) -> [FocusHandle; 2] {
        [self.share_focus.clone(), self.ok_focus.clone()]
    }

    fn toggle_share(&mut self, cx: &mut Context<Self>) {
        if matches!(self.can_share, Share::Edit { .. }) {
            self.share = !self.share;
            self.failed = None;
            cx.notify();
        }
    }

    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || !self.is_ready() {
            return;
        }
        let (Some(offer), Some(svc)) = (self.chosen(cx).cloned(), projection::service(cx)) else {
            return;
        };
        let m = LocalMapping {
            profile: self.profile,
            database: DatabaseName::new(&self.database),
            library: offer.id,
        };
        let before = match (&self.can_share, self.share) {
            (Share::Edit { before, .. }, true) => Some(before.clone()),
            _ => None,
        };
        self.busy = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = match before {
                Some(before) => svc.map_sharing(m, before).await,
                None => svc.map(m, false).await,
            };
            let _ = this.update_in(cx, |d, window, cx| match result {
                Ok(()) => close(window, cx),
                Err(e) => {
                    d.failed = Some(match e {
                        MapError::Alias(manifest::WriteError::Changed) => "changed",
                        MapError::Unavailable => "unavailable",
                        _ => "other",
                    });
                    d.busy = false;
                    // The file is read again, so what is shown is what is there now.
                    d.share = false;
                    d.library_changed(window, cx);
                }
            });
        })
        .detach();
    }

    /// "Attach another folder…": this dialog closes and the attach dialog opens for this node.
    fn attach_other(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let origin = Origin {
            profile: self.profile,
            database: self.database.clone(),
        };
        close(window, cx);
        // ## m4b-t16-attach fix round 1 (I5, R-T16-1): this node's name is local by
        // construction (Q5): the attach dialog is prefilled from the chosen folder's name.
        attach_dialog::start_with(Some(origin), attach_dialog::Prefill::Folder, window, cx);
    }

    fn stops(&self, cx: &App) -> Vec<FocusHandle> {
        vec![
            self.library.read(cx).focus_handle(cx),
            self.share_focus.clone(),
            self.other_focus.clone(),
            self.cancel_focus.clone(),
            self.ok_focus.clone(),
        ]
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let stroke = &event.keystroke;
        let m = &stroke.modifiers;
        let plain = !(m.control || m.platform || m.alt || m.shift);
        match stroke.key.as_str() {
            "escape" => close(window, cx),
            "enter" if plain && !event.is_held => {
                if self.cancel_focus.is_focused(window) {
                    close(window, cx);
                } else if self.other_focus.is_focused(window) {
                    self.attach_other(window, cx);
                } else {
                    // ## m4b-t16-attach fix round 1 (m3): Enter is the dialog's, as in every M2
                    // dialog: on the share box too it presses Show here (Space ticks it).
                    self.confirm(window, cx);
                }
            }
            "tab" if !m.control && !m.platform => {
                let order = self.stops(cx);
                let n = order.len();
                let at = order.iter().position(|h| h.is_focused(window));
                let next = match (at, m.shift) {
                    (Some(i), false) => (i + 1) % n,
                    (Some(i), true) => (i + n - 1) % n,
                    (None, false) => 0,
                    (None, true) => n - 1,
                };
                window.focus(&order[next], cx);
            }
            _ => return,
        }
        cx.stop_propagation();
    }
}

/// What the share box can do over a library's manifest.
fn share_of(preview: &ManifestPreview, folder: &str, alias: &LogicalDb) -> Share {
    if !preview.present {
        return Share::Off("none");
    }
    if preview.text.is_none() {
        return Share::Off("unreadable");
    }
    match manifest::with_alias(&preview.bytes, folder, alias) {
        Ok(Some(after)) => match String::from_utf8(after) {
            Ok(after) => Share::Edit {
                before: preview.bytes.clone(),
                after,
            },
            Err(_) => Share::Off("unreadable"),
        },
        // Bound by that name already: nothing to add.
        Ok(None) => Share::Off("name"),
        Err(_) => Share::Off("unreadable"),
    }
}

/// The lines of `after` that `before` does not have, in place (the common head and tail kept).
fn added_lines(before: &str, after: &str) -> Vec<usize> {
    let a: Vec<&str> = before.lines().collect();
    let b: Vec<&str> = after.lines().collect();
    let head = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let tail = a
        .iter()
        .rev()
        .zip(b.iter().rev())
        .take_while(|(x, y)| x == y)
        .count()
        .min(a.len().min(b.len()) - head);
    (head..b.len() - tail).collect()
}

impl Render for MapDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let shown_db = escape_untrusted(&self.database, 512);
        let title = text(
            cx,
            "hv-scripts-map-title",
            vec![("database", arg(&shown_db))],
        );
        let close_x = div()
            .id("scripts-map-close")
            .declare(Role::Button, ui_text(cx, "hui-dialog-close"), None)
            .flex()
            .items_center()
            .flex_none()
            .p(px(t.spacing.xs))
            .rounded(px(t.radii.sm))
            .cursor_pointer()
            .on_click(cx.listener(|_, _, window, cx| close(window, cx)))
            .child(
                Icon::new(IconName::X)
                    .size(IconSize::Chrome)
                    .color(c.text_2.to_hsla()),
            );
        let subtitle = cx
            .try_global::<ConnectionsModel>()
            .and_then(|m| m.get(self.profile))
            .map(|p| {
                let line = text(
                    cx,
                    "hv-scripts-attach-from",
                    vec![
                        ("conn", arg(&escape_untrusted(&p.name, 512))),
                        ("database", arg(&shown_db)),
                    ],
                );
                div()
                    .mt(px(-t.spacing.sm))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(t.spacing.sm))
                    .child(env_swatch(
                        crate::connections::capsule::env_kind(p.environment),
                        &theme,
                    ))
                    .child(
                        Label::new(
                            line,
                            TextSource::Data,
                            t.type_ramp.body_sm,
                            c.text_2.to_hsla(),
                        )
                        .truncate(true)
                        .id("scripts-map-subtitle"),
                    )
            });
        let mut body = div().flex().flex_col().gap(px(t.spacing.xl));
        if self.offers.is_empty() {
            body = body.child(
                paragraph(
                    tr_id(cx, "hv-explorer-menu-map-library-none"),
                    TextSource::Fluent,
                    t.type_ramp.body_sm,
                    c.text_2.to_hsla(),
                )
                .id("scripts-map-none"),
            );
        } else {
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(t.spacing.xs))
                    .child(caption(tr_id(cx, "hv-scripts-map-library"), &theme))
                    .child(
                        div()
                            .debug_selector(|| "scripts-map-library".to_string())
                            .child(self.library.clone()),
                    ),
            );
        }
        body = body.child(
            paragraph(
                tr_id(cx, "hv-scripts-map-note"),
                TextSource::Fluent,
                t.type_ramp.body_sm,
                c.text_2.to_hsla(),
            )
            .id("scripts-map-note"),
        );
        let label = text(
            cx,
            "hv-scripts-map-alias",
            vec![("database", arg(&shown_db))],
        );
        let sub = match &self.can_share {
            Share::Off(reason) => text(
                cx,
                "hv-scripts-map-alias-off",
                vec![("reason", arg(reason))],
            ),
            _ => tr_id(cx, "hv-scripts-map-alias-sub"),
        };
        let enabled = matches!(self.can_share, Share::Edit { .. });
        let this = cx.entity().downgrade();
        body = body.child(
            div()
                .debug_selector(|| "scripts-map-share".to_string())
                .child(
                    Checkbox::new("scripts-map-share", label)
                        .checked(self.share)
                        .sub_label(sub)
                        .disabled(!enabled)
                        .wrap(true)
                        .focus_handle(self.share_focus.clone())
                        .on_toggle(move |_, _, cx| {
                            if let Some(d) = this.upgrade() {
                                d.update(cx, |d, cx| d.toggle_share(cx));
                            }
                        }),
                ),
        );
        if let (true, Share::Edit { before, after }) = (self.share, &self.can_share)
            && let Some(offer) = self.chosen(cx)
        {
            let before = String::from_utf8_lossy(before);
            let marked = added_lines(&before, after);
            body = body.child(file_inset(
                &offer.root.join(".hacivat").join("workspace.toml"),
                after,
                &marked,
                &theme,
                cx,
            ));
        }
        if let Some(reason) = self.failed {
            body = body.child(ink_line(
                text(cx, "hv-scripts-map-failed", vec![("reason", arg(reason))]),
                "scripts-map-error",
                &theme,
            ));
        }
        let other = Button::new("scripts-map-other", tr_id(cx, "hv-scripts-map-other"))
            .focus_handle(self.other_focus.clone())
            .on_click(cx.listener(|this, _, window, cx| this.attach_other(window, cx)));
        let cancel = Button::new("scripts-map-cancel", tr_id(cx, "hv-tx-cancel"))
            .focus_handle(self.cancel_focus.clone())
            .on_click(cx.listener(|_, _, window, cx| close(window, cx)));
        let ok = Button::new("scripts-map-ok", tr_id(cx, "hv-scripts-map-btn"))
            .kind(ButtonKind::Primary)
            .disabled(self.offers.is_empty() || self.busy || !self.is_ready())
            .focus_handle(self.ok_focus.clone())
            .on_click(cx.listener(|this, _, window, cx| this.confirm(window, cx)));
        let card = dialog_frame(&theme, DialogSize::Wide)
            .id(MAP_LAYER)
            .debug_selector(|| MAP_LAYER.to_string())
            .track_focus(&self.root)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.on_key(event, window, cx);
            }))
            .capture_key_up(|event, window, _| {
                let stroke = &event.keystroke;
                if stroke.key == "enter" && !stroke.modifiers.modified() {
                    window.prevent_default();
                }
            })
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_start()
                    .justify_between()
                    .gap(px(t.spacing.xl))
                    .child(div().flex_1().min_w_0().child(paragraph(
                        title.clone(),
                        TextSource::Data,
                        t.type_ramp.body_strong,
                        c.text.to_hsla(),
                    )))
                    .child(close_x),
            )
            .children(subtitle)
            .child(
                div()
                    .id("scripts-map-body")
                    .flex()
                    .flex_col()
                    .flex_shrink(1.0)
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(hacivat_ui::text::scroll_viewport())
                    .child(body),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .gap(px(t.spacing.md))
                    .child(other)
                    // ## m4b-t16-attach fix round 1 (m9): Cancel and Show here stay together and
                    // to the right, also on a second line when a long locale wraps the footer.
                    .child(
                        div()
                            .ml_auto()
                            .flex()
                            .flex_row()
                            .flex_none()
                            .gap(px(t.spacing.md))
                            .child(cancel)
                            .child(ok),
                    ),
            );
        let node: AnyElement =
            A11yNode::new("scripts-map-node", Role::Dialog, title, true, card).into_any_element();
        node
    }
}

/// Shot fixtures.
#[cfg(feature = "test-support")]
pub mod fixtures {
    use super::*;

    /// Ticks the share box, as Space on it does.
    pub fn tick_share(cx: &mut App) {
        if let Some(d) = open_dialog(cx) {
            d.update(cx, |d, cx| d.toggle_share(cx));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_added_lines_are_found_between_the_kept_head_and_tail() {
        let before = "# keep\nformat = 1\ndatabase = \"orders\"\n\n# tail\n";
        let after =
            "# keep\nformat = 1\ndatabase = \"orders\"\naliases = [\"orders_p\"]\n\n# tail\n";
        assert_eq!(added_lines(before, after), [3]);
        let before = "format = 1\naliases = [\"a\"]\n";
        let after = "format = 1\naliases = [\"a\", \"b\"]\n";
        assert_eq!(added_lines(before, after), [1]);
        assert_eq!(added_lines("a\n", "a\n"), Vec::<usize>::new());
    }

    #[test]
    fn the_share_box_is_off_without_a_readable_manifest() {
        let alias = LogicalDb::parse("orders_p").unwrap();
        assert!(matches!(
            share_of(&ManifestPreview::default(), "x", &alias),
            Share::Off("none")
        ));
        let unreadable = ManifestPreview {
            present: true,
            ..ManifestPreview::default()
        };
        assert!(matches!(
            share_of(&unreadable, "x", &alias),
            Share::Off("unreadable")
        ));
        let bytes = b"format = 1\ndatabase = \"orders\"\n".to_vec();
        let ok = ManifestPreview {
            present: true,
            text: Some(String::from_utf8(bytes.clone()).unwrap()),
            bytes,
            ..ManifestPreview::default()
        };
        assert!(matches!(share_of(&ok, "x", &alias), Share::Edit { .. }));
        assert!(matches!(
            share_of(&ok, "x", &LogicalDb::parse("ORDERS").unwrap()),
            Share::Off("name")
        ));
    }
}
// ## m4b-t16-attach
