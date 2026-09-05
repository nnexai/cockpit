#!/usr/bin/env python3
"""Read-only deterministic changed-scope quality reports and gates.

This script never installs tools, changes source, starts Herdr, or updates a
baseline during report/gate runs. Optional metric providers are explicit JSON
inputs because their pinned adapters are not installed yet; absent or
unjoinable evidence is inconclusive rather than an invented zero-coverage row.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import sys
import tempfile
import time
from datetime import UTC, datetime
from pathlib import Path, PurePosixPath
from typing import Any, Iterable


EXIT_PASS = 0
EXIT_FAIL = 1
EXIT_INCONCLUSIVE = 2
EXIT_INVALID = 3
SCHEMA_VERSION = 1
MAX_CAPTURE_BYTES = 64 * 1024
ROOT = Path(__file__).resolve().parents[2]


class QualityError(ValueError):
    pass


def stable_json(value: Any) -> str:
    return json.dumps(value, indent=2, sort_keys=True, ensure_ascii=False) + "\n"


def sha256_bytes(value: bytes) -> str:
    return "sha256:" + hashlib.sha256(value).hexdigest()


def read_json(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise QualityError(f"cannot read JSON {path}: {error}") from error
    if not isinstance(value, dict):
        raise QualityError(f"JSON object required: {path}")
    return value


def run(argv: list[str], *, cwd: Path | None = None, timeout: int = 30) -> subprocess.CompletedProcess[bytes]:
    try:
        return subprocess.run(argv, cwd=ROOT if cwd is None else cwd, check=False, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=timeout)
    except (OSError, subprocess.TimeoutExpired) as error:
        raise QualityError(f"cannot execute {' '.join(argv)}: {error}") from error


def git(argv: list[str]) -> bytes:
    completed = run(["git", *argv])
    if completed.returncode != 0:
        raise QualityError(f"git {' '.join(argv)} failed: {completed.stderr.decode('utf-8', 'replace').strip()}")
    return completed.stdout


def resolved_ref(ref: str) -> str:
    return git(["rev-parse", "--verify", f"{ref}^{{commit}}"]).decode().strip()


def canonical_path(raw: str) -> str:
    if not raw or "\x00" in raw:
        raise QualityError("Git returned an empty or NUL path")
    path = PurePosixPath(raw)
    if path.is_absolute() or any(part in {"", ".", ".."} for part in path.parts):
        raise QualityError(f"Git returned an unsafe path: {raw!r}")
    return path.as_posix()


def nul_parts(value: bytes) -> list[str]:
    if value and not value.endswith(b"\0"):
        raise QualityError("expected NUL-delimited Git output")
    return [item.decode("utf-8", "surrogateescape") for item in value.split(b"\0")[:-1]]


def changed_status(base: str) -> list[tuple[str, str | None, str]]:
    parts = nul_parts(git(["diff", "--name-status", "-z", "--find-renames=50%", base]))
    result: list[tuple[str, str | None, str]] = []
    index = 0
    while index < len(parts):
        status = parts[index]
        index += 1
        if not status:
            raise QualityError("empty Git change status")
        code = status[0]
        if code in {"R", "C"}:
            if index + 1 >= len(parts):
                raise QualityError("truncated Git rename record")
            old, new = canonical_path(parts[index]), canonical_path(parts[index + 1])
            index += 2
            result.append(("renamed" if code == "R" else "copied", old, new))
        else:
            if index >= len(parts):
                raise QualityError("truncated Git change record")
            path = canonical_path(parts[index])
            index += 1
            result.append(({"A": "added", "D": "deleted", "M": "modified", "T": "type_changed"}.get(code, "modified"), path if code == "D" else None, path))
    return result


def working_sha(path: str) -> str | None:
    candidate = ROOT / path
    try:
        metadata = candidate.lstat()
    except FileNotFoundError:
        return None
    if not candidate.is_file() or candidate.is_symlink():
        return None
    return sha256_bytes(candidate.read_bytes())


def base_sha(base: str, path: str) -> str | None:
    completed = run(["git", "show", f"{base}:{path}"])
    if completed.returncode != 0:
        return None
    return sha256_bytes(completed.stdout)


def is_test_path(path: str) -> bool:
    return path.endswith((".test.ts", ".test.tsx", ".spec.ts", ".spec.tsx")) or "/tests/" in path or path.startswith("tests/")


def is_source_path(path: str) -> bool:
    if path.startswith("src/protocol/generated/") or is_test_path(path) or "/fixtures/" in path or "/mocks/" in path:
        return False
    return (path.startswith("src/") and path.endswith((".ts", ".tsx"))) or (path.startswith("crates/") and path.endswith(".rs")) or (path.startswith("src-tauri/") and path.endswith(".rs"))


def mapped_test_source(path: str) -> str | None:
    suffixes = ((".test.tsx", ".tsx"), (".test.ts", ".ts"), (".spec.tsx", ".tsx"), (".spec.ts", ".ts"))
    for test_suffix, source_suffix in suffixes:
        if path.endswith(test_suffix):
            candidate = path[: -len(test_suffix)] + source_suffix
            if working_sha(candidate) is not None:
                return candidate
    return None


def changed_scope(base: str) -> tuple[list[dict[str, Any]], list[dict[str, str]]]:
    staged = set(nul_parts(git(["diff", "--name-only", "-z", "--cached"])))
    unstaged = set(nul_parts(git(["diff", "--name-only", "-z"])))
    files: dict[str, dict[str, Any]] = {}
    diagnostics: list[dict[str, str]] = []
    deleted_languages: set[str] = set()
    added_languages: set[str] = set()
    for change, old, path in changed_status(base):
        source = is_source_path(path) or (old is not None and is_source_path(old))
        item = {
            "path": path,
            "change": change,
            "sha256": working_sha(path),
            "base_sha256": base_sha(base, old or path),
            "base_path": old,
            "source": source,
            "worktree_state": sorted(state for state, paths in (("staged", staged), ("unstaged", unstaged)) if path in paths or (old is not None and old in paths)),
        }
        files[path] = item
        if change == "deleted" and is_source_path(path):
            deleted_languages.add(Path(path).suffix)
        if change == "added" and is_source_path(path):
            added_languages.add(Path(path).suffix)
        if is_test_path(path):
            mapped = mapped_test_source(path)
            if mapped is None:
                diagnostics.append({"code": "test_only_mapping_inconclusive", "message": f"cannot map changed test {path} to a source file"})
            elif mapped not in files:
                files[mapped] = {"path": mapped, "change": "test_only", "sha256": working_sha(mapped), "base_sha256": base_sha(base, mapped), "base_path": mapped, "source": True, "worktree_state": ["test_only"]}
    for raw in nul_parts(git(["ls-files", "--others", "--exclude-standard", "-z"])):
        path = canonical_path(raw)
        if path in files:
            continue
        files[path] = {"path": path, "change": "untracked", "sha256": working_sha(path), "base_sha256": None, "base_path": None, "source": is_source_path(path), "worktree_state": ["untracked"]}
        if is_test_path(path):
            mapped = mapped_test_source(path)
            if mapped is None:
                diagnostics.append({"code": "test_only_mapping_inconclusive", "message": f"cannot map untracked test {path} to a source file"})
            elif mapped not in files:
                files[mapped] = {"path": mapped, "change": "test_only", "sha256": working_sha(mapped), "base_sha256": base_sha(base, mapped), "base_path": mapped, "source": True, "worktree_state": ["test_only"]}
    if deleted_languages & added_languages:
        diagnostics.append({"code": "rename_mapping_inconclusive", "message": "an added/deleted source pair was not an unambiguous Git rename"})
    return [files[path] for path in sorted(files)], diagnostics


def probe_versions(manifest: dict[str, Any]) -> tuple[dict[str, str | None], list[dict[str, str]]]:
    tools = manifest.get("tools")
    if not isinstance(tools, dict):
        raise QualityError("tool manifest tools must be an object")
    observed: dict[str, str | None] = {}
    diagnostics: list[dict[str, str]] = []
    for name in sorted(tools):
        entry = tools[name]
        if not isinstance(entry, dict) or not isinstance(entry.get("argv"), list):
            raise QualityError(f"tool manifest {name} is invalid")
        argv = entry["argv"]
        if not all(isinstance(value, str) and value for value in argv):
            raise QualityError(f"tool manifest {name} argv is invalid")
        completed = run(argv)
        version = completed.stdout.decode("utf-8", "replace").strip() if completed.returncode == 0 else None
        observed[name] = version
        expected = entry.get("expected")
        if expected is None:
            diagnostics.append({"code": "optional_provider_unavailable" if version is None else "optional_provider_unpinned", "message": f"{name} is {'unavailable' if version is None else 'present but unpinned'}"})
        elif version != expected:
            diagnostics.append({"code": "tool_version_mismatch", "message": f"{name} expected {expected!r}, observed {version!r}"})
    return observed, diagnostics


def provider_methods(path: Path, label: str) -> list[dict[str, Any]]:
    raw = read_json(path)
    methods = raw.get("methods")
    if not isinstance(methods, list):
        raise QualityError(f"{label} provider must contain a methods array")
    result: list[dict[str, Any]] = []
    for value in methods:
        if not isinstance(value, dict) or not all(isinstance(value.get(field), str) and value[field] for field in ("key", "language", "path", "symbol", "signature_hash")):
            raise QualityError(f"{label} provider contains an invalid method identity")
        result.append(value)
    return result


def coverage_value(value: Any, kind: str) -> dict[str, Any] | None:
    if not isinstance(value, dict) or value.get("kind") != kind:
        return None
    covered, total = value.get("covered"), value.get("total")
    if not isinstance(covered, int) or not isinstance(total, int) or covered < 0 or total <= 0 or covered > total:
        return None
    fraction = covered / total
    return {"kind": kind, "covered": covered, "total": total, "fraction": fraction}


def crap_score(complexity: int, coverage_fraction: float) -> float:
    return complexity * complexity * (1 - coverage_fraction) ** 3 + complexity


def join_methods(scope_files: list[dict[str, Any]], config: dict[str, Any], complexity_path: Path | None, coverage_path: Path | None) -> tuple[list[dict[str, Any]], list[dict[str, str]]]:
    source_paths = {item["path"] for item in scope_files if item["source"] and item["change"] != "deleted"}
    if not source_paths:
        return [], []
    diagnostics: list[dict[str, str]] = []
    if complexity_path is None:
        return [], [{"code": "complexity_provider_unavailable", "message": "no complexity provider report was supplied"}]
    complexity = provider_methods(complexity_path, "complexity")
    selected = [item for item in complexity if item["path"] in source_paths]
    if not selected:
        return [], [{"code": "complexity_mapping_inconclusive", "message": "complexity provider did not map changed source files"}]
    by_key: dict[str, dict[str, Any]] = {}
    for item in selected:
        if item["key"] in by_key or not isinstance(item.get("complexity"), int) or item["complexity"] < 1:
            return [], [{"code": "complexity_mapping_inconclusive", "message": "complexity provider has duplicate or invalid method data"}]
        by_key[item["key"]] = item
    coverage_by_key: dict[str, dict[str, Any]] = {}
    kind = config["policy"]["coverage_kind"]
    if coverage_path is None:
        diagnostics.append({"code": "coverage_provider_unavailable", "message": "no coverage provider report was supplied"})
    else:
        for item in provider_methods(coverage_path, "coverage"):
            if item["key"] in coverage_by_key:
                diagnostics.append({"code": "coverage_mapping_inconclusive", "message": f"duplicate coverage key {item['key']}"})
            else:
                coverage = coverage_value(item.get("coverage"), kind)
                if coverage is None:
                    diagnostics.append({"code": "coverage_mapping_inconclusive", "message": f"invalid or incompatible coverage for {item['key']}"})
                else:
                    coverage_by_key[item["key"]] = coverage
    methods: list[dict[str, Any]] = []
    for key in sorted(by_key):
        item = by_key[key]
        coverage = coverage_by_key.get(key)
        if coverage is None:
            diagnostics.append({"code": "coverage_mapping_inconclusive", "message": f"coverage is missing for {key}"})
            crap: dict[str, Any] = {"formula": "c^2*(1-cov)^3+c", "score": None, "status": "missing"}
        else:
            score = crap_score(item["complexity"], coverage["fraction"])
            crap = {"formula": "c^2*(1-cov)^3+c", "score": score, "status": "info"}
        declaration = item.get("declaration")
        if not isinstance(declaration, dict) or not all(isinstance(declaration.get(field), int) and declaration[field] > 0 for field in ("start_line", "end_line")):
            raise QualityError(f"complexity provider declaration is invalid for {key}")
        methods.append({"key": key, "language": item["language"], "path": item["path"], "symbol": item["symbol"], "declaration": declaration, "signature_hash": item["signature_hash"], "complexity": item["complexity"], "coverage": coverage, "crap": crap, "baseline": None})
    return methods, diagnostics


def parse_mutants(path: Path | None) -> tuple[list[dict[str, Any]], list[dict[str, str]]]:
    if path is None:
        return [], [{"code": "mutation_provider_unavailable", "message": "no mutation provider report was supplied"}]
    raw = read_json(path)
    if raw.get("status") == "not_applicable":
        if not isinstance(raw.get("reason"), str) or not raw["reason"]:
            raise QualityError("not-applicable mutation report needs a reason")
        return [], []
    mutants = raw.get("mutants")
    if not isinstance(mutants, list):
        raise QualityError("mutation provider must contain mutants or an explicit not_applicable status")
    allowed = {"killed", "survived", "no_coverage", "timeout", "compile_error", "skipped", "inconclusive"}
    result: list[dict[str, Any]] = []
    for item in mutants:
        if not isinstance(item, dict) or item.get("status") not in allowed or not all(isinstance(item.get(field), str) and item[field] for field in ("id", "language", "path", "symbol", "detail")) or not isinstance(item.get("duration_ms"), int) or item["duration_ms"] < 0:
            raise QualityError("mutation provider contains an invalid mutant")
        result.append(item)
    if not result:
        return [], [{"code": "mutation_empty_inconclusive", "message": "mutation provider returned no mutants without not_applicable"}]
    return sorted(result, key=lambda item: item["id"]), []


def mutation_summary(mutants: list[dict[str, Any]], diagnostics: list[dict[str, str]]) -> dict[str, Any]:
    statuses = ("killed", "survived", "no_coverage", "timeout", "compile_error", "skipped", "inconclusive")
    counts = {status: sum(1 for mutant in mutants if mutant["status"] == status) for status in statuses}
    if any(item["code"].startswith("mutation_") for item in diagnostics):
        state = "inconclusive"
    elif not mutants:
        state = "not_applicable"
    elif any(counts[status] for status in ("timeout", "compile_error", "skipped", "inconclusive")):
        state = "inconclusive"
    else:
        state = "complete"
    executed_denominator = counts["killed"] + counts["survived"]
    adequacy_denominator = executed_denominator + counts["no_coverage"]
    return {"status": state, "counts": counts, "executed_score": None if executed_denominator == 0 else counts["killed"] / executed_denominator, "adequacy_score": None if adequacy_denominator == 0 else counts["killed"] / adequacy_denominator}


def metrics_status(diagnostics: Iterable[dict[str, str]]) -> bool:
    return any("inconclusive" in item["code"] or "unavailable" in item["code"] or "unconfigured" in item["code"] or "unpinned" in item["code"] or item["code"] == "tool_version_mismatch" for item in diagnostics)


def run_check(check: dict[str, Any], timeout: int) -> dict[str, Any]:
    argv = check.get("argv")
    if not isinstance(check.get("name"), str) or not isinstance(argv, list) or not all(isinstance(item, str) for item in argv):
        raise QualityError("quality check configuration is invalid")
    started = time.monotonic()
    try:
        completed = subprocess.run(argv, cwd=ROOT, check=False, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=timeout)
        status = "pass" if completed.returncode == 0 else "fail"
        stdout, stderr = completed.stdout, completed.stderr
        exit_code = completed.returncode
    except (OSError, subprocess.TimeoutExpired) as error:
        status, exit_code, stdout, stderr = "inconclusive", None, b"", str(error).encode()
    return {"name": check["name"], "command": argv, "scope": "workspace", "exit_code": exit_code, "duration_ms": round((time.monotonic() - started) * 1000), "status": status, "stdout_stderr_digest": sha256_bytes((stdout + b"\n" + stderr)[:MAX_CAPTURE_BYTES])}


def build_report(args: argparse.Namespace) -> dict[str, Any]:
    config = read_json(ROOT / "quality/config.json")
    tools_manifest = read_json(ROOT / "quality/tool-versions.json")
    if config.get("schema_version") != SCHEMA_VERSION or tools_manifest.get("schema_version") != SCHEMA_VERSION:
        raise QualityError("unsupported quality configuration schema")
    if args.scope != "changed":
        raise QualityError("only --scope changed is implemented; full scope is an explicit future provider run")
    if not args.base:
        raise QualityError("--base is required; the quality gate never guesses a branch")
    base = resolved_ref(args.base)
    commit = resolved_ref("HEAD")
    started = datetime.now(UTC)
    files, diagnostics = changed_scope(base)
    observed_tools, tool_diagnostics = probe_versions(tools_manifest)
    diagnostics.extend(tool_diagnostics)
    methods, method_diagnostics = join_methods(files, config, args.complexity_report, args.coverage_report)
    diagnostics.extend(method_diagnostics)
    mutants, mutant_diagnostics = parse_mutants(args.mutation_report)
    diagnostics.extend(mutant_diagnostics)
    mutation = mutation_summary(mutants, mutant_diagnostics)
    checks = [] if args.no_checks else [run_check(check, config["limits"]["timeout_seconds"]) for check in config["checks"]]
    status = "fail" if any(check["status"] == "fail" for check in checks) else "inconclusive" if metrics_status(diagnostics) else "pass"
    duration_ms = round((datetime.now(UTC) - started).total_seconds() * 1000)
    return {"schema_version": SCHEMA_VERSION, "status": status, "commit": commit, "base": base, "scope": {"kind": "changed", "files": files}, "toolchain": {"node": observed_tools.get("node"), "bun": observed_tools.get("bun"), "rust": observed_tools.get("rustc"), "tools": observed_tools}, "policy": {**config["policy"], "quality_seed": config["quality_seed"]}, "checks": checks, "methods": methods, "mutants": mutants, "mutation": mutation, "cost": {"started_at": started.isoformat().replace("+00:00", "Z"), "duration_ms": duration_ms, "test_runs": sum(1 for check in checks if "test" in check["name"]), "mutants_run": len(mutants)}, "diagnostics": sorted(diagnostics, key=lambda item: (item["code"], item["message"]))}


def baseline_map(baseline: dict[str, Any], key: str) -> dict[str, Any] | None:
    values = baseline.get(key, [])
    if not isinstance(values, list):
        raise QualityError(f"baseline {key} must be an array")
    result: dict[str, Any] = {}
    for value in values:
        if not isinstance(value, dict) or not isinstance(value.get("key") if key == "methods" else value.get("id"), str):
            raise QualityError(f"baseline {key} contains an invalid identity")
        identity = value["key"] if key == "methods" else value["id"]
        if identity in result:
            raise QualityError(f"baseline {key} has duplicate identity {identity}")
        result[identity] = value
    return result


def reviewed_exceptions(baseline: dict[str, Any]) -> dict[tuple[str, str], dict[str, Any]]:
    values = baseline.get("exceptions", [])
    if not isinstance(values, list):
        raise QualityError("baseline exceptions must be an array")
    result: dict[tuple[str, str], dict[str, Any]] = {}
    for value in values:
        if not isinstance(value, dict) or value.get("kind") not in {"method", "mutant"} or not all(isinstance(value.get(field), str) and value[field] for field in ("id", "owner", "reason", "revalidate_on", "fingerprint")):
            raise QualityError("baseline exception requires kind, id, owner, reason, fingerprint, and revalidate_on")
        identity = (value["kind"], value["id"])
        if identity in result:
            raise QualityError(f"baseline has duplicate manual-review exception {identity[0]}:{identity[1]}")
        result[identity] = value
    return result


def exception_matches(exceptions: dict[tuple[str, str], dict[str, Any]], kind: str, identity: str, fingerprint: str | None) -> bool:
    exception = exceptions.get((kind, identity))
    return exception is not None and fingerprint is not None and exception["fingerprint"] == fingerprint


def gate_report(report: dict[str, Any], baseline: dict[str, Any], strict: bool) -> tuple[str, list[dict[str, str]]]:
    findings: list[dict[str, str]] = []
    policy = report["policy"]
    methods = baseline_map(baseline, "methods")
    mutants = baseline_map(baseline, "mutants")
    exceptions = reviewed_exceptions(baseline)
    hard = report["status"] == "fail"
    inconclusive = report["status"] == "inconclusive"
    for method in report.get("methods", []):
        baseline_method = methods.get(method["key"])
        method["baseline"] = None if baseline_method is None else {"crap": baseline_method.get("crap", {}).get("score"), "complexity": baseline_method.get("complexity"), "coverage_fraction": (baseline_method.get("coverage") or {}).get("fraction")}
        coverage, score = method.get("coverage"), method.get("crap", {}).get("score")
        if coverage is None or score is None:
            inconclusive = True
            continue
        if baseline_method is None:
            if coverage["fraction"] < policy["new_logic_coverage_floor"] or score > policy["crap_warn"]:
                hard = True
                findings.append({"code": "new_method_threshold", "message": f"{method['key']} exceeds new-code coverage or CRAP threshold"})
        else:
            old_score = baseline_method.get("crap", {}).get("score")
            old_complexity = baseline_method.get("complexity")
            old_coverage = (baseline_method.get("coverage") or {}).get("fraction")
            if not isinstance(old_score, (int, float)) or not isinstance(old_complexity, int) or not isinstance(old_coverage, (int, float)):
                inconclusive = True
                findings.append({"code": "baseline_method_inconclusive", "message": f"baseline data is incomplete for {method['key']}"})
            elif score > old_score or method["complexity"] > old_complexity or coverage["fraction"] < old_coverage:
                if exception_matches(exceptions, "method", method["key"], method.get("signature_hash")):
                    findings.append({"code": "manual_review_exception", "message": f"{method['key']} has a matching reviewed exception"})
                else:
                    hard = True
                    findings.append({"code": "legacy_regression", "message": f"{method['key']} regressed from its reviewed baseline"})
        if score > policy["crap_hard"]:
            hard = True
            findings.append({"code": "crap_hard", "message": f"{method['key']} exceeds the hard CRAP threshold"})
    for mutant in report.get("mutants", []):
        status = mutant["status"]
        before = mutants.get(mutant["id"], {}).get("status")
        if status in {"timeout", "compile_error", "skipped", "inconclusive"}:
            inconclusive = True
        if status in {"survived", "no_coverage"} and before not in {"survived", "no_coverage"}:
            if exception_matches(exceptions, "mutant", mutant["id"], mutant.get("fingerprint")):
                findings.append({"code": "manual_review_exception", "message": f"{mutant['id']} has a matching reviewed exception"})
            else:
                hard = True
                findings.append({"code": "new_mutant_failure", "message": f"{mutant['id']} is {status}"})
    if hard:
        return "fail", findings
    if inconclusive:
        return "inconclusive", findings
    return "pass", findings


def default_output(report: dict[str, Any]) -> Path:
    return ROOT / "quality/reports" / f"{report['commit'][:12]}-{report['base'][:12]}-{report['scope']['kind']}.json"


def write_report(report: dict[str, Any], output: Path | None) -> Path:
    target = default_output(report) if output is None else output
    if not target.is_absolute():
        target = ROOT / target
    try:
        target.resolve().relative_to(ROOT.resolve())
    except ValueError as error:
        raise QualityError("quality reports must remain beneath the repository root") from error
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(stable_json(report), encoding="utf-8")
    return target


def command_report(args: argparse.Namespace) -> int:
    report = build_report(args)
    target = write_report(report, args.output)
    print(stable_json({"status": report["status"], "report": str(target.relative_to(ROOT)), "base": report["base"], "commit": report["commit"]}), end="")
    return EXIT_FAIL if report["status"] == "fail" else EXIT_PASS


def command_gate(args: argparse.Namespace) -> int:
    report = build_report(args)
    target = write_report(report, args.output)
    baseline = read_json(ROOT / "quality/baseline.json")
    status, findings = gate_report(report, baseline, args.strict)
    print(stable_json({"status": status, "report": str(target.relative_to(ROOT)), "findings": findings}), end="")
    if status == "fail":
        return EXIT_FAIL
    if status == "inconclusive" and args.strict:
        return EXIT_INCONCLUSIVE
    return EXIT_PASS


def command_probe(args: argparse.Namespace) -> int:
    del args
    manifest = read_json(ROOT / "quality/tool-versions.json")
    observed, diagnostics = probe_versions(manifest)
    print(stable_json({"schema_version": SCHEMA_VERSION, "status": "inconclusive" if diagnostics else "pass", "tools": observed, "diagnostics": diagnostics}), end="")
    return EXIT_INCONCLUSIVE if diagnostics else EXIT_PASS


def command_baseline(args: argparse.Namespace) -> int:
    report = read_json(args.from_report)
    if report.get("schema_version") != SCHEMA_VERSION:
        raise QualityError("unsupported report schema")
    current = read_json(ROOT / "quality/baseline.json")
    candidate = {"schema_version": SCHEMA_VERSION, "commit": report.get("commit"), "policy": report.get("policy"), "toolchain": report.get("toolchain"), "methods": sorted(report.get("methods", []), key=lambda item: item["key"]), "mutants": sorted(report.get("mutants", []), key=lambda item: item["id"]), "exceptions": current.get("exceptions", []), "incomplete_data_exceptions": current.get("incomplete_data_exceptions", [])}
    if not args.write:
        print(stable_json({"status": "review_required", "baseline": candidate}), end="")
        return EXIT_PASS
    (ROOT / "quality/baseline.json").write_text(stable_json(candidate), encoding="utf-8")
    print(stable_json({"status": "written", "path": "quality/baseline.json"}), end="")
    return EXIT_PASS


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(description=__doc__)
    commands = root.add_subparsers(dest="command", required=True)
    for name in ("report", "gate"):
        command = commands.add_parser(name)
        command.add_argument("--base", required=True)
        command.add_argument("--scope", default="changed", choices=("changed", "full"))
        command.add_argument("--complexity-report", type=Path)
        command.add_argument("--coverage-report", type=Path)
        command.add_argument("--mutation-report", type=Path)
        command.add_argument("--output", type=Path)
        command.add_argument("--no-checks", action="store_true")
    commands.choices["gate"].add_argument("--strict", action="store_true")
    commands.add_parser("probe")
    baseline = commands.add_parser("baseline")
    baseline.add_argument("--from", dest="from_report", type=Path, required=True)
    baseline.add_argument("--write", action="store_true")
    return root


def main(argv: list[str] | None = None) -> int:
    args = parser().parse_args(argv)
    try:
        if args.command == "report":
            return command_report(args)
        if args.command == "gate":
            return command_gate(args)
        if args.command == "probe":
            return command_probe(args)
        if args.command == "baseline":
            return command_baseline(args)
        raise QualityError("unknown command")
    except QualityError as error:
        print(stable_json({"schema_version": SCHEMA_VERSION, "status": "invalid", "error": str(error)}), file=sys.stderr, end="")
        return EXIT_INVALID


if __name__ == "__main__":
    raise SystemExit(main())
