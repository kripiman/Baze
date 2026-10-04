#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Gabriel Piñones
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Invariants of the map style that a generic style validator does not know about.

The app loads data/styles/style.json with the MapLibre `pmtiles://` source pointing at a file it downloaded,
so the style must not depend on the network or on one device, must say where its data comes from, and must
only reference what it defines. Standard library only; run it from anywhere:

    python3 scripts/check-style.py [path/to/style.json]
"""

import json
import os
import re
import sys

DEFAULT = os.path.join(os.path.dirname(__file__), "..", "data", "styles", "style.json")
PLACEHOLDER = "{{PMTILES_URL}}"


def check(style):
    errors = []
    if style.get("version") != 8:
        errors.append("version must be 8")

    sources = style.get("sources") or {}
    if not sources:
        errors.append("the style has no sources")
    for name, source in sources.items():
        if source.get("type") == "vector":
            url = source.get("url", "")
            if url != f"pmtiles://{PLACEHOLDER}":
                errors.append(f"source {name}: url must be pmtiles://{PLACEHOLDER} (the app substitutes the local file), not {url!r}")
            attribution = source.get("attribution", "")
            if "OpenStreetMap" not in attribution:
                errors.append(f"source {name}: the attribution must credit OpenStreetMap contributors (ODbL)")
            if "OpenMapTiles" not in attribution:
                errors.append(f"source {name}: the attribution must credit OpenMapTiles (CC-BY 4.0)")

    # Nothing may point at a host or a path of one device: offline maps must not need either.
    text = json.dumps(style)
    for pattern, why in (
        (r"localhost|127\.0\.0\.1", "a loopback host"),
        (r"/data/user/\d+/", "a device-specific absolute path"),
        (r'"(?:sprite|glyphs)"\s*:\s*"http://', "a cleartext sprite or glyphs URL"),
    ):
        if re.search(pattern, text):
            errors.append(f"the style contains {why}")

    layers = style.get("layers") or []
    ids = [layer.get("id") for layer in layers]
    if len(ids) != len(set(ids)):
        errors.append("layer ids are not unique")
    for layer in layers:
        source = layer.get("source")
        if source is not None and source not in sources:
            errors.append(f"layer {layer.get('id')}: unknown source {source!r}")
        if layer.get("type") == "symbol" and not style.get("glyphs"):
            errors.append(f"layer {layer.get('id')}: a symbol layer needs `glyphs` (and the font files published)")
        if layer.get("type") != "background" and source is None:
            errors.append(f"layer {layer.get('id')}: missing source")
    if ("sprite" in style) and not any(layer.get("layout", {}).get("icon-image") for layer in layers):
        errors.append("`sprite` is declared but no layer uses an icon")
    return errors


def main():
    path = sys.argv[1] if len(sys.argv) > 1 else DEFAULT
    with open(path, encoding="utf-8") as f:
        style = json.load(f)
    errors = check(style)
    if errors:
        print("\n".join(f"  - {e}" for e in errors))
        return 1
    print(f"  ok: {path} ({len(style['layers'])} layers, {len(style['sources'])} source)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
