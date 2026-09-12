#!/usr/bin/env python3
"""Embed the canonical current/index.json in index.html for file:// viewing."""

import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
INDEX = ROOT / "index.json"
HTML = ROOT / "index.html"
START = "<!-- atlas-manifest:start -->"
END = "<!-- atlas-manifest:end -->"

manifest = json.dumps(json.loads(INDEX.read_text()), ensure_ascii=False, separators=(",", ":"))
manifest = manifest.replace("</", "<\\/")
page = HTML.read_text()
start = page.index(START) + len(START)
end = page.index(END, start)
HTML.write_text(page[:start] + "\n    <script id=\"atlas-data\" type=\"application/json\">" + manifest + "</script>\n    " + page[end:])
print(f"embedded {len(json.loads(INDEX.read_text())['shots'])} shots from {INDEX}")
