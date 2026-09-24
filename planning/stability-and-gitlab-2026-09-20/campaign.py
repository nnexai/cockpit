#!/usr/bin/env python3
"""Read-only campaign gate.
Usage: python3 planning/stability-and-gitlab-2026-09-20/campaign.py
       [--ledger PATH] {check|ready|complete}
--ledger may appear before or after the command and defaults to the sibling
 tasks.json. It is useful for isolated temporary-git smoke tests.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import re
import subprocess
import sys
from pathlib import Path, PurePosixPath
from typing import Any

SHA = re.compile(r"^[0-9a-fA-F]{40}$")
STATUSES = {"pending", "queued", "in_progress", "verifying", "blocked", "done", "deferred"}
ACTIVE = {"in_progress", "verifying"}
TOP_KEYS = {"schema_version", "campaign", "baseline_commit", "scope", "tasks"}
TASK_KEYS = {
    "id",
    "title",
    "required",
    "priority",
    "depends_on",
    "locks",
    "issues",
    "brief",
    "status",
    "owner",
    "started_at",
    "completed_at",
    "blockers",
    "evidence",
    "commits",
    "notes",
}


class CampaignError(Exception):
    pass


def bad(message: str) -> None:
    raise CampaignError(message)


def string(value: Any, nonempty: bool = False) -> bool:
    return isinstance(value, str) and (not nonempty or bool(value.strip()))


def strings(value: Any, field: str) -> None:
    if not isinstance(value, list) or any(not string(item, True) for item in value):
        bad(f"{field} must be an array of non-empty strings")


def timestamp(value: Any, field: str) -> None:
    if value is None:
        return
    if not string(value, True):
        bad(f"{field} must be a UTC ISO-8601 string or null")
    try:
        parsed = dt.datetime.fromisoformat(
            value[:-1] + "+00:00" if value.endswith("Z") else value
        )
    except ValueError:
        bad(f"{field} must be a UTC ISO-8601 string or null")
    if parsed.tzinfo is None or parsed.utcoffset() != dt.timedelta(0):
        bad(f"{field} must be a UTC ISO-8601 string or null")


def safe_path(root: Path, value: Any, label: str) -> Path:
    if not string(value, True) or any(
        char in value for char in ("\\", "\x00", "\n", "\r")
    ):
        bad(f"{label} must be a safe relative path")
    relative = PurePosixPath(value)
    if relative.is_absolute() or any(
        part in {"", ".", ".."} for part in relative.parts
    ):
        bad(f"{label} escapes the campaign root")
    candidate = root.joinpath(*relative.parts)
    if not candidate.exists():
        bad(f"{label} does not exist: {value}")
    current = root
    for part in relative.parts:
        current /= part
        if current.is_symlink():
            bad(f"{label} uses a symlink: {value}")
    try:
        resolved = candidate.resolve(strict=True)
        if os.path.commonpath((str(root), str(resolved))) != str(root):
            bad(f"{label} escapes the campaign root")
    except (OSError, RuntimeError, ValueError):
        bad(f"{label} cannot be resolved safely")
    return candidate


def git(repo: Path, *args: str) -> subprocess.CompletedProcess[str]:
    try:
        return subprocess.run(
            ["git", "-C", str(repo), *args], text=True, capture_output=True, check=False
        )
    except OSError as exc:
        bad(f"cannot execute git: {exc}")


def discover_repo(root: Path) -> Path:
    result = git(root, "rev-parse", "--show-toplevel")
    if result.returncode or not result.stdout.strip():
        bad("ledger is not inside a Git repository")
    try:
        repo = Path(result.stdout.strip()).resolve(strict=True)
        if os.path.commonpath((str(repo), str(root))) != str(repo):
            bad("campaign root is outside its discovered Git repository")
        return repo
    except (OSError, RuntimeError, ValueError):
        bad("Git repository path is unsafe")


def repo_path(repo: Path, path: Path) -> str:
    try:
        return path.resolve(strict=True).relative_to(repo).as_posix()
    except (OSError, RuntimeError, ValueError):
        bad(f"path is outside the Git repository: {path}")


def check_commit(repo: Path, sha: str) -> None:
    if git(repo, "cat-file", "-e", f"{sha}^{{commit}}").returncode:
        bad(f"commit does not exist: {sha}")
    if git(repo, "merge-base", "--is-ancestor", sha, "HEAD").returncode:
        bad(f"commit is not an ancestor of HEAD: {sha}")


def check_evidence(repo: Path, path: Path) -> None:
    relative = repo_path(repo, path)
    if path.stat().st_size <= 0:
        bad(f"evidence is empty: {relative}")
    listed = git(repo, "ls-tree", "-r", "--name-only", "HEAD", "--", relative)
    if listed.returncode or listed.stdout.splitlines() != [relative]:
        bad(f"evidence is not committed at HEAD: {relative}")
    kind = git(repo, "cat-file", "-t", f"HEAD:{relative}")
    size = git(repo, "cat-file", "-s", f"HEAD:{relative}")
    if (
        kind.returncode
        or kind.stdout.strip() != "blob"
        or size.returncode
        or not size.stdout.strip().isdigit()
    ):
        bad(f"evidence is not a committed file: {relative}")
    if int(size.stdout) <= 0:
        bad(f"committed evidence is empty: {relative}")
    if git(repo, "diff", "--quiet", "HEAD", "--", relative).returncode:
        bad(f"evidence differs from HEAD: {relative}")


def validate(ledger: Path) -> tuple[dict[str, Any], Path]:
    try:
        with ledger.open(encoding="utf-8") as stream:
            board = json.load(stream)
    except FileNotFoundError:
        bad(f"ledger not found: {ledger}")
    except (OSError, UnicodeError) as exc:
        bad(f"cannot read ledger: {exc}")
    except json.JSONDecodeError as exc:
        bad(f"ledger is not valid JSON: line {exc.lineno} column {exc.colno}")
    if not isinstance(board, dict) or set(board) != TOP_KEYS:
        bad("ledger metadata shape is invalid")
    version = board["schema_version"]
    if not isinstance(version, int) or isinstance(version, bool) or version != 1:
        bad("schema_version must be integer 1")
    if any(not string(board[field], True) for field in ("campaign", "scope")):
        bad("campaign and scope must be non-empty strings")
    if not string(board["baseline_commit"], True) or not SHA.fullmatch(
        board["baseline_commit"]
    ):
        bad("baseline_commit must be a full Git SHA")
    if not isinstance(board["tasks"], list) or not board["tasks"]:
        bad("tasks must be a non-empty array")
    root = ledger.parent.resolve(strict=True)
    repo = discover_repo(root)
    tasks: dict[str, dict[str, Any]] = {}
    for number, task in enumerate(board["tasks"], 1):
        if not isinstance(task, dict) or set(task) != TASK_KEYS:
            bad(f"task {number} metadata shape is invalid")
        task_id = task["id"]
        if not string(task_id, True):
            bad(f"task {number} has an invalid id")
        if task_id in tasks:
            bad(f"duplicate task id: {task_id}")
        tasks[task_id] = task
        if not string(task["title"], True) or not string(task["brief"], True):
            bad(f"task {task_id} title and brief must be non-empty strings")
        if not isinstance(task["required"], bool):
            bad(f"task {task_id} required must be boolean")
        if not isinstance(task["priority"], int) or isinstance(task["priority"], bool):
            bad(f"task {task_id} priority must be an integer")
        for field in (
            "depends_on",
            "locks",
            "issues",
            "blockers",
            "evidence",
            "commits",
            "notes",
        ):
            if not isinstance(task[field], list):
                bad(f"task {task_id}.{field} must be an array")
        for field in (
            "depends_on",
            "locks",
            "blockers",
            "evidence",
            "commits",
            "notes",
        ):
            strings(task[field], f"task {task_id}.{field}")
        if len(task["depends_on"]) != len(set(task["depends_on"])) or len(
            task["locks"]
        ) != len(set(task["locks"])):
            bad(f"task {task_id} has duplicate dependencies or locks")
        if any(
            not isinstance(issue, int) or isinstance(issue, bool)
            for issue in task["issues"]
        ):
            bad(f"task {task_id}.issues must contain integers")
        if not string(task["status"], True) or task["status"] not in STATUSES:
            bad(f"task {task_id} has invalid status: {task['status']!r}")
        if task["owner"] is not None and not string(task["owner"], True):
            bad(f"task {task_id}.owner must be a non-empty string or null")
        timestamp(task["started_at"], f"task {task_id}.started_at")
        timestamp(task["completed_at"], f"task {task_id}.completed_at")
        brief = safe_path(root, task["brief"], f"task {task_id}.brief")
        if not brief.is_file():
            bad(f"task {task_id}.brief is not a file")
    required = [task for task in board["tasks"] if task["required"]]
    if not required:
        bad("ledger must contain at least one required task")
    for task_id, task in tasks.items():
        for dependency in task["depends_on"]:
            if dependency not in tasks:
                bad(f"task {task_id} depends on unknown task: {dependency}")
    visiting: set[str] = set()
    visited: set[str] = set()

    def visit(task_id: str) -> None:
        if task_id in visiting:
            bad(f"dependency cycle includes {task_id}")
        if task_id in visited:
            return
        visiting.add(task_id)
        for dependency in tasks[task_id]["depends_on"]:
            visit(dependency)
        visiting.remove(task_id)
        visited.add(task_id)

    for task_id in tasks:
        visit(task_id)
    for task_id, task in tasks.items():
        status, owner, started = task["status"], task["owner"], task["started_at"]
        if owner is not None and started is None:
            bad(f"task {task_id} owner requires started_at")
        if status in ACTIVE and (owner is None or started is None):
            bad(f"active task {task_id} needs owner and started_at")
        if status != "done" and task["completed_at"] is not None:
            bad(f"non-done task {task_id} cannot have completed_at")
        if status == "queued" and owner is not None:
            bad(f"queued task {task_id} cannot hold an owner or locks")
        if status == "queued" and task["blockers"]:
            bad(f"queued task {task_id} cannot have external blockers")
        if status == "blocked" and not any(item.strip() for item in task["blockers"]):
            bad(f"blocked task {task_id} needs a meaningful blocker")
        if status == "done":
            if owner is None or started is None or task["completed_at"] is None:
                bad(f"done task {task_id} needs owner, started_at and completed_at")
            if task["blockers"]:
                bad(f"done task {task_id} must have no blockers")
            if not task["evidence"] or not task["commits"]:
                bad(f"done task {task_id} needs evidence and commits")
            for dependency in task["depends_on"]:
                if tasks[dependency]["status"] != "done":
                    bad(f"done task {task_id} has incomplete dependency: {dependency}")
            for sha in task["commits"]:
                if not SHA.fullmatch(sha):
                    bad(f"done task {task_id} needs full SHA commits")
                check_commit(repo, sha)
        if status in {"pending", "deferred"}:
            if (
                owner is not None
                or started is not None
                or task["completed_at"] is not None
            ):
                bad(f"{status} task {task_id} has completion/ownership metadata")
            if task["blockers"] or task["evidence"] or task["commits"]:
                bad(f"{status} task {task_id} has recorded execution metadata")
        if not task["required"] and status != "deferred":
            bad(f"non-required task {task_id} must remain deferred")
        if status == "deferred" and task["required"]:
            bad(f"required task {task_id} cannot be deferred")
        if status == "done" and not task["required"]:
            bad(f"deferred-scope task {task_id} cannot be done")
        for evidence in task["evidence"]:
            path = safe_path(root, evidence, f"task {task_id}.evidence")
            if (
                len(PurePosixPath(evidence).parts) < 2
                or PurePosixPath(evidence).parts[0] != "runs"
            ):
                bad(f"task {task_id}.evidence must live under runs/")
            if not path.is_file() or path.is_symlink():
                bad(f"task {task_id}.evidence is not a regular file")
            if status == "done":
                check_evidence(repo, path)
        for sha in task["commits"]:
            if not SHA.fullmatch(sha):
                bad(f"task {task_id} has a non-full commit SHA")

    def deferred_dependency(task_id: str, seen: set[str]) -> str | None:
        for dependency in tasks[task_id]["depends_on"]:
            if dependency in seen:
                continue
            seen.add(dependency)
            if not tasks[dependency]["required"]:
                return dependency
            found = deferred_dependency(dependency, seen)
            if found:
                return found
        return None

    for task_id, task in tasks.items():
        if task["required"]:
            dependency = deferred_dependency(task_id, set())
            if dependency:
                bad(
                    f"required task {task_id} depends on non-required task: {dependency}"
                )
    held: dict[str, str] = {}
    for task_id, task in tasks.items():
        if task["status"] not in ACTIVE and not (
            task["status"] == "blocked" and task["owner"] is not None
        ):
            continue
        for lock in task["locks"]:
            if lock in held:
                bad(f"lock {lock!r} held by both {held[lock]} and {task_id}")
            held[lock] = task_id
    return board, repo


def ready(board: dict[str, Any]) -> dict[str, Any]:
    tasks = {task["id"]: task for task in board["tasks"]}
    held: dict[str, str] = {}
    for task in board["tasks"]:
        if task["status"] in ACTIVE or (
            task["status"] == "blocked" and task["owner"] is not None
        ):
            for lock in task["locks"]:
                held[lock] = task["id"]
    candidates, waiting = [], []
    for task in board["tasks"]:
        if task["status"] not in {"pending", "queued"}:
            continue
        waiting_on = [
            item for item in task["depends_on"] if tasks[item]["status"] != "done"
        ]
        lock_wait = sorted({held[lock] for lock in task["locks"] if lock in held})
        if not waiting_on and not lock_wait:
            candidates.append(
                {"id": task["id"], "priority": task["priority"], "locks": task["locks"]}
            )
        else:
            waiting.append(
                {"id": task["id"], "waiting_on": waiting_on, "locks_held_by": lock_wait}
            )
    candidates.sort(key=lambda item: (item["priority"], item["id"]))
    waiting.sort(key=lambda item: (tasks[item["id"]]["priority"], item["id"]))
    active = sorted(task["id"] for task in board["tasks"] if task["status"] in ACTIVE)
    blocked = sorted(
        (
            {"id": task["id"], "blockers": task["blockers"]}
            for task in board["tasks"]
            if task["status"] == "blocked"
        ),
        key=lambda item: item["id"],
    )
    return {
        "candidates": candidates,
        "active": active,
        "blocked": blocked,
        "waiting": waiting,
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Read-only campaign ledger gate")
    parser.add_argument(
        "--ledger", metavar="PATH", help="ledger path (default: sibling tasks.json)"
    )
    parser.add_argument("command", choices=("check", "ready", "complete"))
    args = parser.parse_args(argv)
    ledger = (
        Path(args.ledger).expanduser()
        if args.ledger
        else Path(__file__).resolve().with_name("tasks.json")
    )
    try:
        ledger = ledger.resolve(strict=True)
        board, _repo = validate(ledger)
        if args.command == "check":
            print(json.dumps({"ok": True, "tasks": len(board["tasks"])}))
        elif args.command == "ready":
            print(json.dumps(ready(board), sort_keys=True))
        else:
            incomplete = sorted(
                task["id"]
                for task in board["tasks"]
                if task["required"] and task["status"] != "done"
            )
            if incomplete:
                bad("required tasks incomplete: " + ", ".join(incomplete))
            print(
                json.dumps(
                    {
                        "complete": True,
                        "required_tasks": sum(
                            task["required"] for task in board["tasks"]
                        ),
                    }
                )
            )
        return 0
    except CampaignError as exc:
        print(f"campaign: {exc}", file=sys.stderr)
        return 1
    except (OSError, ValueError) as exc:
        print(f"campaign: cannot access ledger: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
