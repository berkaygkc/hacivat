//! Key press resolution (UI-EDIT-13 steps 3 and 4).

use super::normalize::normalize_char;
use super::{
    Binding, ContextStack, KeyPress, KeyboardLayoutId, Keymap, KeymapPlatform, NormKey, Resolution,
};

/// Whether `press` triggers the chord press `bound`.
///
/// Equal presses match. A symbol chord also matches with an extra Shift when the layout needs
/// Shift to type that symbol (`shifted` lists those characters), so `ctrl-/` fires on the
/// `ctrl-shift-/` a Turkish Q user has to press.
fn press_matches(bound: &KeyPress, press: &KeyPress, shifted: &[char]) -> bool {
    if bound == press {
        return true;
    }
    match bound.key {
        NormKey::Char(c) if shifted.contains(&c) && !bound.mods.shift => {
            press.key == bound.key
                && press.mods.shift
                && press.mods
                    == super::Mods {
                        shift: true,
                        ..bound.mods
                    }
        }
        _ => false,
    }
}

/// AltGr text: on Windows and Linux AltGr arrives as Ctrl+Alt, and a printable character typed
/// with it must reach the text input whatever the bindings say (Review Focus 1).
fn is_altgr_text(platform: KeymapPlatform, press: &KeyPress) -> bool {
    platform != KeymapPlatform::MacOs
        && press.mods.ctrl
        && press.mods.alt
        && press.typed.is_some_and(|c| !c.is_control())
}

impl Keymap {
    /// Resolves one key press against the bindings.
    ///
    /// `pending` holds the earlier presses of a multi-key chord, `layout` selects overlay
    /// bindings and the shift-level symbols, and `composing` is true during IME composition
    /// (only `during_composition` bindings run then). The context stack is walked innermost
    /// first; the first context with a complete match returns its command, the first with only
    /// a longer chord returns `Pending`, and no match is `PassThrough`.
    pub fn resolve(
        &self,
        ctx: &ContextStack,
        pending: &[KeyPress],
        press: &KeyPress,
        layout: &KeyboardLayoutId,
        composing: bool,
    ) -> Resolution {
        if is_altgr_text(self.platform, press) {
            return Resolution::PassThrough;
        }
        let mut press = *press;
        if let NormKey::Char(c) = press.key {
            press.key = NormKey::Char(normalize_char(c));
        }
        let shifted: &[char] = self.shift_levels.get(layout).map_or(&[], Vec::as_slice);
        let mut seq: Vec<KeyPress> = pending.to_vec();
        seq.push(press);

        // Candidate bindings come from the first-press hash: the press itself, and the
        // unshifted symbol it may stand for.
        let first = seq[0];
        let mut candidates: Vec<usize> = self.index.get(&first).cloned().unwrap_or_default();
        if first.mods.shift {
            let unshifted = KeyPress {
                mods: super::Mods {
                    shift: false,
                    ..first.mods
                },
                ..first
            };
            if let Some(more) = self.index.get(&unshifted) {
                candidates.extend(more);
            }
        }
        let candidates: Vec<&Binding> = candidates
            .into_iter()
            .map(|i| &self.bindings[i])
            .filter(|b| b.when_layout.as_ref().is_none_or(|l| l == layout))
            .filter(|b| !composing || b.during_composition)
            .filter(|b| {
                b.chord.len() >= seq.len()
                    && b.chord
                        .iter()
                        .zip(&seq)
                        .all(|(bound, p)| press_matches(bound, p, shifted))
            })
            .collect();

        for name in ctx.0.iter().rev() {
            let mut in_ctx = candidates.iter().filter(|b| b.context == *name).peekable();
            if in_ctx.peek().is_none() {
                continue;
            }
            let mut longer = false;
            for b in in_ctx {
                if b.chord.len() == seq.len() {
                    return Resolution::Command(b.command.clone());
                }
                longer = true;
            }
            if longer {
                return Resolution::Pending;
            }
        }
        Resolution::PassThrough
    }
}
