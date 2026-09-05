#!/usr/bin/env python3
"""Pure regression cases for the temporal detector's negative controls."""

from __future__ import annotations
import json
import os
import signal
import time
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

try:
    from . import temporal_detector
    from .temporal_detector import DetectorConfig, DetectorError, FrameSample, MarkerROI, _decode_ppm, analyze_frames, load_samples
except ImportError:  # Direct ``python scripts/verify/test_temporal_detector.py``.
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import temporal_detector
    from temporal_detector import DetectorConfig, DetectorError, FrameSample, MarkerROI, _decode_ppm, analyze_frames, load_samples  # type: ignore[no-redef]


WIDTH = 8
HEIGHT = 4
ROI = MarkerROI(0, 0, 4, 1)
CONFIG = DetectorConfig(ROI, expected_cadence_ms=16.667, marker_tolerance=4.0, min_duration_ms=0)


def frame(timestamp: float, *, blank: bool = False) -> FrameSample:
    marker = bytes((0, 0, 0) if blank else (255, 255, 255)) * 4
    body = bytes((25, 35, 45)) * ((WIDTH * HEIGHT) - 4)
    pixels = marker + body
    return FrameSample(timestamp, WIDTH, HEIGHT, pixels)


class TemporalDetectorTests(unittest.TestCase):
    def test_stable_reference_sequence_passes(self) -> None:
        result = analyze_frames([frame(index * 16.667) for index in range(4)], CONFIG)
        self.assertEqual(result["status"], "PASS")
        self.assertEqual(result["frame_count"], 4)
        self.assertEqual(result["marker_losses"], [])

    def test_single_blanked_frame_fails_even_when_surrounding_frames_are_stable(self) -> None:
        result = analyze_frames([frame(0), frame(16.667, blank=True), frame(33.334), frame(50.001)], CONFIG)
        self.assertEqual(result["status"], "FAIL")
        self.assertEqual(result["reason"], "marker_loss_or_blank_interval")
        self.assertEqual([item["index"] for item in result["marker_losses"]], [1])

    def test_hundred_millisecond_blank_interval_is_reported(self) -> None:
        result = analyze_frames([frame(0), frame(16.667, blank=True), frame(116.667)], CONFIG)
        self.assertEqual(result["status"], "FAIL")
        self.assertEqual(round(result["blank_intervals"][0]["duration_ms"], 3), 100.0)
        self.assertEqual(result["blank_intervals"][0]["start_ms"], 16.667)

    def test_inadequate_samples_are_inconclusive_not_pass(self) -> None:
        result = analyze_frames([frame(0), frame(16.667)], CONFIG)
        self.assertEqual(result["status"], "INCONCLUSIVE")
        self.assertIn("inadequate_samples", result["reason"])

    def test_dropped_capture_is_inconclusive_when_marker_is_stable(self) -> None:
        result = analyze_frames([frame(index * 16.667) for index in range(4)], CONFIG, capture_dropped=1)
        self.assertEqual(result["status"], "INCONCLUSIVE")
        self.assertIn("dropped_capture_frames", result["reason"])

    def test_missing_timestamps_are_inconclusive(self) -> None:
        result = analyze_frames([frame(float("nan")) for _ in range(4)], CONFIG)
        self.assertEqual(result["status"], "INCONCLUSIVE")
        self.assertIn("missing_or_irregular_cadence", result["reason"])

    def test_nonmonotonic_timestamps_are_inconclusive(self) -> None:
        result = analyze_frames([frame(0), frame(33.334), frame(16.667), frame(50.001)], CONFIG)
        self.assertEqual(result["status"], "INCONCLUSIVE")
        self.assertIn("missing_or_irregular_cadence", result["reason"])

    def test_truncated_ppm_is_rejected(self) -> None:
        with self.assertRaises(DetectorError):
            _decode_ppm(b"P6\n2 1\n255\n\xff")

    def test_supplied_presentation_timestamps_drive_cadence_report(self) -> None:
        result = analyze_frames([frame(index * 1000.0) for index in range(4)], DetectorConfig(ROI, expected_cadence_ms=1000, min_duration_ms=0))
        self.assertEqual(result["status"], "PASS")
        self.assertEqual(result["successive_frame_deltas_ms"], [1000.0, 1000.0, 1000.0])

    @unittest.skipUnless(shutil.which("ffmpeg") and shutil.which("ffprobe"), "ffmpeg and ffprobe are required for video decoder coverage")
    def test_video_cli_uses_recorded_pts_and_retains_only_marker_roi(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            video = Path(temporary) / "five-fps.mkv"
            created = subprocess.run(
                [
                    shutil.which("ffmpeg"),
                    "-y",
                    "-v",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    "color=c=white:s=32x32:r=5:d=1",
                    "-frames:v",
                    "5",
                    "-c:v",
                    "ffv1",
                    str(video),
                ],
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )
            self.assertEqual(created.returncode, 0, created.stderr.decode("utf-8", "replace"))

            samples = load_samples(video, expected_cadence_ms=16.6666667, marker_roi=MarkerROI(0, 0, 4, 4))
            self.assertEqual(len(samples), 5)
            self.assertTrue(all((sample.width, sample.height) == (32, 32) for sample in samples))
            self.assertTrue(all(sample.decoded_roi == MarkerROI(0, 0, 4, 4) for sample in samples))
            self.assertTrue(all(len(sample.pixels) == 4 * 4 * 3 for sample in samples))

            completed = subprocess.run(
                [
                    sys.executable,
                    str(Path(__file__).with_name("temporal_detector.py")),
                    "--input",
                    str(video),
                    "--marker-roi",
                    "0,0,4,4",
                    "--expected-cadence-ms",
                    "16.6666667",
                ],
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                check=False,
            )
            self.assertEqual(completed.returncode, 2, completed.stderr)
            self.assertNotIn("Traceback", completed.stderr)
            report = json.loads(completed.stdout)
            self.assertEqual(report["status"], "INCONCLUSIVE")
            self.assertIn("missing_or_irregular_cadence", report["reason"])
            self.assertEqual(report["successive_frame_deltas_ms"], [200.0, 200.0, 200.0, 200.0])

    def test_oversized_frame_directory_cannot_silently_pass_a_prefix(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            payload = b"P6\n8 4\n255\n" + frame(0).pixels
            for name in ("first.ppm", "second.ppm"):
                (Path(directory) / name).write_bytes(payload)
            with self.assertRaises(DetectorError):
                load_samples(directory, expected_cadence_ms=16.667, marker_roi=ROI, max_frames=1)

    @unittest.skipUnless(sys.platform == "linux", "owned descendant inspection requires Linux procfs")
    def test_exited_decoder_parent_cannot_leave_a_pipe_holding_descendant(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            receipt = Path(directory) / "child.pid"
            program = (
                "import os,sys,time,signal\n"
                "child=os.fork()\n"
                "if child:\n"
                " with open(sys.argv[1]+'.tmp','w') as receipt: receipt.write(str(child))\n"
                " os.replace(sys.argv[1]+'.tmp',sys.argv[1])\n"
                " os._exit(0)\n"
                "signal.signal(signal.SIGTERM,signal.SIG_IGN)\n"
                "time.sleep(60)\n"
            )
            process = subprocess.Popen([sys.executable, "-c", program, str(receipt)], stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
            child = None
            try:
                deadline = time.monotonic() + 3
                while not receipt.exists() and time.monotonic() < deadline:
                    time.sleep(0.01)
                child = int(receipt.read_text())
                with patch.object(temporal_detector, "MAX_DECODE_SECONDS", 0.1):
                    with self.assertRaises(DetectorError):
                        temporal_detector._read_bounded_process(process, 1024)
                self.assertIsNotNone(process.returncode)
                state = Path(f"/proc/{child}/stat")
                def child_running() -> bool:
                    try:
                        return state.read_text().rsplit(")", 1)[1].split()[0] != "Z"
                    except FileNotFoundError:
                        return False
                deadline = time.monotonic() + 3
                while child_running() and time.monotonic() < deadline:
                    time.sleep(0.01)
                self.assertFalse(child_running())
            finally:
                try:
                    if process.returncode is None or (child is not None and os.getpgid(child) == process.pid):
                        os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                process.wait(timeout=3)
                if process.stdout is not None:
                    process.stdout.close()
                if process.stderr is not None:
                    process.stderr.close()


if __name__ == "__main__":
    unittest.main()
