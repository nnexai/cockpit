#!/usr/bin/env python3
"""Hermetic safety-boundary tests for explicit mutation targets."""

from __future__ import annotations

import contextlib
import importlib.util
import io
import json
import os
import select
import signal
import socket
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("quality_mutation", Path(__file__).with_name("mutation.py"))
assert SPEC and SPEC.loader
mutation = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(mutation)


class TargetBoundaryTests(unittest.TestCase):
    def setUp(self) -> None:
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name).resolve()
        self.root_patch = patch.object(mutation, "ROOT", self.root)
        self.root_patch.start()
        self.addCleanup(self.root_patch.stop)

    def source(self, name: str) -> Path:
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("export const value = 1;\n", encoding="utf-8")
        return path

    def test_source_language_and_excluded_targets(self) -> None:
        ts = "src/module.ts"
        rust = "crates/example/src/module.rs"
        self.source(ts)
        self.source(rust)
        self.assertEqual(mutation.target_path(ts, "ts"), ts)
        self.assertEqual(mutation.target_path(rust, "rust"), rust)
        for path, language in ((ts, "rust"), (rust, "ts"),
                               ("src/module.test.ts", "ts"), ("src/module.d.ts", "ts"),
                               ("src/protocol/generated/module.ts", "ts"),
                               ("crates/example/tests/module.rs", "rust"),
                               ("crates/example/src/tests.rs", "rust")):
            self.source(path)
            with self.subTest(path=path, language=language), self.assertRaises(mutation.gate.QualityError):
                mutation.target_path(path, language)

    def test_canonical_literal_existing_files_only(self) -> None:
        source = self.source("src/module.ts")
        self.source("src/[module].ts")
        for path in (str(source), "./src/module.ts", "src/../src/module.ts",
                     "src//module.ts", "src/[module].ts", "src/missing.ts", "src/*.ts"):
            with self.subTest(path=path), self.assertRaises(mutation.gate.QualityError):
                mutation.target_path(path, "ts")

    def test_symlink_file_and_parent_rejected(self) -> None:
        source = self.source("src/module.ts")
        (self.root / "src/alias.ts").symlink_to(source)
        (self.root / "src/linked").symlink_to(source.parent, target_is_directory=True)
        for path in ("src/alias.ts", "src/linked/module.ts"):
            with self.subTest(path=path), self.assertRaises(mutation.gate.QualityError):
                mutation.target_path(path, "ts")

    def test_missing_target_and_unbounded_budget_rejected(self) -> None:
        self.source("src/module.ts")
        quality = self.root / "quality"
        quality.mkdir()
        (quality / "config.json").write_text('{"limits":{"workers":1}}', encoding="utf-8")
        commands = [["ts"], ["rust"],
                    ["ts", "--file", "src/module.ts", "--budget-minutes", "nan"],
                    ["ts", "--file", "src/module.ts", "--budget-minutes", "inf"],
                    ["ts", "--file", "src/module.ts", "--jobs", "0"]]
        for command in commands:
            with self.subTest(command=command), contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(mutation.main(command), 3)


@unittest.skipUnless(hasattr(os, "pidfd_open"), "Linux pidfds are required for process-exit proof")
class ProcessCleanupTests(unittest.TestCase):
    def test_sigterm_stops_worker_and_stubborn_descendant(self) -> None:
        # The descendant acknowledges readiness only after ignoring SIGTERM.
        # PID descriptors prove both workers exited, without sleeps or PID reuse.
        descendant = """
import json, os, signal, socket, sys
signal.signal(signal.SIGTERM, signal.SIG_IGN)
with socket.socket(socket.AF_UNIX) as connection:
    connection.connect(sys.argv[1])
    connection.sendall(json.dumps([os.getppid(), os.getpid()]).encode())
while True:
    signal.pause()
"""
        worker = """
import subprocess, sys, signal
subprocess.Popen([sys.executable, '-c', sys.argv[1], sys.argv[2]])
while True:
    signal.pause()
"""
        driver = """
import runpy, sys, time
from pathlib import Path
module = runpy.run_path(sys.argv[1])
module['execute'].__globals__['ROOT'] = Path(sys.argv[2])
try:
    module['execute']([sys.executable, '-c', sys.argv[3], sys.argv[4], sys.argv[5]],
                      Path(sys.argv[2]) / 'tool.log', time.monotonic() + 60)
except module['RunInterrupted']:
    sys.exit(2)
"""
        with tempfile.TemporaryDirectory() as directory, socket.socket(socket.AF_UNIX) as listener:
            address = str(Path(directory) / "ready.sock")
            listener.bind(address)
            listener.listen(1)
            listener.settimeout(10)
            process = subprocess.Popen([sys.executable, "-c", driver, str(Path(mutation.__file__).resolve()),
                                        directory, worker, descendant, address],
                                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            workers = []
            descriptors = []
            try:
                with listener.accept()[0] as connection:
                    connection.settimeout(10)
                    chunks = []
                    while chunk := connection.recv(4096):
                        chunks.append(chunk)
                    workers = json.loads(b"".join(chunks))
                descriptors = [os.pidfd_open(pid) for pid in workers]
                process.terminate()
                self.assertEqual(process.wait(timeout=10), 2)
                for descriptor in descriptors:
                    self.assertEqual(select.select([descriptor], [], [], 10)[0], [descriptor])
            finally:
                if workers:
                    try:
                        os.killpg(workers[0], signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                if process.poll() is None:
                    process.terminate()
                    try:
                        process.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait()
                for descriptor in descriptors:
                    os.close(descriptor)


if __name__ == "__main__":
    unittest.main()
