#!/usr/bin/env python3
"""Builds crates/capsule-core/data/pinyin.bin from mozillazg/pinyin-data's pinyin.txt (MIT).

Usage: gen_pinyin.py <pinyin.txt> <pinyin.bin>

Layout (little-endian u16 unless noted):
  syllable count S, list count L
  S syllables, 6 ASCII bytes each, NUL padded
  L + 1 offsets into the list data
  list data: syllable ids
  one list index + 1 per code point of RANGES (0 = no reading)
"""

import struct
import sys
import unicodedata

RANGES = [(0x3400, 0x4DBF), (0x4E00, 0x9FFF)]


def toneless(reading):
    s = unicodedata.normalize("NFD", reading.lower()).replace("u\u0308", "v")
    return "".join(c for c in s if "a" <= c <= "z")


def main(src, dst):
    readings = {}
    for line in open(src, encoding="utf-8"):
        line = line.split("#", 1)[0].strip()
        if not line:
            continue
        cp, values = line.split(":", 1)
        cp = int(cp.strip()[2:], 16)
        if not any(lo <= cp <= hi for lo, hi in RANGES):
            continue
        out = []
        for v in values.split(","):
            t = toneless(v.strip())
            if t and t not in out:
                out.append(t)
        if out:
            readings[cp] = out

    syllables = sorted({s for r in readings.values() for s in r})
    assert all(len(s) <= 6 for s in syllables)
    syl_id = {s: i for i, s in enumerate(syllables)}
    lists, list_id = [], {}
    for r in readings.values():
        key = tuple(syl_id[s] for s in r)
        if key not in list_id:
            list_id[key] = len(lists)
            lists.append(key)

    out = bytearray(struct.pack("<HH", len(syllables), len(lists)))
    for s in syllables:
        out += s.encode().ljust(6, b"\0")
    offset = 0
    for key in lists:
        out += struct.pack("<H", offset)
        offset += len(key)
    out += struct.pack("<H", offset)
    assert offset < 0x10000
    for key in lists:
        out += struct.pack(f"<{len(key)}H", *key)
    for lo, hi in RANGES:
        for cp in range(lo, hi + 1):
            r = readings.get(cp)
            out += struct.pack("<H", list_id[tuple(syl_id[s] for s in r)] + 1 if r else 0)
    open(dst, "wb").write(out)
    print(f"{len(syllables)} syllables, {len(lists)} lists, {len(readings)} chars, {len(out)} bytes")


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
