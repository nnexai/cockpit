#!/usr/bin/env python3
"""Heuristic production-function length report for the cleanup plan.

Counts brace-delimited Rust `fn` bodies (stopping at the first `#[cfg(test)]`)
and TypeScript `function`/arrow-const bodies. Test files, generated protocol
code and history directories are excluded. Numbers are approximate; use them
to compare before/after, not as exact sizes.

Usage: python3 planning/code-cleanup-2026-10-09/measure_functions.py [--top N]
"""
import re
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
EXCLUDED = re.compile(r"^(archive|planning|research|poc|spikes|\.audit)/|/generated/")
TEST_FILE = re.compile(r"(test|\.test\.|_tests\.rs$|/tests/)")
RUST_FN = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:const\s+)?(?:unsafe\s+)?fn\s+(\w+)")
TS_FN = re.compile(
    r"^\s*(?:export\s+)?(?:default\s+)?(?:async\s+)?function\s+(\w+)"
    r"|^\s*(?:export\s+)?const\s+(\w+)\s*=\s*(?:async\s*)?\("
)
STRIP = re.compile(r'"(?:\\.|[^"\\])*"|\'(?:\\.|[^\'\\])\'|//.*')


def body_length(lines, start):
    depth, started = 0, False
    for j in range(start, len(lines)):
        text = STRIP.sub("", lines[j])
        for ch in text:
            if ch == "{":
                depth, started = depth + 1, True
            elif ch == "}":
                depth -= 1
        if started and depth <= 0:
            return j - start + 1
        if not started and (j > start + 15 or text.rstrip().endswith(";")):
            return None
    return None


def functions():
    tracked = subprocess.check_output(["git", "ls-files"], cwd=REPO, text=True).splitlines()
    for path in tracked:
        if not re.search(r"\.(rs|tsx?)$", path) or EXCLUDED.search(path) or TEST_FILE.search(path):
            continue
        lines = (REPO / path).read_text(encoding="utf8").split("\n")
        rust = path.endswith(".rs")
        end = next((i for i, l in enumerate(lines) if rust and re.match(r"^\s*#\[cfg\(test\)\]", l)), len(lines))
        for i in range(end):
            match = (RUST_FN if rust else TS_FN).match(lines[i])
            if not match:
                continue
            length = body_length(lines, i)
            if length:
                yield length, path, i + 1, next(g for g in match.groups() if g)


def main():
    top = int(sys.argv[sys.argv.index("--top") + 1]) if "--top" in sys.argv else 40
    found = sorted(functions(), reverse=True)
    counts = {limit: sum(1 for f in found if f[0] > limit) for limit in (80, 150, 300)}
    print(f"production functions: {len(found)}; over 80/150/300 lines: {counts[80]}/{counts[150]}/{counts[300]}")
    for length, path, line, name in found[:top]:
        print(f"{length:5d}  {path}:{line}  {name}")


if __name__ == "__main__":
    main()
