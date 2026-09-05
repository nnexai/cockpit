#!/usr/bin/env python3
"""Bounded, harmless terminal fixture for temporal and input-capture checks.

The fixture never executes input.  Bytes read from stdin are copied unchanged to
``--output`` when input capture is enabled.  All terminal state changed by this
process is restored before exit.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import selectors
import shutil
import signal
import sys
import termios
import time
import tty
from pathlib import Path
from typing import BinaryIO, Sequence

MARKER = "COCKPIT-STABLE-MARKER"
_MAX_DURATION = 300.0
_MAX_OUTPUT = 8 * 1024 * 1024
_MAX_CAPTURE = 2 * 1024 * 1024
_MOUSE_RE = re.compile(rb"\x1b\[<(?P<button>\d+);(?P<x>\d+);(?P<y>\d+)(?P<action>[Mm])")


class FixtureError(ValueError):
    """A fixture argument or output path is unsafe or invalid."""


class BoundedWriter:
    def __init__(self, stream: BinaryIO, limit: int) -> None:
        self.stream = stream
        self.limit = limit
        self.count = 0

    def write(self, data: bytes) -> bool:
        if not data:
            return True
        remaining = self.limit - self.count
        if remaining <= 0:
            return False
        chunk = data[:remaining]
        pending = memoryview(chunk)
        while pending:
            written = self.stream.write(pending)
            if written is None or written <= 0:
                raise FixtureError("stdout stopped accepting fixture output")
            self.count += written
            pending = pending[written:]
        self.stream.flush()
        return len(chunk) == len(data)


def _bounded_number(value: str, *, name: str, minimum: float, maximum: float) -> float:
    try:
        parsed = float(value)
    except ValueError as error:
        raise FixtureError(f"{name} must be numeric") from error
    if not minimum <= parsed <= maximum:
        raise FixtureError(f"{name} must be between {minimum} and {maximum}")
    return parsed


def _output_path(raw: str) -> Path:
    if not raw or raw == "-":
        raise FixtureError("--output must be an explicit run-owned file path")
    path = Path(raw)
    if not path.is_absolute():
        raise FixtureError("--output must be an absolute run-owned path")
    if path.exists() and path.is_symlink():
        raise FixtureError("--output must not be a symlink")
    parent = path.parent
    if not parent.exists() or not parent.is_dir():
        raise FixtureError("the --output parent must already exist")
    return path


def _size() -> tuple[int, int]:
    columns, rows = shutil.get_terminal_size((80, 24))
    return max(20, columns), max(4, rows)


def _mouse_modes(enable: bool) -> bytes:
    # SGR coordinates are unambiguous and use the normal application mouse
    # reporting modes.  1002 preserves drag/release observations.
    return b"\x1b[?1002h\x1b[?1006h" if enable else b"\x1b[?1006l\x1b[?1002l"


def _consume_mouse(buffer: bytearray) -> list[dict[str, object]]:
    """Parse complete SGR reports while retaining a split trailing sequence."""
    observations: list[dict[str, object]] = []
    last_end = 0
    for match in _MOUSE_RE.finditer(bytes(buffer)):
        button = int(match.group("button"))
        observations.append(
            {
                "type": "mouse",
                "x": int(match.group("x")),
                "y": int(match.group("y")),
                "button": button & 3,
                "modifiers": {
                    "shift": bool(button & 4),
                    "alt": bool(button & 8),
                    "ctrl": bool(button & 16),
                    "motion": bool(button & 32),
                    "wheel": bool(button & 64),
                },
                "action": "press" if match.group("action") == b"M" else "release",
            }
        )
        last_end = match.end()
    remainder = bytes(buffer[last_end:])
    marker = remainder.rfind(b"\x1b")
    buffer.clear()
    if marker >= 0:
        buffer.extend(remainder[marker:][-64:])
    return observations




def _repaint(writer: BoundedWriter, mode: str, frame: int, *, initialize: bool) -> bool:
    columns, rows = _size()
    if mode == "scrollback":
        prefix = b"\x1b[1;97;44m" + MARKER.encode() + b"\x1b[0m "
        numbers = range(512) if initialize else (511 + frame,)
        payload = b"\x1b[r\x1b[2J\x1b[H" if initialize else b""
        payload += b"".join(prefix + f"scrollback line {number:06d}\x1b[K\r\n".encode() for number in numbers)
        return writer.write(payload)
    status = f"mode={mode} frame={frame:05d} size={columns}x{rows}"
    marker = b"\x1b[1;1H\x1b[1;97;44m" + MARKER.encode() + b"\x1b[0m"
    payload = b"\x1b[2J" if initialize else b""
    payload += f"\x1b[2;{rows}r".encode("ascii") + marker
    lines = [status, "Input is captured as bytes; it is never executed."]
    if mode == "agent":
        lines.extend(f"agent burst {frame:05d}.{index:02d}: deterministic text" for index in range(3))
    elif mode == "text":
        lines.append(f"text update {frame:05d}: stable marker remains visible")
    elif mode == "graphics":
        lines.append("graphics unsupported/parked; usable text fallback remains active")
    payload += b"\x1b[3;1H\x1b[2K" + "\r\n".join(lines).encode("utf-8")
    payload += b"\x1b[K"
    return writer.write(payload)


def _configure_terminal(capture_input: bool) -> tuple[bool, object | None]:
    if not capture_input:
        return False, None
    fd = sys.stdin.fileno()
    if not sys.stdin.isatty():
        os.set_blocking(fd, False)
        return False, None
    original = termios.tcgetattr(fd)
    tty.setraw(fd)
    return True, original


def _restore_terminal(original: object | None) -> None:
    if original is None or not sys.stdin.isatty():
        return
    try:
        termios.tcsetattr(sys.stdin.fileno(), termios.TCSADRAIN, original)
    except (OSError, termios.error):
        pass


def run(args: argparse.Namespace) -> int:
    duration = _bounded_number(args.duration, name="duration", minimum=0.01, maximum=_MAX_DURATION)
    interval = _bounded_number(args.interval, name="interval", minimum=0.005, maximum=10.0)
    output_path = _output_path(args.output)
    max_output = min(args.max_output, _MAX_OUTPUT)
    max_capture = min(args.max_capture, _MAX_CAPTURE)
    capture_input = args.capture_input or args.mode == "mouse"
    deadline = time.monotonic() + duration
    writer = BoundedWriter(sys.stdout.buffer, max_output)
    capture: BinaryIO | None = None
    captured = 0
    mouse_buffer = bytearray()
    selector: selectors.BaseSelector | None = None
    terminal_original: object | None = None
    resize = False

    def on_resize(_signum: int, _frame: object) -> None:
        nonlocal resize
        resize = True

    def on_terminate(_signum: int, _frame: object) -> None:
        raise KeyboardInterrupt


    old_winch = signal.getsignal(signal.SIGWINCH)
    old_term = signal.getsignal(signal.SIGTERM)
    old_hup = signal.getsignal(signal.SIGHUP)
    try:
        if capture_input:
            capture = output_path.open("wb")
            selector = selectors.DefaultSelector()
            _interactive, terminal_original = _configure_terminal(True)
            selector.register(sys.stdin, selectors.EVENT_READ)
            writer.write(_mouse_modes(True) if args.mode == "mouse" else b"")
        signal.signal(signal.SIGWINCH, on_resize)
        signal.signal(signal.SIGTERM, on_terminate)
        signal.signal(signal.SIGHUP, on_terminate)
        frame = 0
        next_emit = time.monotonic()
        while time.monotonic() < deadline:
            now = time.monotonic()
            if resize or frame == 0:
                resize = False
                if not _repaint(writer, args.mode, frame, initialize=frame == 0):
                    break
                frame += 1
            elif args.mode != "idle" and now >= next_emit:
                if not _repaint(writer, args.mode, frame, initialize=False):
                    break
                frame += 1
                next_emit = now + interval
            wait = min(0.05, max(0.0, deadline - time.monotonic()))
            if selector is not None:
                for key, _ in selector.select(wait):
                    if captured >= max_capture:
                        break
                    try:
                        data = os.read(key.fd, min(65536, max_capture - captured + 1))
                    except BlockingIOError:
                        continue
                    if not data:
                        selector.unregister(key.fileobj)
                        continue
                    if captured + len(data) > max_capture:
                        data = data[: max_capture - captured]
                    if data and capture is not None:
                        capture.write(data)
                        capture.flush()
                        captured += len(data)
                        if args.mode == "mouse":
                            mouse_buffer.extend(data)
                            for observation in _consume_mouse(mouse_buffer):
                                observation["timestamp"] = time.monotonic()
                                writer.write((json.dumps(observation, sort_keys=True) + "\n").encode())
            elif wait:
                time.sleep(wait)
    finally:
        signal.signal(signal.SIGWINCH, old_winch)
        signal.signal(signal.SIGTERM, old_term)
        signal.signal(signal.SIGHUP, old_hup)
        if capture_input:
            writer.write(_mouse_modes(False) if args.mode == "mouse" else b"")
        _restore_terminal(terminal_original)
        if selector is not None:
            selector.close()
        if capture is not None:
            capture.close()
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", choices=("idle", "text", "agent", "scrollback", "resize", "mouse", "graphics"), default="idle")
    parser.add_argument("--duration", default="10", help="bounded run duration in seconds (0.01..300)")
    parser.add_argument("--interval", default="0.1", help="update interval in seconds")
    parser.add_argument("--output", required=True, help="absolute run-owned path for exact captured stdin bytes")
    parser.add_argument("--capture-input", action="store_true", help="capture stdin without interpreting it")
    parser.add_argument("--max-output", type=int, default=512 * 1024, help="stdout byte limit")
    parser.add_argument("--max-capture", type=int, default=256 * 1024, help="captured stdin byte limit")
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    try:
        if args.max_output <= 0 or args.max_capture <= 0:
            raise FixtureError("byte limits must be positive")
        return run(args)
    except (FixtureError, OSError, termios.error) as error:
        print(f"terminal_fixture: rejected: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
