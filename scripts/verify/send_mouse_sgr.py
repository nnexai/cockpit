#!/usr/bin/env python3
"""Send SGR mouse reports through Herdr's pane.send_keys CLI.

Each report is split into Herdr key tokens: ``esc`` followed by one printable
character per token. The fixture must already have enabled SGR mouse reporting.
"""

from __future__ import annotations

import argparse
import shlex
import subprocess
import sys
import time
from typing import Sequence

BUTTON_CODES = {"left": 0, "middle": 1, "right": 2}
MODIFIER_BITS = {"shift": 4, "alt": 8, "ctrl": 16}
MAX_COORDINATE = 65_535
MAX_DELAY_MS = 60_000


def _bounded_int(value: str, *, name: str, minimum: int, maximum: int) -> int:
    try:
        parsed = int(value)
    except ValueError as error:
        raise argparse.ArgumentTypeError(f"{name} must be an integer") from error
    if not minimum <= parsed <= maximum:
        raise argparse.ArgumentTypeError(f"{name} must be between {minimum} and {maximum}")
    return parsed


def _report_keys(report: str) -> list[str]:
    """Convert one SGR report into semantic Herdr key arguments."""
    assert report.startswith("\x1b")
    return ["esc", *report[1:]]


def _modifier_bits(modifiers: Sequence[str]) -> int:
    bits = 0
    for name in modifiers:
        bits |= MODIFIER_BITS[name]
    return bits


def _command(session: str, pane: str, report: str) -> list[str]:
    return [
        "herdr",
        "--session",
        session,
        "pane",
        "send-keys",
        pane,
        *_report_keys(report),
    ]


def _send(session: str, pane: str, report: str, *, dry_run: bool) -> None:
    command = _command(session, pane, report)
    if dry_run:
        print(shlex.join(command))
        return
    subprocess.run(command, check=True)


def _report(button: int, x: int, y: int, action: str) -> str:
    return f"\x1b[<{button};{x};{y}{action}"


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Send one or more SGR mouse clicks to an explicit Herdr pane."
    )
    parser.add_argument("--session", required=True, help="explicit Herdr session name")
    parser.add_argument("--pane", required=True, help="explicit Herdr pane ID, for example w1:p1")
    parser.add_argument(
        "--button", choices=tuple(BUTTON_CODES), default="left", help="mouse button"
    )
    parser.add_argument(
        "--modifier",
        choices=tuple(MODIFIER_BITS),
        action="append",
        default=[],
        help="modifier bit to include; may be repeated",
    )
    parser.add_argument(
        "--x",
        type=lambda value: _bounded_int(value, name="x", minimum=1, maximum=MAX_COORDINATE),
        default=30,
        help="1-based SGR column (default: 30)",
    )
    parser.add_argument(
        "--y",
        type=lambda value: _bounded_int(value, name="y", minimum=1, maximum=MAX_COORDINATE),
        default=9,
        help="1-based SGR row (default: 9)",
    )
    parser.add_argument(
        "--action",
        choices=("click", "press", "release"),
        default="click",
        help="send a full click, press only, or release only",
    )
    parser.add_argument(
        "--delay-ms",
        type=lambda value: _bounded_int(value, name="delay-ms", minimum=0, maximum=MAX_DELAY_MS),
        default=50,
        help="delay between click press and release (default: 50)",
    )
    parser.add_argument(
        "--repeat",
        type=lambda value: _bounded_int(value, name="repeat", minimum=1, maximum=100),
        default=1,
        help="number of clicks to send (default: 1)",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="print Herdr commands without sending input",
    )
    return parser


def run(args: argparse.Namespace) -> int:
    button = BUTTON_CODES[args.button] | _modifier_bits(args.modifier)
    press = _report(button, args.x, args.y, "M")
    release = _report(button, args.x, args.y, "m")

    if args.action == "press":
        for _ in range(args.repeat):
            _send(args.session, args.pane, press, dry_run=args.dry_run)
        return 0
    if args.action == "release":
        for _ in range(args.repeat):
            _send(args.session, args.pane, release, dry_run=args.dry_run)
        return 0

    for index in range(args.repeat):
        _send(args.session, args.pane, press, dry_run=args.dry_run)
        if args.delay_ms and not args.dry_run:
            time.sleep(args.delay_ms / 1000)
        _send(args.session, args.pane, release, dry_run=args.dry_run)
        if index + 1 < args.repeat and args.delay_ms and not args.dry_run:
            time.sleep(args.delay_ms / 1000)
    return 0


def main(argv: Sequence[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    try:
        return run(args)
    except (OSError, subprocess.CalledProcessError) as error:
        print(f"send_mouse_sgr: Herdr command failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
