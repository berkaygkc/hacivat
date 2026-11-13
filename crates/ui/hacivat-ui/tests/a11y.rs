mod support;

use std::rc::Rc;

use gpui::{App, Role, TestAppContext, VisualTestContext, point, px};
use hacivat_ui::button::{Button, ButtonKind};
use hacivat_ui::dialog::{Dialog, DialogButton, DialogSize, DialogSpec};
use hacivat_ui::icon::IconName;
use hacivat_ui::menu::{MenuItemSpec, PopupMenu};
use hacivat_ui::panes::{PaneId, TabId, TabSpec, TabStrip};
use hacivat_ui::popover::{A11yDecl, a11y_record, a11y_take};
use hacivat_ui::text::{TextFn, TextSource};
use hacivat_ui::toast::{Toast, ToastKind, ToastQueue, ToastStack};
use hacivat_ui_core::gpui_surface::{IntoElement, ParentElement, Styled, accesskit, div};
use hacivat_ui_core::theme::{Theme, ThemeMode};
use support::host;

/// `(role, name, selected)` of every node in the window's accessibility tree.
type Node = (Role, String, bool);

/// What the widgets declared while the window drew: GPUI builds its AccessKit tree only while
/// assistive technology is attached (`Window::is_a11y_active`) and the test platforms never
/// attach one, so `debug_a11y_tree_json` stays empty here. The widgets record their `.role(..)`
/// / `.aria_label(..)` declarations (`popover::Declare`) and the tests read those back.
fn a11y_nodes(_: &mut VisualTestContext) -> Vec<Node> {
    a11y_take()
        .into_iter()
        .filter_map(|d| Some((d.role?, d.name.to_string(), d.selected.unwrap_or(false))))
        .collect()
}

fn tab(id: u32, title: &'static str) -> TabSpec {
    TabSpec {
        id: TabId(id),
        title: title.into(),
        title_source: TextSource::Data,
        dirty: false,
        env: None,
    }
}

fn text(s: &'static str) -> TextFn {
    Rc::new(move |_: &App| s.into())
}

#[gpui::test]
fn chrome_exposes_roles_and_names(cx: &mut TestAppContext) {
    a11y_record();
    cx.update(|cx| {
        Theme::install(cx, ThemeMode::Dark);
        ToastQueue::push(cx, Toast::new(ToastKind::Info, text("Saved")));
        ToastQueue::push(cx, Toast::new(ToastKind::Error, text("Failed")));
    });
    let vcx = host(cx, |_, _| {
        div()
            .child(Button::new("save", "Save".into()))
            .child(
                Button::new("more", "More actions".into())
                    .kind(ButtonKind::Icon)
                    .icon(IconName::Ellipsis),
            )
            .child(
                TabStrip::new(PaneId(0), vec![tab(1, "Console 1"), tab(2, "Console 2")]).active(0),
            )
            .child(ToastStack::new())
            .into_any_element()
    });
    let n = a11y_nodes(vcx);
    assert!(n.contains(&(Role::Button, "Save".into(), false)));
    assert!(n.contains(&(Role::Button, "More actions".into(), false)));
    assert!(n.iter().any(|x| x.0 == Role::TabList));
    assert!(n.contains(&(Role::Tab, "Console 1".into(), true)));
    assert!(n.contains(&(Role::Tab, "Console 2".into(), false)));
    assert!(n.contains(&(Role::Status, "Saved".into(), false)));
    assert!(n.contains(&(Role::Alert, "Failed".into(), false)));
}

#[gpui::test]
fn dialog_and_popup_menu_expose_roles_and_names(cx: &mut TestAppContext) {
    cx.update(|cx| Theme::install(cx, ThemeMode::Dark));
    a11y_record();
    let spec = DialogSpec {
        title: text("Delete?"),
        body: text("Gone for good."),
        buttons: ["Cancel", "Delete"]
            .into_iter()
            .map(|l| DialogButton {
                label: text(l),
                kind: ButtonKind::Default,
                accesskey: None,
                english_accesskey: None,
            })
            .collect(),
        cancel: 0,
        primary: 1,
        size: DialogSize::Standard,
    };
    let (host_view, vcx) =
        cx.add_window_view(|window, cx| Wrap::Dialog(Dialog::new(spec, window, cx)));
    let _ = host_view;
    let n = a11y_nodes(vcx);
    assert!(n.contains(&(Role::Dialog, "Delete?".into(), false)));
    assert!(n.contains(&(Role::Button, "Cancel".into(), false)));
    assert!(n.contains(&(Role::Button, "Delete".into(), false)));

    a11y_record();
    let (_, vcx) = cx.add_window_view(|window, cx| {
        Wrap::Menu(PopupMenu::new(
            point(px(10.), px(10.)),
            vec![
                MenuItemSpec::new(text("Copy"), "edit.copy"),
                MenuItemSpec::new(text("Paste"), "edit.paste"),
            ],
            window,
            cx,
        ))
    });
    let n = a11y_nodes(vcx);
    assert!(n.iter().any(|x| x.0 == Role::Menu));
    assert!(n.contains(&(Role::MenuItem, "Copy".into(), false)));
    assert!(n.contains(&(Role::MenuItem, "Paste".into(), false)));
}

fn declared(decls: &[A11yDecl], role: Role, name: &str) -> A11yDecl {
    decls
        .iter()
        .find(|d| d.role == Some(role) && d.name.as_ref() == name)
        .unwrap_or_else(|| panic!("no {role:?} named {name:?} in {decls:?}"))
        .clone()
}

#[gpui::test]
fn toasts_are_live_regions_with_their_text_as_value(cx: &mut TestAppContext) {
    cx.update(|cx| {
        Theme::install(cx, ThemeMode::Dark);
        ToastQueue::push(cx, Toast::new(ToastKind::Info, text("Saved")));
        ToastQueue::push(cx, Toast::new(ToastKind::Error, text("Failed")));
    });
    a11y_record();
    host(cx, |_, _| div().child(ToastStack::new()).into_any_element());
    let decls = a11y_take();
    let info = declared(&decls, Role::Status, "Saved");
    assert_eq!(
        (info.live, info.value.as_deref()),
        (Some(accesskit::Live::Polite), Some("Saved"))
    );
    let error = declared(&decls, Role::Alert, "Failed");
    assert_eq!(
        (error.live, error.value.as_deref()),
        (Some(accesskit::Live::Assertive), Some("Failed"))
    );
}

#[gpui::test]
fn dialog_is_modal_and_described_by_its_body(cx: &mut TestAppContext) {
    cx.update(|cx| Theme::install(cx, ThemeMode::Dark));
    a11y_record();
    let spec = DialogSpec {
        title: text("Delete?"),
        body: text("Gone for good."),
        buttons: vec![DialogButton {
            label: text("OK"),
            kind: ButtonKind::Default,
            accesskey: None,
            english_accesskey: None,
        }],
        cancel: 0,
        primary: 0,
        size: DialogSize::Standard,
    };
    let _ = cx.add_window_view(|window, cx| Wrap::Dialog(Dialog::new(spec, window, cx)));
    let dialog = declared(&a11y_take(), Role::Dialog, "Delete?");
    assert!(dialog.modal);
    assert_eq!(dialog.description.as_deref(), Some("Gone for good."));
}

#[gpui::test]
fn disabled_button_and_menu_item_are_marked_disabled(cx: &mut TestAppContext) {
    cx.update(|cx| Theme::install(cx, ThemeMode::Dark));
    a11y_record();
    host(cx, |_, _| {
        div()
            .child(Button::new("on", "Run".into()))
            .child(Button::new("off", "Stop".into()).disabled(true))
            .into_any_element()
    });
    let decls = a11y_take();
    assert!(!declared(&decls, Role::Button, "Run").disabled);
    assert!(declared(&decls, Role::Button, "Stop").disabled);

    a11y_record();
    let _ = cx.add_window_view(|window, cx| {
        Wrap::Menu(PopupMenu::new(
            point(px(0.), px(0.)),
            vec![
                MenuItemSpec::new(text("Cut"), "cut"),
                MenuItemSpec::new(text("Copy"), "copy").disabled(true),
            ],
            window,
            cx,
        ))
    });
    let decls = a11y_take();
    assert!(declared(&decls, Role::MenuItem, "Copy").disabled);
    assert!(!declared(&decls, Role::MenuItem, "Cut").disabled);
}

#[gpui::test]
fn the_highlighted_menu_row_is_the_active_descendant(cx: &mut TestAppContext) {
    cx.update(|cx| Theme::install(cx, ThemeMode::Dark));
    a11y_record();
    let _ = cx.add_window_view(|window, cx| {
        Wrap::Menu(PopupMenu::new(
            point(px(0.), px(0.)),
            vec![
                MenuItemSpec::new(text("Cut"), "cut"),
                MenuItemSpec::new(text("Copy"), "copy"),
            ],
            window,
            cx,
        ))
    });
    let decls = a11y_take();
    assert!(declared(&decls, Role::MenuItem, "Cut").active_descendant);
    assert!(!declared(&decls, Role::MenuItem, "Copy").active_descendant);
}

#[gpui::test]
fn capsule_describes_environment_access_and_path(cx: &mut TestAppContext) {
    use hacivat_ui::capsule::{Capsule, CapsuleModel, EnvKind};
    a11y_record();
    host(cx, |_, _| {
        div()
            .child(Capsule::new(CapsuleModel::Connection {
                env: EnvKind::Production,
                env_label: "PRODUCTION".into(),
                name: "orders-prod".into(),
                engine: "PostgreSQL 16".into(),
                access: "read-only".into(),
                path: "Direct".into(),
                read_only: true,
                direct: true,
                latency: None,
                unencrypted: None,
                unsupported: None,
            }))
            .into_any_element()
    });
    let capsule = declared(&a11y_take(), Role::Button, "orders-prod");
    assert_eq!(
        capsule.description.as_deref(),
        Some("PRODUCTION, read-only, Direct")
    );
}

enum Wrap {
    Dialog(gpui::Entity<Dialog>),
    Menu(gpui::Entity<PopupMenu>),
}

impl gpui::Render for Wrap {
    fn render(&mut self, _: &mut gpui::Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        let el = div().size_full();
        match self {
            Wrap::Dialog(d) => el.child(d.clone()),
            Wrap::Menu(m) => el.child(m.clone()),
        }
    }
}
