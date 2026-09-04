# Maintainability quality-gate research

Date: 2026-09-04

Scope: deterministic, changed-code maintainability checks for future Cockpit code-writing agents. This is research and planning only. No package, Cargo tool, or workstation installation was performed.

## CRAP is a per-method risk signal

The original CRAP4J description defines the score for one Java method as:

```text
CRAP(m) = comp(m)^2 * (1 - cov(m)/100)^3 + comp(m)
```

`comp(m)` is the method's cyclomatic complexity and `cov(m)` is automated test coverage. The original description says the coverage input is basis-path coverage and describes 30 as the point at which a method is considered "crappy". The source is the original 2007 CRAP4J article by Alberto Savoia and Bob Evans, not an attribution to Uncle Bob: [The Code C.R.A.P. Metric Hits the Fan](https://www.artima.com/weblogs/viewpost.jsp?thread=215899).

Most practical adapters will use a line or region coverage fraction because that is what the available runners emit. That is a declared variant, not a claim that line coverage is the original CRAP4J input. A report must record `coverage_kind` and must leave the score absent when coverage cannot be mapped to the same method. Missing data must never be represented as 0% coverage, since that turns an unknown into an artificial high-risk score.

The local 6-8 target is deliberately stricter than the historical 30 warning point. It means that a method with complexity 6-8 and complete or strong tests is an acceptable review candidate, while a score above 8 deserves attention and a score above 30 is a hard risk finding. These are Cockpit policy thresholds. They are not universal CRAP standards.

The nonlinear formula makes the policy legible:

| Cyclomatic complexity | 100% coverage | 80% coverage | 50% coverage | 0% coverage |
|---:|---:|---:|---:|---:|
| 6 | 6.000 | 6.288 | 10.500 | 42 |
| 7 | 7.000 | 7.358 | 15.750 | 56 |
| 8 | 8.000 | 8.512 | 24.000 | 72 |

The proposed gate requires new methods to score at most 8; 6–8 is informational, with 6 the preferred target. Touched legacy methods must not regress against a reviewed baseline. Historical scores above 30 identify priority debt, not a looser allowance for new code. The score is a prompt to inspect behavior and tests, never a reason to add shallow assertions or split code solely to improve a number.

## Rust coverage, complexity, and mutation tools

Rust has a compiler-owned source coverage path. The Rust compiler documents `-C instrument-coverage`, which records function and branch counters in LLVM coverage maps and emits raw profiles for `llvm-profdata` and `llvm-cov`: [rustc instrumentation-based code coverage](https://doc.rust-lang.org/rustc/instrument-coverage.html). The proposed wrapper is [cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov), whose first-party README documents line, region, and optional branch reports plus JSON and LCOV output. It wraps the compiler facility and can run the workspace test suite.

Rust does not provide a stable built-in CRAP report. The candidate adapter is [cargo-crap](https://github.com/minikin/cargo-crap), which reports the same per-function formula from Rust complexity and coverage. The adoption probe must confirm that its function identity and coverage ranges can be joined for this workspace. If that join is unavailable, the report must expose Rust complexity and Rust coverage as separate fields and mark CRAP `inconclusive`; it must not manufacture a method score. A later fallback may use a pinned AST metric tool, but writing a second complexity parser is outside CLEAN-05.

[cargo-mutants](https://mutants.rs/) is a suitable Rust mutation runner. Its documentation describes a baseline test run followed by source-tree copies for each mutant, and its workspace guide documents restricting a run with `--file`. The tool's reproducibility page documents `--no-shuffle`, which runs mutants in source order, and says reproducibility still depends on deterministic builds and tests: [stability and reproducibility](https://mutants.rs/stability.html). Its own model distinguishes mutants that do not compile or are otherwise unviable from mutants that survive the tests: [how cargo-mutants works](https://mutants.rs/how-it-works.html).

The proposed Rust command is therefore a wrapper around a pinned `cargo mutants --no-shuffle` invocation with a changed-file filter, a fixed worker count, and JSON output. Exact flags and the JSON shape must be probed against the selected tool version before scripts are written. The report keeps `killed`, `survived`, `no_coverage`, `timeout`, `compile_error`, `skipped`, and `inconclusive` as separate outcomes. The custom executed score uses killed and survived; a separate adequacy score includes uncovered mutants. Preserve raw tool scores and outcomes rather than labeling the custom score as the provider standard.

## TypeScript coverage and Stryker compatibility

The repository has Vitest 4.1.10, TypeScript 7.0.2, Vite 8.2.2, and exact Bun package versions in `package.json` and `bun.lock`. Vitest 4 documents V8 and Istanbul providers. V8 is the recommended provider and produces remapped source reports, but its profiler path requires a V8 runtime such as Node, Deno, or Chromium and does not work on Bun: [Vitest coverage guide](https://v4.vitest.dev/guide/coverage). The current `bun run test` command delegates the Vitest executable to Node 26.8.1 on this workstation; the existing three test files pass, 55 tests in total. The coverage provider package is not installed, so coverage remains a future adoption step. Istanbul is the fallback when a supported V8 execution path cannot be established, with the same install-and-pin requirement.

Stryker's official Vitest runner page says the runner was introduced in Stryker 7, requires Vitest to be installed by the project, and is configured with `testRunner: "vitest"`: [Stryker Vitest runner](https://stryker-mutator.io/docs/stryker-js/vitest-runner/). The runner package metadata declares a Vitest peer of `>=2.0.0`, so Vitest 4.1.10 is within its declared range. The current package source also declares Node `>=22.0.0`, satisfied by Node 26.8.1: [official runner package metadata](https://github.com/stryker-mutator/stryker-js/blob/master/packages/vitest-runner/package.json). The Stryker configuration docs support an explicit `mutate` file list/glob, `coverageAnalysis: "perTest"`, JSON output, and incremental reports: [configuration](https://stryker-mutator.io/docs/stryker-js/configuration/), [incremental mode](https://stryker-mutator.io/docs/stryker-js/incremental/).

The plugin and Stryker core are absent from `node_modules`. CLEAN-05 may add exact package versions only after the compatibility probe; it must not use an unpinned `npx` download. Stryker's docs do not establish a user-facing random seed option. The wrapper should set a fixed `QUALITY_SEED` for any project fixtures, run with deterministic worker/concurrency settings, sort and record mutant IDs, and state `seed: "tool-does-not-expose-one"` in the report rather than inventing a flag.

For TypeScript complexity, the first candidate is ESLint's built-in `complexity` rule with the `classic` variant. ESLint documents that it reports cyclomatic complexity per function and that the default limit is only a lint threshold, so CLEAN-05 must consume machine-readable JSON rather than treat the lint exit code as the CRAP value: [ESLint complexity rule](https://eslint.org/docs/latest/rules/complexity), [ESLint JSON formatter](https://eslint.org/docs/latest/use/formatters/). ESLint is not installed in this checkout. The adoption probe must pin ESLint and a TypeScript parser, confirm that each message maps to the function's start/end range, and record the exact parser and `variant` in the report. If a stable function-range join cannot be produced, TypeScript CRAP is inconclusive and separate complexity/coverage fields are the fallback.

## Readiness probes and blockers

Read-only probes on 2026-09-04 found:

| Capability | Observed state | Planning consequence |
|---|---|---|
| Bun | 1.3.14 | Matches the lower bound in the package engine. Keep `bun run` as the package entry point. |
| Node | v26.8.1 | Runs Vitest 4.1.10 and satisfies the repository and Stryker runner engine requirements. |
| Rust | rustc/cargo 1.98.0, `rust-toolchain.toml` channel 1.98.0 | Pin Rust quality commands to this repository toolchain. |
| Vitest | 4.1.10; 3 files and 55 tests pass | Existing test command is usable; no coverage provider is installed. |
| `@vitest/coverage-v8` / Istanbul provider | Absent | Coverage commands are proposed, not runnable until a matching exact package is added. |
| cargo-llvm-cov | Absent | Rust coverage is blocked pending a pinned tool installation. |
| cargo-crap or another Rust CRAP adapter | Absent | Rust CRAP is blocked pending an adapter probe; separate metrics remain the safe fallback. |
| cargo-mutants | Absent | Rust mutation is opt-in and blocked pending a pinned tool installation. |
| Stryker core/Vitest runner | Absent | TypeScript mutation is opt-in and blocked pending exact package installation. |

No Tauri native runtime probe belongs in this quality infrastructure story. Existing project research records Linux WebKitGTK/GTK development packages as a prerequisite for native Tauri verification. The quality report must keep native compile and native runtime evidence separate from browser and workspace checks.
