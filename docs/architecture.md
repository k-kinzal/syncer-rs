# Architecture and support boundary

Syncer synchronizes files on macOS, Linux and Windows. It is a policy-aware reconciler, with local enrollment controlling what downloaded policies may touch. HTTP/Git/Drive transports, document adapters and application target aliases are separately shipped native libraries. The CLI never links those official extension implementations.

## Pipeline

1. Read local enrollment, explicit source priorities, allowed roots, extension digests and environment-variable credential references.
2. Fetch every source. Fail closed on missing sources, invalid policies or digest mismatch. Offline mode is explicit; there is no silent stale-policy fallback.
3. Parse schema-versioned HCL. Reject unknown fields, duplicate blocks, dynamic expressions, unknown roles and duplicate policy names.
4. Select roles from the current or an ancestor layer. Expand selected roles at their defining layer.
5. Merge matching rule IDs. Preserve ancestor constraints and locked postconditions.
6. Resolve targets with local home/project context. Require explicit local target roots; reject symbolic links and hard links.
7. Build the full proposed file state, retaining unmanaged JSON values and matching text regions. Validate effective rules and inherited guards against the final state.
8. Dry-run shows this exact plan. Apply verifies originals have not changed, stores private backups and a transaction journal, then atomically replaces individual files.
9. After actual application, opt-in reporting encrypts allowlisted compliance fields to a locally pinned age recipient, queues ciphertext and attempts transport delivery.

All rules are evaluated before any target is changed. Conflicting final postconditions fail the plan. Rule IDs are merged globally, so teams deliberately reuse the same ID to specialize a preference. Different IDs do not bypass ancestor guards. Rules are processed in source priority order and lexicographic rule-ID order within each source; replacing an ID retains its original position. Use independent field patches; do not rely on overlapping replacement order.

## Override semantics

| Mode | Descendant behavior | Final-state validation |
|---|---|---|
| `free` | Same rule ID can be replaced | Effective rule only |
| `constrained` | Desired value may change; target, kind, operation, pointer and match pattern stay fixed | Every inherited constraint AND the effective rule |
| `locked` | Rule definition cannot change | Locked rule's postcondition, even when another rule ID modifies the file |

A descendant can tighten protection. Inherited constraints survive further descendant overrides, even when a descendant says `free`. A locked rule using `ensure` can accept several compliant values; locking the rule is distinct from demanding one exact value.

Roles are qualified as `policy-name/role-name`. Local unqualified selection refers to roles defined in that policy. Selecting an ancestor role activates its rules at the ancestor layer, including its restrictions. Roles can be defined at any layer. Selection is additive, not a mechanism for disabling already selected roles. Enrollment remains under the user's control: this CLI is not an OS security boundary against a local administrator removing the company source. Enterprise enforcement must manage enrollment/device identity externally.

## Extension boundary

See [extensions.md](extensions.md). The host's `Extension` interface is asynchronous and object-safe. Native ABI v1 calls execute on Tokio blocking workers, serially per library; third-party extensions can implement asynchronous SDK-facing adapters without exposing Rust futures across the C ABI. Native code is trusted code, not sandboxed. Remote policy cannot install a library, choose a credential, enroll telemetry or enlarge target roots. A hung native call cannot be safely force-cancelled in process; official network and Git implementations have timeouts. Out-of-process/WASM isolation is a future ABI, not a claim of v1.

## File safety and limits

Schema v1 handles files up to 16 MiB. Text/document patches require UTF-8; full replacement may use binary assets through `content_from`. `file/replace` controls a whole file; text and JSON modes patch it. Directories are not mirrored or deleted. JSON retains unmanaged values but normalizes formatting on change; it is strict JSON, not JSONC. Read/modify/write rejects invalid JSON rather than reconstructing it.

File replacement is atomic per file, not one filesystem transaction across every file. A handled write failure rolls already-written files back if they still contain Syncer's bytes. Abrupt power loss can leave a partially applied transaction; `backups/<uuid>/journal.json` and numbered backups are retained for inspection and recovery. Backup retention is intentionally manual in v0.1. New files and private state use restrictive creation permissions on Unix; existing target modes are preserved. On Windows access is governed by inherited user-profile ACLs.

An advisory state lock prevents concurrent Syncer processes using the same state directory. Content is rechecked immediately before writes. This detects normal editor races but does not provide a race-proof security boundary against a malicious process swapping directories between checks; use managed OS permissions for hostile multi-user machines. The engine refuses known symlink targets and targets inside its own state directory.

## Cloud direction

The current seams support authenticated sources, conditional rule publishing, encrypted reporting, policy digests, scoped pseudonyms, before/after compliance, timestamps and durable retry. These can feed a future Syncer Cloud. Fleet enrollment/denominators, signed policy distribution and key rotation, revocation, device attestation, hosted dashboards, remotely managed secrets, fleet-wide rollout/rollback and managed OS services are not shipped cloud services in this release.
