#!/usr/bin/env python3
"""Hermetic tests for scripts/install-native.py."""

from __future__ import annotations

import importlib.util
import ctypes
import ctypes.util
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path


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
        self.paths = installer.install_paths(self.root / "prefix")
        self.binary = self.root / "candidate"
        self.binary.write_bytes(b"first native binary")
        self.binary.chmod(0o755)
        self.icon = self.root / "icon.png"
        self.icon.write_bytes(b"png fixture")

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def test_prefix_install_writes_stable_launcher_desktop_entry_and_receipt(self) -> None:
        installer.install(self.paths, self.binary, self.icon)

        self.assertEqual(self.paths.binary.read_bytes(), b"first native binary")
        self.assertTrue(os.access(self.paths.binary, os.X_OK))
        self.assertTrue(self.paths.launcher.is_symlink())
        self.assertEqual(self.paths.launcher.resolve(), self.paths.binary)
        desktop = self.paths.desktop.read_text(encoding="utf-8")
        self.assertIn(f'Exec="{self.paths.launcher}"', desktop)
        self.assertIn(installer.DESKTOP_MARKER, desktop)
        self.assertEqual(self.paths.icon.read_bytes(), b"png fixture")
        receipt = json.loads(self.paths.receipt.read_text(encoding="utf-8"))
        self.assertEqual(receipt["status"], "installed")
        self.assertEqual(receipt["paths"]["binary"], str(self.paths.binary))
        self.assertEqual(receipt["artifact_sha256"]["binary"], installer.sha256(self.paths.binary))

    def test_update_replaces_the_stable_binary_without_changing_launcher_path(self) -> None:
        installer.install(self.paths, self.binary, self.icon)
        original_inode = self.paths.binary.stat().st_ino
        with self.paths.binary.open("rb") as running_binary:
            self.binary.write_bytes(b"second native binary")
            installer.install(self.paths, self.binary, self.icon)
            self.assertEqual(running_binary.read(), b"first native binary")

        self.assertEqual(self.paths.binary.read_bytes(), b"second native binary")
        self.assertNotEqual(self.paths.binary.stat().st_ino, original_inode)
        self.assertEqual(self.paths.launcher.resolve(), self.paths.binary)

    def test_desktop_launcher_is_discoverable_by_glib(self) -> None:
        library = ctypes.util.find_library("gio-2.0")
        if not library:
            self.skipTest("GLib desktop discovery is not installed")
        # TryExec is a string, not a shell command: quoting it hides the app.
        self.paths = installer.install_paths(self.root / "prefix with spaces")
        installer.install(self.paths, self.binary, self.icon)
        gio = ctypes.CDLL(library)
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
            installer.install(self.paths, self.binary, self.icon)

        self.assertEqual(self.paths.launcher.read_text(encoding="utf-8"), "foreign launcher\n")

    def test_uninstall_removes_owned_files_and_preserves_configuration(self) -> None:
        installer.install(self.paths, self.binary, self.icon)
        configuration = self.paths.app_root / "config" / "settings.json"
        configuration.parent.mkdir()
        configuration.write_text('{"keep": true}\n', encoding="utf-8")

        self.assertTrue(installer.uninstall(self.paths))

        self.assertFalse(self.paths.binary.exists())
        self.assertFalse(self.paths.launcher.exists())
        self.assertFalse(self.paths.desktop.exists())
        self.assertFalse(self.paths.icon.exists())
        self.assertEqual(configuration.read_text(encoding="utf-8"), '{"keep": true}\n')

    def test_uninstall_keeps_an_unrecognized_launcher_and_its_receipt(self) -> None:
        installer.install(self.paths, self.binary, self.icon)
        self.paths.launcher.unlink()
        self.paths.launcher.write_text("replacement launcher\n", encoding="utf-8")

        self.assertFalse(installer.uninstall(self.paths))

        self.assertTrue(self.paths.receipt.exists())
        self.assertEqual(self.paths.launcher.read_text(encoding="utf-8"), "replacement launcher\n")

    def test_update_refuses_to_replace_a_modified_installed_binary(self) -> None:
        installer.install(self.paths, self.binary, self.icon)
        self.paths.binary.write_bytes(b"replacement binary")
        self.paths.binary.chmod(0o755)

        with self.assertRaisesRegex(installer.InstallError, "modified installed files: binary"):
            installer.install(self.paths, self.binary, self.icon)

    def test_xdg_paths_respect_disposable_environment(self) -> None:
        paths = installer.install_paths(None, {"HOME": str(self.root / "home"), "XDG_DATA_HOME": str(self.root / "data"), "XDG_BIN_HOME": str(self.root / "bin")})

        self.assertEqual(paths.data_home, (self.root / "data").resolve())
        self.assertEqual(paths.bin_home, (self.root / "bin").resolve())


if __name__ == "__main__":
    unittest.main()
