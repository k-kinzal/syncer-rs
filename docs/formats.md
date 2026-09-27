# Structured document formats (v0.2.0 development)

These adapters are available in local v0.2.0 builds. The latest published release is v0.1.0; v0.2.0 has not been tagged or published.

Build `cargo build --release --workspace`, then explicitly install `libsyncer_extension_structured.dylib` on macOS, `libsyncer_extension_structured.so` on Linux, or `syncer_extension_structured.dll` on Windows. Resolve symlinked library paths before installation. The existing JSON extension remains independently installable.

```sh
syncer extension install ./target/release/libsyncer_extension_structured.dylib
```

All document adapters share `set`, `ensure`, `array`, and `remove` semantics with JSON. Use `set` for an exact value and `ensure` to keep any compliant personal value. `array` preserves acceptable personal members and repairs required/forbidden members. Operations must be representable in the selected format; an XML attribute, INI value, dotenv variable, properties value, or CSV cell is a string, not an array or number.

Pointers use RFC 6901 escaping: `/` inside a key is `~1`, and `~` is `~0`. Array/row indices are zero-based canonical integers, not wildcards. No JSONPath or XPath evaluation occurs. Missing object fields can be added; a scalar parent is never replaced implicitly. Array indices address existing members. Removing an array element that shifts another value into the selected pointer is not idempotent and is rejected; use `array` constraints for membership removal.

| `kind` | Selection example | Preservation and scope |
|---|---|---|
| `json` | `/sandbox/network/allowedDomains` | Existing JSON extension; unmanaged values remain, formatting normalizes on change. |
| `jsonc` | `/editor.tabSize` | Comments and trailing commas; rejects JSON5-only syntax and duplicate keys. Retains surrounding syntax/comments. |
| `json5` | `/server/port` | Quoted/unquoted keys, single quotes, hexadecimal numbers and trailing commas. Rejects duplicate keys and non-finite numbers. Retains surrounding syntax/comments. |
| `yaml` | `/services/api/port` | One YAML document, JSON-compatible string-keyed maps and values. Unmanaged values are verified. Comment loss is rejected. Layout can normalize when a subtree is replaced. |
| `toml` | `/server/port`, `/servers/0/port` | Tables, inline tables, arrays and arrays of tables; preserves surrounding comments/layout. Null is unsupported. |
| `hcl` | `/resource/aws_instance/web/ami` | Traverse existing block type and labels, then an attribute and optional object/array path. Patch literal values; retain expressions elsewhere without evaluating them. Duplicate matching blocks are rejected. |
| `xml` | `/config/server/0/@port`, `/config/name/0`, `/config/name/0/#text` | Attributes or leaf text using source ranges. Preserve bytes outside the selected range, including namespaces and unrelated comments. Refuse DTDs and mixed-content replacement. |
| `ini` | `/key`, `/section/key` | Case-sensitive string keys/values, `=` or `:` separators, `#`/`;` comments. Preserve other lines and the selected line's spacing/comment. Repeated sections/keys are refused. |
| `dotenv` | `/API_URL` | String values, optional `export`, single/double quotes and inline `#` comments. Preserve other lines. No variable expansion or shell execution. Multiline quoted input is refused; escaped newlines in double quotes are supported. |
| `properties` | `/service.timeout` | UTF-8 Java properties with escaped keys, Unicode escapes and continuation lines. Preserve other logical entries. New non-ASCII characters use Unicode escapes. |
| `csv` | `/0/timeout` | Header names identify columns, row indices exclude the header. Cells are strings. Preserve unchanged rows; quote changed rows as needed. |
| `tsv` | `/0/timeout` | Same model as CSV, with tab delimiters. |
| `plist` | `/Sandbox/Enabled` | XML property lists: dictionaries, arrays, booleans, integers, reals, strings, dates and data. Unmanaged native values/types remain; XML formatting/comments normalize on change. Binary plists are outside this text adapter. |

Every changed output is parsed again before it is returned. A compliant `ensure` or a `check` returns the original bytes. Core dry-run/apply, inherited policy guards, allowed roots, backups and conflict checks apply to all formats.

## Examples

```hcl
schema_version = 1
name = "developer-files"

rule "minimum-workers" {
  target = "config.yaml"
  kind = "yaml"
  pointer = "/server/workers"
  operation = "ensure"
  value = 1
  override = "constrained"
  constraints { min = 1 }
}

rule "cargo-jobs" {
  target = ".cargo/config.toml"
  kind = "toml"
  pointer = "/build/jobs"
  operation = "set"
  value = 4
}

rule "xml-endpoint" {
  target = "config.xml"
  kind = "xml"
  pointer = "/config/server/0/@url"
  operation = "set"
  value = "https://api.example.com"
}
```

HCL selection follows the source structure: `resource "aws_instance" "web" { ami = "..." }` exposes `/resource/aws_instance/web/ami`. A pointer must end at an attribute or its literal descendants, not at a whole block. Existing parent blocks are required; the adapter does not infer labels or generate new blocks. Add a missing attribute by selecting it directly. A dynamic expression in the selected value fails without changing the document.

XML children always require an index, even if only one matches. Use expanded names for namespaces, for example `/config/{urn:app}server/0/@port`. Escape namespace URI slashes with `~1`. Qualified names being added require an already declared namespace. XML files need an existing root; use a template when creating one. `remove` removes an attribute or leaf element; clear `#text` by setting an empty string. Removing a repeated element cannot shift another match into the same pointer.

CSV/TSV require a header with unique, nonempty column names and consistent row widths. Set a cell to `""` to clear it; removing a column from one row would invalidate the table. Policy values are strings even for numeric-looking cells.

TOML datetimes use `{ "$toml_datetime" = "2026-09-27T00:00:00Z" }` when selecting/replacing their value. Plist date/data values use `{ "$plist_date" = "2026-09-27T00:00:00Z" }` and `{ "$plist_data" = "AQID" }` (base64). These single-key shapes are reserved typed literals when writing those formats. Null cannot be written to TOML or plist; remove the field instead.

These are configuration-data adapters. They do not execute HCL, source dotenv files, resolve XML entities, or interpret application-specific INI dialects. Files and output are limited to 16 MiB and UTF-8. Syntax that cannot be parsed or round-tripped safely is reported as an error.
