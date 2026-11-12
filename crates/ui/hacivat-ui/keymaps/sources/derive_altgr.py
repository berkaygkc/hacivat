#!/usr/bin/env python3
"""Derives keymaps/altgr/*.toml and the `shift_level` lines of keymaps/layouts/*.toml from
Microsoft keyboard layout tables in KLC form (see README.md in this folder).

Usage: python3 -I derive_altgr.py <dir with KBDTUQ.klc KBDTUF.klc KBDGR.klc KBDFR.klc KBDPL1.klc>

The KLC files are data only. Nothing in them is executed.
"""
import re
import sys
from pathlib import Path

LAYOUTS = {  # layout id -> KLC base name
    "tr-q": "KBDTUQ",
    "tr-f": "KBDTUF",
    "de": "KBDGR",
    "fr": "KBDFR",
    "pl": "KBDPL1",
}
ALTGR, SHIFT_ALTGR = 6, 7  # KLC shift states: Ctrl+Alt, Shift+Ctrl+Alt
HERE = Path(__file__).resolve().parent
KEYMAPS = HERE.parent


def cell(tok):
    """A KLC cell as (char, dead) or None. `-1` and control characters mean no character."""
    if tok in ("-1", ""):
        return None
    dead = tok.endswith("@")
    tok = tok.rstrip("@")
    ch = tok if len(tok) == 1 else chr(int(tok, 16))
    if ord(ch) < 0x20 or ord(ch) == 0x7F:
        return None
    return ch, dead


def parse(path):
    lines = path.read_bytes().decode("utf-16").replace("\r", "").split("\n")
    version = next(re.search(r"(kbd\w+\.dll version [\d.]+)", l).group(1) for l in lines if "dll version" in l)
    states, in_states, in_layout, rows = [], False, False, []
    for line in lines:
        body = line.split("//")[0].strip()
        if line.startswith("SHIFTSTATE"):
            in_states = True
            continue
        if line.startswith("LAYOUT"):
            in_states, in_layout = False, True
            continue
        if line.startswith(("DEADKEY", "LIGATURE", "KEYNAME")):
            in_layout = False
        if in_states and body:
            states.append(int(body))
        elif in_layout and body and not line.startswith("//"):
            parts = body.split()
            rows.append((parts[1], parts[3:]))
    return version, states, rows


def derive(path):
    version, states, rows = parse(path)
    col = {s: i for i, s in enumerate(states)}
    layers = {ALTGR: {}, SHIFT_ALTGR: {}}  # state -> name -> [(char, dead)]
    base_chars, shift_chars = set(), set()
    for vk, cells in rows:
        if vk in ("DECIMAL",) or vk.startswith("NUMPAD"):
            continue
        get = lambda s: cell(cells[col[s]]) if s in col and col[s] < len(cells) else None
        base, shifted = get(0), get(1)
        if base is None:
            continue
        base_chars.add(base[0])
        if shifted:
            shift_chars.add(shifted[0])
        names = {base[0]}
        # AZERTY digits are typed with Shift: a chord written with the digit must be checked too.
        if shifted and shifted[0].isascii() and shifted[0].isdigit():
            names.add(shifted[0])
        for state, layer in layers.items():
            c = get(state)
            if c is None:
                continue
            for n in names:
                entry = layer.setdefault(n, [])
                if c not in entry:
                    entry.append(c)
    need_shift = sorted(c for c in shift_chars - base_chars if not c.isalpha())
    return version, layers, "".join(need_shift)


def q(s):
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'


def table(layer, dead):
    out = []
    for name in sorted(layer, key=lambda n: (not n.isascii(), n)):
        chars = "".join(c for c, d in layer[name] if d == dead)
        if chars:
            out.append(f"{q(name)} = {q(chars)}")
    return out


def main(klc_dir):
    klc_dir = Path(klc_dir)
    for layout, base in LAYOUTS.items():
        version, layers, need_shift = derive(klc_dir / f"{base}.klc")
        out = [
            f"# AltGr layers of {base} ({version}), derived by sources/derive_altgr.py.",
            "# DO NOT EDIT: regenerate. Keys are the key caps GPUI reports (the unshifted character,",
            "# plus the digit on AZERTY); values are the characters typed. Several characters under",
            "# one key mean several physical keys fold onto it (Turkish dotless i). `dead` tables",
            "# hold dead keys: the value is the accent. `shift_*` is Shift+AltGr.",
            f'layout = "{layout}"',
        ]
        for name, state, dead in (
            ("keys", ALTGR, False),
            ("dead", ALTGR, True),
            ("shift_keys", SHIFT_ALTGR, False),
            ("shift_dead", SHIFT_ALTGR, True),
        ):
            rows = table(layers[state], dead)
            if rows:
                out += ["", f"[{name}]"] + rows
        (KEYMAPS / "altgr" / f"{layout}.toml").write_text("\n".join(out) + "\n", encoding="utf-8")
        lp = KEYMAPS / "layouts" / f"{layout}.toml"
        text = lp.read_text(encoding="utf-8")
        text = re.sub(r"(?m)^shift_level = .*$", "shift_level = " + q(need_shift), text)
        lp.write_text(text, encoding="utf-8")
        print(layout, version, "shift_level =", need_shift)


if __name__ == "__main__":
    main(sys.argv[1])
