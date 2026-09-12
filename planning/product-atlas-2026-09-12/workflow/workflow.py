#!/usr/bin/env python3
"""Deterministic, fail-closed evidence workflow engine.

The engine only writes below a named ``runs/<run-id>`` directory.  It is
intentionally a small stdlib program: workers perform browser collection and
publish JSON receipts; this program gates the receipts and advances work.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import sys
import tempfile
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Iterable

HERE = Path(__file__).resolve().parent
FRAGMENTS = (
    "browser-extension.json", "cockpit-surfaces.json", "environment.json",
    "evidence-contract.json", "orchestration.json", "coverage-matrix.json",
)
HEX64 = re.compile(r"^[a-f0-9]{64}$")
RUN_ID = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_.-]{1,63}$")
SCREEN = re.compile(r"^(before|action-[0-9]{2}|after)-[a-z0-9-]+-(1440x900|1024x640)\.png$")
RFC3339 = re.compile(r"^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d+)?(?:Z|[+-]\d\d:\d\d)$")
SURFACE_ALIASES = {
    "feedbackpanel": {"feedbackpanel", "feedback", "browser-feedback", "browser feedback"},
    "command menu": {"command", "command-menu", "commandmenu", "menus"},
    "files": {"files", "file", "files-viewer", "filesviewer"},
    "context": {"context", "context-browser", "contextbrowser", "sources"},
    "review": {"review", "review-setup", "reviewsetup"},
    "page": {"page", "browser", "fixture-page"},
    "terminal": {"terminal", "pane", "workbench"},
    "popup": {"popup", "extension-popup", "browser-extension"},
    "capture": {"capture", "capture-pipeline"},
    "environment": {"environment"},
    "herdr": {"herdr", "session", "space", "tab", "pane", "agent"},
}


def _norm(value: Any) -> str:
    return re.sub(r"[^a-z0-9]+", "-", str(value).lower()).strip("-")


def _surface_tokens(value: Any) -> set[str]:
    text = _norm(value)
    tokens = {text} if text else set()
    tokens.update(part for part in text.split("-") if len(part) > 2)
    for name, aliases in SURFACE_ALIASES.items():
        normalized = {_norm(name), *(_norm(a) for a in aliases)}
        if text in normalized or any(a in text or text in a for a in normalized):
            tokens.update(normalized)
    return tokens


def expected_surface_tokens(packet: dict[str, Any]) -> set[str]:
    """Return concrete names accepted as proof of this packet's surface."""
    tokens: set[str] = set()
    for surface in packet.get("surfaces", []):
        tokens.update(_surface_tokens(surface))
    # A scenario/cell name is useful for machine evidence, but must not make a
    # generic fixture pass merely because it repeats the packet's identifier.
    tokens.update(_surface_tokens(packet.get("cellId", "")))
    tokens.update(_surface_tokens(packet.get("scenarioId", "")))
    return tokens


def _surface_matches(value: Any, packet: dict[str, Any]) -> bool:
    actual = _norm(value)
    if not actual:
        return False
    expected: set[str] = set()
    for surface in packet.get("surfaces", []):
        expected.add(_norm(surface))
        expected.update(_norm(a) for a in SURFACE_ALIASES.get(_norm(surface), set()))
        # Composite journey labels expose each concrete named surface.
        expected.update(_norm(a) for a in SURFACE_ALIASES
                        if _norm(a) in _norm(surface))
    return actual in expected


def _json_artifact(path: Path) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None


def _jsonl(path: Path) -> list[Any] | None:
    try:
        rows = []
        for line in path.read_text(encoding="utf-8").splitlines():
            if not line.strip():
                continue
            rows.append(json.loads(line))
        return rows
    except (OSError, ValueError):
        return None


def _iso(value: Any) -> datetime | None:
    if not isinstance(value, str) or not RFC3339.fullmatch(value):
        return None
    try:
        return datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError:
        return None


def now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def digest(value: Any) -> str:
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()).hexdigest()


def file_digest(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for block in iter(lambda: f.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def load_json(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as e:
        raise ValueError(f"fragment-invalid:{path.name}: {e}") from e
    if not isinstance(value, dict):
        raise ValueError(f"fragment-invalid:{path.name}: root must be object")
    return value


def load_fragments(root: Path = HERE) -> dict[str, dict[str, Any]]:
    result = {name[:-5]: load_json(root / name) for name in FRAGMENTS}
    versions = {v.get("schemaVersion") for v in result.values()}
    if versions != {1}:
        raise ValueError("fragment-schemaVersion-mismatch")
    commits = {result["orchestration"].get("baselineCommit"), result["coverage-matrix"].get("baselineCommit")}
    if len(commits) != 1 or not next(iter(commits), ""):
        raise ValueError("baseline-commit-mismatch")
    contract = result["evidence-contract"]
    for state in ("PASS", "FAIL", "INCONCLUSIVE"):
        if state not in contract.get("decisionStates", []):
            raise ValueError(f"evidence-contract-missing-state:{state}")
    if not contract.get("screenshots", {}).get("requiredViewports"):
        raise ValueError("evidence-contract-missing-viewports")
    return result


def _slug(text: str) -> str:
    return re.sub(r"[^a-z0-9]+", "-", str(text).lower()).strip("-") or "cell"


def expand_packets(frags: dict[str, dict[str, Any]]) -> list[dict[str, Any]]:
    """Expand all four matrix forms into stable packet records."""
    rows: list[dict[str, Any]] = []
    # The coverage matrix is the canonical product-level index.
    for cell in frags["coverage-matrix"].get("matrix", []):
        if isinstance(cell, dict) and cell.get("id"):
            rows.append({"source": "coverage-matrix", "scenarioId": cell["id"], "cellId": cell["id"],
                         "lane": _lane_for(cell), "surfaces": cell.get("surfaces", []),
                         "proof": cell.get("expectedVisibleOutput", []), "actions": cell.get("actionVariants", [])})
    # Surface-level journey cells.
    for journey in frags["cockpit-surfaces"].get("journeys", []):
        if not isinstance(journey, dict):
            continue
        jid = journey.get("id", "journey")
        for cell in journey.get("cells", []):
            if isinstance(cell, dict) and cell.get("id"):
                key = f"cockpit.{jid}.{cell['id']}"
                rows.append({"source": "cockpit-surfaces", "scenarioId": jid, "cellId": cell["id"],
                             "key": key, "lane": "setup-context-review", "surfaces": [journey.get("surface", "")],
                             "proof": cell.get("proof", []), "actions": [cell.get("action", "")]})
    for journey in frags["browser-extension"].get("journeys", []):
        if not isinstance(journey, dict):
            continue
        jid = journey.get("id", "journey")
        for cell in journey.get("evidenceMatrix", []):
            if isinstance(cell, dict) and cell.get("cell"):
                key = f"browser.{jid}.{cell['cell']}"
                rows.append({"source": "browser-extension", "scenarioId": jid, "cellId": cell["cell"],
                             "key": key, "lane": "browser-extension", "surfaces": ["browser-extension"],
                             "proof": cell.get("machineProof", []), "actions": [cell.get("cell", "")]})
    env = frags["environment"].get("collectionMatrix", {})
    for cell in env.get("cells", []):
        if isinstance(cell, dict) and cell.get("id"):
            key = f"environment.{cell['id']}"
            rows.append({"source": "environment", "scenarioId": key, "cellId": cell["id"], "key": key,
                         "lane": "fixture", "surfaces": ["environment"], "proof": cell.get("observableProof", []),
                         "actions": [cell.get("id", "")]})
    # Bootstrap and fixture are explicit packets so the DAG cannot deadlock:
    # workers must prove the prerequisite phases before capture lanes unlock.
    rows = [
        {"source": "orchestration", "scenarioId": "bootstrap", "cellId": "bootstrap",
         "key": "phase.bootstrap", "lane": "bootstrap", "phase": "bootstrap", "surfaces": ["Herdr"], "proof": [], "actions": ["bootstrap"]},
        {"source": "orchestration", "scenarioId": "fixture", "cellId": "fixture",
         "key": "phase.fixture", "lane": "fixture", "phase": "fixture", "surfaces": ["fixture"], "proof": [], "actions": ["fixture"]},
    ] + rows
    views = frags["evidence-contract"]["screenshots"]["requiredViewports"]
    budget = int(frags["evidence-contract"].get("retryPolicy", {}).get("defaultBudget", 2))
    out = []
    seen: set[str] = set()
    for row in rows:
        key = row.get("key", row["cellId"])
        if key in seen:
            continue
        seen.add(key)
        pid = "packet-" + hashlib.sha256(key.encode()).hexdigest()[:16]
        packet_surfaces = row["surfaces"]
        surface_text = " ".join(str(s) for s in packet_surfaces)
        surface_tokens = sorted(_surface_tokens(surface_text))
        out.append({"packetId": pid, "key": key, "source": row["source"], "scenarioId": row["scenarioId"],
                    "cellId": row["cellId"], "lane": row["lane"], "surfaces": packet_surfaces,
                    "surfaceTokens": surface_tokens, "expectedSurfaceTokens": surface_tokens,
                    "expectedSurfaceTypes": sorted({_norm(s) for s in packet_surfaces}),
                    "expectedArtifactKinds": ["screenshot", "machine-evidence", "action-log",
                                              "authority-snapshot"],
                    "proof": row["proof"], "actions": row["actions"], "viewports": views,
                    "retryBudget": budget, "phase": row.get("phase", "capture-lanes"), "status": "PENDING"})
    return sorted(out, key=lambda x: x["key"])


def _lane_for(cell: dict[str, Any]) -> str:
    surfaces = {str(s).lower() for s in cell.get("surfaces", [])}
    if "page" in surfaces or "feedbackpanel" in surfaces:
        return "browser-extension"
    if "files" in surfaces or "context" in surfaces or "review" in surfaces:
        return "setup-context-review"
    return "workbench"


def atomic_write(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp = tempfile.mkstemp(prefix=f".{path.name}.", dir=str(path.parent))
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            json.dump(value, f, sort_keys=True, indent=2, ensure_ascii=False)
            f.write("\n")
        os.replace(tmp, path)
    finally:
        if os.path.exists(tmp):
            os.unlink(tmp)


def read_run(run: Path, name: str, default: Any) -> Any:
    path = run / name
    if not path.exists():
        return default
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as e:
        raise ValueError(f"run-file-invalid:{name}: {e}") from e


def run_path(root: Path, run_id: str) -> Path:
    if not RUN_ID.fullmatch(run_id) or run_id in {".", ".."}:
        raise ValueError("invalid-run-id")
    runs = (root / "runs").resolve()
    target = (runs / run_id).resolve()
    if runs not in target.parents:
        raise ValueError("run-path-escape")
    return target


def packet_error(packet: dict[str, Any], predicate: str, detail: str = "") -> dict[str, str]:
    return {"packetId": packet.get("packetId", "unknown"), "predicate": predicate, "detail": detail or predicate}


def protected(value: Any, names: Iterable[str] = ("default", "user-browser-profile", "unrelated-tabs", "user active agent/session")) -> str | None:
    if isinstance(value, dict):
        for k, v in value.items():
            found = protected(v, names)
            if found:
                return found
    elif isinstance(value, list):
        for v in value:
            found = protected(v, names)
            if found:
                return found
    elif isinstance(value, str):
        low = value.lower()
        for name in names:
            if name.lower() in low:
                return name
    return None


def resolve_artifact(run: Path, raw: str) -> Path:
    p = Path(raw)
    if not p.is_absolute():
        p = run / p
    return p.resolve()


def artifact_in_run(run: Path, path: Path) -> bool:
    try:
        path.relative_to(run.resolve())
        return True
    except ValueError:
        return False

def png_size(path: Path) -> tuple[int, int] | None:
    try:
        with path.open("rb") as f:
            header = f.read(24)
        if len(header) < 24 or header[:8] != b"\x89PNG\r\n\x1a\n" or header[12:16] != b"IHDR":
            return None
        return int.from_bytes(header[16:20], "big"), int.from_bytes(header[20:24], "big")
    except OSError:
        return None


def _as_list(value: Any) -> list[Any]:
    if isinstance(value, list):
        return value
    if isinstance(value, dict):
        return [value]
    return []


def validate_receipt(packet: dict[str, Any], receipt: dict[str, Any], run: Path,
                     frags: dict[str, dict[str, Any]]) -> list[dict[str, str]]:
    """Validate one immutable receipt without changing its claimed result."""
    errors: list[dict[str, str]] = []
    m = receipt.get("manifest", receipt)
    if not isinstance(m, dict):
        return [packet_error(packet, "manifest.object", "manifest must be an object")]
    required = ["scenarioId", "captureId", "startedAt", "finishedAt", "sourceCommit",
                "runtimeIdentity", "viewportSet", "disposableSession", "browserProfile",
                "fixtureIdentity", "authoritySnapshot", "actionLog", "stateTimeline",
                "artifacts", "result", "cleanup"]
    for field in required:
        if field not in m:
            errors.append(packet_error(packet, f"manifest.{field}", f"missing {field}"))
    expected_commit = frags["coverage-matrix"].get("baselineCommit")
    run_meta = read_run(run, "run.json", {})
    if m.get("scenarioId") not in (packet.get("scenarioId"), packet.get("key"), packet.get("cellId")):
        errors.append(packet_error(packet, "manifest.scenarioId", "does not match packet"))
    if m.get("sourceCommit") != expected_commit:
        errors.append(packet_error(packet, "binding.source-commit", "receipt commit must equal workflow baseline"))
    if run_meta.get("baselineCommit") and run_meta.get("baselineCommit") != expected_commit:
        errors.append(packet_error(packet, "binding.source-commit", "run baseline must equal workflow baseline"))
    if run_meta.get("baselineCommit") and m.get("sourceCommit") != run_meta.get("baselineCommit"):
        errors.append(packet_error(packet, "binding.commit", "receipt commit differs from run baseline"))
    declared_packet = m.get("packetId", receipt.get("packetId"))
    if declared_packet and declared_packet != packet.get("packetId"):
        errors.append(packet_error(packet, "binding.packet", "receipt packet differs from workload packet"))
    if protected(m):
        errors.append(packet_error(packet, "safety.protected-resource", "protected/default resource appears in receipt"))
    started, finished = _iso(m.get("startedAt")), _iso(m.get("finishedAt"))
    for field, value in (("startedAt", started), ("finishedAt", finished)):
        if value is None:
            errors.append(packet_error(packet, f"manifest.{field}.rfc3339", "timestamp must include timezone"))
    if started and finished and finished < started:
        errors.append(packet_error(packet, "manifest.time-order", "finishedAt precedes startedAt"))
    if not isinstance(m.get("captureId"), str) or not re.fullmatch(
            r"[a-z0-9][a-z0-9-]{7,63}", str(m.get("captureId", ""))):
        errors.append(packet_error(packet, "manifest.captureId", "invalid capture id"))

    identity = m.get("runtimeIdentity")
    if not isinstance(identity, dict):
        errors.append(packet_error(packet, "runtimeIdentity.object", "runtime identity is required"))
    else:
        for fld in ("runId", "lane", "owner"):
            if not identity.get(fld):
                errors.append(packet_error(packet, f"runtimeIdentity.{fld}", f"missing {fld}"))
        if identity.get("runId") != run.name:
            errors.append(packet_error(packet, "binding.run", "runtime is not this run"))
        if identity.get("lane") and packet.get("lane") and identity["lane"] != packet["lane"]:
            errors.append(packet_error(packet, "binding.lane", "runtime lane differs from packet lane"))

    states = _as_list(m.get("stateTimeline"))
    phases: dict[str, dict[str, Any]] = {}
    state_ids: set[str] = set()
    for s in states:
        if not isinstance(s, dict):
            errors.append(packet_error(packet, "state.object", "state record must be object"))
            continue
        phase = str(s.get("phase", ""))
        if phase in phases:
            errors.append(packet_error(packet, "state.phase-unique", f"duplicate {phase} state"))
        phases[phase] = s
        state_ids.add(str(s.get("stateId", "")))
        for fld in ("stateId", "phase", "observedAt", "surface", "visibleAssertions",
                    "authorityRefs", "artifactRefs", "result"):
            if fld not in s:
                errors.append(packet_error(packet, f"state.{fld}", "missing state field"))
        if _iso(s.get("observedAt")) is None:
            errors.append(packet_error(packet, "state.observedAt.rfc3339", "state timestamp must include timezone"))
        if not _surface_matches(s.get("surface"), packet):
            errors.append(packet_error(packet, "surface.expected", "state is not on the packet's named surface"))
    for phase in ("before", "action", "after"):
        if phase not in phases:
            errors.append(packet_error(packet, f"state.{phase}", f"required {phase} state is missing"))
    if m.get("result") == "PASS" and any(phase not in phases for phase in ("before", "action", "after")):
        errors.append(packet_error(packet, "result.PASS.complete-timeline", "PASS requires before/action/after"))

    actions_raw = m.get("actionLog")
    actions: list[Any] = []
    action_path: Path | None = None
    if not isinstance(actions_raw, str):
        errors.append(packet_error(packet, "actionLog.jsonl", "action log must be a run-relative JSONL file"))
    else:
        if not actions_raw.lower().endswith(".jsonl"):
            errors.append(packet_error(packet, "actionLog.jsonl", "action log must use .jsonl extension"))
        action_path = resolve_artifact(run, actions_raw)
        if not artifact_in_run(run, action_path):
            errors.append(packet_error(packet, "artifact.escape", "action log escapes run directory"))
        elif not action_path.exists():
            errors.append(packet_error(packet, "actionLog.path", "action log path does not exist"))
        else:
            parsed = _jsonl(action_path)
            if parsed is None:
                errors.append(packet_error(packet, "actionLog.jsonl", "invalid JSONL action log"))
            else:
                actions = parsed
    previous_action_at: datetime | None = None
    for n, action in enumerate(actions, 1):
        if not isinstance(action, dict):
            errors.append(packet_error(packet, "actionLog.object", "action must be object"))
            continue
        if action.get("sequence") != n:
            errors.append(packet_error(packet, "actionLog.sequence-contiguous", f"expected sequence {n}"))
        action_at = _iso(action.get("at"))
        if action_at is None:
            errors.append(packet_error(packet, "actionLog.at.rfc3339", "action timestamp must include timezone"))
        elif previous_action_at and action_at <= previous_action_at:
            errors.append(packet_error(packet, "actionLog.timestamp-order", "action timestamps must be strictly increasing"))
        previous_action_at = action_at or previous_action_at
        for fld in ("sequence", "at", "actor", "surface", "action", "target", "inputs",
                    "expectedAck", "observedAck", "authorityRefs", "artifactRefs", "outcome"):
            if fld not in action:
                errors.append(packet_error(packet, f"actionLog.{fld}", "missing action field"))
        if not action.get("observedAck"):
            errors.append(packet_error(packet, "actionLog.observed-ack", "action acknowledgement was not observed"))
        if not _surface_matches(action.get("surface"), packet):
            errors.append(packet_error(packet, "surface.expected", "action is not on the packet's named surface"))
    if not actions:
        errors.append(packet_error(packet, "actionLog.entries", "at least one action is required"))

    raw_arts = _as_list(m.get("artifacts"))
    arts = [a for a in raw_arts if isinstance(a, dict)]
    if len(arts) != len(raw_arts):
        errors.append(packet_error(packet, "artifact.object", "artifact must be object"))
    artifact_paths: dict[str, dict[str, Any]] = {}
    artifact_ids: dict[str, dict[str, Any]] = {}
    machine_payloads: list[Any] = []
    screenshots: list[tuple[dict[str, Any], Path]] = []
    present_kinds = {str(a.get("kind", "")).lower() for a in arts}
    for required_kind in ("screenshot", "machine-evidence", "action-log"):
        if required_kind not in present_kinds:
            errors.append(packet_error(packet, f"artifact.kind.{required_kind}",
                                        f"required {required_kind} artifact is missing"))
    allowed_classes = frags["evidence-contract"].get(
        "allowedEvidenceClasses",
        frags["evidence-contract"].get("artifactNaming", {}).get("allowedEvidenceClasses", []))
    for art in arts:
        for fld in ("path", "kind", "evidenceClass", "scenarioId", "captureId", "capturedAt",
                    "sourceCommit", "sha256", "provenance"):
            if fld not in art:
                errors.append(packet_error(packet, f"artifact.{fld}", "missing artifact field"))
        raw = art.get("path")
        if not isinstance(raw, str) or not raw:
            errors.append(packet_error(packet, "artifact.path", "artifact path is required"))
            continue
        p = resolve_artifact(run, raw)
        if not artifact_in_run(run, p):
            errors.append(packet_error(packet, "artifact.escape", f"artifact escapes run: {raw}"))
            continue
        artifact_paths[raw] = art
        if art.get("artifactId"):
            artifact_ids[str(art["artifactId"])] = art
        if not p.exists():
            errors.append(packet_error(packet, "artifact.path", f"missing artifact {raw}"))
            continue
        declared_hash = str(art.get("sha256", ""))
        if not HEX64.fullmatch(declared_hash):
            errors.append(packet_error(packet, "artifact.sha256", f"invalid SHA-256 for {raw}"))
        elif file_digest(p) != declared_hash:
            errors.append(packet_error(packet, "artifact.sha256", f"hash mismatch for {raw}"))
        if _iso(art.get("capturedAt")) is None:
            errors.append(packet_error(packet, "artifact.capturedAt.rfc3339", f"timestamp invalid for {raw}"))
        if art.get("scenarioId") != m.get("scenarioId") or art.get("captureId") != m.get("captureId"):
            errors.append(packet_error(packet, "artifact.identity", f"scenario/capture mismatch for {raw}"))
        if art.get("sourceCommit") != m.get("sourceCommit"):
            errors.append(packet_error(packet, "artifact.sourceCommit", f"commit mismatch for {raw}"))
        if art.get("evidenceClass") not in allowed_classes:
            errors.append(packet_error(packet, "artifact.evidenceClass", f"unsupported class for {raw}"))
        if not art.get("provenance"):
            errors.append(packet_error(packet, "artifact.provenance", f"missing provenance for {raw}"))
        provenance = art.get("provenance")
        if isinstance(provenance, dict) and provenance.get("surface") is not None:
            prov_surfaces = provenance.get("surface")
            if isinstance(prov_surfaces, list):
                if not any(_surface_matches(v, packet) for v in prov_surfaces):
                    errors.append(packet_error(packet, "artifact.surface",
                                                f"artifact is not bound to named surface: {raw}"))
            elif not _surface_matches(prov_surfaces, packet):
                errors.append(packet_error(packet, "artifact.surface",
                                            f"artifact is not bound to named surface: {raw}"))
        if art.get("evidenceClass") in ("mock", "static", "historical") and art.get("runtimeClaim", True):
            errors.append(packet_error(packet, "screenshots.runtime-evidence", f"mock/static evidence used as runtime proof: {raw}"))
        kind = str(art.get("kind", "")).lower()
        if kind in {"machine-evidence", "runtime-state", "dom-snapshot", "authority-snapshot"}:
            payload = _json_artifact(p)
            if payload is None:
                errors.append(packet_error(packet, "artifact.json", f"machine artifact is not valid JSON: {raw}"))
            else:
                machine_payloads.append(payload)
        if kind == "screenshot" or raw.lower().endswith(".png"):
            screenshots.append((art, p))
            name = Path(raw).name
            phase_name = Path(raw).name.split("-", 1)[0] if raw else ""
            viewport_name = Path(raw).stem.rsplit("-", 1)[-1] if raw else ""
            if not art.get("stateId"):
                errors.append(packet_error(packet, "screenshot.stateId", f"missing stateId for {raw}"))
            if not art.get("viewport"):
                errors.append(packet_error(packet, "screenshot.viewport", f"missing viewport for {raw}"))
            state_value = str(art.get("stateId", ""))
            state_names = {phase_name}
            if phase_name == "action":
                action_match = re.match(r"^action-([0-9]{2})-", name)
                if action_match:
                    state_names.add("action-" + action_match.group(1))
            if state_value and state_value not in state_names:
                errors.append(packet_error(packet, "screenshot.stateId", f"stateId mismatch for {raw}"))
            if art.get("viewport") and str(art["viewport"]) != viewport_name:
                errors.append(packet_error(packet, "screenshot.viewport", f"viewport mismatch for {raw}"))
            match = SCREEN.fullmatch(name)
            if not match:
                errors.append(packet_error(packet, "screenshot.filename", f"ambiguous screenshot name {name}"))
            else:
                got = png_size(p)
                expected = tuple(map(int, match.group(2).split("x")))
                if got != expected:
                    errors.append(packet_error(packet, "screenshot.viewport-dimensions",
                                                f"{name}: header {got}, expected {expected}"))
            if art.get("evidenceClass") != "runtime":
                errors.append(packet_error(packet, "screenshot.runtime-class", f"{name} is not runtime evidence"))

    # References are intentionally resolved against both paths and optional IDs.
    known_refs = set(artifact_paths) | set(artifact_ids)
    auth_raw = _as_list(m.get("authoritySnapshot"))
    auth_by_id: dict[str, dict[str, Any]] = {}
    for snap in auth_raw:
        if not isinstance(snap, dict):
            errors.append(packet_error(packet, "authority.object", "authority snapshot must be object"))
            continue
        aid = str(snap.get("authorityId", ""))
        if aid:
            auth_by_id[aid] = snap
        for fld in ("authorityId", "surface", "capturedAt", "sourceCommit", "sessionId",
                    "ownedResourceIds", "stateDigest", "sha256", "redaction"):
            if fld not in snap:
                errors.append(packet_error(packet, f"authority.{fld}", "missing authority field"))
        if _iso(snap.get("capturedAt")) is None:
            errors.append(packet_error(packet, "authority.capturedAt.rfc3339", "authority timestamp must include timezone"))
        if snap.get("sourceCommit") != m.get("sourceCommit"):
            errors.append(packet_error(packet, "authority.sourceCommit", "authority commit mismatch"))
        if not _surface_matches(snap.get("surface"), packet):
            errors.append(packet_error(packet, "authority.surface", "authority is not bound to named surface"))
        if snap.get("sessionId") != m.get("disposableSession") and m.get("disposableSession") not in _as_list(snap.get("ownedResourceIds")):
            errors.append(packet_error(packet, "authority.binding", "authority is not bound to disposable session"))
        if not _as_list(snap.get("ownedResourceIds")):
            errors.append(packet_error(packet, "authority.ownership", "owned resource IDs are required"))
        if not HEX64.fullmatch(str(snap.get("sha256", ""))):
            errors.append(packet_error(packet, "authority.sha256", "invalid authority hash"))
    if not auth_raw:
        errors.append(packet_error(packet, "authority.snapshot", "authority snapshot required"))
    required_surfaces = {_norm(s) for s in packet.get("surfaces", []) if _norm(s)}
    covered_surfaces = {_norm(s.get("surface")) for s in auth_raw if isinstance(s, dict)}
    if required_surfaces and not all(any(_surface_matches(s, {"surfaces": [surface]}) for s in covered_surfaces)
                                    for surface in required_surfaces):
        errors.append(packet_error(packet, "authority.cross-surface-provenance", "authority does not cover every packet surface"))

    for collection in (states, actions):
        for row in collection:
            if not isinstance(row, dict):
                continue
            for ref in _as_list(row.get("artifactRefs")):
                if ref not in known_refs:
                    errors.append(packet_error(packet, "reference.artifact", f"unknown artifact reference {ref}"))
            for ref in _as_list(row.get("authorityRefs")):
                if ref not in auth_by_id:
                    errors.append(packet_error(packet, "reference.authority", f"unknown authority reference {ref}"))
    if isinstance(m.get("actionLog"), str) and m["actionLog"] not in known_refs:
        errors.append(packet_error(packet, "reference.action-log", "actionLog must reference a declared artifact"))
    for d in _as_list(m.get("diagnostics")):
        if isinstance(d, dict):
            for field in ("logsOrTraceRefs", "screenshotsRefs", "environmentRefs"):
                for ref in _as_list(d.get(field)):
                    if ref not in known_refs:
                        errors.append(packet_error(packet, "reference.diagnostic", f"unknown diagnostic reference {ref}"))

    names = {Path(str(a.get("path", ""))).name for a, _ in screenshots}
    for vp in packet.get("viewports", []):
        suffix = f"{vp['width']}x{vp['height']}"
        if not any(n.startswith("before-") and n.endswith(f"-{suffix}.png") for n in names):
            errors.append(packet_error(packet, f"screenshot.before.{suffix}", "missing before screenshot"))
        if not any(n.startswith("after-") and n.endswith(f"-{suffix}.png") for n in names):
            errors.append(packet_error(packet, f"screenshot.after.{suffix}", "missing after screenshot"))
    state_actions = [a for a in actions if a.get("stateChanging", True)]
    for idx, _ in enumerate(state_actions, 1):
        for vp in packet.get("viewports", []):
            suffix = f"{vp['width']}x{vp['height']}"
            if not any(n.startswith(f"action-{idx:02d}-") and n.endswith(f"-{suffix}.png") for n in names):
                errors.append(packet_error(packet, f"screenshot.action-{idx:02d}.{suffix}", "missing action screenshot"))
    # A state-changing path must actually produce different bytes at each size.
    for vp in packet.get("viewports", []):
        suffix = f"{vp['width']}x{vp['height']}"
        frames = [(a, file_digest(p)) for a, p in screenshots
                  if Path(str(a.get("path", ""))).name.endswith(f"-{suffix}.png")
                  and (Path(str(a.get("path", ""))).name.startswith(("before-", "after-"))
                       or Path(str(a.get("path", ""))).name.startswith("action-"))]
        if state_actions and len(frames) >= 2 and len({h for _, h in frames}) < len(frames):
            errors.append(packet_error(packet, "screenshot.duplicate-state",
                                        f"state-changing frames are byte-identical at {suffix}"))

    # Machine evidence must bind the receipt to a concrete named runtime surface.
    def machine_values(value: Any, key: str = "") -> list[Any]:
        values = [value] if key in {"surface", "surfaces", "url", "route", "title", "documentTitle", "runtimeUrl"} else []
        if isinstance(value, dict):
            for k, v in value.items():
                values.extend(machine_values(v, str(k)))
        elif isinstance(value, list):
            for v in value:
                values.extend(machine_values(v, key))
        return values
    if not machine_payloads:
        errors.append(packet_error(packet, "runtime.machine-evidence", "runtime machine evidence is required"))
    else:
        observed = [v for payload in machine_payloads for v in machine_values(payload)]
        if not any(_surface_matches(v, packet) for v in observed):
            errors.append(packet_error(packet, "runtime.surface", "runtime evidence names no expected surface"))
        if not any(isinstance(v, str) and
                   (v.startswith("/") or re.match(r"^[a-z][a-z0-9+.-]*://", v))
                   for v in observed):
            errors.append(packet_error(packet, "runtime.url", "runtime URL/route is missing"))
        if any("atlas fixture app" in str(v).lower() for v in observed) and not any(
                _surface_matches(v, packet) for v in observed if isinstance(v, str)):
            errors.append(packet_error(packet, "runtime.fixture-substitution", "generic fixture is not the named surface"))

    result = m.get("result")
    if result not in ("PASS", "FAIL", "INCONCLUSIVE"):
        errors.append(packet_error(packet, "result.state", "result must be PASS, FAIL, or INCONCLUSIVE"))
    diagnostics = _as_list(m.get("diagnostics"))
    cleanup = m.get("cleanup")
    if result in ("FAIL", "INCONCLUSIVE"):
        if not diagnostics:
            errors.append(packet_error(packet, "diagnostics.required", f"{result} requires diagnostics"))
        diagnostic_fields = ("diagnosticId", "at", "category", "symptom", "attempts",
                             "logsOrTraceRefs", "screenshotsRefs", "environmentRefs",
                             "lastKnownState", "ownershipImpact", "nextStep")
        categories = frags["evidence-contract"].get("diagnostics", {}).get("categories", [])
        for d in diagnostics:
            if not isinstance(d, dict):
                errors.append(packet_error(packet, "diagnostics.object", "diagnostic must be object"))
                continue
            for fld in diagnostic_fields:
                if fld not in d:
                    errors.append(packet_error(packet, f"diagnostics.{fld}", "required diagnostic field missing"))
            if categories and d.get("category") not in categories:
                errors.append(packet_error(packet, "diagnostics.category", "unsupported diagnostic category"))
    if result == "INCONCLUSIVE":
        try:
            attempts = int(m.get("attempts", receipt.get("attempts", 0)) or 0)
        except (TypeError, ValueError):
            attempts = 0
        blocker = m.get("blocker") or m.get("capabilityBlocker")
        if attempts < packet.get("retryBudget", 2) and not blocker:
            errors.append(packet_error(packet, "result.INCONCLUSIVE.retry-exhausted",
                                        "INCONCLUSIVE before retry budget or named blocker"))
    if not isinstance(cleanup, dict) or cleanup.get("completed") is not True or cleanup.get("ownershipVerified") is not True:
        errors.append(packet_error(packet, "cleanup.completed", "cleanup must be completed with ownership verified"))
    if isinstance(cleanup, dict) and cleanup.get("residuals"):
        errors.append(packet_error(packet, "cleanup.residuals", "unresolved cleanup residuals"))
    if result == "PASS" and diagnostics:
        unresolved = [d for d in diagnostics if isinstance(d, dict) and d.get("resolved") is not True]
        if unresolved:
            errors.append(packet_error(packet, "result.PASS.diagnostics", "PASS has unresolved diagnostics"))
    return errors


def packet_records(run: Path) -> list[dict[str, Any]]:
    return read_run(run, "submissions.json", [])


def _repair_packet_id(packet_id: str, supersedes: str, attempt: int) -> str:
    key = f"{packet_id}:{supersedes}:{attempt}"
    return "repair-packet-" + hashlib.sha256(key.encode()).hexdigest()[:16]


def _repair_id(packet_id: str, supersedes: str, attempt: int) -> str:
    key = f"{packet_id}:{supersedes}:{attempt}"
    return "repair-" + hashlib.sha256(key.encode()).hexdigest()[:16]


def _repair_packet(repair: dict[str, Any], packet: dict[str, Any]) -> dict[str, Any]:
    """Expose a repair as a packet while retaining the original packet identity."""
    result = dict(packet)
    result.update({
        "packetId": repair.get("repairPacketId") or repair.get("packetId"),
        "originalPacketId": packet.get("packetId"),
        "repairId": repair.get("repairId"),
        "repairPredicates": repair.get("predicates", []),
        "supersedes": repair.get("supersedes"),
        "repairAttempt": repair.get("attempt"),
        "status": "REPAIR",
    })
    return result


def _repair_records(run: Path) -> list[dict[str, Any]]:
    return [r for r in read_run(run, "repairs.json", []) if isinstance(r, dict)]


def _repair_for_packet(run: Path, packet_id: str) -> dict[str, Any] | None:
    rows = [r for r in _repair_records(run)
            if r.get("repairPacketId") == packet_id or
            (r.get("repairPacketId") is None and r.get("packetId") == packet_id)]
    return rows[-1] if rows else None
def _repair_completed_by_submission(repair: dict[str, Any],
                                    submission: dict[str, Any] | None) -> bool:
    """Treat a READY row as completed when a valid attempt supersedes its target."""
    if not submission or submission.get("auditValid") is not True:
        return False
    target = str(repair.get("supersedes") or "")
    return bool(target) and str(submission.get("supersedes") or "") == target




def _base_packet_id(submission: dict[str, Any]) -> str:
    return str(submission.get("originalPacketId") or submission.get("packetId") or "")


def _queue_followup_repair(run: Path, packet: dict[str, Any], failed: dict[str, Any]) -> dict[str, Any] | None:
    budget = int(packet.get("retryBudget", 2) or 0)
    rows = _repair_records(run)
    for index, row in enumerate(rows):
        if row.get("repairId") == failed.get("repairId"):
            rows[index] = dict(failed)
    attempts = [r for r in rows if str(r.get("originalPacketId") or r.get("packetId")) == packet["packetId"]]
    next_attempt = max([int(r.get("attempt", 0) or 0) for r in attempts] + [0]) + 1
    if next_attempt > budget:
        atomic_write(run / "repairs.json", rows)
        return None
    supersedes = str(failed.get("receiptSha256") or failed.get("supersedes") or "")
    predicates = failed.get("predicates") or (
        [{"predicate": failed["predicate"], "detail": failed.get("reason", "")}]
        if failed.get("predicate") else [])
    repair_id = _repair_id(packet["packetId"], supersedes, next_attempt)
    repair_packet_id = _repair_packet_id(packet["packetId"], supersedes, next_attempt)
    row = {
        "repairId": repair_id, "repairPacketId": repair_packet_id,
        "packetId": packet["packetId"], "originalPacketId": packet["packetId"],
        "cellId": packet.get("cellId"), "scenarioId": packet.get("scenarioId"),
        "surfaceTokens": sorted(expected_surface_tokens(packet)),
        "predicates": sorted(predicates, key=lambda x: (x.get("predicate", ""), x.get("detail", ""))),
        "predicate": (predicates[0].get("predicate") if predicates else ""),
        "reason": "; ".join(str(x.get("detail", "")) for x in predicates),
        "supersedes": supersedes, "attempt": next_attempt, "createdAt": now(), "status": "READY",
        "packet": {"packetId": packet["packetId"], "scenarioId": packet.get("scenarioId"),
                   "cellId": packet.get("cellId"), "surfaces": packet.get("surfaces", []),
                   "viewports": packet.get("viewports", [])},
    }
    rows.append(row)
    atomic_write(run / "repairs.json", rows)
    return row

def cmd_init(args: argparse.Namespace) -> int:
    root = Path(args.root).resolve()
    frags = load_fragments(root)
    packets = expand_packets(frags)
    run_id = args.run_id or datetime.now(timezone.utc).strftime("run-%Y%m%dT%H%M%SZ")
    run = run_path(root, run_id)
    if run.exists() and any(run.iterdir()):
        raise ValueError("run-already-exists")
    run.mkdir(parents=True, exist_ok=True)
    baseline = frags["coverage-matrix"]["baselineCommit"]
    atomic_write(run / "run.json", {"runId": run_id, "createdAt": now(), "baselineCommit": baseline,
                                     "fragments": list(FRAGMENTS), "dryRunFixture": bool(args.dry_run_fixture), "state": "OPEN"})
    atomic_write(run / "workload.json", {"packets": packets, "packetCount": len(packets), "workloadDigest": digest(packets)})
    atomic_write(run / "resource-ledger.json", {"runId": run_id, "owner": frags["environment"]["ownership"]["owner"],
                                                  "protected": ["default", "user-browser-profile", "unrelated-tabs"], "lanes": {}, "resources": []})
    atomic_write(run / "claims.json", [])
    atomic_write(run / "submissions.json", [])
    atomic_write(run / "repairs.json", [])
    if args.dry_run_fixture and packets:
        atomic_write(run / "dry-run-fixture" / "incomplete-receipt.json", {
            "packetId": packets[0]["packetId"], "scenarioId": packets[0]["scenarioId"], "captureId": "dry-run-0001",
            "sourceCommit": baseline, "result": "PASS", "artifacts": [], "stateTimeline": [{"phase": "before"}],
            "runtimeIdentity": {"runId": run_id, "lane": packets[0]["lane"], "owner": "dry-run"},
        })
    atomic_write(run / "rejections.json", [])
    return emit(args, {"ok": True, "command": "init", "runId": run_id, "packetCount": len(packets),
                       "workloadDigest": digest(packets), "dryRunFixture": bool(args.dry_run_fixture)})


def get_run(args: argparse.Namespace) -> tuple[Path, dict[str, dict[str, Any]]]:
    root = Path(args.root).resolve()
    if not args.run_id:
        raise ValueError("--run-id is required")
    run = run_path(root, args.run_id)
    if not run.is_dir():
        raise ValueError("run-not-found")
    return run, load_fragments(root)
def cmd_status(args: argparse.Namespace) -> int:
    run, _ = get_run(args)
    packets = read_run(run, "workload.json", {}).get("packets", [])
    submissions = packet_records(run)
    invalid_ids = {r.get("packetId") for r in read_run(run, "audit-invalid.json", {}).get("submissions", [])}
    latest: dict[str, dict[str, Any]] = {}
    for row in submissions:
        latest[_base_packet_id(row)] = row
    by: dict[str, dict[str, Any]] = {}
    for pid, row in latest.items():
        if row.get("auditValid") is False or (
                row.get("auditValid") is not True and pid in invalid_ids):
            by[pid] = {"result": "INCONCLUSIVE"}
        else:
            by[pid] = row
    counts = {s: 0 for s in ("PENDING", "CLAIMED", "PASS", "FAIL", "INCONCLUSIVE")}
    claims = read_run(run, "claims.json", [])
    for p in packets:
        state = by.get(p["packetId"], {}).get("result", "PENDING")
        if state == "PENDING" and any(c.get("originalPacketId", c.get("packetId")) == p["packetId"]
                                      for c in claims):
            state = "CLAIMED"
        counts[state] = counts.get(state, 0) + 1
    repair_rows = _repair_records(run)
    repair_ready = [
        r for r in repair_rows
        if r.get("status") in ("READY", "CLAIMED")
        and not _repair_completed_by_submission(
            r, latest.get(str(r.get("originalPacketId") or r.get("packetId") or "")))
    ]
    return emit(args, {"ok": True, "runId": run.name, "packetCount": len(packets), "counts": counts,
                       "claims": len(claims), "repairs": len(repair_rows),
                       "repairReadyCount": len(repair_ready), "repairPackets": repair_ready})


def ready_packets(run: Path) -> tuple[list[dict[str, Any]], list[dict[str, str]]]:
    packets = read_run(run, "workload.json", {}).get("packets", [])
    submissions = packet_records(run)
    claims = read_run(run, "claims.json", [])
    invalid_ids = {r.get("packetId") for r in read_run(run, "audit-invalid.json", {}).get("submissions", [])}
    latest: dict[str, dict[str, Any]] = {}
    for row in submissions:
        latest[_base_packet_id(row)] = row
    claimed = {c.get("packetId") for c in claims}
    packet_by_id = {p.get("packetId"): p for p in packets}
    out, blockers = [], []
    terminal = {pid for pid, row in latest.items()
                if row.get("result") in ("PASS", "FAIL", "INCONCLUSIVE")
                and row.get("auditValid", True) is not False
                and (row.get("auditValid") is True or pid not in invalid_ids)}
    grouped: dict[str, dict[str, Any]] = {}
    for row in _repair_records(run):
        original = str(row.get("originalPacketId") or row.get("packetId") or "")
        if (not original or original not in packet_by_id
                or row.get("status") not in ("READY", "CLAIMED")
                or _repair_completed_by_submission(row, latest.get(original))):
            continue
        grouped[original] = row
    for original, repair in grouped.items():
        base = packet_by_id[original]
        packet = _repair_packet(repair, base)
        packet_id = packet["packetId"]
        if packet_id in claimed:
            blockers.append(packet_error(packet, "claim.exclusive", "packet is already claimed"))
        else:
            out.append(packet)

    bootstrap = (run / "bootstrap" / "receipt.json").exists()
    fixture = (run / "fixture" / "receipt.json").exists()
    for p in packets:
        pid = p["packetId"]
        if pid in terminal or pid in grouped:
            continue
        if pid in claimed:
            blockers.append(packet_error(p, "claim.exclusive", "packet is already claimed"))
            continue
        if p.get("phase") == "bootstrap":
            out.append(p); continue
        if not bootstrap:
            blockers.append(packet_error(p, "phase.bootstrap", "bootstrap receipt required")); continue
        if p.get("phase") == "fixture":
            out.append(p); continue
        if not fixture:
            blockers.append(packet_error(p, "phase.fixture", "fixture receipt required")); continue
        out.append(p)
    return out, blockers


def cmd_ready(args: argparse.Namespace) -> int:
    run, _ = get_run(args)
    ready, blockers = ready_packets(run)
    return emit(args, {"ok": True, "runId": run.name, "ready": ready, "readyCount": len(ready),
                       "blockers": blockers, "blockerCount": len(blockers)})


def _claim_packet(run: Path, packet_id: str, claims: list[dict[str, Any]]) -> dict[str, Any] | None:
    for claim in claims:
        if claim.get("packetId") == packet_id:
            return claim
    return None


def cmd_claim(args: argparse.Namespace) -> int:
    run, _ = get_run(args)
    ready, _ = ready_packets(run)
    claims = read_run(run, "claims.json", [])
    packet = next((p for p in ready if not args.packet or p["packetId"] == args.packet), None)
    if packet is None:
        return emit(args, {"ok": False, "error": packet_error(
            {"packetId": args.packet or ""}, "claim.not-ready", "packet is not ready")}, 2)
    namespace = args.resource_namespace or f"{run.name}-{packet['lane']}-{args.worker}"
    namespace_claim = next((c for c in claims
                            if c.get("resourceNamespace") == namespace
                            and c.get("packetId") != packet["packetId"]), None)
    if namespace_claim:
        return emit(args, {"ok": False, "error": packet_error(
            packet, "claim.resource-namespace", "resource namespace is already claimed")}, 2)
    claim = {"packetId": packet["packetId"], "originalPacketId": packet.get("originalPacketId", packet["packetId"]),
             "repairId": packet.get("repairId"), "supersedes": packet.get("supersedes"),
             "repairPredicates": packet.get("repairPredicates", []),
             "lane": packet["lane"], "worker": args.worker,
             "resourceNamespace": namespace, "claimedAt": now(),
             "claimId": "claim-" + hashlib.sha256(
                 f"{run.name}:{packet['packetId']}:{args.worker}".encode()).hexdigest()[:16]}
    claims = [c for c in claims if c.get("packetId") != packet["packetId"]] + [claim]
    atomic_write(run / "claims.json", claims)
    ledger = read_run(run, "resource-ledger.json", {})
    lanes = ledger.setdefault("lanes", {})
    lane_entry = lanes.setdefault(packet["lane"], {"owners": {}})
    lane_entry.setdefault("owners", {})[packet["packetId"]] = {
        "worker": args.worker, "resourceNamespace": claim["resourceNamespace"],
        "claimId": claim["claimId"]}
    atomic_write(run / "resource-ledger.json", ledger)
    return emit(args, {"ok": True, "claim": claim, "packet": packet})


def validate_phase_receipt(packet: dict[str, Any], receipt: dict[str, Any],
                           run: Path, phase: str) -> list[dict[str, str]]:
    errors: list[dict[str, str]] = []
    if receipt.get("phase") != phase:
        errors.append(packet_error(packet, f"phase.{phase}", f"receipt phase must be {phase}"))
    if receipt.get("result") not in ("PASS", "FAIL", "INCONCLUSIVE"):
        errors.append(packet_error(packet, f"phase.{phase}.result", "phase result is required"))
    expected_commit = read_run(run, "run.json", {}).get("baselineCommit")
    receipt_commit = receipt.get("sourceCommit", receipt.get("baselineCommit"))
    if not receipt_commit or receipt_commit != expected_commit:
        errors.append(packet_error(packet, f"phase.{phase}.sourceCommit",
                                    "phase receipt must bind to this run baseline"))
    identity = receipt.get("runtimeIdentity", {})
    if not isinstance(identity, dict) or identity.get("runId") != run.name:
        errors.append(packet_error(packet, f"phase.{phase}.ownership",
                                    "receipt is not owned by this run"))
    elif identity.get("lane") and identity["lane"] != phase:
        errors.append(packet_error(packet, f"phase.{phase}.ownership",
                                    "receipt lane does not match phase"))
    if not receipt.get("resources") and phase == "bootstrap":
        errors.append(packet_error(packet, f"phase.{phase}.resources",
                                    "owned resource inventory is required"))
    if protected(receipt):
        errors.append(packet_error(packet, "safety.protected-resource",
                                    "protected/default resource appears in receipt"))
    if receipt.get("result") in ("FAIL", "INCONCLUSIVE") and not receipt.get("diagnostics"):
        errors.append(packet_error(packet, f"phase.{phase}.diagnostics",
                                    "failure requires diagnostics"))
    return errors


def cmd_submit(args: argparse.Namespace) -> int:
    run, frags = get_run(args)
    packets = read_run(run, "workload.json", {}).get("packets", [])
    packet_id = args.packet
    repair = _repair_for_packet(run, packet_id)
    base_id = str(repair.get("originalPacketId") or repair.get("packetId")) if repair else packet_id
    base = next((p for p in packets if p["packetId"] == base_id), None)
    if not base:
        return emit(args, {"ok": False, "error": packet_error({"packetId": packet_id}, "packet.exists", "unknown packet")}, 2)
    if repair and getattr(args, "repair_id", None) and repair.get("repairId") != args.repair_id:
        return emit(args, {"ok": False, "error": packet_error(base, "repair.identity", "repair id does not match packet")}, 2)
    packet = _repair_packet(repair, base) if repair else base
    claims = read_run(run, "claims.json", [])
    claim = _claim_packet(run, packet_id, claims)
    if not claim or (args.worker and claim.get("worker") != args.worker):
        return emit(args, {"ok": False, "error": packet_error(packet, "claim.ownership", "packet is not claimed by worker")}, 2)
    try:
        receipt = load_json(Path(args.receipt).resolve())
    except ValueError as e:
        return emit(args, {"ok": False, "error": packet_error(packet, "receipt.json", str(e))}, 2)
    errors = (validate_phase_receipt(packet, receipt, run, base["phase"])
              if base.get("phase") in ("bootstrap", "fixture")
              else validate_receipt(packet, receipt, run, frags))
    subs = packet_records(run)
    attempt = len([s for s in subs if _base_packet_id(s) == base_id]) + 1
    receipt_path = run / "receipts" / f"{packet_id}-attempt-{attempt}.json"
    atomic_write(receipt_path, receipt)
    result = receipt.get("result", receipt.get("manifest", {}).get("result"))
    submission = {
        "packetId": packet_id, "originalPacketId": base_id, "worker": claim["worker"],
        "submittedAt": now(), "result": result, "receipt": receipt,
        "receiptPath": str(receipt_path.relative_to(run)),
        "receiptSha256": digest(receipt), "attempt": attempt,
        "auditValid": False if errors else True,
    }
    if repair:
        submission.update({"repairId": repair.get("repairId"), "repairPredicates": repair.get("predicates", []),
                           "supersedes": repair.get("supersedes")})
    if errors:
        # Rejected repair attempts are still immutable attempts in the history.
        if repair:
            submission["errors"] = errors
        else:
            rejects = read_run(run, "rejections.json", [])
            rejects.append({"packetId": base_id, "worker": claim["worker"], "submittedAt": now(),
                            "attempt": len(rejects) + 1, "receipt": receipt,
                            "receiptSha256": digest(receipt), "errors": errors})
            atomic_write(run / "rejections.json", rejects)
        subs.append(submission)
        atomic_write(run / "submissions.json", subs)
        claims = [c for c in claims if c.get("packetId") != packet_id]
        atomic_write(run / "claims.json", claims)
        if repair:
            repair["status"] = "FAILED"
            _queue_followup_repair(run, base, repair)
        return emit(args, {"ok": False, "accepted": False, "packetId": packet_id,
                           "repairId": repair.get("repairId") if repair else None, "errors": errors}, 2)

    if base.get("phase") in ("bootstrap", "fixture"):
        atomic_write(run / base["phase"] / "receipt.json", receipt)
    subs.append(submission)
    atomic_write(run / "submissions.json", subs)
    claims = [c for c in claims if c.get("packetId") != packet_id]
    atomic_write(run / "claims.json", claims)

    if repair:
        # A structural pass is not terminal until the complete audit sees it.
        _, invalid = _audit_records(run, frags)
        if any(row.get("packetId") == base_id for row in invalid):
            submission["auditValid"] = False
            atomic_write(run / "submissions.json", subs)
            repair["status"] = "FAILED"
            _queue_followup_repair(run, base, repair)
            return emit(args, {"ok": False, "accepted": False, "packetId": packet_id,
                               "repairId": repair.get("repairId"), "errors":
                               next((r.get("predicates", []) for r in invalid if r.get("packetId") == base_id), [])}, 2)
        repair["status"] = "COMPLETED"
        atomic_write(run / "repairs.json", _repair_records(run))
    return emit(args, {"ok": True, "accepted": True, "packetId": packet_id,
                       "repairId": repair.get("repairId") if repair else None,
                       "result": submission["result"], "receiptSha256": submission["receiptSha256"]})


def _audit_records(run: Path, frags: dict[str, dict[str, Any]]) -> tuple[
        list[dict[str, str]], list[dict[str, Any]]]:
    packets = read_run(run, "workload.json", {}).get("packets", [])
    packet_by_id = {p.get("packetId"): p for p in packets}
    issues: list[dict[str, str]] = []
    invalid: list[dict[str, Any]] = []
    seen_hashes: dict[str, str] = {}
    submissions = packet_records(run)
    latest_index: dict[str, int] = {}
    for idx, row in enumerate(submissions):
        latest_index[_base_packet_id(row)] = idx
    for submission_index, submission in enumerate(submissions):
        base_id = _base_packet_id(submission)
        base = packet_by_id.get(base_id)
        if not base:
            issues.append(packet_error({"packetId": submission.get("packetId", "unknown")},
                                       "submission.packet", "submission references unknown packet"))
            continue
        p = base
        if submission.get("packetId") != base_id:
            p = dict(base)
            p["packetId"] = submission.get("packetId")
        receipt = submission.get("receipt", {})
        if submission.get("receiptPath"):
            stored = resolve_artifact(run, str(submission["receiptPath"]))
            if not artifact_in_run(run, stored) or not stored.exists():
                errs = [packet_error(p, "submission.immutable", "stored receipt is missing or escapes run")]
            else:
                try:
                    stored_receipt = load_json(stored)
                    errs = ([packet_error(p, "submission.immutable", "stored receipt hash changed")]
                            if submission.get("receiptSha256") != digest(stored_receipt) else [])
                    receipt = stored_receipt
                except ValueError:
                    errs = [packet_error(p, "submission.immutable", "stored receipt is invalid")]
        else:
            errs = []
        if not errs:
            errs = (validate_phase_receipt(p, receipt, run, base["phase"])
                    if base.get("phase") in ("bootstrap", "fixture")
                    else validate_receipt(p, receipt, run, frags))
        # Audit predicates are addressed to the original packet, while the
        # submission retains the generated repair packet id.
        errs = [dict(e, packetId=base_id) for e in errs]
        if errs:
            # Historical attempts are audited for immutable integrity, but
            # only the newest attempt for a packet contributes current
            # blockers.  Older failures remain in submissions.json history.
            if submission_index == latest_index.get(base_id):
                issues.extend(errs)
            if submission.get("result") == "PASS":
                invalid.append({"packetId": base_id, "originalResult": "PASS",
                                "status": "AUDIT_INVALID", "predicates": errs,
                                "receiptSha256": submission.get("receiptSha256"),
                                "submissionIndex": submission_index,
                                "submissionPacketId": submission.get("packetId")})
        # Cross-packet reuse is checked from declared screenshot hashes only
        # after the receipt itself has been safely parsed.
        manifest = receipt.get("manifest", receipt) if isinstance(receipt, dict) else {}
        for art in _as_list(manifest.get("artifacts")):
            if not isinstance(art, dict) or str(art.get("kind", "")).lower() != "screenshot":
                continue
            sha = str(art.get("sha256", ""))
            if not HEX64.fullmatch(sha):
                continue
            other = seen_hashes.get(sha)
            if other and other != base_id:
                issue = packet_error(base, "screenshot.cross-packet-reuse",
                                     f"screenshot hash is already used by {other}")
                if submission_index == latest_index.get(base_id):
                    issues.append(issue)
                if submission.get("result") == "PASS":
                    invalid.append({"packetId": base_id, "originalResult": "PASS",
                                    "status": "AUDIT_INVALID", "predicates": [issue],
                                    "receiptSha256": submission.get("receiptSha256"),
                                    "submissionIndex": submission_index,
                                    "submissionPacketId": submission.get("packetId")})
            else:
                seen_hashes[sha] = base_id
    submitted_ids = {_base_packet_id(s) for s in submissions}
    for p in packets:
        if p["packetId"] not in submitted_ids:
            issues.append(packet_error(p, "coverage.terminal", "no terminal receipt"))
    for rejection in read_run(run, "rejections.json", []):
        p = packet_by_id.get(rejection.get("packetId"))
        if p and p["packetId"] not in submitted_ids:
            for error in rejection.get("errors", []):
                if isinstance(error, dict):
                    issues.append(packet_error(p, str(error.get("predicate", "evidence.rejected")),
                                               str(error.get("detail", ""))))
    # Stable order and no repeated repair work for the same predicate.  Only
    # the newest attempt can invalidate current coverage; older invalid
    # receipts remain in submissions.json as immutable history.
    unique: dict[tuple[str, str], dict[str, str]] = {}
    for issue in issues:
        unique[(issue["packetId"], issue["predicate"])] = issue
    invalid_by_packet: dict[str, dict[str, Any]] = {}
    for row in invalid:
        if row.get("submissionIndex") != latest_index.get(row["packetId"]):
            continue
        current = invalid_by_packet.setdefault(row["packetId"], {
            "packetId": row["packetId"], "originalResult": "PASS",
            "status": "AUDIT_INVALID", "predicates": [],
            "receiptSha256": row.get("receiptSha256")})
        current["predicates"].extend(row.get("predicates", []))
    for row in invalid_by_packet.values():
        pred = {(x.get("predicate"), x.get("detail")) for x in row["predicates"]}
        row["predicates"] = [{"packetId": row["packetId"], "predicate": p, "detail": d}
                             for p, d in sorted(pred)]
    return list(unique.values()), list(invalid_by_packet.values())


def _issues(run: Path, frags: dict[str, dict[str, Any]]) -> list[dict[str, str]]:
    return _audit_records(run, frags)[0]


def cmd_audit(args: argparse.Namespace) -> int:
    run, frags = get_run(args)
    issues, invalid = _audit_records(run, frags)
    ready, blockers = ready_packets(run)
    all_issues = issues + blockers
    report = {"runId": run.name, "generatedAt": now(), "issues": all_issues,
              "auditInvalid": invalid, "readyCount": len(ready),
              "status": "PASS" if not all_issues else "BLOCKED"}
    # Original submissions/receipts remain untouched. This sidecar is the
    # reclassification record consumed by operators and repair.
    atomic_write(run / "audit-invalid.json", {"runId": run.name, "generatedAt": report["generatedAt"],
                                               "submissions": invalid})
    atomic_write(run / "audit.json", report)
    return emit(args, {"ok": True, **report})


def cmd_repair(args: argparse.Namespace) -> int:
    run, frags = get_run(args)
    packets = read_run(run, "workload.json", {}).get("packets", [])
    existing = _repair_records(run)
    issues, invalid = _audit_records(run, frags)
    by_id = {p["packetId"]: p for p in packets}
    latest: dict[str, dict[str, Any]] = {}
    for row in packet_records(run):
        latest[_base_packet_id(row)] = row
    invalid_by_id = {row["packetId"]: row for row in invalid}
    created: list[dict[str, Any]] = []
    for p in packets:
        current = latest.get(p["packetId"])
        current_predicates = [i for i in issues
                              if i.get("packetId") == p["packetId"]
                              and i.get("predicate") != "coverage.terminal"]
        # A valid terminal receipt is never superseded.  An unmarked legacy
        # terminal result is eligible when the current audit found a
        # predicate (for example cross-packet reuse).
        if current and not current_predicates and p["packetId"] not in invalid_by_id:
            continue
        predicates = list(invalid_by_id.get(p["packetId"], {}).get("predicates", []))
        if not predicates:
            predicates = [i for i in issues if i.get("packetId") == p["packetId"]
                          and i.get("predicate") != "coverage.terminal"]
        unique = {(x.get("predicate"), x.get("detail")): x for x in predicates if isinstance(x, dict)}
        predicates = [unique[k] for k in sorted(unique)]
        if not predicates:
            continue
        prior = [r for r in existing
                 if str(r.get("originalPacketId") or r.get("packetId")) == p["packetId"]]
        if any(r.get("status") in ("READY", "CLAIMED")
               and not _repair_completed_by_submission(r, current)
               for r in prior):
            continue
        attempt = max([int(r.get("attempt", 0) or 0) for r in prior] + [0]) + 1
        if attempt > int(p.get("retryBudget", 2) or 0):
            continue
        supersedes = str(invalid_by_id.get(p["packetId"], {}).get("receiptSha256")
                         or (current or {}).get("receiptSha256") or "")
        repair = {
            "repairId": _repair_id(p["packetId"], supersedes, attempt),
            "repairPacketId": _repair_packet_id(p["packetId"], supersedes, attempt),
            "packetId": p["packetId"], "originalPacketId": p["packetId"],
            "cellId": p.get("cellId"), "scenarioId": p.get("scenarioId"),
            "surfaceTokens": sorted(expected_surface_tokens(p)),
            "predicates": predicates, "predicate": predicates[0].get("predicate"),
            "reason": "; ".join(str(x.get("detail", "")) for x in predicates),
            "supersedes": supersedes, "attempt": attempt, "createdAt": now(), "status": "READY",
            "packet": {"packetId": p["packetId"], "scenarioId": p.get("scenarioId"),
                       "cellId": p.get("cellId"), "surfaces": p.get("surfaces", []),
                       "viewports": p.get("viewports", [])},
        }
        existing.append(repair)
        created.append(repair)
    atomic_write(run / "repairs.json", existing)
    atomic_write(run / "repair-log.json", {"runId": run.name, "attempts": existing,
                                           "invalidated": invalid,
                                           "termination": "budget" if invalid and not created else "open"})
    return emit(args, {"ok": True, "repairs": created, "repairCount": len(created), "issues": issues,
                       "auditInvalid": invalid})


def cmd_validate(args: argparse.Namespace) -> int:
    run, frags = get_run(args)
    packets = read_run(run, "workload.json", {}).get("packets", [])
    subs = packet_records(run)
    errors, invalid = _audit_records(run, frags)
    by: dict[str, list[dict[str, Any]]] = {}
    for s in subs:
        by.setdefault(_base_packet_id(s), []).append(s)
    run_meta = read_run(run, "run.json", {})
    if run_meta.get("dryRunFixture"):
        fixture_path = run / "dry-run-fixture" / "incomplete-receipt.json"
        if fixture_path.exists():
            dry_receipt = load_json(fixture_path)
            dry_packet = next((p for p in packets if p["packetId"] == dry_receipt.get("packetId")), None)
            if dry_packet:
                errors.extend(validate_receipt(dry_packet, dry_receipt, run, frags))
    invalid_ids = {row["packetId"] for row in invalid}
    coverage = []
    for p in packets:
        rows = by.get(p["packetId"], [])
        valid_rows = [s for s in rows if s.get("auditValid", True) is not False]
        if p["packetId"] in invalid_ids:
            if rows and rows[-1].get("auditValid") is False:
                valid_rows = [s for s in rows[:-1] if s.get("auditValid", True) is not False]
            else:
                valid_rows = []
        s = valid_rows[-1] if valid_rows else None
        if not s:
            coverage.append({"packetId": p["packetId"], "scenarioId": p["scenarioId"],
                             "cellId": p["cellId"], "result": None})
            continue
        coverage.append({"packetId": p["packetId"], "scenarioId": p["scenarioId"],
                         "cellId": p["cellId"], "result": s.get("result")})
    atomic_write(run / "audit-invalid.json", {"runId": run.name, "generatedAt": now(),
                                               "submissions": invalid})
    atomic_write(run / "coverage.json", {"runId": run.name, "generatedAt": now(), "cells": coverage,
                                         "counts": {r: sum(1 for c in coverage if c.get("result") == r)
                                                    for r in ("PASS", "FAIL", "INCONCLUSIVE")}})
    terminal_count = sum(1 for c in coverage if c.get("result") is not None)
    ok = not errors and terminal_count == len(packets)
    report = {"runId": run.name, "generatedAt": now(), "status": "PASS" if ok else "FAIL",
              "errors": errors, "auditInvalid": invalid, "packetCount": len(packets),
              "terminalCount": terminal_count}
    atomic_write(run / "validation.json", report)
    return emit(args, {"ok": ok, **report}, 0 if ok else 2)


def emit(args: argparse.Namespace, value: dict[str, Any], code: int = 0) -> int:
    if args.json:
        print(json.dumps(value, sort_keys=True, indent=2))
    else:
        if value.get("ok") is False: print("BLOCKED: " + json.dumps(value.get("error", value.get("errors", [])), sort_keys=True))
        elif "readyCount" in value: print(f"ready packets: {value['readyCount']}  blockers: {value.get('blockerCount', 0)}")
        elif "packetCount" in value: print(f"packets: {value['packetCount']}  run: {value.get('runId', '')}")
        elif "repairs" in value: print(f"repair packets: {value.get('repairCount', 0)}")
        else: print("ok")
    return code


def parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(description="Fail-closed browser-first Product Atlas workflow engine")
    p.add_argument("--root", default=str(HERE), help="workflow directory (default: this directory)")
    p.add_argument("--run-id", help="named run directory")
    p.add_argument("--json", action="store_true", help="machine-readable JSON output")
    sub = p.add_subparsers(dest="command", required=True)

    def command(name: str, **kwargs: Any) -> argparse.ArgumentParser:
        child = sub.add_parser(name, **kwargs)
        # Accept the global switches after the command too.  SUPPRESS keeps a
        # child parser from overwriting values supplied before the command.
        child.add_argument("--root", default=argparse.SUPPRESS, help=argparse.SUPPRESS)
        child.add_argument("--run-id", default=argparse.SUPPRESS, help=argparse.SUPPRESS)
        child.add_argument("--json", action="store_true", default=argparse.SUPPRESS, help=argparse.SUPPRESS)
        return child
    i = command("init", help="cross-validate fragments and create a run workload")
    i.add_argument("--dry-run-fixture", action="store_true")
    command("status", help="show packet and claim status")
    command("ready", help="show deterministic ready packets and blockers")
    c = command("claim", help="claim one packet exclusively")
    c.add_argument("--worker", required=True); c.add_argument("--packet"); c.add_argument("--resource-namespace")
    s = command("submit", help="ingest and validate an immutable worker receipt")
    s.add_argument("--packet", required=True); s.add_argument("--receipt", required=True); s.add_argument("--worker"); s.add_argument("--repair-id")
    command("audit", help="emit audit packet and blockers")
    command("repair", help="generate targeted repair packets")
    command("validate", help="validate receipts and coverage")
    return p

def main(argv: list[str] | None = None) -> int:
    args = parser().parse_args(argv)
    try:
        return {"init": cmd_init, "status": cmd_status, "ready": cmd_ready, "claim": cmd_claim, "submit": cmd_submit,
                "audit": cmd_audit, "repair": cmd_repair, "validate": cmd_validate}[args.command](args)
    except (ValueError, OSError) as e:
        return emit(args, {"ok": False, "error": {"predicate": "engine.input", "detail": str(e)}}, 2)

if __name__ == "__main__":
    raise SystemExit(main())
