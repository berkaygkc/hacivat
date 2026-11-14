//! M3a Task 1b: the editor's seams fail closed, the minimal view and input handler work, and
//! the command table matches the keymap rows.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    AppContext as _, Context, Entity, EntityInputHandler as _, IntoElement, ParentElement as _,
    Render, Styled as _, TestAppContext, VisualTestContext, Window, div,
};
use hacivat_text::TextBuffer;
use hacivat_ui::keymaps::{PRESETS, load};
use hacivat_ui_core::keymap::KeymapPlatform;
use hacivat_ui_editor::{
    DocId, EditorConfig, EditorEvent, EditorView, ExecAction, ExecAt, ExecMarkState,
    FailClosedService, LanguageService, MarkKey, PreviewState, commands, large_file,
};
use smallvec::smallvec;

#[test]
fn fail_closed_service_never_previews_a_range() {
    let p = FailClosedService
        .exec_preview(
            DocId(1),
            0,
            &ExecAt::Carets(smallvec![0]),
            ExecAction::Execute1,
        )
        .unwrap();
    assert!(p.ranges.is_empty());
    assert_eq!(p.state, PreviewState::Uncertain);
    assert!(FailClosedService.statements(DocId(1), 0, 0..10).is_empty());
    assert!(FailClosedService.decoration_sources(DocId(1)).is_empty());
}

#[test]
fn every_bound_editor_command_is_in_the_table() {
    let ids: Vec<&str> = commands().iter().map(|c| c.id).collect();
    for p in [
        KeymapPlatform::MacOs,
        KeymapPlatform::Windows,
        KeymapPlatform::Linux,
    ] {
        for preset in PRESETS {
            for b in load(p, preset, None).0.bindings() {
                if b.command.starts_with("editor.") {
                    assert!(ids.contains(&b.command.as_str()), "{} {p:?}", b.command);
                }
            }
        }
    }
    // Palette-only in M3a (R-M3-38).
    for id in ["editor.run_2", "editor.run_3", "editor.describe_exec_range"] {
        assert!(ids.contains(&id), "{id}");
    }
    assert!(
        commands()
            .iter()
            .all(|c| c.title_id.starts_with("hv-cmd-editor-"))
    );
}

#[test]
fn tiers_follow_the_size() {
    let limits = EditorConfig::default().tiers;
    assert_eq!(
        large_file::tier_for(3 << 20, 60_000, &limits),
        hacivat_ui_editor::Tier::Full
    );
    assert_eq!(
        large_file::tier_for(12 << 20, 1_000, &limits),
        hacivat_ui_editor::Tier::Reduced
    );
}

/// A window whose root holds one editor.
struct Host {
    editor: Entity<EditorView>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.editor.clone())
    }
}

fn editor<'a>(
    cx: &'a mut TestAppContext,
    text: &'static str,
    cfg: EditorConfig,
) -> (Entity<EditorView>, &'a mut VisualTestContext) {
    let slot: Rc<RefCell<Option<Entity<EditorView>>>> = Rc::default();
    let out = slot.clone();
    let (_, vcx) = cx.add_window_view(move |window, cx| {
        let editor = cx.new(|cx| {
            EditorView::new(
                DocId(7),
                text,
                cfg,
                Rc::new(FailClosedService),
                Rc::new(|_: &gpui::App| "Console 1".into()),
                window,
                cx,
            )
        });
        *out.borrow_mut() = Some(editor.clone());
        Host { editor }
    });
    let editor = slot.borrow().clone().unwrap();
    (editor, vcx)
}

fn text(e: &Entity<EditorView>, vcx: &VisualTestContext) -> String {
    vcx.read(|cx| e.read(cx).snapshot().buffer.to_string())
}

#[gpui::test]
fn plain_text_input_goes_through_the_model(cx: &mut TestAppContext) {
    let (e, vcx) = editor(cx, "SELECT ;", EditorConfig::default());
    let events: Rc<RefCell<Vec<String>>> = Rc::default();
    let seen = events.clone();
    vcx.update(|_, cx| {
        cx.subscribe(&e, move |_, ev: &EditorEvent, _| {
            seen.borrow_mut().push(format!("{ev:?}"));
        })
        .detach();
    });
    // Insert at UTF-16 7..7, then type at the caret the insert left.
    e.update_in(vcx, |v, window, cx| {
        v.replace_text_in_range(Some(7..7), "1", window, cx);
        v.replace_text_in_range(None, "ş", window, cx);
    });
    assert_eq!(text(&e, vcx), "SELECT 1ş;");
    let status = vcx.read(|cx| e.read(cx).status());
    assert_eq!((status.line, status.column), (1, 10));
    assert_eq!(vcx.read(|cx| e.read(cx).snapshot().version), 2);
    assert!(
        events
            .borrow()
            .iter()
            .any(|e| e == "Changed { version: 2 }"),
        "{:?}",
        events.borrow()
    );
    // The selection reported to the OS is in UTF-16 units.
    let sel = e.update_in(vcx, |v, window, cx| {
        v.selected_text_range(false, window, cx)
    });
    assert_eq!(sel.unwrap().range, 9..9);
    // Selecting a range and replacing it.
    e.update(vcx, |v, cx| v.select_range(0..6, cx));
    e.update_in(vcx, |v, window, cx| {
        v.replace_text_in_range(None, "VALUES", window, cx)
    });
    assert_eq!(text(&e, vcx), "VALUES 1ş;");
    e.update(vcx, |v, cx| v.set_text("SELECT 2", cx));
    assert_eq!(text(&e, vcx), "SELECT 2");
}

#[gpui::test]
fn a_read_only_editor_takes_no_text(cx: &mut TestAppContext) {
    let cfg = EditorConfig {
        read_only: true,
        ..EditorConfig::default()
    };
    let (e, vcx) = editor(cx, "SELECT 1", cfg);
    let accepts = e.update_in(vcx, |v, window, cx| v.accepts_text_input(window, cx));
    assert!(!accepts);
    e.update_in(vcx, |v, window, cx| {
        v.replace_text_in_range(None, "x", window, cx)
    });
    assert_eq!(text(&e, vcx), "SELECT 1");
}

#[gpui::test]
fn marks_and_commands_delegate_to_their_modules(cx: &mut TestAppContext) {
    let (e, vcx) = editor(cx, "SELECT 1;\nSELECT 2;", EditorConfig::default());
    e.update(vcx, |v, cx| {
        v.add_exec_marks(0, vec![(MarkKey(1), 0..8), (MarkKey(2), 10..18)], cx);
        v.set_exec_state(MarkKey(2), ExecMarkState::Skipped, None, None, cx);
    });
    vcx.read(|cx| {
        let marks = e.read(cx).marks();
        assert_eq!(marks.len(), 2);
        assert_eq!(marks.state(MarkKey(2)), Some(&ExecMarkState::Skipped));
        assert_eq!(marks.state(MarkKey(1)), None);
    });
    e.update(vcx, |v, cx| v.clear_exec_marks(cx));
    assert!(vcx.read(|cx| e.read(cx).marks().is_empty()));
    // Known commands run (Task 1b: as no-ops), unknown ones are refused.
    let ran = e.update_in(vcx, |v, window, cx| {
        (
            v.run_command("editor.run", window, cx),
            v.run_command("editor.fly", window, cx),
        )
    });
    assert_eq!(ran, (true, false));
    // Nothing is laid out before Task 19's element.
    assert!(vcx.read(|cx| e.read(cx).caret_bounds().is_none()));
    assert!(vcx.read(|cx| e.read(cx).last_layout().is_none()));
    let _ = TextBuffer::from_str("");
}
