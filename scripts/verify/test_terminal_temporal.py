#!/usr/bin/env python3
"""Focused pure trace guards for the bounded G01 replay harness."""
from __future__ import annotations
import hashlib
import json
import sys
import tempfile
import unittest
from pathlib import Path

try:
    from .terminal_temporal import (
        TemporalHarnessError,
        _load_trace,
        _paired_latencies,
        build_parser,
        first_matching_frame_after,
        verify_trace,
        visible_latency_ms,
    )
    from .temporal_detector import FrameSample
except ImportError:  # Direct invocation from scripts/verify.
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    from terminal_temporal import (  # type: ignore[no-redef]
        TemporalHarnessError,
        _load_trace,
        _paired_latencies,
        build_parser,
        first_matching_frame_after,
        verify_trace,
        visible_latency_ms,
    )
    from temporal_detector import FrameSample  # type: ignore[no-redef]


def _frames(payloads: list[bytes], start: float = 1000.0, step: float = 16.667) -> tuple[list[FrameSample], list[str]]:
    frames = [FrameSample(start + index * step, 1, 1, payload) for index, payload in enumerate(payloads)]
    return frames, [hashlib.sha256(payload).hexdigest() for payload in payloads]


class TerminalTemporalTraceTests(unittest.TestCase):
    def test_first_matching_frame_is_after_input_and_latency_uses_recorded_pts(self) -> None:
        frames, hashes = _frames([b"before", b"visible", b"later"])
        index = first_matching_frame_after(frames, hashes, 1001.0, hashes[1])
        self.assertEqual(index, 1)
        self.assertAlmostEqual(visible_latency_ms(frames, index, 1001.0) or -1, 15.667, places=3)

    def test_away_from_tail_requires_every_presented_hash_to_be_unchanged(self) -> None:
        frames, hashes = _frames([b"a", b"b", b"c"])
        sample = {
            "input_timestamp": 1000.0,
            "before_viewport_hash": hashes[0],
            "expected_viewport_hash": hashes[0],
            "expected_authoritative_offset": 4,
            "observed_authoritative_offset": 4,
        }
        result = verify_trace(
            frames,
            hashes,
            {"clock": "unix_ms", "samples": [sample], "away_from_tail_intervals": [{"start_ms": 1000, "end_ms": 1030}]},
            scenario="text",
        )
        self.assertEqual(result["status"], "FAIL")
        self.assertIn("away_from_tail_viewport_changed", result["reason"])

    def test_scroll_bound_rejects_duplicate_input_ids(self) -> None:
        payloads = [bytes((index % 251,)) for index in range(101)]
        frames, hashes = _frames(payloads)
        samples = [
            {
                "event_id": "duplicate",
                "input_timestamp": 1000.0 + index * 16.667 - 1,
                "before_viewport_hash": hashes[max(0, index - 1)],
                "expected_viewport_hash": hashes[index],
                "expected_authoritative_offset": index,
                "observed_authoritative_offset": index,
            }
            for index in range(101)
        ]
        result = verify_trace(
            frames,
            hashes,
            {"clock": "unix_ms", "samples": samples},
            scenario="scroll",
            paired_control=[16.667] * 101,
        )
        self.assertEqual(result["distinct_input_samples"], 1)
        self.assertEqual(result["status"], "INCONCLUSIVE")
        self.assertIn("paired_control_missing_provenance", result["reason"])
        self.assertIn("insufficient_distinct_motion_samples", result["reason"])

    def test_repeated_ab_viewport_motion_uses_immediate_before_frame(self) -> None:
        frames, hashes = _frames([b"a", b"b", b"a", b"b"])
        samples = [
            {
                "input_timestamp": 1000.1,
                "before_viewport_hash": hashes[0],
                "expected_viewport_hash": hashes[1],
                "expected_authoritative_offset": 1,
                "observed_authoritative_offset": 1,
            },
            {
                "input_timestamp": 1033.5,
                "before_viewport_hash": hashes[2],
                "expected_viewport_hash": hashes[3],
                "expected_authoritative_offset": 3,
                "observed_authoritative_offset": 3,
            },
        ]
        trace = {"clock": "unix_ms", "samples": samples}
        result = verify_trace(frames, hashes, trace, scenario="text")
        self.assertEqual(result["status"], "PASS")

        samples[1]["before_viewport_hash"] = hashes[1]
        result = verify_trace(frames, hashes, trace, scenario="text")
        self.assertNotEqual(result["status"], "PASS")
        self.assertIn("invalid_trace_samples", result["reason"])

    def test_trace_rejects_missing_fixture_identity(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "trace.json"
            path.write_text(json.dumps({"clock": "unix_ms", "samples": [{"input_timestamp": 1}]}), encoding="utf-8")
            with self.assertRaises(TemporalHarnessError):
                _load_trace(path)

    def test_capture_parser_requires_fixture_identity(self) -> None:
        with self.assertRaises(SystemExit):
            build_parser().parse_args(
                [
                    "capture",
                    "--ledger", "/owned/ledger.json",
                    "--run-id", "run-1",
                    "--session", "session-1",
                    "--client", "browser",
                    "--display", ":191",
                    "--output", "/owned/capture.ffv1",
                ]
            )

    def test_paired_latencies_accepts_emitted_text_only_replay_report(self) -> None:
        scroll_source = {"path": "/owned/scroll.ffv1", "sha256": "a" * 64, "size_bytes": 12}
        scroll_trace = {"path": "/owned/scroll-trace.json", "sha256": "b" * 64, "size_bytes": 34}
        control_source = {"path": "/owned/control.ffv1", "sha256": "c" * 64, "size_bytes": 56}
        control_trace = {"path": "/owned/control-trace.json", "sha256": "d" * 64, "size_bytes": 78}
        trace = {
            "clock": "unix_ms",
            "fixture_id": "fixture-1",
            "run_id": "run-1",
            "session": "session-1",
            "client": "browser",
            "source_identity": scroll_source,
            "trace_identity": scroll_trace,
        }
        samples = [
            {
                "event_id": f"event-{index}",
                "input_timestamp_unix_ms": 1000.0 + index,
                "before_viewport_hash": hashlib.sha256(f"before-{index}".encode()).hexdigest(),
                "expected_viewport_hash": hashlib.sha256(f"after-{index}".encode()).hexdigest(),
                "latency_ms": float(index) / 2,
            }
            for index in range(100)
        ]
        report = {
            "kind": "G01 terminal temporal replay",
            "status": "PASS",
            "scenario": "text-only",
            "fixture_id": "fixture-1",
            "run_id": "run-1",
            "session": "session-1",
            "client": "browser",
            "source_identity": control_source,
            "trace_identity": control_trace,
            "sampled_latencies": {"samples": samples},
        }
        values, error = _paired_latencies(report, trace)
        self.assertIsNone(error)
        self.assertEqual(len(values), 100)

        samples[1]["event_id"] = samples[0]["event_id"]
        _, error = _paired_latencies(report, trace)
        self.assertEqual(error, "paired_control_duplicate_samples")

        samples[1]["event_id"] = "event-1"
        report["client"] = "native"
        _, error = _paired_latencies(report, trace)
        self.assertEqual(error, "paired_control_identity_mismatch")

    def test_measured_scroll_budget_violations_fail_instead_of_appearing_inconclusive(self) -> None:
        for step, control_latency in ((200.0, 180.0), (80.0, 40.0)):
            with self.subTest(step=step, control_latency=control_latency):
                frames, hashes = _frames([b"a", b"b"] * 50 + [b"a"], step=step)
                samples = [{
                    "event_id": f"wheel-{index}",
                    "input_timestamp_unix_ms": frames[index].timestamp_ms + 1,
                    "before_viewport_hash": hashes[index],
                    "expected_viewport_hash": hashes[index + 1],
                    "expected_authoritative_offset": index + 1,
                    "observed_authoritative_offset": index + 1,
                } for index in range(100)]
                identity = {"run_id": "run-1", "session": "owned-1", "client": "browser", "fixture_id": "fixture-1"}
                trace = {
                    **identity, "clock": "unix_ms", "samples": samples,
                    "expected_final_authoritative_offset": 100, "observed_final_authoritative_offset": 100,
                    "away_from_tail_intervals": [{
                        "start_ms": 999, "end_ms": 1000, "away_from_tail": True,
                        "expected_authoritative_offset": 0, "observed_authoritative_offset": 0,
                    }],
                }
                control = {
                    **identity, "kind": "G01 terminal temporal replay", "scenario": "text-only", "status": "PASS",
                    "source_identity": {"path": "/owned/control.mkv", "sha256": "a" * 64, "size_bytes": 1},
                    "trace_identity": {"path": "/owned/control.json", "sha256": "b" * 64, "size_bytes": 1},
                    "sampled_latencies": {"samples": [{**sample, "latency_ms": control_latency} for sample in samples]},
                }
                frames.insert(0, FrameSample(999, 1, 1, b"a"))
                hashes.insert(0, hashes[0])
                result = verify_trace(frames, hashes, trace, scenario="scroll", paired_control=control)
                self.assertEqual(result["usable_motion_samples"], 100)
                self.assertEqual(result["status"], "FAIL")
                self.assertNotIn("invalid_trace_samples", result["reason"])


if __name__ == "__main__":
    unittest.main()
