//! SSH host keys (M2a NOTES surface 6, NOTES surface 1, B11, Task 21): trust an unknown key,
//! review a changed one.
//!
//! - [`open`] asks `Question::HostKey`: a 420 px modal layer `hostkey-prompt`, "First connection
//!   to <host>", the algorithm and the `SHA256:` fingerprint in an inset, the store note, and
//!   Cancel / **Trust and connect** (the visual primary). Fix round 1 (review I2, controller
//!   ruling; the PO's rule for the certificate dialog): the prompt opens mid-connect, perhaps
//!   while the user types elsewhere, so **Cancel has the initial focus and Enter cancels**
//!   (on a Copy button it copies); trusting a first-seen key takes a click or Space on Trust
//!   and connect. This departs from the M2a mock, which focused Trust. Escape, Cancel or the
//!   close button answer `Cancelled`.
//! - [`review_changed`] is the "Review host key…" fix of `ssh.host_key_changed`: a hard stop
//!   (Q34), a 480 px alertdialog `hk-changed` with both keys. Cancel is the primary with the
//!   initial focus and the only Enter default: Enter cancels wherever the focus is, except on a
//!   Copy button (it copies) or Try again. The stored key's source decides the trust action:
//!   - Hacivat's store: "Remove the old key and trust the new one" (an outline button on the far
//!     left, reached with Tab and pressed with Space or a click, never with Enter) calls
//!     `KnownHosts::replace` on the core runtime, then reconnects;
//!   - an OpenSSH file: no replace button, ever. Hacivat never edits `~/.ssh`; it shows the
//!     `ssh-keygen -R` command (built from `known_hosts::host_pattern`, with `-f` for any file
//!     other than `~/.ssh/known_hosts`) to copy, and Try again reconnects without a restart:
//!     `KnownHosts::check` re-reads a file whose `(mtime, len)` changed (Q24). A host or file
//!     name a shell could misread (quotes, spaces, a leading dash) gets no command.

// ## m2b-t21-prompts

use std::path::Path;
use std::rc::Rc;

use hacivat_l10n::{NoArgs, Text, ids};
use hacivat_net::ssh::known_hosts::host_pattern;
use hacivat_net::ssh::{HostKey, KnownSource};
use hacivat_proto::ProfileId;
use hacivat_session::pipeline::{Answer, HostKeyMismatch, Question};
use hacivat_ui::button::{Button, ButtonKind};
use hacivat_ui::dialog::DialogSize;
use hacivat_ui::icon::IconName;
use hacivat_ui::popover::A11yNode;
use hacivat_ui::text::{Label, TextSource, paragraph};
use hacivat_ui::toast::{Toast, ToastKind, ToastQueue};
use hacivat_ui_core::gpui_surface::{
    App, AppContext as _, ClipboardItem, Context, Div, FocusHandle, Focusable, InteractiveElement,
    IntoElement, KeyDownEvent, ParentElement, Render, Role, SharedString, Styled, Window, div, px,
};
use hacivat_ui_core::theme::Theme;
use tokio::sync::oneshot;

use crate::connections::ConnectionsModel;
use crate::connections::prompt_kit::{
    self, Part, acting_enter, card, cycle_focus, footer, header, inset, register_part,
    remove_modal, subject, subtitle, trapped_tab, visible,
};
use crate::l10n_global::tr;

/// The new-host-key prompt's layer ID.
pub const HOSTKEY_LAYER_ID: &str = "hostkey-prompt";
/// The changed-host-key review's layer ID.
pub const CHANGED_LAYER_ID: &str = "hk-changed";

/// The longest algorithm name shown as sent (an unknown one).
const MAX_ALGORITHM_CHARS: usize = 64;

/// The key type as OpenSSH prints it next to a fingerprint (`ED25519`, `ECDSA`, `RSA`).
fn key_type(algorithm: &str) -> String {
    let short = match algorithm {
        "ssh-ed25519" => "ED25519",
        "sk-ssh-ed25519@openssh.com" => "ED25519-SK",
        "ssh-ed448" => "ED448",
        "ecdsa-sha2-nistp256" | "ecdsa-sha2-nistp384" | "ecdsa-sha2-nistp521" => "ECDSA",
        "sk-ecdsa-sha2-nistp256@openssh.com" => "ECDSA-SK",
        "ssh-rsa" | "rsa-sha2-256" | "rsa-sha2-512" => "RSA",
        "ssh-dss" => "DSA",
        other => return visible(other, MAX_ALGORITHM_CHARS),
    };
    short.to_string()
}

/// `host:port`, IPv6 literals in brackets.
fn address(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

/// The fingerprint inset (NOTES "Key comparison"): the label (12.5 px text-2), the key type
/// (mono, text), a ghost Copy icon button on the right, then the `SHA256:` fingerprint (mono
/// 11.5 text). Raised Ink: nothing in text-3 (Q34). `extra` goes under a seam.
fn fingerprint_inset(
    theme: &Theme,
    label: SharedString,
    key: &HostKey,
    ids: (&'static str, &'static str),
    copy: Button,
    extra: Option<Div>,
) -> Div {
    let t = theme.tokens;
    let c = &t.colors;
    let head = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(t.spacing.sm))
        .child(Label::new(
            label,
            TextSource::Fluent,
            t.type_ramp.body_sm,
            c.text_2.to_hsla(),
        ))
        .child(
            Label::new(
                key_type(&key.algorithm),
                TextSource::Data,
                t.type_ramp.mono_data,
                c.text.to_hsla(),
            )
            .id(ids.0),
        )
        .child(div().flex_1())
        .child(copy);
    inset(theme)
        .gap(px(t.spacing.xs))
        .child(head)
        .child(
            paragraph(
                key.fingerprint(),
                TextSource::Data,
                t.type_ramp.mono_data,
                c.text.to_hsla(),
            )
            .id(ids.1),
        )
        .children(extra.map(|extra| {
            div()
                .mt(px(t.spacing.xs))
                .pt(px(t.spacing.sm))
                .border_t_1()
                .border_color(c.seam.to_hsla())
                .child(extra)
        }))
}

/// A ghost icon button that copies.
fn copy_button(id: &'static str, focus: &FocusHandle, cx: &App) -> Button {
    Button::new(id, tr(cx, ids::hv::CONN_HK_COPY_FP, NoArgs))
        .kind(ButtonKind::Icon)
        .icon(IconName::Copy)
        .focus_handle(focus.clone())
}

/// Puts `text` on the clipboard.
fn copy_text(text: String, cx: &mut App) {
    cx.write_to_clipboard(ClipboardItem::new_string(text));
}

// ---- the new-host-key prompt ----------------------------------------------------------------

/// What a host-key question shows (`Question::HostKey` without its profile ID).
struct Asked {
    host: String,
    port: u16,
    user: Option<String>,
    key: HostKey,
}

/// The new-host-key prompt (M2a surface 6).
pub struct HostKeyPrompt {
    asked: Asked,
    reply: Option<oneshot::Sender<Answer>>,
    root: FocusHandle,
    copy_focus: FocusHandle,
    cancel_focus: FocusHandle,
    trust_focus: FocusHandle,
}

impl Focusable for HostKeyPrompt {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.root.clone()
    }
}

impl HostKeyPrompt {
    fn answer(&mut self, answer: Answer, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(reply) = self.reply.take() {
            let _ = reply.send(answer);
        }
        let view = cx.entity_id();
        window.defer(cx, move |window, cx| remove_modal(view, window, cx));
    }

    fn copy(&mut self, cx: &mut Context<Self>) {
        copy_text(self.asked.key.fingerprint(), cx);
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "escape" => self.answer(Answer::Cancelled, window, cx),
            // ## m2b-t21-prompts (fix round 1, I2): Enter never trusts a key; it copies on the
            // Copy button and cancels anywhere else, Trust and connect included.
            "enter" if acting_enter(event) => {
                if self.copy_focus.is_focused(window) {
                    self.copy(cx);
                } else {
                    self.answer(Answer::Cancelled, window, cx);
                }
            }
            "enter" => {}
            _ if trapped_tab(event) => {
                let order = [
                    self.copy_focus.clone(),
                    self.cancel_focus.clone(),
                    self.trust_focus.clone(),
                ];
                cycle_focus(&order, event.keystroke.modifiers.shift, window, cx);
            }
            _ => return,
        }
        cx.stop_propagation();
    }
}

impl Render for HostKeyPrompt {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let a = &self.asked;
        let title = tr(
            cx,
            ids::hv::CONN_HK_TITLE,
            ids::hv::HvConnHkTitle {
                host: Text(a.host.clone()),
            },
        );
        let os = if prompt_kit::on_mac() {
            ids::hv::HvConnHkBodyOs::Mac
        } else {
            ids::hv::HvConnHkBodyOs::Other
        };
        let body = tr(cx, ids::hv::CONN_HK_BODY, ids::hv::HvConnHkBody { os });
        let mut parts = vec![Part::Mono(address(&a.host, a.port).into())];
        if let Some(user) = &a.user {
            parts.push(Part::Mono(user.clone().into()));
        }
        let copy = copy_button("hk-copy", &self.copy_focus, cx)
            .on_click(cx.listener(|this, _, _, cx| this.copy(cx)));
        let cancel = Button::new("hk-cancel", tr(cx, ids::hv::CONN_PW_CANCEL, NoArgs))
            .kind(ButtonKind::Ghost)
            .focus_handle(self.cancel_focus.clone())
            .on_click(cx.listener(|this, _, window, cx| {
                this.answer(Answer::Cancelled, window, cx);
            }));
        let trust = Button::new("hk-trust", tr(cx, ids::hv::CONN_HK_TRUST, NoArgs))
            .kind(ButtonKind::Primary)
            .icon(IconName::Check)
            .focus_handle(self.trust_focus.clone())
            .on_click(cx.listener(|this, _, window, cx| {
                this.answer(Answer::TrustHostKey, window, cx);
            }));
        let inset = fingerprint_inset(
            &theme,
            tr(cx, ids::hv::CONN_HK_FP, NoArgs),
            &a.key,
            ("hk-algorithm", "hk-fingerprint"),
            copy,
            None,
        );
        let card = card(&theme, DialogSize::Standard, HOSTKEY_LAYER_ID)
            .track_focus(&self.root)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.on_key(event, window, cx);
            }))
            .child(header(
                &theme,
                title.clone(),
                "hk-title",
                Some(subtitle(&theme, None, parts)),
                cx.listener(|this, _, window, cx| this.answer(Answer::Cancelled, window, cx)),
                cx,
            ))
            .child(
                paragraph(
                    body.clone(),
                    TextSource::Fluent,
                    t.type_ramp.body,
                    c.text_2.to_hsla(),
                )
                .id("hk-body"),
            )
            .child(inset)
            .child(
                paragraph(
                    tr(cx, ids::hv::CONN_HK_STORE, NoArgs),
                    TextSource::Fluent,
                    t.type_ramp.body_sm,
                    c.text_2.to_hsla(),
                )
                .id("hk-store"),
            )
            .child(footer(
                &theme,
                Vec::new(),
                vec![cancel.into_any_element(), trust.into_any_element()],
            ));
        A11yNode::new("hostkey-prompt-node", Role::Dialog, title, true, card).description(body)
    }
}

/// Asks `q` (`Question::HostKey`); the answer goes to `reply`. Any other question, or a window
/// without a workspace, answers `Cancelled`.
pub(crate) fn open(q: Question, reply: oneshot::Sender<Answer>, window: &mut Window, cx: &mut App) {
    let Question::HostKey {
        profile,
        hop,
        host,
        port,
        key,
        ..
    } = q
    else {
        let _ = reply.send(Answer::Cancelled);
        return;
    };
    let user = subject(cx, profile)
        .and_then(|s| s.hops.get(usize::from(hop)).map(|h| h.user.clone()))
        .filter(|u| !u.is_empty());
    let (answer_tx, answer_rx) = oneshot::channel();
    let prompt = cx.new(|cx| HostKeyPrompt {
        asked: Asked {
            host,
            port,
            user,
            key,
        },
        reply: Some(answer_tx),
        root: cx.focus_handle(),
        copy_focus: cx.focus_handle().tab_stop(true),
        cancel_focus: cx.focus_handle().tab_stop(true),
        trust_focus: cx.focus_handle().tab_stop(true),
    });
    let (copy, cancel, trust) = {
        let p = prompt.read(cx);
        (
            p.copy_focus.clone(),
            p.cancel_focus.clone(),
            p.trust_focus.clone(),
        )
    };
    register_part(cx, "hk-copy", &copy, None);
    register_part(cx, "hk-cancel", &cancel, None);
    register_part(cx, "hk-trust", &trust, None);
    let view = prompt.entity_id();
    // Fix round 1 (I2): Cancel has the initial focus.
    if !prompt_kit::push_modal(HOSTKEY_LAYER_ID, prompt.into(), &cancel, window, cx) {
        return;
    }
    prompt_kit::forward(reply, answer_rx, window.window_handle(), view, cx);
}

// ---- the changed-host-key review --------------------------------------------------------------

/// `ssh-keygen -R` for `host:port` stored in the OpenSSH file `file` (NOTES surface 1, open
/// question 1): the name is `known_hosts::host_pattern` (the bare host on port 22, else
/// `"[host]:port"`, quoted for the shell), and a file other than `~/.ssh/known_hosts` adds
/// `-f <file>` (with `~` for the home directory `home`). `None` when the host or the file holds a
/// character a shell could misread (host names were validated when the profile was saved; this
/// refuses anyway) or starts with a dash (it would read as an option).
pub fn keygen_command(host: &str, port: u16, file: &Path, home: Option<&Path>) -> Option<String> {
    let name = host_pattern(host, port);
    let host_safe = |ch: char| ch.is_ascii_alphanumeric() || ".-_:%[]".contains(ch);
    if name.is_empty() || name.starts_with('-') || !name.chars().all(host_safe) {
        return None;
    }
    let name = if port == 22 {
        name
    } else {
        format!("\"{name}\"")
    };
    if is_default_known_hosts(file) {
        return Some(format!("ssh-keygen -R {name}"));
    }
    let shown = tilde(file, home);
    // `:` for a Windows drive prefix (`C:/…`); a shell reads it literally.
    let path_safe = |ch: char| ch.is_ascii_alphanumeric() || "._/~+@,-:".contains(ch);
    if shown.is_empty() || shown.starts_with('-') || !shown.chars().all(path_safe) {
        return None;
    }
    Some(format!("ssh-keygen -f {shown} -R {name}"))
}

/// Whether `file` is OpenSSH's default `known_hosts` (`<home>/.ssh/known_hosts`), the one
/// `ssh-keygen -R` edits without `-f`. The stores read only the user's `~/.ssh/known_hosts` and
/// `~/.ssh/known_hosts2` (`ConnectionsBoot::load`), so the shape names the default file.
fn is_default_known_hosts(file: &Path) -> bool {
    file.file_name().is_some_and(|n| n == "known_hosts")
        && file
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|d| d == ".ssh")
}

/// `file` with the home directory written as `~` and `/` between components on every system:
/// OpenSSH for Windows takes forward slashes, and a backslash would be refused as unsafe (the
/// command would vanish on Windows). A backslash inside a Unix file name stays, and is refused.
fn tilde(file: &Path, home: Option<&Path>) -> String {
    match home.and_then(|h| file.strip_prefix(h).ok()) {
        Some(rest) => format!("~/{}", slashed(rest)),
        None => slashed(file),
    }
}

/// `path` with its components joined by `/` (a Windows prefix such as `C:` kept as is).
fn slashed(path: &Path) -> String {
    use std::path::Component;
    let mut out = String::new();
    for part in path.components() {
        match part {
            Component::Prefix(prefix) => out.push_str(&prefix.as_os_str().to_string_lossy()),
            Component::RootDir => out.push('/'),
            other => {
                if !out.is_empty() && !out.ends_with('/') {
                    out.push('/');
                }
                out.push_str(&other.as_os_str().to_string_lossy());
            }
        }
    }
    out
}

/// The review of a changed host key (NOTES surface 1).
pub struct HostKeyChanged {
    m: HostKeyMismatch,
    profile: ProfileId,
    /// The OpenSSH file holding the old key, as shown (`~/.ssh/known_hosts`), and its line.
    openssh: Option<(String, usize)>,
    /// The `ssh-keygen` command (OpenSSH source only; `None` when refused).
    command: Option<String>,
    root: FocusHandle,
    copy_old: FocusHandle,
    copy_new: FocusHandle,
    copy_command: FocusHandle,
    action: FocusHandle,
    cancel: FocusHandle,
    /// A replace is running on the core runtime.
    busy: bool,
}

impl Focusable for HostKeyChanged {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.root.clone()
    }
}

impl HostKeyChanged {
    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.entity_id();
        window.defer(cx, move |window, cx| remove_modal(view, window, cx));
    }

    /// Cancel: the connect stays failed (`ssh.host_key_changed`).
    fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close(window, cx);
    }

    /// Try again (OpenSSH source): reconnects; the stores re-read the changed file (Q24).
    fn retry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let profile = self.profile;
        self.close(window, cx);
        ConnectionsModel::connect(cx, profile, None);
    }

    /// "Remove the old key and trust the new one" (Hacivat's store only): `KnownHosts::replace`
    /// on the core runtime, then a reconnect. The reconnect checks the key again, so a replace
    /// that failed (the old line went meanwhile, the key is revoked) trusts nothing.
    fn replace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.m.source != KnownSource::Hacivat {
            return;
        }
        let Some(model) = cx.try_global::<ConnectionsModel>() else {
            return;
        };
        self.busy = true;
        let known = model.known_hosts();
        let runtime = model.runtime();
        let m = self.m.clone();
        let (tx, rx) = oneshot::channel();
        runtime.spawn_blocking(move || {
            let _ = tx.send(known.replace(&m.host, m.port, &m.old, &m.new).is_ok());
        });
        let host = self.m.host.clone();
        let profile = self.profile;
        cx.spawn_in(window, async move |this, cx| {
            let replaced = rx.await.unwrap_or(false);
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                this.close(window, cx);
                if replaced {
                    let text = Rc::new(move |cx: &App| {
                        tr(
                            cx,
                            ids::hv::CONN_HK_CHANGED_TRUSTED,
                            ids::hv::HvConnHkChangedTrusted {
                                host: Text(host.clone()),
                            },
                        )
                    });
                    ToastQueue::push(cx, Toast::new(ToastKind::Info, text));
                } else {
                    tracing::warn!("connections: replacing a changed host key failed");
                }
                ConnectionsModel::connect(cx, profile, None);
            });
        })
        .detach();
        cx.notify();
    }

    fn copy_command(&mut self, cx: &mut Context<Self>) {
        let Some(command) = self.command.clone() else {
            return;
        };
        copy_text(command, cx);
        let text = Rc::new(|cx: &App| tr(cx, ids::hv::CONN_HK_CHANGED_COPIED, NoArgs));
        ToastQueue::push(cx, Toast::new(ToastKind::Info, text));
    }

    fn hacivat(&self) -> bool {
        self.m.source == KnownSource::Hacivat
    }

    /// Tab order (NOTES): Copy (stored), Copy (offered), [Copy command], the trust action or
    /// Try again, Cancel.
    fn order(&self) -> Vec<FocusHandle> {
        let mut order = vec![self.copy_old.clone(), self.copy_new.clone()];
        if self.command.is_some() {
            order.push(self.copy_command.clone());
        }
        order.push(self.action.clone());
        order.push(self.cancel.clone());
        order
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "escape" => self.cancel(window, cx),
            // A hard stop (Q34): Cancel is the only Enter default. Enter copies on a Copy
            // button and retries on Try again; anywhere else, the replace button included, it
            // cancels. The replace runs only from a click or Space.
            "enter" if acting_enter(event) => {
                if self.copy_old.is_focused(window) {
                    copy_text(self.m.old.fingerprint(), cx);
                } else if self.copy_new.is_focused(window) {
                    copy_text(self.m.new.fingerprint(), cx);
                } else if self.copy_command.is_focused(window) {
                    self.copy_command(cx);
                } else if !self.hacivat() && self.action.is_focused(window) {
                    self.retry(window, cx);
                } else {
                    self.cancel(window, cx);
                }
            }
            "enter" => {}
            _ if trapped_tab(event) => {
                cycle_focus(&self.order(), event.keystroke.modifiers.shift, window, cx);
            }
            _ => return,
        }
        cx.stop_propagation();
    }
}

impl Render for HostKeyChanged {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let host = self.m.host.clone();
        let title = tr(
            cx,
            ids::hv::CONN_HK_CHANGED_TITLE,
            ids::hv::HvConnHkChangedTitle {
                host: Text(host.clone()),
            },
        );
        let body = tr(cx, ids::hv::CONN_HK_CHANGED_BODY, NoArgs);
        let subject = subject(cx, self.profile);
        let mut parts = Vec::new();
        if let Some(s) = &subject {
            parts.push(Part::Mono(s.name.clone().into()));
        }
        let total = subject.as_ref().map_or(0, |s| s.hops.len());
        let hop = usize::from(self.m.hop) + 1;
        if total >= hop {
            parts.push(Part::Fluent(tr(
                cx,
                ids::hv::CONN_SSH_SECRET_HOP,
                ids::hv::HvConnSshSecretHop {
                    host: Text(host.clone()),
                    n: prompt_kit::count(hop),
                    total: prompt_kit::count(total),
                },
            )));
        } else {
            parts.push(Part::Mono(host.clone().into()));
        }
        let env = subject.as_ref().and_then(|s| s.env);

        // Where the old key is stored.
        let stored_in = match &self.openssh {
            None => tr(cx, ids::hv::CONN_HK_CHANGED_SRC_HACIVAT, NoArgs),
            Some((file, line)) => tr(
                cx,
                ids::hv::CONN_HK_CHANGED_SRC_OPENSSH,
                ids::hv::HvConnHkChangedSrcOpenssh {
                    file: Text(file.clone()),
                    n: prompt_kit::count(*line),
                },
            ),
        };
        let source_line = div()
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap(px(t.spacing.sm))
            .child(Label::new(
                tr(cx, ids::hv::CONN_HK_CHANGED_STORED_IN, NoArgs),
                TextSource::Fluent,
                t.type_ramp.caption,
                c.text_2.to_hsla(),
            ))
            .child(
                Label::new(
                    stored_in,
                    TextSource::Fluent,
                    t.type_ramp.caption,
                    c.text.to_hsla(),
                )
                .id("hk-stored-in"),
            );
        let copy_old = copy_button("hk-copy-old", &self.copy_old, cx).on_click(cx.listener(
            |this, _, _, cx| {
                copy_text(this.m.old.fingerprint(), cx);
            },
        ));
        let copy_new = copy_button("hk-copy-new", &self.copy_new, cx).on_click(cx.listener(
            |this, _, _, cx| {
                copy_text(this.m.new.fingerprint(), cx);
            },
        ));
        let old = fingerprint_inset(
            &theme,
            tr(cx, ids::hv::CONN_HK_CHANGED_OLD, NoArgs),
            &self.m.old,
            ("hk-old-algorithm", "hk-old-fingerprint"),
            copy_old,
            Some(source_line),
        );
        let new = fingerprint_inset(
            &theme,
            tr(cx, ids::hv::CONN_HK_CHANGED_NEW, NoArgs),
            &self.m.new,
            ("hk-new-algorithm", "hk-new-fingerprint"),
            copy_new,
            None,
        );
        let ask = paragraph(
            tr(
                cx,
                ids::hv::CONN_HK_CHANGED_ASK,
                ids::hv::HvConnHkChangedAsk {
                    host: Text(host.clone()),
                },
            ),
            TextSource::Fluent,
            t.type_ramp.body_sm,
            c.text_2.to_hsla(),
        )
        .id("hk-ask");

        // The OpenSSH source: the note and the command to copy.
        let openssh = self.openssh.as_ref().map(|(file, _)| {
            let note = paragraph(
                tr(
                    cx,
                    ids::hv::CONN_HK_CHANGED_OPENSSH_NOTE,
                    ids::hv::HvConnHkChangedOpensshNote {
                        file: Text(file.clone()),
                    },
                ),
                TextSource::Fluent,
                t.type_ramp.body_sm,
                c.text_2.to_hsla(),
            )
            .id("hk-openssh-note");
            let command = self.command.clone().map(|command| {
                let copy = Button::new(
                    "hk-copy-command",
                    tr(cx, ids::hv::CONN_HK_CHANGED_COPY, NoArgs),
                )
                .kind(ButtonKind::Ghost)
                .icon(IconName::Copy)
                .focus_handle(self.copy_command.clone())
                .on_click(cx.listener(|this, _, _, cx| this.copy_command(cx)));
                inset(&theme)
                    .flex_row()
                    .items_center()
                    .gap(px(t.spacing.md))
                    .child(
                        div().flex_1().min_w_0().child(
                            paragraph(
                                command,
                                TextSource::Data,
                                t.type_ramp.mono_data,
                                c.text.to_hsla(),
                            )
                            .id("hk-keygen"),
                        ),
                    )
                    .child(copy)
            });
            div()
                .flex()
                .flex_col()
                .gap(px(t.spacing.sm))
                .child(note)
                .children(command)
        });

        let cancel = Button::new("hk-cancel", tr(cx, ids::hv::CONN_PW_CANCEL, NoArgs))
            .kind(ButtonKind::Primary)
            .focus_handle(self.cancel.clone())
            .on_click(cx.listener(|this, _, window, cx| this.cancel(window, cx)));
        let (left, right) = if self.hacivat() {
            let replace = Button::new(
                "hk-replace",
                tr(cx, ids::hv::CONN_HK_CHANGED_REPLACE, NoArgs),
            )
            .kind(ButtonKind::Default)
            // ## m2b-t24-shots: the mock's key icon (T21 review m10).
            .icon(IconName::KeyRound)
            .disabled(self.busy)
            .focus_handle(self.action.clone())
            .on_click(cx.listener(|this, _, window, cx| this.replace(window, cx)));
            (
                vec![replace.into_any_element()],
                vec![cancel.into_any_element()],
            )
        } else {
            let retry = Button::new("hk-retry", tr(cx, ids::hv::CONN_FIX_RETRY, NoArgs))
                .kind(ButtonKind::Default)
                .icon(IconName::RotateCw)
                .focus_handle(self.action.clone())
                .on_click(cx.listener(|this, _, window, cx| this.retry(window, cx)));
            (
                Vec::new(),
                vec![retry.into_any_element(), cancel.into_any_element()],
            )
        };

        let card = card(&theme, DialogSize::Wide, CHANGED_LAYER_ID)
            .track_focus(&self.root)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.on_key(event, window, cx);
            }))
            .child(header(
                &theme,
                title.clone(),
                "hk-changed-title",
                Some(subtitle(&theme, env, parts)),
                cx.listener(|this, _, window, cx| this.cancel(window, cx)),
                cx,
            ))
            .child(
                paragraph(
                    body.clone(),
                    TextSource::Fluent,
                    t.type_ramp.body,
                    c.text_2.to_hsla(),
                )
                .id("hk-changed-body"),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(t.spacing.md))
                    .child(old)
                    .child(new),
            )
            .child(ask)
            .children(openssh)
            .child(footer(&theme, left, right));
        // An alertdialog (a hard stop), labelled by its title.
        A11yNode::new("hk-changed-node", Role::AlertDialog, title, true, card).description(body)
    }
}

/// The "Review host key" fix: shows the changed key `m` of `profile`'s hop (a hard stop) and
/// replaces it in Hacivat's store only on an explicit choice. Without a workspace in `window`
/// it does nothing.
pub fn review_changed(m: HostKeyMismatch, profile: ProfileId, window: &mut Window, cx: &mut App) {
    let home = std::env::home_dir();
    let (openssh, command) = match &m.source {
        KnownSource::Hacivat => (None, None),
        KnownSource::OpenSsh { path, line } => (
            Some((tilde(path, home.as_deref()), *line)),
            keygen_command(&m.host, m.port, path, home.as_deref()),
        ),
    };
    let view = cx.new(|cx| HostKeyChanged {
        m,
        profile,
        openssh,
        command,
        root: cx.focus_handle(),
        copy_old: cx.focus_handle().tab_stop(true),
        copy_new: cx.focus_handle().tab_stop(true),
        copy_command: cx.focus_handle().tab_stop(true),
        action: cx.focus_handle().tab_stop(true),
        cancel: cx.focus_handle().tab_stop(true),
        busy: false,
    });
    let (cancel, names) = {
        let v = view.read(cx);
        let action_name = if v.hacivat() {
            "hk-replace"
        } else {
            "hk-retry"
        };
        (
            v.cancel.clone(),
            [
                ("hk-copy-old", v.copy_old.clone()),
                ("hk-copy-new", v.copy_new.clone()),
                ("hk-copy-command", v.copy_command.clone()),
                (action_name, v.action.clone()),
            ],
        )
    };
    for (name, focus) in &names {
        register_part(cx, name, focus, None);
    }
    register_part(cx, "hk-cancel", &cancel, None);
    // Q34: Cancel is the primary and has the initial focus.
    let _ = prompt_kit::push_modal(CHANGED_LAYER_ID, view.into(), &cancel, window, cx);
}

// ## m2b-t24-shots: shots fixtures (R28, P19).
#[cfg(feature = "shots")]
mod shots {
    use hacivat_net::ssh::KnownSource;
    use hacivat_proto::EnvTag;
    use hacivat_session::pipeline::{HostKeyMismatch, Question};
    use hacivat_ui_core::gpui_surface::{App, Window};

    use crate::connections::prompt_kit::shots::{held_reply, host_keys, save_profile, two_hops};

    /// First connection to `bastion.acme.io` (M2a surface 6, as M2b reuses it).
    pub fn new_fixture(window: &mut Window, cx: &mut App) {
        let id = save_profile(cx, "orders-prod", EnvTag::Production, two_hops());
        let reply = held_reply(cx);
        super::open(
            Question::HostKey {
                profile: id,
                name: "orders-prod".to_string(),
                hop: 1,
                host: "bastion.acme.io".to_string(),
                port: 22,
                key: host_keys().1,
            },
            reply,
            window,
            cx,
        );
    }

    /// The changed key of `bastion.acme.io` (hop 2 of 2), stored in `source`.
    fn changed(source: KnownSource, window: &mut Window, cx: &mut App) {
        let id = save_profile(cx, "orders-prod", EnvTag::Production, two_hops());
        let (old, new) = host_keys();
        super::review_changed(
            HostKeyMismatch {
                hop: 1,
                host: "bastion.acme.io".to_string(),
                port: 22,
                old,
                new,
                source,
            },
            id,
            window,
            cx,
        );
    }

    /// The hard stop for a key in Hacivat's store: the replace button (NOTES M2b surface 1).
    pub fn changed_fixture(window: &mut Window, cx: &mut App) {
        changed(KnownSource::Hacivat, window, cx);
    }

    /// The hard stop for a key in `~/.ssh/known_hosts` line 14: the `ssh-keygen -R` command and
    /// Try again. The path is under the home folder, so the shot shows it as `~/…`.
    pub fn changed_openssh_fixture(window: &mut Window, cx: &mut App) {
        let home = std::env::home_dir().unwrap_or_else(|| "/home/deploy".into());
        changed(
            KnownSource::OpenSsh {
                path: home.join(".ssh").join("known_hosts"),
                line: 14,
            },
            window,
            cx,
        );
    }
}

#[cfg(feature = "shots")]
pub use shots::{changed_fixture, changed_openssh_fixture, new_fixture};

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{key_type, keygen_command};

    #[test]
    fn the_command_follows_openssh_names() {
        let home = Some(Path::new("/home/me"));
        let kh = Path::new("/home/me/.ssh/known_hosts");
        assert_eq!(
            keygen_command("bastion.acme.io", 22, kh, home).as_deref(),
            Some("ssh-keygen -R bastion.acme.io"),
            "port 22 is the bare host"
        );
        assert_eq!(
            keygen_command("Bastion.ACME.io", 2222, kh, home).as_deref(),
            Some("ssh-keygen -R \"[bastion.acme.io]:2222\""),
            "any other port is the bracket form, lower-cased like the store"
        );
        assert_eq!(
            keygen_command(
                "bastion.acme.io",
                22,
                Path::new("/home/me/.ssh/known_hosts2"),
                home
            )
            .as_deref(),
            Some("ssh-keygen -f ~/.ssh/known_hosts2 -R bastion.acme.io"),
            "another file needs -f"
        );
    }

    /// Windows paths are shown with forward slashes (OpenSSH for Windows takes them); before,
    /// the backslashes were refused and the command vanished on Windows.
    #[cfg(windows)]
    #[test]
    fn a_windows_path_uses_forward_slashes() {
        let home = Some(Path::new(r"C:\Users\me"));
        assert_eq!(
            keygen_command(
                "bastion.acme.io",
                22,
                Path::new(r"C:\Users\me\.ssh\known_hosts2"),
                home
            )
            .as_deref(),
            Some("ssh-keygen -f ~/.ssh/known_hosts2 -R bastion.acme.io")
        );
        assert_eq!(
            keygen_command(
                "bastion.acme.io",
                22,
                Path::new(r"D:\keys\known_hosts2"),
                home
            )
            .as_deref(),
            Some("ssh-keygen -f D:/keys/known_hosts2 -R bastion.acme.io")
        );
    }

    /// On Unix a backslash is part of a file name, and a shell would read it: refused.
    #[cfg(unix)]
    #[test]
    fn a_backslash_in_a_unix_file_name_is_refused() {
        let home = Some(Path::new("/home/me"));
        assert_eq!(
            keygen_command(
                "bastion.acme.io",
                22,
                Path::new("/home/me/.ssh/known\\hosts2"),
                home
            ),
            None
        );
    }

    #[test]
    fn a_command_a_shell_could_misread_is_refused() {
        let home = Some(Path::new("/home/me"));
        let kh = Path::new("/home/me/.ssh/known_hosts");
        for host in [
            "bad host",
            "x\";rm -rf ~;\"",
            "-oProxyCommand=x",
            "a$(id)",
            "a`id`",
            "",
        ] {
            assert_eq!(keygen_command(host, 22, kh, home), None, "{host:?}");
        }
        assert_eq!(
            keygen_command(
                "bastion.acme.io",
                22,
                Path::new("/srv/shared files/known_hosts2"),
                home
            ),
            None,
            "a file name with a space"
        );
        assert_eq!(
            keygen_command("fe80::1", 2222, kh, home).as_deref(),
            Some("ssh-keygen -R \"[fe80::1]:2222\""),
            "an IPv6 literal is fine"
        );
    }

    #[test]
    fn key_types_read_like_openssh() {
        assert_eq!(key_type("ssh-ed25519"), "ED25519");
        assert_eq!(key_type("rsa-sha2-512"), "RSA");
        assert_eq!(key_type("ecdsa-sha2-nistp384"), "ECDSA");
        assert_eq!(
            key_type("x\u{1b}[31m"),
            "x\\033[31m",
            "an unknown name is made visible"
        );
    }
}
