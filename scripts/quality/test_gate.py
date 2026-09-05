#!/usr/bin/env python3
"""Hermetic contract tests for the deterministic quality gate."""

from __future__ import annotations

import importlib.util
import json
import subprocess
import tempfile
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).with_name("gate.py")
SPEC = importlib.util.spec_from_file_location("quality_gate", MODULE_PATH)
assert SPEC and SPEC.loader
quality_gate = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(quality_gate)


def git(root: Path, *argv: str) -> str:
    completed = subprocess.run(["git", *argv], cwd=root, check=True, text=True, stdout=subprocess.PIPE)
    return completed.stdout.strip()


def method(key: str, coverage: float, complexity: int = 6) -> dict[str, object]:
    total = 100
    covered = round(coverage * total)
    return {
        "key": key,
        "language": "ts",
        "path": "src/example.ts",
        "symbol": "example",
        "declaration": {"start_line": 1, "end_line": 4},
        "signature_hash": "sha256:fixture",
        "complexity": complexity,
        "coverage": {"kind": "line", "covered": covered, "total": total, "fraction": covered / total},
        "crap": {"formula": "c^2*(1-cov)^3+c", "score": quality_gate.crap_score(complexity, covered / total), "status": "info"},
        "baseline": None,
    }


class QualityGateTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tempdir = tempfile.TemporaryDirectory()
        self.root = Path(self.tempdir.name)
        self.previous_root = quality_gate.ROOT
        quality_gate.ROOT = self.root
        git(self.root, "init", "-q")
        git(self.root, "config", "user.email", "quality@example.test")
        git(self.root, "config", "user.name", "Quality Fixture")
        (self.root / "src").mkdir()
        (self.root / "crates/core/src").mkdir(parents=True)
        (self.root / "src/old.ts").write_text("export const old = 1;\n", encoding="utf-8")
        (self.root / "src/delete.ts").write_text("export const deleted = 1;\n", encoding="utf-8")
        (self.root / "src/changed.ts").write_text("export const changed = 1;\n", encoding="utf-8")
        (self.root / "crates/core/src/lib.rs").write_text("pub fn fixture() {}\n", encoding="utf-8")
        git(self.root, "add", ".")
        git(self.root, "commit", "-qm", "fixture base")
        self.base = git(self.root, "rev-parse", "HEAD")

    def tearDown(self) -> None:
        quality_gate.ROOT = self.previous_root
        self.tempdir.cleanup()

    def test_changed_scope_enumerates_rename_deletion_unstaged_and_untracked_bytes(self) -> None:
        git(self.root, "mv", "src/old.ts", "src/renamed.ts")
        (self.root / "src/delete.ts").unlink()
        (self.root / "src/changed.ts").write_text("export const changed = 2;\n", encoding="utf-8")
        (self.root / "src/new.ts").write_text("export const newFile = 1;\n", encoding="utf-8")
        first, diagnostics = quality_gate.changed_scope(self.base)
        second, repeated_diagnostics = quality_gate.changed_scope(self.base)
        self.assertEqual(first, second)
        self.assertEqual(diagnostics, repeated_diagnostics)
        entries = {item["path"]: item for item in first}
        self.assertEqual(entries["src/renamed.ts"]["change"], "renamed")
        self.assertEqual(entries["src/renamed.ts"]["base_path"], "src/old.ts")
        self.assertEqual(entries["src/delete.ts"]["change"], "deleted")
        self.assertIsNone(entries["src/delete.ts"]["sha256"])
        self.assertTrue(entries["src/delete.ts"]["base_sha256"].startswith("sha256:"))
        self.assertEqual(entries["src/changed.ts"]["change"], "modified")
        self.assertTrue(entries["src/changed.ts"]["sha256"].startswith("sha256:"))
        self.assertEqual(entries["src/new.ts"]["change"], "untracked")
        self.assertIsNone(entries["src/new.ts"]["base_sha256"])

    def test_crap_formula_matches_declared_examples(self) -> None:
        self.assertEqual(quality_gate.crap_score(6, 1), 6)
        self.assertAlmostEqual(quality_gate.crap_score(6, 0.8), 6.288)
        self.assertAlmostEqual(quality_gate.crap_score(7, 0.5), 13.125)
        self.assertEqual(quality_gate.crap_score(8, 0), 72)

    def test_missing_coverage_is_not_zero_and_is_inconclusive(self) -> None:
        complexity = self.root / "complexity.json"
        complexity.write_text(json.dumps({"methods": [{"key": "ts:src/changed.ts:changed", "language": "ts", "path": "src/changed.ts", "symbol": "changed", "signature_hash": "sha256:one", "declaration": {"start_line": 1, "end_line": 1}, "complexity": 6}]}), encoding="utf-8")
        config = {"policy": {"coverage_kind": "line"}}
        methods, diagnostics = quality_gate.join_methods([{"path": "src/changed.ts", "source": True, "change": "modified"}], config, complexity, None)
        self.assertIsNone(methods[0]["coverage"])
        self.assertIsNone(methods[0]["crap"]["score"])
        self.assertTrue(any(item["code"] == "coverage_provider_unavailable" for item in diagnostics))

    def test_provider_join_uses_stable_keys_and_declared_crap_variant(self) -> None:
        complexity = self.root / "complexity.json"
        coverage = self.root / "coverage.json"
        identity = {"key": "ts:src/changed.ts:changed", "language": "ts", "path": "src/changed.ts", "symbol": "changed", "signature_hash": "sha256:one"}
        complexity.write_text(json.dumps({"methods": [{**identity, "declaration": {"start_line": 1, "end_line": 1}, "complexity": 6}]}), encoding="utf-8")
        coverage.write_text(json.dumps({"methods": [{**identity, "coverage": {"kind": "line", "covered": 80, "total": 100}}]}), encoding="utf-8")
        methods, diagnostics = quality_gate.join_methods([{"path": "src/changed.ts", "source": True, "change": "modified"}], {"policy": {"coverage_kind": "line"}}, complexity, coverage)
        self.assertEqual(diagnostics, [])
        self.assertEqual(methods[0]["coverage"]["fraction"], 0.8)
        self.assertAlmostEqual(methods[0]["crap"]["score"], 6.288)

    def test_new_coverage_and_legacy_ratchets_fail(self) -> None:
        policy = {"crap_warn": 8, "crap_hard": 30, "new_logic_coverage_floor": 0.9}
        report = {"status": "pass", "policy": policy, "methods": [method("ts:src/example.ts:example", 0.89)], "mutants": []}
        status, findings = quality_gate.gate_report(report, {"methods": [], "mutants": []}, strict=True)
        self.assertEqual(status, "fail")
        self.assertEqual(findings[0]["code"], "new_method_threshold")
        report["methods"] = [method("ts:src/example.ts:example", 0.90)]
        baseline = {"methods": [method("ts:src/example.ts:example", 0.95)], "mutants": []}
        status, findings = quality_gate.gate_report(report, baseline, strict=True)
        self.assertEqual(status, "fail")
        self.assertEqual(findings[0]["code"], "legacy_regression")


if __name__ == "__main__":
    unittest.main()
