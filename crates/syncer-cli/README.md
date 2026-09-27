# syncer-cli

Part of [syncer-rs](https://github.com/k-kinzal/syncer-rs), the layered file policy synchronization workspace.

Installs the `syncer` CLI. See the project documentation for source enrollment, HCL policies, dry-run/apply, daemon mode and reporting.

Output defaults to readable summaries and tables. Use `--output json` for
automation, `--query` for JMESPath selection, and `--output jsonl` for a continuous
daemon stream. YAML, table and headerless text output are also available.

MIT licensed. Rust 1.92 or later.
