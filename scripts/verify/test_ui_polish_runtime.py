"""Disposable startup readiness regressions using owned local subprocesses only."""
import contextlib
import io
import json
import os
from pathlib import Path
import struct
import sys
import tempfile
import time
import unittest
import uuid
from unittest.mock import MagicMock, patch

try:
    from . import ui_polish_runtime as runtime
except ImportError:
    import ui_polish_runtime as runtime


class RuntimeOmpAuthEnvironmentTests(unittest.TestCase):
    def test_default_isolation_and_explicit_auth_inheritance(self):
        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory)
            agent = home / ".omp" / "agent"
            agent.mkdir(parents=True)
            (agent / "agent.db").touch()
            fixture = home / "fixture"
            fixture.mkdir()
            with patch.dict(os.environ, {"HOME": str(home), "OMP_PROFILE": "default",
                                         "PI_PROFILE": "default", "OMP_AUTH_BROKER_TOKEN": "secret"}, clear=True):
                private = runtime.environment(fixture)
                self.assertEqual(private["PI_CODING_AGENT_DIR"], str(fixture / "omp-agent"))
                resolved = runtime.inherited_omp_agent_dir()
                inherited = runtime.environment(fixture, resolved)
                self.assertEqual(inherited["PI_CODING_AGENT_DIR"], str(agent))
                self.assertEqual(inherited["PI_CODING_AGENT_SESSION_DIR"], str(fixture / "omp-sessions"))
                self.assertEqual(inherited["PI_NATIVES_DIR"], str(fixture / "native-cache"))
                self.assertEqual(inherited["HOME"], str(fixture))
                self.assertNotIn("OMP_PROFILE", inherited)
                self.assertNotIn("PI_PROFILE", inherited)
                self.assertNotIn("OMP_AUTH_BROKER_TOKEN", inherited)

    def test_rejects_named_profiles_brokers_and_xdg_migrated_store(self):
        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory)
            agent = home / ".omp" / "agent"
            agent.mkdir(parents=True)
            (agent / "agent.db").touch()
            (home / "data" / "omp").mkdir(parents=True)
            for extra in ({"OMP_PROFILE": "work"}, {"PI_PROFILE": "work"},
                          {"OMP_AUTH_BROKER_URL": "http://127.0.0.1"},
                          {"XDG_DATA_HOME": str(home / "data")}):
                with self.subTest(extra=extra), patch.dict(os.environ, {"HOME": str(home), **extra}, clear=True):
                    with self.assertRaises(ValueError):
                        runtime.inherited_omp_agent_dir()

class LifecycleWebStartTests(unittest.TestCase):
    def test_web_start_fences_owner_and_sends_ephemeral_config(self):
        with tempfile.TemporaryDirectory(prefix="cpol-") as directory:
            root = Path(directory)
            instance = str(uuid.uuid4())
            web = {"instance_id": str(uuid.uuid4()), "config": {"bind": "127.0.0.1:12345"}}
            client = MagicMock()
            client.__enter__.return_value = client
            client.getsockopt.return_value = struct.pack("3i", os.getpid(), os.getuid(), os.getgid())
            client.makefile.return_value = io.BytesIO(json.dumps({"ok": True, "info": web}).encode() + b"\n")
            with patch.object(runtime.socket, "socket", return_value=client):
                self.assertEqual(runtime.start_web(root, instance, os.getpid()), web)
            client.connect.assert_called_once_with(str(root / "cockpit-state/backend/lifecycle.v1.sock"))
            request = json.loads(client.sendall.call_args.args[0])
            self.assertEqual(request["op"], "web_start")
            self.assertEqual(request["expected_instance"], instance)
            self.assertEqual(request["payload"], {"bind": "127.0.0.1:0",
                "static_dir": str((runtime.REPO / "dist").resolve()),
                "config_file": str((root / "cockpit.toml").resolve())})

    def test_web_start_refuses_wrong_peer_before_request(self):
        client = MagicMock()
        client.__enter__.return_value = client
        client.getsockopt.return_value = struct.pack("3i", os.getpid() + 1, os.getuid(), os.getgid())
        with patch.object(runtime.socket, "socket", return_value=client):
            with self.assertRaisesRegex(ValueError, "socket peer differs"):
                runtime.start_web(Path("/tmp/cpol-test"), str(uuid.uuid4()), os.getpid())
        client.sendall.assert_not_called()



class RuntimeReadinessTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="cpol-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.children = {}
        self.addCleanup(self.stop_children)
        self.stdout = io.StringIO()
        self.stderr = io.StringIO()
        self.service_source = ""
        self.owner_available = True
        self.web_calls = []
        self.owner_instance = str(uuid.uuid4())
        self.web_instance = str(uuid.uuid4())
        original_launch = runtime.launch

        def launch(root, name, argv, env):
            if name == "herdr":
                argv = [sys.executable, "-c",
                        "from pathlib import Path; import sys,time; "
                        "p=Path(sys.argv[1]); p.parent.mkdir(parents=True); p.touch(); time.sleep(30)",
                        json.loads((root / "runtime.json").read_text())["socket"]]
            elif name == "service":
                self.assertEqual(argv[1], "service")
                self.assertEqual(argv[-1], "run")
                argv = [sys.executable, "-u", "-c",
                        "from pathlib import Path\nimport json,sys,time\nroot=Path(sys.argv[1])\n"
                        "while not (root / 'release-service').exists(): time.sleep(.01)\n"
                        + self.service_source, str(root)]
            child = original_launch(root, name, argv, env)
            self.children[name] = child
            if name == "service":
                (root / "release-service").touch()
            return child

        def backend_status(root, endpoint):
            if not self.owner_available:
                raise ValueError("service not ready")
            return {"state": "running", "instance": {
                "instance_id": self.owner_instance, "pid": self.children["service"].pid,
                "herdr": endpoint}}

        def start_web(root, instance_id, owner_pid):
            self.web_calls.append((instance_id, owner_pid))
            deadline = time.monotonic() + 5
            while not (root / "web-port").exists():
                if time.monotonic() > deadline:
                    raise RuntimeError("test listener did not publish a port")
                time.sleep(.01)
            port = (root / "web-port").read_text()
            return {"instance_id": self.web_instance,
                "config": {"bind": f"127.0.0.1:{port}", "static_dir": str((runtime.REPO / "dist").resolve()),
                           "config_file": str((root / "cockpit.toml").resolve())}}

        for manager in (patch.object(runtime.tempfile, "mkdtemp", return_value=str(self.root)),
                        patch.object(runtime, "launch", side_effect=launch),
                        patch.object(runtime, "rpc", return_value={"result": {"workspace_id": "test"}}),
                        patch.object(runtime, "backend_status", side_effect=backend_status),
                        patch.object(runtime, "start_web", side_effect=start_web),
                        contextlib.redirect_stdout(self.stdout),
                        contextlib.redirect_stderr(self.stderr)):
            self.enterContext(manager)

    def stop_children(self):
        for child in self.children.values():
            if child.poll() is None:
                child.kill()
            child.wait(timeout=5)

    def test_start_publishes_gateway_origin_only_after_delayed_http_ready(self):
        self.service_source = """
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        assert self.path == '/'
        (root / 'http-served').touch()
        self.send_response(200)
        self.end_headers()
server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
(root / 'web-port').write_text(str(server.server_port))
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
        self.assertEqual(ledger["web"]["instance_id"], self.web_instance)
        self.assertEqual(ledger["backend"]["instance_id"], self.owner_instance)
        self.assertEqual(self.web_calls, [(self.owner_instance, self.children["service"].pid)])
        self.assertTrue(origin.startswith("http://127.0.0.1:"))
        self.assertFalse(origin.endswith("/"))
        self.assertNotEqual(origin, ledger["fixture_url"])
        self.assertTrue(ledger["fixture_url"].endswith("/"))
        manifest = json.loads((self.root / runtime.fixture_resources.MANIFEST).read_text())
        self.assertEqual([entry["name"] for entry in manifest["processes"]], ["fixture", "herdr", "service"])
        with patch.object(runtime, "service_command") as stop_command:
            runtime.stop_backend(self.root)
            stop_command.assert_called_once_with(
                self.root, ledger["backend"]["herdr"], "stop", "--operator",
                "--expected-instance", self.owner_instance)
        wrong_status = {"state": "running", "instance": {
            "instance_id": self.owner_instance, "pid": self.children["service"].pid + 1,
            "herdr": ledger["backend"]["herdr"]}}
        with patch.object(runtime, "backend_status", return_value=wrong_status), \
                patch.object(runtime, "service_command") as stop_command:
            with self.assertRaisesRegex(ValueError, "process ownership is unproven"):
                runtime.stop_backend(self.root)
            stop_command.assert_not_called()

    def test_listener_timeout_retains_backend_receipt_without_publishing_url(self):
        self.service_source = """
from http.server import ThreadingHTTPServer, BaseHTTPRequestHandler
server = ThreadingHTTPServer(('127.0.0.1', 0), BaseHTTPRequestHandler)
(root / 'web-port').write_text(str(server.server_port))
time.sleep(30)
"""
        with patch.object(runtime, "GATEWAY_READY_SECONDS", .3):
            with self.assertRaisesRegex(RuntimeError, "Cockpit web listener: not ready"):
                runtime.start()
        self.assert_retained_failure()
        ledger = json.loads((self.root / "runtime.json").read_text())
        self.assertEqual(ledger["backend"]["instance_id"], self.owner_instance)

    def assert_retained_failure(self):
        self.assertEqual(self.stdout.getvalue(), "")
        self.assertIn(f"evidence retained at {self.root}", self.stderr.getvalue())
        self.assertIn(f"python3 {Path(runtime.__file__).resolve()} stop {self.root}", self.stderr.getvalue())
        self.assertNotIn("gateway_url", json.loads((self.root / "runtime.json").read_text()))
        self.assertTrue((self.root / runtime.fixture_resources.MANIFEST).exists())
        self.assertIsNone(self.children["fixture"].poll())
        self.assertIsNone(self.children["herdr"].poll())

    def test_start_reports_early_service_exit_and_retains_evidence(self):
        self.owner_available = False
        self.service_source = "print('boom', flush=True)\nraise SystemExit(3)\n"
        started = time.monotonic()
        with self.assertRaisesRegex(RuntimeError, "service exited with 3 before readiness") as raised:
            runtime.start()
        self.assertLess(time.monotonic() - started, 5)
        self.assertIn("boom", str(raised.exception))
        self.assert_retained_failure()

    def test_start_bounds_never_ready_service_and_retains_children(self):
        self.owner_available = False
        self.service_source = "print('waiting forever', flush=True)\ntime.sleep(30)\n"
        started = time.monotonic()
        with patch.object(runtime, "GATEWAY_READY_SECONDS", .3):
            with self.assertRaisesRegex(RuntimeError, "Cockpit service: not ready after 0.3s") as raised:
                runtime.start()
        self.assertLess(time.monotonic() - started, 5)
        self.assertIn("waiting forever", str(raised.exception))
        self.assertIsNone(self.children["service"].poll())
        self.assert_retained_failure()


if __name__ == "__main__":
    unittest.main()
