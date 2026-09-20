#!/usr/bin/env python3
"""Fail-closed Herdr invocation plans for recorded disposable resources.

This module never starts a process.  Callers must use :func:`prepare_subprocess`
with one of the narrowly authorized operations below; it returns an executable,
argument vector, and isolated environment only after every ownership, filesystem,
and executable-identity check has passed.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import stat
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Mapping, Sequence


class ResourceGuardError(ValueError):
    """A ledger, target, or operation is not safe to use."""


_NAME_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_.-]*$")
_SHA256_RE = re.compile(r"^[0-9a-fA-F]{64}$")
_DISPATCHABLE_STATUSES = frozenset({"planned", "ready", "running"})
_TARGET_SELECTOR_FLAGS = frozenset(
    {
        "--session",
        "--socket",
        "--socket-path",
        "--config",
        "--config-path",
        "--herdr",
        "--remote",
        "--remote-keybindings",
        "--no-session",
        "--handoff",
    }
)
_READ_ONLY_COMMANDS = frozenset(
    {
        ("status", "server", "--json"),
        ("session", "list", "--json"),
        ("api", "schema", "--json"),
        ("api", "snapshot"),
    }
)


@dataclass(frozen=True)
class SessionResource:
    name: str
    owner: str
    xdg_config_home: str
    xdg_state_home: str
    config_path: str
    socket_path: str
    status: str


@dataclass(frozen=True)
class ExecutableIdentity:
    path: str
    sha256: str
    version: str | None = None
    protocol: int | None = None
    schema_version: int | None = None


@dataclass(frozen=True)
class ResourceLedger:
    run_id: str
    protected_session: str
    resource_root: str
    allowed_sessions: tuple[SessionResource, ...]
    executable: ExecutableIdentity
    protected_config_path: str
    protected_socket_path: str
    forbidden_executable_paths: tuple[str, ...] = ()


@dataclass(frozen=True)
class GuardedTarget:
    """The one validated session and executable for a future subprocess."""

    run_id: str
    resource_root: str
    session: SessionResource
    executable: ExecutableIdentity

    @property
    def config_path(self) -> str:
        return self.session.config_path

    @property
    def socket_path(self) -> str:
        return self.session.socket_path


def _require_text(value: object, field: str) -> str:
    if not isinstance(value, str) or not value or "\x00" in value:
        raise ResourceGuardError(f"{field} must be a non-empty string")
    return value


def _validate_name(value: object, field: str) -> str:
    name = _require_text(value, field)
    if name != name.strip() or any(character.isspace() for character in name):
        raise ResourceGuardError(f"{field} must not contain whitespace")
    if name in {".", ".."} or ".." in name:
        raise ResourceGuardError(f"{field} must not contain traversal")
    if not _NAME_RE.fullmatch(name):
        raise ResourceGuardError(f"{field} is malformed")
    return name


def _validate_path(value: object, field: str) -> str:
    path = _require_text(value, field)
    if not os.path.isabs(path) or path != os.path.normpath(path):
        raise ResourceGuardError(f"{field} must be a normalized absolute path")
    if any(part in {".", ".."} for part in Path(path).parts):
        raise ResourceGuardError(f"{field} must not contain traversal")
    return path


def _optional_path(raw: Mapping[str, object], key: str) -> str | None:
    value = raw.get(key)
    return None if value is None else _validate_path(value, key)


def _path_chain(path: Path) -> tuple[Path, ...]:
    chain: list[Path] = []
    current = path
    while current != current.parent:
        chain.append(current)
        current = current.parent
    chain.append(current)
    return tuple(reversed(chain))


def _reject_symlink_path(path: Path, field: str) -> None:
    for component in _path_chain(path):
        if component.is_symlink():
            raise ResourceGuardError(f"{field} has a symlink ancestor or leaf")


def _validate_resource_root(root: str) -> None:
    root_path = Path(root)
    _reject_symlink_path(root_path, "resource_root")
    try:
        identity = os.lstat(root_path)
    except OSError as error:
        raise ResourceGuardError(f"resource_root is unavailable: {error}") from error
    if not stat.S_ISDIR(identity.st_mode):
        raise ResourceGuardError("resource_root must be an existing directory")
    if identity.st_uid != os.geteuid() or identity.st_mode & 0o022:
        raise ResourceGuardError("resource_root must be owned by this user and not shared-writable")


def _validate_under_root(path: str, root: str, field: str) -> None:
    root_path = Path(root)
    target_path = Path(path)
    _validate_resource_root(root)
    try:
        relative = target_path.relative_to(root_path)
    except ValueError as error:
        raise ResourceGuardError(f"{field} must be beneath resource_root") from error
    if not relative.parts:
        raise ResourceGuardError(f"{field} must be beneath resource_root")
    _reject_symlink_path(target_path, field)


def _reject_same_regular_file(candidate: str, protected: str, field: str) -> None:
    """Reject existing hard-link aliases without requiring planned leaves to exist."""

    candidate_path = Path(candidate)
    protected_path = Path(protected)
    if not candidate_path.exists() or not protected_path.exists():
        return
    try:
        candidate_stat = os.stat(candidate_path, follow_symlinks=False)
        protected_stat = os.stat(protected_path, follow_symlinks=False)
    except OSError as error:
        raise ResourceGuardError(f"unable to inspect {field} identity: {error}") from error
    if (
        stat.S_ISREG(candidate_stat.st_mode)
        and stat.S_ISREG(protected_stat.st_mode)
        and (candidate_stat.st_dev, candidate_stat.st_ino)
        == (protected_stat.st_dev, protected_stat.st_ino)
    ):
        raise ResourceGuardError(f"{field} aliases a protected filesystem object")


def _parse_session(raw: object, index: int) -> SessionResource:
    if not isinstance(raw, dict):
        raise ResourceGuardError(f"allowed_sessions[{index}] must be an object")
    required = (
        "name",
        "owner",
        "xdg_config_home",
        "xdg_state_home",
        "config_path",
        "socket_path",
        "status",
    )
    missing = [key for key in required if key not in raw]
    if missing:
        raise ResourceGuardError(
            f"allowed_sessions[{index}] missing {', '.join(missing)}"
        )
    name = _validate_name(raw["name"], f"allowed_sessions[{index}].name")
    owner = _validate_name(raw["owner"], f"allowed_sessions[{index}].owner")
    xdg_config_home = _validate_path(
        raw["xdg_config_home"], f"allowed_sessions[{index}].xdg_config_home"
    )
    xdg_state_home = _validate_path(
        raw["xdg_state_home"], f"allowed_sessions[{index}].xdg_state_home"
    )
    config_path = _validate_path(
        raw["config_path"], f"allowed_sessions[{index}].config_path"
    )
    socket_path = _validate_path(
        raw["socket_path"], f"allowed_sessions[{index}].socket_path"
    )
    if len({xdg_config_home, xdg_state_home, config_path, socket_path}) != 4:
        raise ResourceGuardError(f"allowed_sessions[{index}] resource paths collide")
    status = _require_text(raw["status"], f"allowed_sessions[{index}].status").lower()
    if status != status.strip() or any(character.isspace() for character in status):
        raise ResourceGuardError(f"allowed_sessions[{index}].status is malformed")
    return SessionResource(
        name,
        owner,
        xdg_config_home,
        xdg_state_home,
        config_path,
        socket_path,
        status,
    )


def _parse_executable(raw: object) -> ExecutableIdentity:
    if not isinstance(raw, dict):
        raise ResourceGuardError("executables.selected must be an object")
    path = _validate_path(raw.get("path"), "executables.selected.path")
    digest = _require_text(raw.get("sha256"), "executables.selected.sha256").lower()
    if not _SHA256_RE.fullmatch(digest):
        raise ResourceGuardError("executables.selected.sha256 must be a SHA-256 digest")
    version = raw.get("version")
    if version is not None:
        version = _require_text(version, "executables.selected.version")
    protocol = raw.get("protocol")
    if protocol is not None and (isinstance(protocol, bool) or not isinstance(protocol, int)):
        raise ResourceGuardError("executables.selected.protocol must be an integer")
    schema_version = raw.get("schema_version")
    if schema_version is not None and (
        isinstance(schema_version, bool) or not isinstance(schema_version, int)
    ):
        raise ResourceGuardError("executables.selected.schema_version must be an integer")
    return ExecutableIdentity(path, digest, version, protocol, schema_version)


def _require_root_owner_receipt(raw: Mapping[str, object], root: str, run_id: str) -> None:
    resources = raw.get("resources")
    if not isinstance(resources, list):
        raise ResourceGuardError("ledger must record resources ownership receipts")
    for item in resources:
        if isinstance(item, dict) and item.get("path") == root and item.get("owner") == run_id:
            return
    raise ResourceGuardError("resource_root lacks a run-owned ownership receipt")


def load_ledger(path: str | os.PathLike[str]) -> ResourceLedger:
    """Load and structurally validate a JSON resource ledger without dispatching."""

    ledger_path = Path(path)
    try:
        with ledger_path.open("r", encoding="utf-8") as stream:
            raw = json.load(stream)
    except (OSError, json.JSONDecodeError) as error:
        raise ResourceGuardError(f"unable to load ledger: {error}") from error
    if not isinstance(raw, dict):
        raise ResourceGuardError("ledger must be a JSON object")

    run_id = _validate_name(raw.get("run_id"), "run_id")
    protected_session = _validate_name(raw.get("protected_session"), "protected_session")
    if protected_session != "default":
        raise ResourceGuardError("protected_session must be default")
    resource_root = _validate_path(raw.get("resource_root"), "resource_root")
    _validate_resource_root(resource_root)
    _require_root_owner_receipt(raw, resource_root, run_id)

    raw_sessions = raw.get("allowed_sessions")
    if not isinstance(raw_sessions, list):
        raise ResourceGuardError("allowed_sessions must be an array")
    sessions = tuple(_parse_session(item, index) for index, item in enumerate(raw_sessions))
    names = [session.name for session in sessions]
    if len(names) != len(set(names)):
        raise ResourceGuardError("allowed_sessions contains duplicate names")
    if protected_session in names:
        raise ResourceGuardError("allowed_sessions must not record the protected default")
    paths = [
        path
        for session in sessions
        for path in (
            session.xdg_config_home,
            session.xdg_state_home,
            session.config_path,
            session.socket_path,
        )
    ]
    if len(paths) != len(set(paths)):
        raise ResourceGuardError("allowed_sessions contains duplicate resource paths")
    for index, session in enumerate(sessions):
        for field, endpoint in (
            ("xdg_config_home", session.xdg_config_home),
            ("xdg_state_home", session.xdg_state_home),
            ("config_path", session.config_path),
            ("socket_path", session.socket_path),
        ):
            _validate_under_root(endpoint, resource_root, f"allowed_sessions[{index}].{field}")

    executables = raw.get("executables")
    if not isinstance(executables, dict):
        raise ResourceGuardError("executables must be an object")
    executable = _parse_executable(executables.get("selected"))
    forbidden_raw = raw.get("forbidden_executable_paths", [])
    if not isinstance(forbidden_raw, list):
        raise ResourceGuardError("forbidden_executable_paths must be an array")
    forbidden_paths = [
        _validate_path(value, f"forbidden_executable_paths[{index}]")
        for index, value in enumerate(forbidden_raw)
    ]
    for key, value in executables.items():
        if key != "selected" and isinstance(value, dict) and value.get("permitted") is False:
            if "path" not in value:
                raise ResourceGuardError(f"executables.{key} forbidden identity lacks path")
            forbidden_paths.append(_validate_path(value["path"], f"executables.{key}.path"))
    if len(forbidden_paths) != len(set(forbidden_paths)):
        raise ResourceGuardError("forbidden executable paths contain duplicates")

    protected = raw.get("protected", {})
    if protected is None:
        protected = {}
    if not isinstance(protected, dict):
        raise ResourceGuardError("protected must be an object")
    protected_config = _optional_path(raw, "protected_config_path") or _optional_path(
        protected, "config_path"
    )
    protected_socket = _optional_path(raw, "protected_socket_path") or _optional_path(
        protected, "socket_path"
    )
    if protected_config is None or protected_socket is None:
        raise ResourceGuardError(
            "ledger must record protected_config_path and protected_socket_path"
        )
    if protected_config == protected_socket:
        raise ResourceGuardError("protected config/socket paths collide")

    return ResourceLedger(
        run_id,
        protected_session,
        resource_root,
        sessions,
        executable,
        protected_config,
        protected_socket,
        tuple(forbidden_paths),
    )


def _validate_target_paths(ledger: ResourceLedger, session: SessionResource) -> None:
    for field, endpoint in (
        ("xdg_config_home", session.xdg_config_home),
        ("xdg_state_home", session.xdg_state_home),
        ("config_path", session.config_path),
        ("socket_path", session.socket_path),
    ):
        _validate_under_root(endpoint, ledger.resource_root, field)
    _reject_symlink_path(Path(ledger.protected_config_path), "protected_config_path")
    _reject_symlink_path(Path(ledger.protected_socket_path), "protected_socket_path")
    if session.config_path == ledger.protected_config_path:
        raise ResourceGuardError("session config targets the protected default")
    if session.socket_path == ledger.protected_socket_path:
        raise ResourceGuardError("session socket targets the protected default")
    _reject_same_regular_file(
        session.config_path, ledger.protected_config_path, "session config"
    )
    _reject_same_regular_file(
        session.socket_path, ledger.protected_socket_path, "session socket"
    )
    expected_config = Path(session.xdg_config_home) / "herdr" / "config.toml"
    expected_socket = (
        Path(session.xdg_config_home) / "herdr" / "sessions" / session.name / "herdr.sock"
    )
    if session.config_path != str(expected_config):
        raise ResourceGuardError("session config does not match its XDG configuration home")
    if session.socket_path != str(expected_socket):
        raise ResourceGuardError("session socket does not match its explicit session")


def validate_target(
    ledger: ResourceLedger,
    run_id: str,
    session_name: str,
    *,
    cleanup: bool = False,
) -> GuardedTarget:
    """Revalidate explicit run/session ownership before every dispatch."""

    requested_run = _validate_name(run_id, "run_id")
    requested_session = _validate_name(session_name, "session")
    if requested_run != ledger.run_id:
        raise ResourceGuardError("run_id does not own this ledger")
    if requested_session == ledger.protected_session:
        raise ResourceGuardError("the protected default session is not a test target")
    matches = [item for item in ledger.allowed_sessions if item.name == requested_session]
    if len(matches) != 1:
        raise ResourceGuardError("session is not recorded uniquely in allowed_sessions")
    session = matches[0]
    if session.owner != ledger.run_id:
        raise ResourceGuardError("session is foreign to this run")
    if session.status not in _DISPATCHABLE_STATUSES:
        raise ResourceGuardError("session status is not dispatchable")
    _validate_target_paths(ledger, session)
    if ledger.executable.path in ledger.forbidden_executable_paths:
        raise ResourceGuardError("selected executable is forbidden by the ledger")
    for forbidden_path in ledger.forbidden_executable_paths:
        _reject_same_regular_file(
            ledger.executable.path, forbidden_path, "selected executable"
        )
    return GuardedTarget(
        ledger.run_id, ledger.resource_root, session, ledger.executable
    )


def validate_recorded_executable(target: GuardedTarget) -> None:
    """Check that the selected executable remains the recorded stable identity."""

    path = Path(target.executable.path)
    _reject_symlink_path(path, "selected executable")
    try:
        mode = os.lstat(path).st_mode
    except OSError as error:
        raise ResourceGuardError(f"selected executable is unavailable: {error}") from error
    if not stat.S_ISREG(mode) or not os.access(path, os.X_OK):
        raise ResourceGuardError("selected executable is not an executable regular file")
    digest = hashlib.sha256()
    try:
        with path.open("rb") as stream:
            for chunk in iter(lambda: stream.read(1024 * 1024), b""):
                digest.update(chunk)
    except OSError as error:
        raise ResourceGuardError(f"unable to hash selected executable: {error}") from error
    if digest.hexdigest() != target.executable.sha256:
        raise ResourceGuardError("selected executable hash drifted from the ledger")


def _validate_command_parts(command: Sequence[str]) -> tuple[str, ...]:
    if isinstance(command, (str, bytes)):
        raise ResourceGuardError("subprocess command must be an argv sequence")
    argv = tuple(command)
    if not argv or any(not isinstance(part, str) or not part or "\x00" in part for part in argv):
        raise ResourceGuardError("subprocess argv contains an invalid argument")
    for argument in argv:
        if argument == "--" or argument in _TARGET_SELECTOR_FLAGS or any(
            argument.startswith(flag + "=") for flag in _TARGET_SELECTOR_FLAGS
        ):
            raise ResourceGuardError("subprocess argv cannot override or escape the target")
    return argv


def _validate_runtime_path(target: GuardedTarget, value: str, field: str) -> None:
    path = _validate_path(value, field)
    _validate_under_root(path, target.resource_root, field)


def _validate_fixture_operation(target: GuardedTarget, argv: tuple[str, ...]) -> bool:
    """Authorize only resource-scoped fixture setup/teardown commands.

    Every filesystem argument is required to be below the session's recorded
    state home.  Commands that can inject arbitrary process arguments, mutate
    configuration, or address another Herdr session are deliberately absent.
    """

    if target.session.status != "running" or len(argv) < 2:
        return False
    family, operation, *arguments = argv
    if (family, operation) in {
        ("workspace", "list"),
        ("pane", "list"),
        ("agent", "list"),
    }:
        return not arguments
    if (family, operation) in {
        ("workspace", "get"),
        ("workspace", "close"),
        ("tab", "get"),
        ("tab", "close"),
        ("pane", "get"),
        ("agent", "get"),
    }:
        return len(arguments) == 1 and _is_runtime_identifier(arguments[0])
    if (family, operation) == ("workspace", "create"):
        return _validate_create_at_runtime_root(target, arguments, {"--cwd", "--label", "--focus", "--no-focus"})
    if (family, operation) == ("tab", "create"):
        return _validate_create_at_runtime_root(target, arguments, {"--cwd", "--label", "--focus", "--no-focus"})
    if (family, operation) in {("worktree", "list"), ("worktree", "create"), ("worktree", "open")}:
        return _validate_worktree_operation(target, operation, arguments)
    if (family, operation) == ("worktree", "remove"):
        return arguments[:1] == ["--workspace"] and len(arguments) == 2 and _is_runtime_identifier(arguments[1])
    if (family, operation) == ("pane", "close"):
        return len(arguments) == 1 and _is_runtime_identifier(arguments[0])
    if (family, operation) == ("agent", "read"):
        return len(arguments) == 3 and _is_runtime_identifier(arguments[0]) and arguments[1:] in (["--source", "screen"], ["--source", "scrollback"])
    if (family, operation) == ("agent", "wait"):
        return len(arguments) == 1 and _is_runtime_identifier(arguments[0])
    return False


def _is_runtime_identifier(value: str) -> bool:
    return bool(value) and "\x00" not in value and not value.startswith("-") and not any(
        character.isspace() for character in value
    )


def _validate_create_at_runtime_root(
    target: GuardedTarget, arguments: list[str], allowed_flags: set[str]
) -> bool:
    index = 0
    saw_cwd = False
    while index < len(arguments):
        flag = arguments[index]
        if flag not in allowed_flags:
            return False
        if flag in {"--focus", "--no-focus"}:
            index += 1
            continue
        if index + 1 >= len(arguments) or not arguments[index + 1]:
            return False
        value = arguments[index + 1]
        if flag == "--cwd":
            _validate_runtime_path(target, value, "fixture cwd")
            saw_cwd = True
        elif "\x00" in value:
            return False
        index += 2
    return saw_cwd


def _validate_worktree_operation(
    target: GuardedTarget, operation: str, arguments: list[str]
) -> bool:
    allowed = {
        "list": {"--cwd", "--json"},
        "create": {"--cwd", "--path", "--branch", "--base", "--label", "--focus", "--no-focus", "--json"},
        "open": {"--cwd", "--path", "--branch", "--label", "--focus", "--no-focus", "--json"},
    }[operation]
    index = 0
    saw_cwd = False
    saw_path = False
    while index < len(arguments):
        flag = arguments[index]
        if flag not in allowed:
            return False
        if flag in {"--focus", "--no-focus", "--json"}:
            index += 1
            continue
        if index + 1 >= len(arguments) or not arguments[index + 1]:
            return False
        value = arguments[index + 1]
        if flag in {"--cwd", "--path"}:
            _validate_runtime_path(target, value, f"worktree {flag[2:]}")
            saw_cwd |= flag == "--cwd"
            saw_path |= flag == "--path"
        elif "\x00" in value:
            return False
        index += 2
    if operation == "list":
        return saw_cwd
    return saw_cwd and (operation == "create" or saw_path)


def build_subprocess_argv(
    target: GuardedTarget,
    command: Sequence[str],
    *,
    cleanup: bool = False,
) -> list[str]:
    """Return only a fixed, audited Herdr invocation for ``target``.

    Read-only inventory calls, exact planned-session startup, narrowly scoped
    fixture resource commands, and the exact owned cleanup operation are the
    complete vocabulary.  Unknown operations fail closed.
    """

    argv = _validate_command_parts(command)
    cleanup_commands = {
        ("session", "stop", target.session.name),
        ("session", "stop", target.session.name, "--json"),
    }
    if cleanup:
        if argv not in cleanup_commands:
            raise ResourceGuardError("cleanup permits only exact session stop for the target")
    elif argv == ("tui",):
        if target.session.status != "running":
            raise ResourceGuardError("TUI attachment requires a running run-owned session")
        return [target.executable.path, "--session", target.session.name]
    elif argv in _READ_ONLY_COMMANDS:
        pass
    elif argv == ("server",):
        if target.session.status != "planned":
            raise ResourceGuardError("server startup requires a planned run-owned session")
    elif _validate_fixture_operation(target, argv):
        pass
    else:
        raise ResourceGuardError("subprocess operation is not authorized")
    return [target.executable.path, "--session", target.session.name, *argv]


def build_subprocess_environment(
    target: GuardedTarget,
    base_environment: Mapping[str, str] | None = None,
) -> dict[str, str]:
    """Replace inherited home, Herdr selectors and XDG roots with owned paths."""

    source = os.environ if base_environment is None else base_environment
    environment = {
        key: value for key, value in source.items() if not key.startswith("HERDR_")
    }
    home = os.path.join(target.resource_root, "home")
    _validate_runtime_path(target, home, "process HOME")
    environment["HOME"] = home
    environment["XDG_CONFIG_HOME"] = target.session.xdg_config_home
    environment["XDG_STATE_HOME"] = target.session.xdg_state_home
    environment["HERDR_CONFIG_PATH"] = target.session.config_path
    environment["HERDR_SOCKET_PATH"] = target.session.socket_path
    return environment


def prepare_subprocess(
    ledger: ResourceLedger,
    run_id: str,
    session_name: str,
    command: Sequence[str],
    *,
    cleanup: bool = False,
    base_environment: Mapping[str, str] | None = None,
) -> tuple[GuardedTarget, list[str], dict[str, str]]:
    """Return a complete invocation plan only after every guard check passes."""

    target = validate_target(ledger, run_id, session_name, cleanup=cleanup)
    validate_recorded_executable(target)
    environment = build_subprocess_environment(target, base_environment)
    return (
        target,
        build_subprocess_argv(target, command, cleanup=cleanup),
        environment,
    )


def _report_path(path: str) -> str:
    home = str(Path.home())
    if path == home:
        return "$HOME"
    prefix = home + os.sep
    return "$HOME/" + path[len(prefix) :] if path.startswith(prefix) else path


def _report(target: GuardedTarget, cleanup: bool) -> dict[str, object]:
    executable = target.executable
    identity: dict[str, object] = {"path": _report_path(executable.path), "sha256": executable.sha256}
    if executable.version is not None:
        identity["version"] = executable.version
    if executable.protocol is not None:
        identity["protocol"] = executable.protocol
    if executable.schema_version is not None:
        identity["schema_version"] = executable.schema_version
    return {
        "status": "ok",
        "cleanup": cleanup,
        "run_id": target.run_id,
        "session": {"name": target.session.name, "owner": target.session.owner, "status": target.session.status},
        "effective": {
            "xdg_config_home": _report_path(target.session.xdg_config_home),
            "xdg_state_home": _report_path(target.session.xdg_state_home),
            "config_path": _report_path(target.config_path),
            "socket_path": _report_path(target.socket_path),
            "executable": identity,
        },
    }


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--ledger", required=True, help="run-owned resources.json path")
    parser.add_argument("--run-id", required=True, help="exact run identifier")
    parser.add_argument("--session", required=True, help="exact disposable session name")
    parser.add_argument("--cleanup", action="store_true", help="validate an owned cleanup target")
    args = parser.parse_args(argv)
    try:
        ledger = load_ledger(args.ledger)
        target = validate_target(ledger, args.run_id, args.session, cleanup=args.cleanup)
        validate_recorded_executable(target)
    except ResourceGuardError as error:
        print(f"resource_guard: rejected: {error}", file=sys.stderr)
        return 2
    print(json.dumps(_report(target, args.cleanup), sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
