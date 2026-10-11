"""Creation/normal-stop regressions; never operate on existing fixtures."""
import copy
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

try:
    from . import fixture_resources as resources
except ImportError:
    import fixture_resources as resources


class FixtureResourcesTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="cpol-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        for name in resources.SCRATCH:
            (self.root / name).mkdir()
        (self.root / "runtime.json").write_text(json.dumps({
            "root": str(self.root), "session": "polish-test", "socket": str(self.root / "herdr.sock")}))
        with patch.dict(os.environ, {"COCKPIT_RUN_ID": "worker", "COCKPIT_ROOT_ID": "root"}):
            self.manifest = resources.create(self.root, "polish-test")
        self.identity = {"pid": 123456, "boot_id": resources.boot_id(), "start_ticks": 17,
                         "session_id": 123456, "process_group": 123456,
                         "exe_device": 1, "exe_inode": 2}
        self.key = "process:123456:17"
        self.manifest["processes"] = [{"name": "gateway", "identity": self.identity}]
        resources.atomic_json(self.root / resources.MANIFEST, self.manifest)
        self.journal = {"effects": {}, "members": {self.key: self.identity}}

    def test_normal_driver_import_and_environment_preserve_creation_membership(self):
        spec = importlib.util.spec_from_file_location(
            "isolated_polish_driver", Path(resources.__file__).with_name("ui_polish_runtime.py"))
        driver = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(driver)
        with patch.dict(os.environ, {"COCKPIT_RUN_ID": "ambient-worker"}):
            child_environment = driver.environment(self.root)
        self.assertEqual(child_environment["COCKPIT_FIXTURE_CREATION_ID"], self.manifest["creation_id"])
        self.assertEqual(child_environment["COCKPIT_HERDR_SESSION"], "polish-test")
        self.assertNotIn("COCKPIT_RUN_ID", child_environment)
        self.assertEqual(self.manifest["owner_run_id"], "worker")

    def test_registration_binds_owner_and_exact_creation(self):
        with patch.object(resources, "inspect_process", return_value=("live", "pinned")):
            actual = resources.registration(self.root)
            fixture = actual["resources"][0]
            result = resources.helper_operation({"resource": fixture, "run_id": "worker", "root_id": "root", "action": "register"})
            self.assertEqual(result["state"], "ready")
            with self.assertRaises(ValueError):
                resources.helper_operation({"resource": fixture, "run_id": "other", "root_id": "root", "action": "register"})
        changed = copy.deepcopy(fixture)
        changed["identity"]["filesystem"]["inode"] = "wrong"
        with self.assertRaises(ValueError):
            resources.helper_operation({"resource": changed, "run_id": "worker", "root_id": "root", "action": "apply"})

    def test_pid_reuse_boot_change_and_foreign_membership_never_signal(self):
        for kind in ("reuse", "boot", "foreign"):
            with self.subTest(kind=kind):
                current = dict(self.identity)
                if kind == "reuse":
                    current["start_ticks"] += 1
                environment = {"COCKPIT_FIXTURE_CREATION_ID": "foreign" if kind == "foreign" else self.manifest["creation_id"]}
                with patch.object(resources, "boot_id", return_value="other" if kind == "boot" else self.identity["boot_id"]), \
                     patch.object(resources, "process_identity", return_value=current), \
                     patch.object(resources, "process_environment", return_value=environment), \
                     patch("os.pidfd_open", create=True) as opened, \
                     patch("signal.pidfd_send_signal", create=True) as signalled:
                    result = resources.stop_process(self.root, self.manifest, self.journal, self.key, self.identity)
                    self.assertEqual(result["state"], "refused")
                    opened.assert_not_called()
                    signalled.assert_not_called()

    def test_interrupted_intent_only_observes_never_resignals(self):
        self.journal["effects"][self.key] = {"state": "intent"}
        with patch.object(resources, "inspect_process", return_value=("live", "pinned")), \
             patch("signal.pidfd_send_signal", create=True) as signalled:
            result = resources.stop_process(self.root, self.manifest, self.journal, self.key, self.identity)
        self.assertEqual(result["state"], "unknown")
        signalled.assert_not_called()
        self.assertEqual(resources.read_journal(self.root)["effects"][self.key]["state"], "unknown")

    def test_receipts_recover_opaque_intent_without_dispatch(self):
        intent = resources.receipt(self.root, self.journal, self.key, "intent", "before_signal")
        fixture = resources.typed_registration(self.root, self.manifest)["resources"][0]
        with patch("signal.pidfd_send_signal", create=True) as signalled:
            recovered = resources.helper_operation({
                "resource": fixture, "run_id": "worker", "root_id": "root", "action": "receipts"})
        self.assertEqual(recovered["effects"], [intent])
        self.assertNotEqual(intent["effect_id"], self.key)
        self.assertEqual(len(intent["effect_id"]), 36)
        self.assertTrue(intent["updated_at"].endswith("Z"))
        signalled.assert_not_called()

    def test_fresh_identity_after_pidfd_precedes_sigterm(self):
        descriptor = os.open("/dev/null", os.O_RDONLY)
        with patch.object(resources, "inspect_process", side_effect=[("live", "pinned"), ("refused", "process_identity_changed")]), \
             patch("os.pidfd_open", return_value=descriptor, create=True), \
             patch("signal.pidfd_send_signal", create=True) as signalled:
            result = resources.stop_process(self.root, self.manifest, self.journal, self.key, self.identity)
        self.assertEqual(result["state"], "refused")
        signalled.assert_not_called()

    def test_sigterm_intent_is_durable_then_joined(self):
        def signal_observer(*args):
            intent = resources.read_journal(self.root)["effects"][self.key]
            self.assertEqual(intent["state"], "intent")
            self.assertEqual(intent["kind"], "stop_process")
            self.assertEqual(len(intent["identity_digest"]), 64)
        descriptor = os.open("/dev/null", os.O_RDONLY)
        with patch.object(resources, "inspect_process", return_value=("live", "pinned")), \
             patch("os.pidfd_open", return_value=descriptor, create=True), \
             patch("signal.pidfd_send_signal", side_effect=signal_observer, create=True), \
             patch("select.select", return_value=([descriptor], [], [])):
            result = resources.stop_process(self.root, self.manifest, self.journal, self.key, self.identity)
        self.assertEqual(result["state"], "absent")
        self.assertEqual(result["evidence"], "pidfd_joined_exit")

    def test_missing_parent_does_not_certify_child_quiescence(self):
        child = dict(self.identity, pid=123457, start_ticks=18)
        child_key = "process:123457:18"
        with patch.object(resources, "members", return_value={child_key: child}), \
             patch.object(resources, "inspect_process", side_effect=lambda identity, manifest: ("absent", "gone") if identity == self.identity else ("live", "child")):
            result = resources.shutdown(self.root, inspect_only=True)
        self.assertNotEqual(result["state"], "absent")
        journal = resources.read_journal(self.root)
        self.assertIn(child_key, journal["members"])
        self.assertEqual(journal["effects"][self.key]["state"], "absent")

    def test_pidfile_alone_is_never_adopted(self):
        (self.root / "native.pid").write_text("555555")
        with patch.object(resources, "members", return_value={}), \
             patch.object(resources, "inspect_process", return_value=("absent", "gone")):
            result = resources.shutdown(self.root, inspect_only=True)
        self.assertEqual(result["state"], "absent")
        self.assertNotIn("555555", json.dumps(resources.read_journal(self.root)))

    def test_old_fixture_kept_without_new_lock_or_pid_adoption(self):
        (self.root / resources.MANIFEST).unlink()
        with self.assertRaises(ValueError):
            resources.shutdown(self.root)
        self.assertFalse((self.root / ".cleanup.lock").exists())

    def stopped(self):
        self.journal["effects"][self.key] = {"state": "absent"}
        resources.atomic_json(self.root / resources.JOURNAL, self.journal)

    def test_unregistered_scratch_and_live_consumer_are_kept(self):
        self.stopped()
        with self.assertRaises(ValueError):
            resources.cleanup_scratch(self.root, self.manifest["creation_id"], str(self.root / "repositories"))
        with patch.object(resources, "members", return_value={}), patch.object(resources, "references_root", return_value=True):
            result = resources.cleanup_scratch(self.root, self.manifest["creation_id"], str(self.root / "cache"))
        self.assertEqual(result["state"], "refused")
        self.assertTrue((self.root / "cache").exists())

    def test_foreign_symlink_blocks_scratch_without_following(self):
        self.stopped()
        (self.root / "cache/foreign").symlink_to(self.root / "runtime.json")
        with patch.object(resources, "members", return_value={}), patch.object(resources, "references_root", return_value=False):
            result = resources.cleanup_scratch(self.root, self.manifest["creation_id"], str(self.root / "cache"))
        self.assertEqual(result["state"], "refused")
        self.assertTrue((self.root / "runtime.json").exists())

    def test_scratch_separate_effect_after_join_retains_fixture_evidence(self):
        self.stopped()
        (self.root / "cache/generated").write_text("generated")
        with patch.object(resources, "members", return_value={}), patch.object(resources, "references_root", return_value=False):
            result = resources.cleanup_scratch(self.root, self.manifest["creation_id"], str(self.root / "cache"))
        self.assertEqual(result["state"], "absent")
        self.assertFalse((self.root / "cache").exists())
        self.assertTrue((self.root / "runtime.json").exists())
        self.assertTrue((self.root / resources.MANIFEST).exists())
        self.assertEqual(resources.read_journal(self.root)["effects"]["scratch:cache"]["kind"], "remove_scratch")

    def test_replacement_between_verification_and_reservation_is_restored_not_deleted(self):
        self.stopped()
        (self.root / "cache/generated").write_text("owned")
        rename = resources.rename_no_replace

        def replace_before_rename(source_fd, source, target_fd, target):
            if source == "cache" and target == "resource":
                os.rename(self.root / "cache", self.root / "saved-owned-cache")
                (self.root / "cache").mkdir()
                (self.root / "cache/foreign").write_text("must survive")
            rename(source_fd, source, target_fd, target)

        with patch.object(resources, "members", return_value={}), \
             patch.object(resources, "references_root", return_value=False), \
             patch.object(resources, "rename_no_replace", side_effect=replace_before_rename), \
             patch("shutil.rmtree") as removed:
            result = resources.cleanup_scratch(self.root, self.manifest["creation_id"], str(self.root / "cache"))
        self.assertEqual(result["state"], "refused")
        removed.assert_not_called()
        self.assertEqual((self.root / "cache/foreign").read_text(), "must survive")
        self.assertEqual((self.root / "saved-owned-cache/generated").read_text(), "owned")

    def test_public_replacement_during_reserved_removal_survives(self):
        self.stopped()
        (self.root / "cache/generated").write_text("owned")
        remove = resources.shutil.rmtree

        def install_replacement_then_remove_reserved(path, *, dir_fd):
            self.assertEqual(path, "resource")
            self.assertFalse((self.root / "cache").exists())
            (self.root / "cache").mkdir()
            (self.root / "cache/foreign").write_text("must survive")
            remove(path, dir_fd=dir_fd)

        with patch.object(resources, "members", return_value={}), \
             patch.object(resources, "references_root", return_value=False), \
             patch("shutil.rmtree", side_effect=install_replacement_then_remove_reserved):
            result = resources.cleanup_scratch(self.root, self.manifest["creation_id"], str(self.root / "cache"))
        self.assertEqual(result["state"], "absent")
        self.assertEqual((self.root / "cache/foreign").read_text(), "must survive")
        self.assertEqual(resources.read_journal(self.root)["reservations"]["scratch:cache"]["state"], "removed")

    def test_interrupted_reserved_root_is_not_mistaken_for_public_absence(self):
        self.stopped()
        reserved = self.root / "reserved-owned-cache"
        os.rename(self.root / "cache", reserved)
        journal = resources.read_journal(self.root)
        journal["effects"]["scratch:cache"] = {"state": "intent"}
        journal["reservations"] = {"scratch:cache": {"state": "reserved", "path": str(reserved)}}
        resources.atomic_json(self.root / resources.JOURNAL, journal)
        with patch("shutil.rmtree") as removed:
            result = resources.cleanup_scratch(
                self.root, self.manifest["creation_id"], str(self.root / "cache"), inspect_only=True)
        self.assertEqual(result["state"], "unknown")
        self.assertTrue(reserved.exists())
        removed.assert_not_called()

    def test_unknown_scratch_intent_never_blindly_repeats_removal(self):
        self.stopped()
        journal = resources.read_journal(self.root)
        journal["effects"]["scratch:cache"] = {"state": "intent"}
        resources.atomic_json(self.root / resources.JOURNAL, journal)
        with patch.object(resources, "members", return_value={}), patch.object(resources, "references_root", return_value=False), \
             patch("shutil.rmtree") as removed:
            result = resources.cleanup_scratch(self.root, self.manifest["creation_id"], str(self.root / "cache"), inspect_only=True)
        self.assertEqual(result["state"], "unknown")
        removed.assert_not_called()

    def test_unknown_scratch_missing_root_reconciles_absence(self):
        self.stopped()
        journal = resources.read_journal(self.root)
        journal["effects"]["scratch:cache"] = {"state": "intent"}
        resources.atomic_json(self.root / resources.JOURNAL, journal)
        (self.root / "cache").rmdir()
        with patch("shutil.rmtree") as removed:
            result = resources.cleanup_scratch(
                self.root, self.manifest["creation_id"], str(self.root / "cache"), inspect_only=True)
        self.assertEqual(result["state"], "absent")
        removed.assert_not_called()


if __name__ == "__main__":
    unittest.main()
