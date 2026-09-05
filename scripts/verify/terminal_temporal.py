#!/usr/bin/env python3
"""Bounded capture/replay and trace verification for G01 terminal evidence.

The capture path records only an explicitly owned X11 display.  Replay decodes
that recording through :func:`temporal_detector.load_samples`, preserving the
recorded presentation timestamps and retaining only requested ROIs.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import re
import selectors
import shutil
import signal
import subprocess
import sys
import time
from typing import Any, Iterable, Mapping, Sequence

try:
    from .resource_guard import ResourceGuardError, load_ledger, validate_target, validate_recorded_executable
    from .temporal_detector import (
        MAX_FRAMES,
        DetectorConfig,
        DetectorError,
        FrameSample,
        MarkerROI,
        analyze_frames,
        load_samples,
        probe_video,
    )
except ImportError:  # Direct invocation from scripts/verify.
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    from resource_guard import ResourceGuardError, load_ledger, validate_target, validate_recorded_executable  # type: ignore[no-redef]
    from temporal_detector import MAX_FRAMES, DetectorConfig, DetectorError, FrameSample, MarkerROI, analyze_frames, load_samples, probe_video  # type: ignore[no-redef]


MAX_TRACE_BYTES = 32 * 1024 * 1024
MAX_REPORT_BYTES = 16 * 1024 * 1024
MAX_VIDEO_BYTES = 2 * 1024 * 1024 * 1024
MAX_CAPTURE_SECONDS = 180.0
MAX_CAPTURE_FRAMES = MAX_FRAMES
SHA256_RE = re.compile(r"^[0-9a-fA-F]{64}$")
DISPLAY_RE = re.compile(r"^:[0-9]+$")


class TemporalHarnessError(ValueError):
    """Invalid, unowned, or incomplete temporal-harness input."""
MAX_PROCESS_OUTPUT_BYTES = 64 * 1024
MAX_PROCESS_CLEANUP_SECONDS = 5.0
CAPTURE_CLOCK_TOLERANCE_MS = 2_000.0
MAX_OUTPUT_TAIL_BYTES = 8 * 1024
SCENARIOS = frozenset({"scroll", "idle", "text", "text-only", "image", "resize", "focus"})
CONTROL_MIN_SAMPLES = 100


def _json_safe(value: Any) -> Any:
    if isinstance(value, float) and (math.isnan(value) or math.isinf(value)):
        return None
    if isinstance(value, Mapping):
        return {str(key): _json_safe(item) for key, item in value.items()}
    if isinstance(value, (list, tuple)):
        return [_json_safe(item) for item in value]
    return value


def _sha256_file(path: Path, *, limit: int | None = None) -> tuple[str, int]:
    digest = hashlib.sha256()
    size = 0
    try:
        with path.open("rb") as stream:
            while chunk := stream.read(1024 * 1024):
                size += len(chunk)
                if limit is not None and size > limit:
                    raise TemporalHarnessError(f"file exceeds bounded size: {path}")
                digest.update(chunk)
    except OSError as error:
        raise TemporalHarnessError(f"unable to read {path}: {error}") from error
    return digest.hexdigest(), size


def _path_chain(path: Path) -> Iterable[Path]:
    current = path
    chain: list[Path] = []
    while True:
        chain.append(current)
        if current.parent == current:
            break
        current = current.parent
    return reversed(chain)


def _reject_symlinks(path: Path, field: str) -> None:
    for component in _path_chain(path):
        try:
            if component.is_symlink():
                raise TemporalHarnessError(f"{field} has a symlink ancestor or leaf")
        except OSError as error:
            raise TemporalHarnessError(f"unable to inspect {field}: {error}") from error


def _owned_path(raw: str, root: str, field: str, *, existing: bool = False) -> Path:
    if not isinstance(raw, str) or not raw or "\x00" in raw:
        raise TemporalHarnessError(f"{field} must be a non-empty path")
    path = Path(raw)
    if not path.is_absolute():
        raise TemporalHarnessError(f"{field} must be absolute")
    root_path = Path(root)
    try:
        candidate = path.resolve(strict=False)
        root_resolved = root_path.resolve(strict=True)
        candidate.relative_to(root_resolved)
    except (OSError, ValueError) as error:
        raise TemporalHarnessError(f"{field} must be below the owned runtime root") from error
    _reject_symlinks(path, field)
    if existing and (not path.exists() or not path.is_file()):
        raise TemporalHarnessError(f"{field} must be an existing regular file")
    return path


def _load_json(path: Path, field: str, limit: int) -> Any:
    try:
        if path.stat().st_size > limit:
            raise TemporalHarnessError(f"{field} exceeds bounded size")
        with path.open("r", encoding="utf-8") as stream:
            return json.load(stream)
    except (OSError, json.JSONDecodeError) as error:
        raise TemporalHarnessError(f"invalid {field}: {error}") from error


def _write_json(path: Path, report: Mapping[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.tmp-{os.getpid()}")
    try:
        with temporary.open("x", encoding="utf-8") as stream:
            json.dump(_json_safe(report), stream, indent=2, sort_keys=True)
            stream.write("\n")
        os.replace(temporary, path)
    except OSError as error:
        try:
            temporary.unlink(missing_ok=True)
        except OSError:
            pass
        raise TemporalHarnessError(f"unable to write report: {error}") from error


def _guard(args: argparse.Namespace) -> tuple[Any, Path]:
    for name in ("ledger", "run_id", "session", "client", "fixture_id"):
        if not isinstance(getattr(args, name, None), str) or not getattr(args, name).strip():
            raise TemporalHarnessError(f"--{name.replace('_', '-')} is required before effects")
    if args.client not in {"browser", "native"}:
        raise TemporalHarnessError("--client must be browser or native")
    ledger = load_ledger(args.ledger)
    target = validate_target(ledger, args.run_id, args.session)
    validate_recorded_executable(target)
    return target, Path(target.resource_root)


def _nonempty_string(value: Any) -> bool:
    return isinstance(value, str) and bool(value.strip())


def _validate_recorded_display(
    ledger_path: str, display: str, run_id: str, session: str, client: str
) -> dict[str, Any]:
    """Require one typed, unambiguous X11 display receipt before capture."""
    raw = _load_json(Path(ledger_path), "ledger", MAX_REPORT_BYTES)
    resources = raw.get("resources") if isinstance(raw, dict) else None
    if not isinstance(resources, list):
        raise TemporalHarnessError("ledger lacks display ownership receipts")
    matches = [
        resource
        for resource in resources
        if isinstance(resource, dict)
        and resource.get("kind") == "x11_display"
        and resource.get("owner") == run_id
        and resource.get("run_id") == run_id
        and resource.get("session") == session
        and resource.get("client") == client
        and resource.get("display") == display
        and resource.get("status") in {"ready", "running"}
    ]
    if len(matches) != 1:
        raise TemporalHarnessError("display lacks one unambiguous typed run-owned receipt")
    receipt = matches[0]
    required = ("resource_id", "kind", "owner", "run_id", "session", "client", "display", "status")
    if any(key not in receipt for key in required) or not _nonempty_string(receipt.get("resource_id")):
        raise TemporalHarnessError("display receipt is incomplete")
    return {str(key): value for key, value in receipt.items()}

def _parse_roi(raw: str, field: str) -> MarkerROI:
    try:
        values = tuple(int(value) for value in raw.split(","))
    except ValueError as error:
        raise TemporalHarnessError(f"--{field} must be x,y,width,height") from error
    if len(values) != 4:
        raise TemporalHarnessError(f"--{field} must be x,y,width,height")
    try:
        roi = MarkerROI(*values)
        roi.validate(FrameSample(0.0, 4096, 4096, b""))
    except (TypeError, ValueError, DetectorError) as error:
        raise TemporalHarnessError(f"invalid --{field}: {error}") from error
    return roi


def _parse_video_size(raw: str) -> tuple[int, int]:
    try:
        width, height = (int(value) for value in raw.lower().split("x"))
    except ValueError as error:
        raise TemporalHarnessError("--video-size must be WIDTHxHEIGHT") from error
    if not 1 <= width <= 4096 or not 1 <= height <= 4096:
        raise TemporalHarnessError("--video-size is outside the bounded range")
    return width, height


def _run_ffmpeg_capture(
    command: Sequence[str], deadline_seconds: float
) -> tuple[int | None, str, str, int, int, bool, bool]:
    ffmpeg = shutil.which("ffmpeg")
    if ffmpeg is None:
        raise TemporalHarnessError("ffmpeg is required for capture")
    process = subprocess.Popen(
        list(command), stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True
    )
    assert process.stdout is not None
    assert process.stderr is not None
    selector = selectors.DefaultSelector()
    selector.register(process.stdout.fileno(), selectors.EVENT_READ, "stdout")
    selector.register(process.stderr.fileno(), selectors.EVENT_READ, "stderr")
    stdout_tail = bytearray()
    stderr_tail = bytearray()
    stdout_count = 0
    stderr_count = 0
    timed_out = False
    output_exceeded = False
    deadline = time.monotonic() + deadline_seconds
    try:
        while selector.get_map():
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                timed_out = True
                break
            for key, _ in selector.select(min(1.0, remaining)):
                try:
                    chunk = os.read(key.fd, 1024 * 1024)
                except BlockingIOError:
                    continue
                if not chunk:
                    selector.unregister(key.fd)
                    continue
                if key.data == "stdout":
                    stdout_count += len(chunk)
                    stdout_tail.extend(chunk)
                    if len(stdout_tail) > MAX_OUTPUT_TAIL_BYTES:
                        del stdout_tail[:-MAX_OUTPUT_TAIL_BYTES]
                else:
                    stderr_count += len(chunk)
                    stderr_tail.extend(chunk)
                    if len(stderr_tail) > MAX_OUTPUT_TAIL_BYTES:
                        del stderr_tail[:-MAX_OUTPUT_TAIL_BYTES]
                if stdout_count + stderr_count > MAX_PROCESS_OUTPUT_BYTES:
                    output_exceeded = True
                    break
            if timed_out or output_exceeded:
                break
    finally:
        selector.close()
        kill_required = timed_out or output_exceeded
        if not kill_required:
            remaining = max(0.0, deadline - time.monotonic())
            try:
                process.wait(timeout=remaining)
            except subprocess.TimeoutExpired:
                timed_out = True
                kill_required = True
        if kill_required:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            try:
                process.wait(timeout=MAX_PROCESS_CLEANUP_SECONDS)
            except subprocess.TimeoutExpired as error:
                raise TemporalHarnessError("ffmpeg cleanup exceeded bounded duration") from error
        process.stdout.close()
        process.stderr.close()
    return (
        process.returncode,
        stdout_tail.decode("utf-8", "replace"),
        stderr_tail.decode("utf-8", "replace"),
        stdout_count,
        stderr_count,
        timed_out,
        output_exceeded,
    )


def _executable_identity(path: str, *, limit: int = 256 * 1024 * 1024) -> dict[str, Any]:
    executable = Path(path)
    digest, size = _sha256_file(executable, limit=limit)
    return {"path": str(executable), "sha256": digest, "size_bytes": size}


def capture(args: argparse.Namespace) -> int:
    target, root = _guard(args)
    display_receipt = _validate_recorded_display(
        args.ledger, args.display, target.run_id, target.session.name, args.client
    )
    output = _owned_path(args.output, str(root), "capture output")
    report_path = _owned_path(args.report or f"{args.output}.json", str(root), "capture report")
    if output == report_path:
        raise TemporalHarnessError("capture output and report must be different files")
    if output.exists():
        raise TemporalHarnessError("capture output already exists; choose a new owned path")
    width, height = _parse_video_size(args.video_size)
    if not DISPLAY_RE.fullmatch(args.display):
        raise TemporalHarnessError("--display must be an explicit X11 display such as :191")
    if not math.isfinite(args.duration_seconds) or args.duration_seconds <= 0 or args.duration_seconds > MAX_CAPTURE_SECONDS:
        raise TemporalHarnessError("--duration-seconds is outside the bounded range")
    required_frames = math.ceil(args.duration_seconds * 60.0) + 1
    if required_frames > MAX_CAPTURE_FRAMES:
        raise TemporalHarnessError("duration requires more frames than the bounded capture limit")
    frames = required_frames if args.frames is None else args.frames
    if not 1 <= frames <= MAX_CAPTURE_FRAMES or frames < required_frames:
        raise TemporalHarnessError("capture frame count cannot cover the requested duration")
    if not math.isfinite(args.deadline_seconds) or args.deadline_seconds <= 0:
        raise TemporalHarnessError("--deadline-seconds must be finite and positive")
    try:
        output.parent.mkdir(parents=True, exist_ok=True)
        report_path.parent.mkdir(parents=True, exist_ok=True)
    except OSError as error:
        raise TemporalHarnessError(f"unable to create owned capture directories: {error}") from error
    ffmpeg = shutil.which("ffmpeg")
    if ffmpeg is None:
        raise TemporalHarnessError("ffmpeg is required for capture")
    executable = _executable_identity(ffmpeg)
    command = [
        ffmpeg,
        "-copyts",
        "-f", "x11grab",
        "-framerate", "60",
        "-video_size", f"{width}x{height}",
        "-draw_mouse", "0",
        "-i", args.display,
        "-frames:v", str(frames),
        "-c:v", "ffv1",
        "-pix_fmt", "bgr0",
        "-threads", "2",
        str(output),
    ]
    started = time.time()
    exit_status: int | None = None
    stdout = ""
    stderr = ""
    stdout_count = 0
    stderr_count = 0
    timed_out = False
    output_exceeded = False
    error: str | None = None
    try:
        (
            exit_status,
            stdout,
            stderr,
            stdout_count,
            stderr_count,
            timed_out,
            output_exceeded,
        ) = _run_ffmpeg_capture(command, max(15.0, min(360.0, args.deadline_seconds)))
    except TemporalHarnessError as caught:
        error = str(caught)
    finished = time.time()
    identity: dict[str, Any] | None = None
    actual: dict[str, Any] | None = None
    if output.exists() and output.is_file():
        digest, size = _sha256_file(output, limit=MAX_VIDEO_BYTES)
        identity = {"path": str(output), "sha256": digest, "size_bytes": size}
        if error is None and exit_status == 0:
            try:
                actual_width, actual_height, pts = probe_video(output, frames)
                finite_pts = [value * 1000.0 for value in pts if math.isfinite(value)]
                if actual_width != width or actual_height != height or len(pts) != frames or len(finite_pts) != frames:
                    raise TemporalHarnessError("capture metadata does not match requested dimensions or frame count")
                observed_duration = max(finite_pts) - min(finite_pts)
                if observed_duration < args.duration_seconds * 1000.0:
                    raise TemporalHarnessError("capture PTS do not cover requested duration")
                identity.update({
                    "width": actual_width,
                    "height": actual_height,
                    "frame_count": len(pts),
                    "pts_start_unix_ms": min(finite_pts),
                    "pts_end_unix_ms": max(finite_pts),
                    "observed_duration_ms": observed_duration,
                })
                actual = {
                    "width": actual_width,
                    "height": actual_height,
                    "frame_count": len(pts),
                    "pts_start_unix_ms": min(finite_pts),
                    "pts_end_unix_ms": max(finite_pts),
                    "observed_duration_ms": observed_duration,
                }
            except (DetectorError, OSError, TemporalHarnessError) as caught:
                error = str(caught)
    report = {
        "kind": "G01 terminal temporal capture",
        "status": "PASS" if exit_status == 0 and not timed_out and not output_exceeded and identity and actual else "INCONCLUSIVE",
        "client": args.client,
        "fixture_id": args.fixture_id,
        "run_id": target.run_id,
        "session": target.session.name,
        "display": args.display,
        "display_receipt": display_receipt,
        "video_size": {"width": width, "height": height},
        "requested_frames": frames,
        "required_frames": required_frames,
        "requested_fps": 60,
        "requested_duration_ms": args.duration_seconds * 1000.0,
        "started_unix_ms": int(started * 1000),
        "finished_unix_ms": int(finished * 1000),
        "clock": {"kind": "unix_ms", "pts_basis": "ffprobe.best_effort_timestamp_time", "tolerance_ms": CAPTURE_CLOCK_TOLERANCE_MS},
        "command": list(command),
        "executable": executable,
        "exit_status": exit_status,
        "timed_out": timed_out,
        "output_exceeded": output_exceeded,
        "stdout_tail": stdout,
        "stderr_tail": stderr,
        "stdout_bytes": stdout_count,
        "stderr_bytes": stderr_count,
        "actual": actual,
        "source_identity": identity,
        "error": error,
        "guard": {"resource_root": str(root), "executable": target.executable.path},
        "claims": ["capture evidence only; does not claim application mouse support or G01 completion"],
    }
    _write_json(report_path, report)
    return 0 if report["status"] == "PASS" else 2


def _value(sample: Mapping[str, Any], *keys: str) -> Any:
    for key in keys:
        if key in sample:
            return sample[key]
    return None


def _timestamp(sample: Mapping[str, Any]) -> float | None:
    value = _value(
        sample,
        "input_timestamp_unix_ms",
        "input_timestamp_ms",
        "input_timestamp",
        "inputTimestampUnixMs",
        "inputTimestampMs",
        "inputTimestamp",
        "timestamp_unix_ms",
    )
    try:
        result = float(value)
    except (TypeError, ValueError):
        return None
    return result if math.isfinite(result) else None


def _hash_value(sample: Mapping[str, Any], *keys: str) -> str | None:
    value = _value(sample, *keys)
    if not isinstance(value, str) or not SHA256_RE.fullmatch(value):
        return None
    return value.lower()

def _offset(sample: Mapping[str, Any], expected: bool) -> Any:
    if expected:
        return _value(
            sample,
            "expected_authoritative_offset",
            "expected_authoritative_scroll_offset",
            "expected_offset",
            "expectedAuthoritativeOffset",
        )
    return _value(
        sample,
        "observed_authoritative_offset",
        "observed_authoritative_scroll_offset",
        "observed_offset",
        "observedAuthoritativeOffset",
    )

def _valid_offset(value: Any) -> bool:
    if isinstance(value, bool):
        return False
    try:
        return math.isfinite(float(value))
    except (TypeError, ValueError):
        return False


def _load_trace(path: Path) -> dict[str, Any]:
    raw = _load_json(path, "trace", MAX_TRACE_BYTES)
    if not isinstance(raw, dict):
        raise TemporalHarnessError("trace must be a JSON object")
    if raw.get("clock") != "unix_ms":
        raise TemporalHarnessError("trace clock must be exactly unix_ms")
    if not _nonempty_string(raw.get("fixture_id")):
        raise TemporalHarnessError("trace fixture_id must be a non-empty string")
    samples = raw.get("samples")
    if not isinstance(samples, list) or not samples or len(samples) > MAX_FRAMES:
        raise TemporalHarnessError("trace samples must be a finite non-empty array")
    for index, sample in enumerate(samples):
        if not isinstance(sample, dict):
            raise TemporalHarnessError(f"trace sample {index} must be an object")
        if _timestamp(sample) is None:
            raise TemporalHarnessError(f"trace sample {index} lacks a finite input timestamp")
    return raw



def first_matching_frame_after(
    frames: Sequence[FrameSample], viewport_hashes: Sequence[str], input_timestamp_ms: float, expected_hash: str
) -> int | None:
    """Return the first presented frame at/after input whose ROI hash matches."""
    if not math.isfinite(input_timestamp_ms) or not SHA256_RE.fullmatch(expected_hash):
        return None
    for index, frame in enumerate(frames):
        if index >= len(viewport_hashes) or not math.isfinite(frame.timestamp_ms):
            continue
        if frame.timestamp_ms >= input_timestamp_ms and viewport_hashes[index].lower() == expected_hash.lower():
            return index
    return None




def visible_latency_ms(frames: Sequence[FrameSample], frame_index: int | None, input_timestamp_ms: float) -> float | None:
    if frame_index is None or frame_index < 0 or frame_index >= len(frames) or not math.isfinite(input_timestamp_ms):
        return None
    timestamp = frames[frame_index].timestamp_ms
    if not math.isfinite(timestamp) or timestamp < input_timestamp_ms:
        return None
    return timestamp - input_timestamp_ms

def _trace_intervals(trace: Mapping[str, Any]) -> list[dict[str, float]]:
    raw = trace.get("away_from_tail_intervals", trace.get("away_tail_intervals", []))
    if not isinstance(raw, list) or len(raw) > MAX_FRAMES:
        raise TemporalHarnessError("away-from-tail intervals must be a bounded array")
    intervals: list[dict[str, float]] = []
    for item in raw:
        if not isinstance(item, dict):
            raise TemporalHarnessError("away-from-tail interval must be an object")
        start = _value(item, "start_unix_ms", "start_timestamp_unix_ms", "start_ms", "startUnixMs")
        end = _value(item, "end_unix_ms", "end_timestamp_unix_ms", "end_ms", "endUnixMs")
        try:
            start_f, end_f = float(start), float(end)
        except (TypeError, ValueError) as error:
            raise TemporalHarnessError("away-from-tail interval timestamps must be finite") from error
        if not math.isfinite(start_f) or not math.isfinite(end_f) or end_f < start_f:
            raise TemporalHarnessError("away-from-tail interval timestamps must be ordered")
        intervals.append({"start_ms": start_f, "end_ms": end_f})
    return intervals

def _percentile(values: Sequence[float], percentile: float) -> float | None:
    if not values:
        return None
    ordered = sorted(values)
    rank = max(0, min(len(ordered) - 1, math.ceil(percentile * len(ordered)) - 1))
    return ordered[rank]
def _artifact_identity(value: Any) -> tuple[str, str, int] | None:
    if not isinstance(value, Mapping):
        return None
    path = value.get("path")
    digest = value.get("sha256")
    size = value.get("size_bytes")
    if (
        not _nonempty_string(path)
        or not isinstance(digest, str)
        or not SHA256_RE.fullmatch(digest)
        or isinstance(size, bool)
        or not isinstance(size, int)
        or size < 0
    ):
        return None
    return str(path), digest.lower(), size


def _paired_latencies(raw: Any, trace: Mapping[str, Any]) -> tuple[list[float], str | None]:
    if not isinstance(raw, Mapping):
        return [], "paired_control_missing_provenance"
    if raw.get("kind") != "G01 terminal temporal replay" or raw.get("status") != "PASS":
        return [], "paired_control_not_successful"
    if raw.get("scenario") != "text-only":
        return [], "paired_control_wrong_scenario"
    for key in ("run_id", "session", "client", "fixture_id"):
        expected = trace.get(key)
        actual = raw.get(key)
        if not _nonempty_string(expected) or not _nonempty_string(actual):
            return [], "paired_control_missing_provenance"
        if actual != expected:
            return [], "paired_control_identity_mismatch"
    source = _artifact_identity(raw.get("source_identity"))
    pair_trace = _artifact_identity(raw.get("trace_identity"))
    if source is None or pair_trace is None:
        return [], "paired_control_missing_source_identity"
    sampled = raw.get("sampled_latencies")
    samples = sampled.get("samples") if isinstance(sampled, Mapping) else None
    if not isinstance(samples, list) or len(samples) < CONTROL_MIN_SAMPLES:
        return [], "paired_control_inadequate_samples"
    ids: set[str] = set()
    values: list[float] = []
    previous_input = -math.inf
    for item in samples:
        if not isinstance(item, Mapping):
            return [], "paired_control_invalid_sample"
        event_id = item.get("event_id", item.get("input_id"))
        if not _nonempty_string(event_id) or event_id in ids:
            return [], "paired_control_duplicate_samples"
        ids.add(event_id)
        input_ms = _timestamp(item)
        before = _hash_value(item, "before_viewport_hash", "before_viewport_sha256", "before_hash", "beforeViewportHash")
        expected = _hash_value(item, "expected_viewport_hash", "expected_viewport_sha256", "expected_hash", "expectedViewportHash")
        if input_ms is None or input_ms <= previous_input:
            return [], "paired_control_unordered_timestamps"
        previous_input = input_ms
        if before is None or expected is None or before == expected:
            return [], "paired_control_missing_motion"
        try:
            value = float(item.get("latency_ms"))
        except (TypeError, ValueError):
            return [], "paired_control_invalid_latency"
        if not math.isfinite(value) or value < 0:
            return [], "paired_control_invalid_latency"
        values.append(value)
    return values, None
def _validate_paired_artifacts(raw: Any, root: str) -> None:
    if not isinstance(raw, Mapping):
        raise TemporalHarnessError("paired control must be a replay report object")
    for field, label in (("source_identity", "paired control source"), ("trace_identity", "paired control trace")):
        identity = _artifact_identity(raw.get(field))
        if identity is None:
            raise TemporalHarnessError(f"paired_control_{field}_missing")
        path, digest, size = identity
        artifact = _owned_path(path, root, label, existing=True)
        actual_digest, actual_size = _sha256_file(artifact, limit=MAX_VIDEO_BYTES if field == "source_identity" else MAX_TRACE_BYTES)
        if actual_digest.lower() != digest or actual_size != size:
            raise TemporalHarnessError(f"paired_control_{field}_identity_mismatch")


def _frame_index_at_or_before(frames: Sequence[FrameSample], hashes: Sequence[str], timestamp_ms: float) -> int | None:
    found: int | None = None
    for index, frame in enumerate(frames):
        if index >= len(hashes) or not math.isfinite(frame.timestamp_ms):
            continue
        if frame.timestamp_ms <= timestamp_ms:
            found = index
        else:
            break
    return found


def verify_trace(
    frames: Sequence[FrameSample],
    viewport_hashes: Sequence[str] | None,
    trace: Mapping[str, Any],
    *,
    scenario: str,
    paired_control: Any = None,
) -> dict[str, Any]:
    """Correlate a finite trace with presented frames and optional viewport hashes."""
    if trace.get("clock") != "unix_ms":
        return {"status": "INCONCLUSIVE", "reason": "missing_or_invalid_trace_clock"}
    if scenario not in SCENARIOS:
        return {"status": "INCONCLUSIVE", "reason": "unknown_scenario"}
    raw_samples = trace.get("samples")
    if not isinstance(raw_samples, list) or not raw_samples:
        return {"status": "INCONCLUSIVE", "reason": "missing_trace_samples"}
    if viewport_hashes is not None and len(frames) != len(viewport_hashes):
        return {"status": "INCONCLUSIVE", "reason": "viewport_frame_count_mismatch"}
    latencies: list[float] = []
    details: list[dict[str, Any]] = []
    offset_failures: list[int] = []
    viewport_failures: list[int] = []
    invalid_samples = 0
    event_ids: set[str] = set()
    previous_input = -math.inf
    for index, item in enumerate(raw_samples):
        if not isinstance(item, Mapping):
            invalid_samples += 1
            continue
        input_ms = _timestamp(item)
        before = _hash_value(item, "before_viewport_hash", "before_viewport_sha256", "before_hash", "beforeViewportHash")
        expected = _hash_value(item, "expected_viewport_hash", "expected_viewport_sha256", "expected_hash", "expectedViewportHash")
        event_id = item.get("event_id", item.get("input_id"))
        if scenario == "scroll":
            if not _nonempty_string(event_id) or event_id in event_ids:
                invalid_samples += 1
            else:
                event_ids.add(event_id)
            if input_ms is None or input_ms <= previous_input:
                invalid_samples += 1
            elif input_ms is not None:
                previous_input = input_ms
        if input_ms is None or before is None or expected is None or (scenario == "scroll" and before == expected):
            invalid_samples += 1
            continue
        frame_index: int | None = None
        latency: float | None = None
        before_index: int | None = None
        if viewport_hashes is not None:
            before_index = _frame_index_at_or_before(frames, viewport_hashes, input_ms)
            if before_index is None or viewport_hashes[before_index].lower() != before:
                invalid_samples += 1
            frame_index = first_matching_frame_after(frames, viewport_hashes, input_ms, expected)
            latency = visible_latency_ms(frames, frame_index, input_ms)
            if frame_index is None:
                viewport_failures.append(index)
            elif before != expected:
                latencies.append(latency if latency is not None else math.inf)
        expected_offset = _offset(item, True)
        observed_offset = _offset(item, False)
        if not _valid_offset(expected_offset) or not _valid_offset(observed_offset):
            invalid_samples += 1
        elif expected_offset != observed_offset:
            offset_failures.append(index)
        details.append({
            "sample_index": index,
            "event_id": event_id,
            "input_timestamp_unix_ms": input_ms,
            "before_frame_index": before_index,
            "frame_index": frame_index,
            "latency_ms": latency,
            "before_viewport_hash": before,
            "expected_viewport_hash": expected,
            "motion": before != expected,
            "expected_authoritative_offset": expected_offset,
            "observed_authoritative_offset": observed_offset,
        })

    final_expected = _value(trace, "expected_final_authoritative_offset", "expectedFinalAuthoritativeOffset")
    final_observed = _value(trace, "observed_final_authoritative_offset", "observedFinalAuthoritativeOffset")
    final_offset_failure = _valid_offset(final_expected) and _valid_offset(final_observed) and final_expected != final_observed
    if scenario == "scroll" and (
        not _valid_offset(final_expected) or not _valid_offset(final_observed)
    ):
        invalid_samples += 1
    away_checks: list[dict[str, Any]] = []
    away_failures: list[int] = []
    away_missing: list[int] = []
    try:
        intervals = _trace_intervals(trace)
    except TemporalHarnessError as error:
        return {"status": "INCONCLUSIVE", "reason": str(error), "samples": details}
    if scenario == "scroll" and not intervals:
        away_missing.append(0)
    raw_intervals = trace.get("away_from_tail_intervals", trace.get("away_tail_intervals", []))
    for interval_index, interval in enumerate(intervals):
        raw_interval = raw_intervals[interval_index]
        interval_expected = _value(raw_interval, "expected_authoritative_offset", "expected_offset") if isinstance(raw_interval, Mapping) else None
        interval_observed = _value(raw_interval, "observed_authoritative_offset", "observed_offset") if isinstance(raw_interval, Mapping) else None
        if scenario == "scroll" and (
            not isinstance(raw_interval, Mapping)
            or raw_interval.get("away_from_tail") is not True
            or not _valid_offset(interval_expected)
            or not _valid_offset(interval_observed)
            or interval_expected != interval_observed
        ):
            invalid_samples += 1
        presented = [
            (index, viewport_hashes[index])
            for index, frame in enumerate(frames)
            if viewport_hashes is not None and math.isfinite(frame.timestamp_ms) and interval["start_ms"] <= frame.timestamp_ms <= interval["end_ms"]
        ]
        hashes = [value for _, value in presented]
        unchanged = len(hashes) >= 2 and len(set(hashes)) == 1
        away_checks.append({"interval_index": interval_index, **interval, "presented_frames": len(presented), "unchanged": unchanged})
        if len(hashes) < 2 or interval["end_ms"] <= interval["start_ms"]:
            away_missing.append(interval_index)
        elif not unchanged:
            away_failures.append(interval_index)

    p95 = _percentile([value for value in latencies if math.isfinite(value)], 0.95)
    paired, paired_error = (
        _paired_latencies(paired_control, trace)
        if scenario == "scroll"
        else ([], None)
    )
    paired_p95 = _percentile(paired, 0.95)
    reasons: list[str] = []
    if invalid_samples:
        reasons.append("invalid_trace_samples")
    if viewport_failures:
        reasons.append("viewport_assertion_failed")
    if offset_failures or final_offset_failure:
        reasons.append("authoritative_offset_assertion_failed")
    if away_failures:
        reasons.append("away_from_tail_viewport_changed")
    if away_missing:
        reasons.append("away_from_tail_inadequate_coverage")
    usable_motion = len([item for item in details if item.get("motion") and item.get("latency_ms") is not None])
    if scenario == "scroll":
        if usable_motion < CONTROL_MIN_SAMPLES or len(event_ids) != len(raw_samples):
            reasons.append("insufficient_distinct_motion_samples")
        if p95 is None or p95 > 100.0:
            reasons.append("scroll_p95_exceeds_100ms")
        if paired_error:
            reasons.append(paired_error)
        elif paired_p95 is None:
            reasons.append("missing_paired_text_control")
        elif p95 is not None and p95 > paired_p95 * 1.25:
            reasons.append("scroll_p95_exceeds_paired_text_control")
    latency_failed = scenario == "scroll" and usable_motion >= CONTROL_MIN_SAMPLES and p95 is not None and (
        p95 > 100.0 or (paired_error is None and paired_p95 is not None and p95 > paired_p95 * 1.25)
    )
    hard_fail = bool(viewport_failures or offset_failures or final_offset_failure or away_failures or latency_failed)
    status = "FAIL" if hard_fail else ("INCONCLUSIVE" if reasons else "PASS")
    return {
        "status": status,
        "reason": ",".join(dict.fromkeys(reasons)) if reasons else "trace_assertions_passed",
        "usable_motion_samples": usable_motion,
        "distinct_input_samples": len(event_ids),
        "required_motion_samples": CONTROL_MIN_SAMPLES if scenario == "scroll" else 0,
        "visible_latencies_ms": [round(value, 3) for value in latencies if math.isfinite(value)],
        "p95_visible_latency_ms": p95,
        "paired_text_control_p95_ms": paired_p95,
        "samples": details,
        "away_from_tail_checks": away_checks,
        "offset_failures": offset_failures,
        "viewport_failures": viewport_failures,
        "final_offset_failure": final_offset_failure,
    }

def _replay_source_identity(
    source: Path, capture_report: Path, target: Any, args: argparse.Namespace
) -> tuple[dict[str, Any], dict[str, Any]]:
    raw = _load_json(capture_report, "capture report", MAX_REPORT_BYTES)
    if (
        not isinstance(raw, dict)
        or raw.get("kind") != "G01 terminal temporal capture"
        or raw.get("status") != "PASS"
        or raw.get("fixture_id") != args.fixture_id
        or raw.get("run_id") != target.run_id
        or raw.get("session") != target.session.name
        or raw.get("client") != args.client
        or raw.get("display") != args.display
    ):
        raise TemporalHarnessError("capture provenance is missing, malformed, or bound to another run")
    display_receipt = raw.get("display_receipt")
    if (
        not isinstance(display_receipt, Mapping)
        or not _nonempty_string(display_receipt.get("resource_id"))
        or display_receipt.get("kind") != "x11_display"
        or display_receipt.get("owner") != target.run_id
        or display_receipt.get("run_id") != target.run_id
        or display_receipt.get("session") != target.session.name
        or display_receipt.get("client") != args.client
        or display_receipt.get("display") != args.display
        or display_receipt.get("status") not in {"ready", "running"}
    ):
        raise TemporalHarnessError("capture display receipt is missing or unbound")
    identity = raw.get("source_identity")
    actual = raw.get("actual")
    clock = raw.get("clock")
    command = raw.get("command")
    executable = raw.get("executable")
    if not isinstance(identity, dict) or not isinstance(actual, dict) or not isinstance(clock, dict):
        raise TemporalHarnessError("capture provenance lacks complete source metadata")
    if not isinstance(command, list) or "-copyts" not in command or str(source) not in command:
        raise TemporalHarnessError("capture command is not bound to the replay source with recorded PTS")
    if clock.get("kind") != "unix_ms" or clock.get("pts_basis") != "ffprobe.best_effort_timestamp_time":
        raise TemporalHarnessError("capture clock provenance is missing or invalid")
    if not isinstance(executable, dict) or not SHA256_RE.fullmatch(str(executable.get("sha256", ""))):
        raise TemporalHarnessError("capture executable provenance is incomplete")
    ffmpeg = shutil.which("ffmpeg")
    if ffmpeg is None or _executable_identity(ffmpeg) != executable:
        raise TemporalHarnessError("capture ffmpeg executable identity drifted")
    recorded = _artifact_identity(identity)
    if recorded is None:
        raise TemporalHarnessError("capture provenance lacks a valid source identity")
    recorded_path, recorded_hash, recorded_size = recorded
    if recorded_path != str(source):
        raise TemporalHarnessError("replay source path differs from capture provenance")
    actual_hash, actual_size = _sha256_file(source, limit=MAX_VIDEO_BYTES)
    if actual_hash.lower() != recorded_hash or actual_size != recorded_size:
        raise TemporalHarnessError("replay source identity drifted from capture provenance")
    required_actual = ("width", "height", "frame_count", "pts_start_unix_ms", "pts_end_unix_ms", "observed_duration_ms")
    if any(key not in identity or key not in actual for key in required_actual):
        raise TemporalHarnessError("capture source identity lacks complete decoded metadata")
    return {"path": str(source), "sha256": actual_hash, "size_bytes": actual_size, **actual}, raw


def replay(args: argparse.Namespace) -> int:
    target, root = _guard(args)
    source = _owned_path(args.input, str(root), "replay source", existing=True)
    report_path = _owned_path(args.report, str(root), "replay report")
    capture_report_path = _owned_path(args.capture_report, str(root), "capture report", existing=True) if args.capture_report else None
    input_paths = {source, capture_report_path, Path(args.trace).resolve()}
    if args.paired_control:
        input_paths.add(Path(args.paired_control).resolve())
    if report_path in input_paths:
        raise TemporalHarnessError("replay report cannot alias an input artifact")
    provenance_error: str | None = None
    source_identity: dict[str, Any] | None = None
    capture_metadata: dict[str, Any] | None = None
    if capture_report_path is None:
        provenance_error = "missing_capture_provenance"
    else:
        try:
            source_identity, capture_metadata = _replay_source_identity(source, capture_report_path, target, args)
        except TemporalHarnessError as error:
            provenance_error = str(error)
    try:
        trace_path = _owned_path(args.trace, str(root), "trace", existing=True)
    except TemporalHarnessError as error:
        report = {
            "kind": "G01 terminal temporal replay",
            "status": "INCONCLUSIVE",
            "reason": f"missing_trace:{error}",
            "client": args.client,
            "fixture_id": args.fixture_id,
            "run_id": target.run_id,
            "session": target.session.name,
            "source_identity": source_identity,
            "trace_identity": {"path": args.trace, "sha256": None},
            "commands": [],
            "claims": ["does not claim application mouse support or G01 completion"],
        }
        _write_json(report_path, report)
        return 2
    trace_hash, trace_size = _sha256_file(trace_path, limit=MAX_TRACE_BYTES)
    try:
        trace = _load_trace(trace_path)
    except TemporalHarnessError as error:
        report = {
            "kind": "G01 terminal temporal replay",
            "status": "INCONCLUSIVE",
            "reason": str(error),
            "client": args.client,
            "fixture_id": args.fixture_id,
            "run_id": target.run_id,
            "session": target.session.name,
            "source_identity": source_identity,
            "trace_identity": {"path": str(trace_path), "sha256": trace_hash, "size_bytes": trace_size},
            "commands": [],
            "claims": ["does not claim application mouse support or G01 completion"],
        }
        _write_json(report_path, report)
        return 2
    scenario = args.scenario
    trace_errors: list[str] = []
    if trace.get("scenario") != scenario:
        trace_errors.append("trace_scenario_mismatch")
    for key, expected in (
        ("run_id", target.run_id),
        ("session", target.session.name),
        ("client", args.client),
        ("fixture_id", args.fixture_id),
    ):
        if trace.get(key) != expected:
            trace_errors.append(f"trace_{key}_mismatch")
    marker_roi = _parse_roi(args.marker_roi, "marker-roi")
    viewport_roi = _parse_roi(args.viewport_roi, "viewport-roi") if args.viewport_roi else None
    detector_report: dict[str, Any]
    viewport_samples: list[FrameSample] = []
    viewport_hashes: list[str] | None = None
    decode_error: str | None = None
    clock_error: str | None = None
    try:
        marker_samples = load_samples(
            source, expected_cadence_ms=1000.0 / 60.0, marker_roi=marker_roi, max_frames=args.max_frames
        )
        detector_report = analyze_frames(
            marker_samples,
            DetectorConfig(marker_roi, expected_cadence_ms=1000.0 / 60.0, min_duration_ms=30_000.0),
        )
        if capture_metadata is not None:
            start_ms = float(capture_metadata.get("started_unix_ms", math.nan))
            end_ms = float(capture_metadata.get("finished_unix_ms", math.nan))
            tolerance = float((capture_metadata.get("clock") or {}).get("tolerance_ms", CAPTURE_CLOCK_TOLERANCE_MS))
            timestamps = [frame.timestamp_ms for frame in marker_samples]
            recorded_actual = capture_metadata.get("actual")
            if not isinstance(recorded_actual, Mapping) or recorded_actual.get("frame_count") != len(marker_samples):
                clock_error = "capture_metadata_frame_count_mismatch"
            elif not marker_samples or recorded_actual.get("width") != marker_samples[0].width or recorded_actual.get("height") != marker_samples[0].height:
                clock_error = "capture_metadata_dimensions_mismatch"
            elif (
                not math.isfinite(tolerance)
                or tolerance < 0
                or tolerance > CAPTURE_CLOCK_TOLERANCE_MS
                or not math.isfinite(start_ms)
                or not math.isfinite(end_ms)
                or end_ms < start_ms
                or not timestamps
                or any(not math.isfinite(value) or value < start_ms - tolerance or value > end_ms + tolerance for value in timestamps)
            ):
                clock_error = "decoded_pts_outside_capture_wall_clock_envelope"
            else:
                recorded_start = float(recorded_actual.get("pts_start_unix_ms", math.nan))
                recorded_end = float(recorded_actual.get("pts_end_unix_ms", math.nan))
                if abs(recorded_start - min(timestamps)) > 1.0 or abs(recorded_end - max(timestamps)) > 1.0:
                    clock_error = "capture_metadata_pts_mismatch"
        if viewport_roi is not None:
            full_frame = bool(marker_samples) and viewport_roi == MarkerROI(0, 0, marker_samples[0].width, marker_samples[0].height)
            if full_frame:
                if any(sample.pixel_sha256 is None for sample in marker_samples):
                    raise DetectorError("full-frame viewport hashes lack raw-RGB provenance")
                viewport_hashes = [str(sample.pixel_sha256) for sample in marker_samples]
            else:
                viewport_samples = load_samples(
                    source, expected_cadence_ms=1000.0 / 60.0, marker_roi=viewport_roi, max_frames=args.max_frames
                )
                viewport_hashes = [hashlib.sha256(sample.pixels).hexdigest() for sample in viewport_samples]
    except (DetectorError, OSError, TemporalHarnessError, ValueError) as error:
        detector_report = {"status": "INCONCLUSIVE", "reason": "capture_or_decode_error", "error": str(error)}
        decode_error = str(error)
        marker_samples = []
        viewport_hashes = None
    paired_control = None
    if args.paired_control:
        try:
            paired_path = _owned_path(args.paired_control, str(root), "paired control", existing=True)
            if report_path == paired_path:
                raise TemporalHarnessError("replay report cannot alias paired control")
            paired_control = _load_json(paired_path, "paired control", MAX_REPORT_BYTES)
            _validate_paired_artifacts(paired_control, str(root))
            for field, current in (("source_identity", source_identity), ("trace_identity", {"path": str(trace_path), "sha256": trace_hash})):
                paired_identity = paired_control.get(field)
                if isinstance(current, Mapping) and isinstance(paired_identity, Mapping) and (
                    paired_identity.get("path") == current.get("path") or paired_identity.get("sha256") == current.get("sha256")
                ):
                    raise TemporalHarnessError("paired control must use a distinct capture and trace")
        except TemporalHarnessError as error:
            trace_errors.append(str(error))
    if scenario == "scroll" and viewport_hashes is None:
        trace_errors.append("missing_viewport_roi")
    elif viewport_hashes is None and any(
        isinstance(item, Mapping) and (_hash_value(item, "before_viewport_hash", "before_viewport_sha256", "before_hash", "beforeViewportHash") or _hash_value(item, "expected_viewport_hash", "expected_viewport_sha256", "expected_hash", "expectedViewportHash"))
        for item in trace.get("samples", [])
    ):
        trace_errors.append("viewport_claim_without_roi")
    trace_report = {"status": "INCONCLUSIVE", "reason": ",".join(trace_errors)} if trace_errors else verify_trace(
        marker_samples,
        viewport_hashes,
        trace,
        scenario=scenario,
        paired_control=paired_control,
    )
    reasons: list[str] = list(dict.fromkeys(trace_errors))
    if provenance_error:
        reasons.append(provenance_error)
    if decode_error:
        reasons.append("capture_or_decode_error")
    if clock_error:
        reasons.append(clock_error)
    if detector_report.get("status") == "FAIL" or trace_report.get("status") == "FAIL":
        status = "FAIL"
    elif reasons or detector_report.get("status") != "PASS" or trace_report.get("status") != "PASS":
        status = "INCONCLUSIVE"
        if trace_report.get("reason"):
            reasons.append(str(trace_report["reason"]))
    else:
        status = "PASS"
    report = {
        "kind": "G01 terminal temporal replay",
        "status": status,
        "reason": ",".join(dict.fromkeys(reasons)) if reasons else "temporal_and_trace_assertions_passed",
        "client": args.client,
        "scenario": scenario,
        "fixture_id": args.fixture_id,
        "run_id": target.run_id,
        "session": target.session.name,
        "source_identity": source_identity,
        "trace_identity": {"path": str(trace_path), "sha256": trace_hash, "size_bytes": trace_size},
        "marker_roi": vars(marker_roi),
        "viewport_roi": vars(viewport_roi) if viewport_roi else None,
        "frame_data": detector_report,
        "sampled_latencies": trace_report,
        "clock_validation": {"status": "PASS" if clock_error is None else "INCONCLUSIVE", "error": clock_error},
        "commands": [{"operation": "bounded temporal decode", "exit_status": 0 if not decode_error else None}],
        "claims": ["does not claim application mouse support or G01 completion"],
    }
    _write_json(report_path, report)
    return {"PASS": 0, "FAIL": 1, "INCONCLUSIVE": 2}[status]


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="operation", required=True)
    common = argparse.ArgumentParser(add_help=False)
    common.add_argument("--ledger", required=True, help="run-owned resource ledger JSON")
    common.add_argument("--fixture-id", required=True, help="non-empty fixture identity shared by capture and trace")
    common.add_argument("--run-id", required=True, help="ledger run identity")
    common.add_argument("--session", required=True, help="explicit run-owned non-default session")
    common.add_argument("--client", required=True, choices=("browser", "native"), help="actual client producing the trace")
    capture_parser = subparsers.add_parser("capture", parents=[common], help="capture an owned X11 display with bounded ffmpeg")
    capture_parser.add_argument("--display", required=True, help="owned X11 display, for example :191")
    capture_parser.add_argument("--output", required=True, help="new lossless video path below the ledger runtime root")
    capture_parser.add_argument("--report", help="capture report path below the ledger runtime root")
    capture_parser.add_argument("--video-size", default="1440x900")
    capture_parser.add_argument("--duration-seconds", type=float, default=30.0)
    capture_parser.add_argument("--frames", type=int, help="finite frame count; overrides duration")
    capture_parser.add_argument("--deadline-seconds", type=float, default=90.0)
    capture_parser.set_defaults(handler=capture)
    replay_parser = subparsers.add_parser("replay", parents=[common], help="decode a capture and verify supplied unix_ms trace")
    replay_parser.add_argument("--display", required=True, help="owned X11 display recorded by capture")
    replay_parser.add_argument("--input", required=True, help="owned captured FFV1 video")
    replay_parser.add_argument("--capture-report", required=True, help="capture report proving source identity")
    replay_parser.add_argument("--trace", required=True, help="finite trace JSON with clock=unix_ms")
    replay_parser.add_argument("--paired-control", help="successful text-only replay report used as the paired control")
    replay_parser.add_argument("--report", required=True, help="replay report path below runtime root")
    replay_parser.add_argument("--marker-roi", required=True, help="x,y,width,height")
    replay_parser.add_argument("--viewport-roi", help="displayed RGB ROI, required for viewport claims")
    replay_parser.add_argument("--scenario", required=True, choices=sorted(SCENARIOS))
    replay_parser.add_argument("--max-frames", type=int, default=MAX_FRAMES)
    replay_parser.set_defaults(handler=replay)
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    try:
        args = build_parser().parse_args(argv)
        return int(args.handler(args))
    except (TemporalHarnessError, ResourceGuardError, DetectorError) as error:
        print(f"terminal-temporal: INCONCLUSIVE: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
