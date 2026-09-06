# Repository discovery limits

Cockpit reads `$XDG_CONFIG_HOME/cockpit/config.toml`, defaulting to `~/.config/cockpit/config.toml`. `COCKPIT_CONFIG` selects another file for both native and browser hosts. Restart the host after changing this file.

```toml
version = 1
repository_roots = ["/path/to/projects"]

[limits]
catalog_entries = 10000
catalog_depth = 3
```

`catalog_entries` defaults to 1,024 and accepts 1 through 100,000. It bounds filesystem work during repository discovery across all configured roots together. The scan counts each directory visited and each directory entry examined, including files. It is not a repository count or a file-size limit. Dependencies and build output beneath a configured root can consume the budget. Narrow repository roots reduce the work; raising the budget allows a larger scan.

`catalog_depth` defaults to 3 and accepts 1 through 32. The configured root is depth zero. `.git` directories and symlink directories are not traversed. Discovery also has a separate `operation_timeout_ms` budget, defaulting to 30,000 milliseconds.

The Files directory listing uses `context_directory_entries`, default 1,000, and `context_tree_depth`, default 32. Document reads use `context_preview_bytes` and `context_preview_lines`. These limits are separate from repository discovery, so increasing a document limit does not remove a repository discovery warning.

Use `cargo run -p cockpit-host --bin cockpit -- configuration` to inspect the effective configuration and the source of its values. Browser hosts also accept `--config` and `--repository-root`; see `cockpit serve --help` for invocation overrides.
