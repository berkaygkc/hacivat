//! The read-only explanation popover (NOTES M2b surface 6, B21; Task 22): opened from the
//! capsule's access segment, the sheet's Access value and the palette command "Explain
//! read-only access".
//!
//! - **Content.** Enforced: the title, why, and the role probe's findings with the TEMP note
//!   when TEMP is the only one (Q21). Guarded: why, the findings ("Found on this role:", one
//!   `hv-conn-ro-warn-*` line per `RoleWarning`), the limit, the fix line and the SQL that
//!   `remediation_for_server` returns, in a mono inset with Copy SQL, which copies that string
//!   unchanged. MariaDB: why and the findings, and no SQL (`remediation` is `None`). SQLite: no
//!   roles, no findings, no SQL.
//! - **SQL inset.** One Fragment Mono 11.5/17 block per line, 4 px apart. A line wraps at its
//!   words with a 2ch hanging indent. Keywords take the syntax keyword colour, strings the
//!   string colour; placeholders (`<role>`, `<password>`, `<database>`, PostgreSQL 13's
//!   `<schema>`: any `<word>`) are italic text-2 (Raised Ink: never text-3 on the raised card),
//!   and `-- optional:` comment lines are italic in the comment colour.
//! - **Place.** 300 px wide, below its trigger, or above it when that fits better; it never
//!   crosses the title bar or the status bar (6 px clear of each). Beyond that height the body
//!   scrolls (4 px seam-strong thumb), the title stays on top and Copy SQL at the bottom.
//! - **Keyboard.** The popover takes the focus on its first frame (it is a dialog labelled by
//!   its title); Tab and Shift-Tab move between it and Copy SQL; Escape (the workspace) or a
//!   press outside closes it, and the focus goes back to the trigger (the layer's restore
//!   focus).
//!
//! Debug selectors: `access-popover`, `access-popover-thumb`; the Copy SQL button is
//! `button-ro-copy-snippet`.

use std::cell::Cell;
use std::rc::Rc;

use hacivat_l10n::{NoArgs, Text, ids};
use hacivat_proto::{AccessLabel, DbmsKind, EnvTag, ProfileId};
use hacivat_session::profile::Profile;
use hacivat_session::session::readonly::remediation_for_server;
use hacivat_session::{LiveInfo, LiveState, RoleWarning};
use hacivat_ui::button::{Button, ButtonKind};
use hacivat_ui::icon::{Icon, IconName, IconSize};
use hacivat_ui::popover::A11yNode;
use hacivat_ui::text::{Label, TextSource, paragraph};
use hacivat_ui::toast::{Toast, ToastKind, ToastQueue};
use hacivat_ui_core::fonts::font_for;
use hacivat_ui_core::gpui_surface::{
    AnyElement, App, AppContext as _, Bounds, ClipboardItem, Context, FocusHandle,
    InteractiveElement, IntoElement, KeyDownEvent, MouseButton, ParentElement, Pixels, Render,
    Role, ScrollHandle, SharedString, StatefulInteractiveElement as _, Styled, Subscription,
    Window, canvas, div, px,
};
use hacivat_ui_core::theme::{Theme, ThemeMode, TypeStyle, popover_surface};

use crate::connections::capsule::POPOVER_LAYER;
use crate::connections::model::ConnectionsModel;
use crate::l10n_global::{text_fn, tr};
use crate::shell::{Layer, POPOVER_CONTEXT, Workspace};

/// The card's width (NOTES: 300 px).
const WIDTH_PX: f32 = 300.0;
/// The clearance from the trigger and from the title and status bars.
const CLEAR_PX: f32 = 6.0;
/// The SQL inset's line height (NOTES: Fragment Mono 11.5/17).
const SQL_LINE_PX: f32 = 17.0;
/// The gap between two SQL lines (NOTES: 4 px).
const SQL_GAP_PX: f32 = 4.0;
/// The square bullet of a finding (the sticky-toast list anatomy: 3 px, text-2).
const BULLET_PX: f32 = 3.0;
/// The gap between a finding's bullet and its text (mock `.warns li { gap: 7px }`).
const FINDING_GAP_PX: f32 = 7.0;
/// A finding's line height.
const FINDING_LINE_PX: f32 = 17.0;
/// The scroll thumb's width and shortest length (M2a's 4 px seam-strong thumb).
const THUMB_PX: f32 = 4.0;
const THUMB_MIN_PX: f32 = 16.0;

/// SQL words drawn in the syntax keyword colour.
const KEYWORDS: &[&str] = &[
    "ALL",
    "ALTER",
    "BY",
    "CREATE",
    "DEFAULT",
    "FROM",
    "GRANT",
    "IDENTIFIED",
    "IN",
    "LOGIN",
    "ON",
    "PASSWORD",
    "PRIVILEGES",
    "REVOKE",
    "ROLE",
    "SCHEMA",
    "SELECT",
    "SEQUENCES",
    "SHOW",
    "TABLES",
    "TO",
    "USAGE",
    "USER",
    "VIEW",
];

/// What the popover says about one connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessFacts {
    pub access: AccessLabel,
    /// The engine the server runs (a MySQL profile on a MariaDB server is MariaDB).
    pub engine: DbmsKind,
    /// The role the connection signs in as.
    pub user: String,
    pub warnings: Vec<RoleWarning>,
    /// `remediation_for_server`: the SQL of a least-privilege role, if one would make the
    /// label enforced.
    pub remediation: Option<&'static str>,
    pub production: bool,
}

impl AccessFacts {
    /// The facts of `p`'s live connection `info`.
    pub fn of(p: &Profile, info: &LiveInfo) -> AccessFacts {
        AccessFacts {
            access: info.access,
            engine: info.server.kind,
            user: info
                .user
                .clone()
                .or_else(|| p.user.clone())
                .unwrap_or_default(),
            warnings: info.access_warnings.clone(),
            remediation: remediation_for_server(&info.server),
            production: p.environment == Some(EnvTag::Production),
        }
    }
}

/// The facts of profile `id`, while it is connected read-only.
pub fn facts_for(cx: &App, id: ProfileId) -> Option<AccessFacts> {
    let m = cx.try_global::<ConnectionsModel>()?;
    let p = m.get(id)?;
    match m.live(id) {
        LiveState::Connected(info) if info.access != AccessLabel::ReadWrite => {
            let mut facts = AccessFacts::of(&p, &info);
            // Fix round 1 (M8): a gap that changed data leaves "enforced" behind.
            facts.access =
                crate::connections::session_toasts::effective_access(cx, id, facts.access);
            Some(facts)
        }
        _ => None,
    }
}

/// One block of the popover's body.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Block {
    /// A paragraph (12.5 px, text-2).
    Text(SharedString),
    /// The findings: the lead, then one line each.
    Findings(Vec<SharedString>),
}

/// The words a finding names (its `hv-conn-ro-warn-*` message).
fn warning_text(cx: &App, w: &RoleWarning) -> SharedString {
    let id = match w {
        RoleWarning::Superuser => ids::hv::CONN_RO_WARN_SUPERUSER,
        RoleWarning::BypassRls => ids::hv::CONN_RO_WARN_BYPASS_RLS,
        RoleWarning::Replication => ids::hv::CONN_RO_WARN_REPLICATION,
        RoleWarning::CreateRole => ids::hv::CONN_RO_WARN_CREATE_ROLE,
        RoleWarning::ServerFileRoles => ids::hv::CONN_RO_WARN_SERVER_FILE_ROLES,
        RoleWarning::ExecuteServerProgram => ids::hv::CONN_RO_WARN_EXECUTE_SERVER_PROGRAM,
        RoleWarning::SignalBackend => ids::hv::CONN_RO_WARN_SIGNAL_BACKEND,
        RoleWarning::WritePrivileges => ids::hv::CONN_RO_WARN_WRITE_PRIVILEGES,
        RoleWarning::TempPrivilege => ids::hv::CONN_RO_WARN_TEMP_PRIVILEGE,
        RoleWarning::RoleGrantNotExpanded => ids::hv::CONN_RO_WARN_ROLE_GRANT_NOT_EXPANDED,
        RoleWarning::Unparsed => ids::hv::CONN_RO_WARN_UNPARSED,
        // ## m2b-t23-f1: PostgreSQL row 6 (`pg_logical_emit_message` and the like).
        RoleWarning::NonTransactionalWrite => ids::hv::CONN_RO_WARN_NON_TRANSACTIONAL_WRITE,
        RoleWarning::EgressExtension(name) => {
            return tr(
                cx,
                ids::hv::CONN_RO_WARN_EGRESS_EXTENSION,
                ids::hv::HvConnRoWarnEgressExtension {
                    name: Text(name.clone()),
                },
            );
        }
    };
    tr(cx, id, NoArgs)
}

/// The popover's title, body and SQL for `f` (the mock's `roPopHTML`).
fn content(cx: &App, f: &AccessFacts) -> (SharedString, Vec<Block>, Option<&'static str>) {
    let user = || Text(f.user.clone());
    let findings = || {
        (!f.warnings.is_empty())
            .then(|| Block::Findings(f.warnings.iter().map(|w| warning_text(cx, w)).collect()))
    };
    let text = |id| Block::Text(tr(cx, id, NoArgs));
    if f.access == AccessLabel::ReadOnlyEnforced {
        let mut body = vec![Block::Text(tr(
            cx,
            ids::hv::CONN_RO_POP_ENF_BODY,
            ids::hv::HvConnRoPopEnfBody { user: user() },
        ))];
        body.extend(findings());
        // The "stays enforced" reassurance lives only here (Q21, mock N1).
        if f.warnings.contains(&RoleWarning::TempPrivilege) {
            body.push(text(ids::hv::CONN_RO_POP_ENF_TEMP));
        }
        return (tr(cx, ids::hv::CONN_RO_POP_ENF_TITLE, NoArgs), body, None);
    }
    let title = tr(cx, ids::hv::CONN_RO_POP_G_TITLE, NoArgs);
    match f.engine {
        DbmsKind::Sqlite => (title, vec![text(ids::hv::CONN_RO_POP_SQLITE_BODY)], None),
        DbmsKind::MariaDb => {
            let mut body = vec![text(ids::hv::CONN_RO_POP_MARIA_BODY)];
            body.extend(findings());
            body.push(text(ids::hv::CONN_RO_POP_NO_FIX));
            (title, body, None)
        }
        _ => {
            let mut body = vec![Block::Text(tr(
                cx,
                ids::hv::CONN_RO_POP_G_BODY,
                ids::hv::HvConnRoPopGBody { user: user() },
            ))];
            body.extend(findings());
            body.push(text(ids::hv::CONN_RO_POP_G_LIMIT));
            match f.remediation {
                Some(sql) => {
                    body.push(text(ids::hv::CONN_RO_POP_G_FIX));
                    (title, body, Some(sql))
                }
                // No role change makes the label enforced on this engine.
                None => {
                    body.push(text(ids::hv::CONN_RO_POP_NO_FIX));
                    (title, body, None)
                }
            }
        }
    }
}

/// How a piece of SQL is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ink {
    Plain,
    Keyword,
    Str,
    Placeholder,
    Comment,
}

/// One run of one ink inside a word.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Piece {
    text: String,
    ink: Ink,
}

/// A SQL line as words (split at whitespace, where it may wrap), each a run of inked pieces.
fn sql_words(line: &str) -> Vec<Vec<Piece>> {
    let comment = line.trim_start().starts_with("--");
    let chars: Vec<char> = line.chars().collect();
    let mut inks = vec![if comment { Ink::Comment } else { Ink::Plain }; chars.len()];
    if !comment {
        // Strings, quotes included.
        let mut in_string = false;
        for (i, c) in chars.iter().enumerate() {
            if *c == '\'' {
                inks[i] = Ink::Str;
                in_string = !in_string;
            } else if in_string {
                inks[i] = Ink::Str;
            }
        }
        // Placeholders: `<` word `>`, inside strings too.
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '<' {
                let end = chars[i + 1..]
                    .iter()
                    .position(|c| !(c.is_ascii_alphanumeric() || *c == '_'))
                    .map(|n| i + 1 + n);
                if let Some(end) = end
                    && end > i + 1
                    && chars[end] == '>'
                {
                    inks[i..=end].fill(Ink::Placeholder);
                    i = end + 1;
                    continue;
                }
            }
            i += 1;
        }
        // Keywords: whole ASCII words outside strings and placeholders.
        let mut i = 0;
        while i < chars.len() {
            let word = |c: char| c.is_ascii_alphanumeric() || c == '_';
            if inks[i] == Ink::Plain && word(chars[i]) {
                let end = (i..chars.len())
                    .find(|&j| !word(chars[j]) || inks[j] != Ink::Plain)
                    .unwrap_or(chars.len());
                let text: String = chars[i..end].iter().collect();
                if KEYWORDS.contains(&text.as_str()) {
                    inks[i..end].fill(Ink::Keyword);
                }
                i = end;
            } else {
                i += 1;
            }
        }
    }
    let mut words: Vec<Vec<Piece>> = Vec::new();
    let mut current: Vec<Piece> = Vec::new();
    for (c, ink) in chars.into_iter().zip(inks) {
        if c.is_whitespace() {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            continue;
        }
        match current.last_mut() {
            Some(piece) if piece.ink == ink => piece.text.push(c),
            _ => current.push(Piece {
                text: c.to_string(),
                ink,
            }),
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

/// The read-only explanation, a non-modal layer (see the module docs).
pub struct AccessPopover {
    // ## fix round 1 (M10): it follows its connection.
    profile: ProfileId,
    facts: AccessFacts,
    _follow: Subscription,
    title: SharedString,
    body: Vec<Block>,
    sql: Option<&'static str>,
    production: bool,
    /// The trigger's bounds, in window coordinates.
    trigger: Bounds<Pixels>,
    focus: FocusHandle,
    copy_focus: FocusHandle,
    /// Whether the popover has taken the focus yet (on its first frame).
    opened: bool,
    scroll: ScrollHandle,
    /// The card's height as last laid out: the placement reads it (below or above).
    height: Rc<Cell<Option<Pixels>>>,
}

/// Opens the explanation of profile `id`'s access under `trigger` (window coordinates). The
/// focused element when it opens (the trigger) gets the focus back when it closes. Nothing
/// opens unless the profile is connected read-only.
pub fn open(id: ProfileId, trigger: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
    let Some(facts) = facts_for(cx, id) else {
        return;
    };
    let (title, body, sql) = content(cx, &facts);
    let view = cx.new(|cx| AccessPopover {
        profile: id,
        facts: facts.clone(),
        _follow: cx.observe_global::<ConnectionsModel>(|_, cx| cx.notify()),
        title,
        body,
        sql,
        production: facts.production,
        trigger,
        focus: cx.focus_handle(),
        copy_focus: cx.focus_handle(),
        opened: false,
        scroll: ScrollHandle::new(),
        height: Rc::new(Cell::new(None)),
    });
    let Some(ws) = window.root::<Workspace>().flatten() else {
        return;
    };
    ws.update(cx, |ws, cx| {
        // Fix round 1 (M9): one explanation at a time; the trigger keeps the focus to restore.
        if ws.layer_ids().last() == Some(&POPOVER_LAYER) {
            ws.pop_layer(window, cx);
        }
        ws.push_layer(
            Layer {
                id: POPOVER_LAYER,
                context: POPOVER_CONTEXT,
                view: view.clone().into(),
                restore_focus: None,
                modal: false,
            },
            window,
            cx,
        );
    });
    // ## m2b-t24-shots: the triggers (capsule segment, sheet value) say it is open.
    crate::connections::capsule::note_expanded(
        hacivat_ui::capsule::CapsuleSegment::Access,
        &view,
        cx,
    );
}

impl AccessPopover {
    /// Closes this popover's layer (a press outside it); the focus goes back to the trigger.
    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let me = cx.entity().entity_id();
        window.defer(cx, move |window, cx| {
            if let Some(ws) = window.root::<Workspace>().flatten() {
                ws.update(cx, |ws, cx| ws.remove_layer(me, window, cx));
            }
        });
    }

    /// Tab and Shift-Tab move between the popover and Copy SQL (the workspace leaves Tab to
    /// an open layer).
    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let stroke = &event.keystroke;
        let m = &stroke.modifiers;
        if stroke.key != "tab" || m.control || m.alt || m.platform || m.function {
            return;
        }
        cx.stop_propagation();
        if self.sql.is_none() {
            window.focus(&self.focus, cx);
            return;
        }
        let next = if self.copy_focus.is_focused(window) {
            self.focus.clone()
        } else {
            self.copy_focus.clone()
        };
        window.focus(&next, cx);
        cx.notify();
    }

    /// Where the card goes (the mock's `placePop`): `CLEAR_PX` below the trigger, or above it
    /// when it does not fit below, always between the title bar and the status bar.
    fn top(&self, theme: &Theme, viewport_h: Pixels) -> (Pixels, Pixels) {
        let t = theme.tokens;
        let min_top = px(t.spacing.titlebar_height + CLEAR_PX);
        let max_bottom = viewport_h - px(t.spacing.statusbar_height + CLEAR_PX);
        let max_h = (max_bottom - min_top).max(px(0.0));
        let below = self.trigger.bottom() + px(CLEAR_PX);
        let top = match self.height.get() {
            None => below,
            Some(h) => {
                let h = h.min(max_h);
                let top = if below + h > max_bottom {
                    self.trigger.top() - h - px(CLEAR_PX)
                } else {
                    below
                };
                top.min(max_bottom - h)
            }
        };
        (top.max(min_top), max_h)
    }
}

/// The findings: the lead (12.5 px text-2), then one 12 px `text` line each with a 3 px
/// square text-2 bullet, 2 px apart (the sticky-toast list anatomy).
fn findings_block(theme: &Theme, lead: SharedString, lines: &[SharedString]) -> AnyElement {
    let t = theme.tokens;
    let c = &t.colors;
    // Fix round 1 (M2): 12 px on a whole-pixel 17 px line (mock `line-height: 1.4`), so a
    // wrapped finding's height never rounds below its lines.
    let style = TypeStyle {
        line_height: FINDING_LINE_PX,
        ..t.type_ramp.caption
    };
    let line_h = style.line_height;
    // Fix round 1 (M2): the text column's exact width (the card less its border, padding, the
    // bullet and the gap), so a line wraps at the width it is drawn in.
    let text_w = WIDTH_PX - 2.0 - 2.0 * t.spacing.xl - BULLET_PX - FINDING_GAP_PX;
    div()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .pb(px(t.spacing.sm))
        .child(paragraph(
            lead,
            TextSource::Fluent,
            t.type_ramp.body_sm,
            c.text_2.to_hsla(),
        ))
        .children(lines.iter().map(|line| {
            div()
                .flex()
                .flex_row()
                .items_start()
                .gap(px(FINDING_GAP_PX))
                .child(
                    div()
                        .mt(px((line_h - BULLET_PX) / 2.0))
                        .size(px(BULLET_PX))
                        .flex_none()
                        .bg(c.text_2.to_hsla()),
                )
                .child(div().w(px(text_w)).flex_none().child(paragraph(
                    line.clone(),
                    TextSource::Fluent,
                    style,
                    c.text.to_hsla(),
                )))
        }))
        .into_any_element()
}

/// The SQL inset (the fingerprint-inset anatomy): one block per line, wrapping at its words
/// with a 2ch hanging indent; `ch` is the mono face's advance.
fn sql_inset(theme: &Theme, sql: &str, ch: Pixels) -> AnyElement {
    let t = theme.tokens;
    let c = &t.colors;
    let style = TypeStyle {
        line_height: SQL_LINE_PX,
        ..t.type_ramp.mono_data
    };
    let fill = match theme.mode {
        ThemeMode::Dark => c.panel,
        ThemeMode::Light => c.ground,
    };
    let ink = |ink: Ink| match ink {
        Ink::Plain => c.text,
        Ink::Keyword => c.syn_keyword,
        Ink::Str => c.syn_string,
        // Raised Ink (Q34): placeholders are text-2 on the raised card, never text-3.
        Ink::Placeholder => c.text_2,
        Ink::Comment => c.syn_comment,
    };
    let lines = sql.lines().map(|line| {
        let words = sql_words(line).into_iter().enumerate().map(|(i, word)| {
            let mut w = div().flex().flex_row().flex_none();
            if i == 0 {
                // The first word starts at the line's edge; continuations hang by 2ch.
                w = w.ml(-ch * 2.0);
            }
            w.children(word.into_iter().map(|piece| {
                Label::new(
                    piece.text,
                    TextSource::Data,
                    style,
                    ink(piece.ink).to_hsla(),
                )
                .italic(matches!(piece.ink, Ink::Placeholder | Ink::Comment))
            }))
        });
        div()
            .flex()
            .flex_row()
            .flex_wrap()
            .pl(ch * 2.0)
            .gap_x(ch)
            .children(words)
    });
    div()
        .flex()
        .flex_col()
        .gap(px(SQL_GAP_PX))
        .my(px(t.spacing.sm))
        .py(px(t.spacing.md))
        .px(px(t.spacing.lg))
        .border_1()
        .border_color(c.seam_strong.to_hsla())
        .rounded(px(t.radii.sm))
        .bg(fill.to_hsla())
        .children(lines)
        .into_any_element()
}

/// The scroll thumb of the body while it overflows (as the sheet's): 4 px wide in the
/// card's right padding, its length and place in proportion.
fn thumb(theme: &Theme, scroll: &ScrollHandle) -> Option<AnyElement> {
    let max = f32::from(scroll.max_offset().y);
    let view = f32::from(scroll.bounds().size.height);
    if max < 0.5 || view <= 0.0 {
        return None;
    }
    let t = theme.tokens;
    let length = (view * view / (view + max)).max(THUMB_MIN_PX);
    let at = (-f32::from(scroll.offset().y) / max).clamp(0.0, 1.0) * (view - length);
    Some(
        div()
            .debug_selector(|| "access-popover-thumb".to_string())
            .absolute()
            .right(px(-8.0))
            .top(px(at))
            .w(px(THUMB_PX))
            .h(px(length))
            .rounded(px(t.radii.xs))
            .bg(t.colors.seam_strong.to_hsla())
            .into_any_element(),
    )
}

/// Copies `sql` exactly as `remediation_for_server` returned it, and says what to do next.
fn copy_sql(sql: &'static str, cx: &mut App) {
    cx.write_to_clipboard(ClipboardItem::new_string(sql.to_string()));
    ToastQueue::push(
        cx,
        Toast::new(ToastKind::Info, text_fn(ids::hv::CONN_RO_COPIED)),
    );
}

impl Render for AccessPopover {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Fix round 1 (M10): the connection is no longer live and read-only: nothing left to
        // explain; new facts (a reconnect, a downgraded label) are explained anew.
        match facts_for(cx, self.profile) {
            None => self.close(window, cx),
            Some(facts) if facts != self.facts => {
                let (title, body, sql) = content(cx, &facts);
                self.title = title;
                self.body = body;
                self.sql = sql;
                self.production = facts.production;
                self.facts = facts;
            }
            Some(_) => {}
        }
        if !self.opened {
            // Take the focus once the first frame is drawn: the dialog is announced by title.
            self.opened = true;
            let focus = self.focus.clone();
            window.defer(cx, move |window, cx| window.focus(&focus, cx));
        }
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let viewport = window.viewport_size();
        let (top, max_h) = self.top(&theme, viewport.height);
        let left = (self.trigger.left() - px(8.0))
            .min(viewport.width - px(WIDTH_PX + 12.0))
            .max(px(8.0));
        let mono = t.type_ramp.mono_data;
        let font_id = window.text_system().resolve_font(&font_for(&mono));
        let ch = window
            .text_system()
            .advance(font_id, px(mono.size_px), 'm')
            .map_or(px(mono.size_px * 0.6), |s| s.width);

        let lead = tr(cx, ids::hv::CONN_RO_WARN_HEAD, NoArgs);
        let mut body = div().flex().flex_col();
        for block in &self.body {
            body = body.child(match block {
                Block::Text(text) => div()
                    .pb(px(t.spacing.sm))
                    .child(paragraph(
                        text.clone(),
                        TextSource::Fluent,
                        t.type_ramp.body_sm,
                        c.text_2.to_hsla(),
                    ))
                    .into_any_element(),
                Block::Findings(lines) => findings_block(&theme, lead.clone(), lines),
            });
        }
        if let Some(sql) = self.sql {
            body = body.child(sql_inset(&theme, sql, ch));
        }
        // Hue Is Meaning: the lock is red only on production.
        let lock = if self.production { c.red } else { c.text };
        let title = div()
            .flex()
            .flex_row()
            .items_start()
            .flex_none()
            .gap(px(7.0))
            .mb(px(t.spacing.xs))
            .child(
                div()
                    .flex_none()
                    .mt(px((t.type_ramp.body_strong.line_height
                        - IconSize::Chrome.px())
                        / 2.0))
                    .child(
                        Icon::new(IconName::Lock)
                            .size(IconSize::Chrome)
                            .color(lock.to_hsla()),
                    ),
            )
            .child(div().flex_1().min_w(px(0.0)).child(paragraph(
                self.title.clone(),
                TextSource::Fluent,
                t.type_ramp.body_strong,
                c.text.to_hsla(),
            )));
        let scrolled = div()
            .relative()
            .flex_1()
            .min_h_0()
            .child(
                div()
                    .id("access-popover-body")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .child(hacivat_ui::text::scroll_viewport())
                    .child(body),
            )
            .children(thumb(&theme, &self.scroll));
        let mut card = popover_surface(&theme)
            .id("access-popover")
            .debug_selector(|| "access-popover".to_string())
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down_out(cx.listener(|this, _, window, cx| this.close(window, cx)))
            .occlude()
            .absolute()
            .left(left)
            .top(top)
            .w(px(WIDTH_PX))
            .max_h(max_h)
            .flex()
            .flex_col()
            .px(px(t.spacing.xl))
            .py(px(t.spacing.lg))
            .child(title)
            .child(scrolled);
        if let Some(sql) = self.sql {
            card = card.child(
                div()
                    .flex()
                    .flex_row()
                    .flex_none()
                    .justify_end()
                    .pt(px(t.spacing.xs))
                    .child(
                        Button::new("ro-copy-snippet", tr(cx, ids::hv::CONN_RO_COPY_SQL, NoArgs))
                            .kind(ButtonKind::Default)
                            .icon(IconName::Copy)
                            .on_raised(true)
                            .focus_handle(self.copy_focus.clone())
                            .on_click(move |_, _, cx| copy_sql(sql, cx)),
                    ),
            );
        }
        // The card's laid-out height: a card that does not fit below its trigger goes above it
        // on the next frame.
        let height = self.height.clone();
        let me = cx.entity().downgrade();
        card = card.child(
            canvas(
                move |bounds, _, cx| {
                    if height.get() != Some(bounds.size.height) {
                        height.set(Some(bounds.size.height));
                        if let Some(me) = me.upgrade() {
                            me.update(cx, |_, cx| cx.notify());
                        }
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        );
        let description: SharedString = self
            .body
            .iter()
            .map(|b| match b {
                Block::Text(text) => text.to_string(),
                Block::Findings(lines) => lines
                    .iter()
                    .map(SharedString::to_string)
                    .collect::<Vec<_>>()
                    .join(" "),
            })
            .collect::<Vec<_>>()
            .join(" ")
            .into();
        div().size_full().child(
            A11yNode::new(
                "access-popover-node",
                Role::Dialog,
                self.title.clone(),
                false,
                card,
            )
            .description(description),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inks(line: &str) -> Vec<(String, Ink)> {
        sql_words(line)
            .into_iter()
            .flatten()
            .map(|p| (p.text, p.ink))
            .collect()
    }

    #[test]
    fn placeholders_strings_keywords_and_comments_are_inked() {
        let s = |text: &str, ink| (text.to_string(), ink);
        assert_eq!(
            inks("CREATE USER '<role>'@'%' IDENTIFIED BY '<password>';"),
            [
                s("CREATE", Ink::Keyword),
                s("USER", Ink::Keyword),
                s("'", Ink::Str),
                s("<role>", Ink::Placeholder),
                s("'", Ink::Str),
                s("@", Ink::Plain),
                s("'%'", Ink::Str),
                s("IDENTIFIED", Ink::Keyword),
                s("BY", Ink::Keyword),
                s("'", Ink::Str),
                s("<password>", Ink::Placeholder),
                s("'", Ink::Str),
                s(";", Ink::Plain),
            ]
        );
        assert!(
            inks("-- optional: REVOKE TEMP ON DATABASE <database> FROM PUBLIC;")
                .iter()
                .all(|(_, ink)| *ink == Ink::Comment)
        );
        // PostgreSQL 13's form (the T5 carry): `<schema>` is a placeholder like `<role>`.
        let pg13 = inks("GRANT USAGE ON SCHEMA <schema> TO <role>;");
        assert!(pg13.contains(&s("<schema>", Ink::Placeholder)));
        assert!(pg13.contains(&s("<role>", Ink::Placeholder)));
        assert!(pg13.contains(&s("USAGE", Ink::Keyword)));
        // An identifier is never a keyword, whatever it contains.
        assert!(
            inks("GRANT pg_read_all_data TO <role>;").contains(&s("pg_read_all_data", Ink::Plain))
        );
    }

    #[test]
    fn the_words_rebuild_every_remediation_line() {
        // Fix round 1 (M13): PostgreSQL 13's schema-grant form too.
        let pg13 = remediation_for_server(&hacivat_driver::ServerInfo {
            kind: DbmsKind::Postgres,
            product: "PostgreSQL".into(),
            version: hacivat_driver::SemVerLoose::parse("13.12"),
            raw_version: "13.12".into(),
            is_replica: Some(false),
        });
        assert!(pg13.is_some_and(|s| s.contains("<schema>")));
        // ## m2b-t23-f1: 14 to 16 name the three-argument `pg_logical_emit_message`, 17 and
        // later (`remediation`) the four-argument one.
        let pg16 = remediation_for_server(&hacivat_driver::ServerInfo {
            kind: DbmsKind::Postgres,
            product: "PostgreSQL".into(),
            version: hacivat_driver::SemVerLoose::parse("16.4"),
            raw_version: "16.4".into(),
            is_replica: Some(false),
        });
        assert!(pg16.is_some_and(|s| s.contains("pg_logical_emit_message(boolean, text, text)")));
        for sql in [
            hacivat_session::remediation(DbmsKind::Postgres),
            hacivat_session::remediation(DbmsKind::MySql),
            pg13,
            pg16,
        ] {
            for line in sql.expect("a remediation").lines() {
                let words: Vec<String> = sql_words(line)
                    .into_iter()
                    .map(|w| w.into_iter().map(|p| p.text).collect())
                    .collect();
                assert_eq!(words.join(" "), line);
            }
        }
    }
}
