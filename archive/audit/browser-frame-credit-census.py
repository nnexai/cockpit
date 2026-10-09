#!/usr/bin/env python3
"""Count frame-credit progress by controller and observer from run evidence."""

import json
import sys
from pathlib import Path


DEFAULT_EVIDENCE = Path(
    "planning/stability-and-gitlab-2026-09-20/runs/run-20260920-a3e9b950/"
    "final-protocol-boundaries.json"
)


def main() -> int:
    evidence_path = Path(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT_EVIDENCE
    evidence = json.loads(evidence_path.read_text())
    transport = evidence["frame_transport"]
    sequences = transport["controller_sequences"]
    credits = transport["credits"]
    observer_frames = transport["observer_received_binary_frames"]
    observer_credits = int(transport["observer_credit_sent"])
    controller_balanced = len(sequences) == len(credits)
    print(json.dumps({
        "controller": {
            "frames": len(sequences),
            "sequences": sequences,
            "credits": len(credits),
            "balanced": controller_balanced,
        },
        "observer": {
            "frames": observer_frames,
            "credits": observer_credits,
            "withheld_credit": observer_frames > observer_credits,
        },
        "controller_progressed_while_observer_unacked": transport[
            "controller_progress_while_observer_unacked"
        ],
    }, sort_keys=True))
    return 0 if controller_balanced and transport[
        "controller_progress_while_observer_unacked"
    ] else 1


if __name__ == "__main__":
    raise SystemExit(main())
