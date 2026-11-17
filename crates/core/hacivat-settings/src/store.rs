//! Reading and writing `settings.toml`.

use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use toml_edit::{DocumentMut, Item, Table, TableLike, Value, value};
use unic_langid::LanguageIdentifier;

use crate::fsio;
use crate::schema::{
    CaretBlinkPref, EditorSettings, FindTolerancePref, KeymapPreset, LanguagePref,
    SETTINGS_VERSION, Settings, TabSize, TextRenderingPref, ThemePref, TimeZonePref,
};
// ## m3b-t01c-core-seams
use crate::schema::{ExecSettings, HistorySettings, ValueLimit};

/// A problem found while loading; Task 13 shows each code as a toast.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsDiag {
    pub code: &'static str,
    pub key: String,
}

/// The result of [`load`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadOutcome {
    pub settings: Settings,
    pub diags: Vec<SettingsDiag>,
    pub quarantined: Option<PathBuf>,
}

/// Loads `path`, validating every key on its own so one bad value never resets the others.
/// A missing file gives the defaults; an unparsable one is set aside as `*.corrupt-<ts>`.
pub fn load(path: &Path) -> LoadOutcome {
    let mut out = LoadOutcome {
        settings: Settings::default(),
        diags: Vec::new(),
        quarantined: None,
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        if path.exists() {
            out.quarantined = fsio::quarantine(path, "corrupt", SystemTime::now()).ok();
        }
        return out;
    };
    let Ok(doc) = text.parse::<DocumentMut>() else {
        out.quarantined = fsio::quarantine(path, "corrupt", SystemTime::now()).ok();
        return out;
    };
    let root = doc.as_table();
    let s = &mut out.settings;
    let d = &mut out.diags;

    match root.get("version") {
        None => {}
        Some(item) => match item.as_integer().and_then(|v| u32::try_from(v).ok()) {
            Some(v) if v >= 1 => {
                s.version = v;
                if v > SETTINGS_VERSION {
                    d.push(diag("settings-newer-version", "version"));
                }
            }
            _ => d.push(diag("settings-invalid-value", "version")),
        },
    }
    if let Some(ui) = section(root, "ui", d) {
        s.ui.language = read_key(ui, "ui", "language", d);
        s.ui.regional_format = read_key(ui, "ui", "regional_format", d);
        s.ui.time_zone = read_key(ui, "ui", "time_zone", d);
        s.ui.theme = read_key(ui, "ui", "theme", d);
        s.ui.text_rendering = read_key(ui, "ui", "text_rendering", d);
    }
    if let Some(km) = section(root, "keymap", d) {
        s.keymap.preset = read_key(km, "keymap", "preset", d);
    }
    // ## m3a-t01b-seams
    if let Some(ed) = section(root, "editor", d) {
        let def = EditorSettings::default();
        s.editor.tab_size = read_key_or(ed, "editor", "tab_size", def.tab_size, d);
        s.editor.insert_spaces = read_key_or(ed, "editor", "insert_spaces", def.insert_spaces, d);
        s.editor.copy_line_without_selection = read_key_or(
            ed,
            "editor",
            "copy_line_without_selection",
            def.copy_line_without_selection,
            d,
        );
        s.editor.caret_blink = read_key_or(ed, "editor", "caret_blink", def.caret_blink, d);
        // ## m3c-t33-find
        s.editor.find_turkish_tolerant = read_key_or(
            ed,
            "editor",
            "find_turkish_tolerant",
            def.find_turkish_tolerant,
            d,
        );
    }
    // ## m3b-t01c-core-seams (P-M3b-9): a missing table means its defaults.
    if let Some(ex) = section(root, "exec", d) {
        let def = ExecSettings::default();
        s.exec.confirm_unsafe_where = read_key_or(
            ex,
            "exec",
            "confirm_unsafe_where",
            def.confirm_unsafe_where,
            d,
        );
        s.exec.error_policy_ask =
            read_key_or(ex, "exec", "error_policy_ask", def.error_policy_ask, d);
    }
    if let Some(rs) = section(root, "results", d) {
        s.results.max_value_bytes = read_value_limit(rs, d);
    }
    if let Some(hi) = section(root, "history", d) {
        let def = HistorySettings::default();
        s.history.enabled = read_key_or(hi, "history", "enabled", def.enabled, d);
    }
    if let Some(gr) = section(root, "grid", d) {
        s.grid.raw_display = read_key(gr, "grid", "raw_display", d);
    }
    out
}

// ## m3b-t01c-core-seams (P-M3b-9)
/// `results.max_value_bytes`: an integer clamped to `ValueLimit`'s range, with a diagnostic
/// when it was out of it (the value still counts, at the nearest bound); anything else is the
/// default, with a diagnostic.
fn read_value_limit(table: &dyn TableLike, diags: &mut Vec<SettingsDiag>) -> ValueLimit {
    const KEY: &str = "results.max_value_bytes";
    match table.get("max_value_bytes") {
        None => ValueLimit::default(),
        Some(item) => match item.as_integer() {
            Some(n) => {
                let limit = ValueLimit::clamped(n);
                if i64::from(limit.get()) != n {
                    diags.push(diag("settings-invalid-value", KEY));
                }
                limit
            }
            None => {
                diags.push(diag("settings-invalid-value", KEY));
                ValueLimit::default()
            }
        },
    }
}

/// Merges `new` into the file at `path` with `toml_edit`: only keys whose value changed are
/// written, comments, unknown keys and a newer `version` stay, then the file is replaced
/// atomically.
pub fn save_changes(path: &Path, new: &Settings) -> io::Result<()> {
    let mut doc = match std::fs::read_to_string(path) {
        Ok(t) => t
            .parse::<DocumentMut>()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => DocumentMut::new(),
        Err(e) => return Err(e),
    };
    if !doc.contains_key("version") {
        doc["version"] = value(i64::from(new.version));
    }
    write_key(&mut doc, "ui", "language", &new.ui.language);
    write_key(&mut doc, "ui", "regional_format", &new.ui.regional_format);
    write_key(&mut doc, "ui", "time_zone", &new.ui.time_zone);
    write_key(&mut doc, "ui", "theme", &new.ui.theme);
    write_key(&mut doc, "ui", "text_rendering", &new.ui.text_rendering);
    write_key(&mut doc, "keymap", "preset", &new.keymap.preset);
    // ## m3a-t01b-seams
    let def = EditorSettings::default();
    let ed = &new.editor;
    write_key_or(&mut doc, "editor", "tab_size", &ed.tab_size, &def.tab_size);
    write_key_or(
        &mut doc,
        "editor",
        "insert_spaces",
        &ed.insert_spaces,
        &def.insert_spaces,
    );
    write_key_or(
        &mut doc,
        "editor",
        "copy_line_without_selection",
        &ed.copy_line_without_selection,
        &def.copy_line_without_selection,
    );
    write_key_or(
        &mut doc,
        "editor",
        "caret_blink",
        &ed.caret_blink,
        &def.caret_blink,
    );
    // ## m3c-t33-find
    write_key_or(
        &mut doc,
        "editor",
        "find_turkish_tolerant",
        &ed.find_turkish_tolerant,
        &def.find_turkish_tolerant,
    );
    // ## m3b-t01c-core-seams (P-M3b-9): each key written only when it changed; a key at its
    // default is never added.
    let def = ExecSettings::default();
    write_key_or(
        &mut doc,
        "exec",
        "confirm_unsafe_where",
        &new.exec.confirm_unsafe_where,
        &def.confirm_unsafe_where,
    );
    write_key_or(
        &mut doc,
        "exec",
        "error_policy_ask",
        &new.exec.error_policy_ask,
        &def.error_policy_ask,
    );
    write_key(
        &mut doc,
        "results",
        "max_value_bytes",
        &new.results.max_value_bytes,
    );
    write_key_or(
        &mut doc,
        "history",
        "enabled",
        &new.history.enabled,
        &HistorySettings::default().enabled,
    );
    write_key(&mut doc, "grid", "raw_display", &new.grid.raw_display);
    fsio::atomic_write(path, doc.to_string().as_bytes())
}

fn diag(code: &'static str, key: &str) -> SettingsDiag {
    SettingsDiag {
        code,
        key: key.to_owned(),
    }
}

/// The named table, or `None` (with a diagnostic when it exists but is not a table).
fn section<'a>(
    root: &'a Table,
    name: &str,
    diags: &mut Vec<SettingsDiag>,
) -> Option<&'a dyn TableLike> {
    let item = root.get(name)?;
    let table = item.as_table_like();
    if table.is_none() {
        diags.push(diag("settings-invalid-value", name));
    }
    table
}

fn read_key<T: Pref>(
    table: &dyn TableLike,
    section: &str,
    key: &str,
    diags: &mut Vec<SettingsDiag>,
) -> T {
    read_key_or(table, section, key, T::default(), diags)
}

/// [`read_key`] with the key's own default (## m3a-t01b-seams: a `bool` key that defaults to
/// `true`).
fn read_key_or<T: Pref>(
    table: &dyn TableLike,
    section: &str,
    key: &str,
    default: T,
    diags: &mut Vec<SettingsDiag>,
) -> T {
    match table.get(key) {
        None => default,
        Some(item) => T::from_item(item).unwrap_or_else(|| {
            diags.push(diag("settings-invalid-value", &format!("{section}.{key}")));
            default
        }),
    }
}

fn write_key<T: Pref>(doc: &mut DocumentMut, section: &str, key: &str, new: &T) {
    write_key_or(doc, section, key, new, &T::default());
}

/// [`write_key`] with the key's own default (## m3a-t01b-seams).
fn write_key_or<T: Pref>(doc: &mut DocumentMut, section: &str, key: &str, new: &T, default: &T) {
    let existing = doc
        .get(section)
        .and_then(Item::as_table_like)
        .and_then(|t| t.get(key));
    match existing {
        // Unchanged: the file already means this value (even if spelled differently).
        Some(item) if T::from_item(item).as_ref() == Some(new) => return,
        None if new == default => return,
        _ => {}
    }
    let mut v = new.to_value();
    if let Some(old) = existing.and_then(Item::as_value) {
        *v.decor_mut() = old.decor().clone();
    }
    if !doc.get(section).is_some_and(Item::is_table_like) {
        doc[section] = Item::Table(Table::new());
    }
    if let Some(t) = doc[section].as_table_like_mut() {
        t.insert(key, Item::Value(v));
    }
}

/// A setting that is stored as a TOML string (an integer or a boolean overrides `from_item` and
/// `to_value`, ## m3a-t01b-seams).
trait Pref: Sized + PartialEq + Default {
    fn parse(text: &str) -> Option<Self>;
    fn as_text(&self) -> String;
    fn from_item(item: &Item) -> Option<Self> {
        item.as_str().and_then(Self::parse)
    }
    fn to_value(&self) -> Value {
        Value::from(self.as_text())
    }
}

impl Pref for LanguagePref {
    fn parse(text: &str) -> Option<Self> {
        if text == "system" {
            return Some(Self::System);
        }
        text.parse::<LanguageIdentifier>()
            .ok()
            .map(|l| Self::Tag(l.to_string()))
    }
    fn as_text(&self) -> String {
        match self {
            Self::System => "system".to_owned(),
            Self::Tag(t) => t.clone(),
        }
    }
}

impl Pref for TimeZonePref {
    /// Shape check only (this crate carries no tz database): `Area/Location` style names.
    fn parse(text: &str) -> Option<Self> {
        if text == "system" {
            return Some(Self::System);
        }
        let ok = !text.is_empty()
            && text.len() <= 64
            && !text.starts_with('/')
            && !text.ends_with('/')
            && !text.contains("//")
            && !text.contains("..")
            && text
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-' | '+'));
        ok.then(|| Self::Iana(text.to_owned()))
    }
    fn as_text(&self) -> String {
        match self {
            Self::System => "system".to_owned(),
            Self::Iana(z) => z.clone(),
        }
    }
}

impl Pref for ThemePref {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "dark" => Some(Self::Dark),
            "light" => Some(Self::Light),
            "system" => Some(Self::System),
            _ => None,
        }
    }
    fn as_text(&self) -> String {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
            Self::System => "system",
        }
        .to_owned()
    }
}

impl Pref for TextRenderingPref {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "crisp" => Some(Self::Crisp),
            "standard" => Some(Self::Standard),
            _ => None,
        }
    }
    fn as_text(&self) -> String {
        match self {
            Self::Crisp => "crisp",
            Self::Standard => "standard",
        }
        .to_owned()
    }
}

impl Pref for KeymapPreset {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "hacivat" => Some(Self::Hacivat),
            "datagrip" => Some(Self::Datagrip),
            _ => None,
        }
    }
    fn as_text(&self) -> String {
        match self {
            Self::Hacivat => "hacivat",
            Self::Datagrip => "datagrip",
        }
        .to_owned()
    }
}

// ## m3a-t01b-seams

/// `editor.tab_size`: a TOML integer, 1 to 16.
impl Pref for TabSize {
    fn parse(text: &str) -> Option<Self> {
        text.parse::<u8>().ok().and_then(TabSize::new)
    }
    fn as_text(&self) -> String {
        self.get().to_string()
    }
    fn from_item(item: &Item) -> Option<Self> {
        item.as_integer()
            .and_then(|v| u8::try_from(v).ok())
            .and_then(TabSize::new)
    }
    fn to_value(&self) -> Value {
        Value::from(i64::from(self.get()))
    }
}

/// A TOML boolean (`editor.insert_spaces`, `editor.copy_line_without_selection`).
impl Pref for bool {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        }
    }
    fn as_text(&self) -> String {
        self.to_string()
    }
    fn from_item(item: &Item) -> Option<Self> {
        item.as_bool()
    }
    fn to_value(&self) -> Value {
        Value::from(*self)
    }
}

// ## m3b-t01c-core-seams
/// `results.max_value_bytes`: a TOML integer in `ValueLimit`'s range (loading clamps one out
/// of it, `read_value_limit`).
impl Pref for ValueLimit {
    fn parse(text: &str) -> Option<Self> {
        text.parse::<u32>().ok().and_then(ValueLimit::new)
    }
    fn as_text(&self) -> String {
        self.get().to_string()
    }
    fn from_item(item: &Item) -> Option<Self> {
        item.as_integer()
            .and_then(|v| u32::try_from(v).ok())
            .and_then(ValueLimit::new)
    }
    fn to_value(&self) -> Value {
        Value::from(i64::from(self.get()))
    }
}

impl Pref for CaretBlinkPref {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "system" => Some(Self::System),
            "off" => Some(Self::Off),
            _ => None,
        }
    }
    fn as_text(&self) -> String {
        match self {
            Self::System => "system",
            Self::Off => "off",
        }
        .to_owned()
    }
}

// ## m3c-t33-find
/// `editor.find_turkish_tolerant`: `"auto"`, `"on"` or `"off"`.
impl Pref for FindTolerancePref {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "auto" => Some(Self::Auto),
            "on" => Some(Self::On),
            "off" => Some(Self::Off),
            _ => None,
        }
    }
    fn as_text(&self) -> String {
        match self {
            Self::Auto => "auto",
            Self::On => "on",
            Self::Off => "off",
        }
        .to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{LanguagePref, ThemePref};
    use crate::{LocalPaths, workspace_state};

    fn tmp() -> PathBuf {
        tempfile::tempdir().unwrap().keep()
    }
    fn write_tmp_named(name: &str, content: &str) -> PathBuf {
        let p = tmp().join(name);
        std::fs::write(&p, content).unwrap();
        p
    }
    fn write_tmp(content: &str) -> PathBuf {
        write_tmp_named("settings.toml", content)
    }
    fn sibling_with_prefix(p: &Path, prefix: &str) -> bool {
        std::fs::read_dir(p.parent().unwrap())
            .unwrap()
            .any(|e| e.unwrap().file_name().to_string_lossy().starts_with(prefix))
    }
    #[cfg(unix)]
    fn mode(p: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn defaults_when_file_missing() {
        let o = load(&tmp().join("settings.toml"));
        assert_eq!(o.settings, Settings::default());
        assert!(o.diags.is_empty());
    }

    #[test]
    fn round_trip_keeps_unknown_keys_and_comments() {
        let p = write_tmp("# mine\nversion = 1\n[ui]\ntheme = \"light\"\nfuture_key = 3\n");
        let mut s = load(&p).settings;
        s.ui.theme = ThemePref::Dark;
        save_changes(&p, &s).unwrap();
        let t = std::fs::read_to_string(&p).unwrap();
        assert!(
            t.contains("# mine") && t.contains("future_key = 3") && t.contains("theme = \"dark\"")
        );
    }

    #[test]
    fn invalid_language_tag_falls_back_to_system() {
        let o = load(&write_tmp("[ui]\nlanguage = \"klingon!!\"\n"));
        assert_eq!(o.settings.ui.language, LanguagePref::System);
        assert_eq!(
            o.diags,
            vec![SettingsDiag {
                code: "settings-invalid-value",
                key: "ui.language".into()
            }]
        );
    }

    #[test]
    fn wrong_type_uses_default() {
        let o = load(&write_tmp("[ui]\ntheme = 3\nlanguage = \"tr\"\n"));
        assert_eq!(o.settings.ui.theme, ThemePref::Dark);
        assert_eq!(o.settings.ui.language, LanguagePref::Tag("tr".into()));
        assert_eq!(
            o.diags,
            vec![SettingsDiag {
                code: "settings-invalid-value",
                key: "ui.theme".into()
            }]
        );
    }

    #[test]
    fn unparsable_file_is_quarantined_not_overwritten() {
        let p = write_tmp("[[[");
        let o = load(&p);
        assert!(
            o.quarantined
                .unwrap()
                .to_string_lossy()
                .contains("settings.toml.corrupt-")
        );
        assert!(!p.exists());
        assert_eq!(o.settings, Settings::default());
    }

    #[cfg(unix)]
    #[test]
    fn permissions_are_0700_and_0600() {
        let r = tmp();
        let p = LocalPaths::under(&r);
        p.ensure_dirs().unwrap();
        assert_eq!(mode(&p.config), 0o700);
        assert_eq!(mode(&p.data), 0o700);
        assert_eq!(mode(&p.cache), 0o700);
        save_changes(&p.settings_toml(), &Settings::default()).unwrap();
        assert_eq!(mode(&p.settings_toml()), 0o600);
    }

    #[test]
    fn local_paths_follow_spec_table() {
        let p = LocalPaths::under(Path::new("/r"));
        let u = "6f1c…";
        assert_eq!(p.settings_toml(), Path::new("/r/config/settings.toml"));
        assert_eq!(p.history_db(), Path::new("/r/data/history.sqlite"));
        assert_eq!(
            p.catalog_db(u, 1),
            Path::new("/r/data/catalog").join(format!("{u}.v1.sqlite"))
        );
        assert_eq!(p.workspace_json(), Path::new("/r/data/workspace.json"));
        assert_eq!(p.logs_dir(), Path::new("/r/cache/logs"));
        assert_eq!(p.locales_overlay_dir(), Path::new("/r/config/locales"));
        assert_eq!(
            p.ipc_endpoint(Some("S-1-5-21-1")).to_string_lossy(),
            if cfg!(windows) {
                r"\\.\pipe\hacivat-S-1-5-21-1"
            } else {
                "/r/runtime/hacivat/broker.sock"
            }
        );
    }

    #[test]
    fn future_workspace_version_is_set_aside() {
        let p = write_tmp_named("workspace.json", r#"{"version":2,"panes":{}}"#);
        assert!(workspace_state::load(&p, 1).is_none());
        assert!(sibling_with_prefix(&p, "workspace.json.newer-"));
        assert!(!p.exists());
    }

    #[test]
    fn newer_settings_version_loads_known_keys_and_keeps_the_file() {
        let p = write_tmp("version = 2\n[ui]\ntheme = \"light\"\nnew_thing = true\n");
        let o = load(&p);
        assert_eq!(o.settings.ui.theme, ThemePref::Light);
        assert_eq!(
            o.diags,
            vec![SettingsDiag {
                code: "settings-newer-version",
                key: "version".into()
            }]
        );
        assert!(o.quarantined.is_none());
        let mut s = o.settings;
        s.ui.theme = ThemePref::Dark;
        save_changes(&p, &s).unwrap();
        let t = std::fs::read_to_string(&p).unwrap();
        assert!(
            t.contains("version = 2")
                && t.contains("new_thing = true")
                && t.contains("theme = \"dark\"")
        );
    }

    #[test]
    fn wrong_typed_version_uses_default_and_keeps_the_file() {
        let p = write_tmp("version = \"one\"\n[ui]\ntheme = \"light\"\n");
        let o = load(&p);
        assert_eq!(o.settings.version, 1);
        assert_eq!(o.settings.ui.theme, ThemePref::Light);
        assert_eq!(
            o.diags,
            vec![SettingsDiag {
                code: "settings-invalid-value",
                key: "version".into()
            }]
        );
        assert!(o.quarantined.is_none() && p.exists());
    }

    #[test]
    fn unparsable_workspace_json_is_set_aside_as_corrupt() {
        let p = write_tmp_named("workspace.json", "{not json");
        assert!(workspace_state::load(&p, 1).is_none());
        assert!(sibling_with_prefix(&p, "workspace.json.corrupt-"));
        assert!(!p.exists());
    }

    #[test]
    fn misspelled_key_is_kept_as_unknown() {
        let p = write_tmp("[ui]\nthem = \"light\"\n");
        let o = load(&p);
        assert_eq!(o.settings.ui.theme, ThemePref::Dark);
        assert!(o.diags.is_empty());
        let mut s = o.settings;
        s.ui.theme = ThemePref::Light;
        save_changes(&p, &s).unwrap();
        let t = std::fs::read_to_string(&p).unwrap();
        assert!(t.contains("them = \"light\"") && t.contains("theme = \"light\""));
    }

    // ## m3a-t01b-seams
    #[test]
    fn editor_keys_load_and_bad_ones_fall_back_alone() {
        let p = write_tmp(
            "[editor]\ntab_size = 2\ninsert_spaces = false\ncopy_line_without_selection = \"yes\"\ncaret_blink = \"off\"\n",
        );
        let o = load(&p);
        let e = o.settings.editor;
        assert_eq!(e.tab_size, TabSize::new(2).unwrap());
        assert!(!e.insert_spaces);
        assert!(
            e.copy_line_without_selection,
            "the bad value keeps the default"
        );
        assert_eq!(e.caret_blink, CaretBlinkPref::Off);
        assert_eq!(
            o.diags,
            vec![SettingsDiag {
                code: "settings-invalid-value",
                key: "editor.copy_line_without_selection".into()
            }]
        );
        let o = load(&write_tmp("[editor]\ntab_size = 40\n"));
        assert_eq!(o.settings.editor.tab_size, TabSize::default());
        assert_eq!(o.diags.len(), 1);
    }

    #[test]
    fn editor_keys_round_trip_and_defaults_write_nothing() {
        let p = write_tmp("[ui]\ntheme = \"light\"\n");
        let mut s = load(&p).settings;
        save_changes(&p, &s).unwrap();
        assert!(!std::fs::read_to_string(&p).unwrap().contains("[editor]"));
        s.editor.tab_size = TabSize::new(8).unwrap();
        s.editor.insert_spaces = false;
        s.editor.caret_blink = CaretBlinkPref::Off;
        save_changes(&p, &s).unwrap();
        let t = std::fs::read_to_string(&p).unwrap();
        assert!(
            t.contains("tab_size = 8") && t.contains("insert_spaces = false"),
            "{t}"
        );
        assert!(!t.contains("copy_line_without_selection"), "{t}");
        assert_eq!(load(&p).settings.editor, s.editor);
    }

    // ## m3c-t33-find
    #[test]
    fn find_tolerance_round_trips_and_auto_writes_nothing() {
        let p = write_tmp("[editor]\nfind_turkish_tolerant = \"sometimes\"\n");
        let o = load(&p);
        assert_eq!(
            o.settings.editor.find_turkish_tolerant,
            FindTolerancePref::Auto
        );
        assert_eq!(o.diags.len(), 1, "{:?}", o.diags);
        let p = write_tmp("[ui]\ntheme = \"light\"\n");
        let mut s = load(&p).settings;
        save_changes(&p, &s).unwrap();
        assert!(!std::fs::read_to_string(&p).unwrap().contains("find_"));
        s.editor.find_turkish_tolerant = FindTolerancePref::Off;
        save_changes(&p, &s).unwrap();
        let t = std::fs::read_to_string(&p).unwrap();
        assert!(t.contains("find_turkish_tolerant = \"off\""), "{t}");
        assert_eq!(
            load(&p).settings.editor.find_turkish_tolerant,
            FindTolerancePref::Off
        );
    }

    // ## m3b-t01c-core-seams (P-M3b-9)

    /// A settings file as an M3a build writes it.
    const M3A_SETTINGS_TEXT: &str = "version = 1\n[ui]\ntheme = \"light\"\nlanguage = \"tr\"\n[keymap]\npreset = \"datagrip\"\n[editor]\ntab_size = 2\n";

    #[test]
    fn m3a_settings_file_loads_with_new_defaults_and_no_diagnostic() {
        let o = load(&write_tmp(M3A_SETTINGS_TEXT));
        assert!(o.diags.is_empty(), "{:?}", o.diags);
        assert!(o.settings.exec.confirm_unsafe_where);
        assert!(o.settings.exec.error_policy_ask);
        assert_eq!(o.settings.results.max_value_bytes.get(), 1 << 20);
        assert!(o.settings.history.enabled);
        assert!(!o.settings.grid.raw_display);
        assert_eq!(o.settings.ui.theme, ThemePref::Light);
    }

    #[test]
    fn value_limit_clamps_with_a_diagnostic() {
        let o = load(&write_tmp("[results]\nmax_value_bytes = 1\n"));
        assert_eq!(o.settings.results.max_value_bytes.get(), 65_536);
        assert_eq!(o.diags.len(), 1);
        assert_eq!(o.diags[0].key, "results.max_value_bytes");
        let o = load(&write_tmp("[results]\nmax_value_bytes = 1000000000000\n"));
        assert_eq!(o.settings.results.max_value_bytes.get(), 67_108_864);
        assert_eq!(o.diags.len(), 1);
        let o = load(&write_tmp("[results]\nmax_value_bytes = \"big\"\n"));
        assert_eq!(o.settings.results.max_value_bytes.get(), 1 << 20);
        assert_eq!(o.diags.len(), 1);
        let o = load(&write_tmp("[results]\nmax_value_bytes = 2097152\n"));
        assert_eq!(o.settings.results.max_value_bytes.get(), 2 << 20);
        assert!(o.diags.is_empty());
    }

    #[test]
    fn new_tables_round_trip_through_save() {
        let p = write_tmp(M3A_SETTINGS_TEXT);
        let mut s = load(&p).settings;
        // Defaults write nothing: an M3a file stays as it was.
        save_changes(&p, &s).unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), M3A_SETTINGS_TEXT);
        s.exec.confirm_unsafe_where = false;
        s.exec.error_policy_ask = false;
        s.results.max_value_bytes = ValueLimit::new(8 << 20).unwrap();
        s.history.enabled = false;
        s.grid.raw_display = true;
        save_changes(&p, &s).unwrap();
        let t = std::fs::read_to_string(&p).unwrap();
        for line in [
            "confirm_unsafe_where = false",
            "error_policy_ask = false",
            "max_value_bytes = 8388608",
            "enabled = false",
            "raw_display = true",
        ] {
            assert!(t.contains(line), "{line} in {t}");
        }
        let back = load(&p);
        assert!(back.diags.is_empty());
        assert_eq!(back.settings, s);
        // A wrong type keeps the default alone, beside the good keys.
        let o = load(&write_tmp(
            "[exec]\nconfirm_unsafe_where = \"no\"\nerror_policy_ask = false\n",
        ));
        assert!(o.settings.exec.confirm_unsafe_where);
        assert!(!o.settings.exec.error_policy_ask);
        assert_eq!(o.diags[0].key, "exec.confirm_unsafe_where");
    }
}
