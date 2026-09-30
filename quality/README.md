# Changed-scope quality gate

The gate is local and read-only during normal use. It never installs a package,
changes source files, starts Herdr, or guesses a comparison branch.

```sh
bun run quality:probe
bun run quality:report -- --base <commit>
bun run quality:gate -- --base <commit> --strict
```

`--base` is required. Reports are written below `quality/reports/`, which is
ignored by Git. The scope records committed-range, staged, unstaged, deleted,
renamed, and untracked paths with current and base content hashes.

Coverage and complexity providers remain unconfigured. A probe and strict gate
therefore return `inconclusive` until all required exact versions and provider
inputs are configured; this is exit `2`, never a coverage score of zero or a
pass. The opt-in mutation tools below do not supply normalized gate inputs.

Optional normalized provider inputs use JSON objects with a `methods` array.
Complexity rows require `key`, `language`, `path`, `symbol`, `signature_hash`,
`declaration`, and integer `complexity`. Coverage rows use the same `key` and
`coverage: { kind, covered, total }`. A join must be exact. The score is
`c^2 * (1 - coverage)^3 + c` and a newly measured method needs at least 90%
coverage and CRAP at most 8. Mutation input has either `status:
"not_applicable"` plus a reason, or complete mutant rows with stable IDs and
one of `killed`, `survived`, `no_coverage`, `timeout`, `compile_error`,
`skipped`, or `inconclusive`.

Exit codes are stable: `0` pass or report-only completion, `1` threshold or
ratchet failure, `2` strict inconclusive evidence, and `3` invalid invocation
or infrastructure failure.

Baselines are never overwritten by a normal run. Review a candidate first:

```sh
python3 scripts/quality/gate.py baseline --from quality/reports/<report>.json
python3 scripts/quality/gate.py baseline --from quality/reports/<report>.json --write
```

Reviewed exceptions live in `quality/baseline.json`. Each exception must name a
method or mutant ID, exact signature or mutation fingerprint, owner, reason,
and revalidation condition. A changed fingerprint invalidates the exception.

## Targeted mutation testing (opt-in)

Mutation testing is a manual precision tool, not part of `bun run test`, Cargo
tests, CI, the quality checks, or the quality gate. There is no mutation-score
gate: surviving mutants are evidence to review, not a command failure. Use the
target-required launcher, not bare `cargo mutants` or `stryker run` (whose
defaults can select broad source sets).

Install the pinned tools explicitly:

```sh
bun install --frozen-lockfile
cargo install --locked cargo-mutants@27.1.0
```

The released pins are cargo-mutants **27.1.0** and matching Stryker core/Vitest
runner **10.0.0**. Release metadata:
[cargo-mutants](https://github.com/sourcefrog/cargo-mutants/releases/tag/v27.1.0),
[Stryker core](https://registry.npmjs.org/@stryker-mutator%2fcore/10.0.0),
[Vitest runner](https://registry.npmjs.org/@stryker-mutator%2fvitest-runner/10.0.0).
The launcher checks installed versions against `quality/tool-versions.json`
and refuses missing or mismatched tools; it never installs anything.

Start with these pure-module pilots:

```sh
bun run quality:mutation -- rust --file crates/cockpit-core/src/jira_query.rs --test-filter jira_query:: --re bare_project_key
bun run quality:mutation -- ts --file src/app/layout/solveLayout.ts:14-18
```

Verified setup pilots: the Rust command above produced 7 mutants (4 caught,
3 survived); the TS range produced 26 (14 killed, 12 survived), including a
two-worker run. These are scoped observations, not project-wide scores or
reviewed test gaps. A whole-file TS run also exercised timeout reporting:
228 killed, 124 survived, 18 uncovered and 5 timed out; its launcher result
was inconclusive, not a pass.

At least one literal, repository-relative source `--file` is required. Repeat
`--file` for multiple distinct files; wildcards, tests, generated files,
declaration-only `.d.ts` files, nonexistent files, symlinks, and the wrong
language are rejected. Rust derives the package from its Cargo manifest and
tests the mutated package; `--test-filter NAME` selects matching lib unit tests,
and `--re REGEX` narrows the mutant names. TS also accepts a line range such as
`--file src/app/layout/solveLayout.ts:14-43`. Every requested file must produce
mutants. A types-only range is inconclusive, never a pass.

`--jobs N` bounds concurrent mutation workers (default: quality-config workers,
currently 1). `--budget-minutes M` defaults to 30 for the run; Rust listing has
an additional 300-second cap. When exceeded, the child process group receives
SIGTERM and then SIGKILL as needed, with up to 30 seconds of shutdown grace.
Tool-native per-mutant timeouts remain enabled. Logs are retained in the run
directory; the launcher prints its location before running.
SIGTERM or Ctrl-C of the launcher also terminates its child process group and
reports interruption as inconclusive. SIGKILL cannot run cleanup; check for
remaining tool processes and sandboxes after forcibly killing the launcher.

Artifacts stay under ignored `quality/reports/mutation/<UTC-stamp>-rust|ts/`:
`summary.json` contains native status counts, survivors with file/line, and
diagnostics; Rust retains `preflight.json`, logs and `mutants.out/outcomes.json`;
TS retains the effective config, `tool.log`, `mutation.json`, and
`mutation.html` (open in a browser). Reports are not normalized gate inputs.

Launcher exit codes:

- `0`: completed with nonzero mutants for every target; survivors do not fail.
- `2`: inconclusive—zero mutants, failed/empty baseline, incomplete evidence,
  mutant timeout, budget expiry, or a leftover Stryker sandbox.
- `3`: invalid invocation or infrastructure—missing target/tool, version drift,
  unsafe path, wrong language, stale sandbox, malformed report, or tool error.
- No score-based exit `1`.

Both tools mutate isolated source copies, not the checkout. Cargo explicitly
does not copy `target/` and respects Git ignores; Stryker excludes build/runtime
artifacts from its copies, retains `docs/` used by tests, and symlinks the
installed `node_modules`. Ordinary Vitest excludes root `.stryker-tmp/**`, so
leftover sandboxes are not collected by normal tests while tests inside the
active sandbox remain discoverable. Stryker normally cleans on success or
failure; interruption can still leave `.stryker-tmp`. The launcher refuses a
stale sandbox rather than deleting it. Inspect it and confirm no mutation
processes remain before removing it yourself. Do not run concurrent TS
launchers in the same checkout.

Stryker 10's sandbox tsconfig preprocessor requires the TypeScript 5 JavaScript
compiler API, which this project's TypeScript 7 package does not expose. The
supported `ignorePatterns` setting omits `tsconfig.json` during Stryker
preprocessing. A mutation-only Vite config imports the ordinary Vite config and
restores the launcher's exact original tsconfig text during `configResolved`,
before source transforms, only after verifying the destination is a real
Stryker sandbox. Native Vite/Oxc then consumes the unchanged compiler settings.
The normal TypeScript version and config are unchanged; no API error is
suppressed, no in-place mutation is used, and no transform options are
hand-maintained. This compatibility hook assumes the current self-contained
tsconfig; external `extends` or project references need a fresh compatibility
review because Stryker no longer rewrites those paths.

**Source isolation is not a security sandbox.** Tests inherit your environment,
HOME/XDG directories, credentials and network access; mutation can redirect
deletion or other side-effecting code, and dependencies are shared. Only target
pure modules initially. Before mutating teardown, persistence, credential or
provider code, use a disposable environment with disposable data and no access
to personal sessions or secrets. A successful baseline does not make mutated
side effects safe. Type-invalid TS mutants can be noise: Vitest transpiles
without typechecking, and no TypeScript checker plugin is configured. Review
survivors and rerun surprising results before changing tests.

To upgrade, update the exact package pins and `bun.lock`, the tool-version
manifest, and the cargo install version together. Recheck released metadata,
run both pilots and invalid/zero-mutant/budget cases, and confirm ordinary tests
still discover the same files. No automatic invocation or score gate is added.
