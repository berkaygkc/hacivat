//! Chord parsing and keymap loading from TOML sources (UI-EDIT-13 step 1).

use super::normalize::{named_key, normalize_char};
use super::{
    Binding, BindingSource, KeyPress, KeyboardLayoutId, Keymap, KeymapDiag, KeymapError,
    KeymapPlatform, Mods, NamedKey, NormKey,
};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

fn err(message: impl Into<String>) -> KeymapError {
    KeymapError {
        message: message.into(),
    }
}

/// Parses a chord such as `"ctrl-shift-7"` or `"cmd-k cmd-s"` (presses separated by spaces).
///
/// Modifiers are `ctrl`, `alt` (`option`), `shift` and `cmd` (`super`, `win`, `meta`); the key
/// is a named key (`tab`, `f4`, ...) or one character, which is normalized like a typed key.
pub fn parse_chord(s: &str) -> Result<Vec<KeyPress>, KeymapError> {
    let presses: Vec<KeyPress> = s
        .split_whitespace()
        .map(parse_press)
        .collect::<Result<_, _>>()?;
    if presses.is_empty() {
        return Err(err("empty chord"));
    }
    Ok(presses)
}

fn parse_press(token: &str) -> Result<KeyPress, KeymapError> {
    let mut mods = Mods::default();
    let mut rest = token;
    while let Some((head, tail)) = rest.split_once('-') {
        let slot = match head {
            "ctrl" | "control" => &mut mods.ctrl,
            "alt" | "option" => &mut mods.alt,
            "shift" => &mut mods.shift,
            "cmd" | "super" | "win" | "meta" => &mut mods.cmd,
            "" => break,
            other if tail.is_empty() || other.chars().count() > 1 => {
                return Err(err(format!("unknown modifier `{other}` in `{token}`")));
            }
            _ => break,
        };
        if *slot {
            return Err(err(format!("repeated modifier `{head}` in `{token}`")));
        }
        *slot = true;
        rest = tail;
    }
    if rest.is_empty() {
        return Err(err(format!("missing key in `{token}`")));
    }
    let key = if let Some(named) = named_key(rest) {
        NormKey::Named(named)
    } else {
        let mut it = rest.chars();
        match (it.next(), it.next()) {
            (Some(c), None) => NormKey::Char(normalize_char(c)),
            _ => return Err(err(format!("unknown key `{rest}` in `{token}`"))),
        }
    };
    Ok(KeyPress {
        mods,
        key,
        typed: None,
    })
}

fn key_name(key: NormKey) -> String {
    match key {
        NormKey::Char(c) => c.to_string(),
        NormKey::Named(n) => match n {
            NamedKey::Enter => "enter".into(),
            NamedKey::Escape => "escape".into(),
            NamedKey::Tab => "tab".into(),
            NamedKey::Backspace => "backspace".into(),
            NamedKey::Delete => "delete".into(),
            NamedKey::Insert => "insert".into(),
            NamedKey::Space => "space".into(),
            NamedKey::Up => "up".into(),
            NamedKey::Down => "down".into(),
            NamedKey::Left => "left".into(),
            NamedKey::Right => "right".into(),
            NamedKey::Home => "home".into(),
            NamedKey::End => "end".into(),
            NamedKey::PageUp => "pageup".into(),
            NamedKey::PageDown => "pagedown".into(),
            NamedKey::F(n) => format!("f{n}"),
        },
    }
}

/// The canonical text of a chord, the inverse of [`parse_chord`]: modifiers in the order
/// `ctrl`, `alt`, `shift`, `cmd`, presses separated by one space (`"ctrl-alt-s"`).
pub fn chord_string(chord: &[KeyPress]) -> String {
    let mut out = String::new();
    for (i, p) in chord.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        for (on, name) in [
            (p.mods.ctrl, "ctrl-"),
            (p.mods.alt, "alt-"),
            (p.mods.shift, "shift-"),
            (p.mods.cmd, "cmd-"),
        ] {
            if on {
                out.push_str(name);
            }
        }
        out.push_str(&key_name(p.key));
    }
    out
}

#[derive(Deserialize)]
struct SourceFile {
    #[serde(default)]
    binding: Vec<Entry>,
    /// Layout overlays only: base-layer characters that need Shift on that layout.
    shift_level: Option<String>,
}

#[derive(Deserialize)]
struct Entry {
    chord: toml::Spanned<String>,
    command: String,
    context: String,
    when_layout: Option<String>,
    #[serde(default)]
    during_composition: bool,
    /// `"macos" | "windows" | "linux"`; absent means every platform.
    platforms: Option<Vec<String>>,
}

fn line_of(text: &str, offset: usize) -> u32 {
    let end = offset.min(text.len());
    let newlines = text.as_bytes()[..end]
        .iter()
        .filter(|b| **b == b'\n')
        .count();
    u32::try_from(newlines + 1).unwrap_or(u32::MAX)
}

fn platform_name(p: KeymapPlatform) -> &'static str {
    match p {
        KeymapPlatform::MacOs => "macos",
        KeymapPlatform::Windows => "windows",
        KeymapPlatform::Linux => "linux",
    }
}

type Key = (Vec<KeyPress>, String, Option<KeyboardLayoutId>);

fn key_of(b: &Binding) -> Key {
    (b.chord.clone(), b.context.clone(), b.when_layout.clone())
}

struct Loaded {
    bindings: Vec<Binding>,
    shift_level: Option<Vec<char>>,
}

/// Parses one source file. A file that does not parse yields one `Parse` diag and no bindings;
/// a bad entry yields one `Parse` diag and the other entries still load.
fn load_source(
    text: &str,
    source: BindingSource,
    platform: KeymapPlatform,
    forced_layout: Option<&KeyboardLayoutId>,
    diags: &mut Vec<KeymapDiag>,
) -> Loaded {
    let mut loaded = Loaded {
        bindings: Vec::new(),
        shift_level: None,
    };
    let file: SourceFile = match toml::from_str(text) {
        Ok(f) => f,
        Err(e) => {
            diags.push(KeymapDiag::Parse {
                source,
                line: e.span().map_or(0, |s| line_of(text, s.start)),
                message: e.message().to_string(),
            });
            return loaded;
        }
    };
    loaded.shift_level = file.shift_level.map(|s| s.chars().collect());
    let mut seen: HashSet<Key> = HashSet::new();
    for entry in file.binding {
        let line = line_of(text, entry.chord.span().start);
        if let Some(platforms) = &entry.platforms {
            let known = ["macos", "windows", "linux"];
            if let Some(bad) = platforms.iter().find(|p| !known.contains(&p.as_str())) {
                diags.push(KeymapDiag::Parse {
                    source,
                    line,
                    message: format!("unknown platform `{bad}`"),
                });
                continue;
            }
            if !platforms.iter().any(|p| p == platform_name(platform)) {
                continue;
            }
        }
        let chord = match parse_chord(entry.chord.get_ref()) {
            Ok(c) => c,
            Err(e) => {
                diags.push(KeymapDiag::Parse {
                    source,
                    line,
                    message: e.message,
                });
                continue;
            }
        };
        let binding = Binding {
            chord,
            command: entry.command,
            context: entry.context,
            when_layout: forced_layout
                .cloned()
                .or_else(|| entry.when_layout.map(KeyboardLayoutId)),
            during_composition: entry.during_composition,
            source,
        };
        if !seen.insert(key_of(&binding)) {
            diags.push(KeymapDiag::DuplicateChord {
                chord: chord_string(&binding.chord),
                context: binding.context,
            });
            continue;
        }
        loaded.bindings.push(binding);
    }
    loaded
}

impl Keymap {
    /// Loads a keymap from TOML sources, lowest priority first: `default`, `preset`, the layout
    /// overlays, then the `user` file.
    ///
    /// Each file holds `[[binding]]` tables (`chord`, `command`, `context`, and optionally
    /// `when_layout`, `during_composition` and `platforms`). An overlay file also carries
    /// `shift_level` and its bindings apply only on its layout.
    ///
    /// A higher source replaces lower bindings on the same chord, context and layout. A preset
    /// or user binding also replaces the lower bindings of the same command in the same
    /// context, so rebinding a command never leaves its old chord behind. Duplicate chords
    /// inside one source are reported and the first one wins.
    pub fn from_sources(
        platform: KeymapPlatform,
        default: &str,
        preset: &str,
        layout_overlays: &[(&KeyboardLayoutId, &str)],
        user: Option<&str>,
    ) -> (Keymap, Vec<KeymapDiag>) {
        let mut diags = Vec::new();
        let mut bindings: Vec<Binding> = Vec::new();
        let mut shift_levels: HashMap<KeyboardLayoutId, Vec<char>> = HashMap::new();

        let mut layers: Vec<(BindingSource, Option<&KeyboardLayoutId>, &str)> = vec![
            (BindingSource::Default, None, default),
            (BindingSource::Preset, None, preset),
        ];
        layers.extend(
            layout_overlays
                .iter()
                .map(|(id, text)| (BindingSource::Layout, Some(*id), *text)),
        );
        if let Some(user) = user {
            layers.push((BindingSource::User, None, user));
        }

        for (source, layout, text) in layers {
            let loaded = load_source(text, source, platform, layout, &mut diags);
            if let (Some(id), Some(levels)) = (layout, loaded.shift_level) {
                shift_levels.insert(id.clone(), levels);
            }
            let replaces_command = matches!(source, BindingSource::Preset | BindingSource::User);
            let incoming_keys: HashSet<Key> = loaded.bindings.iter().map(key_of).collect();
            let replaced_commands: HashSet<(&str, &str)> = if replaces_command {
                loaded
                    .bindings
                    .iter()
                    .filter(|b| b.when_layout.is_none())
                    .map(|b| (b.command.as_str(), b.context.as_str()))
                    .collect()
            } else {
                HashSet::new()
            };
            bindings.retain(|old| {
                if incoming_keys.contains(&key_of(old)) {
                    return false;
                }
                let lower_than_user = source == BindingSource::User
                    || matches!(old.source, BindingSource::Default | BindingSource::Preset);
                !(lower_than_user
                    && replaced_commands.contains(&(old.command.as_str(), old.context.as_str())))
            });
            bindings.extend(loaded.bindings);
        }

        let mut index: HashMap<KeyPress, Vec<usize>> = HashMap::new();
        for (i, b) in bindings.iter().enumerate() {
            index.entry(b.chord[0]).or_default().push(i);
        }
        (
            Keymap {
                platform,
                bindings,
                index,
                shift_levels,
            },
            diags,
        )
    }
}
