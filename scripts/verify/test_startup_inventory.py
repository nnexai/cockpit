#!/usr/bin/env python3
"""Focused fail-closed behavioral coverage for startup_inventory."""
from __future__ import annotations

import argparse
import hashlib
import json
import struct
import sys
import tempfile
import unittest
import zlib
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

try:
    from . import startup_inventory as inventory
except ImportError:
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import startup_inventory as inventory  # type: ignore[no-redef]


RUN_ID = "run-test-20260904"
SESSION = "run-test-20260904-browser"
SOCKET = "/tmp/run-test/herdr.sock"


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def png_bytes(width: int = 1) -> bytes:
    def chunk(kind: bytes, payload: bytes) -> bytes:
        return struct.pack(">I", len(payload)) + kind + payload + struct.pack(">I", zlib.crc32(kind + payload) & 0xffffffff)
    header = struct.pack(">IIBBBBB", width, 1, 8, 6, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(b"\x00" + b"\xff\x00\x00\xff" * width)) + chunk(b"IEND", b"")


def write_receipt(root: Path, name: str, value: object) -> Path:
    path = root / name
    path.write_text(json.dumps(value), encoding="utf-8")
    return path


def base_evidence(root: Path, client: str = "browser") -> dict[str, object]:
    fixture = root / "fixture.png"
    fixture.write_bytes(png_bytes())
    input_receipt = write_receipt(root, "input.json", {"session": SESSION, "exact": True, "expected_hex": "c3a9", "captured_hex": "c3a9"})
    observation = write_receipt(root, "observation.json", {"provenance": "main-observed-receipt", "client": client, "session": SESSION, "fixture_id": "fixture-1", "artifact_sha256": digest(fixture)})
    artifacts: dict[str, object] = {
        "visible_fixture": {"path": str(fixture), "sha256": digest(fixture)},
        "input_receipt": {"path": str(input_receipt), "sha256": digest(input_receipt)},
        "observation_receipt": {"path": str(observation), "sha256": digest(observation)},
    }
    evidence: dict[str, object] = {
        "schema_version": 1,
        "client": client,
        "run_id": RUN_ID,
        "session": SESSION,
        "source": {"commit": "41101de"},
        "build": {"commit": "41101de", "sha256": "1" * 64},
        "generated_client": {"version": "1", "sha256": "2" * 64},
        "display": {"name": ":191"},
        "renderer": {"name": "webkit"},
        "fixture": {"id": "fixture-1", "pane_id": "pane-1", "terminal_id": "terminal-1"},
        "backend": {"version": "0.8.2", "protocol": 20, "schema_version": 1, "schema_sha256": "3" * 64, "effective_config_sha256": "4" * 64},
        "capabilities": {name: True for name in ("mouse_input", "click_focus", "app_mode_pointer", "wheel_scroll", "graphics")},
        "artifacts": artifacts,
        "assertions": {"visible_fixture": True, "click_focus": True, "basic_input": True, "app_mouse": False},
    }
    if client == "native":
        binary = root / "Cockpit.AppImage"
        binary.write_bytes(b"\x7fELF" + b"\x02" * 64 + b"AI\x02")
        binary.chmod(0o755)
        launch = write_receipt(root, "launch.json", {"client": "native", "session": SESSION, "binary_sha256": digest(binary), "status": "observed"})
        artifacts["launch_receipt"] = {"path": str(launch), "sha256": digest(launch)}
        evidence["client_binary"] = {"path": str(binary), "sha256": digest(binary)}
    return evidence


class StartupInventoryTests(unittest.TestCase):
    def test_guard_rejection_happens_before_command_or_host_effects(self) -> None:
        args = argparse.Namespace(ledger="missing.json", run_id=RUN_ID, session="default", client="browser", host_url="http://127.0.0.1:1", host_evidence=None)
        with mock.patch.object(inventory, "load_ledger", return_value=object()), mock.patch.object(inventory, "prepare_subprocess", side_effect=inventory.ResourceGuardError("default rejected")), mock.patch.object(inventory, "_run_prepared") as run, mock.patch.object(inventory, "_check_browser_host") as host:
            report, code = inventory.collect(args)
        self.assertEqual(code, inventory.EXIT_INCONCLUSIVE)
        self.assertEqual(report["guard"]["status"], "rejected")
        run.assert_not_called()
        host.assert_not_called()

    def test_stable_snapshot_without_session_id_is_accepted(self) -> None:
        executable = SimpleNamespace(version="0.8.2", protocol=20, schema_version=1)
        target = SimpleNamespace(session=SimpleNamespace(name=SESSION), socket_path=SOCKET)
        status = {"status": "running", "running": True, "version": "0.8.2", "protocol": 20, "compatible": True, "socket": SOCKET, "session": SESSION, "restart_needed": False, "capabilities": {"live_handoff": True, "detached_server_daemon": True}}
        sessions = {"sessions": [{"id": SESSION, "running": True, "default": False}]}
        report, missing, mismatches = inventory._check_backend(status, sessions, {"version": "0.8.2", "protocol": 20}, {"protocol": 20, "schema_version": 1}, target, executable, "a" * 64, "a" * 64)
        self.assertEqual(report["status"], "pass")
        self.assertEqual(missing, [])
        self.assertEqual(mismatches, [])

    def test_wrong_status_target_fails(self) -> None:
        executable = SimpleNamespace(version="0.8.2", protocol=20, schema_version=1)
        target = SimpleNamespace(session=SimpleNamespace(name=SESSION), socket_path=SOCKET)
        status = {"status": "running", "running": True, "version": "0.8.2", "protocol": 20, "compatible": True, "socket": "/foreign.sock", "session": "foreign", "restart_needed": False, "capabilities": {"live_handoff": True, "detached_server_daemon": True}}
        _report, missing, mismatches = inventory._check_backend(status, {"sessions": [{"id": SESSION, "running": True, "default": False}]}, {"version": "0.8.2", "protocol": 20}, {"protocol": 20, "schema_version": 1}, target, executable, "a" * 64, "a" * 64)
        self.assertEqual(missing, [])
        self.assertIn("server.session", mismatches)
        self.assertIn("server.socket", mismatches)

    def test_invalid_image_is_inconclusive(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            evidence = root / "browser.json"
            data = base_evidence(root)
            fixture = root / "fixture.png"
            fixture.write_bytes(b"not-an-image")
            data["artifacts"]["visible_fixture"] = {"path": str(fixture), "sha256": digest(fixture)}  # type: ignore[index]
            observation = write_receipt(root, "observation.json", {"provenance": "main-observed-receipt", "client": "browser", "session": SESSION, "fixture_id": "fixture-1", "artifact_sha256": digest(fixture)})
            data["artifacts"]["observation_receipt"] = {"path": str(observation), "sha256": digest(observation)}  # type: ignore[index]
            evidence.write_text(json.dumps(data), encoding="utf-8")
            report, missing, mismatches = inventory._load_host_evidence(evidence, "browser", RUN_ID, SESSION, (root,))
        self.assertEqual(report["status"], "inconclusive")
        self.assertIn("visible_fixture.image", missing)
        self.assertEqual(mismatches, [])

    def test_missing_identity_cannot_pass(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            evidence = root / "browser.json"
            data = base_evidence(root)
            del data["renderer"]
            evidence.write_text(json.dumps(data), encoding="utf-8")
            report, missing, mismatches = inventory._load_host_evidence(evidence, "browser", RUN_ID, SESSION, (root,))
        self.assertEqual(report["status"], "inconclusive")
        self.assertIn("identity.renderer", missing)
        self.assertEqual(mismatches, [])

    def test_changed_capture_hash_rejects_observation_binding(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            evidence = root / "browser.json"
            data = base_evidence(root)
            data["artifacts"]["visible_fixture"]["sha256"] = "0" * 64  # type: ignore[index]
            evidence.write_text(json.dumps(data), encoding="utf-8")
            report, missing, mismatches = inventory._load_host_evidence(evidence, "browser", RUN_ID, SESSION, (root,))
        self.assertEqual(report["status"], "fail")
        self.assertIn("visible_fixture.hash", missing)
        self.assertIn("observation_receipt.artifact_sha256", mismatches)

    def test_complete_browser_receipt_passes(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            evidence = root / "browser.json"
            evidence.write_text(json.dumps(base_evidence(root)), encoding="utf-8")
            report, missing, mismatches = inventory._load_host_evidence(evidence, "browser", RUN_ID, SESSION, (root,))
        self.assertEqual(report["status"], "pass")
        self.assertEqual(missing, [])
        self.assertEqual(mismatches, [])

    def test_complete_native_receipt_requires_real_executable_identity(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            evidence = root / "native.json"
            evidence.write_text(json.dumps(base_evidence(root, "native")), encoding="utf-8")
            report, missing, mismatches = inventory._load_host_evidence(evidence, "native", RUN_ID, SESSION, (root,))
        self.assertEqual(report["status"], "pass")
        self.assertEqual(missing, [])
        self.assertEqual(mismatches, [])

    def test_report_replacement_does_not_modify_a_linked_file(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            protected = root / "protected-config"
            protected.write_text("preserve", encoding="utf-8")
            output = root / "report.json"
            output.hardlink_to(protected)
            inventory._write_output(output, '{"status":"PASS"}')
            self.assertEqual(protected.read_text(encoding="utf-8"), "preserve")
            self.assertEqual(output.read_text(encoding="utf-8"), '{"status":"PASS"}')

    def test_empty_owned_roots_cannot_authorize_evidence_parent(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            evidence = write_receipt(root, "browser.json", base_evidence(root))
            report, _, _ = inventory._load_host_evidence(evidence, "browser", RUN_ID, SESSION, ())
            self.assertEqual(report["status"], "inconclusive")

    def test_oversize_png_dimensions_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            image = Path(temporary) / "wide.png"
            image.write_bytes(png_bytes(width=8193))
            self.assertIsNotNone(inventory._decode_image(image))

    def test_deep_invalid_evidence_is_inconclusive(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            evidence = root / "deep.json"
            evidence.write_bytes(b"[" * 2000 + b"0" + b"]" * 2000)
            report, _, _ = inventory._load_host_evidence(evidence, "browser", RUN_ID, SESSION, (root,))
            self.assertEqual(report["status"], "inconclusive")


if __name__ == "__main__":
    unittest.main()
