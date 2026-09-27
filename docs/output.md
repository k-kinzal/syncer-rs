# CLI output and queries

From v0.2.0 development builds, normal output is intended for people: lists show
tables, mutations show confirmations, and apply shows changed paths and rule
states. Scripts that parsed the old default JSON must add `--output json`.
Terminals and pipes use the same default, without a pager or color escapes.

Global options work before or after a subcommand:

| Option | Behavior |
| --- | --- |
| `--output human` | Default command-specific summaries and tables |
| `--output json` | One complete, pretty-printed JSON result |
| `--output jsonl` | One complete result per line, including an entire array on one line |
| `--output yaml` | One YAML document representing the same result |
| `--output table` | Generic table of the result or selected fields |
| `--output text` / `--output tsv` | Headerless text; rows use tab-separated cells |
| `-o FORMAT` | Short form of `--output` |
| `--query EXPRESSION` | Apply a JMESPath query before formatting |
| `--quiet` / `-q` | Suppress normal output and backup notices; retain errors and warnings |

`--quiet` and `--query` are mutually exclusive. Help and version retain standard
CLI text. Errors and warnings go to stderr regardless of the output format.
JSON stdout contains no banners or backup notices. Most errors produce no result;
an unresolved apply can return its plan on stdout with a nonzero exit code.
Always check the exit code as well as the data.

## Selecting fields

Queries use [JMESPath](https://jmespath.org/tutorial.html), including projections,
filters, functions and multi-select objects/lists. They operate on the complete
structured result, not rendered text. Quote the expression for your shell.

```sh
# Readable default and complete machine-readable manifests
syncer extension list
syncer extension list --output json

# A name per line
syncer extension list --query '[].name' --output text

# Select columns and sort rows
syncer extension list --query 'sort_by(@, &name)[].{name:name,version:version}' --output table

# Extensions providing document formats
syncer extension list --query '[?length(kinds) > `0`].{name:name,kinds:kinds}' --output json

# Source priorities with explicit text-column order
syncer list --query 'sort_by(@, &priority)[].[name, priority]' --output text

# Preview only changed paths; or select the drift count (still check exit code)
syncer apply --dry-run --query 'files[].path' --output text
syncer apply --check --query changed_files --output json

# Capture a public reporting recipient, never the private key
syncer report keygen ./provider.agekey --query recipient --output text
```

Object columns are sorted by key. Use a multi-select list such as
`[].[name, priority]` when column order matters. Nested values remain compact JSON
inside cells. Text strings have no surrounding quotes; tabs, newlines and terminal
control characters inside strings are escaped to keep each cell on one line.
Prefer JSON or YAML for a lossless round trip. Arrays of scalars print one value
per line. No matches produce `[]` in JSON, an empty text stream, or `No results.`
in a table. A missing field is `null`, not an error.

Queries affect presentation only: a query returning no files still applies the
whole validated plan unless `--dry-run`/`--check` is present. Syntax is checked
before state access. Query evaluation and rendering happen before target writes
or publishing, so query type/function errors cannot accidentally apply a change.
Mutating commands may acquire their local lock before evaluation. Sensitive diffs
stay redacted, including when selected with a query.

## Result shapes

| Command | Structured result |
| --- | --- |
| `list` | Source array: `name`, `priority`, `endpoint`, `roots`, `asset`, `sha256` |
| `extension list` | Manifest array: `name`, `version`, `methods`, `schemes`, `kinds`, `targets`, `abi_version` |
| `report list` | Reporter array: `policy`, `endpoint`, `recipient` |
| `apply`, `daemon` | Plan object: `changed_files`, `compliant`, `files`, `rules` |
| `fetch` | Array: `name`, `sha256`, `revision`, `fetched_at`; no source contents |
| `validate` | Object: `valid`, `name`, `rules`, `roles`, `message` |
| `add` | Object: `action`, `source`, `message` |
| `remove`, `extension remove` | Object: `action`, `name`, `message` |
| `extension install` | Object: `action`, `path`, `sha256`, `message` |
| `push` | Object: `action`, `name`, `bytes`, `dry_run`, `message` |
| `report keygen` | Object: `recipient`, `identity` path, `message`; no private key |
| `report enable` | Object: `action`, `policy`, `sink`, `recipient`, `message` |
| `report disable` | Object: `action`, `policy`, `message` |
| `report flush` | Object: `action`, `message` |
| `report summarize` | Object: `scope`, `rules`; see [reporting](reporting.md) |

Plan data describes proposed changes and expected compliance. Rule fields `before`
and `after` mean compliance before/after the proposed patch; they do not imply a
dry-run changed files. `files` contains changed paths, hashes, flags, and `diff`
(null unless `--diff` is requested and the file is neither sensitive nor binary).
On a successful normal apply the complete plan has been applied. Human output
distinguishes dry-run, applied, clean and blocked plans.

Queries and formats do not alter exit codes: `apply --check` returns 0 when clean,
2 for repairable drift and 3 for unresolved violations. Plain dry-run returns 0
for a repairable plan or 3 for unresolved violations. Normal apply returns 1 if
unresolved violations prevent applying the plan. Other runtime errors return 1;
CLI argument errors return 2. Quiet mode preserves these codes.

## Daemon streams

```sh
syncer daemon --interval 86400 --output jsonl
syncer daemon --once --output json
```

Continuous daemons support human, table, text and JSON Lines. Each successful
cycle emits and flushes one result, with a query applied per cycle. Blocked plans
also emit a result; other failed cycles report an error on stderr and retry.
JSON and YAML require `--once` so stdout remains a single complete document.
