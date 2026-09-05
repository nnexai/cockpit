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

The current checkout does not install coverage, complexity, or mutation
providers. A probe and strict gate therefore return `inconclusive` until exact
versions are configured in `tool-versions.json`; this is exit `2`, never a
coverage score of zero or a pass.

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
