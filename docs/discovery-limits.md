# Repository discovery limits

See [shared configuration](configuration.md) for the configuration path, environment/invocation precedence, repository-root examples and limit defaults/ranges. Restart the host after changing its configuration.

`catalog_entries` bounds filesystem work across all configured roots together: each directory visited and each entry examined, including files. It is not a repository count or file-size limit. Discovery is breadth-first and recognizes a checkout when its parent is listed, so nearer repositories are found first when the budget runs out. Dependencies and build output can exhaust the budget before deeper repositories are reached. Narrow roots reduce the work; raising the budget allows a larger scan.

The configured root is depth zero. `.git` directories and symlink directories are not traversed. `catalog_depth` and the separate `operation_timeout_ms` budget bound traversal.

Files listings use `context_directory_entries` and `context_tree_depth`; document reads use `context_preview_bytes` and `context_preview_lines`. These limits are separate from repository discovery, so increasing a document limit does not remove a discovery warning.

Use `cargo run -p cockpit-host --bin cockpit -- configuration` to inspect the effective configuration and the source of its values. Browser hosts also accept `--config` and `--repository-root`; see `cockpit serve --help` for invocation overrides.
