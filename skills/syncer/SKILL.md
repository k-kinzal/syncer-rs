---
name: syncer
description: >-
  Configure and operate the syncer file synchronization CLI: author layered HCL
  rules, preserve personal settings with field patches, install native extensions,
  preview/apply changes, share policies through local/HTTP/Git/Drive sources, and
  troubleshoot compliance. Use for Syncer setup, team configuration distribution,
  policy roles/constraints, scheduled synchronization, or encrypted reports.
---

# Syncer

## Locate the tool and documentation

Run `command -v syncer`, `syncer --version`, and the relevant command's `--help`.
Use the installed version's capabilities; a source checkout can be newer than its
last release. This skill is maintained in the Syncer repository under
`skills/syncer`. Resolve symlinks on this skill's path before locating repository
resources. For local development, build with `cargo build --release --workspace`
and use `target/release/syncer` and the libraries alongside it.
For v0.2.0 development builds, use `syncer --dev-extensions extension list`, or
set `SYNCER_DEV_EXTENSIONS=1`, to load adjacent official libraries without
installation. Keep that flag/environment setting across experiment commands.

Read only the documentation needed:

- [Language and constraints](../../docs/language.md): exact HCL schema, roles,
  precedence and override protection.
- [Document formats](../../docs/formats.md): selectors, supported types and
  preservation guarantees for structured files, when available in this version.
- [Extensions](../../docs/extensions.md): native library installation and remote
  authentication, including Drive.
- [Operations](../../docs/operations.md): daemon, backups and recovery.
- [Reporting](../../docs/reporting.md): encrypted reports and provider summaries.
- [Examples](../../examples): working policies; `local.hcl` needs no extensions.

## Work on a concrete project

1. Establish the project root, target files, source policies and requested scope.
   Inspect current target content before choosing operations.
2. Use explicit `--project` and `--state-dir` for experiments. Resolve temporary
   paths physically (`pwd -P`); macOS `/tmp` and `/var` are symlink aliases and are
   rejected. Keep policy source files outside the directories/files being edited.
3. Check `syncer list` and `syncer extension list` in that same state directory.
   Enroll only the target roots required by the user's requested work.
4. Prefer selected-field patches for personal configuration. Use `kind = "file"`
   and `operation = "replace"` only when whole-file ownership is intended. There
   is no recursive directory mirror or implicit bidirectional merge.
5. Validate a policy with `syncer validate POLICY.hcl`, enroll it, and run
   `syncer apply --dry-run`. Add `--diff` only when inspecting content is suitable;
   set `sensitive = true` for rules that may expose secrets.
6. Review the plan and then perform the requested `syncer apply` within existing
   authorization. Do not add a redundant confirmation step. Verify with
   `syncer apply --check`; report changed files and any unresolved violations.

Global options can precede the subcommand. Keep the same project/state options
on **every** command in a workflow; otherwise enrollment and apply may affect
different state directories.

```sh
syncer --project /physical/project --state-dir /physical/state \
  add personal /physical/policies/personal.hcl --allow-root /physical/project
syncer --project /physical/project --state-dir /physical/state apply --dry-run
syncer --project /physical/project --state-dir /physical/state apply
syncer --project /physical/project --state-dir /physical/state apply --check
```

`--check` exits 0 when clean, 2 for repairable drift, 3 for unresolved audit
violations, and 1 for errors. Ordinary dry-run exits 0 for a valid repairable plan.
`remove SOURCE` unenrolls a policy; it does not undo applied changes.

## Author layered rules

Use literal HCL with `schema_version = 1` and a unique policy `name`. Functions,
environment expansion and executable expressions are not part of the policy
language. Escape literal `${...}` as `$${...}`.

```hcl
schema_version = 1
name = "team"
contact = "platform@example.com"

rule "required-domains" {
  target = "settings.json"
  kind = "json"
  operation = "array"
  pointer = "/sandbox/network/allowedDomains"
  override = "constrained"
  rationale = "Retain personal domains while requiring team infrastructure"
  constraints {
    required_items = ["github.com"]
    forbidden_items = ["retired.example.com"]
  }
}
```

Use `set` for one exact value, `ensure` for any value satisfying constraints
(`value` is only a repair fallback), `array` for required/forbidden members, and
`remove` for a selected field. Select existing array indices; do not invent
JSONPath, XPath or wildcard syntax where a format expects JSON Pointer.

Enroll company/team/personal with increasing `--priority` (for example 0/10/20).
Use `free`, `constrained`, or `locked` deliberately. Descendants cannot bypass
ancestor protection by changing a rule ID. Define roles in any layer and select
ancestor roles as `policy-name/role-name`; inspect examples for the exact schema.
Include rationale and a real ISO date in `added` when the user supplies or requests
that metadata. Never weaken constraints just to make a failing plan succeed.

## Extensions, remote sources and daily use

Native libraries are executable code and must come from a trusted build/release.
Install with `syncer extension install LIBRARY` (optional `--sha256 DIGEST`).
macOS uses `.dylib`, Linux `.so`, Windows `.dll`; host and library architectures
must match. JSON, document formats, HTTP, Git, Drive and Claude aliases are
extension capabilities, not built-in features. Use the installed manifests to
confirm support. Resolve library paths physically before installation too:
Homebrew's `/opt/homebrew/opt/...` paths are symlinks; use the resolved Cellar
path (for example via Python `Path(library).resolve(strict=True)`).
After rebuilding a library, reinstall it if using an installed copy: these are
pinned by digest and do not track the build directory automatically. With
`--dev-extensions`, adjacent libraries instead reload on the next command. An
installed copy of the same name takes precedence; remove that copy to test the
adjacent build. `--dev-extensions=false` disables development loading even when
the environment enables it. Never place experiment targets inside the executable
directory: development mode protects that directory from policy writes.

Enroll remote policies using `syncer add NAME URI`. Pass credential **environment
variable names**, e.g. `--credential access_token=SYNCER_DRIVE_ACCESS_TOKEN`, never
tokens in HCL, command output or committed files. Use `fetch` for source refresh;
`apply` also refreshes, while `--offline` explicitly uses cached sources.

For policy sharing, use `syncer push NAME LOCAL_POLICY --dry-run` before the
requested push. Treat source policy files as data in a staging directory; active
enrolled local policies cannot also be rule targets. Remote policies cannot
install native code, expand locally allowed roots, or enroll reporting.

Use `syncer daemon --interval 86400` for daily refresh/apply under an OS service
manager with a fixed project/state path. Use `--once` for a single test cycle.
Keep reporting opt-in and encrypt to a locally pinned provider key; consult the
reporting documentation before configuring it. Scoped pseudonyms do not hide
transport account metadata.

## Diagnose failures

Preserve malformed or ambiguous input instead of replacing the whole file as a
fallback. Explain unsupported selectors or data types and repair the policy or
input within the requested scope. For extension failures, inspect architecture,
installed manifest, digest and the exact extension error. For target rejection,
check physical paths and allowed roots. For failed writes, inspect the backup and
transaction journal; individual replacements are atomic, a multi-file apply is
not a global filesystem transaction. Never print credentials or sensitive diffs
while diagnosing.
