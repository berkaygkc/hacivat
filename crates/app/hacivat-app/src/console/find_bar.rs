//! The editor's find bar (UI-EDIT-22 without replace, R-M3-10, D-074; M3c Task 33): the query
//! field on `TextInput`, Match case, Regex, Words, Turkish-tolerant, "k of N", and the
//! `regex`-backed `FindMatcher` the editor steps with (Task 37's grid find reuses the builder:
//! [`FindOpts`], [`build_matcher`], [`RegexMatcher`]).
//!
//! ## m3b-t01b-seams: the slot (Task 1b): [`render`] answers the console's bar while it is
//! open, `None` otherwise (so every M3a/M3b shot holds).
//!
//! ## m3c-t33-find
//!
//! **The matcher is built here, over the original text** (R-M3-10): a literal query is
//! escaped; with Turkish-tolerant on, each letter becomes a class of its variants (`i` →
//! `[iıìíîïĩīĭįIİÌÍÎÏĨĪĬĮ]`, the Turkish dotted and dotless `i` and the accented forms, each
//! followed by `[\x{300}-\x{36F}]*` so a decomposed accent is ignored too; a literal longer
//! than `TOLERANT_MAX` is matched as written); Match case keeps the classes to the letter's own
//! case (lower `i` finds `ı`, upper `I` finds `İ`). Exact mode never folds a
//! Turkish letter: `(?i)` is the `regex` crate's simple case folding, where `i`/`I` do not
//! reach `İ`/`ı`. A regular expression is the user's own, its literal letters rewritten the
//! same way when Turkish-tolerant is on (escapes, classes' ranges, group names and flags are
//! left alone; a rewrite that no longer compiles falls back to the pattern as written). Every
//! pattern is multi-line with CRLF line ends (`(?mR)`). Whole words are checked after the match
//! with SQL word characters (letters, digits, `_`, `$`; ALG UI-EDIT-22 step 2, no
//! look-behind in `regex`). Empty matches are passed over. The editor (a UI crate) takes the
//! compiled matcher only.
//!
//! **The bar** (NOTES surface 16, M3c-9) is a 32 px row under the console toolbar, never over
//! the code: the find field (search glyph, mono query, four 20 × 18 toggles inside its well),
//! the count, previous / next, Select all occurrences, close. While it is open the editor has a
//! query; the count runs on the editor's background executor ("3 of 1,240+" with the static
//! amber marker meanwhile). An invalid pattern searches nothing: the field border turns
//! `text-2` and the count reads "Invalid pattern" in ink with the engine's message in its
//! tooltip (M3c-11: no red). With no matches or an invalid pattern the step buttons and Select
//! all are disabled.
//!
//! **Keys.** `editor.find` (⌘F / Ctrl+F, Editor context) opens it, seeded with a one-line
//! selection (else the find pasteboard on macOS, else the last query, selected). Typing finds
//! the first match from where find started (the editor scrolls to it and opens a fold over
//! it). Enter / Shift+Enter step; the editor's find chords (⌘G, ⇧⌘G, F3, ⌃⌘G) work in the
//! field too: its focus handle carries the Editor context and those commands route here while
//! it has the focus ([`run_in_bar`]). Tab moves field → toggles → previous → next → Select all
//! → close in the window's own order. **Escape** (or close) ends find and returns the caret to
//! where find started, as long as the selection is still the one find made (a caret the user
//! moved stays). `editor.find_close` (palette) closes it without moving anything. F6 enters the
//! open bar first ([`focus_cycle`]; NOTES M3c cross-cutting 7).
//!
//! **Turkish-tolerant is on by default** when the UI language or the regional format is
//! Turkish (`turkish_tolerant_for`), and the toggle is remembered
//! (`editor.find_turkish_tolerant` through `shell::update_settings`).
//!
//! **macOS find pasteboard:** each query is written to it; a bar opened without a seed or an
//! earlier query reads it. (`⌘E` "Use selection for find" has no Task 1b row; logged.)

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use hacivat_l10n::{ArgValue, turkish_tolerant_for};
use hacivat_settings::FindTolerancePref;
use hacivat_ui::button::{Button, ButtonKind};
use hacivat_ui::icon::{Icon, IconName, IconSize};
use hacivat_ui::input::{FocusedInput, InputEvent, TextInput};
use hacivat_ui::popover::{A11yNode, Tip};
use hacivat_ui::step_trail::run_marker;
use hacivat_ui::text::{Label, TextFn, TextSource};
use hacivat_ui_core::gpui_surface::{
    AnyElement, App, AppContext as _, Context, Entity, EntityId, FocusHandle, Focusable as _,
    Global, InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, Render, Role,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, WeakEntity, Window,
    div, px,
};
use hacivat_ui_core::theme::{Theme, ThemeMode, focus_outline};
use hacivat_ui_editor::{EDITOR_CONTEXT, EditorEvent, EditorView, FindMatcher, FindStatus};
use regex::{Regex, RegexBuilder};

use crate::console::{ConsoleId, Consoles};
use crate::exec_ui::marks::message;

// ---- the matcher (R-M3-10; Task 37 reuses it) ------------------------------------------------

/// The find options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct FindOpts {
    pub match_case: bool,
    pub words: bool,
    pub regex: bool,
    pub tolerant: bool,
}

/// A compiled query over the original text.
#[derive(Debug, Clone)]
pub struct RegexMatcher {
    re: Regex,
    words: bool,
}

/// The most a compiled pattern may take (ALG UI-EDIT-22 step 11).
const SIZE_LIMIT: usize = 10 << 20;
// ## m3c-t33-find (fix round 1, I2)
/// The longest literal compiled with Turkish-tolerant classes (twice the 1,000-byte seed);
/// a longer one is matched as written (case folding still applies).
pub const TOLERANT_MAX: usize = 2_000;

/// The Turkish-tolerant letter classes: each group's lower and upper case members (the
/// Turkish letters and the Latin-1 / Latin Extended-A accents).
const GROUPS: &[(&str, &str)] = &[
    ("aàáâãäåāăą", "AÀÁÂÃÄÅĀĂĄ"),
    ("cçćĉċč", "CÇĆĈĊČ"),
    ("dďđ", "DĎĐ"),
    ("eèéêëēĕėęě", "EÈÉÊËĒĔĖĘĚ"),
    ("gĝğġģ", "GĜĞĠĢ"),
    ("hĥħ", "HĤĦ"),
    ("iıìíîïĩīĭį", "IİÌÍÎÏĨĪĬĮ"),
    ("jĵ", "JĴ"),
    ("kķ", "KĶ"),
    ("lĺļľŀł", "LĹĻĽĿŁ"),
    ("nñńņň", "NÑŃŅŇ"),
    ("oòóôõöøōŏő", "OÒÓÔÕÖØŌŎŐ"),
    ("rŕŗř", "RŔŖŘ"),
    ("sśŝşš", "SŚŜŞŠ"),
    ("tţťŧ", "TŢŤŦ"),
    ("uùúûüũūŭůűų", "UÙÚÛÜŨŪŬŮŰŲ"),
    ("wŵ", "WŴ"),
    ("yýÿŷ", "YÝŸŶ"),
    ("zźżž", "ZŹŻŽ"),
];

/// The class members of letter `c` with Turkish-tolerant on (its own case only with
/// `match_case`); `None` for a character with no variants.
fn class_of(c: char, match_case: bool) -> Option<String> {
    let (lower, upper) = GROUPS
        .iter()
        .find(|(l, u)| l.contains(c) || u.contains(c))?;
    Some(match (match_case, lower.contains(c)) {
        (true, true) => (*lower).to_owned(),
        (true, false) => (*upper).to_owned(),
        (false, _) => format!("{lower}{upper}"),
    })
}

/// A combining accent (U+0300–U+036F): ignored in a tolerant query.
fn is_accent(c: char) -> bool {
    ('\u{300}'..='\u{36F}').contains(&c)
}

/// `c` as a tolerant class with its decomposed accents (a group, so a quantifier after it
/// applies to the whole).
fn tolerant_class(class: &str) -> String {
    // ## m3c-t33-find (fix round 1, I2): the combining accents only (U+0300–U+036F), not all
    // of `\p{Mn}`: its automaton per letter pushed a 550-letter query past the size limit.
    format!(r"(?:[{class}][\x{{300}}-\x{{36F}}]*)")
}

/// A literal query as a pattern.
fn literal_pattern(q: &str, o: FindOpts) -> String {
    let mut out = String::new();
    let mut buf = [0u8; 4];
    for c in q.chars() {
        if o.tolerant {
            if is_accent(c) {
                continue;
            }
            if let Some(class) = class_of(c, o.match_case) {
                out.push_str(&tolerant_class(&class));
                continue;
            }
        }
        out.push_str(&regex::escape(c.encode_utf8(&mut buf)));
    }
    out
}

/// Copies an escape's body (after its `\` and letter): `{…}`, or `n` characters.
fn copy_escape_body(chars: &[char], i: &mut usize, n: usize, out: &mut String) {
    if chars.get(*i) == Some(&'{') {
        while let Some(&c) = chars.get(*i) {
            out.push(c);
            *i += 1;
            if c == '}' {
                break;
            }
        }
    } else {
        for _ in 0..n {
            let Some(&c) = chars.get(*i) else { break };
            out.push(c);
            *i += 1;
        }
    }
}

/// Copies one escape sequence starting at `\`.
fn copy_escape(chars: &[char], i: &mut usize, out: &mut String) {
    out.push('\\');
    *i += 1;
    let Some(&e) = chars.get(*i) else { return };
    out.push(e);
    *i += 1;
    match e {
        'p' | 'P' => copy_escape_body(chars, i, 1, out),
        'x' => copy_escape_body(chars, i, 2, out),
        'u' => copy_escape_body(chars, i, 4, out),
        'U' => copy_escape_body(chars, i, 8, out),
        _ => {}
    }
}

/// A user regex with its literal letters made tolerant (module docs).
fn tolerant_regex(p: &str, match_case: bool) -> String {
    let chars: Vec<char> = p.chars().collect();
    let mut out = String::with_capacity(p.len() * 2);
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        match c {
            '\\' => copy_escape(&chars, &mut i, &mut out),
            '[' => copy_class(&chars, &mut i, match_case, &mut out),
            '(' if chars.get(i + 1) == Some(&'?') => {
                // A group's flags or name: copied up to its `:`, `)` or `>`.
                let named = matches!(
                    (chars.get(i + 2), chars.get(i + 3)),
                    (Some('<'), _) | (Some('P'), Some('<'))
                );
                while let Some(&g) = chars.get(i) {
                    out.push(g);
                    i += 1;
                    if (named && g == '>') || (!named && (g == ':' || g == ')')) {
                        break;
                    }
                }
            }
            '{' => {
                // A repetition's bounds.
                while let Some(&g) = chars.get(i) {
                    out.push(g);
                    i += 1;
                    if g == '}' {
                        break;
                    }
                }
            }
            _ => {
                match class_of(c, match_case) {
                    Some(class) => out.push_str(&tolerant_class(&class)),
                    None if is_accent(c) => {}
                    None => out.push(c),
                }
                i += 1;
            }
        }
    }
    out
}

/// Copies a bracketed class, each single letter followed by its tolerant variants (ranges and
/// `[:name:]` classes as written).
fn copy_class(chars: &[char], i: &mut usize, match_case: bool, out: &mut String) {
    let mut depth = 0usize;
    let mut first = true;
    while let Some(&c) = chars.get(*i) {
        match c {
            '\\' => {
                copy_escape(chars, i, out);
                first = false;
                continue;
            }
            '[' if chars.get(*i + 1) == Some(&':') => {
                while let Some(&g) = chars.get(*i) {
                    out.push(g);
                    *i += 1;
                    if g == ']' {
                        break;
                    }
                }
                continue;
            }
            '[' => {
                depth += 1;
                out.push(c);
                *i += 1;
                first = true;
                if chars.get(*i) == Some(&'^') {
                    out.push('^');
                    *i += 1;
                }
                continue;
            }
            ']' if !first => {
                out.push(c);
                *i += 1;
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return;
                }
                continue;
            }
            _ => {}
        }
        first = false;
        let range = chars.get(*i + 1) == Some(&'-') && chars.get(*i + 2).is_some_and(|&e| e != ']');
        if range {
            out.push(c);
            out.push('-');
            *i += 2;
            // Fix round 1 (M6): the range's end may be an escape (`[a-\x7A]`).
            if chars.get(*i) == Some(&'\\') {
                copy_escape(chars, i, out);
            } else if let Some(&g) = chars.get(*i) {
                out.push(g);
                *i += 1;
            }
            continue;
        }
        out.push(c);
        *i += 1;
        if let Some(class) = class_of(c, match_case) {
            out.extend(class.chars().filter(|&m| m != c));
        }
    }
}

/// The engine's message, short: its last line.
fn short_error(e: &regex::Error) -> String {
    let text = e.to_string();
    text.lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or(&text)
        .trim_start_matches("error: ")
        .to_owned()
}

fn compile(body: &str, match_case: bool) -> Result<Regex, regex::Error> {
    let flags = if match_case { "(?mR)" } else { "(?imR)" };
    RegexBuilder::new(&format!("{flags}{body}"))
        .size_limit(SIZE_LIMIT)
        .dfa_size_limit(SIZE_LIMIT)
        .build()
}

/// Compiles `query` with `o`: `Ok(None)` for an empty query (nothing to search), `Err` with the
/// engine's message for an invalid regular expression.
pub fn build_matcher(query: &str, o: FindOpts) -> Result<Option<RegexMatcher>, String> {
    if query.is_empty() {
        return Ok(None);
    }
    let re = if o.regex {
        let plain = compile(query, o.match_case);
        match (o.tolerant, plain) {
            (_, Err(e)) => return Err(short_error(&e)),
            (false, Ok(re)) => re,
            (true, Ok(re)) => {
                compile(&tolerant_regex(query, o.match_case), o.match_case).unwrap_or(re)
            }
        }
    } else {
        // A literal never fails (I2): longer than `TOLERANT_MAX` characters, or past the size
        // limit, the tolerant classes fall back to the plain literal (case folding still
        // applies); a literal too large even then searches nothing.
        let plain = FindOpts {
            tolerant: false,
            ..o
        };
        let tolerant = o.tolerant && query.chars().count() <= TOLERANT_MAX;
        let first = if tolerant { o } else { plain };
        let built = compile(&literal_pattern(query, first), o.match_case)
            .or_else(|_| compile(&literal_pattern(query, plain), o.match_case));
        match built {
            Ok(re) => re,
            Err(_) => return Ok(None),
        }
    };
    Ok(Some(RegexMatcher { re, words: o.words }))
}

/// A SQL word character (identifiers: letters, digits, `_`, `$`).
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// `r` stands as a whole word in `hay`.
fn bounded(hay: &str, r: &Range<usize>) -> bool {
    let before = hay[..r.start].chars().next_back();
    let after = hay[r.end..].chars().next();
    !before.is_some_and(is_word_char) && !after.is_some_and(is_word_char)
}

/// The first position after `at` whose preceding character is not a word character (the next
/// place a whole word may start).
fn past_word_run(hay: &str, at: usize) -> Option<usize> {
    let mut chars = hay.get(at..)?.char_indices();
    let (_, first) = chars.next()?;
    if !is_word_char(first) {
        return Some(at + first.len_utf8());
    }
    for (i, c) in chars {
        if !is_word_char(c) {
            return Some(at + i + c.len_utf8());
        }
    }
    Some(hay.len())
}

/// The character boundary after `at`.
fn after(hay: &str, at: usize) -> Option<usize> {
    let c = hay.get(at..)?.chars().next()?;
    Some(at + c.len_utf8())
}

impl FindMatcher for RegexMatcher {
    fn find(&self, hay: &str, from: usize) -> Option<Range<usize>> {
        let mut at = from;
        while at <= hay.len() {
            if !hay.is_char_boundary(at) {
                at += 1;
                continue;
            }
            let m = self.re.find_at(hay, at)?;
            let r = m.range();
            if r.is_empty() {
                at = after(hay, r.start)?;
                continue;
            }
            if !self.words || bounded(hay, &r) {
                return Some(r);
            }
            // Fix round 1 (M5): a whole-word match starts only where the character before is
            // not a word character, so the rest of this word run is skipped at once (linear,
            // where retrying each next character was quadratic for a greedy pattern).
            at = past_word_run(hay, r.start)?;
        }
        None
    }

    fn find_back(&self, hay: &str, before: usize) -> Option<Range<usize>> {
        let mut last = None;
        let mut at = 0;
        while let Some(r) = self.find(hay, at) {
            if r.end > before {
                break;
            }
            at = r.end;
            last = Some(r);
        }
        last
    }
}

/// A matcher that finds nothing: the editor's query while the field is empty or invalid (the
/// bar stays open and nothing is searched).
struct Nothing;

impl FindMatcher for Nothing {
    fn find(&self, _: &str, _: usize) -> Option<Range<usize>> {
        None
    }
    fn find_back(&self, _: &str, _: usize) -> Option<Range<usize>> {
        None
    }
}

/// Turkish-tolerant's state when the bar opens: the remembered choice, else on when the UI
/// language or the regional format is Turkish (D-074).
pub fn tolerant_default(cx: &App) -> bool {
    let pref = cx
        .try_global::<crate::AppState>()
        .map(|s| s.settings.editor.find_turkish_tolerant)
        .unwrap_or_default();
    match pref {
        FindTolerancePref::On => true,
        FindTolerancePref::Off => false,
        FindTolerancePref::Auto => cx
            .try_global::<crate::l10n_global::L10nGlobal>()
            .is_some_and(|g| {
                let rt = g.l10n.snapshot();
                let ui = rt
                    .ui_chain
                    .first()
                    .map(ToString::to_string)
                    .unwrap_or_default();
                turkish_tolerant_for(&ui, &rt.format.format_locale)
            }),
    }
}

// ---- the bar ---------------------------------------------------------------------------------

/// The most matches Select all occurrences selects (UI-EDIT-05 step 6).
pub const SELECT_ALL_CAP: u32 = 10_000;

/// Every console's bar (made when its editor is).
#[derive(Default)]
struct Bars(HashMap<ConsoleId, Entity<FindBar>>);

impl Global for Bars {}

/// The toggles, in their order inside the field.
const TOGGLES: [Toggle; 4] = [Toggle::Case, Toggle::Words, Toggle::Regex, Toggle::Tolerant];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Toggle {
    Case,
    Words,
    Regex,
    Tolerant,
}

impl Toggle {
    fn icon(self) -> IconName {
        match self {
            Toggle::Case => IconName::CaseSensitive,
            Toggle::Words => IconName::WholeWord,
            Toggle::Regex => IconName::Regex,
            Toggle::Tolerant => IconName::Languages,
        }
    }

    fn name_id(self) -> &'static str {
        match self {
            Toggle::Case => "hv-find-match-case",
            Toggle::Words => "hv-find-words",
            Toggle::Regex => "hv-find-regex",
            Toggle::Tolerant => "hv-find-tolerant",
        }
    }

    fn element_id(self) -> &'static str {
        match self {
            Toggle::Case => "find-match-case",
            Toggle::Words => "find-words",
            Toggle::Regex => "find-regex",
            Toggle::Tolerant => "find-tolerant",
        }
    }

    fn get(self, o: &FindOpts) -> bool {
        match self {
            Toggle::Case => o.match_case,
            Toggle::Words => o.words,
            Toggle::Regex => o.regex,
            Toggle::Tolerant => o.tolerant,
        }
    }

    fn flip(self, o: &mut FindOpts) {
        match self {
            Toggle::Case => o.match_case = !o.match_case,
            Toggle::Words => o.words = !o.words,
            Toggle::Regex => o.regex = !o.regex,
            Toggle::Tolerant => o.tolerant = !o.tolerant,
        }
    }
}

/// One console's find bar.
pub struct FindBar {
    console: ConsoleId,
    editor: WeakEntity<EditorView>,
    input: Entity<TextInput>,
    toggles: [FocusHandle; 4],
    prev: FocusHandle,
    next: FocusHandle,
    all: FocusHandle,
    close: FocusHandle,
    opts: FindOpts,
    open: bool,
    /// The engine's message for an invalid pattern.
    error: Option<SharedString>,
    /// The main selection when find started (Escape returns there while the selection is
    /// still the one find made, `EditorView::find_made`).
    origin: Range<usize>,
    // ## m3c-t33-find (fix round 1, M3, M4)
    /// The last count shown that was not waiting for its first batch: shown (as pending) while
    /// a new count starts, so the count and the buttons do not flash.
    last: Option<FindStatus>,
    /// The count's accessible name: changed only when a count settles (announced once).
    announced: SharedString,
    _subscriptions: Vec<Subscription>,
}

/// Attaches a find bar to console `console`'s editor (once, when the editor is made or shown).
pub fn attach(console: ConsoleId, editor: &Entity<EditorView>, window: &mut Window, cx: &mut App) {
    if cx
        .try_global::<Bars>()
        .and_then(|b| b.0.get(&console))
        .is_some_and(|bar| bar.read(cx).editor.upgrade().as_ref() == Some(editor))
    {
        return;
    }
    let bar = cx.new(|cx| FindBar::new(console, editor, window, cx));
    cx.default_global::<Bars>().0.insert(console, bar);
}

/// Forgets console `console`'s bar (its console closed).
pub fn console_closed(console: ConsoleId, cx: &mut App) {
    if cx.has_global::<Bars>() {
        cx.global_mut::<Bars>().0.remove(&console);
    }
}

fn bar(console: ConsoleId, cx: &App) -> Option<Entity<FindBar>> {
    cx.try_global::<Bars>()?.0.get(&console).cloned()
}

/// The bar of console `console`, while it is open.
pub fn render(console: ConsoleId, cx: &App) -> Option<AnyElement> {
    let bar = bar(console, cx)?;
    bar.read(cx).open.then(|| bar.into_any_element())
}

/// The console whose find field has the keyboard focus.
pub fn focused_bar(cx: &App) -> Option<ConsoleId> {
    let focused = cx.try_global::<FocusedInput>()?.entity?;
    let bars = cx.try_global::<Bars>()?;
    bars.0
        .iter()
        .find(|(_, b)| b.read(cx).input.entity_id() == focused)
        .map(|(c, _)| *c)
}

/// The editor commands that act on the bar while its field has the focus.
pub const ROUTED: [&str; 5] = [
    "editor.find",
    "editor.find_next",
    "editor.find_prev",
    "editor.select_all_occurrences",
    "editor.find_close",
];

/// Runs editor command `id` on the focused find bar; `false` when no bar has the focus or the
/// command is not one of [`ROUTED`].
pub fn run_in_bar(id: &str, window: &mut Window, cx: &mut App) -> bool {
    let Some(console) = focused_bar(cx).filter(|_| ROUTED.contains(&id)) else {
        return false;
    };
    let Some(bar) = bar(console, cx) else {
        return false;
    };
    bar.update(cx, |b, cx| match id {
        "editor.find" => {
            b.input.update(cx, |i, cx| {
                i.select_all();
                cx.notify();
            });
        }
        "editor.find_next" => b.step(false, cx),
        "editor.find_prev" => b.step(true, cx),
        "editor.select_all_occurrences" => b.select_all(window, cx),
        _ => b.close(true, window, cx),
    });
    true
}

/// F6 (`workspace.focus_notification`): with the focused console's bar open, the first F6
/// from its editor enters the find field; from the bar the cycle goes on (the error bar, the
/// toasts, the editor). `true` when this moved the focus.
pub fn focus_cycle(window: &mut Window, cx: &mut App) -> bool {
    let Some(console) = crate::exec_ui::cancel::focused_console(cx) else {
        return false;
    };
    let Some(bar) = bar(console, cx).filter(|b| b.read(cx).open) else {
        return false;
    };
    let in_bar = bar.read(cx).has_focus(window, cx);
    if in_bar {
        if crate::exec_ui::error_bar::focus_cycle(window, cx) {
            return true;
        }
        focus_editor(console, window, cx);
        hacivat_ui::toast::focus_newest_sticky(window, cx);
        return true;
    }
    let editor_focused = Consoles::get(cx, console)
        .and_then(|s| s.editor.clone())
        .is_some_and(|e| e.read(cx).focus_handle(cx).is_focused(window));
    if !editor_focused {
        return false;
    }
    let handle = bar.read(cx).input.focus_handle(cx);
    window.focus(&handle, cx);
    true
}

fn focus_editor(console: ConsoleId, window: &mut Window, cx: &mut App) {
    if let Some(editor) = Consoles::get(cx, console).and_then(|s| s.editor.clone()) {
        let handle = editor.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
    }
}

/// Re-renders console `console`'s host (the bar opened or closed under its toolbar).
fn notify_host(console: ConsoleId, cx: &mut App) {
    let Some(tab) = Consoles::get(cx, console).map(|s| s.tab) else {
        return;
    };
    if let Some(host) = crate::console::view::host_of(cx, tab) {
        host.update(cx, |_, cx| cx.notify());
    }
}

// ---- test seams --------------------------------------------------------------------------------

/// Whether console `console`'s bar is open.
pub fn is_open(console: ConsoleId, cx: &App) -> bool {
    bar(console, cx).is_some_and(|b| b.read(cx).open)
}

/// The bar's query.
pub fn query(console: ConsoleId, cx: &App) -> Option<String> {
    Some(bar(console, cx)?.read(cx).input.read(cx).value().to_owned())
}

/// The bar's options.
pub fn options(console: ConsoleId, cx: &App) -> Option<FindOpts> {
    Some(bar(console, cx)?.read(cx).opts)
}

/// The bar's invalid-pattern message.
pub fn error(console: ConsoleId, cx: &App) -> Option<SharedString> {
    bar(console, cx)?.read(cx).error.clone()
}

/// The bar's focus handles in Tab order: the field, the four toggles, previous, next, Select
/// all, close.
pub fn focus_order(console: ConsoleId, cx: &App) -> Vec<FocusHandle> {
    let Some(bar) = bar(console, cx) else {
        return Vec::new();
    };
    let b = bar.read(cx);
    let mut out = vec![b.input.focus_handle(cx)];
    out.extend(b.toggles.iter().cloned());
    out.extend([
        b.prev.clone(),
        b.next.clone(),
        b.all.clone(),
        b.close.clone(),
    ]);
    out
}

/// Types `query` into console `console`'s field, as the input would on an edit.
pub fn set_query(console: ConsoleId, query: &str, window: &mut Window, cx: &mut App) {
    let Some(bar) = bar(console, cx) else { return };
    bar.update(cx, |b, cx| {
        b.input.update(cx, |i, cx| {
            i.set_value(query.to_owned());
            cx.notify();
        });
        b.apply(window, cx);
    });
}

/// Flips toggle `index` (0 Match case, 1 Whole words, 2 Regular expression, 3 Turkish-tolerant)
/// as a click would.
pub fn flip_toggle(console: ConsoleId, index: usize, window: &mut Window, cx: &mut App) {
    let Some(bar) = bar(console, cx) else { return };
    if let Some(t) = TOGGLES.get(index).copied() {
        bar.update(cx, |b, cx| b.flip(t, window, cx));
    }
}

// ---- the view ----------------------------------------------------------------------------------

impl FindBar {
    fn new(
        console: ConsoleId,
        editor: &Entity<EditorView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let placeholder: TextFn =
            std::rc::Rc::new(|cx: &App| message(cx, "hv-find-placeholder", vec![]));
        let input = cx.new(|cx| {
            let mut input = TextInput::new(cx, placeholder);
            input.set_mono(true);
            input.set_frameless(true);
            input
        });
        // The field's keys reach the editor's find commands (routed here, `run_in_bar`).
        let field = input.focus_handle(cx);
        crate::shell::register_focus_context(cx, field, EDITOR_CONTEXT);
        let subscriptions = vec![
            cx.subscribe_in(editor, window, Self::on_editor_event),
            cx.subscribe_in(&input, window, Self::on_input_event),
        ];
        let handle = |cx: &mut Context<Self>| cx.focus_handle().tab_stop(true);
        FindBar {
            console,
            editor: editor.downgrade(),
            input,
            toggles: [handle(cx), handle(cx), handle(cx), handle(cx)],
            prev: handle(cx),
            next: handle(cx),
            all: handle(cx),
            close: handle(cx),
            opts: FindOpts {
                tolerant: tolerant_default(cx),
                ..FindOpts::default()
            },
            open: false,
            error: None,
            origin: 0..0,
            last: None,
            announced: SharedString::default(),
            _subscriptions: subscriptions,
        }
    }

    fn has_focus(&self, window: &Window, cx: &App) -> bool {
        self.input.focus_handle(cx).is_focused(window)
            || self
                .toggles
                .iter()
                .chain([&self.prev, &self.next, &self.all, &self.close])
                .any(|h| h.is_focused(window))
    }

    fn on_editor_event(
        &mut self,
        editor: &Entity<EditorView>,
        event: &EditorEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            EditorEvent::FindRequested { seed } => self.open(editor, seed.clone(), window, cx),
            EditorEvent::FindChanged => {
                // `editor.find_close` (or anyone) cleared the query: the bar closes in place.
                if self.open && !editor.read(cx).find_status().active {
                    self.close(false, window, cx);
                }
                cx.notify();
            }
            _ => {}
        }
    }

    fn on_input_event(
        &mut self,
        _: &Entity<TextInput>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Changed => self.apply(window, cx),
            InputEvent::Cancelled => self.close(true, window, cx),
            // Enter is taken before the input (`on_field_key`), so Shift+Enter steps back.
            InputEvent::Submitted | InputEvent::CompositionChanged(_) => {}
        }
    }

    /// Opens the bar (or takes the focus back to it), seeded with `seed`.
    fn open(
        &mut self,
        editor: &Entity<EditorView>,
        seed: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let main = editor.read(cx).snapshot().selection.main().range();
        if !self.open {
            self.open = true;
            self.opts.tolerant = tolerant_default(cx);
        }
        // Asked again after the user moved the caret: find starts from there now.
        if editor.read(cx).find_made().as_ref() != Some(&main) {
            self.origin = main;
        }
        let seed = seed.or_else(|| {
            let empty = self.input.read(cx).value().is_empty();
            find_pasteboard(cx).filter(|_| empty)
        });
        // Fix round 1 (M11): a seed is text; with Regular expression on it is escaped.
        let seed = seed.map(|s| {
            if self.opts.regex {
                regex::escape(&s)
            } else {
                s
            }
        });
        if let Some(seed) = seed {
            self.input.update(cx, |i, cx| {
                i.set_value(seed);
                cx.notify();
            });
        }
        self.input.update(cx, |i, cx| {
            i.select_all();
            cx.notify();
        });
        self.apply(window, cx);
        let handle = self.input.focus_handle(cx);
        window.focus(&handle, cx);
        notify_host(self.console, cx);
        cx.notify();
    }

    /// The query or an option changed: a new matcher, and the first match from where find
    /// started.
    fn apply(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.open {
            return;
        }
        let Some(editor) = self.editor.upgrade() else {
            return;
        };
        let query = self.input.read(cx).value().to_owned();
        let built = build_matcher(&query, self.opts);
        self.error = built.as_ref().err().map(|e| SharedString::from(e.clone()));
        let matcher: Arc<dyn FindMatcher> = match built {
            Ok(Some(m)) => Arc::new(m),
            Ok(None) | Err(_) => Arc::new(Nothing),
        };
        // Fix round 1 (M14): a caret the user moved since find last selected is the new origin.
        let (main, made) = {
            let v = editor.read(cx);
            (v.snapshot().selection.main().range(), v.find_made())
        };
        if made.as_ref() != Some(&main) && main != self.origin {
            self.origin = main;
        }
        // Fix round 1 (I1, I3): the first match from the origin is found on the background
        // executor and lands as one selection change.
        let origin = self.origin.clone();
        editor.update(cx, |v, cx| {
            v.set_find(Some(matcher), cx);
            v.find_from(origin, cx);
        });
        cx.notify();
    }

    fn step(&mut self, back: bool, cx: &mut Context<Self>) {
        let Some(editor) = self.editor.upgrade() else {
            return;
        };
        editor.update(cx, |v, cx| v.find_step(back, cx));
        self.commit_query(cx);
        cx.notify();
    }

    fn select_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.editor.upgrade() else {
            return;
        };
        // It answers 0 while it waits for the background (fix round 1, M8): the carets land
        // then. Either way they are the user's, and the editor takes the keys.
        editor.update(cx, |v, cx| v.select_all_matches(SELECT_ALL_CAP, cx));
        self.commit_query(cx);
        let handle = editor.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        cx.notify();
    }

    fn flip(&mut self, t: Toggle, window: &mut Window, cx: &mut Context<Self>) {
        t.flip(&mut self.opts);
        if t == Toggle::Tolerant {
            let on = self.opts.tolerant;
            crate::shell::update_settings(cx, |s| {
                s.editor.find_turkish_tolerant = if on {
                    FindTolerancePref::On
                } else {
                    FindTolerancePref::Off
                };
            });
        }
        self.apply(window, cx);
    }

    /// Closes the bar; `restore`: the caret goes back to where find started (while the
    /// selection is still the one find made).
    fn close(&mut self, restore: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.open {
            return;
        }
        self.open = false;
        let had_focus = self.has_focus(window, cx);
        self.commit_query(cx);
        self.last = None;
        if let Some(editor) = self.editor.upgrade() {
            let origin = self.origin.clone();
            editor.update(cx, |v, cx| {
                let main = v.snapshot().selection.main().range();
                if restore && v.find_made().as_ref() == Some(&main) {
                    v.select_range(origin, cx);
                }
                if v.find_status().active {
                    v.set_find(None, cx);
                }
            });
            if had_focus {
                let handle = editor.read(cx).focus_handle(cx);
                window.focus(&handle, cx);
            }
        }
        self.error = None;
        notify_host(self.console, cx);
        cx.notify();
    }

    // ## m3c-t33-find (fix round 1, M10)
    /// A committed search (a step, Select all, close) writes its query to the find pasteboard;
    /// typing never does.
    fn commit_query(&self, cx: &App) {
        let query = self.input.read(cx).value();
        if !query.is_empty() && self.error.is_none() {
            write_find_pasteboard(query, cx);
        }
    }

    /// Enter and Shift+Enter in the field step (taken before the input, which would submit).
    fn on_field_key(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let k = &ev.keystroke;
        let m = &k.modifiers;
        if k.key != "enter" || m.control || m.alt || m.platform || m.function {
            return;
        }
        if self.input.read(cx).is_composing() {
            return;
        }
        cx.stop_propagation();
        self.step(m.shift, cx);
    }

    /// Escape anywhere in the bar (the field answers it through `InputEvent::Cancelled`).
    fn on_bar_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &ev.keystroke;
        if k.key == "escape" && !k.modifiers.modified() {
            cx.stop_propagation();
            self.close(true, window, cx);
        }
    }

    /// What the count shows: (text, tooltip, counting, invalid).
    fn count(
        &self,
        status: FindStatus,
        cx: &App,
    ) -> Option<(SharedString, Option<SharedString>, bool, bool)> {
        if let Some(e) = &self.error {
            let tip = message(
                cx,
                "hv-find-invalid-tip",
                vec![("error", ArgValue::Text(e.to_string()))],
            );
            return Some((
                message(cx, "hv-find-invalid", vec![]),
                Some(tip),
                false,
                true,
            ));
        }
        if self.input.read(cx).value().is_empty() {
            return None;
        }
        let counting = status.total.is_none();
        let n = u64::from(status.total.unwrap_or(status.counted));
        if !counting && n == 0 {
            return Some((message(cx, "hv-find-none", vec![]), None, false, false));
        }
        let text = match (status.current, counting) {
            (Some(i), false) => message(
                cx,
                "hv-find-count",
                vec![
                    ("i", ArgValue::Count(u64::from(i))),
                    ("n", ArgValue::Count(n)),
                ],
            ),
            (Some(i), true) => message(
                cx,
                "hv-find-count-more",
                vec![
                    ("i", ArgValue::Count(u64::from(i))),
                    ("n", ArgValue::Count(n)),
                ],
            ),
            (None, false) => message(cx, "hv-find-total", vec![("n", ArgValue::Count(n))]),
            (None, true) => message(cx, "hv-find-total-more", vec![("n", ArgValue::Count(n))]),
        };
        let tip = counting.then(|| message(cx, "hv-find-counting-tip", vec![]));
        Some((text, tip, counting, false))
    }
}

// ## m3c-t33-find (fix round 1, M10)
/// Reads the find pasteboard's query.
pub type PasteboardRead = std::rc::Rc<dyn Fn(&App) -> Option<String>>;
/// Writes a query to the find pasteboard.
pub type PasteboardWrite = std::rc::Rc<dyn Fn(&str, &App)>;

/// The find pasteboard the bar uses: none unless installed (tests and the shots harness never
/// touch the system's); the app's binary installs the macOS one
/// ([`install_system_find_pasteboard`]).
struct FindPasteboard {
    read: PasteboardRead,
    write: PasteboardWrite,
}

impl Global for FindPasteboard {}

/// Installs a find pasteboard (tests inject one).
pub fn install_find_pasteboard(cx: &mut App, read: PasteboardRead, write: PasteboardWrite) {
    cx.set_global(FindPasteboard { read, write });
}

/// Installs the macOS find pasteboard (shared between apps); nothing elsewhere.
pub fn install_system_find_pasteboard(cx: &mut App) {
    #[cfg(target_os = "macos")]
    install_find_pasteboard(
        cx,
        std::rc::Rc::new(|cx: &App| cx.read_from_find_pasteboard()?.text()),
        std::rc::Rc::new(|query: &str, cx: &App| {
            cx.write_to_find_pasteboard(hacivat_ui_core::gpui_surface::ClipboardItem::new_string(
                query.to_owned(),
            ));
        }),
    );
    #[cfg(not(target_os = "macos"))]
    let _ = cx;
}

/// The find pasteboard's query (one line, not empty).
fn find_pasteboard(cx: &App) -> Option<String> {
    let pb = cx.try_global::<FindPasteboard>()?;
    (pb.read)(cx).filter(|t| !t.is_empty() && !t.contains(['\n', '\r']))
}

fn write_find_pasteboard(query: &str, cx: &App) {
    if let Some(pb) = cx.try_global::<FindPasteboard>() {
        (pb.write)(query, cx);
    }
}

impl Render for FindBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let status = self
            .editor
            .upgrade()
            .map(|e| e.read(cx).find_status())
            .unwrap_or_default();
        // Fix round 1 (M3): until the new count's first batch lands, the last count stays (as
        // pending, with the amber marker) and so do the buttons.
        let pending = status.active && status.total.is_none() && status.counted == 0;
        let stale = pending && self.last.is_some();
        let status = match self.last {
            Some(last) if pending => last,
            _ => {
                if status.active && !pending {
                    self.last = Some(status);
                }
                status
            }
        };
        let count = self.count(status, cx);
        let searchable = self.error.is_none()
            && !self.input.read(cx).value().is_empty()
            && status.total.is_none_or(|n| n > 0)
            && (status.total.is_some() || status.counted > 0);
        let well = match theme.mode {
            ThemeMode::Dark => c.panel,
            ThemeMode::Light => c.ground,
        };
        let field_focused = self.input.focus_handle(cx).is_focused(window);
        let field_handle = self.input.focus_handle(cx).tab_stop(true);
        let mut field = div()
            .id("find-field")
            .debug_selector(|| "find-field".to_string())
            .flex()
            .flex_row()
            .items_center()
            .h(px(24.0))
            .flex_grow(1.0)
            .flex_shrink(1.0)
            .flex_basis(px(300.0))
            .min_w(px(150.0))
            .max_w(px(380.0))
            .gap(px(2.0))
            .pl(px(7.0))
            .pr(px(2.0))
            .rounded(px(t.radii.sm))
            .border_1()
            // An invalid pattern: the border in ink (`text-2`), never red (M3c-11).
            .border_color(if self.error.is_some() {
                c.text_2.to_hsla()
            } else {
                c.seam_strong.to_hsla()
            })
            .bg(well.to_hsla())
            .child(
                div().mr(px(4.0)).flex_none().child(
                    // Inside the well its fill restores text-3 (NOTES cross-cutting #4).
                    Icon::new(IconName::Search)
                        .size(IconSize::Chrome)
                        .color(c.text_3.to_hsla()),
                ),
            )
            .child(
                div()
                    .id("find-query")
                    .flex_1()
                    .min_w(px(40.0))
                    .track_focus(&field_handle)
                    .capture_key_down(cx.listener(Self::on_field_key))
                    .child(self.input.clone()),
            );
        for (i, toggle) in TOGGLES.into_iter().enumerate() {
            field = field.child(self.toggle(toggle, i, window, cx));
        }
        let field = focus_outline(field, &theme, field_focused);
        let mut count_el = div()
            .id("find-count")
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .min_w(px(58.0))
            .gap(px(6.0));
        // Fix round 1 (M4): the polite region's name changes only when a count settles.
        match &count {
            Some((text, _, false, _)) if !stale => self.announced = text.clone(),
            None => self.announced = SharedString::default(),
            _ => {}
        }
        let count_name = self.announced.clone();
        if let Some((text, tip, counting, invalid)) = count {
            let tip = if stale {
                Some(message(cx, "hv-find-counting-tip", vec![]))
            } else {
                tip
            };
            if counting || stale {
                count_el = count_el.child(run_marker(&theme));
            }
            if invalid {
                count_el = count_el.child(
                    Icon::new(IconName::CircleAlert)
                        .size(IconSize::Chrome)
                        .color(c.text.to_hsla()),
                );
            }
            let (style, ink) = if invalid {
                (t.type_ramp.caption, c.text.to_hsla())
            } else if status.total == Some(0) {
                (t.type_ramp.caption, c.text_2.to_hsla())
            } else {
                (t.type_ramp.mono_data, c.text_2.to_hsla())
            };
            count_el = count_el
                .child(Label::new(text, TextSource::Fluent, style, ink).id("find-count-text"));
            if let Some(tip) = tip {
                count_el = count_el.tooltip(move |_, cx| Tip::view(tip.clone(), cx));
            }
        }
        let count_el = A11yNode::new("find-count-node", Role::Status, count_name, false, count_el)
            .live(hacivat_ui_core::gpui_surface::accesskit::Live::Polite);
        let entity = cx.entity();
        let icon_button = |id: &'static str,
                           name: &'static str,
                           icon: IconName,
                           handle: &FocusHandle,
                           disabled: bool,
                           cx: &App| {
            Button::new(id, message(cx, name, vec![]))
                .kind(ButtonKind::Icon)
                .icon(icon)
                .compact(true)
                .on_raised(true)
                .disabled(disabled)
                .focus_handle(handle.clone())
        };
        let (e1, e2, e3, e4) = (
            entity.clone(),
            entity.clone(),
            entity.clone(),
            entity.clone(),
        );
        let prev = icon_button(
            "find-prev",
            "hv-find-prev",
            IconName::ArrowUp,
            &self.prev,
            !searchable,
            cx,
        )
        .on_click(move |_, _, cx| e1.update(cx, |b, cx| b.step(true, cx)));
        let next = icon_button(
            "find-next",
            "hv-find-next",
            IconName::ArrowDown,
            &self.next,
            !searchable,
            cx,
        )
        .on_click(move |_, _, cx| e2.update(cx, |b, cx| b.step(false, cx)));
        let all = Button::new(
            "find-select-all",
            message(cx, "hv-cmd-editor-select-all-occurrences", vec![]),
        )
        .kind(ButtonKind::Ghost)
        .compact(true)
        .on_raised(true)
        .disabled(!searchable)
        .focus_handle(self.all.clone())
        .on_click(move |_, window, cx| e3.update(cx, |b, cx| b.select_all(window, cx)));
        let close = icon_button(
            "find-close",
            "hv-cmd-editor-find-close",
            IconName::X,
            &self.close,
            false,
            cx,
        )
        .on_click(move |_, window, cx| e4.update(cx, |b, cx| b.close(true, window, cx)));
        let row = div()
            .id(("find-bar", self.console.0))
            .debug_selector(|| "find-bar".to_string())
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .h(px(32.0))
            .gap(px(6.0))
            .pl(px(12.0))
            .pr(px(10.0))
            .overflow_hidden()
            .bg(c.raised.to_hsla())
            .border_b_1()
            .border_color(c.seam.to_hsla())
            .on_key_down(cx.listener(Self::on_bar_key))
            .child(field)
            .child(count_el)
            .child(prev)
            .child(next)
            .child(all)
            .child(div().flex_1().min_w(px(0.0)))
            .child(close);
        A11yNode::new(
            "find-bar-node",
            Role::Search,
            message(cx, "hv-find-placeholder", vec![]),
            false,
            row,
        )
    }
}

impl FindBar {
    /// One toggle inside the field (NOTES "Find toggle"): 20 × 18, pressed in the selection
    /// tint with a blue border; its name in its tooltip and accessible name.
    fn toggle(&self, t: Toggle, i: usize, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = *Theme::get(cx);
        let c = &theme.tokens.colors;
        let on = t.get(&self.opts);
        let handle = self.toggles[i].clone();
        let focused = handle.is_focused(window);
        let name = message(cx, t.name_id(), vec![]);
        let tip = if t == Toggle::Tolerant {
            message(cx, "hv-find-tolerant-tip", vec![])
        } else {
            name.clone()
        };
        let entity = cx.entity();
        let body = div()
            .id(t.element_id())
            .debug_selector(move || t.element_id().to_string())
            .track_focus(&handle)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .w(px(20.0))
            .h(px(18.0))
            .rounded(px(3.0))
            .border_1()
            .border_color(if on {
                c.blue.to_hsla()
            } else {
                hacivat_ui_core::gpui_surface::hsla(0.0, 0.0, 0.0, 0.0)
            })
            .when_some_bg(on.then(|| c.blue_tint_strong.to_hsla()))
            .hover(|s| s.bg(c.hover.to_hsla()))
            .cursor_pointer()
            .child(Icon::new(t.icon()).size(IconSize::Chrome).color(if on {
                c.text.to_hsla()
            } else {
                c.text_2.to_hsla()
            }))
            .on_click(move |_, window, cx| entity.update(cx, |b, cx| b.flip(t, window, cx)))
            .tooltip(move |_, cx| Tip::view(tip.clone(), cx));
        let body = focus_outline(body, &theme, focused);
        let mut node =
            A11yNode::new(("find-toggle-node", i), Role::Button, name, false, body).toggled(on);
        if t == Toggle::Tolerant {
            node = node.description(message(cx, "hv-find-tolerant-tip", vec![]));
        }
        node.into_any_element()
    }
}

/// `bg` when `Some`.
trait WhenSomeBg: Sized {
    fn when_some_bg(self, color: Option<hacivat_ui_core::gpui_surface::Hsla>) -> Self;
}

impl<E: hacivat_ui_core::gpui_surface::Styled> WhenSomeBg for E {
    fn when_some_bg(self, color: Option<hacivat_ui_core::gpui_surface::Hsla>) -> Self {
        match color {
            Some(c) => self.bg(c),
            None => self,
        }
    }
}

/// The id of console `console`'s bar entity (tests).
pub fn bar_entity_id(console: ConsoleId, cx: &App) -> Option<EntityId> {
    Some(bar(console, cx)?.entity_id())
}

impl std::fmt::Debug for FindBar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FindBar")
            .field("console", &self.console)
            .field("open", &self.open)
            .field("opts", &self.opts)
            .finish_non_exhaustive()
    }
}
