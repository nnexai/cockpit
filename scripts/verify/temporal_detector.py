#!/usr/bin/env python3
"""Detect temporal terminal blanks and marker loss from bounded frame/video samples.

This module is deliberately pure with respect to Herdr: it reads frame files or
asks an explicitly installed ``ffmpeg`` to decode a recording, then reports
PASS, FAIL, or INCONCLUSIVE.  A final screenshot is never considered sufficient.
"""

from __future__ import annotations
import argparse
import hashlib
import json
import math
import os
import selectors
import shutil
import signal
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Sequence

MAX_DIMENSION = 4096
MAX_FRAME_BYTES = 64 * 1024 * 1024
MAX_RETAINED_ROI_BYTES = 256 * 1024 * 1024
MAX_FRAMES = 10_000
MAX_DECODE_SECONDS = 120.0
MAX_STDERR_BYTES = 8 * 1024
MAX_STDOUT_BYTES = 8 * 1024 * 1024
MAX_PROCESS_CLEANUP_SECONDS = 5.0
CADENCE_TOLERANCE_FRACTION = 0.20

class DetectorError(ValueError):
    """Invalid detector input or bounded decoding failure."""


@dataclass(frozen=True)
class FrameSample:
    timestamp_ms: float
    width: int
    height: int
    pixels: bytes
    path: str | None = None
    sha256: str | None = None
    decoded_roi: MarkerROI | None = None
    pixel_sha256: str | None = None


@dataclass(frozen=True)
class MarkerROI:
    x: int
    y: int
    width: int
    height: int

    def validate(self, frame: FrameSample) -> None:
        if self.width <= 0 or self.height <= 0 or self.x < 0 or self.y < 0:
            raise DetectorError("marker ROI must have positive dimensions and non-negative origin")
        if self.x + self.width > frame.width or self.y + self.height > frame.height:
            raise DetectorError("marker ROI lies outside a frame")

@dataclass(frozen=True)
class DetectorConfig:
    marker_roi: MarkerROI
    expected_cadence_ms: float = 16.667
    marker_tolerance: float = 24.0
    dark_luma_threshold: float = 8.0
    min_samples: int = 3
    max_gap_factor: float = 2.5
    min_duration_ms: float = 30_000.0

    def validate(self) -> None:
        if not math.isfinite(self.expected_cadence_ms) or not 0.1 <= self.expected_cadence_ms <= 10_000:
            raise DetectorError("expected cadence must be between 0.1 and 10000 ms")
        if not math.isfinite(self.marker_tolerance) or not 0 <= self.marker_tolerance <= 255:
            raise DetectorError("marker tolerance must be between 0 and 255")
        if not math.isfinite(self.dark_luma_threshold) or not 0 <= self.dark_luma_threshold <= 255:
            raise DetectorError("dark luma threshold must be between 0 and 255")
        if self.min_samples < 3 or self.min_samples > MAX_FRAMES:
            raise DetectorError("min_samples must be between 3 and 10000")
        if not math.isfinite(self.max_gap_factor) or self.max_gap_factor < 1:
            raise DetectorError("max gap factor must be at least 1")
        if not math.isfinite(self.min_duration_ms) or self.min_duration_ms < 0:
            raise DetectorError("minimum duration must be finite and non-negative")


def _sha256(data: bytes | bytearray | memoryview) -> str:
    return hashlib.sha256(data).hexdigest()


def _ppm_tokens(data: bytes) -> tuple[str, list[int], int]:
    """Parse a bounded PPM header and return magic, numeric fields, payload offset."""
    index = 0
    tokens: list[bytes] = []
    while len(tokens) < 4:
        while index < len(data) and data[index] in b" \t\r\n":
            index += 1
        if index < len(data) and data[index] == ord("#"):
            newline = data.find(b"\n", index)
            index = len(data) if newline < 0 else newline + 1
            continue
        start = index
        while index < len(data) and data[index] not in b" \t\r\n#":
            index += 1
        if start == index:
            break
        tokens.append(data[start:index])
    if len(tokens) != 4:
        raise DetectorError("invalid PPM header")
    try:
        magic = tokens[0].decode("ascii")
        width, height, maximum = (int(item) for item in tokens[1:])
    except (ValueError, UnicodeDecodeError) as error:
        raise DetectorError("invalid PPM dimensions") from error
    if magic not in {"P6", "P3"} or not 1 <= width <= MAX_DIMENSION or not 1 <= height <= MAX_DIMENSION:
        raise DetectorError("unsupported or unsafe PPM dimensions")
    if not 1 <= maximum <= 255:
        raise DetectorError("PPM maximum must be between 1 and 255")
    # P6 has one whitespace separator after maxval; P3 is decoded below.
    return magic, [width, height, maximum], index


def _skip_ppm_separator(data: bytes, offset: int) -> int:
    if offset < len(data) and data[offset] in b" \t\r\n":
        offset += 1
        if offset < len(data) and data[offset - 1] == ord("\r") and data[offset] == ord("\n"):
            offset += 1
    return offset


def _decode_ppm(data: bytes) -> tuple[int, int, bytes]:
    magic, values, offset = _ppm_tokens(data)
    width, height, maximum = values
    count = width * height * 3
    if magic == "P6":
        offset = _skip_ppm_separator(data, offset)
        payload = data[offset : offset + count]
        if len(payload) != count:
            raise DetectorError("truncated PPM pixel data")
        if maximum != 255:
            payload = bytes(round(value * 255 / maximum) for value in payload)
        return width, height, payload
    numbers = data[offset:].replace(b"#", b" #")
    values_text: list[int] = []
    for token in numbers.split():
        if token == b"#":
            break
        try:
            values_text.append(int(token))
        except ValueError as error:
            raise DetectorError("invalid P3 pixel data") from error
    if len(values_text) < count:
        raise DetectorError("truncated P3 pixel data")
    if any(value < 0 or value > maximum for value in values_text[:count]):
        raise DetectorError("P3 pixel is outside its declared range")
    return width, height, bytes(round(value * 255 / maximum) for value in values_text[:count])
def _decode_image(path: Path) -> tuple[int, int, bytes]:
    try:
        if path.stat().st_size > MAX_FRAME_BYTES:
            raise DetectorError(f"frame exceeds {MAX_FRAME_BYTES} bytes: {path}")
        data = path.read_bytes()
    except OSError as error:
        raise DetectorError(f"unable to read frame: {path}") from error
    if len(data) > MAX_FRAME_BYTES:
        raise DetectorError(f"frame exceeds {MAX_FRAME_BYTES} bytes: {path}")
    if data.startswith((b"P3", b"P6")):
        return _decode_ppm(data)
    ffmpeg = shutil.which("ffmpeg")
    if ffmpeg is None:
        raise DetectorError("ffmpeg is required to decode non-PPM images")
    process = subprocess.Popen(
        [ffmpeg, "-v", "error", "-i", str(path), "-frames:v", "1", "-f", "image2pipe", "-vcodec", "ppm", "pipe:1"],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        start_new_session=True,
    )
    encoded, _, stderr_bytes = _read_bounded_process(process, MAX_FRAME_BYTES)
    if process.returncode != 0:
        detail = stderr_bytes.decode("utf-8", "replace")[-240:]
        raise DetectorError(f"ffmpeg could not decode {path}: {detail}")
    return _decode_ppm(encoded)


def _read_bounded_process(
    process: subprocess.Popen[bytes], limit: int, *, stderr_limit: int = MAX_STDERR_BYTES
) -> tuple[bytes, int, bytes]:
    chunks: list[bytes] = []
    total = 0
    stderr_tail = bytearray()
    assert process.stdout is not None
    assert process.stderr is not None
    stdout_fd = process.stdout.fileno()
    stderr_fd = process.stderr.fileno()
    os.set_blocking(stdout_fd, False)
    os.set_blocking(stderr_fd, False)
    selector = selectors.DefaultSelector()
    selector.register(stdout_fd, selectors.EVENT_READ, "stdout")
    selector.register(stderr_fd, selectors.EVENT_READ, "stderr")
    deadline = time.monotonic() + MAX_DECODE_SECONDS
    try:
        while selector.get_map():
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                _terminate_process_group(process)
                raise DetectorError("ffmpeg process exceeded bounded duration")
            for key, _ in selector.select(min(1.0, remaining)):
                try:
                    chunk = os.read(key.fd, 1024 * 1024)
                except BlockingIOError:
                    continue
                if not chunk:
                    selector.unregister(key.fd)
                    continue
                if key.data == "stderr":
                    stderr_tail.extend(chunk)
                    if len(stderr_tail) > stderr_limit:
                        del stderr_tail[:-stderr_limit]
                    continue
                total += len(chunk)
                if total > limit:
                    _terminate_process_group(process)
                    raise DetectorError("ffmpeg process output exceeds bound")
                chunks.append(chunk)
        try:
            process.wait(timeout=MAX_PROCESS_CLEANUP_SECONDS)
        except subprocess.TimeoutExpired as error:
            _terminate_process_group(process)
            raise DetectorError("ffmpeg process cleanup exceeded bound") from error
    finally:
        selector.close()
        if process.returncode is None:
            _terminate_process_group(process)
        process.stdout.close()
        process.stderr.close()
    return b"".join(chunks), total, bytes(stderr_tail)


def _probe_video(path: Path, max_frames: int) -> tuple[int, int, list[float]]:
    ffprobe = shutil.which("ffprobe")
    if ffprobe is None:
        raise DetectorError("ffprobe is required for video presentation timestamps")
    process = subprocess.Popen(
        [
            ffprobe,
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_streams",
            "-show_frames",
            "-show_entries",
            "stream=width,height:frame=best_effort_timestamp_time",
            "-read_intervals",
            f"%+#{max_frames}",
            "-of",
            "json",
            str(path),
        ],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        start_new_session=True,
    )
    try:
        stdout, _, stderr_bytes = _read_bounded_process(process, MAX_STDOUT_BYTES)
    except DetectorError:
        raise
    if process.returncode != 0:
        detail = stderr_bytes.decode("utf-8", "replace").strip()[:240]
        raise DetectorError(f"ffprobe could not inspect video: {detail}")
    try:
        metadata = json.loads(stdout)
        stream = metadata["streams"][0]
        width, height = int(stream["width"]), int(stream["height"])
        raw_frames = metadata.get("frames", [])
    except (KeyError, IndexError, TypeError, ValueError, json.JSONDecodeError) as error:
        raise DetectorError("ffprobe returned incomplete video metadata") from error
    if not 1 <= width <= MAX_DIMENSION or not 1 <= height <= MAX_DIMENSION:
        raise DetectorError("video dimensions are outside the bounded range")
    if not isinstance(raw_frames, list) or len(raw_frames) > max_frames:
        raise DetectorError("ffprobe returned too many frames")
    timestamps: list[float] = []
    for item in raw_frames:
        try:
            timestamps.append(float(item["best_effort_timestamp_time"]))
        except (KeyError, TypeError, ValueError):
            timestamps.append(math.nan)
    return width, height, timestamps
def probe_video(path: str | Path, max_frames: int = MAX_FRAMES) -> tuple[int, int, list[float]]:
    """Return bounded video dimensions and recorded presentation timestamps."""
    if max_frames < 1 or max_frames > MAX_FRAMES:
        raise DetectorError("max_frames is outside the bounded range")
    return _probe_video(Path(path), max_frames)


def _crop_rgb(payload: bytes | bytearray, frame_width: int, roi: MarkerROI) -> bytes:
    row_bytes = frame_width * 3
    cropped = bytearray(roi.width * roi.height * 3)
    output_offset = 0
    for row in range(roi.y, roi.y + roi.height):
        start = row * row_bytes + roi.x * 3
        end = start + roi.width * 3
        cropped[output_offset : output_offset + roi.width * 3] = payload[start:end]
        output_offset += roi.width * 3
    return bytes(cropped)


def _terminate_process_group(process: subprocess.Popen[bytes]) -> None:
    # Signal the owned group before reaping its leader. A dead leader can still
    # have descendants holding the capture pipes open.
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    try:
        process.wait(timeout=MAX_PROCESS_CLEANUP_SECONDS)
    except subprocess.TimeoutExpired as error:
        raise DetectorError("process cleanup exceeded bounded duration") from error


def _decode_video(path: Path, marker_roi: MarkerROI, max_frames: int) -> list[FrameSample]:
    ffmpeg = shutil.which("ffmpeg")
    if ffmpeg is None:
        raise DetectorError("ffmpeg is required to decode video")
    width, height, timestamps = _probe_video(path, max_frames)
    marker_roi.validate(FrameSample(0.0, width, height, b""))
    frame_size = width * height * 3
    if frame_size > MAX_FRAME_BYTES:
        raise DetectorError("decoded frame exceeds bounded size")
    roi_size = marker_roi.width * marker_roi.height * 3
    max_retained_frames = MAX_RETAINED_ROI_BYTES // roi_size
    if max_retained_frames < 1:
        raise DetectorError("marker ROI exceeds retained decode bound")
    process = subprocess.Popen(
        [
            ffmpeg,
            "-v",
            "error",
            "-i",
            str(path),
            "-map",
            "0:v:0",
            "-frames:v",
            str(max_frames),
            "-fps_mode",
            "passthrough",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
            "pipe:1",
        ],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        start_new_session=True,
    )
    assert process.stdout is not None
    assert process.stderr is not None
    stdout_fd = process.stdout.fileno()
    stderr_fd = process.stderr.fileno()
    os.set_blocking(stdout_fd, False)
    os.set_blocking(stderr_fd, False)
    selector = selectors.DefaultSelector()
    selector.register(stdout_fd, selectors.EVENT_READ, "stdout")
    selector.register(stderr_fd, selectors.EVENT_READ, "stderr")
    deadline = time.monotonic() + MAX_DECODE_SECONDS
    payload = bytearray()
    stderr_tail = bytearray()
    samples: list[FrameSample] = []
    try:
        while selector.get_map():
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise DetectorError("ffmpeg decode exceeded bounded duration")
            for key, _ in selector.select(min(1.0, remaining)):
                try:
                    chunk = os.read(key.fd, 1024 * 1024)
                except BlockingIOError:
                    continue
                if not chunk:
                    selector.unregister(key.fd)
                    continue
                if key.data == "stderr":
                    stderr_tail.extend(chunk)
                    if len(stderr_tail) > MAX_STDERR_BYTES:
                        del stderr_tail[:-MAX_STDERR_BYTES]
                    continue
                payload.extend(chunk)
                while len(payload) >= frame_size:
                    if len(samples) >= max_frames:
                        raise DetectorError("ffmpeg emitted frames beyond the requested bound")
                    if len(samples) >= max_retained_frames:
                        raise DetectorError("decoded marker ROIs exceed retained decode bound")
                    timestamp = timestamps[len(samples)] * 1000.0 if len(samples) < len(timestamps) else math.nan
                    frame = memoryview(payload)[:frame_size]
                    marker_pixels = _crop_rgb(frame, width, marker_roi)
                    frame_hash = _sha256(frame)
                    samples.append(
                        FrameSample(
                            timestamp,
                            width,
                            height,
                            marker_pixels,
                            str(path),
                            frame_hash,
                            marker_roi,
                            frame_hash,
                        )
                    )
                    del frame
                    del payload[:frame_size]
        if payload:
            raise DetectorError("truncated raw video frame")
    finally:
        selector.close()
        try:
            _terminate_process_group(process)
        finally:
            process.stdout.close()
            process.stderr.close()
    if process.returncode != 0:
        detail = bytes(stderr_tail).decode("utf-8", "replace").strip()[-240:]
        raise DetectorError(f"ffmpeg could not decode video: {detail}")
    if len(samples) != len(timestamps):
        raise DetectorError("ffmpeg and ffprobe frame counts differ; presentation timing is incomplete")
    return samples


def _read_timestamps(path: Path, frame_paths: Sequence[Path]) -> list[float]:
    if len(frame_paths) > MAX_FRAMES:
        raise DetectorError("frame count exceeds bound")
    try:
        data = path.read_bytes()
        if len(data) > 8 * 1024 * 1024:
            raise DetectorError("timestamp sidecar exceeds bound")
        raw = json.loads(data)
    except (OSError, json.JSONDecodeError) as error:
        raise DetectorError(f"invalid timestamp sidecar: {path}") from error
    if isinstance(raw, list):
        values = raw
    elif isinstance(raw, dict):
        values = []
        for item in frame_paths:
            key = item.name
            if key not in raw:
                raise DetectorError(f"timestamp sidecar lacks {key}")
            values.append(raw[key])
    else:
        raise DetectorError("timestamp sidecar must be a JSON array or filename map")
    if len(values) != len(frame_paths):
        raise DetectorError("timestamp sidecar count does not match frame count")
    try:
        return [float(value) for value in values]
    except (TypeError, ValueError) as error:
        raise DetectorError("timestamp sidecar contains non-numeric values") from error


def load_samples(
    source: str | Path,
    *,
    expected_cadence_ms: float,
    marker_roi: MarkerROI | None = None,
    max_frames: int = MAX_FRAMES,
    timestamps: str | Path | None = None,
) -> list[FrameSample]:
    path = Path(source)
    if not path.exists():
        raise DetectorError(f"input does not exist: {path}")
    if max_frames < 1 or max_frames > MAX_FRAMES:
        raise DetectorError("max_frames is outside the bounded range")
    if path.is_dir():
        paths = []
        for item in path.iterdir():
            if item.is_file() and item.suffix.lower() in {".ppm", ".pnm", ".png", ".jpg", ".jpeg", ".webp"}:
                if len(paths) == max_frames:
                    raise DetectorError("frame directory exceeds the bounded frame count")
                paths.append(item)
        paths.sort()
        times = _read_timestamps(Path(timestamps), paths) if timestamps is not None else [math.nan] * len(paths)
        samples: list[FrameSample] = []
        for index, item in enumerate(paths):
            width, height, pixels = _decode_image(item)
            try:
                if item.stat().st_size > MAX_FRAME_BYTES:
                    raise DetectorError(f"frame exceeds {MAX_FRAME_BYTES} bytes: {item}")
                raw = item.read_bytes()
            except OSError as error:
                raise DetectorError(f"unable to read frame: {item}") from error
            samples.append(FrameSample(times[index], width, height, pixels, str(item), _sha256(raw), None, _sha256(pixels)))
        return samples
    if path.suffix.lower() in {".ppm", ".pnm"}:
        width, height, pixels = _decode_image(path)
        try:
            if path.stat().st_size > MAX_FRAME_BYTES:
                raise DetectorError(f"frame exceeds {MAX_FRAME_BYTES} bytes: {path}")
            raw = path.read_bytes()
        except OSError as error:
            raise DetectorError(f"unable to read frame: {path}") from error
        return [FrameSample(math.nan, width, height, pixels, str(path), _sha256(raw), None, _sha256(pixels))]
    if timestamps is not None:
        raise DetectorError("timestamp sidecar is only valid for frame directories")
    if marker_roi is None:
        raise DetectorError("marker ROI is required for bounded video decoding")
    return _decode_video(path, marker_roi, max_frames)

def _roi_bytes(frame: FrameSample, roi: MarkerROI) -> bytes:
    roi.validate(frame)
    if frame.decoded_roi is not None:
        if frame.decoded_roi != roi:
            raise DetectorError("decoded marker ROI does not match the requested marker ROI")
        expected_size = roi.width * roi.height * 3
        if len(frame.pixels) != expected_size:
            raise DetectorError("decoded marker ROI is truncated")
        return frame.pixels
    return _crop_rgb(frame.pixels, frame.width, roi)


def _luma(data: bytes) -> float:
    if not data:
        return 0.0
    return sum((0.2126 * data[index] + 0.7152 * data[index + 1] + 0.0722 * data[index + 2]) for index in range(0, len(data), 3)) / (len(data) // 3)


def _mae(left: bytes, right: bytes) -> float:
    if len(left) != len(right) or not left:
        return math.inf
    return sum(abs(a - b) for a, b in zip(left, right)) / len(left)


def analyze_frames(
    frames: Sequence[FrameSample],
    config: DetectorConfig,
    *,
    reference: FrameSample | None = None,
    capture_dropped: int = 0,
    capture_missing: bool = False,
) -> dict[str, object]:
    """Analyze successive displayed samples without mutating any external state."""
    config.validate()
    if not frames:
        return {"status": "INCONCLUSIVE", "reason": "no_frames", "frame_count": 0, "frames": []}
    if len(frames) > MAX_FRAMES:
        raise DetectorError("frame count exceeds bound")
    baseline = reference or frames[0]
    config.marker_roi.validate(baseline)
    baseline_marker = _roi_bytes(baseline, config.marker_roi)
    timing_missing = any(not math.isfinite(frame.timestamp_ms) for frame in frames)
    if timing_missing:
        capture_missing = True
    frame_reports: list[dict[str, object]] = []
    gaps: list[float] = []
    marker_losses: list[dict[str, object]] = []
    blank_intervals: list[dict[str, object]] = []
    dimensions = (frames[0].width, frames[0].height)
    for index, frame in enumerate(frames):
        if (frame.width, frame.height) != dimensions:
            raise DetectorError("frame dimensions changed without a resize sample")
        marker = _roi_bytes(frame, config.marker_roi)
        marker_luma = _luma(marker)
        difference = _mae(marker, baseline_marker)
        present = difference <= config.marker_tolerance and marker_luma > config.dark_luma_threshold
        timestamp = round(frame.timestamp_ms, 3) if math.isfinite(frame.timestamp_ms) else None
        report: dict[str, object] = {
            "index": index,
            "timestamp_ms": timestamp,
            "path": frame.path,
            "sha256": frame.sha256,
            "width": frame.width,
            "height": frame.height,
            "marker_luma": round(marker_luma, 3),
            "marker_mae_from_reference": round(difference, 3) if math.isfinite(difference) else None,
            "marker_present": present,
        }
        frame_reports.append(report)
        if not present:
            marker_losses.append({"index": index, "timestamp_ms": timestamp})
            next_time = frames[index + 1].timestamp_ms if index + 1 < len(frames) else math.nan
            start = frame.timestamp_ms
            end = next_time if math.isfinite(next_time) else (start + config.expected_cadence_ms if math.isfinite(start) else math.nan)
            blank_intervals.append(
                {
                    "start_ms": round(start, 3) if math.isfinite(start) else None,
                    "end_ms": round(end, 3) if math.isfinite(end) else None,
                    "duration_ms": round(max(0.0, end - start), 3) if math.isfinite(start) and math.isfinite(end) else None,
                }
            )
        if index:
            gap = frame.timestamp_ms - frames[index - 1].timestamp_ms
            gaps.append(gap)
    for gap in gaps:
        if (
            not math.isfinite(gap)
            or gap <= 0
            or abs(gap - config.expected_cadence_ms) > config.expected_cadence_ms * CADENCE_TOLERANCE_FRACTION
        ):
            capture_missing = True
    finite_times = [frame.timestamp_ms for frame in frames if math.isfinite(frame.timestamp_ms)]
    observed_duration = max(finite_times) - min(finite_times) if finite_times else math.nan
    # A declared dropped count is evidence quality, not a successful run.
    inconclusive_reasons: list[str] = []
    if capture_dropped:
        inconclusive_reasons.append("dropped_capture_frames")
    if capture_missing:
        inconclusive_reasons.append("missing_or_irregular_cadence")
    if len(frames) < config.min_samples:
        inconclusive_reasons.append("inadequate_samples")
    if math.isfinite(observed_duration) and observed_duration < config.min_duration_ms:
        inconclusive_reasons.append("inadequate_duration")
    if marker_losses:
        status = "FAIL"
        reason = "marker_loss_or_blank_interval"
    elif inconclusive_reasons:
        status = "INCONCLUSIVE"
        reason = ",".join(inconclusive_reasons)
    else:
        status = "PASS"
        reason = "stable_reference_marker"
    finite_gaps = [gap for gap in gaps if math.isfinite(gap)]
    reference_info = {
        "path": baseline.path,
        "sha256": baseline.sha256,
        "marker_luma": round(_luma(baseline_marker), 3),
    }
    return {
        "status": status,
        "observed_duration_ms": observed_duration if math.isfinite(observed_duration) else None,
        "reason": reason,
        "frame_count": len(frames),
        "dimensions": {"width": dimensions[0], "height": dimensions[1]},
        "expected_cadence_ms": config.expected_cadence_ms,
        "observed_cadence_ms": {"min": min(finite_gaps) if finite_gaps else None, "max": max(finite_gaps) if finite_gaps else None},
        "successive_frame_deltas_ms": [round(gap, 3) if math.isfinite(gap) else None for gap in gaps],
        "dropped_capture_frames": capture_dropped,
        "capture_missing": capture_missing,
        "marker_roi": {"x": config.marker_roi.x, "y": config.marker_roi.y, "width": config.marker_roi.width, "height": config.marker_roi.height},
        "marker_reference": reference_info,
        "stable_marker_control": not marker_losses,
        "marker_losses": marker_losses,
        "blank_intervals": blank_intervals,
        "frames": frame_reports,
    }


def detect_temporal_stability(
    source: str | Path,
    *,
    marker_roi: MarkerROI,
    reference: str | Path | None = None,
    expected_cadence_ms: float = 16.667,
    max_frames: int = MAX_FRAMES,
    capture_dropped: int = 0,
    capture_missing: bool = False,
    timestamps: str | Path | None = None,
    min_duration_ms: float = 30_000.0,
) -> dict[str, object]:
    config = DetectorConfig(marker_roi, expected_cadence_ms=expected_cadence_ms, min_duration_ms=min_duration_ms)
    config.validate()
    samples = load_samples(
        source,
        expected_cadence_ms=expected_cadence_ms,
        marker_roi=marker_roi,
        max_frames=max_frames,
        timestamps=timestamps,
    )
    reference_sample = None
    if reference is not None:
        reference_path = Path(reference)
        width, height, pixels = _decode_image(reference_path)
        reference_sample = FrameSample(0.0, width, height, pixels, str(reference_path), _sha256(reference_path.read_bytes()))
    return analyze_frames(samples, config, reference=reference_sample, capture_dropped=capture_dropped, capture_missing=capture_missing)


def _parse_roi(raw: str) -> MarkerROI:
    try:
        values = [int(part) for part in raw.split(",")]
    except ValueError as error:
        raise DetectorError("--marker-roi must be x,y,width,height") from error
    if len(values) != 4:
        raise DetectorError("--marker-roi must be x,y,width,height")
    return MarkerROI(*values)


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", required=True, help="bounded frame directory or ffmpeg-readable video")
    parser.add_argument("--marker-roi", required=True, help="explicit x,y,width,height marker region")
    parser.add_argument("--reference", help="optional reference image; otherwise first frame is reference")
    parser.add_argument("--timestamps", help="JSON timestamp sidecar required for frame directories")
    parser.add_argument("--expected-cadence-ms", type=float, default=16.667)
    parser.add_argument("--min-duration-ms", type=float, default=30_000.0, help="acceptance duration floor; default 30000 (30 seconds)")
    parser.add_argument("--max-frames", type=int, default=MAX_FRAMES)
    parser.add_argument("--dropped-frames", type=int, default=0)
    parser.add_argument("--missing-capture", action="store_true")
    parser.add_argument("--report", help="optional JSON report path")
    args = parser.parse_args(argv)
    try:
        if args.dropped_frames < 0:
            raise DetectorError("--dropped-frames cannot be negative")
        if args.min_duration_ms < 30_000:
            raise DetectorError("--min-duration-ms must be at least 30000 for acceptance CLI")
        report = detect_temporal_stability(
            args.input,
            marker_roi=_parse_roi(args.marker_roi),
            reference=args.reference,
            timestamps=args.timestamps,
            expected_cadence_ms=args.expected_cadence_ms,
            min_duration_ms=args.min_duration_ms,
            max_frames=args.max_frames,
            capture_dropped=args.dropped_frames,
            capture_missing=args.missing_capture,
        )
    except (DetectorError, OSError) as error:
        print(f"temporal_detector: rejected: {error}", file=sys.stderr)
        return 2
    rendered = json.dumps(report, indent=2, sort_keys=True, allow_nan=False)
    print(rendered)
    if args.report:
        report_path = Path(args.report)
        report_path.write_text(rendered + "\n", encoding="utf-8")
    return {"PASS": 0, "FAIL": 1, "INCONCLUSIVE": 2}[str(report["status"])]


if __name__ == "__main__":
    raise SystemExit(main())
