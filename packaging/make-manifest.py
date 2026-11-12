#!/usr/bin/env python3
"""Render packaging/update-manifest.template.json for the update artifacts found in a directory.

usage: make-manifest.py <version> <base-url> <artifact-dir>

An artifact is `<name>` plus its `<name>.minisig` (made with `minisign -S`). The `signature` field
the updater wants is the base64 of the .minisig file's text. Platforms without an artifact are dropped.
"""
import base64
import datetime
import json
import pathlib
import sys

version, base_url, art_dir = sys.argv[1], sys.argv[2].rstrip("/"), pathlib.Path(sys.argv[3])
PATTERNS = {"macos": "*.app.tar.gz", "windows": "*-setup.exe", "linux": "*.AppImage"}
template = (pathlib.Path(__file__).parent / "update-manifest.template.json").read_text()
now = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
doc = json.loads(template.replace("@VERSION@", version).replace("@PUB_DATE@", now))

files = {}
for os_name, pattern in PATTERNS.items():
    found = sorted(p for p in art_dir.glob(pattern) if p.with_name(p.name + ".minisig").exists())
    if found:
        sig_file = found[-1].with_name(found[-1].name + ".minisig")
        files[os_name] = (found[-1].name, base64.b64encode(sig_file.read_bytes()).decode())

platforms = {}
for key, entry in doc["platforms"].items():
    os_name = key.split("-")[0]
    if os_name in files:
        name, sig = files[os_name]
        entry["url"] = f"{base_url}/{name.replace(' ', '%20')}"
        entry["signature"] = sig
        platforms[key] = entry
doc["platforms"] = platforms
print(json.dumps(doc, indent=2))
