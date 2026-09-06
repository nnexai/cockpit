#!/usr/bin/env python3
"""Bounded, read-only BOOT-01/G00B startup inventory collector.

All targets, evidence, and output files are explicitly bound to the resource
ledger.  The collector never starts a service, creates a resource, or sends
terminal input.  A JSON receipt is provenance, not independent proof that a
renderer behaved correctly; rendered evidence must carry a Main-observed
receipt bound to its bytes and identities.

Exit codes: 0 PASS, 1 FAIL, 2 INCONCLUSIVE, 64 invalid usage.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import selectors
import stat
import struct
import subprocess
import sys
import tempfile
import time
import zlib
from typing import Any, Mapping, Sequence
from urllib.error import HTTPError, URLError
from urllib.parse import quote, urlsplit, urlunsplit
from urllib.request import Request, urlopen

try:
    from .resource_guard import ResourceGuardError, load_ledger, prepare_subprocess
except ImportError:
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    from resource_guard import ResourceGuardError, load_ledger, prepare_subprocess  # type: ignore[no-redef]

EXIT_PASS, EXIT_FAIL, EXIT_INCONCLUSIVE, EXIT_USAGE = 0, 1, 2, 64
COMMAND_TIMEOUT_SECONDS = 5.0
MAX_COMMAND_STDOUT = 1024 * 1024
MAX_COMMAND_STDERR = 16 * 1024
MAX_HTTP_BODY = 1024 * 1024
MAX_EVIDENCE_BODY = 1024 * 1024
MAX_ARTIFACT_BYTES = 512 * 1024 * 1024
MAX_SOURCE_FILE_BYTES = 16 * 1024 * 1024
MAX_SOURCE_FILES = 4096
MAX_SOURCE_BYTES = 256 * 1024 * 1024
_SHA256_RE = re.compile(r"^[0-9a-f]{64}$")
_READ_ONLY_OPERATIONS: tuple[tuple[str, ...], ...] = (
    ("status", "server", "--json"),
    ("session", "list", "--json"),
    ("api", "schema", "--json"),
    ("api", "snapshot"),
)
_SOURCE_SUFFIXES = {".rs", ".py", ".ts", ".tsx", ".css", ".html", ".json"}
_SOURCE_ROOTS = ("crates", "src", "src-tauri", "scripts/verify")
_EXTENSIONLESS_WRAPPERS = {"scripts/verify/startup-inventory", "scripts/verify/terminal-temporal"}
_CONFIG_IDENTITIES = (
    "Cargo.toml", "Cargo.lock", "package.json", "bun.lock", "tsconfig.json", "vite.config.ts",
    "src-tauri/Cargo.toml", "src-tauri/tauri.conf.json", "src-tauri/capabilities/default.json",
    "AGENTS.md", "CLAUDE.md",
)
_REQUIRED_CAPABILITIES = ("mouse_input", "click_focus", "app_mode_pointer", "wheel_scroll", "graphics")
_BACKEND_KEYS = ("version", "protocol", "schema_version", "schema_sha256", "effective_config_sha256")


def _redact_path(path: str | os.PathLike[str]) -> str:
    value = str(path)
    home = os.path.expanduser("~")
    if value == home:
        return "$HOME"
    if value.startswith(home + os.sep):
        return "$HOME/" + value[len(home) + 1 :]
    if value == "/tmp":
        return "$RUNTIME"
    if value.startswith("/tmp/"):
        return "$RUNTIME/" + value[5:]
    return "$PATH" if os.path.isabs(value) else value


def _sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _sha256_file(path: Path, limit: int = MAX_ARTIFACT_BYTES) -> str | None:
    try:
        if not path.is_file() or path.stat().st_size > limit:
            return None
        digest = hashlib.sha256()
        total = 0
        with path.open("rb") as stream:
            for chunk in iter(lambda: stream.read(1024 * 1024), b""):
                total += len(chunk)
                if total > limit:
                    return None
                digest.update(chunk)
        return digest.hexdigest()
    except OSError:
        return None


def _read_file_bounded(path: Path, limit: int) -> bytes | None:
    try:
        with path.open("rb") as stream:
            value = stream.read(limit + 1)
    except OSError:
        return None
    return value if len(value) <= limit else None


def _json_bytes(data: bytes) -> Any | None:
    try:
        return json.loads(data.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError, RecursionError):
        return None


def _unwrap(document: Any) -> Mapping[str, Any] | None:
    if not isinstance(document, Mapping):
        return None
    nested = document.get("result")
    return nested if isinstance(nested, Mapping) else document


def _bounded_process(argv: Sequence[str], environment: Mapping[str, str], stdout_limit: int = MAX_COMMAND_STDOUT) -> dict[str, Any]:
    """Run a fixed command without retaining unbounded stdout or stderr."""
    started = time.monotonic()
    result: dict[str, Any] = {"argv": list(argv)}
    try:
        process = subprocess.Popen(list(argv), env=dict(environment), stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    except OSError as error:
        return {**result, "status": "unavailable", "exit_status": None, "duration_ms": round((time.monotonic() - started) * 1000), "error": type(error).__name__}
    assert process.stdout is not None and process.stderr is not None
    selector = selectors.DefaultSelector()
    streams = {process.stdout.fileno(): (process.stdout, bytearray(), stdout_limit), process.stderr.fileno(): (process.stderr, bytearray(), MAX_COMMAND_STDERR)}
    for descriptor, (stream, _buffer, _limit) in streams.items():
        os.set_blocking(descriptor, False)
        selector.register(stream, selectors.EVENT_READ)
    overflow = False
    timed_out = False
    while selector.get_map():
        remaining = COMMAND_TIMEOUT_SECONDS - (time.monotonic() - started)
        if remaining <= 0:
            timed_out = True
            break
        for key, _event in selector.select(remaining):
            descriptor = key.fileobj.fileno()
            stream, buffer, limit = streams[descriptor]
            try:
                chunk = os.read(descriptor, min(65536, limit + 1 - len(buffer)))
            except BlockingIOError:
                continue
            if not chunk:
                selector.unregister(stream)
                continue
            buffer.extend(chunk)
            if len(buffer) > limit:
                overflow = True
                break
        if overflow:
            break
    selector.close()
    if timed_out or overflow:
        process.kill()
    try:
        exit_status = process.wait(timeout=1)
    except subprocess.TimeoutExpired:
        process.kill()
        exit_status = process.wait()
    stdout = bytes(streams[process.stdout.fileno()][1])
    status = "timeout" if timed_out else "output_limit" if overflow else "ok" if exit_status == 0 else "failed"
    result.update(status=status, exit_status=exit_status, duration_ms=round((time.monotonic() - started) * 1000))
    if status == "ok":
        result["_stdout"] = stdout
    return result


def _run_prepared(operation: tuple[str, ...], argv: Sequence[str], environment: Mapping[str, str]) -> dict[str, Any]:
    result = _bounded_process(argv, environment)
    result["operation"] = list(operation)
    return result


def _public_command(result: Mapping[str, Any]) -> dict[str, Any]:
    public = {key: value for key, value in result.items() if key != "_stdout"}
    argv = public.get("argv")
    if isinstance(argv, list):
        public["argv"] = [_redact_path(item) if isinstance(item, str) and os.path.isabs(item) else item for item in argv]
    return public


def _command_document(result: Mapping[str, Any]) -> Mapping[str, Any] | None:
    raw = result.get("_stdout")
    return _unwrap(_json_bytes(raw)) if isinstance(raw, bytes) else None


def _snapshot_document(result: Mapping[str, Any]) -> Mapping[str, Any] | None:
    document = _command_document(result)
    if not document:
        return None
    snapshot = document.get("snapshot")
    return snapshot if isinstance(snapshot, Mapping) else document


def _snapshot_ids(value: Any) -> dict[str, list[str]]:
    found: dict[str, set[str]] = {}
    pending = [value]
    while pending:
        node = pending.pop()
        if isinstance(node, Mapping):
            for key, item in node.items():
                if isinstance(key, str) and (key == "id" or key.endswith("_id")) and isinstance(item, str) and item:
                    found.setdefault(key, set()).add(item)
                if isinstance(item, (Mapping, list)):
                    pending.append(item)
        elif isinstance(node, list):
            pending.extend(node)
    return {key: sorted(values) for key, values in sorted(found.items())}


def _git_files(repo: Path) -> tuple[list[Path], dict[str, Any]]:
    pathspec = [*_SOURCE_ROOTS, *_CONFIG_IDENTITIES]
    commands = [
        ["git", "-C", str(repo), "ls-files", "-z", "--", *pathspec],
        ["git", "-C", str(repo), "ls-files", "--others", "--exclude-standard", "-z", "--", *pathspec],
    ]
    results = [_bounded_process(command, os.environ, MAX_SOURCE_FILES * 512) for command in commands]
    paths: set[Path] = set()
    for result in results:
        raw = result.pop("_stdout", None)
        if result["status"] != "ok" or not isinstance(raw, bytes):
            return [], {"status": "failed", "tracked": results[0], "untracked": results[1]}
        for raw_name in raw.split(b"\0"):
            if not raw_name:
                continue
            try:
                relative = raw_name.decode("utf-8")
            except UnicodeDecodeError:
                return [], {"status": "failed", "reason": "non_utf8_path"}
            candidate = repo / relative
            if relative in _CONFIG_IDENTITIES or candidate.suffix in _SOURCE_SUFFIXES or relative in _EXTENSIONLESS_WRAPPERS:
                paths.add(candidate)
    if len(paths) > MAX_SOURCE_FILES:
        return [], {"status": "inconclusive", "reason": "source_file_limit", "count": len(paths)}
    return sorted(paths), {"status": "ok", "tracked": results[0], "untracked": results[1], "count": len(paths)}


def _git_commit(repo: Path) -> tuple[str | None, dict[str, Any]]:
    result = _bounded_process(["git", "-C", str(repo), "rev-parse", "HEAD"], os.environ, 128)
    raw = result.pop("_stdout", None)
    commit = raw.decode("ascii", "ignore").strip() if result["status"] == "ok" and isinstance(raw, bytes) else None
    return commit or None, result


def _content_hashes(paths: Sequence[Path], repo: Path) -> dict[str, str | None]:
    if len(paths) > MAX_SOURCE_FILES:
        return {}
    total = 0
    result: dict[str, str | None] = {}
    for path in paths:
        try:
            size = path.stat().st_size
        except OSError:
            result[path.relative_to(repo).as_posix()] = None
            continue
        total += size
        if size > MAX_SOURCE_FILE_BYTES or total > MAX_SOURCE_BYTES:
            result[path.relative_to(repo).as_posix()] = None
            continue
        result[path.relative_to(repo).as_posix()] = _sha256_file(path, MAX_SOURCE_FILE_BYTES)
    return result


def _config_identity(target: Any) -> dict[str, Any]:
    path = Path(target.config_path)
    digest = _sha256_file(path, MAX_EVIDENCE_BODY) if path.is_file() else None
    return {"path": _redact_path(path), "sha256": digest, "status": "available" if digest else "absent"}


def _ledger_schema_hash(ledger_path: str) -> str | None:
    raw_bytes = _read_file_bounded(Path(ledger_path), MAX_EVIDENCE_BODY)
    if raw_bytes is None:
        return None
    raw = _json_bytes(raw_bytes)
    if not isinstance(raw, Mapping):
        return None
    selected = raw.get("executables", {}).get("selected", {})
    value = selected.get("schema_sha256") if isinstance(selected, Mapping) else None
    return value.lower() if isinstance(value, str) and _SHA256_RE.fullmatch(value.lower()) else None


def _session_entry(entries: Any, target: Any, label: str) -> tuple[list[str], list[str]]:
    unavailable: list[str] = []
    mismatches: list[str] = []
    if not isinstance(entries, list):
        return [f"{label}.list"], []
    matching: list[Mapping[str, Any]] = []
    for item in entries:
        if not isinstance(item, Mapping):
            continue
        identity_keys = [key for key in ("id", "name", "session") if key in item]
        if not identity_keys:
            continue
        if all(item.get(key) == target.session.name for key in identity_keys):
            matching.append(item)
    if not matching:
        mismatches.append(f"{label}.target")
        return unavailable, mismatches
    if len(matching) != 1:
        mismatches.append(f"{label}.ambiguous")
        return unavailable, mismatches
    item = matching[0]
    if "running" not in item:
        unavailable.append(f"{label}.running")
    elif item.get("running") is not True:
        mismatches.append(f"{label}.running")
    default_keys = [key for key in ("default", "is_default") if key in item]
    if not default_keys:
        unavailable.append(f"{label}.default")
    elif any(item.get(key) is not False for key in default_keys):
        mismatches.append(f"{label}.default")
    if "socket" in item and target.socket_path is not None and item.get("socket") != target.socket_path:
        mismatches.append(f"{label}.socket")
    return unavailable, mismatches


def _check_backend(status: Mapping[str, Any] | None, sessions: Mapping[str, Any] | None, snapshot: Mapping[str, Any] | None, schema: Mapping[str, Any] | None, target: Any, executable: Any, schema_hash: str | None, expected_schema_hash: str | None) -> tuple[dict[str, Any], list[str], list[str]]:
    unavailable: list[str] = []
    mismatches: list[str] = []
    server = status if isinstance(status, Mapping) else None
    if server is None:
        unavailable.append("server_status")
        server = {}
    for key, value in {"version": executable.version, "protocol": executable.protocol}.items():
        if value is not None and key not in server:
            unavailable.append(f"server.{key}")
        elif value is not None and server.get(key) != value:
            mismatches.append(f"server.{key}")
    for key, expected in (("status", "running"), ("running", True), ("compatible", True), ("session", target.session.name), ("socket", target.socket_path), ("restart_needed", False)):
        if key not in server:
            unavailable.append(f"server.{key}")
        elif server.get(key) != expected:
            mismatches.append(f"server.{key}")
    capabilities = server.get("capabilities")
    if not isinstance(capabilities, Mapping):
        unavailable.append("server.capabilities")
        capabilities = {}
    else:
        for key in ("live_handoff", "detached_server_daemon"):
            if key not in capabilities:
                unavailable.append(f"server.capabilities.{key}")
            elif not isinstance(capabilities[key], (bool, str)):
                unavailable.append(f"server.capabilities.{key}")
    if not isinstance(schema, Mapping):
        unavailable.append("schema")
    else:
        for key, value in {"protocol": executable.protocol, "schema_version": executable.schema_version}.items():
            if value is not None and key not in schema:
                unavailable.append(f"schema.{key}")
            elif value is not None and schema.get(key) != value:
                mismatches.append(f"schema.{key}")
    absent, session_mismatches = _session_entry(sessions.get("sessions") if isinstance(sessions, Mapping) else None, target, "session_list")
    unavailable.extend(absent)
    mismatches.extend(session_mismatches)
    if not isinstance(snapshot, Mapping):
        unavailable.append("snapshot")
    else:
        # Stable Herdr 0.8.2 does not put session_id in api snapshot.  The
        # explicit session is already authenticated by the guarded CLI plan.
        for key, value in {"version": executable.version, "protocol": executable.protocol}.items():
            if value is not None and key not in snapshot:
                unavailable.append(f"snapshot.{key}")
            elif value is not None and snapshot.get(key) != value:
                mismatches.append(f"snapshot.{key}")
    if expected_schema_hash is None or schema_hash is None:
        unavailable.append("schema_sha256")
    elif schema_hash != expected_schema_hash:
        mismatches.append("schema_sha256")
    report = {"status": "pass" if not unavailable and not mismatches else "fail" if mismatches else "inconclusive", "identity": {"version": executable.version, "protocol": executable.protocol, "schema_version": executable.schema_version, "schema_sha256": schema_hash, "expected_schema_sha256": expected_schema_hash}, "target": {"session": target.session.name, "socket_path": _redact_path(target.socket_path)}, "capabilities": dict(sorted(capabilities.items()))}
    return report, unavailable, mismatches


def _host_origin(raw_url: str) -> tuple[str, str | None]:
    parsed = urlsplit(raw_url)
    if parsed.scheme not in {"http", "https"} or not parsed.hostname or parsed.username or parsed.password or parsed.query or parsed.fragment:
        return "", "host URL must be a credential-free http(s) origin without query or fragment"
    return urlunsplit((parsed.scheme, parsed.netloc, parsed.path.rstrip("/"), "", "")), None


def _host_get(origin: str, suffix: str) -> dict[str, Any]:
    started = time.monotonic()
    try:
        with urlopen(Request(origin + suffix, headers={"Accept": "application/json"}, method="GET"), timeout=COMMAND_TIMEOUT_SECONDS) as response:
            body = response.read(MAX_HTTP_BODY + 1)
            http_status = int(response.status)
    except (HTTPError, URLError, OSError, TimeoutError) as error:
        return {"endpoint": suffix, "status": "unavailable", "http_status": getattr(error, "code", None), "duration_ms": round((time.monotonic() - started) * 1000), "error": type(error).__name__}
    status = "ok" if http_status < 400 and len(body) <= MAX_HTTP_BODY else "failed"
    result: dict[str, Any] = {"endpoint": suffix, "status": status, "http_status": http_status, "duration_ms": round((time.monotonic() - started) * 1000)}
    if status == "ok":
        result["_document"] = _json_bytes(body)
    return result


def _check_browser_host(host_url: str | None, session: str, executable: Any, socket_path: str | None = None) -> tuple[dict[str, Any], list[str], list[str]]:
    if not host_url:
        return {"status": "inconclusive", "reason": "browser host URL was not supplied"}, ["host_url"], []
    origin, error = _host_origin(host_url)
    if error:
        return {"status": "inconclusive", "reason": error}, ["host_url"], []
    paths = ("/api/v1/status", "/api/v1/sessions", f"/api/v1/sessions/{quote(session, safe='')}/snapshot")
    requests = [_host_get(origin, path) for path in paths]
    unavailable = [f"host.{item['endpoint']}" for item in requests if item["status"] != "ok"]
    mismatches: list[str] = []
    docs = [_unwrap(item.get("_document")) for item in requests]
    status, sessions, snapshot = docs
    if not unavailable:
        server = status if isinstance(status, Mapping) else {}
        herdr = server.get("herdr") if isinstance(server.get("herdr"), Mapping) else server
        identity = herdr.get("identity") if isinstance(herdr.get("identity"), Mapping) else herdr
        if herdr.get("status") != "compatible":
            mismatches.append("host.herdr.status")
        if server.get("protocol_version") != "1":
            mismatches.append("host.protocol_version")
        if server.get("mode") != "normal":
            mismatches.append("host.mode")
        version_config = _json_bytes(_read_file_bounded(Path(__file__).resolve().parents[2] / "src-tauri/tauri.conf.json", MAX_EVIDENCE_BODY) or b"")
        expected_version = version_config.get("version") if isinstance(version_config, Mapping) else None
        if not isinstance(expected_version, str) or not expected_version:
            unavailable.append("host.expected_cockpit_version")
        elif server.get("cockpit_version") != expected_version:
            mismatches.append("host.cockpit_version")
        capabilities = server.get("capabilities")
        if not isinstance(capabilities, Mapping) or capabilities.get("terminal_mouse_input") is not True:
            mismatches.append("host.capabilities.terminal_mouse_input")
        for key, value in {"version": executable.version, "protocol": executable.protocol, "schema_version": executable.schema_version}.items():
            if value is not None and identity.get(key) != value:
                mismatches.append(f"host.herdr.identity.{key}")
        if not isinstance(identity, Mapping):
            unavailable.append("host.status.identity")
        session_absent, session_mismatch = _session_entry(sessions.get("sessions") if isinstance(sessions, Mapping) else None, SimpleTarget(session, socket_path), "host.session")
        unavailable.extend(session_absent)
        mismatches.extend(session_mismatch)
        if not isinstance(snapshot, Mapping):
            unavailable.append("host.snapshot_dto")
        else:
            for key, value in {"session_id": session, "version": executable.version, "protocol": executable.protocol}.items():
                if value is not None and snapshot.get(key) != value:
                    mismatches.append(f"host.snapshot.{key}")
    report: dict[str, Any] = {"status": "pass" if not unavailable and not mismatches else "fail" if mismatches else "inconclusive", "origin": _redact_path(origin), "requests": [{key: value for key, value in item.items() if key != "_document"} for item in requests]}
    if isinstance(status, Mapping):
        report["cockpit_capabilities"] = status.get("capabilities") if isinstance(status.get("capabilities"), Mapping) else None
        herdr = status.get("herdr")
        if isinstance(herdr, Mapping):
            report["herdr_identity"] = _safe_identity(herdr)
    return report, unavailable, mismatches


class SimpleTarget:
    def __init__(self, session: str, socket_path: str | None):
        self.session = SimpleName(session)
        self.socket_path = socket_path


class SimpleName:
    def __init__(self, name: str):
        self.name = name


def _safe_identity(value: Any) -> dict[str, Any] | None:
    if not isinstance(value, Mapping):
        return None
    safe: dict[str, Any] = {}
    for key, item in value.items():
        if not isinstance(key, str) or any(word in key.lower() for word in ("password", "token", "secret", "credential", "stdout", "stderr", "output", "text", "input", "capture", "title", "message")):
            continue
        if isinstance(item, str) and len(item) <= 1024:
            safe[key] = _redact_path(item) if "path" in key.lower() or item.startswith("/") else item
        elif isinstance(item, (bool, int)) or item is None:
            safe[key] = item
    return dict(sorted(safe.items()))


def _private_root(root: Path) -> Path | None:
    try:
        if any(component.is_symlink() for component in _path_chain(root)):
            return None
        identity = root.stat()
        if not stat.S_ISDIR(identity.st_mode) or identity.st_uid != os.geteuid() or identity.st_mode & 0o022:
            return None
        return root.resolve(strict=True)
    except OSError:
        return None


def _path_chain(path: Path) -> tuple[Path, ...]:
    chain: list[Path] = []
    current = path
    while current != current.parent:
        chain.append(current)
        current = current.parent
    chain.append(current)
    return tuple(reversed(chain))


def _roots_from_ledger(ledger: Any, ledger_path: str) -> tuple[Path, ...]:
    roots: list[Path] = []
    runtime = _private_root(Path(ledger.resource_root))
    if runtime:
        roots.append(runtime)
    raw_bytes = _read_file_bounded(Path(ledger_path), MAX_EVIDENCE_BODY)
    raw = _json_bytes(raw_bytes) if raw_bytes is not None else None
    resources = raw.get("resources") if isinstance(raw, Mapping) else None
    if isinstance(resources, list):
        for resource in resources:
            if not isinstance(resource, Mapping) or resource.get("owner") != ledger.run_id or resource.get("kind") != "durable evidence":
                continue
            value = resource.get("path")
            if isinstance(value, str):
                durable = _private_root(Path(value))
                if durable:
                    roots.append(durable)
    return tuple(dict.fromkeys(roots))


def _canonical_owned_file(raw_path: str, roots: Sequence[Path], kind: str, limit: int) -> tuple[Path | None, str | None]:
    try:
        candidate = Path(raw_path)
        if not candidate.is_absolute() or any(component.is_symlink() for component in _path_chain(candidate)):
            return None, f"{kind}.ownership"
        canonical = candidate.resolve(strict=True)
        if not any(canonical != root and root in canonical.parents for root in roots):
            return None, f"{kind}.ownership"
        identity = os.lstat(candidate)
        if not stat.S_ISREG(identity.st_mode) or identity.st_uid != os.geteuid() or identity.st_nlink != 1 or identity.st_size > limit:
            return None, f"{kind}.ownership"
        return canonical, None
    except (OSError, RuntimeError):
        return None, f"{kind}.missing"


def _owned_hashed_artifact(value: Any, kind: str, roots: Sequence[Path]) -> tuple[dict[str, Any] | None, str | None]:
    if not isinstance(value, Mapping) or not isinstance(value.get("path"), str) or not isinstance(value.get("sha256"), str):
        return None, f"{kind}.schema"
    claimed = value["sha256"].lower()
    if not _SHA256_RE.fullmatch(claimed):
        return None, f"{kind}.sha256"
    selected_roots = tuple(roots)
    path, error = _canonical_owned_file(value["path"], selected_roots, kind, MAX_ARTIFACT_BYTES)
    if error or path is None:
        return None, error or f"{kind}.missing"
    actual = _sha256_file(path)
    if actual != claimed:
        return None, f"{kind}.hash"
    return {"path": _redact_path(path), "sha256": claimed}, None


def _decode_image(path: Path) -> str | None:
    """Validate bounded, non-interlaced RGB/RGBA PNG screenshots."""
    limit = 64 * 1024 * 1024
    data = _read_file_bounded(path, limit)
    if data is None:
        return "visible_fixture.image_limit"
    if path.suffix.lower() != ".png" or not data.startswith(b"\x89PNG\r\n\x1a\n"):
        return "visible_fixture.image"
    if len(data) < 33 or data[8:16] != b"\0\0\0\rIHDR":
        return "visible_fixture.image"
    width, height, depth, colour, compression, filtering, interlace = struct.unpack(">IIBBBBB", data[16:29])
    if not 1 <= width <= 8192 or not 1 <= height <= 8192 or depth not in {8, 16} or colour not in {2, 6} or compression != 0 or filtering != 0 or interlace != 0:
        return "visible_fixture.image"
    row_bytes = width * (3 if colour == 2 else 4) * (depth // 8)
    expected = (row_bytes + 1) * height
    if expected > limit:
        return "visible_fixture.image_limit"
    offset, compressed, end_seen = 8, bytearray(), False
    try:
        while offset + 12 <= len(data):
            length = struct.unpack(">I", data[offset:offset + 4])[0]
            chunk = data[offset + 4:offset + 8]
            if offset + 12 + length > len(data):
                return "visible_fixture.image"
            body = data[offset + 8:offset + 8 + length]
            crc = struct.unpack(">I", data[offset + 8 + length:offset + 12 + length])[0]
            if zlib.crc32(chunk + body) & 0xffffffff != crc:
                return "visible_fixture.image"
            if chunk == b"IHDR" and offset != 8:
                return "visible_fixture.image"
            if chunk == b"IDAT":
                compressed.extend(body)
            offset += 12 + length
            if chunk == b"IEND":
                if length:
                    return "visible_fixture.image"
                end_seen = True
                break
        if not end_seen or offset != len(data) or not compressed:
            return "visible_fixture.image"
        decoder = zlib.decompressobj()
        decoded = decoder.decompress(compressed, expected + 1)
        if len(decoded) != expected or not decoder.eof or decoder.unused_data or decoder.unconsumed_tail:
            return "visible_fixture.image"
        if any(value > 4 for value in decoded[::row_bytes + 1]):
            return "visible_fixture.image"
    except (zlib.error, struct.error):
        return "visible_fixture.image"
    return None


def _native_header(path: Path) -> str | None:
    try:
        with path.open("rb") as stream:
            prefix = stream.read(1024 * 1024)
            stream.seek(max(0, path.stat().st_size - 1024 * 1024))
            suffix = stream.read(1024 * 1024)
    except OSError:
        return "native.elf"
    if not prefix.startswith(b"\x7fELF"):
        return "native.elf"
    if b"AI\x01" not in prefix + suffix and b"AI\x02" not in prefix + suffix and b"AppImage" not in prefix + suffix:
        return "native.appimage_header"
    return None


def _receipt_json(path: Path, kind: str) -> tuple[Mapping[str, Any] | None, str | None]:
    raw = _read_file_bounded(path, MAX_EVIDENCE_BODY)
    document = _json_bytes(raw) if raw is not None else None
    if not isinstance(document, Mapping):
        return None, f"{kind}.json"
    return document, None


def _input_receipt(artifact: Mapping[str, Any], session: str, canonical: Path | None = None) -> str | None:
    path = canonical or Path(str(artifact["path"]))
    receipt, error = _receipt_json(path, "input_receipt")
    if error or receipt is None:
        return error
    if receipt.get("session") != session or receipt.get("exact") is not True:
        return "input_receipt.identity"
    expected, captured = receipt.get("expected_hex"), receipt.get("captured_hex")
    if not isinstance(expected, str) or not isinstance(captured, str) or not expected or len(expected) % 2 or expected.lower() != captured.lower():
        return "input_receipt.bytes"
    try:
        bytes.fromhex(expected)
    except ValueError:
        return "input_receipt.bytes"
    return None


def _identity_required(raw: Mapping[str, Any], name: str, fields: Sequence[str], unavailable: list[str]) -> Mapping[str, Any] | None:
    value = raw.get(name)
    if not isinstance(value, Mapping):
        unavailable.append(f"identity.{name}")
        return None
    for field in fields:
        field_value = value.get(field)
        valid = isinstance(field_value, (str, int, bool)) and field_value not in ("", None)
        if name == "backend" and field in {"protocol", "schema_version"}:
            valid = isinstance(field_value, int) and not isinstance(field_value, bool)
        elif field not in {"protocol", "schema_version"}:
            valid = isinstance(field_value, str) and bool(field_value)
        if not valid:
            unavailable.append(f"identity.{name}.{field}")
    return value


def _load_host_evidence(path: Path, client: str, run_id: str, session: str, roots: Sequence[Path]) -> tuple[dict[str, Any], list[str], list[str]]:
    selected_roots = tuple(roots)
    canonical, ownership_error = _canonical_owned_file(str(path), selected_roots, "evidence", MAX_EVIDENCE_BODY)
    if ownership_error or canonical is None:
        return {"status": "inconclusive", "reason": ownership_error or "evidence.missing"}, [ownership_error or "evidence.missing"], []
    raw_bytes = _read_file_bounded(canonical, MAX_EVIDENCE_BODY)
    if raw_bytes is None:
        return {"status": "inconclusive", "reason": "evidence_missing_or_limit"}, ["evidence_missing_or_limit"], []
    evidence_hash = _sha256_bytes(raw_bytes)
    raw = _json_bytes(raw_bytes)
    if not isinstance(raw, Mapping) or raw.get("schema_version") != 1 or raw.get("client") != client or raw.get("run_id") != run_id or raw.get("session") != session:
        return {"status": "inconclusive", "reason": "evidence_identity"}, ["evidence_identity"], []
    unavailable: list[str] = []
    mismatches: list[str] = []
    for name, fields in (("source", ("commit",)), ("build", ("commit", "sha256")), ("generated_client", ("version", "sha256")), ("display", ("name",)), ("renderer", ("name",)), ("backend", _BACKEND_KEYS)):
        _identity_required(raw, name, fields, unavailable)
    fixture_identity = _identity_required(raw, "fixture", ("pane_id", "terminal_id"), unavailable)
    if fixture_identity is not None and not isinstance(fixture_identity.get("id", fixture_identity.get("fixture_id")), str):
        unavailable.append("identity.fixture.id")
    for identity_name, hash_fields in (("build", ("sha256",)), ("generated_client", ("sha256",)), ("backend", ("schema_sha256", "effective_config_sha256"))):
        identity = raw.get(identity_name)
        if isinstance(identity, Mapping):
            for field in hash_fields:
                value = identity.get(field)
                if not isinstance(value, str) or not _SHA256_RE.fullmatch(value.lower()):
                    unavailable.append(f"identity.{identity_name}.{field}")
    capabilities = raw.get("capabilities")
    if not isinstance(capabilities, Mapping):
        unavailable.append("capabilities")
    else:
        for name in _REQUIRED_CAPABILITIES:
            value = capabilities.get(name)
            if name not in capabilities:
                unavailable.append(f"capabilities.{name}")
            elif not isinstance(value, (bool, str)) or value == "":
                unavailable.append(f"capabilities.{name}")
    artifacts = raw.get("artifacts")
    canonical_artifacts: dict[str, Path] = {}
    checked_artifacts: dict[str, dict[str, Any] | None] = {}
    if not isinstance(artifacts, Mapping):
        unavailable.append("artifacts")
    else:
        required_artifacts = ["visible_fixture", "input_receipt", "observation_receipt"] + (["launch_receipt"] if client == "native" else [])
        for name in required_artifacts:
            report, error = _owned_hashed_artifact(artifacts.get(name), name, selected_roots)
            checked_artifacts[name] = report
            if error:
                unavailable.append(error)
                continue
            assert isinstance(artifacts.get(name), Mapping)
            selected, selected_error = _canonical_owned_file(str(artifacts[name]["path"]), selected_roots, name, MAX_ARTIFACT_BYTES)
            if selected_error or selected is None:
                unavailable.append(selected_error or f"{name}.ownership")
            else:
                canonical_artifacts[name] = selected
        fixture_report = checked_artifacts.get("visible_fixture")
        if fixture_report and "visible_fixture" in canonical_artifacts:
            if _decode_image(canonical_artifacts["visible_fixture"]):
                unavailable.append("visible_fixture.image")
        if "input_receipt" in canonical_artifacts and isinstance(artifacts.get("input_receipt"), Mapping):
            error = _input_receipt(artifacts["input_receipt"], session, canonical_artifacts["input_receipt"])
            if error:
                unavailable.append(error)
        observation, error = _receipt_json(canonical_artifacts.get("observation_receipt", Path("/nonexistent")), "observation_receipt")
        if error or observation is None:
            unavailable.append(error or "observation_receipt.json")
        else:
            fixture = raw.get("fixture") if isinstance(raw.get("fixture"), Mapping) else {}
            fixture_id = fixture.get("id", fixture.get("fixture_id"))
            if observation.get("provenance") not in {"main-observed", "main-observed-receipt"}:
                unavailable.append("observation_receipt.provenance")
            for key, value in (("client", client), ("session", session), ("artifact_sha256", artifacts.get("visible_fixture", {}).get("sha256") if isinstance(artifacts.get("visible_fixture"), Mapping) else None), ("fixture_id", fixture_id)):
                if key not in observation:
                    unavailable.append(f"observation_receipt.{key}")
                elif value is not None and observation.get(key) != value:
                    mismatches.append(f"observation_receipt.{key}")
    assertions = raw.get("assertions")
    if not isinstance(assertions, Mapping):
        unavailable.append("assertions")
    else:
        for name in ("visible_fixture", "click_focus", "basic_input", "app_mouse"):
            if name not in assertions:
                unavailable.append(f"assertions.{name}")
            elif name != "app_mouse" and assertions[name] not in (True, "pass", "passed"):
                mismatches.append(f"assertions.{name}")
        if "app_mouse" in assertions and assertions["app_mouse"] is not False:
            mismatches.append("assertions.app_mouse")
    client_binary = _safe_identity(raw.get("client_binary"))
    if client == "native":
        binary, binary_error = _owned_hashed_artifact(raw.get("client_binary"), "client_binary", selected_roots)
        if binary_error:
            unavailable.append(binary_error)
        else:
            client_binary = binary
            assert isinstance(raw.get("client_binary"), Mapping)
            binary_path, _ = _canonical_owned_file(str(raw["client_binary"]["path"]), selected_roots, "client_binary", MAX_ARTIFACT_BYTES)
            if binary_path is None:
                unavailable.append("client_binary.ownership")
            else:
                try:
                    mode = os.lstat(binary_path).st_mode
                except OSError:
                    mode = 0
                if not stat.S_ISREG(mode) or not (mode & 0o111):
                    unavailable.append("client_binary.executable")
                native_error = _native_header(binary_path)
                if native_error:
                    unavailable.append(native_error)
            if "launch_receipt" in canonical_artifacts:
                launch, launch_error = _receipt_json(canonical_artifacts["launch_receipt"], "launch_receipt")
                if launch_error or launch is None:
                    unavailable.append(launch_error or "launch_receipt.json")
                else:
                    binary_hash = raw["client_binary"].get("sha256") if isinstance(raw.get("client_binary"), Mapping) else None
                    for key, value in (("client", "native"), ("session", session), ("binary_sha256", binary_hash)):
                        if key not in launch:
                            unavailable.append(f"launch_receipt.{key}")
                        elif value is not None and launch.get(key) != value:
                            mismatches.append(f"launch_receipt.{key}")
                    if launch.get("status") not in {"launched", "running", "ready", "observed"}:
                        mismatches.append("launch_receipt.status")
    backend = raw.get("backend")
    backend_identity = _safe_identity(backend)
    if client == "native" and raw.get("host_url") is not None:
        mismatches.append("native.http_masquerade")
    report: dict[str, Any] = {"status": "pass" if not unavailable and not mismatches else "fail" if mismatches else "inconclusive", "path": _redact_path(canonical), "sha256": evidence_hash, "artifacts": checked_artifacts, "client_binary": client_binary, "identities": {name: _safe_identity(raw.get(name)) for name in ("source", "build", "generated_client", "display", "renderer", "fixture")}, "backend_identity": backend_identity, "capabilities": _safe_identity(capabilities), "assertions": assertions if isinstance(assertions, Mapping) else None, "provenance": "Main-observed receipt required; JSON fields are not UI proof"}
    report["client"] = client
    return report, unavailable, mismatches


def _pair_compare(primary: Mapping[str, Any], pair: Mapping[str, Any]) -> tuple[dict[str, Any], list[str], list[str]]:
    unavailable: list[str] = []
    mismatches: list[str] = []
    left, right = primary.get("backend_identity"), pair.get("backend_identity")
    if not isinstance(left, Mapping) or not isinstance(right, Mapping):
        unavailable.append("cross_host.backend_identity")
    else:
        for key in _BACKEND_KEYS:
            if key not in left or key not in right:
                unavailable.append(f"cross_host.{key}")
            elif left[key] != right[key]:
                mismatches.append(f"cross_host.{key}")
    return {"status": "pass" if not unavailable and not mismatches else "fail" if mismatches else "inconclusive", "paired_client": pair.get("client")}, unavailable, mismatches


class _Parser(argparse.ArgumentParser):
    def error(self, message: str) -> None:
        raise ValueError(message)


def _parser() -> argparse.ArgumentParser:
    schema = "Evidence schema: schema_version=1, explicit run_id/session/client, source/build/generated_client/display/renderer/fixture identities, backend={version,protocol,schema_version,schema_sha256,effective_config_sha256}, capabilities={mouse_input,click_focus,app_mode_pointer,wheel_scroll,graphics} with explicit states, artifacts={visible_fixture,input_receipt,observation_receipt[,launch_receipt]}, assertions={visible_fixture,click_focus,basic_input,app_mouse}. Artifacts are canonical regular files beneath the ledger-owned runtime or durable-evidence root and include SHA-256 hashes. Images are decoded; native binaries require ELF+AppImage headers and a bound launch receipt. Browser PASS additionally requires host status/sessions/snapshot. Use --paired-evidence for an explicit native/browser agreement; a single host is never reported as agreement."
    return _Parser(description=__doc__, epilog=schema, formatter_class=argparse.RawDescriptionHelpFormatter)


def _guard_output_path(output: str, ledger_path: str, run_id: str) -> Path | None:
    if output == "-":
        return None
    ledger = load_ledger(ledger_path)
    if ledger.run_id != run_id:
        raise ResourceGuardError("run_id does not own this ledger")
    roots = _roots_from_ledger(ledger, ledger_path)
    if not roots:
        raise ResourceGuardError("ledger has no owned evidence roots")
    target = Path(output)
    if not target.is_absolute() or any(component.is_symlink() for component in _path_chain(target)):
        raise ResourceGuardError("output must be a non-symlink path beneath an owned evidence root")
    canonical_parent = target.parent.resolve(strict=True)
    if not any(root in canonical_parent.parents or canonical_parent == root for root in roots):
        raise ResourceGuardError("output is outside owned evidence roots")
    if target.exists():
        identity = os.lstat(target)
        if not stat.S_ISREG(identity.st_mode) or identity.st_uid != os.geteuid() or identity.st_nlink != 1:
            raise ResourceGuardError("output is not an owned regular file")
    return target


def _write_output(path: Path, payload: str) -> None:
    temporary: Path | None = None
    try:
        with tempfile.NamedTemporaryFile(mode="w", encoding="utf-8", dir=path.parent, prefix=".startup-inventory-", delete=False) as stream:
            temporary = Path(stream.name)
            stream.write(payload)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


def collect(args: argparse.Namespace) -> tuple[dict[str, Any], int]:
    ledger = load_ledger(args.ledger)
    try:
        plans = [(operation, *prepare_subprocess(ledger, args.run_id, args.session, operation, cleanup=False)) for operation in _READ_ONLY_OPERATIONS]
    except ResourceGuardError as error:
        return {"schema_version": 1, "status": "INCONCLUSIVE", "run_id": args.run_id, "session": {"name": args.session, "client": args.client}, "guard": {"status": "rejected", "error": str(error)}}, EXIT_INCONCLUSIVE
    target = plans[0][1]
    results = [_run_prepared(operation, argv, environment) for operation, _target, argv, environment in plans]
    documents = {_READ_ONLY_OPERATIONS[index]: _command_document(result) for index, result in enumerate(results)}
    snapshots = _snapshot_document(results[3])
    schema_raw = results[2].get("_stdout")
    schema_hash = _sha256_bytes(schema_raw) if isinstance(schema_raw, bytes) else None
    config = _config_identity(target)
    backend, absent, mismatches = _check_backend(documents[_READ_ONLY_OPERATIONS[0]], documents[_READ_ONLY_OPERATIONS[1]], snapshots, documents[_READ_ONLY_OPERATIONS[2]], target, ledger.executable, schema_hash, _ledger_schema_hash(args.ledger))
    host_absent: list[str] = []
    host_mismatches: list[str] = []
    if args.client == "browser":
        host, host_absent, host_mismatches = _check_browser_host(args.host_url, target.session.name, ledger.executable, target.socket_path)
    elif args.host_url:
        host, host_absent = {"status": "inconclusive", "reason": "host URL is invalid for native evidence"}, ["native.host_url"]
    else:
        host = {"status": "not_applicable"}
    roots = _roots_from_ledger(ledger, args.ledger)
    if args.host_evidence:
        evidence, evidence_absent, evidence_mismatches = _load_host_evidence(Path(args.host_evidence), args.client, args.run_id, target.session.name, roots)
    else:
        evidence, evidence_absent, evidence_mismatches = {"status": "inconclusive", "reason": "host evidence was not supplied"}, ["host_evidence"], []
    host["evidence"] = evidence
    backend_identity = evidence.get("backend_identity") if isinstance(evidence, Mapping) else None
    if isinstance(backend_identity, Mapping):
        actual_identity = {"version": ledger.executable.version, "protocol": ledger.executable.protocol, "schema_version": ledger.executable.schema_version, "schema_sha256": schema_hash, "effective_config_sha256": config.get("sha256")}
        for key, actual in actual_identity.items():
            if backend_identity.get(key) != actual:
                evidence_mismatches.append(f"evidence.backend.{key}")
    else:
        evidence_absent.append("evidence.backend")
    paired_report: dict[str, Any] = {"status": "inconclusive", "reason": "paired evidence was not supplied"}
    pair_absent: list[str] = ["cross_host"]
    pair_mismatches: list[str] = []
    if getattr(args, "paired_evidence", None):
        paired, paired_missing, paired_errors = _load_host_evidence(Path(args.paired_evidence), "native" if args.client == "browser" else "browser", args.run_id, target.session.name, roots)
        paired_report, pair_missing, pair_errors = _pair_compare(evidence, paired)
        pair_absent = paired_missing + pair_missing
        pair_mismatches = paired_errors + pair_errors
        if paired.get("status") != "pass":
            pair_absent.append("cross_host.evidence")
    repo = Path(__file__).resolve().parents[2]
    paths, git_files = _git_files(repo)
    commit, git_commit = _git_commit(repo)
    hashes = _content_hashes(paths, repo)
    source_hash_failure = not paths or git_files.get("status") != "ok" or git_commit.get("status") != "ok" or commit is None or len(hashes) != len(paths) or any(value is None for value in hashes.values())
    source = {"commit": commit, "inventory": [{"path": name, "sha256": digest} for name, digest in sorted(hashes.items())]}
    all_absent = absent + host_absent + evidence_absent + pair_absent
    all_mismatches = mismatches + host_mismatches + evidence_mismatches + pair_mismatches
    if source_hash_failure:
        all_absent.append("source_inventory")
    if any(result["status"] != "ok" for result in results):
        all_absent.append("commands")
    status, exit_code = ("FAIL", EXIT_FAIL) if all_mismatches else ("INCONCLUSIVE", EXIT_INCONCLUSIVE) if all_absent else ("PASS", EXIT_PASS)
    git_public = {"status": git_files.get("status"), "count": git_files.get("count"), "commit_status": git_commit.get("status")}
    inventory = {"schema_version": 1, "status": status, "run_id": args.run_id, "session": {"name": target.session.name, "client": args.client}, "guard": {"status": "accepted", "config_path": _redact_path(target.config_path), "socket_path": _redact_path(target.socket_path)}, "executable": {"path": _redact_path(ledger.executable.path), "sha256": ledger.executable.sha256, "version": ledger.executable.version, "protocol": ledger.executable.protocol, "schema_version": ledger.executable.schema_version}, "effective_config": config, "backend_cli": backend, "cockpit": host, "cross_host": paired_report, "commands": [_public_command(result) for result in results], "snapshot": {"status": "pass" if snapshots else "inconclusive", "resource_ids": _snapshot_ids(snapshots) if snapshots else {}}, "source": source, "git": git_public, "evidence": evidence}
    return inventory, exit_code


def main(argv: Sequence[str] | None = None) -> int:
    parser = _parser()
    parser.add_argument("--ledger", required=True, help="run-owned resources.json ledger")
    parser.add_argument("--run-id", required=True, help="explicit ledger run identifier")
    parser.add_argument("--session", required=True, help="explicit non-default run-owned session")
    parser.add_argument("--client", required=True, choices=("browser", "native"), help="evidence client")
    parser.add_argument("--output", required=True, help="report JSON path beneath an owned root, or - for stdout")
    parser.add_argument("--host-url", help="browser Cockpit gateway origin")
    parser.add_argument("--host-evidence", help="externally collected evidence JSON")
    parser.add_argument("--paired-evidence", help="the other client inventory evidence JSON")
    try:
        args = parser.parse_args(argv)
    except ValueError as error:
        parser.print_usage(sys.stderr)
        print(f"startup-inventory: {error}", file=sys.stderr)
        return EXIT_USAGE
    output_path: Path | None = None
    if args.output != "-":
        try:
            output_path = _guard_output_path(args.output, args.ledger, args.run_id)
        except (ResourceGuardError, OSError, ValueError, json.JSONDecodeError) as error:
            print(f"startup-inventory: output rejected: {type(error).__name__}", file=sys.stderr)
            return EXIT_INCONCLUSIVE
    try:
        inventory, exit_code = collect(args)
        payload = json.dumps(inventory, sort_keys=True, separators=(",", ":")) + "\n"
        if output_path is None:
            sys.stdout.write(payload)
        else:
            _write_output(output_path, payload)
        return exit_code
    except (ResourceGuardError, OSError, ValueError, json.JSONDecodeError) as error:
        payload = json.dumps({"schema_version": 1, "status": "INCONCLUSIVE", "error": type(error).__name__, "message": str(error)}, sort_keys=True, separators=(",", ":")) + "\n"
        if output_path is None:
            sys.stdout.write(payload)
        else:
            _write_output(output_path, payload)
        return EXIT_INCONCLUSIVE


if __name__ == "__main__":
    raise SystemExit(main())
