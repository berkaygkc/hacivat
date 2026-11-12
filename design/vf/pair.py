#!/usr/bin/env python3
"""Crop the same region from a reference and a candidate capture and stack them (ref on top).
usage: pair.py <tag> x0 y0 x1 y1 <out.png>   (device pixels; tag like dark-en-2x)"""
import sys
from PIL import Image

tag = sys.argv[1]
x0, y0, x1, y1 = map(int, sys.argv[2:6])
out = sys.argv[6]
r = Image.open(f"target/vf/ref-{tag}.png").convert("RGB")
c = Image.open(f"target/vf/cand-{tag}.png").convert("RGB")
box = (x0, y0, x1, y1)
w, h = x1 - x0, y1 - y0
im = Image.new("RGB", (w, h * 2 + 4), (255, 0, 255))
im.paste(r.crop(box), (0, 0))
im.paste(c.crop(box), (0, h + 4))
im.save(out)
