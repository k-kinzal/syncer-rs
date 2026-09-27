# Distribution

Workspace packages use independent crates under `crates/` and one coordinated version. The command package is `syncer-cli`; the installed executable is `syncer`. Official extensions are independent `cdylib` packages, not core feature flags.

## Release checks

```sh
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo build --locked --workspace
python3 scripts/smoke.py target/debug
```

CI exercises Linux, macOS and Windows, including native library installation and actual dynamic calls. The release workflow additionally builds these targets:

- `aarch64-apple-darwin`
- `x86_64-apple-darwin`
- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`
- `x86_64-pc-windows-msvc`

Linux binaries are built on Ubuntu 24.04 (glibc); older glibc systems should build from source. Windows binaries use MSVC. Native extensions and host must match target architecture. Release archives contain the executable, five extension libraries, README and license. Each archive is listed in `SHA256SUMS`. macOS artifacts are not Developer ID notarized in this initial release.

Push a version tag after validation to build and publish GitHub release assets. `scripts/package.py` packages each target, and the release job requires every target build/smoke check before publication. Update package version, internal dependency versions, examples/docs and package script default together when preparing a new version. Release notes live in `docs/release-notes.md`.

## crates.io

Package names were available at initial preparation. Registry publishing requires an authorized crates.io account/token; a GitHub login is not sufficient. This environment has no Cargo registry credentials or configured `CARGO_REGISTRY_TOKEN` secret, so **crates.io publication is pending authentication**. Do not advertise `cargo install syncer-cli` as an already available registry install until publication is confirmed.

After `cargo login` in a trusted local terminal, run:

```sh
python3 scripts/publish.py
```

Or set the repository secret `CARGO_REGISTRY_TOKEN` and run the **Publish crates** workflow. Never commit the token or put it in policy files. The script publishes language/SDK first, core/CLI next, then official extensions. Reruns skip versions already available. Workspace dependents cannot complete registry packaging verification before their dependencies exist in the registry; local builds, tests and smoke checks validate the workspace before the first publish.

## Homebrew

The personal tap is `k-kinzal/homebrew-tap`, with `Formula/syncer.rb`. `scripts/homebrew.py VERSION DIST_DIRECTORY` generates the formula from downloaded release archives and their SHA-256 values, using platform-specific binary URLs. Install with `brew install k-kinzal/tap/syncer`.

Homebrew installs native libraries into `$(brew --prefix syncer)/lib/syncer`. Installation does not enable native extensions in user enrollment; use `syncer extension install` explicitly. The formula's test verifies local source enrollment, dry-run and apply in an isolated test directory.
