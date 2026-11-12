# Provenance of the AltGr tables and shift levels

`../altgr/*.toml` and the `shift_level` lines of `../layouts/*.toml` are derived from Microsoft
keyboard layout tables (KBDTABLES) in KLC form, downloaded on 2026-11-12 from kbdlayout.info:

| Layout | Source | DLL version |
|---|---|---|
| tr-q | https://kbdlayout.info/KBDTUQ/download/klc | kbdtuq.dll 10.0.25393.1 |
| tr-f | https://kbdlayout.info/KBDTUF/download/klc | kbdtuf.dll 10.0.25393.1 |
| de | https://kbdlayout.info/KBDGR/download/klc | kbdgr.dll 10.0.25393.1 |
| fr | https://kbdlayout.info/KBDFR/download/klc | kbdfr.dll 10.0.25393.1 |
| pl | https://kbdlayout.info/KBDPL1/download/klc | kbdpl1.dll 10.0.25393.1 |

The KLC files carry a Microsoft copyright notice and no redistribution terms, so the raw files
are NOT committed; only the derived facts (which character a key types) are. To regenerate,
download the five files (UTF-16 `.klc`) into one folder and run:

    python3 -I derive_altgr.py <folder>

These are the Windows layouts. The macOS Option-key layouts differ and are not covered in M1
(macOS has its own rule: no bare Option chord on a printable key).
