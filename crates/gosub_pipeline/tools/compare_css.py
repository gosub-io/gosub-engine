#!/usr/bin/env python3
"""
Compare computed CSS properties between gosub and Firefox.

Usage:
    python3 compare_css.py <gosub.json> <firefox.json>

gosub.json  — produced by running with GOSUB_DUMP_CSS=/tmp/gosub.json
firefox.json — produced in the browser console:
    copy(JSON.stringify([...document.querySelectorAll('*')].map(el => ({
        tag: el.tagName.toLowerCase(), id: el.id, class: el.className,
        styles: Object.fromEntries([...getComputedStyle(el)].map(p => [p, getComputedStyle(el)[p]]))
    })), null, 2))
"""

import json
import re
import sys
from collections import defaultdict

# Elements the gosub render tree skips entirely
INVISIBLE_TAGS = {"head", "script", "style", "meta", "link", "title", "svg", "noscript"}

# Default root font size used for em→px conversion
BASE_FONT_PX = 16.0


def parse_color(s: str):
    """Return (r, g, b, a) floats 0-255 / 1.0, or None."""
    s = s.strip()
    m = re.match(r"rgba?\(\s*([\d.]+)\s*,\s*([\d.]+)\s*,\s*([\d.]+)(?:\s*,\s*([\d.]+))?\s*\)", s)
    if m:
        r, g, b = float(m.group(1)), float(m.group(2)), float(m.group(3))
        a = float(m.group(4)) if m.group(4) is not None else 1.0
        return (r, g, b, a)
    return None


def normalize_value(prop: str, val: str, base_font_px: float = BASE_FONT_PX) -> str:
    """Normalize a CSS value to a canonical form for comparison."""
    val = val.strip()

    # em / rem → px
    m = re.fullmatch(r"([-+]?[\d.]+)em", val)
    if m:
        return f"{float(m.group(1)) * base_font_px:.4g}px"
    m = re.fullmatch(r"([-+]?[\d.]+)rem", val)
    if m:
        return f"{float(m.group(1)) * base_font_px:.4g}px"

    # Colors: canonicalize to rgba(r, g, b, a) with rounded integers
    c = parse_color(val)
    if c is not None:
        r, g, b, a = c
        if a == 1.0:
            return f"rgb({int(round(r))}, {int(round(g))}, {int(round(b))})"
        else:
            return f"rgba({int(round(r))}, {int(round(g))}, {int(round(b))}, {a:.4g})"

    # Strip trailing .0 from pixel values like "8.0px" → "8px"
    m = re.fullmatch(r"([-+]?[\d]+)\.0+px", val)
    if m:
        return f"{m.group(1)}px"

    return val


def match_key(el: dict) -> tuple:
    return (el["tag"], el.get("id", ""), el.get("class", ""))


def filter_firefox(elements: list) -> list:
    return [e for e in elements if e["tag"] not in INVISIBLE_TAGS]


def build_index(elements: list) -> dict:
    """Group elements by (tag, id, class), preserving order within each group."""
    idx = defaultdict(list)
    for el in elements:
        idx[match_key(el)].append(el)
    return idx


def compare(gosub_path: str, firefox_path: str):
    gosub_elements = json.load(open(gosub_path))
    firefox_elements = filter_firefox(json.load(open(firefox_path)))

    firefox_idx = build_index(firefox_elements)
    # Track how many times each key has been consumed
    firefox_consumed = defaultdict(int)

    total_props = 0
    mismatches = []
    unmatched_gosub = []

    for gsub in gosub_elements:
        key = match_key(gsub)
        ff_group = firefox_idx.get(key, [])
        pos = firefox_consumed[key]
        firefox_consumed[key] += 1

        if pos >= len(ff_group):
            unmatched_gosub.append(gsub)
            continue

        ff = ff_group[pos]

        for css_prop, gsub_val in gsub["styles"].items():
            ff_val = ff["styles"].get(css_prop)
            if ff_val is None:
                continue  # Firefox doesn't have this property (shouldn't happen for standard props)

            norm_gsub = normalize_value(css_prop, gsub_val)
            norm_ff   = normalize_value(css_prop, ff_val)
            total_props += 1

            if norm_gsub != norm_ff:
                mismatches.append({
                    "element": f"<{gsub['tag']}> id={gsub.get('id','')!r} class={gsub.get('class','')!r} pos={pos}",
                    "property": css_prop,
                    "gosub":   gsub_val,
                    "firefox": ff_val,
                    "gosub_norm":   norm_gsub,
                    "firefox_norm": norm_ff,
                })

    # ── Report ──────────────────────────────────────────────────────────────
    print(f"Compared {len(gosub_elements)} gosub elements against {len(firefox_elements)} Firefox elements")
    print(f"Checked {total_props} property values\n")

    if unmatched_gosub:
        print(f"WARNING: {len(unmatched_gosub)} gosub element(s) had no Firefox counterpart:")
        for el in unmatched_gosub:
            print(f"  <{el['tag']}> id={el.get('id','')!r} class={el.get('class','')!r}")
        print()

    if not mismatches:
        print("All checked properties match!")
        return

    # Group mismatches by element
    by_element = defaultdict(list)
    for m in mismatches:
        by_element[m["element"]].append(m)

    print(f"MISMATCHES ({len(mismatches)} total across {len(by_element)} element(s)):\n")
    for elem, ms in by_element.items():
        print(f"  {elem}")
        for m in ms:
            print(f"    {m['property']:35s}  gosub={m['gosub_norm']!r:20s}  firefox={m['firefox_norm']!r}")
        print()


if __name__ == "__main__":
    if len(sys.argv) != 3:
        print(__doc__)
        sys.exit(1)
    compare(sys.argv[1], sys.argv[2])
