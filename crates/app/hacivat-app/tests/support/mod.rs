//! Shared app test helpers (Tasks 13, 16, 17, 18 and 19): a real shell in a test window, and
//! getters that read what the last frame drew (the text registry) or the shell's state.
//!
//! `shell*` return a `&'static mut VisualTestContext`, as `add_window_view` does, so a test can
//! open several shells in one app. Every getter takes `&VisualTestContext`. Every text getter
//! strips the Unicode isolation marks U+2068/U+2069.
#![allow(dead_code)]

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use gpui::{
    AnyWindowHandle, App, AppContext as _, BorrowAppContext as _, Context, Entity, FocusHandle,
    Focusable as _, Global, Hsla, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    Styled as _, TestAppContext, VisualContext as _, VisualTestContext, Window, div,
};
use hacivat_app::l10n_global::{L10nGlobal, switch_locale, text_fn_id, tr_id};
use hacivat_app::shell::{
    ActiveKeymap, DemoCapsule, Layer, TabContent, TabContentRegistry, TileLayout, Workspace,
    install_keymap_with, set_key_capture,
};
use hacivat_app::theme_pref::apply_theme;
use hacivat_app::{AppState, Startup, start};
use hacivat_l10n::{
    ArgValue, Catalogs, FormatInputs, L10n, LoadDiag, NegotiationCtx, OsLocale, PrefSource,
    build_runtime, negotiate,
};
use hacivat_settings::{LanguagePref, LocalPaths, ThemePref};
use hacivat_ui::actions::ActionRegistry;
use hacivat_ui::button::ButtonKind;
use hacivat_ui::dialog::{DialogButton, DialogSize, DialogSpec};
use hacivat_ui::input::TextInput;
use hacivat_ui::l10n::UiArg;
use hacivat_ui::panes::PaneTree;
use hacivat_ui::text::{RenderedText, TextRegistry};
use hacivat_ui::toast::{Toast, ToastKind, ToastQueue};
use hacivat_ui_core::keymap::{ContextStack, KeyboardLayoutId, KeymapPlatform};

/// Temp roots kept alive for the app's lifetime (the shell writes `workspace.json` there).
#[derive(Default)]
struct TempRoots(Vec<tempfile::TempDir>);

impl Global for TempRoots {}

/// Text inputs built by test tabs, newest last.
#[derive(Default)]
struct TestInputs(Vec<Entity<TextInput>>);

impl Global for TestInputs {}

/// A shell in `lang` over a fresh temp root.
pub fn shell(cx: &mut TestAppContext, lang: &str) -> &'static mut VisualTestContext {
    let root = tempfile::tempdir().expect("temp dir");
    let path = root.path().to_path_buf();
    cx.update(|cx| cx.default_global::<TempRoots>().0.push(root));
    shell_at(cx, &path, lang)
}

/// A shell over `root` (`LocalPaths::under(root)`): reads `config/settings.toml`, the overlay in
/// `config/locales` and `data/workspace.json` from there. `ui.language` is set to `lang` in
/// memory.
pub fn shell_at(
    cx: &mut TestAppContext,
    root: &Path,
    lang: &str,
) -> &'static mut VisualTestContext {
    let window = boot(cx, root, lang, None);
    finish(cx, window)
}

/// The `--vf-demo` shell: the harman production capsule, in English.
pub fn shell_demo(cx: &mut TestAppContext) -> &'static mut VisualTestContext {
    let root = tempfile::tempdir().expect("temp dir");
    let path = root.path().to_path_buf();
    cx.update(|cx| cx.default_global::<TempRoots>().0.push(root));
    let window = boot(cx, &path, "en", Some(DemoCapsule::harman()));
    finish(cx, window)
}

/// An English shell whose `ActiveKeymap` is loaded for `platform` and `layout` with the `user`
/// keymap text.
pub fn shell_with_keymap(
    cx: &mut TestAppContext,
    platform: KeymapPlatform,
    layout: &str,
    user: Option<&str>,
) -> &'static mut VisualTestContext {
    let vcx = shell(cx, "en");
    let layout = KeyboardLayoutId(layout.to_string());
    let user = user.map(str::to_string);
    vcx.cx
        .update(|cx| install_keymap_with(cx, platform, layout, user));
    vcx.run_until_parked();
    vcx
}

fn boot(
    cx: &mut TestAppContext,
    root: &Path,
    lang: &str,
    demo: Option<DemoCapsule>,
) -> AnyWindowHandle {
    let paths = LocalPaths::under(root);
    paths.ensure_dirs().expect("test dirs");
    let mut outcome = hacivat_settings::load(&paths.settings_toml());
    outcome.settings.ui.language = LanguagePref::Tag(lang.to_string());
    let startup = Startup::new(paths, outcome, OsLocale::default());
    cx.update(|cx| {
        cx.set_global(TextRegistry {
            enabled: true,
            frame: Vec::new(),
        });
        start(cx, startup, demo).expect("open the workspace").into()
    })
}

fn finish(cx: &mut TestAppContext, window: AnyWindowHandle) -> &'static mut VisualTestContext {
    let vcx = VisualTestContext::from_window(window, cx).into_mut();
    vcx.run_until_parked();
    vcx
}

/// Runs `f` with the shell's window.
pub fn in_window<R>(vcx: &VisualTestContext, f: impl FnOnce(&mut Window, &mut App) -> R) -> R {
    let handle = vcx.window_handle();
    vcx.cx
        .update(|cx| cx.update_window(handle, |_, window, cx| f(window, cx)))
        .expect("the shell window is open")
}

/// The shell's root view.
pub fn workspace(vcx: &VisualTestContext) -> Entity<Workspace> {
    in_window(vcx, |window, _| window.root::<Workspace>())
        .flatten()
        .expect("the root view is the workspace")
}

/// Runs `f` on the workspace inside its window.
pub fn with_ws<R>(
    vcx: &VisualTestContext,
    f: impl FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>) -> R,
) -> R {
    let ws = workspace(vcx);
    in_window(vcx, |window, cx| ws.update(cx, |w, cx| f(w, window, cx)))
}

/// Writes `ui.language = lang` into the in-memory settings and switches to it.
pub fn switch_to(vcx: &VisualTestContext, lang: &str) {
    vcx.cx.update(|cx| {
        let settings = cx.update_global::<AppState, _>(|state, _| {
            state.settings.ui.language = LanguagePref::Tag(lang.to_string());
            state.settings.clone()
        });
        switch_locale(cx, &settings).detach();
    });
}

/// Runs a registered action (`ActionRegistry::run`); false when it is unknown or disabled.
pub fn run(vcx: &VisualTestContext, action_id: &str) -> bool {
    let ran = in_window(vcx, |window, cx| ActionRegistry::run(action_id, window, cx));
    vcx.run_until_parked();
    ran
}

/// Applies a theme preference.
pub fn apply(vcx: &VisualTestContext, pref: ThemePref) {
    vcx.cx.update(|cx| apply_theme(cx, pref));
}

/// Turns the text registry on and repaints.
pub fn enable_registry(vcx: &VisualTestContext) {
    vcx.cx.update(|cx| {
        cx.set_global(TextRegistry {
            enabled: true,
            frame: Vec::new(),
        });
        cx.refresh_windows();
    });
}

fn strip(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '\u{2068}' | '\u{2069}'))
        .collect()
}

/// What the last frame drew, isolation marks stripped. It never repaints by itself, so a test
/// sees whether the app repainted (after a language switch, say).
pub fn registry_frame(vcx: &VisualTestContext) -> Vec<RenderedText> {
    vcx.run_until_parked();
    vcx.cx
        .read(|cx| {
            cx.try_global::<TextRegistry>()
                .map(|r| r.frame.clone())
                .unwrap_or_default()
        })
        .into_iter()
        .map(|mut r| {
            r.text = strip(&r.text).into();
            r
        })
        .collect()
}

/// Texts drawn by `file` (a path suffix such as `hacivat-ui/src/toast.rs`), in paint order.
fn texts_from(vcx: &VisualTestContext, file: &str) -> Vec<String> {
    let needle = format!("{file}:");
    registry_frame(vcx)
        .into_iter()
        .filter(|r| r.path.contains(&needle))
        .map(|r| r.text.to_string())
        .collect()
}

/// The text of the label with element ID `id`.
fn text_with_id(vcx: &VisualTestContext, id: &str) -> String {
    let needle = format!("#{id}");
    registry_frame(vcx)
        .into_iter()
        .find(|r| r.path.ends_with(&needle))
        .map(|r| r.text.to_string())
        .unwrap_or_else(|| panic!("no label `{id}` in the frame"))
}

/// The title bar's palette field placeholder (label ID `palette-placeholder`).
pub fn palette_label(vcx: &VisualTestContext) -> String {
    text_with_id(vcx, "palette-placeholder")
}

/// The explorer panel's title.
pub fn panel_title(vcx: &VisualTestContext) -> String {
    text_with_id(vcx, "panel-explorer-title")
}

/// The explorer's empty-state text.
pub fn explorer_empty(vcx: &VisualTestContext) -> String {
    text_with_id(vcx, "explorer-empty")
}

/// The visible toasts' texts, oldest first.
pub fn toast_texts(vcx: &VisualTestContext) -> Vec<String> {
    texts_from(vcx, "hacivat-ui/src/toast.rs")
}

/// The open dialog's title.
pub fn dialog_title(vcx: &VisualTestContext) -> String {
    texts_from(vcx, "hacivat-ui/src/dialog.rs")
        .first()
        .cloned()
        .expect("a dialog is open")
}

/// The open dialog's body.
pub fn dialog_body(vcx: &VisualTestContext) -> String {
    texts_from(vcx, "hacivat-ui/src/dialog.rs")
        .get(1)
        .cloned()
        .expect("a dialog is open")
}

/// Everything the status bar drew, joined by spaces.
pub fn status_bar_text(vcx: &VisualTestContext) -> String {
    registry_frame(vcx)
        .into_iter()
        .filter(|r| {
            r.path.contains("hacivat-ui/src/statusbar.rs:") || r.path.contains("#status-language")
        })
        .map(|r| r.text.to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

/// How many tabs the workspace holds.
pub fn tab_count(vcx: &VisualTestContext) -> usize {
    let ws = workspace(vcx);
    vcx.cx.read(|cx| ws.read(cx).tab_count())
}

/// The title of the focused pane's active tab, as its tab strip draws it.
pub fn tab_title(vcx: &VisualTestContext) -> String {
    let ws = workspace(vcx);
    let title = vcx.cx.read(|cx| {
        let w = ws.read(cx);
        w.active_tab().map(|tab| w.tab_title(tab, cx))
    });
    strip(&title.expect("a tab is active"))
}

/// The layer IDs, bottom to top.
pub fn layer_ids(vcx: &VisualTestContext) -> Vec<&'static str> {
    let ws = workspace(vcx);
    vcx.cx.read(|cx| ws.read(cx).layer_ids())
}

/// `Workspace::context_stack` in the shell's window.
pub fn context_stack(vcx: &VisualTestContext) -> ContextStack {
    let ws = workspace(vcx);
    in_window(vcx, |window, cx| ws.read(cx).context_stack(window, cx))
}

/// The focused element's handle.
pub fn focused(vcx: &VisualTestContext) -> Option<FocusHandle> {
    in_window(vcx, |window, cx| window.focused(cx))
}

/// The fill of the largest quad the last frame painted (the root's background).
pub fn root_bg(vcx: &VisualTestContext) -> Hsla {
    vcx.run_until_parked();
    in_window(vcx, |window, _| {
        window
            .painted_quads()
            .iter()
            .max_by(|a, b| {
                let area = |q: &gpui::Quad| q.bounds.size.width.0 * q.bounds.size.height.0;
                area(a).total_cmp(&area(b))
            })
            .and_then(|q| q.background.as_solid())
            .expect("the root paints a solid background")
    })
}

/// The installed runtime's catalog diagnostics.
pub fn l10n_diags(vcx: &VisualTestContext) -> Vec<LoadDiag> {
    vcx.cx
        .read(|cx| cx.global::<L10nGlobal>().l10n.snapshot().diags.clone())
}

/// The (pane tree, tile layout) pair `workspace.json` persists.
pub fn layout(vcx: &VisualTestContext) -> (PaneTree, TileLayout) {
    let ws = workspace(vcx);
    vcx.cx.read(|cx| ws.read(cx).layout())
}

/// Closes every window.
pub fn close_all_windows(vcx: &VisualTestContext) {
    vcx.cx.update(|cx| {
        for window in cx.windows() {
            let _ = window.update(cx, |_, window, _| window.remove_window());
        }
    });
    vcx.run_until_parked();
}

/// The `ActiveKeymap` global.
pub fn active_keymap(vcx: &VisualTestContext) -> ActiveKeymap {
    vcx.cx.read(|cx| cx.global::<ActiveKeymap>().clone())
}

/// Pushes a toast whose text is message `id`.
pub fn push_toast(vcx: &VisualTestContext, id: &'static str, kind: ToastKind) {
    vcx.cx
        .update(|cx| ToastQueue::push(cx, Toast::new(kind, text_fn_id(id))));
    vcx.run_until_parked();
}

/// Opens a dialog titled `title_id` with body `body_id` and one close button.
pub fn open_dialog_ids(vcx: &VisualTestContext, title_id: &'static str, body_id: &'static str) {
    with_ws(vcx, |w, window, cx| {
        let spec = DialogSpec {
            title: text_fn_id(title_id),
            body: text_fn_id(body_id),
            buttons: vec![DialogButton {
                label: text_fn_id("hui-dialog-close"),
                kind: ButtonKind::Default,
                accesskey: None,
                english_accesskey: None,
            }],
            cancel: 0,
            primary: 0,
            size: DialogSize::Standard,
        };
        w.open_dialog(spec, window, cx);
    });
    vcx.run_until_parked();
}

/// A test tab: a text input in a view with debug selector `test-tab-content`.
struct TestTab {
    input: Entity<TextInput>,
}

impl Render for TestTab {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .debug_selector(|| "test-tab-content".to_string())
            .size_full()
            .child(self.input.clone())
    }
}

/// Registers tab content `kind` (context `Test`) titled by message `title_id`.
pub fn register_test_tab(vcx: &VisualTestContext, kind: &'static str, title_id: &'static str) {
    vcx.cx.update(|cx| {
        cx.default_global::<TabContentRegistry>().register(
            kind,
            TabContent {
                title: Rc::new(move |cx: &App| tr_id(cx, title_id)),
                build: Rc::new(|_: &mut Window, cx: &mut App| {
                    let input = cx.new(|cx| TextInput::new(cx, text_fn_id("hv-explorer-filter")));
                    cx.default_global::<TestInputs>().0.push(input.clone());
                    cx.new(|_| TestTab { input }).into()
                }),
                context: "Test",
            },
        );
    });
}

/// Opens the `test` tab (registering it when needed), focuses its text input and returns the
/// input's focus handle.
pub fn focus_test_input(vcx: &VisualTestContext) -> FocusHandle {
    let registered = vcx.cx.read(|cx| {
        cx.try_global::<TabContentRegistry>()
            .is_some_and(|r| r.get("test").is_some())
    });
    if !registered {
        register_test_tab(vcx, "test", "hv-panel-explorer");
    }
    with_ws(vcx, |w, window, cx| {
        w.open_registered_tab("test", window, cx)
    });
    vcx.run_until_parked();
    let input = vcx.cx.read(|cx| {
        cx.global::<TestInputs>()
            .0
            .last()
            .cloned()
            .expect("the test tab built its input")
    });
    let handle = vcx.cx.read(|cx| input.focus_handle(cx));
    in_window(vcx, |window, cx| window.focus(&handle, cx));
    vcx.run_until_parked();
    handle
}

/// The value of the test input whose focus handle is `handle`.
pub fn input_value(vcx: &VisualTestContext, handle: &FocusHandle) -> String {
    vcx.cx.read(|cx| {
        cx.global::<TestInputs>()
            .0
            .iter()
            .find(|i| i.focus_handle(cx) == *handle)
            .map(|i| i.read(cx).value().to_string())
            .expect("a test input with this handle")
    })
}

/// The test input whose focus handle is `handle` (Task 16: drive its IME composition).
pub fn test_input(vcx: &VisualTestContext, handle: &FocusHandle) -> Entity<TextInput> {
    vcx.cx.read(|cx| {
        cx.global::<TestInputs>()
            .0
            .iter()
            .find(|i| i.focus_handle(cx) == *handle)
            .cloned()
            .expect("a test input with this handle")
    })
}

/// A focusable test layer.
struct TestLayer {
    focus: FocusHandle,
}

impl Render for TestLayer {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().track_focus(&self.focus).size_full()
    }
}

/// Pushes a layer `id` with key context `context`, then focuses it.
pub fn push_test_layer(vcx: &VisualTestContext, id: &'static str, context: &'static str) {
    with_ws(vcx, |w, window, cx| {
        let view = cx.new(|cx| TestLayer {
            focus: cx.focus_handle(),
        });
        let handle = view.read(cx).focus.clone();
        w.push_layer(
            Layer {
                id,
                context,
                view: view.into(),
                restore_focus: None,
                modal: false,
            },
            window,
            cx,
        );
        window.focus(&handle, cx);
    });
    vcx.run_until_parked();
}

/// What a recording key-capture hook saw: (keystroke, context stack).
pub type Seen = Rc<RefCell<Vec<(String, Vec<&'static str>)>>>;

/// Installs a key-capture hook that records every key down and consumes only `consume`
/// (`ctrl-j`, `alt-x`, ...).
pub fn install_recording_hook(vcx: &VisualTestContext, consume: &'static str) -> Seen {
    let seen: Seen = Rc::default();
    let sink = seen.clone();
    vcx.cx.update(|cx| {
        set_key_capture(
            cx,
            Rc::new(move |event, stack, _, _| {
                let m = &event.keystroke.modifiers;
                let mut s = String::new();
                for (on, name) in [
                    (m.control, "ctrl-"),
                    (m.alt, "alt-"),
                    (m.shift, "shift-"),
                    (m.platform, "cmd-"),
                ] {
                    if on {
                        s.push_str(name);
                    }
                }
                s.push_str(&event.keystroke.key);
                let consumed = s == consume;
                sink.borrow_mut().push((s, stack.0.clone()));
                consumed
            }),
        );
    });
    seen
}

/// Writes `content` to `path`, creating its parent directories.
pub fn write(path: impl AsRef<Path>, content: &str) {
    let path = path.as_ref();
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("create dirs");
    std::fs::write(path, content).expect("write file");
}

/// A file of the repository, by its path from the workspace root.
pub fn workspace_file(rel: &str) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    std::fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/// An English-only `L10n` over the embedded catalogs.
pub fn l10n_en() -> L10n {
    let catalogs = Catalogs::embedded();
    let ctx = NegotiationCtx {
        sources: vec![PrefSource::Setting("en".to_string())],
        ..Default::default()
    };
    let neg = negotiate(&ctx, catalogs.locales());
    L10n::new(build_runtime(
        &catalogs,
        &neg,
        &FormatInputs::new(&neg, "UTC"),
        1,
    ))
}

/// One message of an FTL file: its ID and its documented variables `(name, type)`.
struct Documented {
    id: String,
    vars: Vec<(String, String)>,
}

fn documented(ftl: &str) -> Vec<Documented> {
    let mut out = Vec::new();
    let mut vars = Vec::new();
    for line in ftl.lines() {
        if let Some(rest) = line.strip_prefix('#') {
            // `#   $name (Type) - description`
            if let Some(var) = rest.trim().strip_prefix('$')
                && let Some((name, rest)) = var.split_once(' ')
                && let Some(ty) = rest.trim().strip_prefix('(')
                && let Some((ty, _)) = ty.split_once(')')
            {
                vars.push((name.to_string(), ty.to_string()));
            }
            continue;
        }
        let is_message = line
            .split_once(" =")
            .is_some_and(|(id, _)| !id.is_empty() && !id.starts_with([' ', '-', '.']));
        if is_message {
            let id = line
                .split_once(" =")
                .map(|(id, _)| id.to_string())
                .unwrap_or_default();
            out.push(Documented {
                id,
                vars: std::mem::take(&mut vars),
            });
        } else if line.trim().is_empty() {
            vars.clear();
        }
    }
    out
}

/// The message IDs defined in an FTL file's text.
pub fn message_ids_in(ftl: &str) -> Vec<String> {
    documented(ftl).into_iter().map(|d| d.id).collect()
}

fn vars_of(id: &str) -> Vec<(&'static str, String)> {
    let domain = id.split('-').next().unwrap_or_default();
    let file = match domain {
        "hui" => "locales/en/hui.ftl",
        "hv" => "locales/en/hacivat-ui.ftl",
        _ => "locales/en/common.ftl",
    };
    documented(&workspace_file(file))
        .into_iter()
        .find(|d| d.id == id)
        .map(|d| {
            d.vars
                .into_iter()
                .map(|(name, ty)| (&*Box::leak(name.into_boxed_str()), ty))
                .collect()
        })
        .unwrap_or_default()
}

/// Sample `UiArg`s for message `id`: numbers are 2, everything else is "x".
pub fn sample_ui_args(id: &str) -> Vec<(&'static str, UiArg)> {
    vars_of(id)
        .into_iter()
        .map(|(name, ty)| match ty.as_str() {
            "Number" => (name, UiArg::Count(2)),
            _ => (name, UiArg::Text("x".into())),
        })
        .collect()
}

/// The same samples as `ArgValue`s for `L10n::t_dynamic`.
pub fn sample_arg_values(id: &str) -> Vec<(&'static str, ArgValue)> {
    vars_of(id)
        .into_iter()
        .map(|(name, ty)| match ty.as_str() {
            "Number" => (name, ArgValue::Count(2)),
            _ => (name, ArgValue::Text("x".to_string())),
        })
        .collect()
}

// ## m2a-t12-profiles

/// The secret backend under a `shell_with_connections*` shell, for tests to read back.
struct TestSecretBackend(std::sync::Arc<dyn hacivat_secrets::SecretBackend>);

impl Global for TestSecretBackend {}

/// A shell started with `ConnectionsBoot::for_tests` (a memory secret backend) and a
/// `FakeDriver` registry (PostgreSQL, MySQL, MariaDB, SQLite), over a fresh temp root.
pub fn shell_with_connections(
    cx: &mut TestAppContext,
    lang: &str,
) -> &'static mut VisualTestContext {
    shell_with_backend(
        cx,
        lang,
        None,
        std::sync::Arc::new(hacivat_secrets::MemoryBackend::new()),
    )
}

/// Like [`shell_with_connections`], with `connections.toml` holding `text` before boot.
pub fn shell_with_connections_file(
    cx: &mut TestAppContext,
    lang: &str,
    text: &str,
) -> &'static mut VisualTestContext {
    shell_with_backend(
        cx,
        lang,
        Some(text),
        std::sync::Arc::new(hacivat_secrets::MemoryBackend::new()),
    )
}

/// A connections shell whose secret store uses `backend`, with `connections.toml` holding
/// `file` (when given) before boot.
pub fn shell_with_backend(
    cx: &mut TestAppContext,
    lang: &str,
    file: Option<&str>,
    backend: std::sync::Arc<dyn hacivat_secrets::SecretBackend>,
) -> &'static mut VisualTestContext {
    shell_with_connections_prepared(cx, lang, backend, |paths: &LocalPaths| {
        if let Some(text) = file {
            write(paths.connections_toml(), text);
        }
    })
}

/// A connections shell whose secret store uses `backend`; `prepare` sets up the local paths
/// (files in place of `connections.toml`, say) before boot.
pub fn shell_with_connections_prepared(
    cx: &mut TestAppContext,
    lang: &str,
    backend: std::sync::Arc<dyn hacivat_secrets::SecretBackend>,
    prepare: impl FnOnce(&LocalPaths),
) -> &'static mut VisualTestContext {
    use hacivat_driver::DriverRegistry;
    use hacivat_driver::fake::FakeDriver;
    use hacivat_proto::DbmsKind;

    let root = tempfile::tempdir().expect("temp dir");
    let path = root.path().to_path_buf();
    cx.update(|cx| cx.default_global::<TempRoots>().0.push(root));
    let paths = LocalPaths::under(&path);
    paths.ensure_dirs().expect("test dirs");
    prepare(&paths);
    let mut drivers = DriverRegistry::new();
    for kind in [
        DbmsKind::Postgres,
        DbmsKind::MySql,
        DbmsKind::MariaDb,
        DbmsKind::Sqlite,
    ] {
        drivers.register(std::sync::Arc::new(FakeDriver::new(kind)));
    }
    let mut boot = hacivat_app::connections::ConnectionsBoot::for_tests(&paths, drivers);
    boot.secrets = std::sync::Arc::new(hacivat_secrets::SecretStore::new(Some(backend.clone())));
    let mut outcome = hacivat_settings::load(&paths.settings_toml());
    outcome.settings.ui.language = LanguagePref::Tag(lang.to_string());
    let mut startup = Startup::new(paths, outcome, OsLocale::default());
    startup.connections = Some(boot);
    let window = cx.update(|cx| {
        cx.set_global(TestSecretBackend(backend));
        cx.set_global(TextRegistry {
            enabled: true,
            frame: Vec::new(),
        });
        start(cx, startup, None).expect("open the workspace").into()
    });
    finish(cx, window)
}

/// The secret backend of a connections shell.
pub fn secret_backend(
    vcx: &VisualTestContext,
) -> std::sync::Arc<dyn hacivat_secrets::SecretBackend> {
    vcx.cx.read(|cx| cx.global::<TestSecretBackend>().0.clone())
}

/// Waits `d` of real time for core work (Tokio, FakeDriver delays), then runs the GPUI tasks
/// it woke (P22).
pub fn settle(vcx: &VisualTestContext, d: std::time::Duration) {
    vcx.executor().allow_parking();
    std::thread::sleep(d);
    vcx.run_until_parked();
}

/// The app's local paths.
pub fn paths(vcx: &VisualTestContext) -> LocalPaths {
    vcx.cx.read(|cx| cx.global::<AppState>().paths.clone())
}

/// The message IDs of the visible toasts, oldest first: the frame is drawn once without the
/// localization (every message then renders as its ID), then redrawn as it was.
pub fn toast_ids(vcx: &VisualTestContext) -> Vec<String> {
    let l10n = vcx.cx.update(|cx| {
        let l10n = cx.remove_global::<L10nGlobal>();
        cx.refresh_windows();
        l10n
    });
    vcx.run_until_parked();
    let ids = toast_texts(vcx);
    vcx.cx.update(|cx| {
        cx.set_global(l10n);
        cx.refresh_windows();
    });
    vcx.run_until_parked();
    ids
}
