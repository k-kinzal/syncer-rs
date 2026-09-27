# syncer-rs

Cross-platform, extension-driven file policy synchronization. The core support boundary is files.

## Architecture

- `crates/syncer-language`: versioned HCL language, strict schema, policy/constraint types.
- `crates/syncer-core`: layered policy resolution, safe planning/apply, extension loading, encrypted reports.
- `crates/syncer-cli`: `syncer`, local enrollment, source refresh, daemon, distribution UX.
- `crates/syncer-extension-sdk`: versioned C ABI and JSON messages. Never expose Rust ABI types.
- Official source, document and target extensions are independent `cdylib` crates under `crates/`.
- `crates/syncer-document` shares value patch semantics; `syncer-extension-structured` owns format parsers and rendering, never the core.
- Core includes only local sources, full file replacement and text/regex operations. HTTP, Git, Drive, JSON and Claude support must remain extensions.

## Invariants

1. Plan every file and validate every effective rule and inherited constraint before writing any target.
2. Dry-run does not change files, enrollment, caches, credentials, or reports. Its source reads are allowed.
3. Patch preserves unrelated settings. Ambiguous, invalid or non-idempotent patches fail closed.
4. Child policies cannot remove inherited locked rules or weaken inherited constraints, even by using another rule ID.
5. Active enrolled source files cannot be policy targets (prevents cross-cycle privilege bypass); use staging and explicit push. Remote policies never install native code, choose credentials, enroll reporting, or expand locally approved target roots.
6. Installed extensions are trusted native code, pinned by digest; asynchronous host calls run outside executor threads. Explicit local `--dev-extensions` / `SYNCER_DEV_EXTENSIONS=1` may load known official libraries beside the executable without persistent pins. Installed names take precedence. Never scan the working directory or allow policies to write inside the development library directory.
7. Reports are opt-in, encrypted to a locally pinned provider key before leaving the process, and contain no paths, values, usernames or hostnames. Scoped pseudonyms are not a claim of perfect anonymity.
8. Reject symlinks and concurrent target edits. Back up original content before replacement. Describe multi-file crash limits honestly.
9. macOS/Linux/Windows are tested in CI. Shared library suffixes are `.dylib`/`.so`/`.dll` respectively.
10. Never claim a release, registry upload, platform check or cloud integration succeeded without observed evidence.
11. CLI output defaults to readable summaries. Keep machine output structured and diagnostics on stderr; scripts must select `--output json` explicitly. JMESPath queries filter presentation only, never execution or compliance. Evaluate queries before target writes, keep sensitive diffs redacted, and use JSON Lines for continuous daemon output.

## Workflow

Keep design and public examples in sync with implementation. Run `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace`. Exercise an actual dynamically loaded extension and the CLI before release. Add behavioral regression tests for policy bypasses, data loss and privacy; avoid tests that merely restate implementation. Publish crates in dependency order. Keep credentials out of files, output and commits.

When changing publishing automation, run `python3 -m unittest discover -s scripts -p 'test_*.py'`. Honor registry rate-limit retry times and skip versions already published; do not treat other upload errors as transient. Verify a registry installation in isolated state before announcing publication.

The user controls the v0.2.0 release timing. Do not create its tag, upload its crates, or publish its release artifacts until the user explicitly requests that release. Development, tests and local release-profile builds may proceed.
