//! Selection commands (Task 25): select all, Escape, the Tab mode, and what a double click
//! selects.
//!
//! - **Escape** (UI-EDIT-05 step 8) collapses many ranges to the main one, then a selection to
//!   its head. An Escape with nothing to collapse is **not consumed**: it releases the next Tab
//!   (or Shift+Tab), which then moves the focus out of the editor instead of indenting, so the
//!   keyboard is never trapped. The release holds for that one key, and only while nothing else
//!   happened in between ([`escape_released`]).
//! - **Ctrl+M** (`editor.toggle_tab_moves_focus`, UI-EDIT-13) turns the same behaviour into a
//!   mode, said to the status bar and to assistive technology.
//! - **A double click** (UI-EDIT-06 step 9) selects the word under it; inside a string literal
//!   it selects the literal's content first, and a second double click in it the literal with
//!   its quotes. A quoted identifier is selected with its quotes at once.
//!
//! **Strings without token kinds.** The token-oracle seam (`hacivat_text::TokenOracle`) gives
//! each token's range, not its kind, and Task 24's kinds are not merged. So a string is
//! recognised here from the token the service gives: one that opens with `'` (after an optional
//! prefix such as `E`, `N`, `X` or `U&`) and ends with `'`, or a dollar-quoted one
//! (`$tag$…$tag$`). The token's bounds are the lexer's, so a quote inside a comment or an
//! identifier never counts. Without tokens (no analysis yet, reduced mode, or
//! `FailClosedService`) a double click selects the class word, as before. Task 24 fills the
//! seam by serving its tokens; when the oracle reports kinds, [`string_content`] is the one
//! function to replace.

use std::ops::Range;

use hacivat_text::units::word_at;
use hacivat_text::{SelectionRange, TextBuffer, TokenOracle, UserEvent};
use hacivat_ui_core::gpui_surface::{Context, Window};

use super::{Moment, message, say, select, words};
use crate::view::EditorView;

type Cx<'a> = Context<'a, EditorView>;

/// The longest string prefix looked at (`U&`), and the longest dollar tag, in bytes.
const MAX_PREFIX: usize = 2;
const MAX_TAG: usize = 64;

pub(super) fn select_all(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    let len = v.model.buffer().len_bytes();
    select(v, SelectionRange::new(0, len), UserEvent::Select, cx);
}

pub(super) fn escape(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    super::acted(v);
    let Some(collapsed) = v.model.selection().collapse() else {
        // Nothing to collapse: not consumed. The next Tab leaves the editor.
        v.commands.released = Some(Moment::of(v));
        return;
    };
    select(v, *collapsed.main(), UserEvent::Select, cx);
}

/// The last Escape had nothing to collapse and nothing happened since: it was not consumed (the
/// host may pass it on), and the next Tab or Shift+Tab moves the focus out of the editor.
pub fn escape_released(v: &EditorView) -> bool {
    v.commands.released == Some(Moment::of(v))
}

/// Tab and Shift+Tab move the focus: the Ctrl+M mode, or an Escape's release.
pub(super) fn tab_leaves(v: &EditorView) -> bool {
    v.commands.tab_moves_focus || escape_released(v)
}

pub(super) fn toggle_tab_moves_focus(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    super::acted(v);
    v.commands.tab_moves_focus = !v.commands.tab_moves_focus;
    let id = if v.commands.tab_moves_focus {
        "hv-editor-tab-moves-focus-on"
    } else {
        "hv-editor-tab-moves-focus-off"
    };
    let text = message(cx, id, &[]);
    say(v, text, cx);
}

/// The content of the string literal `token` (its bytes without the quotes), when `token` is
/// one (module docs); `None` for anything else, and for an empty string.
fn string_content(buf: &TextBuffer, token: &Range<usize>) -> Option<Range<usize>> {
    // Only the token's edges are read, never its whole text.
    let head_end = buf.snap(
        (token.start + MAX_PREFIX + MAX_TAG + 2).min(token.end),
        hacivat_text::Bias::Before,
    );
    let head = buf.slice(token.start..head_end.max(token.start));
    let (open, close) = if let Some(rest) = head.strip_prefix('$') {
        // `$tag$ … $tag$`: the closing delimiter is the opening one.
        let tag = rest.find('$')? + 2;
        (tag, tag)
    } else {
        let quote = head.find('\'')?;
        let prefix = &head[..quote];
        let known = prefix.len() <= MAX_PREFIX
            && prefix.chars().all(|c| c.is_ascii_alphabetic() || c == '&');
        if !known {
            return None;
        }
        (quote + 1, 1)
    };
    if token.len() < open + close {
        return None;
    }
    let (start, end) = (token.start + open, token.end - close);
    if !buf.is_char_boundary(end) || buf.slice(end..token.end) != head[open - close..open] {
        return None;
    }
    (start < end).then_some(start..end)
}

/// What a double click at `at` selects: the word (`units::word_at`), or, inside a string
/// literal the service's tokens show, the literal's content. `previous` is the content the last
/// double click selected: a second one in the same string selects it with its quotes.
pub fn double_click_range(
    buf: &TextBuffer,
    at: usize,
    tok: &dyn TokenOracle,
    previous: Option<Range<usize>>,
) -> Range<usize> {
    let word = word_at(buf, at, tok);
    // A string only when the word is one of the service's tokens (never a class run that
    // happens to look like one).
    let body = buf.line_range(buf.byte_to_line(word.start), false);
    let is_token = tok
        .words_on_line(body)
        .is_some_and(|words| words.contains(&word));
    if !is_token {
        return word;
    }
    match string_content(buf, &word) {
        Some(content)
            if previous.as_ref() != Some(&content) && content.start <= at && at <= content.end =>
        {
            content
        }
        _ => word,
    }
}

/// The double click's range in `v`, remembering a string's content for the second click.
pub(crate) fn double_click(v: &mut EditorView, at: usize) -> Range<usize> {
    let version = v.model.version();
    let previous = v
        .commands
        .string_content
        .take()
        .filter(|(at_version, _)| *at_version == version)
        .map(|(_, content)| content);
    let buf = v.model.buffer();
    let range = double_click_range(buf, at, words(v).as_ref(), previous);
    let whole = word_at(buf, at, words(v).as_ref());
    if range != whole {
        v.commands.string_content = Some((version, range.clone()));
    }
    range
}
