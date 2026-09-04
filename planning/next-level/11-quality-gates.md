# CLEAN-05. Deterministic quality infrastructure

Status: proposed planning milestone; no implementation, package installation, or source change is included here.

Research basis: [maintainability quality-gate research](../../research/next-level-quality-gates.md). The research records the verified CRAP formula, coverage-tool behavior, mutation-tool behavior, and the current workstation probes.

## Purpose and boundary

CLEAN-05 gives future code-writing agents a repeatable answer to two questions: which changed functions became harder to test, and which executable behavior is not protected by the tests. It is a small local command set for this personal repository. It does not create a hosted CI service, an agent framework, a dashboard, a release gate, or a single overall code-health grade.

The default run is limited to the source files in the current Git diff. A full workspace run is a deliberate, opt-in command because mutation testing is expensive. Existing Herdr and UI runtime proof remains required for changes whose behavior crosses those boundaries. These metrics support review; they do not replace contract tests, real fixture runs, screenshots, or a disposable Herdr smoke.

The first implementation of CLEAN-05 must happen after CLEAN-01 has recorded the behavior baseline and before feature agents consume the new seams. Prime its report and gate before CLEAN-02/03 extractions; tool inventory and compatibility research may overlap CLEAN-01. FND, CTX, LIFE, and PANE feature work consume the integrated gate.

## Proposed files and ownership

Add the following only when CLEAN-05 is authorized:

| Path | Owning responsibility |
|---|---|
| `quality/tool-versions.json` | Exact Rust toolchain and quality-tool versions, package names, invocation mode, and probe date. Placeholder or floating versions are invalid. |
| `quality/config.json` | Include/exclude globs, threshold policy, timeout and worker limits, fixed fixture seed, and report schema version. |
| `quality/baseline.json` | Reviewed per-method and per-mutant baseline. It is updated by an explicit command and never overwritten by a normal gate run. |
| `scripts/quality/changed-files.mjs` | Resolve a fixed base plus staged, unstaged, and untracked worktree changes, canonicalize paths, hash file contents, and map changed tests to source scope without shell glob ambiguity. |
| `scripts/quality/collect.mjs` | Run the existing checks and the available metric providers, capture exit status and tool versions, and write one normalized report. |
| `scripts/quality/crap.mjs` | Join per-function complexity ranges with coverage ranges, calculate the declared CRAP variant, and emit missing-data states. TypeScript complexity comes from a pinned ESLint `complexity` JSON report with the classic variant; Rust complexity comes from the probed Rust adapter. |
| `scripts/quality/mutation.mjs` | Run the selected Rust or TypeScript mutation tool for the resolved changed scope, normalize outcomes, and account for elapsed cost. |
| `scripts/quality/gate.mjs` | Compare the report with `quality/baseline.json`, apply hard/warn/info rules, and return a stable exit code. |
| `quality/reports/` | Local generated JSON and human-readable reports, ignored by Git unless a report is intentionally attached to a review. |

The scripts should use Node's standard library and existing repository commands. Normal report/gate scripts must not install packages, change user configuration, start Herdr, or edit the working source tree. Mutation tools operate only in their isolated temporary copies. Tool installation is a separate documented setup step with exact versions. A future implementation may split the scripts into typed modules once the report contract is stable, but it should keep one obvious command entry point.

## Report contract

Every run writes JSON with a fixed top-level shape. The exact schema can be JSON Schema later, but the fields below are part of CLEAN-05's acceptance contract:

```json
{
  "schema_version": 1,
  "status": "pass|info|warn|fail|inconclusive",
  "commit": "full git sha",
  "base": "resolved comparison ref or null",
  "scope": { "kind": "changed|full", "files": [{ "path": "src/app/sessionReducer.ts", "change": "modified", "sha256": "sha256:...", "base_sha256": "sha256:..." }] },
  "toolchain": {
    "node": "v26.8.1",
    "bun": "1.3.14",
    "rust": "1.98.0",
    "tools": { "vitest": "4.1.10", "cargo_mutants": null }
  },
  "policy": {
    "crap_info": 6,
    "crap_warn": 8,
    "crap_hard": 30,
    "new_logic_coverage_floor": 0.90,
    "quality_seed": "fixed value from quality/config.json"
  },
  "checks": [],
  "methods": [],
  "mutants": [],
  "cost": { "started_at": "UTC ISO-8601", "duration_ms": 0, "test_runs": 0, "mutants_run": 0 },
  "diagnostics": []
}
```

Each `check` records `name`, `command` as an argv array, `scope`, `exit_code`, `duration_ms`, `status`, and a bounded stdout/stderr digest. Each method record uses a stable key made from language, canonical path, and qualified function name. The declaration range and a normalized signature hash are separate fields. Line ranges are relocation hints only and are never part of the identity, so an inserted line does not create a new method:

```json
{
  "key": "ts:src/app/sessionReducer.ts:sessionReducer",
  "language": "ts|rust",
  "path": "src/app/sessionReducer.ts",
  "symbol": "sessionReducer",
  "declaration": { "start_line": 11, "end_line": 309 },
  "signature_hash": "sha256:...",
  "complexity": 7,
  "coverage": { "kind": "line|region|branch", "covered": 92, "total": 100, "fraction": 0.92 },
  "crap": { "formula": "c^2*(1-cov)^3+c", "score": 7.025088, "status": "info|warn|hard|missing|inconclusive" },
  "baseline": { "crap": 6.8, "complexity": 6, "coverage_fraction": 0.94 }
}
```

`coverage` is `null` with `status: "missing"` or `"inconclusive"` when the provider did not run, a file was not included, a function could not be joined, or the coverage kind differs from the configured policy. The collector must never write `fraction: 0` to stand for missing coverage. A method with no executable statements is recorded as `not_applicable`, not as a fully covered or uncovered method.

Mutation records use one row per stable mutant ID and preserve the tool's raw classification:

```json
{
  "id": "rust:crates/cockpit-core/src/lib.rs:CockpitService::mutate:branch:site-hash",
  "language": "rust|ts",
  "path": "crates/cockpit-core/src/lib.rs",
  "symbol": "CockpitService::mutate",
  "status": "killed|survived|no_coverage|timeout|compile_error|skipped|inconclusive",
  "duration_ms": 421,
  "detail": "tool summary or bounded diagnostic"
}
```

Mutant identity includes the owning symbol, operator, and normalized mutation-site fingerprint; source locations are diagnostic hints. Preserve the tool ID separately. Ambiguous matching requires review rather than silently dropping an old survivor.

Only `killed` and `survived` enter the explicitly named `executed_score = killed / (killed + survived)`. The report also shows `adequacy_score = killed / (killed + survived + no_coverage)` when that denominator is nonzero, so uncovered mutants cannot disappear inside an executed-only percentage. `timeout`, `compile_error`, `skipped`, and `inconclusive` remain visible counts and make the run incomplete where appropriate. An unviable generated mutant is not silently counted as killed. A timeout or runner crash makes the run `inconclusive` and records the unfinished work. An empty executed denominator yields `executed_score: N/A`. An all-uncovered set has `adequacy_score: 0` and new uncovered mutants fail. If no eligible mutants exist at all, report `not_applicable` with the generated/excluded counts and reason; an unexpectedly empty provider result is inconclusive, never a silent pass.

## Metric policy and thresholds

The CRAP calculation uses the verified per-method form:

```text
CRAP(m) = c^2 * (1 - coverage_fraction)^3 + c
```

The configured coverage fraction must say whether it came from line, region, or branch data. The original CRAP4J description used automated basis-path coverage and a historical 30 warning point. Cockpit adopts a line/region adapter only where the provider can map the coverage ranges to the same function. No source in this plan attributes the metric to Uncle Bob.

The initial Cockpit policy is:

| Result | Condition on a changed method | Action |
|---|---|---|
| `info` | CRAP `>= 6` and `<= 8` | Show the method and its complexity/coverage in the report. No failure. Complexity 8 reaches this band only at full configured coverage. |
| `hard` | A new method has CRAP `> 8`; a changed legacy method has any score/coverage/complexity regression; or a changed scope introduces a `survived` or `no_coverage` mutant | Gate fails. Existing debt can remain only as an explicit baseline entry with no regression. |
| `warn` | Existing, unchanged legacy CRAP `> 8` and `<= 30`, or a baseline debt finding carried without regression | Keep visible and require an owner decision before a cleanup baseline update. New 9-30 methods never quietly pass. |
| `hard` | CRAP `> 30` | Gate fails for new or worsened code. An unchanged legacy finding may be triaged only through an explicit reviewed baseline exception. |
| `missing` | No trustworthy method coverage join | Gate is `inconclusive` in strict mode and cannot claim a passing maintainability result. |

The 6-8 band is an intentional local target. It is not a universal acceptable score, and a score below 6 is not proof of good design. Complexity and coverage remain visible separately so a high score can lead to a useful refactor or a behavior test rather than metric manipulation.

CRAP is not a coverage floor. A complexity-1 function with zero coverage would score only 2, so the changed-code policy separately requires at least 90% of executable lines or regions in newly added logic to be covered. Branch coverage is collected and displayed when the provider supports it, and becomes an additional warning signal during the baseline period. Touched legacy logic may keep its existing coverage only when it does not regress. A missing or unjoinable coverage report blocks strict gate readiness for the affected language; the separate complexity and coverage values remain available for diagnosis.

For mutations, the changed-scope gate fails on a newly surviving or newly `no_coverage` mutant. Existing baseline `no_coverage`, `compile_error`, or `skipped` outcomes remain warnings until they regress; a runner timeout or incomplete report is inconclusive. A mutation score is displayed only when at least one killed or survived mutant exists and the run is complete. The first baseline may contain existing survivors; the ratchet forbids adding survivors in changed code and requires an explicit reviewed baseline update to retire one.

These thresholds are shallow test signals. Adding assertions that do not check behavior, excluding difficult code, renaming functions to evade matching, or splitting one branch into noise is a policy violation. The agent handoff must include the relevant contract or fixture test and, when applicable, the real runtime proof required by the owning story.

## Deterministic changed-code gate

`changed-files.mjs` resolves the comparison base explicitly. A normal agent run receives `--base <commit-or-ref>` from the integration owner, and refuses to guess a remote branch. It reads the complete worktree state, including staged and unstaged tracked edits plus untracked source files, and uses NUL-delimited Git output and content hashes to avoid shell and index ambiguity. Deletions are retained as baseline findings; Git renames are paired when their similarity is unambiguous, otherwise the method mapping is inconclusive. The source scope includes `src/**/*.{ts,tsx}`, `crates/**/*.rs`, and `src-tauri/**/*.rs`; generated protocol output, tests alone, fixtures, mocks, dependencies, and build output are excluded from method mutation unless a config entry opts them in. A test-only diff reports the affected source as `test_only` and still runs the ordinary tests.

The gate compares the changed report with the report for the same methods in `quality/baseline.json`. It checks new methods, changed declaration ranges, and methods whose complexity or coverage changed. Unchanged historical debt is reported but does not block an unrelated change. A baseline entry is keyed by canonical path and qualified symbol, with line ranges and signature hashes used only as relocation hints. If a rename or extraction cannot be matched safely, the result is `inconclusive` and a human must choose the mapping.

The baseline workflow is explicit:

1. CLEAN-01 records behavior and ordinary-check evidence. CLEAN-05 captures the first metric baseline after tool probes, before ownership extractions, with exact tool versions.
2. `quality:gate -- --base <ref>` compares a changed run with that report. It never writes the baseline.
3. After a deliberate cleanup or an approved threshold exception, `quality:baseline -- --from quality/reports/<run>.json` writes a reviewed, sorted baseline. The command shows the diff and refuses to overwrite without an explicit `--write`.
4. The committed baseline records the commit, policy, tool versions, and incomplete-data exceptions. It cannot use `latest`, a floating package range, or an unexplained `ignore`.

This is a ratchet on changed risk, not a demand to repair every existing finding before feature work. The initial report may carry warnings. New work may not make the affected method or mutant set worse without a written review decision.

## Proposed commands

The following commands are interfaces to implement. They do not exist in the current checkout and must not be described as runnable until CLEAN-05 adds them and probes their providers:

```text
bun run quality:report -- --base <commit> --scope changed
bun run quality:gate -- --base <commit> --scope changed --strict
bun run quality:baseline -- --from quality/reports/<run>.json --write
bun run quality:mutate -- --language ts --scope changed
bun run quality:mutate -- --language rust --scope changed
bun run quality:report -- --scope full --include-mutation
```

The collector retains the existing checks as named checks rather than replacing them:

```text
bun run typecheck
bun run test
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace
cargo check -p cockpit-tauri
```

The future provider invocations are selected from the pinned tool manifest and recorded as argv in the report. The intended shapes are:

```text
node node_modules/vitest/vitest.mjs run --coverage --coverage.provider=v8
cargo llvm-cov --workspace --json --output-path <report>
cargo crap --format json <changed Rust scope>
cargo mutants --no-shuffle --json --output <report> --file <changed Rust path>
stryker run <pinned config with mutate=<changed TS files>, coverageAnalysis=perTest>
```

The exact provider flags must be verified against the selected versions during implementation. A command shown here is a proposed adapter contract, not evidence that the binary or package is installed.

The gate uses stable numeric exits so an agent can act without parsing prose: `0` means pass, informational findings, or acknowledged baseline warnings only, `1` means a hard threshold or ratchet violation, `2` means strict mode is inconclusive because coverage/mapping/provider evidence is missing, and `3` means invalid configuration, unsupported invocation, or an infrastructure error before a quality result exists. Non-strict report-only mode returns 0 after successfully collecting a report even if it contains findings; callers must use the strict gate for completion. An ordinary test/typecheck failure is exit 1 and cannot be hidden by a metrics result. The JSON `status` remains the detailed source of truth.

## Tool choices, cost, and fallback

TypeScript uses Vitest's V8 provider under Node because the Vitest 4 documentation says V8 coverage requires a V8 runtime and does not work on Bun. The package version must match the existing Vitest 4.1.10 entry. Istanbul is the fallback if the Node V8 probe fails. TypeScript cyclomatic complexity comes from a pinned ESLint `complexity` JSON report using the classic variant, with the function range and qualified symbol checked against the coverage map. ESLint is not currently installed, so a stable function-range join is an adoption probe and missing mapping makes TypeScript CRAP inconclusive. StrykerJS with its official Vitest runner is the mutation provider because its declared Vitest peer range includes 4.x. Pin Stryker core and runner to exact compatible versions in `quality/tool-versions.json`; do not use an unpinned `npx` initializer.

Rust uses `cargo-llvm-cov` for compiler-backed coverage. `cargo-crap` is the first CRAP adapter to probe, but the gate accepts only a report that joins complexity and coverage by function. If it cannot produce a trustworthy join, retain separate Rust complexity and coverage records and mark CRAP inconclusive. Do not add a second parser in CLEAN-05. `cargo-mutants` is the Rust mutation provider with `--no-shuffle` and changed-file filters. Its compile failures and unviable mutants remain distinct report outcomes.

The current readiness boundary is known. Rust 1.98.0, Node 26.8.1, Bun 1.3.14, and Vitest 4.1.10 are present. `bun run test` passes 3 files and 55 tests, and the Vitest executable reports Node 26.8.1. The Vitest coverage providers, Stryker packages, cargo-llvm-cov, cargo-crap, cargo-mutants, and a Rust complexity provider are absent. CLEAN-05 starts with a read-only `--probe` that checks exact executable versions, Vitest provider loading, JSON/LCOV output, changed-file filtering, and a one-function mutation fixture. It records blockers and exits with `inconclusive`; it does not install anything.

If a provider remains unavailable, the fast command still runs typecheck, tests, Rust checks, and any available static report. Strict maintainability status remains `inconclusive`, never a false pass. Feature agents may continue only when the integration owner accepts that evidence boundary and supplies the runtime checks required by the feature. Native Tauri runtime status remains separate; a native run may require platform development packages, which must be probed at the time of implementation rather than assumed from this plan.

Mutation work is cost-controlled without a hosted service:

- Changed scope is the default. Full workspace mutation is a manual or scheduled local command.
- Rust uses `--no-shuffle`, a fixed worker limit, and the repository's Rust toolchain. TypeScript uses a fixed Stryker concurrency and `coverageAnalysis: "perTest"` so covered tests are selected consistently.
- `QUALITY_SEED` is fixed in `quality/config.json` and exported to tests that generate data. The report records it. Stryker's public configuration does not establish a mutation-order seed, so the report records `tool-does-not-expose-one` rather than inventing a flag; deterministic fixtures, sorted IDs, fixed concurrency, and pinned versions provide the reproducible boundary.
- Every check records duration, test-run count, mutant count, timeout, and tool version. This makes an expensive scope visible and lets the owner shrink a run without silently changing the gate.
- Reports are local JSON. No dashboard, remote upload, or CI host is required.

## Luna-high execution loop

When this story is implemented, the integration owner assigns one diligent high-context execution lane to Luna-high. The lane reads CLEAN-01's baseline and this story, runs the probes, records exact tool versions, implements the report and gate scripts, and stops at the first missing-data boundary rather than guessing. A separate review pass checks formula math, changed-file mapping, status accounting, baseline behavior, and the no-gaming rules. The owner then runs the changed scope with a fixed base, reviews the JSON, and runs the full mutation command only as an explicit opt-in. The handoff names commands, report paths, changed scope, warnings, inconclusive providers, and any required Herdr/browser/native evidence.

## Dependencies and acceptance

Dependencies are CLEAN-01's behavior baseline and repository toolchain inventory. CLEAN-05 must land before feature agents begin. It precedes the CLEAN-02/03 ownership extractions and must not depend on future context or provider code. Integration order is: probe and pin tools, define and fixture-test the report schema, implement changed-file resolution, add coverage/complexity adapters, add mutation adapters, add baseline comparison, wire package scripts, then run the changed gate and review the report.

Acceptance requires all of the following:

- A clean checkout can run the proposed fast command with only the repository's documented toolchain, and the report states exactly which optional providers are unavailable.
- A fixture proves `CRAP = c^2 * (1-cov)^3 + c`, with the 6, 7, and 8 complexity examples and missing coverage cases. Missing coverage never becomes zero.
- A fixture proves changed-file scope, path canonicalization, baseline comparison, method relocation ambiguity, sorted output, and explicit baseline writes.
- Mutation fixtures or provider probes preserve killed, survived, no-coverage, timeout, compile-error, skipped, and inconclusive as separate statuses. The executed score uses only killed/survived, while the separately named adequacy score includes no-coverage in its denominator.
- Fixtures prove new-function CRAP >8 fails, the 6–8 band is informational, legacy >30 remains explicit debt, coverage below 90% in new logic fails, and newly surviving or uncovered mutants fail. Test exit codes and empty/all-uncovered mutant sets separately.
- Tool versions, fixed seed policy, concurrency, duration, and test/mutant counts appear in JSON. Full runs are opt-in and do not require a CI host or remote dashboard.
- `bun run typecheck`, `bun run test`, Rust format/check/test commands, and the explicit Tauri compile check remain named and independently visible. A browser check or native compile is never presented as native runtime proof.
- The implementation does not add a general quality framework, shallow metric tests, automatic source edits, hosted reporting, or a generic agent plugin surface.

## Small architecture checks and agent handoff

Enforce only boundaries the cleanup actually establishes. Use pinned ESLint restricted-import rules for frontend code that must go through the client boundary instead of importing a native/browser transport directly. Check Rust crate dependencies from Cargo metadata so `cockpit-core` cannot depend on host, Tauri, or concrete Herdr adapters. Record temporary existing violations in the same reviewed debt policy. Add no generic architecture framework or arbitrary function-length gate. The code guide names each boundary and its owning module.

Function mapping must handle anonymous callbacks, nested functions, overloads, and Rust impl/trait methods. Use lexical ownership plus normalized declaration/site fingerprints where qualified names are insufficient; exclude nested executable ranges from their parent consistently. The TypeScript complexity probe must emit a numeric value for every eligible function, including those below the lint warning threshold. Ordinary ESLint defaults report only violations and cannot serve as a complete metric dataset. Verify a deliberately low report threshold or a small adapter using the pinned rule's existing analysis; do not infer unreported complexity as zero. Validate coverage joins against hand-computed fixtures before accepting the adapter.

Record the fixed comparison base, committed range through HEAD, full worktree source hashes, policy hash, relevant test hashes, and tool versions. Reject a stale report when those inputs changed during execution. Test-only changes must re-evaluate previously measured affected code or widen to the relevant package when that mapping is unknown; they must not let reduced test effectiveness bypass the ratchet. Do not compare scores from different coverage kinds or tool algorithms without an explicit baseline migration.

Equivalent mutants may be exempted only with a reviewed explanation and a narrow source/operator fingerprint. Record owner, reason, and revalidation condition; changing the relevant code invalidates the exemption. New compile-error mutants are reported as unviable, while a failed unmutated baseline or crashed runner blocks the run. Neither is a killed mutant. Do not automatically accept an agent's proposed baseline increase or let exclusions silently shrink the denominator.

A later Luna-high feature lane follows: read its story and owning-module examples; write meaningful behavior tests and bounded code; run fast checks; run strict changed-function metrics and the relevant bounded mutation check; fix failures; hand off the commands, report, behavioral evidence, and remaining explicit exceptions. Deterministic tooling checks the result regardless of which model wrote it. The integrator handles ambiguous design tradeoffs and runtime acceptance rather than requiring an expensive model for every routine edit.
