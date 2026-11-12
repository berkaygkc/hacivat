mod support;

use std::rc::Rc;

use gpui::{App, Role, TestAppContext, VisualTestContext, point, px};
use hacivat_ui::button::{Button, ButtonKind};
use hacivat_ui::dialog::{Dialog, DialogButton, DialogSpec};
use hacivat_ui::icon::IconName;
use hacivat_ui::menu::{MenuItemSpec, PopupMenu};
use hacivat_ui::panes::{PaneId, TabId, TabSpec, TabStrip};
use hacivat_ui::popover::{a11y_record, a11y_take};
use hacivat_ui::text::{TextFn, TextSource};
use hacivat_ui::toast::{Toast, ToastKind, ToastQueue, ToastStack};
use hacivat_ui_core::gpui_surface::{IntoElement, ParentElement, Styled, div};
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
        .map(|(role, name, selected)| (role, name.to_string(), selected.unwrap_or(false)))
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
