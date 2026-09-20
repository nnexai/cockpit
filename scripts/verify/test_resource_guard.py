#!/usr/bin/env python3
"""Hermetic behavioral tests for the fail-closed verification resource guard."""

from __future__ import annotations

import contextlib
import hashlib
import io
import json
import os
import stat
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

try:
    from .resource_guard import (
        ResourceGuardError,
        load_ledger,
        main,
        prepare_subprocess,
    )
except ImportError:  # Direct ``python scripts/verify/test_resource_guard.py``.
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    from resource_guard import (  # type: ignore[no-redef]
        ResourceGuardError,
        load_ledger,
        main,
        prepare_subprocess,
    )


RUN_ID = "run-test-20260904"
SESSION = "run-test-20260904-fixture"


class ResourceGuardTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tempdir = tempfile.TemporaryDirectory()
        root = Path(self.tempdir.name)
        self.resource_root = root / "resources"
        self.resource_root.mkdir()
        self.xdg_config_home = self.resource_root / "config"
        self.xdg_state_home = self.resource_root / "state"
        self.herdr_config_dir = self.xdg_config_home / "herdr"
        self.session_dir = self.herdr_config_dir / "sessions" / SESSION
        self.session_dir.mkdir(parents=True)
        self.xdg_state_home.mkdir()
        self.config = self.herdr_config_dir / "config.toml"
        self.socket = self.session_dir / "herdr.sock"
        self.protected_config = root / "protected-config.toml"
        self.protected_config.write_text("protected", encoding="utf-8")
        self.protected_socket = root / "protected.sock"
        self.protected_socket.write_text("protected socket", encoding="utf-8")
        self.executable = root / "herdr"
        self.executable.write_bytes(b"#!/bin/sh\nprintf stable\n")
        self.executable.chmod(self.executable.stat().st_mode | stat.S_IXUSR)
        self.ledger_path = root / "resources.json"
        self._write_ledger()

    def tearDown(self) -> None:
        self.tempdir.cleanup()

    def test_dispatch_rejects_shared_writable_root_after_ledger_load(self) -> None:
        ledger = load_ledger(self.ledger_path)
        self.resource_root.chmod(0o777)
        with self.assertRaises(ResourceGuardError):
            prepare_subprocess(ledger, RUN_ID, SESSION, ("status", "server", "--json"))

    def test_dispatch_rejects_root_owned_by_another_user(self) -> None:
        ledger = load_ledger(self.ledger_path)
        with mock.patch("os.geteuid", return_value=os.geteuid() + 1):
            with self.assertRaises(ResourceGuardError):
                prepare_subprocess(ledger, RUN_ID, SESSION, ("status", "server", "--json"))

    def test_server_and_tui_cannot_inherit_a_foreign_home(self) -> None:
        for command, status in ((("server",), "planned"), (("tui",), "running")):
            self._write_ledger(status=status)
            for inherited in ({}, {"HOME": str(self.protected_config.parent)}):
                with self.subTest(command=command, inherited=inherited):
                    _, _, environment = self._plan(command, base_environment=inherited)
                    self.assertEqual(environment["HOME"], str(self.resource_root / "home"))

    def test_process_home_cannot_escape_through_a_symlink(self) -> None:
        (self.resource_root / "home").symlink_to(self.protected_config.parent)
        for command, status in ((("server",), "planned"), (("tui",), "running")):
            self._write_ledger(status=status)
            with self.subTest(command=command):
                with self.assertRaises(ResourceGuardError):
                    self._plan(command)

    def test_running_tui_requires_exact_target_and_owned_home(self) -> None:
        self._write_ledger(status="running")
        home = self.resource_root / "home"
        home.mkdir()
        _target, argv, environment = prepare_subprocess(
            load_ledger(self.ledger_path), RUN_ID, SESSION, ("tui",),
            base_environment={"HOME": str(home), "HERDR_SESSION": "default"},
        )
        self.assertEqual(argv, [str(self.executable), "--session", SESSION])
        self.assertEqual(environment["HOME"], str(home))
        self.assertNotIn("HERDR_SESSION", environment)

    def _write_ledger(
        self,
        *,
        name: str = SESSION,
        owner: str = RUN_ID,
        status: str = "ready",
        config: Path | str | None = None,
        socket: Path | str | None = None,
        state_home: Path | str | None = None,
        protected_config: Path | str | None = None,
        protected_socket: Path | str | None = None,
        sessions: list[dict[str, object]] | None = None,
        resources: list[dict[str, object]] | None = None,
        forbidden_paths: list[str] | None = None,
    ) -> None:
        if sessions is None:
            sessions = [
                {
                    "name": name,
                    "owner": owner,
                    "xdg_config_home": str(self.xdg_config_home),
                    "xdg_state_home": str(
                        self.xdg_state_home if state_home is None else state_home
                    ),
                    "config_path": str(self.config if config is None else config),
                    "socket_path": str(self.socket if socket is None else socket),
                    "status": status,
                }
            ]
        digest = hashlib.sha256(self.executable.read_bytes()).hexdigest()
        self.ledger_path.write_text(
            json.dumps(
                {
                    "run_id": RUN_ID,
                    "protected_session": "default",
                    "resource_root": str(self.resource_root),
                    "resources": resources
                    if resources is not None
                    else [{"path": str(self.resource_root), "owner": RUN_ID}],
                    "protected_config_path": str(
                        self.protected_config
                        if protected_config is None
                        else protected_config
                    ),
                    "protected_socket_path": str(
                        self.protected_socket
                        if protected_socket is None
                        else protected_socket
                    ),
                    "allowed_sessions": sessions,
                    "executables": {
                        "selected": {
                            "path": str(self.executable),
                            "sha256": digest,
                            "version": "0.8.2",
                            "protocol": 20,
                            "schema_version": 1,
                        }
                    },
                    "forbidden_executable_paths": forbidden_paths or [],
                }
            ),
            encoding="utf-8",
        )

    def _plan(
        self,
        command: tuple[str, ...],
        *,
        cleanup: bool = False,
        base_environment: dict[str, str] | None = None,
    ):
        return prepare_subprocess(
            load_ledger(self.ledger_path),
            RUN_ID,
            SESSION,
            command,
            cleanup=cleanup,
            base_environment=base_environment,
        )

    def test_inventory_commands_return_explicit_target_and_isolated_state(self) -> None:
        inherited = {
            "PATH": "/usr/bin",
            "HOME": "/home/ambient",
            "XDG_CONFIG_HOME": "/ambient/config",
            "XDG_STATE_HOME": "/ambient/state",
            "HERDR_SESSION": "ambient-session",
            "HERDR_SOCKET_PATH": "/ambient/socket",
            "HERDR_CONFIG_PATH": "/ambient/config.toml",
        }
        for command in (
            ("status", "server", "--json"),
            ("session", "list", "--json"),
            ("api", "schema", "--json"),
            ("api", "snapshot"),
        ):
            with self.subTest(command=command):
                target, argv, environment = self._plan(command, base_environment=inherited)
                self.assertEqual(target.session.name, SESSION)
                self.assertEqual(
                    argv,
                    [str(self.executable), "--session", SESSION, *command],
                )
                self.assertEqual(environment["XDG_CONFIG_HOME"], str(self.xdg_config_home))
                self.assertEqual(environment["XDG_STATE_HOME"], str(self.xdg_state_home))
                self.assertEqual(environment["HERDR_CONFIG_PATH"], str(self.config))
                self.assertEqual(environment["HERDR_SOCKET_PATH"], str(self.socket))
                self.assertEqual(environment["HOME"], str(self.resource_root / "home"))
                self.assertNotIn("HERDR_SESSION", environment)

    def test_prepare_rejects_every_forbidden_or_generic_operation_before_a_plan_exists(self) -> None:
        plans: list[object] = []
        for command in (
            ("update",),
            ("update", "--handoff"),
            ("--remote", "host"),
            ("status", "server", "--remote=host"),
            ("status", "server", "--no-session"),
            ("config", "reset-keys"),
            ("channel", "set", "preview"),
            ("session", "delete", SESSION),
            ("session", "attach", SESSION),
            ("server", "stop"),
            ("status", "--session", "default"),
            ("api", "snapshot", "--"),
            ("pane", "run", "echo", "unsafe"),
            ("agent", "start", "anything"),
        ):
            with self.subTest(command=command), self.assertRaises(ResourceGuardError):
                plans.append(self._plan(command))
        self.assertEqual(plans, [])

    def test_cleanup_is_only_exact_owned_session_stop(self) -> None:
        _, argv, _ = self._plan(("session", "stop", SESSION, "--json"), cleanup=True)
        self.assertEqual(argv[-4:], ["session", "stop", SESSION, "--json"])
        for command, cleanup in (
            (("session", "stop", SESSION), False),
            (("session", "stop", "default"), True),
            (("session", "delete", SESSION), True),
            (("status", "server", "--json"), True),
        ):
            with self.subTest(command=command, cleanup=cleanup), self.assertRaises(ResourceGuardError):
                self._plan(command, cleanup=cleanup)

    def test_terminal_and_unknown_statuses_reject_all_dispatches(self) -> None:
        for status in ("cleaned", "deleted", "removed", "released", "foreign", ""):
            with self.subTest(status=status):
                self._write_ledger(status=status)
                with self.assertRaises(ResourceGuardError):
                    self._plan(("status", "server", "--json"))
                with self.assertRaises(ResourceGuardError):
                    self._plan(("session", "stop", SESSION), cleanup=True)

    def test_planned_session_can_start_server_but_ready_session_cannot(self) -> None:
        self._write_ledger(status="planned")
        _, argv, _ = self._plan(("server",))
        self.assertEqual(argv[-1:], ["server"])
        self._write_ledger(status="ready")
        with self.assertRaises(ResourceGuardError):
            self._plan(("server",))

    def test_runtime_fixture_commands_stay_beneath_the_recorded_root(self) -> None:
        fixture = self.resource_root / "fixture"
        self._write_ledger(status="running")
        for command in (
            ("workspace", "create", "--cwd", str(fixture), "--label", "fixture"),
            ("tab", "create", "--cwd", str(fixture), "--no-focus"),
            ("worktree", "create", "--cwd", str(fixture), "--path", str(fixture / "worktree"), "--branch", "fixture"),
            ("worktree", "remove", "--workspace", "ws-1"),
            ("pane", "close", "pane-1"),
            ("agent", "read", "agent-1", "--source", "screen"),
        ):
            with self.subTest(command=command):
                self._plan(command)
        for command in (
            ("workspace", "create", "--cwd", self.tempdir.name),
            ("tab", "create", "--env", "LD_PRELOAD=unsafe", "--cwd", str(fixture)),
            ("worktree", "open", "--cwd", str(fixture), "--path", self.tempdir.name),
            ("worktree", "remove", "--workspace", "--session"),
        ):
            with self.subTest(command=command), self.assertRaises(ResourceGuardError):
                self._plan(command)

    def test_normal_dispatch_rejects_protected_config_hardlink_after_ledger_load(self) -> None:
        ledger = load_ledger(self.ledger_path)
        os.link(self.protected_config, self.config)
        with self.assertRaises(ResourceGuardError):
            prepare_subprocess(ledger, RUN_ID, SESSION, ("api", "snapshot"))

    def test_normal_dispatch_rejects_protected_socket_hardlink_after_ledger_load(self) -> None:
        ledger = load_ledger(self.ledger_path)
        os.link(self.protected_socket, self.socket)
        with self.assertRaises(ResourceGuardError):
            prepare_subprocess(ledger, RUN_ID, SESSION, ("api", "snapshot"))

    def test_prepare_rechecks_symlinked_resource_root_after_ledger_load(self) -> None:
        ledger = load_ledger(self.ledger_path)
        moved_root = self.resource_root.with_name("moved-resources")
        self.resource_root.rename(moved_root)
        self.resource_root.symlink_to(moved_root, target_is_directory=True)
        with self.assertRaises(ResourceGuardError):
            prepare_subprocess(ledger, RUN_ID, SESSION, ("api", "snapshot"))

    def test_prepare_rechecks_symlinked_state_leaf_after_ledger_load(self) -> None:
        ledger = load_ledger(self.ledger_path)
        self.xdg_state_home.rmdir()
        self.xdg_state_home.symlink_to(Path(self.tempdir.name) / "outside")
        with self.assertRaises(ResourceGuardError):
            prepare_subprocess(ledger, RUN_ID, SESSION, ("api", "snapshot"))

    def test_executable_hardlink_to_forbidden_identity_is_rejected(self) -> None:
        forbidden = Path(self.tempdir.name) / "archived-herdr"
        os.link(self.executable, forbidden)
        self._write_ledger(forbidden_paths=[str(forbidden)])
        with self.assertRaises(ResourceGuardError):
            self._plan(("api", "schema", "--json"))

    def test_missing_runtime_root_receipt_and_state_home_are_rejected(self) -> None:
        self._write_ledger(resources=[])
        with self.assertRaises(ResourceGuardError):
            load_ledger(self.ledger_path)
        self._write_ledger()
        raw = json.loads(self.ledger_path.read_text(encoding="utf-8"))
        del raw["allowed_sessions"][0]["xdg_state_home"]
        self.ledger_path.write_text(json.dumps(raw), encoding="utf-8")
        with self.assertRaises(ResourceGuardError):
            load_ledger(self.ledger_path)

    def test_hash_drift_rejects_before_any_plan_is_returned(self) -> None:
        ledger = load_ledger(self.ledger_path)
        self.executable.write_bytes(b"#!/bin/sh\nprintf changed\n")
        with self.assertRaises(ResourceGuardError):
            prepare_subprocess(ledger, RUN_ID, SESSION, ("api", "snapshot"))

    def test_duplicate_and_malformed_sessions_are_rejected(self) -> None:
        duplicate = [
            {
                "name": "same",
                "owner": RUN_ID,
                "xdg_config_home": str(self.xdg_config_home),
                "xdg_state_home": str(self.xdg_state_home),
                "config_path": str(self.config),
                "socket_path": str(self.socket),
                "status": "ready",
            },
            {
                "name": "same",
                "owner": RUN_ID,
                "xdg_config_home": str(self.resource_root / "second-config"),
                "xdg_state_home": str(self.resource_root / "second-state"),
                "config_path": str(self.resource_root / "second-config/herdr/config.toml"),
                "socket_path": str(self.resource_root / "second-config/herdr/sessions/same/herdr.sock"),
                "status": "ready",
            },
        ]
        self._write_ledger(sessions=duplicate)
        with self.assertRaises(ResourceGuardError):
            load_ledger(self.ledger_path)
        for name in ("default", "has whitespace", "../escape", "", "./dot"):
            with self.subTest(name=name):
                self._write_ledger(name=name)
                with self.assertRaises(ResourceGuardError):
                    load_ledger(self.ledger_path)

    def test_cli_report_uses_recorded_state_home_not_an_inherited_selector(self) -> None:
        stdout = io.StringIO()
        stderr = io.StringIO()
        with mock.patch.dict(
            os.environ,
            {"HERDR_SESSION": "ambient", "HERDR_SOCKET_PATH": "/ambient"},
            clear=False,
        ), contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            result = main(
                [
                    "--ledger",
                    str(self.ledger_path),
                    "--run-id",
                    RUN_ID,
                    "--session",
                    SESSION,
                ]
            )
        self.assertEqual(result, 0)
        self.assertEqual(stderr.getvalue(), "")
        report = json.loads(stdout.getvalue())
        self.assertEqual(report["effective"]["xdg_state_home"], str(self.xdg_state_home))
        self.assertNotIn("ambient", stdout.getvalue())


if __name__ == "__main__":
    unittest.main()
