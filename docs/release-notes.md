First public release of Syncer: layered HCL file synchronization for macOS, Linux and Windows.

- Full text/binary file synchronization through locally enrolled assets, plus text/regex patches.
- Company/team/personal policy layers, roles, free/constrained/locked overrides, array/numeric/enum constraints, rationale and metadata.
- Unified dry-run/apply planning, local target-root approvals, concurrent edit detection, atomic file replacement, backups and transaction journals.
- Independent native HTTP, Git, Google Drive, JSON and Claude extensions with versioned C ABI and asynchronous host adapter.
- Daily daemon mode, conditional policy publishing, explicit offline caches.
- Opt-in age-encrypted compliance reporting, private retry outbox and provider aggregation.

Initial-release scope: directory mirroring and hosted Syncer Cloud are not included. Native extensions are trusted code. Multi-file changes are not a global filesystem transaction. Google Drive requires your own OAuth credentials; authenticated live Drive integration is not verified without those credentials. See the architecture and operations documentation for exact limits.

crates.io publication requires registry authentication and is tracked separately from GitHub binary distribution.
