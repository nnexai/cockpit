# PERF-01 — Prove sustained whole application responsiveness

Status, dependencies, owner, locks and completion proof: [task ledger](../tasks.json). Follow the [orchestrator contract](../ORCHESTRATOR.md).

## Outcome

Sustained mixed daily use stays responsive, preserves scroll position, and has bounded resource/request behavior across the integrated application—not merely an empty fixture.

## Evidence and starting points

User priority: good performance, clean scrolling, no random errors. Historical [#1](https://github.com/nnexai/cockpit/issues/1) records request storms; [#9](https://github.com/nnexai/cockpit/issues/9) and [#12](https://github.com/nnexai/cockpit/issues/12) record hidden-renderer churn. Existing temporal helpers and the inline plan's Performance proof section provide a starting method, not current passing numbers.

## Changes

1. Use RUN-01's fixed workloads/build/viewport and measurement method. Record warm-up, sampling interval, host load and browser/native runtime. Take before/after measurements for repairs; do not optimize by intuition alone.
2. Exercise 10 tabs with repeated 60-switch bursts, split terminals with sustained output, a scrolled-back viewport, Files/Review long content within configured limits, picker/search cancellation and repeated source-resource opening.
3. Run static-idle, animation, wheel/trackpad, rapid typing, resize, reconnect and repeated Hide/Show browser workloads, including a deliberately slow observer. Run at least five minutes after warm-up for the animated/reconnect case and repeat a bounded mixed workflow.
4. Measure live terminal streams/renderers, owned helper/browser processes, frame/decoder/object/queue counts, request rate, CPU/PSS and input-to-presented-response latency. Use in-process timing or calibrated clocks; counts and memory samples must identify processes/units.
5. Find the earliest violated invariant, repair in the owning bounded task, and rerun affected plus integrated measurements. Do not add global throttles that hide losses or sacrifice normal input.

## Non-goals

No permanent telemetry service, benchmarking framework, transport rewrite, speculative virtualization, or arbitrary complexity cleanup. Performance cannot be improved by disabling supported behavior, swallowing errors, or silently lowering budgets.

## Acceptance

1. Terminal stream/renderer count follows selected visible panes and returns to baseline; repeated visits do not cause linear retained growth or hidden attach retries. No routine per-Space worktree-list storm returns.
2. Browser frame/input queues obey declared finite bounds; static idle does not continuously screenshot/poll, hidden capture is stopped, and a slow observer does not stall the controller.
3. After warm-up, retained object/buffer/process counts plateau over the sustained run; report memory slope/range and investigate monotonic retained growth rather than requiring exact byte equality.
4. No unintended blank frames, tail jumps, duplicate/lost wheel gestures, or old-resource alerts occur during the mixed workload.
5. Starting browser targets from the existing plan: click-to-visible p95 <150 ms and title/URL/cursor feedback <250 ms on the controlled local fixture. These are targets, not historical measurements. RUN-01 must freeze exact sampling and any explicit evidence-backed target decision before repairs; no post-hoc pass by changing thresholds.
6. Record terminal scroll latency and long-file responsiveness against the frozen baseline/budgets. Both browser and Linux native are exercised; macOS results are cross-referenced from NATIVE-02 or separately measured without implying equivalence.

## Verification

Use actual presented frames/video and input/event receipts, not only canvas buffers or request success. Reuse terminal temporal detector with an injected-blank negative control where applicable. At least 100 interaction samples are needed for reported p95 figures; report exclusions. Run under named disposable sessions/config/profiles; compare final owned-resource counts after cleanup. UI/transport review is required for performance repairs.

## Handoff

Commit a compact performance evidence report with fixture definition, source/build identities, sample counts, measurements, graphs/raw-artifact hashes, target disposition, repaired task hashes and cleanup. Mark the task done only when budgets/invariants pass; source-level optimizations alone are not evidence.
