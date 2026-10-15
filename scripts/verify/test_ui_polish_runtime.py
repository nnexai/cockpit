"""Disposable startup readiness regressions using owned local subprocesses only."""
import contextlib
import io
import json
import os
from pathlib import Path
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

try:
    from . import ui_polish_runtime as runtime
except ImportError:
    import ui_polish_runtime as runtime


class RuntimeReadinessTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="cpol-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.children = {}
        self.addCleanup(self.stop_children)
        self.stdout = io.StringIO()
        self.stderr = io.StringIO()
        self.gateway_source = ""
        original_launch = runtime.launch

        def launch(root, name, argv, env):
            if name == "herdr":
                argv = [sys.executable, "-c",
                        "from pathlib import Path; import sys,time; "
                        "p=Path(sys.argv[1]); p.parent.mkdir(parents=True); p.touch(); time.sleep(30)",
                        json.loads((root / "runtime.json").read_text())["socket"]]
            elif name == "gateway":
                argv = [sys.executable, "-u", "-c",
                        "from pathlib import Path\nimport json,sys,time\nroot=Path(sys.argv[1])\n"
                        "while not (root / 'release-gateway').exists(): time.sleep(.01)\n"
                        + self.gateway_source, str(root)]
            child = original_launch(root, name, argv, env)
            self.children[name] = child
            if name == "gateway":
                (root / "release-gateway").touch()
            return child

        for manager in (patch.object(runtime.tempfile, "mkdtemp", return_value=str(self.root)),
                        patch.object(runtime, "launch", side_effect=launch),
                        patch.object(runtime, "rpc", return_value={"result": {"workspace_id": "test"}}),
                        contextlib.redirect_stdout(self.stdout),
                        contextlib.redirect_stderr(self.stderr)):
            self.enterContext(manager)

    def stop_children(self):
        for child in self.children.values():
            if child.poll() is None:
                child.kill()
            child.wait(timeout=5)

    def test_start_publishes_gateway_origin_only_after_delayed_http_ready(self):
        self.gateway_source = """
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        assert self.path == '/'
        (root / 'http-served').touch()
        self.send_response(200)
        self.end_headers()
server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
origin = f'http://127.0.0.1:{server.server_port}'
print('listening ' + origin, flush=True)
time.sleep(.3)
assert 'gateway_url' not in json.loads((root / 'runtime.json').read_text())
server.serve_forever()
"""
        # The readiness request must bypass ambient proxy configuration.
        with patch.dict(os.environ, {"http_proxy": "http://127.0.0.1:1", "HTTP_PROXY": "http://127.0.0.1:1",
                                     "no_proxy": "", "NO_PROXY": ""}):
            runtime.start()
        ledger = json.loads((self.root / "runtime.json").read_text())
        origin = ledger["gateway_url"]
        self.assertEqual(self.stdout.getvalue(), f"{self.root}\n")
        self.assertEqual(self.stderr.getvalue(), "")
        self.assertTrue((self.root / "http-served").exists())
        self.assertIn(f"listening {origin}\n", (self.root / "gateway.log").read_text())
        self.assertTrue(origin.startswith("http://127.0.0.1:"))
        self.assertFalse(origin.endswith("/"))
        self.assertNotEqual(origin, ledger["fixture_url"])
        self.assertTrue(ledger["fixture_url"].endswith("/"))
        manifest = json.loads((self.root / runtime.fixture_resources.MANIFEST).read_text())
        self.assertEqual([entry["name"] for entry in manifest["processes"]], ["fixture", "herdr", "gateway"])

    def assert_retained_failure(self):
        self.assertEqual(self.stdout.getvalue(), "")
        self.assertIn(f"evidence retained at {self.root}", self.stderr.getvalue())
        self.assertIn(f"python3 {Path(runtime.__file__).resolve()} stop {self.root}", self.stderr.getvalue())
        self.assertNotIn("gateway_url", json.loads((self.root / "runtime.json").read_text()))
        self.assertTrue((self.root / runtime.fixture_resources.MANIFEST).exists())
        self.assertIsNone(self.children["fixture"].poll())
        self.assertIsNone(self.children["herdr"].poll())

    def test_start_reports_early_gateway_exit_and_retains_evidence(self):
        self.gateway_source = "print('boom', flush=True)\nraise SystemExit(3)\n"
        started = time.monotonic()
        with self.assertRaisesRegex(RuntimeError, "gateway exited with 3 before readiness") as raised:
            runtime.start()
        self.assertLess(time.monotonic() - started, 5)
        self.assertIn("boom", str(raised.exception))
        self.assert_retained_failure()

    def test_start_bounds_never_ready_gateway_and_retains_children(self):
        self.gateway_source = "print('waiting forever', flush=True)\ntime.sleep(30)\n"
        started = time.monotonic()
        with patch.object(runtime, "GATEWAY_READY_SECONDS", .3):
            with self.assertRaisesRegex(RuntimeError, "Cockpit gateway: not ready after 0.3s") as raised:
                runtime.start()
        self.assertLess(time.monotonic() - started, 5)
        self.assertIn("waiting forever", str(raised.exception))
        self.assertIsNone(self.children["gateway"].poll())
        self.assert_retained_failure()


if __name__ == "__main__":
    unittest.main()
