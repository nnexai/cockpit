#!/usr/bin/env python3
"""Opt-in, file-targeted mutation runs; never a score gate."""

from __future__ import annotations

import argparse
import importlib.util
import json
import math
import os
import re
import signal
import subprocess
import sys
import time
import tomllib
from collections import Counter
from contextlib import nullcontext
from datetime import datetime, timezone
from pathlib import Path

SPEC = importlib.util.spec_from_file_location("quality_gate", Path(__file__).with_name("gate.py"))
assert SPEC and SPEC.loader
gate = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(gate)
ROOT = gate.ROOT


class Parser(argparse.ArgumentParser):
    def error(self, message: str) -> None:
        raise gate.QualityError(message)


class BudgetExceeded(Exception):
    pass


class RunInterrupted(Exception):
    pass


def target_path(value: str, language: str) -> str:
    """Accept literal, canonical source paths, never tool glob expressions."""
    path = Path(value)
    if path.is_absolute() or value != path.as_posix() or any(part in (".", "..") for part in path.parts):
        raise gate.QualityError(f"target must be a canonical repository-relative path: {value}")
    if any(char in value for char in "*?[]{}!\\\n\r"):
        raise gate.QualityError(f"target must be a literal file, not a glob: {value}")
    full = ROOT / path
    if not full.is_file() or full.resolve() != full:
        raise gate.QualityError(f"target must be an existing non-symlink file: {value}")
    if not gate.is_source_path(value) or gate.is_test_path(value) or path.name in ("test.rs", "tests.rs") or path.stem.endswith(("_test", "_tests")):
        raise gate.QualityError(f"target must be source, not a test/generated/fixture file: {value}")
    rust = value.startswith(("crates/", "src-tauri/")) and path.suffix == ".rs"
    ts = value.startswith("src/") and path.suffix in (".ts", ".tsx") and not value.endswith(".d.ts")
    if not (rust if language == "rust" else ts):
        raise gate.QualityError(f"target does not match {language}: {value}")
    return value


def execute(argv: list[str], output: Path, deadline: float, stderr: Path | None = None, env: dict[str, str] | None = None) -> int:
    """Keep native output, and bound the entire child process group."""
    remaining = deadline - time.monotonic()
    if remaining <= 0:
        raise BudgetExceeded
    print(f"Running {' '.join(argv)} (log: {output.relative_to(ROOT)})", file=sys.stderr, flush=True)
    process = None
    interrupted = False
    terminating = False

    def terminate(signum: int, frame: object) -> None:
        nonlocal interrupted
        interrupted = True
        if process is not None and not terminating:
            raise KeyboardInterrupt

    previous = signal.signal(signal.SIGTERM, terminate)
    try:
        with output.open("wb") as log, (stderr.open("wb") if stderr else nullcontext(log)) as error_log:
            process = subprocess.Popen(argv, cwd=ROOT, stdout=log, stderr=error_log, env=env, start_new_session=True)
            try:
                if interrupted:
                    raise KeyboardInterrupt
                return process.wait(timeout=remaining)
            except (subprocess.TimeoutExpired, KeyboardInterrupt) as error:
                terminating = True
                try:
                    os.killpg(process.pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
                try:
                    process.wait(timeout=30)
                except subprocess.TimeoutExpired:
                    pass
                # Kill descendants even when the group leader already exited.
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                process.wait()
                if isinstance(error, KeyboardInterrupt):
                    raise RunInterrupted from None
                raise BudgetExceeded from None
    finally:
        signal.signal(signal.SIGTERM, previous)


def versions(language: str) -> str:
    tools = gate.read_json(ROOT / "quality/tool-versions.json")["tools"]
    names = ["cargo_mutants"] if language == "rust" else ["stryker", "stryker_vitest_runner"]
    for name in names:
        tool = tools[name]
        if not tool.get("expected"):
            raise gate.QualityError(f"{name} is not pinned")
        completed = gate.run(tool["argv"], timeout=30)
        observed = completed.stdout.decode("utf-8", errors="replace").strip()
        if completed.returncode or observed != tool["expected"]:
            raise gate.QualityError(f"{name} unavailable or version mismatch: expected {tool['expected']!r}, observed {observed!r}")
    return tools[names[0]]["expected"]


def diagnostic(summary: dict, code: str, message: str, exit_code: int = 2) -> None:
    summary["status"] = "invalid" if exit_code == 3 else "inconclusive"
    summary["exit"] = max(summary["exit"], exit_code)
    summary["diagnostics"].append({"code": code, "message": message})


def rust_run(args: argparse.Namespace, targets: list[str], run: Path, deadline: float, summary: dict) -> None:
    packages = set()
    for target in targets:
        directory = (ROOT / target).parent
        while directory != ROOT:
            manifest = directory / "Cargo.toml"
            if manifest.is_file():
                package = tomllib.loads(manifest.read_text(encoding="utf-8")).get("package", {}).get("name")
                if package:
                    packages.add(package)
                    break
            directory = directory.parent
        else:
            raise gate.QualityError(f"cannot find Cargo package for {target}")
    command = ["cargo", "mutants", "--no-config", "--gitignore=true"]
    for package in sorted(packages):
        command.extend(["-p", package])
    for target in targets:
        command.extend(["-f", target])
    if args.re:
        command.extend(["--re", args.re])
    preflight = run / "preflight.json"
    rc = execute(command + ["--list", "--json"], preflight, min(deadline, time.monotonic() + 300), run / "preflight.log")
    if rc:
        raise gate.QualityError(f"cargo-mutants preflight failed (exit {rc}); see {preflight.relative_to(ROOT)}")
    mutants = json.loads(preflight.read_text(encoding="utf-8"))
    if not isinstance(mutants, list):
        raise gate.QualityError("cargo-mutants preflight did not produce a mutant list")
    counts = Counter(mutant["file"] for mutant in mutants)
    if set(counts) - set(targets):
        raise gate.QualityError("cargo-mutants preflight included files outside the explicit targets")
    empty = [target for target in targets if not counts[target]]
    if empty:
        diagnostic(summary, "no_mutants", f"No mutants generated for: {', '.join(empty)}")
        return
    summary["expected_mutants"] = len(mutants)
    command.extend(["--jobs", str(args.jobs), "--output", str(run)])
    if args.test_filter:
        command.extend(["--", "--lib", "--", args.test_filter])
    rc = execute(command, run / "tool.log", deadline)
    summary["tool_exit"] = rc
    report = run / "mutants.out/outcomes.json"
    summary["report"] = report.relative_to(ROOT).as_posix()
    if rc not in (0, 2, 3, 4):
        raise gate.QualityError(f"cargo-mutants failed (exit {rc}); see tool.log")
    if rc == 4:
        diagnostic(summary, "baseline_failed", "The unmutated Cargo baseline failed or timed out")
    if not report.is_file():
        diagnostic(summary, "report_incomplete", "cargo-mutants did not write outcomes.json")
        return
    data = gate.read_json(report)
    outcomes = [row for row in data["outcomes"] if isinstance(row["scenario"], dict) and "Mutant" in row["scenario"]]
    summary["counts"] = dict(Counter(row["summary"] for row in outcomes))
    for row in outcomes:
        mutant = row["scenario"]["Mutant"]
        if row["summary"] == "MissedMutant":
            summary["survivors"].append({"path": mutant["file"], "line": mutant["span"]["start"]["line"], "description": mutant["name"]})
    expected = Counter(mutant["name"] for mutant in mutants)
    observed = Counter(row["scenario"]["Mutant"]["name"] for row in outcomes)
    if observed != expected or data.get("total_mutants") != len(mutants) or not data.get("end_time"):
        diagnostic(summary, "report_incomplete", "Tested mutants do not match the preflight, or the run did not finish")
    baselines = [row for row in data["outcomes"] if row["scenario"] == "Baseline"]
    if not baselines or any(row["summary"] != "Success" for row in baselines):
        diagnostic(summary, "baseline_failed", "A successful unmutated baseline is required")
    elif not any(re.search(r"^running [1-9]\d* tests?$", (run / "mutants.out" / row["log_path"]).read_text(encoding="utf-8", errors="replace"), re.MULTILINE) for row in baselines):
        diagnostic(summary, "no_tests", "The Cargo baseline ran no tests; check the test filter")
    if rc == 3 or summary["counts"].get("Timeout"):
        diagnostic(summary, "mutant_timeout", "One or more mutant runs timed out")
    if any(row["summary"] not in ("CaughtMutant", "MissedMutant", "Unviable", "Timeout") for row in outcomes):
        diagnostic(summary, "report_incomplete", "Unclassified Cargo mutant outcomes")


def ts_run(args: argparse.Namespace, targets: list[str], paths: list[str], run: Path, deadline: float, summary: dict) -> None:
    config = gate.read_json(ROOT / "quality/stryker.config.json")
    config.pop("$schema", None)
    report = run / "mutation.json"
    config.update({"mutate": targets, "concurrency": args.jobs,
                   "jsonReporter": {"fileName": str(report)},
                   "htmlReporter": {"fileName": str(run / "mutation.html")}})
    effective = run / "stryker.config.json"
    effective.write_text(gate.stable_json(config), encoding="utf-8")
    summary["report"] = report.relative_to(ROOT).as_posix()
    log = run / "tool.log"
    env = {**os.environ, "COCKPIT_MUTATION_ROOT": str(ROOT),
           "COCKPIT_MUTATION_TSCONFIG": (ROOT / "tsconfig.json").read_bytes().decode("utf-8")}
    rc = execute(["node", "node_modules/@stryker-mutator/core/bin/stryker.js", "run", str(effective)], log, deadline, env=env)
    summary["tool_exit"] = rc
    if rc:
        text = log.read_text(encoding="utf-8", errors="replace")
        if "Initial test run timed out!" in text:
            diagnostic(summary, "baseline_timeout", "Stryker's initial test run timed out")
        elif any(message in text for message in ("There were failed tests in the initial test run.", "Something went wrong in the initial test run", "No tests were executed.")):
            diagnostic(summary, "baseline_failed", "Stryker's unmutated initial test run failed; see tool.log")
        else:
            raise gate.QualityError(f"Stryker failed (exit {rc}); see tool.log")
    if not report.is_file():
        diagnostic(summary, "report_incomplete", "Stryker did not write mutation.json")
        return
    files = gate.read_json(report)["files"]
    if set(files) - set(paths):
        raise gate.QualityError("Stryker report included files outside the explicit targets")
    counts = Counter()
    for path in paths:
        mutants = files.get(path, {}).get("mutants", [])
        if not mutants:
            diagnostic(summary, "no_mutants", f"No mutants generated for: {path}")
        for mutant in mutants:
            status = mutant["status"]
            counts[status] += 1
            if status in ("Survived", "NoCoverage"):
                summary["survivors"].append({"path": path, "line": mutant["location"]["start"]["line"], "description": mutant.get("description", mutant["mutatorName"])})
    summary["counts"] = dict(counts)
    if counts["Timeout"]:
        diagnostic(summary, "mutant_timeout", "One or more Stryker mutants timed out (not treated as completed evidence)")
    if set(counts) - {"Killed", "Survived", "NoCoverage", "CompileError", "Timeout"}:
        diagnostic(summary, "report_incomplete", "Stryker reported pending, ignored, runtime-error or unknown outcomes")


def main(argv: list[str] | None = None) -> int:
    started = time.monotonic()
    run = None
    summary = {"tool": None, "version": None, "status": "complete", "exit": 0,
               "targets": [], "counts": {}, "survivors": [], "report": None, "diagnostics": []}
    language = None
    try:
        parser = Parser(description=__doc__)
        subparsers = parser.add_subparsers(dest="language", required=True, parser_class=Parser)
        workers = gate.read_json(ROOT / "quality/config.json")["limits"]["workers"]
        for name in ("rust", "ts"):
            sub = subparsers.add_parser(name)
            sub.add_argument("--file", action="append", required=True, help="one literal source path (TS also accepts :START-END)")
            sub.add_argument("--jobs", type=int, default=workers)
            sub.add_argument("--budget-minutes", type=float, default=30)
            if name == "rust":
                sub.add_argument("--test-filter", help="lib unit-test name filter")
                sub.add_argument("--re", help="cargo-mutants mutant-name regex")
        # Bun may preserve the conventional argument separator.
        arguments = list(sys.argv[1:] if argv is None else argv)
        args = parser.parse_args(arguments[1:] if arguments[:1] == ["--"] else arguments)
        language = args.language
        summary["tool"] = "cargo-mutants" if language == "rust" else "stryker"
        if args.jobs < 1 or not math.isfinite(args.budget_minutes) or args.budget_minutes <= 0:
            raise gate.QualityError("--jobs and --budget-minutes must be positive and finite")
        if language == "rust" and args.test_filter is not None and (not args.test_filter.strip() or args.test_filter.startswith("-")):
            raise gate.QualityError("--test-filter must be a nonempty test name, not an option")
        targets, paths = [], []
        for target in args.file:
            path = target
            if language == "ts" and ":" in target:
                match = re.fullmatch(r"(.+):(\d+)-(\d+)", target)
                if not match or not 1 <= int(match[2]) <= int(match[3]):
                    raise gate.QualityError(f"TS range must be PATH:START-END with 1 <= START <= END: {target}")
                path = match[1]
                if int(match[3]) > len((ROOT / target_path(path, language)).read_text(encoding="utf-8").splitlines()):
                    raise gate.QualityError(f"TS range ends beyond the file: {target}")
            paths.append(target_path(path, language))
            targets.append(target)
        if len(set(paths)) != len(paths):
            raise gate.QualityError("name each source file only once per run")
        summary["targets"] = targets
        if language == "ts" and os.path.lexists(ROOT / ".stryker-tmp"):
            raise gate.QualityError("stale .stryker-tmp exists; inspect/remove it before another run")
        summary["version"] = versions(language)
        deadline = started + args.budget_minutes * 60
        stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S.%fZ")
        run = ROOT / "quality/reports/mutation" / f"{stamp}-{language}"
        run.mkdir(parents=True)
        print(f"Mutation artifacts: {run.relative_to(ROOT)}", file=sys.stderr, flush=True)
        if language == "rust":
            rust_run(args, targets, run, deadline, summary)
        else:
            ts_run(args, targets, paths, run, deadline, summary)
    except BudgetExceeded:
        diagnostic(summary, "budget_exceeded", "Execution budget or 300-second preflight limit exceeded; process group terminated")
    except RunInterrupted:
        diagnostic(summary, "interrupted", "Run interrupted; child process group terminated")
    except (gate.QualityError, OSError, ValueError, KeyError, TypeError) as error:
        diagnostic(summary, "invalid_or_infrastructure", str(error), 3)
    finally:
        if run is not None and language == "ts" and os.path.lexists(ROOT / ".stryker-tmp"):
            diagnostic(summary, "sandbox_leftover", ".stryker-tmp remains; inspect/remove it before another run")
    summary["duration_ms"] = round((time.monotonic() - started) * 1000)
    text = gate.stable_json(summary)
    if run is not None:
        (run / "summary.json").write_text(text, encoding="utf-8")
    print(text, end="")
    return summary["exit"]


if __name__ == "__main__":
    sys.exit(main())
