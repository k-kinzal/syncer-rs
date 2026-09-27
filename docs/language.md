# Syncer HCL schema v1

HCL provides comments, named `rule` and `role` blocks, strings/heredocs, objects, arrays, numbers and booleans. Syncer adds policy, role and constraint semantics. Expressions, interpolation, functions, environment lookup and executable hooks are rejected. Escape a literal `${...}` as `$${...}`. Unknown fields and duplicate blocks are errors. `schema_version = 1` and a unique `name` are required.

```hcl
schema_version = 1
name = "company"
description = "Company agent settings"
contact = "developer-platform@example.com"
revision = "2026-09-27"
select_roles = ["developer"]

role "developer" {
  description = "Default development environment"
  rule "sandbox-domains" {
    target = "settings.json"
    kind = "json"
    operation = "array"
    pointer = "/sandbox/network/allowedDomains"
    override = "constrained"
    rationale = "Keep required infrastructure available; block the retired host"
    added = "2026-09-27"
    constraints {
      required_items = ["github.com", "registry.npmjs.org"]
      forbidden_items = ["retired.example.com"]
    }
  }
}
```

A team policy can set `select_roles = ["company/developer"]`; a personal policy can define and select its own roles. CLI `apply --role company/developer` adds a local selection. Duplicate active IDs within a layer are errors. Across layers an ID deliberately overrides the previous definition, subject to protection.

Rule fields: `target`, `kind`, `operation`, optional `value`, `content_from` (locally enrolled asset name), `pattern`, `pointer`, `constraints`, `override` (default `free`), `rationale`, `added` (real ISO date), and `sensitive` (default false). Sensitive rules suppress file diffs, including when other rules touch that file. Diffs are otherwise opt-in with `--diff`; JSON output contains paths and digests but no file content by default.

| Kind | Operation | Meaning |
|---|---|---|
| `file` | `replace` | Replace full content with string `value` or an enrolled `content_from` asset (including binary files) |
| `text` | `match` | Audit literal `pattern`; no automatic repair |
| `regex` | `match` | Audit Rust-regex `pattern`; no automatic repair |
| `text` | `replace` | Replace literal matches with string `value` |
| `regex` | `replace` | Replace all regex matches; `$1`/`${name}` capture syntax uses HCL escaping where needed |
| `json` extension | `set` | Set RFC 6901 `pointer` to `value`, preserving other fields |
| `json` extension | `ensure` | Retain current value if constraints pass; otherwise use `value` as repair fallback |
| `json` extension | `array` | Keep personal entries, remove forbidden/disallowed entries, add required entries; `empty = true` clears |
| `json` extension | `remove` | Remove the addressed field |

Text replacement must be idempotent. A nonmatching replacement is a no-op; combine it with a `match` audit when the presence of a setting is mandatory. JSON refuses to replace a scalar parent implicitly. Array indices address existing elements; appending is handled through `array` constraints. A missing target starts as an empty string (or `{}` for JSON). `ensure` and `array` require at least one constraint. Failed audits and unresolved repair constraints prevent all target writes.

The v0.2.0 `structured` extension applies the same selected-value operations and constraints to additional formats. See [format selectors and limits](formats.md).

Constraints are conjunctive:

- `enum = ["A", "B"]`: whole value must equal one member.
- `min = 1`, `max = 10`: numeric bounds, inclusive.
- `required_items = ["a"]`: an array must contain all listed values.
- `forbidden_items = ["b"]`: none may occur.
- `allowed_items = ["a", "c"]`: optional exhaustive allowlist; `[]` permits no entries.
- `empty = true` or `false`: empty/nonempty strings, arrays or objects.
- `pattern = "..."`: regex constraint on strings.

For built-in text/file rules constraints refer to the entire file string. For JSON they refer to the selected field. Array members can be any JSON value. `required_items` expresses both inclusion and mandatory retention; combining it with ancestor protection makes that requirement non-overridable. Wrong value types fail validation. Conflicting requirements are rejected statically where possible and otherwise against the final proposed content.

Relative targets resolve from `--project` (default current working directory), `~/` from the current user's home. Application aliases are extension-owned. Every target must fall strictly inside a root approved using `add --allow-root`. Sources are ordered by unique integer priority, low to high; default new enrollment appends after existing sources. Therefore enroll company, team, personal in that order or supply explicit priorities.
