# syncer-rs

`syncer` applies layered file policies without throwing away personal settings. Company, team and personal HCL policies can combine full file replacement, text/regex patches and extension-provided JSON field patches. Ancestor constraints and locked rules are checked against the final file contents.

macOS, Linux and Windows. Rust 1.92+. MIT licensed. Initial release: **0.1.0**.

## Install

Download a platform archive from [GitHub Releases](https://github.com/k-kinzal/syncer-rs/releases). Archives include `syncer` and independent official extension libraries. Verify against `SHA256SUMS` before installation.

```sh
brew install k-kinzal/tap/syncer
```

Install the CLI from [crates.io](https://crates.io/crates/syncer-cli):

```sh
cargo install syncer-cli --version 0.1.0 --locked
```

Cargo installs the CLI. Get the optional native extension libraries from a release archive or build them from source.

Build from source:

```sh
cargo install --path crates/syncer-cli --locked
cargo build --release --workspace
```

The crates.io package name is `syncer-cli`, and its binary is `syncer`. Release procedures are documented in [distribution.md](docs/distribution.md). Core has only local file, text and regex support. Enabling extensions is an explicit choice:

```sh
# macOS, from a downloaded archive or target/release
syncer extension install ./libsyncer_extension_json.dylib
syncer extension install ./libsyncer_extension_claude.dylib
# Homebrew libraries: $(brew --prefix syncer)/lib/syncer/
# Linux: .so; Windows: syncer_extension_json.dll
```

## Try it safely

Create a disposable project and use the included local policy:

```sh
demo_root="$(mktemp -d)"
demo_root="$(cd "$demo_root" && pwd -P)"
mkdir "$demo_root/project"
syncer --project "$demo_root/project" --state-dir "$demo_root/state" \
  add personal /absolute/path/to/syncer-rs/examples/local.hcl
syncer --project "$demo_root/project" --state-dir "$demo_root/state" apply --dry-run --diff
syncer --project "$demo_root/project" --state-dir "$demo_root/state" apply
```

The example resolves the temporary directory to its physical path because Syncer rejects symlinks, including macOS's `/tmp` and `/var` aliases.

`apply --dry-run` reads sources and shows the proposed changes without changing targets, caches or reports. `--diff` opts into content diffs; sensitive rules suppress them. `apply --check` exits 2 on drift and 3 on unresolved audit violations. Errors exit 1. Regular dry-run exits 0 for a valid, repairable plan.

## Company → team → personal

```sh
syncer add company ./examples/company.hcl --priority 0 --allow-root .
syncer add team ./examples/team.hcl --priority 10 --allow-root .
syncer add personal ./examples/personal.hcl --priority 20 --allow-root .
syncer apply --dry-run --diff
syncer apply
```

These examples use the JSON and Claude extensions. JSON patches retain unmanaged fields and personal array entries. `ensure` accepts an existing value that satisfies the policy (for example any number ≥ 1); `value` is a fallback for repair. `free`, `constrained` and `locked` control descendant customization. Roles can be defined anywhere and selected from any ancestor. Rationale, dates and policy contacts stay alongside the rules.

Source names are local enrollment names; HCL policy names identify roles and reports. Relative targets resolve under the current project, so use a fixed `--project` for background execution. `--allow-root` is a local trust boundary: a remote policy cannot grant itself permission to edit another directory. To manage user Claude settings, explicitly enroll `--allow-root ~/.claude` after creating that directory.

## Remote policies and rule sharing

```sh
syncer extension install ./libsyncer_extension_google_drive.dylib
syncer add global-config 'https://drive.google.com/file/d/FILE_ID/view' \
  --credential access_token=SYNCER_DRIVE_ACCESS_TOKEN --allow-root .
syncer apply --dry-run
syncer apply

# Publish your own HCL policy to the enrolled Drive file, then let a team enroll it.
syncer push global-config ./my-rules.hcl --dry-run
syncer push global-config ./my-rules.hcl

# Daily refresh and apply. Run under your OS service manager for reboot persistence.
syncer daemon --interval 86400
```

For unattended Drive access, use renewable OAuth credentials as described in [extensions.md](docs/extensions.md). HTTP and Git are separate official extensions; `syncer fetch` refreshes cached policies, and `apply --offline` explicitly uses that cache. Source retrieval failures stop application; old policy is never silently substituted. Rules are files themselves: publish HCL through `push`, Git, or any other file distribution workflow. There is no implicit bidirectional merge.

## Provider reporting

Reporting is off by default. Enroll a provider's age public key and a sink explicitly:

```sh
syncer report enable company drive://REPORT_FOLDER_ID \
  --recipient age1... --extension google-drive \
  --credential access_token=SYNCER_DRIVE_ACCESS_TOKEN
syncer report flush
```

Before/after compliance, rule IDs, policy digests and observation times are encrypted **before** transport. No file paths, settings, usernames or hostnames are included. A random installation secret produces separate provider-scoped pseudonyms, allowing remediation trends without a machine name. Pseudonymity is not perfect anonymity: providers can correlate observations within their scope, and transport accounts/metadata may still identify uploaders. Folder peers can see ciphertext but cannot decrypt it with their own credentials. See [reporting.md](docs/reporting.md).

## Documentation and development

- [HCL language and constraints](docs/language.md)
- [Architecture, guarantees and limits](docs/architecture.md)
- [Extension ABI and authentication](docs/extensions.md)
- [Reporting and privacy](docs/reporting.md)
- [Services and recovery](docs/operations.md)
- [Releases and package distribution](docs/distribution.md)
- [Development instructions](AGENTS.md)

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace
python3 scripts/smoke.py target/debug
```

The supported core boundary is file synchronization. Arbitrary third-party cloud administration remains an extension author's responsibility. This release handles files up to 16 MiB (UTF-8 for patches, arbitrary bytes for asset replacement), not recursive directory mirroring. It does not provide hosted Syncer Cloud, managed fleet enrollment or an OS security boundary against local administrators.
