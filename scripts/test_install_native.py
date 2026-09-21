#!/usr/bin/env python3
"""Hermetic behavior tests for scripts/install-native.py."""

from __future__ import annotations

import ctypes
import ctypes.util
import importlib.util
import json
import os
import plistlib
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock


MODULE_PATH = Path(__file__).with_name("install-native.py")
SPEC = importlib.util.spec_from_file_location("install_native", MODULE_PATH)
assert SPEC and SPEC.loader
installer = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = installer
SPEC.loader.exec_module(installer)


class NativeInstallTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        base_paths = installer.install_paths(self.root / "prefix")
        self.paths = installer.InstallPaths(base_paths.data_home, base_paths.bin_home, application=None)
        self.binary = self.root / "candidate"
        self.binary.write_bytes(b"first native binary")
        self.binary.chmod(0o755)
        self.cli_binary = self.root / "candidate-cli"
        self.cli_binary.write_bytes(b"first Cockpit CLI")
        self.cli_binary.chmod(0o755)
        self.icon = self.root / "icon.png"
        self.icon.write_bytes(b"png fixture")

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def mac_paths(self) -> installer.InstallPaths:
        return installer.InstallPaths(
            data_home=self.paths.data_home,
            bin_home=self.paths.bin_home,
            application=self.root / "Applications" / installer.BUNDLE_NAME,
        )

    def write_bundle(self, path: Path, executable: bytes, resource: bytes = b"bundle resource") -> Path:
        executable_path = path / "Contents" / "MacOS" / installer.BINARY_NAME
        executable_path.parent.mkdir(parents=True, exist_ok=True)
        executable_path.write_bytes(executable)
        executable_path.chmod(0o755)
        resource_path = path / "Contents" / "Resources" / "payload"
        resource_path.parent.mkdir(parents=True, exist_ok=True)
        resource_path.write_bytes(resource)
        (resource_path.parent / "alias").symlink_to("payload")
        with (path / "Contents" / "Info.plist").open("wb") as stream:
            plistlib.dump(
                {
                    "CFBundleExecutable": installer.BINARY_NAME,
                    "CFBundleIdentifier": installer.APPLICATION_ID,
                },
                stream,
            )
        return path

    def test_prefix_install_writes_stable_launcher_desktop_entry_and_receipt(self) -> None:
        installer.install(self.paths, self.binary, self.cli_binary, self.icon)

        self.assertEqual(self.paths.binary.read_bytes(), b"first native binary")
        self.assertTrue(os.access(self.paths.binary, os.X_OK))
        self.assertTrue(self.paths.launcher.is_symlink())
        self.assertEqual(self.paths.launcher.resolve(), self.paths.binary)
        self.assertEqual(self.paths.cli_binary.read_bytes(), b"first Cockpit CLI")
        self.assertTrue(self.paths.cli_launcher.is_symlink())
        self.assertEqual(self.paths.cli_launcher.resolve(), self.paths.cli_binary)
        desktop = self.paths.desktop.read_text(encoding="utf-8")
        self.assertIn(f'Exec="{self.paths.launcher}"', desktop)
        self.assertIn(installer.DESKTOP_MARKER, desktop)
        self.assertEqual(self.paths.icon.read_bytes(), b"png fixture")
        receipt = json.loads(self.paths.receipt.read_text(encoding="utf-8"))
        self.assertEqual(receipt["schema"], 3)
        self.assertEqual(receipt["status"], "installed")
        self.assertEqual(receipt["artifact_sha256"]["cli_binary"], installer.sha256(self.paths.cli_binary))

    def test_macos_install_copies_complete_bundle_and_records_manifest(self) -> None:
        paths = self.mac_paths()
        source = self.write_bundle(self.root / "build" / installer.BUNDLE_NAME, self.binary.read_bytes())

        installer.install(paths, self.binary, self.cli_binary, self.icon, source)

        destination_payload = paths.application / "Contents" / "Resources" / "payload"
        self.assertEqual(destination_payload.read_bytes(), b"bundle resource")
        self.assertTrue((paths.application / "Contents" / "Resources" / "alias").is_symlink())
        self.assertEqual(paths.launcher.resolve(), paths.application / "Contents" / "MacOS" / installer.BINARY_NAME)
        self.assertFalse(paths.binary.exists())
        receipt = json.loads(paths.receipt.read_text(encoding="utf-8"))
        self.assertEqual(receipt["schema"], 4)
        self.assertEqual(receipt["paths"]["application"], str(paths.application))
        self.assertNotIn("binary", receipt["paths"])
        self.assertEqual(receipt["artifact_manifests"]["application"], installer.bundle_manifest(paths.application))
        self.assertEqual(receipt["artifact_sha256"]["application"], installer.sha256_bundle(paths.application))

    def test_macos_update_refuses_modified_bundle_and_preserves_it(self) -> None:
        paths = self.mac_paths()
        source = self.write_bundle(self.root / "build" / installer.BUNDLE_NAME, self.binary.read_bytes())
        installer.install(paths, self.binary, self.cli_binary, self.icon, source)
        payload = paths.application / "Contents" / "Resources" / "payload"
        payload.write_bytes(b"local modification")

        with self.assertRaisesRegex(installer.InstallError, "modified installed files: application"):
            installer.install(paths, self.binary, self.cli_binary, self.icon, source)
        self.assertEqual(payload.read_bytes(), b"local modification")

    def test_macos_foreign_bundle_is_not_adopted(self) -> None:
        paths = self.mac_paths()
        foreign = self.write_bundle(paths.application, self.binary.read_bytes())
        with (foreign / "Contents" / "Info.plist").open("wb") as stream:
            plistlib.dump({"CFBundleIdentifier": installer.APPLICATION_ID, "CFBundleExecutable": installer.BINARY_NAME}, stream)
        with self.assertRaisesRegex(installer.InstallError, "unrecognized macOS application"):
            installer.install(paths, self.binary, self.cli_binary, self.icon, self.write_bundle(self.root / "build" / installer.BUNDLE_NAME, self.binary.read_bytes()))
        self.assertTrue(paths.application.exists())

    def test_macos_rejects_wrong_bundle_identifier(self) -> None:
        source = self.write_bundle(self.root / "build" / installer.BUNDLE_NAME, self.binary.read_bytes())
        with (source / "Contents" / "Info.plist").open("wb") as stream:
            plistlib.dump({"CFBundleIdentifier": "foreign.app", "CFBundleExecutable": installer.BINARY_NAME}, stream)
        with self.assertRaisesRegex(installer.InstallError, "wrong bundle identifier"):
            installer.install(self.mac_paths(), self.binary, self.cli_binary, self.icon, source)

    def test_macos_rejects_escaping_bundle_symlink(self) -> None:
        source = self.write_bundle(self.root / "build" / installer.BUNDLE_NAME, self.binary.read_bytes())
        link = source / "Contents" / "Resources" / "alias"
        link.unlink()
        link.symlink_to("../../../../outside")
        with self.assertRaisesRegex(installer.InstallError, "escapes its bundle"):
            installer.install(self.mac_paths(), self.binary, self.cli_binary, self.icon, source)

    def test_legacy_install_receipt_migrates_without_claiming_bundle(self) -> None:
        self.paths.binary.parent.mkdir(parents=True)
        self.paths.binary.write_bytes(b"legacy native binary")
        self.paths.binary.chmod(0o755)
        self.paths.icon.parent.mkdir(parents=True)
        self.paths.icon.write_bytes(b"legacy icon")
        self.paths.desktop.parent.mkdir(parents=True)
        self.paths.desktop.write_text("legacy desktop\n", encoding="utf-8")
        self.paths.launcher.parent.mkdir(parents=True)
        self.paths.launcher.symlink_to(self.paths.binary)
        legacy = {
            "schema": 1,
            "application_id": installer.APPLICATION_ID,
            "status": "installed",
            "paths": installer.legacy_receipt_paths(self.paths),
            "artifact_sha256": {
                "binary": installer.sha256(self.paths.binary),
                "icon": installer.sha256(self.paths.icon),
                "desktop": installer.sha256(self.paths.desktop),
            },
        }
        self.paths.receipt.write_text(json.dumps(legacy), encoding="utf-8")

        installer.install(self.paths, self.binary, self.cli_binary, self.icon)

        receipt = json.loads(self.paths.receipt.read_text(encoding="utf-8"))
        self.assertEqual(receipt["schema"], 3)
        self.assertTrue(self.paths.cli_launcher.is_symlink())

    def test_update_replaces_binary_without_changing_launcher_path(self) -> None:
        installer.install(self.paths, self.binary, self.cli_binary, self.icon)
        original_inode = self.paths.binary.stat().st_ino
        with self.paths.binary.open("rb") as running_binary:
            self.binary.write_bytes(b"second native binary")
            installer.install(self.paths, self.binary, self.cli_binary, self.icon)
            self.assertEqual(running_binary.read(), b"first native binary")

        self.assertEqual(self.paths.binary.read_bytes(), b"second native binary")
        self.assertNotEqual(self.paths.binary.stat().st_ino, original_inode)
        self.assertEqual(self.paths.launcher.resolve(), self.paths.binary)

    def test_desktop_launcher_is_discoverable_by_glib(self) -> None:
        library = ctypes.util.find_library("gio-2.0")
        if not library:
            self.skipTest("GLib desktop discovery is not installed")
        self.paths = installer.install_paths(self.root / "prefix with spaces")
        installer.install(self.paths, self.binary, self.cli_binary, self.icon)
        gio = ctypes.CDLL(library)
        if not hasattr(gio, "g_desktop_app_info_new_from_filename"):
            self.skipTest("GLib desktop discovery symbols are not installed")
        gio.g_desktop_app_info_new_from_filename.argtypes = [ctypes.c_char_p]
        gio.g_desktop_app_info_new_from_filename.restype = ctypes.c_void_p
        info = gio.g_desktop_app_info_new_from_filename(os.fsencode(self.paths.desktop))
        self.assertTrue(info, "desktop app must be discoverable by a real launcher")
        gio.g_object_unref.argtypes = [ctypes.c_void_p]
        gio.g_object_unref(info)

    def test_install_refuses_to_replace_a_foreign_launcher(self) -> None:
        self.paths.launcher.parent.mkdir(parents=True)
        self.paths.launcher.write_text("foreign launcher\n", encoding="utf-8")

        with self.assertRaisesRegex(installer.InstallError, "without an installer receipt"):
            installer.install(self.paths, self.binary, self.cli_binary, self.icon)

        self.assertEqual(self.paths.launcher.read_text(encoding="utf-8"), "foreign launcher\n")

    def test_publication_failure_restores_prior_generation(self) -> None:
        installer.install(self.paths, self.binary, self.cli_binary, self.icon)
        old = self.paths.binary.read_bytes()
        self.binary.write_bytes(b"second native binary")
        original_replace = installer.os.replace
        failed = False

        def fail_once(source: Path, destination: Path) -> None:
            nonlocal failed
            if destination == self.paths.binary and not failed:
                failed = True
                raise OSError("injected publication failure")
            original_replace(source, destination)

        with mock.patch.object(installer.os, "replace", side_effect=fail_once):
            with self.assertRaisesRegex(installer.InstallError, "publication failed"):
                installer.install(self.paths, self.binary, self.cli_binary, self.icon)
        self.assertEqual(self.paths.binary.read_bytes(), old)
        self.assertTrue(self.paths.pending.exists())
        installer.ensure_installable(self.paths)
        self.assertFalse(self.paths.pending.exists())

    def test_uninstall_refuses_modified_artifacts_and_preserves_configuration(self) -> None:
        installer.install(self.paths, self.binary, self.cli_binary, self.icon)
        configuration = self.paths.app_root / "config" / "settings.json"
        configuration.parent.mkdir()
        configuration.write_text('{"keep": true}\n', encoding="utf-8")
        self.paths.binary.write_bytes(b"replacement binary")
        self.paths.binary.chmod(0o755)

        self.assertFalse(installer.uninstall(self.paths))
        self.assertTrue(self.paths.receipt.exists())
        self.assertEqual(configuration.read_text(encoding="utf-8"), '{"keep": true}\n')

    def test_xdg_paths_respect_disposable_environment(self) -> None:
        paths = installer.install_paths(None, {"HOME": str(self.root / "home"), "XDG_DATA_HOME": str(self.root / "data"), "XDG_BIN_HOME": str(self.root / "bin")})

        self.assertEqual(paths.data_home, (self.root / "data").resolve())
        self.assertEqual(paths.bin_home, (self.root / "bin").resolve())

    def test_macos_defaults_and_prefix_application_are_disposable(self) -> None:
        with mock.patch.object(installer.sys, "platform", "darwin"):
            paths = installer.install_paths(None, {"HOME": str(self.root / "home")})
            prefixed = installer.install_paths(self.root / "prefix", {"HOME": str(self.root / "home")})

        self.assertEqual(paths.data_home, (self.root / "home" / "Library" / "Application Support").resolve())
        self.assertEqual(paths.application, self.root / "home" / "Applications" / installer.BUNDLE_NAME)
        self.assertEqual(prefixed.application, self.root / "prefix" / "Applications" / installer.BUNDLE_NAME)

    def test_macos_build_keeps_tauri_bundling_enabled(self) -> None:
        with mock.patch.object(installer.sys, "platform", "darwin"), mock.patch.object(installer.subprocess, "run") as run:
            installer.run_build(self.root, False)

        self.assertEqual(run.call_args_list[0], mock.call(["bunx", "tauri", "build", "--bundles", "app"], cwd=self.root, check=True))


if __name__ == "__main__":
    unittest.main()
