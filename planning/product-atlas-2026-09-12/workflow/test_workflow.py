import hashlib
import copy

import contextlib
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import workflow

class WorkflowEvidenceTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.frags = workflow.load_fragments(Path(__file__).parent)
        cls.packet = next(p for p in workflow.expand_packets(cls.frags)
                          if p["phase"] == "capture-lanes" and len(p["surfaces"]) > 1)

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.run = Path(self.tmp.name) / "run-test"
        self.run.mkdir()

    def tearDown(self):
        self.tmp.cleanup()

    def _png(self, path, width, height, marker=b""):
        # Header-only PNG is sufficient for the engine's dimension gate.
        data = (b"\x89PNG\r\n\x1a\n" + b"\x00\x00\x00\rIHDR" +
                width.to_bytes(4, "big") + height.to_bytes(4, "big") +
                b"\x08\x06\x00\x00\x00" + marker)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        return hashlib.sha256(data).hexdigest()

    def _receipt(self, result="PASS"):
        times = {"before": "2026-09-12T00:00:00Z",
                 "action": "2026-09-12T00:00:01Z",
                 "after": "2026-09-12T00:00:02Z"}
        capture = "capture-0001"
        arts = []
        for phase in ("before", "action-01", "after"):
            stamp = times["action" if phase.startswith("action") else phase]
            for width, height in ((1440, 900), (1024, 640)):
                name = f"{phase}-state-{width}x{height}.png"
                path = self.run / name
                sha = self._png(path, width, height, phase.encode())
                state_id = "action" if phase.startswith("action") else phase
                arts.append({"path": name, "kind": "screenshot", "evidenceClass": "runtime",
                             "stateId": state_id, "viewport": f"{width}x{height}",
                             "scenarioId": self.packet["scenarioId"], "captureId": capture,
                             "capturedAt": stamp, "sourceCommit": self.frags["coverage-matrix"]["baselineCommit"],
                             "sha256": sha, "provenance": {"surface": self.packet["surfaces"][0]}})
        machine_path = self.run / "machine-evidence.json"
        machine = {"surface": self.packet["surfaces"], "url": "/cockpit/" + self.packet["cellId"],
                   "title": "Cockpit " + self.packet["surfaces"][0],
                   "session": "atlas-run-session", "stateIds": ["before", "action", "after"]}
        machine_path.write_text(json.dumps(machine), encoding="utf-8")
        machine_art = {"path": "machine-evidence.json", "kind": "machine-evidence",
                       "evidenceClass": "runtime", "scenarioId": self.packet["scenarioId"],
                       "captureId": capture, "capturedAt": times["after"],
                       "sourceCommit": self.frags["coverage-matrix"]["baselineCommit"],
                       "sha256": hashlib.sha256(machine_path.read_bytes()).hexdigest(),
                       "provenance": {"surface": self.packet["surfaces"]}}
        arts.append(machine_art)
        action_path = self.run / "actions.jsonl"
        action = {"sequence": 1, "at": times["action"], "actor": "worker",
                  "surface": self.packet["surfaces"][0], "action": "change",
                  "target": "control", "inputs": {}, "expectedAck": "ok",
                  "observedAck": "ok", "authorityRefs": ["authority-" + workflow._norm(self.packet["surfaces"][0])],
                  "artifactRefs": ["action-01-state-1440x900.png"], "outcome": "ok"}
        action_path.write_text(json.dumps(action) + "\n", encoding="utf-8")
        arts.append({"path": "actions.jsonl", "kind": "action-log", "evidenceClass": "runtime",
                     "scenarioId": self.packet["scenarioId"], "captureId": capture,
                     "capturedAt": times["action"], "sourceCommit": self.frags["coverage-matrix"]["baselineCommit"],
                     "sha256": hashlib.sha256(action_path.read_bytes()).hexdigest(),
                     "provenance": {"surface": self.packet["surfaces"][0]}})
        auth = []
        for surface in self.packet["surfaces"]:
            auth.append({"authorityId": "authority-" + workflow._norm(surface), "surface": surface,
                         "capturedAt": times["after"], "sourceCommit": self.frags["coverage-matrix"]["baselineCommit"],
                         "sessionId": "atlas-run-session", "ownedResourceIds": ["atlas-run-session"],
                         "stateDigest": "state", "sha256": "a" * 64, "redaction": "secrets-redacted"})
        state = []
        for phase in ("before", "action", "after"):
            state.append({"stateId": phase, "phase": phase, "observedAt": times[phase],
                          "surface": self.packet["surfaces"][0], "visibleAssertions": ["observed"],
                          "authorityRefs": ["authority-" + workflow._norm(self.packet["surfaces"][0])],
                          "artifactRefs": [f"{phase if phase != 'action' else 'action-01'}-state-1440x900.png"],
                          "result": result})
        return {"scenarioId": self.packet["scenarioId"], "captureId": capture,
                "startedAt": times["before"], "finishedAt": times["after"],
                "sourceCommit": self.frags["coverage-matrix"]["baselineCommit"], "client": "browser",
                "runtimeIdentity": {"runId": self.run.name, "lane": self.packet["lane"], "owner": "worker"},
                "viewportSet": ["1440x900", "1024x640"], "disposableSession": "atlas-run-session",
                "browserProfile": "atlas-run-profile", "fixtureIdentity": "atlas-run-fixture",
                "authoritySnapshot": auth, "actionLog": "actions.jsonl", "stateTimeline": state,
                "artifacts": arts, "result": result,
                "cleanup": {"completed": True, "ownershipVerified": True, "residuals": []}}

    def _errors(self, receipt):
        return workflow.validate_receipt(self.packet, receipt, self.run, self.frags)

    def test_first_picture_rejected(self):
        receipt = self._receipt()
        receipt["artifacts"] = [a for a in receipt["artifacts"] if Path(a["path"]).name.startswith("before-")]
        predicates = {e["predicate"] for e in self._errors(receipt)}
        self.assertIn("screenshot.after.1440x900", predicates)

    def test_wrong_viewport_rejected(self):
        receipt = self._receipt()
        target = next(a for a in receipt["artifacts"] if a["path"].endswith("after-state-1024x640.png"))
        self._png(self.run / target["path"], 1440, 900)
        target["sha256"] = hashlib.sha256((self.run / target["path"]).read_bytes()).hexdigest()
        self.assertIn("screenshot.viewport-dimensions", {e["predicate"] for e in self._errors(receipt)})

    def test_mock_as_runtime_rejected(self):
        receipt = self._receipt()
        receipt["artifacts"][0]["evidenceClass"] = "mock"
        self.assertIn("screenshots.runtime-evidence", {e["predicate"] for e in self._errors(receipt)})

    def test_premature_inconclusive_rejected(self):
        receipt = self._receipt("INCONCLUSIVE")
        receipt["attempts"] = 1
        receipt["diagnostics"] = [{"diagnosticId": "d", "at": "2026-09-12T00:00:00Z", "category": "timeout",
                                   "symptom": "timeout", "attempts": 1, "logsOrTraceRefs": [], "screenshotsRefs": [],
                                   "environmentRefs": [], "lastKnownState": "before", "ownershipImpact": "none", "nextStep": "retry"}]
        self.assertIn("result.INCONCLUSIVE.retry-exhausted", {e["predicate"] for e in self._errors(receipt)})

    def test_protected_resource_rejected(self):
        receipt = self._receipt()
        receipt["disposableSession"] = "default"
        self.assertIn("safety.protected-resource", {e["predicate"] for e in self._errors(receipt)})

    def test_missing_cross_surface_provenance_rejected(self):
        receipt = self._receipt()
        receipt["authoritySnapshot"] = receipt["authoritySnapshot"][:1]
        self.assertIn("authority.cross-surface-provenance", {e["predicate"] for e in self._errors(receipt)})

    def test_complete_pass_acceptance(self):
        self.assertEqual(self._errors(self._receipt()), [])
    def test_generic_fixture_pass_is_rejected(self):
        receipt = self._receipt()
        machine = self.run / "machine-evidence.json"
        machine.write_text(json.dumps({"surface": "Atlas fixture app",
                                       "url": "http://127.0.0.1:49329/index.html",
                                       "title": "Atlas fixture app"}), encoding="utf-8")
        art = next(a for a in receipt["artifacts"] if a["kind"] == "machine-evidence")
        art["sha256"] = hashlib.sha256(machine.read_bytes()).hexdigest()
        predicates = {e["predicate"] for e in self._errors(receipt)}
        self.assertIn("runtime.surface", predicates)

    def test_duplicate_state_screenshots_are_rejected(self):
        receipt = self._receipt()
        before = self.run / "before-state-1440x900.png"
        action = self.run / "action-01-state-1440x900.png"
        action.write_bytes(before.read_bytes())
        target = next(a for a in receipt["artifacts"] if a["path"] == action.name)
        target["sha256"] = hashlib.sha256(action.read_bytes()).hexdigest()
        self.assertIn("screenshot.duplicate-state",
                      {e["predicate"] for e in self._errors(receipt)})

    def test_malformed_refs_and_inline_jsonl_are_rejected(self):
        receipt = self._receipt()
        receipt["actionLog"] = [{"sequence": 1}]
        receipt["stateTimeline"][0]["artifactRefs"] = ["missing-artifact"]
        predicates = {e["predicate"] for e in self._errors(receipt)}
        self.assertIn("actionLog.jsonl", predicates)
        self.assertIn("reference.artifact", predicates)

    def test_same_timestamp_actions_are_rejected(self):
        receipt = self._receipt()
        action_path = self.run / "actions.jsonl"
        first = json.loads(action_path.read_text())
        second = dict(first)
        second["sequence"] = 2
        action_path.write_text(json.dumps(first) + "\n" + json.dumps(second) + "\n")
        target = next(a for a in receipt["artifacts"] if a["path"] == "actions.jsonl")
        target["sha256"] = hashlib.sha256(action_path.read_bytes()).hexdigest()
        self.assertIn("actionLog.timestamp-order",
                      {e["predicate"] for e in self._errors(receipt)})

    def test_foreign_review_surface_is_rejected(self):
        receipt = self._receipt()
        receipt["stateTimeline"][0]["surface"] = "Foreign Review"
        self.assertIn("surface.expected",
                      {e["predicate"] for e in self._errors(receipt)})

    def test_cross_packet_screenshot_reuse_is_rejected(self):
        receipt = self._receipt()
        packet2 = copy.deepcopy(self.packet)
        packet2["packetId"] = "packet-second"
        run_meta = {"runId": self.run.name,
                    "baselineCommit": self.frags["coverage-matrix"]["baselineCommit"]}
        workflow.atomic_write(self.run / "run.json", run_meta)
        workflow.atomic_write(self.run / "workload.json", {"packets": [self.packet, packet2]})
        stored = self.run / "receipts" / "packet-first.json"
        workflow.atomic_write(stored, receipt)
        submission = {"packetId": self.packet["packetId"], "receipt": receipt,
                       "receiptPath": str(stored.relative_to(self.run)),
                       "receiptSha256": workflow.digest(receipt), "result": "PASS"}
        submission2 = dict(submission)
        submission2["packetId"] = packet2["packetId"]
        workflow.atomic_write(self.run / "submissions.json", [submission, submission2])
        issues = workflow._issues(self.run, self.frags)
        self.assertIn("screenshot.cross-packet-reuse",
                      {e["predicate"] for e in issues})


    def test_distinct_packets_same_lane_are_claimable(self):
        root = Path(self.tmp.name) / "workflow"
        root.mkdir()
        for name in workflow.FRAGMENTS:
            (root / name).write_bytes((Path(__file__).parent / name).read_bytes())
        init_args = workflow.parser().parse_args(["--root", str(root), "--run-id", "claims-run", "init", "--json"])
        self.assertEqual(workflow.cmd_init(init_args), 0)
        run = root / "runs" / "claims-run"
        workflow.atomic_write(run / "bootstrap" / "receipt.json", {"phase": "bootstrap"})
        workflow.atomic_write(run / "fixture" / "receipt.json", {"phase": "fixture"})
        packets = [p for p in workflow.read_run(run, "workload.json", {})["packets"] if p["lane"] == "setup-context-review"]
        a = workflow.parser().parse_args(["--root", str(root), "--run-id", "claims-run", "claim", "--worker", "worker-a", "--packet", packets[0]["packetId"], "--json"])
        b = workflow.parser().parse_args(["--root", str(root), "--run-id", "claims-run", "claim", "--worker", "worker-b", "--packet", packets[1]["packetId"], "--json"])
        self.assertEqual(workflow.cmd_claim(a), 0)
        self.assertEqual(workflow.cmd_claim(b), 0)
        duplicate = workflow.parser().parse_args(["--root", str(root), "--run-id", "claims-run", "claim", "--worker", "worker-c", "--packet", packets[0]["packetId"], "--json"])
        self.assertEqual(workflow.cmd_claim(duplicate), 2)
        claims = workflow.read_run(run, "claims.json", [])
        self.assertEqual({c["worker"] for c in claims}, {"worker-a", "worker-b"})

    def _repair_run(self, receipt=None):
        root = Path(self.tmp.name) / "repair-workflow"
        root.mkdir()
        old_run = self.run
        for name in workflow.FRAGMENTS:
            (root / name).write_bytes((Path(__file__).parent / name).read_bytes())
        run = root / "runs" / "repair-run"
        run.mkdir(parents=True)
        self.run = run
        workflow.atomic_write(run / "run.json", {
            "runId": run.name, "baselineCommit": self.frags["coverage-matrix"]["baselineCommit"]})
        workflow.atomic_write(run / "workload.json", {"packets": [self.packet]})
        for name, value in (("claims.json", []), ("submissions.json", []),
                            ("repairs.json", []), ("rejections.json", [])):
            workflow.atomic_write(run / name, value)
        if receipt is not None:
            receipt = copy.deepcopy(receipt)
            receipt["runtimeIdentity"]["runId"] = run.name
            for art in receipt.get("artifacts", []):
                source = old_run / art["path"]
                target = run / art["path"]
                if source.exists():
                    target.parent.mkdir(parents=True, exist_ok=True)
                    target.write_bytes(source.read_bytes())
            stored = run / "receipts" / "original.json"
            workflow.atomic_write(stored, receipt)
            workflow.atomic_write(run / "submissions.json", [{
                "packetId": self.packet["packetId"], "result": "PASS",
                "receipt": receipt, "receiptPath": str(stored.relative_to(run)),
                "receiptSha256": workflow.digest(receipt), "worker": "original"}])
        return root, run

    def _repair(self, root):
        args = workflow.parser().parse_args([
            "--root", str(root), "--run-id", "repair-run", "repair", "--json"])
        self.assertEqual(workflow.cmd_repair(args), 0)
        return workflow.read_run(self.run, "repairs.json", [])

    def _claim_repair(self, root, repair):
        args = workflow.parser().parse_args([
            "--root", str(root), "--run-id", "repair-run", "claim",
            "--worker", "repair-worker", "--packet", repair["repairPacketId"], "--json"])
        self.assertEqual(workflow.cmd_claim(args), 0)

    def _submit_repair(self, root, repair, receipt):
        path = self.run / "replacement.json"
        workflow.atomic_write(path, receipt)
        args = workflow.parser().parse_args([
            "--root", str(root), "--run-id", "repair-run", "submit",
            "--packet", repair["repairPacketId"], "--repair-id", repair["repairId"],
            "--worker", "repair-worker", "--receipt", str(path), "--json"])
        return workflow.cmd_submit(args)

    def test_invalid_receipt_can_be_superseded_and_original_is_immutable(self):
        invalid = self._receipt()
        invalid["artifacts"] = [a for a in invalid["artifacts"]
                                if not Path(a["path"]).name.startswith("after-")]
        root, run = self._repair_run(invalid)
        original_hash = workflow.packet_records(run)[0]["receiptSha256"]
        repairs = self._repair(root)
        self.assertEqual(len(repairs), 1)
        self._claim_repair(root, repairs[0])
        self.assertEqual(self._submit_repair(root, repairs[0], self._receipt()), 0)
        submissions = workflow.packet_records(run)
        self.assertEqual(len(submissions), 2)
        self.assertEqual(submissions[0]["receiptSha256"], original_hash)
        self.assertEqual(submissions[1]["supersedes"], original_hash)
        validate_args = workflow.parser().parse_args([
            "--root", str(root), "--run-id", "repair-run", "validate", "--json"])
        self.assertEqual(workflow.cmd_validate(validate_args), 0)

    def test_valid_receipt_cannot_be_superseded(self):
        root, run = self._repair_run(self._receipt())
        self.assertEqual(self._repair(root), [])
        self.assertEqual(workflow.read_run(run, "submissions.json", [])[0]["receipt"]["result"], "PASS")

    def test_failed_replacement_remains_ready_until_retry_budget(self):
        invalid = self._receipt()
        invalid["artifacts"] = []
        root, _ = self._repair_run(invalid)
        first = self._repair(root)[0]
        self._claim_repair(root, first)
        self.assertEqual(self._submit_repair(root, first, invalid), 2)
        ready, _ = workflow.ready_packets(self.run)
        self.assertEqual(sum(p.get("status") == "REPAIR" for p in ready), 1)
        second = workflow.read_run(self.run, "repairs.json", [])[-1]
        self._claim_repair(root, second)
        self.assertEqual(self._submit_repair(root, second, invalid), 2)
        ready, _ = workflow.ready_packets(self.run)
        self.assertEqual(sum(p.get("status") == "REPAIR" for p in ready), 0)

    def test_coverage_uses_newest_audit_valid_attempt(self):
        invalid = self._receipt()
        invalid["artifacts"] = []
        root, run = self._repair_run(invalid)
        first = self._repair(root)[0]
        self._claim_repair(root, first)
        self.assertEqual(self._submit_repair(root, first, self._receipt()), 0)
        rows = workflow.packet_records(run)
        rows.append({"packetId": "repair-packet-failed", "originalPacketId": self.packet["packetId"],
                     "result": "PASS", "auditValid": False, "receipt": invalid,
                     "receiptSha256": workflow.digest(invalid), "attempt": 99})
        workflow.atomic_write(run / "submissions.json", rows)
        args = workflow.parser().parse_args([
            "--root", str(root), "--run-id", "repair-run", "validate", "--json"])
        workflow.cmd_validate(args)
        coverage = workflow.load_json(run / "coverage.json")
        self.assertEqual(coverage["cells"][0]["result"], "PASS")
    def test_repair_generation_preserves_exact_predicate(self):
        root = Path(self.tmp.name) / "workflow"
        root.mkdir()
        for name in workflow.FRAGMENTS:
            (root / name).write_bytes((Path(__file__).parent / name).read_bytes())
        args = workflow.parser().parse_args(["--root", str(root), "--run-id", "repair-run", "init", "--json"])
        self.assertEqual(workflow.cmd_init(args), 0)
        run = root / "runs" / "repair-run"
        packet = workflow.read_run(run, "workload.json", {})["packets"][2]
        (run / "rejections.json").write_text(json.dumps([{"packetId": packet["packetId"], "errors": [{"predicate": "screenshot.after.1440x900", "detail": "missing"}]}]))
        repair_args = workflow.parser().parse_args(["--root", str(root), "--run-id", "repair-run", "repair", "--json"])
        self.assertEqual(workflow.cmd_repair(repair_args), 0)
        repairs = workflow.read_run(run, "repairs.json", [])
        self.assertIn("screenshot.after.1440x900", {r["predicate"] for r in repairs})

    def test_unmarked_failed_terminal_with_current_audit_issue_is_repairable(self):
        receipt = self._receipt()
        root, run = self._repair_run(receipt)
        receipt["runtimeIdentity"]["runId"] = run.name
        packet2 = copy.deepcopy(self.packet)
        packet2["packetId"] = "packet-cross-reuse"
        workflow.atomic_write(run / "workload.json", {"packets": [self.packet, packet2]})
        first_path = run / "receipts" / "first.json"
        second_path = run / "receipts" / "second.json"
        workflow.atomic_write(first_path, receipt)
        workflow.atomic_write(second_path, receipt)
        workflow.atomic_write(run / "submissions.json", [
            {"packetId": self.packet["packetId"], "result": "PASS", "receipt": receipt,
             "receiptPath": str(first_path.relative_to(run)),
             "receiptSha256": workflow.digest(receipt), "auditValid": True},
            {"packetId": packet2["packetId"], "result": "FAIL", "receipt": receipt,
             "receiptPath": str(second_path.relative_to(run)),
             "receiptSha256": workflow.digest(receipt)}])
        repairs = self._repair(root)
        self.assertEqual({r["packetId"] for r in repairs}, {packet2["packetId"]})

    def test_completed_repair_is_hidden_from_ready_status_and_selection(self):
        invalid = self._receipt()
        invalid["artifacts"] = []
        root, run = self._repair_run(invalid)
        repair = self._repair(root)[0]
        valid = self._receipt()
        valid["runtimeIdentity"]["runId"] = run.name
        rows = workflow.packet_records(run)
        rows.append({
            "packetId": repair["repairPacketId"],
            "originalPacketId": self.packet["packetId"],
            "result": "PASS",
            "auditValid": True,
            "receipt": valid,
            "receiptSha256": workflow.digest(valid),
            "repairId": repair["repairId"],
            "supersedes": repair["supersedes"],
        })
        workflow.atomic_write(run / "submissions.json", rows)

        ready, _ = workflow.ready_packets(run)
        self.assertNotIn(repair["repairPacketId"], {p["packetId"] for p in ready})

        output = io.StringIO()
        status_args = workflow.parser().parse_args([
            "--root", str(root), "--run-id", "repair-run", "status", "--json"])
        with contextlib.redirect_stdout(output):
            self.assertEqual(workflow.cmd_status(status_args), 0)
        status = json.loads(output.getvalue())
        self.assertEqual(status["repairReadyCount"], 0)
        self.assertEqual(status["repairPackets"], [])

        output = io.StringIO()
        repair_args = workflow.parser().parse_args([
            "--root", str(root), "--run-id", "repair-run", "repair", "--json"])
        with contextlib.redirect_stdout(output):
            self.assertEqual(workflow.cmd_repair(repair_args), 0)
        self.assertEqual(json.loads(output.getvalue())["repairCount"], 0)


if __name__ == "__main__":
    unittest.main()
