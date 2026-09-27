# v0.2.0 development (not released)

- Official native extensions are HTTP, Git, JSON and structured documents. Claude
  aliases and Google Drive are deferred from official support pending local
  validation; they are excluded from workspace builds, release archives,
  registry publishing and automatic development loading.
- HTTPS sources use the HTTP transport unless enrollment explicitly selects
  another installed extension. Existing explicit selections are preserved.
- Public layered examples use ordinary file paths and require only JSON support.
- Structured document patches cover YAML, TOML, HCL, XML, JSONC/JSON5, INI, dotenv,
  Java properties, CSV/TSV and XML plist.
- CLI output defaults to human-readable summaries; explicit output formats and
  JMESPath queries support automation.

This version is not tagged or published. Previously released artifacts are
unchanged. The user controls release timing.

# v0.1.0 (historical release)

First public release of Syncer: layered HCL file synchronization for macOS, Linux and Windows.

- Full text/binary file synchronization through locally enrolled assets, plus text/regex patches.
- Company/team/personal policy layers, roles, free/constrained/locked overrides, array/numeric/enum constraints, rationale and metadata.
- Unified dry-run/apply planning, local target-root approvals, concurrent edit detection, atomic file replacement, backups and transaction journals.
- Independent native HTTP, Git, Google Drive, JSON and Claude extensions with versioned C ABI and asynchronous host adapter.
- Daily daemon mode, conditional policy publishing, explicit offline caches.
- Opt-in age-encrypted compliance reporting, private retry outbox and provider aggregation.

Initial-release scope: directory mirroring and hosted Syncer Cloud are not included. Native extensions are trusted code. Multi-file changes are not a global filesystem transaction. Google Drive requires your own OAuth credentials; authenticated live Drive integration is not verified without those credentials. See the architecture and operations documentation for exact limits.

Install the CLI from [crates.io](https://crates.io/crates/syncer-cli): `cargo install syncer-cli --version 0.1.0 --locked`. Optional native extension libraries are included in the GitHub archives and Homebrew package.

Windows archives require the [Microsoft Visual C++ x64 runtime](https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist). Homebrew: `brew install k-kinzal/tap/syncer`.
