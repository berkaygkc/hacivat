//! Rendering a template for display, copy and the driver, and rewriting its endpoint
//! (HV-DRV-03).
//!
//! Render rules:
//! - A named placeholder takes the user value, else its default. Outside an optional group a
//!   field with neither is `MissingField`, never a half URL. An empty value counts as missing.
//! - An optional group renders its first alternative that carries a user value (a default alone
//!   does not count, and neither does literal text); otherwise nothing.
//! - A list renders one entry per host for a host item (`FieldMap::hosts`, else the single
//!   host and port), one entry per extra parameter for the pass-through item, and one entry for
//!   any other item with a user value, joined by its separator.
//! - `%` escaping encodes `@ / ? # % : [ ] ;`, space, controls and every non-ASCII byte, plus
//!   what ends the type (`,` for a host, `&` for a parameter value, `& = ,` for a key). A host
//!   with a `:` is an IPv6 literal and is bracketed instead.
//! - The password never comes from the `FieldMap`: Display shows `****` when the profile has
//!   one, Copy leaves it out (its group collapses), and only `render_driver` writes it.
//!
//! Every buffer that may hold the password is a `Zeroizing<String>` reserved up front, so it is
//! wiped on drop and does not reallocate (and leave a copy behind) as it grows.

use std::borrow::Cow;
use std::ops::Range;

use percent_encoding::utf8_percent_encode;
use secrecy::zeroize::Zeroizing;
use secrecy::{ExposeSecret, SecretString};

use super::matcher::match_spans;
use super::template::{Escape, Node, PhType, Placeholder};
use super::{FieldId, FieldMap, Template};

/// Who a rendered URL is for (HV-DRV-03). Display and Copy never contain the password.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RenderTarget {
    /// The URL row; `has_password` shows a mask where the password goes.
    Display {
        has_password: bool,
    },
    Copy,
}

/// Why a template could not be rendered (HV-DRV-03).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RenderError {
    MissingField(FieldId),
    /// The map has a value for a field this template cannot carry.
    FieldNotAllowed(FieldId),
    /// The endpoint cannot be rewritten; the reason is a stable ID (`no_match`, `no_host`,
    /// `multiple_hosts`, `no_port`, `multiple_ports`).
    CannotRedirect(String),
}

/// Renders `t` with `fields` and `extra` parameters for `target`.
pub fn render(
    t: &Template,
    fields: &FieldMap,
    extra: &[(String, String)],
    target: RenderTarget,
) -> Result<String, RenderError> {
    let secret = match target {
        RenderTarget::Display { has_password: true } => Secret::Mask,
        RenderTarget::Display {
            has_password: false,
        }
        | RenderTarget::Copy => Secret::Omit,
    };
    let out = render_buf(t, fields, extra, secret)?;
    Ok(out.as_str().to_string())
}

/// Renders the driver URL, password included, as a secret. The intermediate buffers are wiped.
pub fn render_driver(
    t: &Template,
    fields: &FieldMap,
    extra: &[(String, String)],
    password: &SecretString,
) -> Result<SecretString, RenderError> {
    let out = render_buf(t, fields, extra, Secret::Value(password.expose_secret()))?;
    Ok(SecretString::from(out.as_str()))
}

/// `url` pointed at `host:port` instead (a forwarder). The URL is matched against `t` and
/// exactly the host and port spans are replaced, so query parameters named `host` or `port`
/// are untouched. A URL without a port gets one only where the template has an optional
/// `[:{port}]` right after the host. Anything ambiguous is refused.
pub fn rewrite_endpoint(
    t: &Template,
    url: &str,
    host: &str,
    port: u16,
) -> Result<String, RenderError> {
    let refuse = |why: &str| RenderError::CannotRedirect(why.to_string());
    let spans = match_spans(t, url).ok_or_else(|| refuse("no_match"))?;
    let mut hosts: Vec<Range<usize>> = Vec::new();
    let mut ports: Vec<Range<usize>> = Vec::new();
    for (f, r) in &spans.named {
        match f {
            FieldId::Host => hosts.push(r.clone()),
            FieldId::Port => ports.push(r.clone()),
            _ => {}
        }
    }
    for (h, p) in &spans.hosts {
        hosts.push(h.clone());
        ports.extend(p.clone());
    }
    let host_span = match hosts.as_slice() {
        [] => return Err(refuse("no_host")),
        [one] => one.clone(),
        _ => return Err(refuse("multiple_hosts")),
    };
    if !t.field_set.contains(&FieldId::Port) {
        return Err(refuse("no_port"));
    }
    let new_host = if host.contains(':') {
        format!("[{host}]")
    } else {
        utf8_percent_encode(host, PhType::Host.encode_set()).to_string()
    };
    let new_port = port.to_string();
    // Edits as (span, text), applied from the end so earlier offsets stay valid.
    let mut edits = vec![(host_span.clone(), new_host)];
    match ports.as_slice() {
        [] if port_follows_host(&t.nodes) => {
            edits.push((host_span.end..host_span.end, format!(":{new_port}")));
        }
        [] => return Err(refuse("no_port")),
        [one] => edits.push((one.clone(), new_port)),
        _ => return Err(refuse("multiple_ports")),
    }
    edits.sort_by_key(|(r, _)| std::cmp::Reverse(r.start));
    let mut out = url.to_string();
    for (r, text) in edits {
        out.replace_range(r, &text);
    }
    Ok(out)
}

/// Some sequence has a host placeholder followed by the optional group `[:{port}]`.
fn port_follows_host(nodes: &[Node]) -> bool {
    let is_port_group = |n: &Node| match n {
        Node::Opt(alts) => matches!(
            alts.as_slice(),
            [alt] if matches!(
                alt.as_slice(),
                [Node::Lit(colon), Node::Ph(Placeholder { name: Some(FieldId::Port), .. })]
                    if colon == ":"
            )
        ),
        _ => false,
    };
    nodes.windows(2).any(|w| {
        matches!(
            &w[0],
            Node::Ph(Placeholder {
                name: Some(FieldId::Host),
                ..
            })
        ) && is_port_group(&w[1])
    }) || nodes.iter().any(|n| match n {
        Node::Opt(alts) | Node::List { items: alts, .. } => {
            alts.iter().any(|s| port_follows_host(s))
        }
        Node::MaybeEmpty(inner) => port_follows_host(std::slice::from_ref(inner.as_ref())),
        _ => false,
    })
}

type Buf = Zeroizing<String>;

/// Where the password comes from.
#[derive(Clone, Copy)]
enum Secret<'a> {
    Omit,
    Mask,
    Value(&'a str),
}

/// A value to write: text to escape, or the mask, written as is.
enum Val<'a> {
    Text(Cow<'a, str>),
    Mask,
}

/// What a rendered part wrote: a user value (`strong`), or only the password mask (`weak`).
/// The mask alone selects a group only when the password is the group's only field, so
/// Display drops `[{user}[:{password}]@]` entirely when there is no user, like Copy does.
#[derive(Clone, Copy, Default)]
struct Wrote {
    strong: bool,
    weak: bool,
}

impl Wrote {
    const NONE: Wrote = Wrote {
        strong: false,
        weak: false,
    };
    const STRONG: Wrote = Wrote {
        strong: true,
        weak: false,
    };
    const WEAK: Wrote = Wrote {
        strong: false,
        weak: true,
    };

    fn any(self) -> bool {
        self.strong || self.weak
    }

    /// Whether this selects the group alternative `alt`.
    fn selects(self, alt: &[Node]) -> bool {
        self.strong || (self.weak && !names_other_than_password(alt))
    }
}

impl std::ops::BitOrAssign for Wrote {
    fn bitor_assign(&mut self, rhs: Wrote) {
        self.strong |= rhs.strong;
        self.weak |= rhs.weak;
    }
}

fn names_other_than_password(nodes: &[Node]) -> bool {
    FieldId::ALL
        .into_iter()
        .filter(|f| *f != FieldId::Password)
        .any(|f| names(nodes, f))
}

struct Ctx<'a> {
    fields: &'a FieldMap,
    extra: &'a [(String, String)],
    secret: Secret<'a>,
    /// Every buffer's reserved capacity.
    cap: usize,
}

/// What the nodes being rendered are inside of.
#[derive(Clone, Copy, Default)]
struct Scope<'a> {
    /// Inside an optional group: a missing field renders nothing instead of failing.
    optional: bool,
    /// A host-list entry: the host and port to use.
    host: Option<(&'a str, Option<&'a str>)>,
    /// A pass-through entry: the key, then the value, for the anonymous placeholders.
    pair: Option<(&'a str, &'a str)>,
}

impl<'a> Ctx<'a> {
    fn buf(&self) -> Buf {
        Zeroizing::new(String::with_capacity(self.cap))
    }

    fn value(&self, f: FieldId, scope: Scope<'a>) -> Option<Val<'a>> {
        let non_empty = |s: &'a str| Some(s).filter(|s| !s.is_empty());
        match (f, scope.host) {
            (FieldId::Password, _) => match self.secret {
                Secret::Omit => None,
                Secret::Mask => Some(Val::Mask),
                Secret::Value(v) => non_empty(v).map(|v| Val::Text(Cow::Borrowed(v))),
            },
            (FieldId::Host, Some((h, _))) => non_empty(h).map(|h| Val::Text(Cow::Borrowed(h))),
            (FieldId::Port, Some((_, p))) => p.and_then(non_empty).map(|p| Val::Text(p.into())),
            _ => self
                .fields
                .values
                .get(&f)
                .and_then(|v| non_empty(v.as_str()))
                .map(|v| Val::Text(Cow::Borrowed(v))),
        }
    }
}

fn render_buf(
    t: &Template,
    fields: &FieldMap,
    extra: &[(String, String)],
    secret: Secret<'_>,
) -> Result<Buf, RenderError> {
    for (f, v) in &fields.values {
        if *f != FieldId::Password && !v.is_empty() && !t.field_set.contains(f) {
            return Err(RenderError::FieldNotAllowed(*f));
        }
    }
    if !fields.hosts.is_empty() && !t.field_set.contains(&FieldId::Host) {
        return Err(RenderError::FieldNotAllowed(FieldId::Host));
    }
    let secret_len = match secret {
        Secret::Value(v) => v.len(),
        Secret::Mask | Secret::Omit => 4,
    };
    let value_bytes: usize = fields.values.values().map(String::len).sum::<usize>()
        + fields.hosts.iter().map(|(h, _)| h.len() + 6).sum::<usize>()
        + extra.iter().map(|(k, v)| k.len() + v.len()).sum::<usize>()
        + secret_len;
    let entries = 1 + fields.hosts.len() + extra.len();
    let ctx = Ctx {
        fields,
        extra,
        secret,
        // Encoding at most triples a byte; literals repeat once per list entry.
        cap: (literal_bytes(&t.nodes) + 16) * entries + 3 * value_bytes + 64,
    };
    let mut out = ctx.buf();
    let mut anon = 0;
    render_seq(&t.nodes, &mut out, &ctx, Scope::default(), &mut anon)?;
    Ok(out)
}

fn literal_bytes(nodes: &[Node]) -> usize {
    nodes
        .iter()
        .map(|n| match n {
            Node::Lit(s) | Node::Scheme(s) => s.len(),
            Node::List { sep, items } => {
                sep.len() + items.iter().map(|i| literal_bytes(i)).sum::<usize>()
            }
            Node::Opt(alts) => alts.iter().map(|a| literal_bytes(a)).sum(),
            Node::MaybeEmpty(inner) => literal_bytes(std::slice::from_ref(inner.as_ref())),
            Node::Ph(p) => p.default.as_ref().map_or(0, |d| 3 * d.len()),
            Node::NotAhead(_) => 0,
        })
        .sum()
}

/// Renders `nodes`, reporting whether a user value (or list entry) was written.
fn render_seq<'a>(
    nodes: &'a [Node],
    out: &mut Buf,
    ctx: &Ctx<'a>,
    scope: Scope<'a>,
    anon: &mut usize,
) -> Result<Wrote, RenderError> {
    let mut wrote = Wrote::NONE;
    for n in nodes {
        wrote |= render_node(n, out, ctx, scope, anon)?;
    }
    Ok(wrote)
}

fn render_node<'a>(
    node: &'a Node,
    out: &mut Buf,
    ctx: &Ctx<'a>,
    scope: Scope<'a>,
    anon: &mut usize,
) -> Result<Wrote, RenderError> {
    match node {
        Node::Lit(s) | Node::Scheme(s) => {
            out.push_str(s);
            Ok(Wrote::NONE)
        }
        Node::NotAhead(_) => Ok(Wrote::NONE),
        Node::MaybeEmpty(inner) => render_node(
            inner,
            out,
            ctx,
            Scope {
                optional: true,
                ..scope
            },
            anon,
        ),
        Node::Ph(p) => render_ph(p, out, ctx, scope, anon),
        Node::Opt(alts) => {
            let inner = Scope {
                optional: true,
                ..scope
            };
            for alt in alts {
                let mut tmp = ctx.buf();
                let mut a = *anon;
                let wrote = render_seq(alt, &mut tmp, ctx, inner, &mut a)?;
                if wrote.selects(alt) {
                    out.push_str(&tmp);
                    *anon = a;
                    return Ok(wrote);
                }
            }
            Ok(Wrote::NONE)
        }
        Node::List { sep, items } => {
            let entries = render_list(items, ctx, scope)?;
            for (i, e) in entries.iter().enumerate() {
                if i > 0 {
                    out.push_str(sep);
                }
                out.push_str(e);
            }
            Ok(if entries.is_empty() {
                Wrote::NONE
            } else {
                Wrote::STRONG
            })
        }
    }
}

fn render_ph<'a>(
    p: &'a Placeholder,
    out: &mut Buf,
    ctx: &Ctx<'a>,
    scope: Scope<'a>,
    anon: &mut usize,
) -> Result<Wrote, RenderError> {
    let Some(f) = p.name else {
        // A pass-through key, then value.
        if let Some((k, v)) = scope.pair {
            let text = if *anon == 0 { k } else { v };
            *anon += 1;
            write_value(out, p, text);
            return Ok(Wrote::STRONG);
        }
        if let Some(d) = &p.default {
            write_value(out, p, d);
        }
        return Ok(Wrote::NONE);
    };
    match ctx.value(f, scope) {
        Some(Val::Mask) => {
            out.push_str("****");
            Ok(Wrote::WEAK)
        }
        Some(Val::Text(v)) => {
            write_value(out, p, &v);
            Ok(Wrote::STRONG)
        }
        None => match &p.default {
            Some(d) => {
                write_value(out, p, d);
                Ok(Wrote::NONE)
            }
            None if scope.optional => Ok(Wrote::NONE),
            None => Err(RenderError::MissingField(f)),
        },
    }
}

fn render_list<'a>(
    items: &'a [Vec<Node>],
    ctx: &Ctx<'a>,
    scope: Scope<'a>,
) -> Result<Vec<Buf>, RenderError> {
    let inner = Scope {
        optional: true,
        ..scope
    };
    let mut entries = Vec::new();
    let mut extras_done = false;
    for item in items {
        if names(item, FieldId::Host) {
            let ports: Vec<Option<String>> = ctx
                .fields
                .hosts
                .iter()
                .map(|(_, p)| p.map(|p| p.to_string()))
                .collect();
            let hosts: Vec<(&str, Option<&str>)> = if ctx.fields.hosts.is_empty() {
                match ctx.fields.values.get(&FieldId::Host) {
                    Some(h) => vec![(
                        h.as_str(),
                        ctx.fields.values.get(&FieldId::Port).map(String::as_str),
                    )],
                    None => Vec::new(),
                }
            } else {
                ctx.fields
                    .hosts
                    .iter()
                    .zip(&ports)
                    .map(|((h, _), p)| (h.as_str(), p.as_deref()))
                    .collect()
            };
            for host in hosts {
                let mut tmp = ctx.buf();
                let s = Scope {
                    host: Some(host),
                    ..inner
                };
                if render_seq(item, &mut tmp, ctx, s, &mut 0)?.any() {
                    entries.push(tmp);
                }
            }
        } else if has_anonymous(item) {
            if extras_done {
                continue;
            }
            extras_done = true;
            for (k, v) in ctx.extra {
                let mut tmp = ctx.buf();
                let s = Scope {
                    pair: Some((k.as_str(), v.as_str())),
                    ..inner
                };
                if render_seq(item, &mut tmp, ctx, s, &mut 0)?.any() {
                    entries.push(tmp);
                }
            }
        } else {
            let mut tmp = ctx.buf();
            if render_seq(item, &mut tmp, ctx, inner, &mut 0)?.any() {
                entries.push(tmp);
            }
        }
    }
    Ok(entries)
}

fn write_value(out: &mut Buf, p: &Placeholder, v: &str) {
    match p.escape {
        Escape::None => out.push_str(v),
        Escape::Percent if p.ty == PhType::Host && v.contains(':') => {
            out.push('[');
            out.extend(utf8_percent_encode(v, PhType::Ipv6.encode_set()));
            out.push(']');
        }
        Escape::Percent => out.extend(utf8_percent_encode(v, p.ty.encode_set())),
    }
}

/// Some placeholder in `nodes` names `f`.
pub(super) fn names(nodes: &[Node], f: FieldId) -> bool {
    nodes.iter().any(|n| match n {
        Node::Ph(p) => p.name == Some(f),
        Node::Opt(alts) | Node::List { items: alts, .. } => alts.iter().any(|s| names(s, f)),
        Node::MaybeEmpty(inner) => names(std::slice::from_ref(inner.as_ref()), f),
        _ => false,
    })
}

/// Some placeholder in `nodes` has no name (a pass-through item).
pub(super) fn has_anonymous(nodes: &[Node]) -> bool {
    nodes.iter().any(|n| match n {
        Node::Ph(p) => p.name.is_none(),
        Node::Opt(alts) | Node::List { items: alts, .. } => alts.iter().any(|s| has_anonymous(s)),
        Node::MaybeEmpty(inner) => has_anonymous(std::slice::from_ref(inner.as_ref())),
        _ => false,
    })
}
