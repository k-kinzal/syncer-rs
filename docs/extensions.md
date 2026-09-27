# Native extension SDK

Build official extensions with `cargo build --release --workspace`. Libraries are under `target/release/` and independently installable:

```sh
syncer extension install ./libsyncer_extension_json.dylib --sha256 DIGEST
syncer extension install ./libsyncer_extension_http.dylib
syncer extension list
```

Linux uses `libsyncer_extension_NAME.so`; Windows uses `syncer_extension_NAME.dll`. The host copies installed libraries into private state and stores SHA-256; later loads reject modified files. Installing a library executes its code, so installation is an explicit local trust decision. No library is automatically downloaded or enabled by a policy. Dependency/ABI incompatibility is a load error, not an implicit fallback. Duplicate capabilities are rejected.

## Local development without installation

In v0.2.0 development builds, opt in to loading official libraries beside the
running executable:

```sh
cargo build --release --workspace
target/release/syncer --dev-extensions extension list
# Or enable the mode for subsequent commands in this shell:
export SYNCER_DEV_EXTENSIONS=1
target/release/syncer apply --dry-run
```

This finds `http`, `git`, `json`, and `structured` under
their usual platform filenames. The directory is the physical executable's
directory, even when launching through PATH or a symlink; the working directory
and its subdirectories are never searched. Missing libraries are skipped; broken
libraries, wrong manifest names, incompatible ABIs and capability conflicts are
errors. Other library filenames are ignored.

Nothing is copied, enrolled or downloaded. A rebuilt library is picked up on the
next command (or daemon cycle). Explicitly installed extensions with the same
name take precedence, retaining their pinned digest checks; remove the installed
copy to try the adjacent build. `extension remove` only removes installed copies.
Use `--dev-extensions=false` or `SYNCER_DEV_EXTENSIONS=0` to disable development
loading. This mode defaults to off.

Enabling this mode trusts executable code from the build directory, without a
persistent integrity pin. Keep it for local testing in your own build directory.
Policies cannot write inside that directory while the mode is enabled.

## ABI v1

Export these C functions using `syncer_extension_sdk::export_extension!(dispatch)`:

```c
uint32_t syncer_extension_abi_version(void);
char *syncer_extension_invoke(const char *request_json);
void syncer_extension_free(char *response_json);
```

Inputs are borrowed NUL-terminated UTF-8 JSON for the call duration. The extension owns returned buffers and frees them via its own allocator. Responses are `{ "result": ..., "error": null }` or `{ "result": null, "error": "..." }`. The SDK contains unwinding panics; aborts or invalid pointers can still crash the process. Methods execute serially per library on host blocking workers. No Rust strings, trait objects, futures or allocators cross the C boundary. ABI version and wire `schema_version` are separate compatibility axes.

A request is `{ "method": "...", "params": {...} }`. `manifest` declares `abi_version`, `name`, `version`, `methods`, `schemes`, `kinds`, `targets`.

| Method | Request parameters | Result |
|---|---|---|
| `fetch` | `uri`, local `auth` object | base64 `data`, optional opaque `revision` |
| `publish` | `uri`, `auth`, base64 `data`, `revision` | transport result; must honor conditional revision |
| `report` | `uri`, `auth`, base64 encrypted `data` | transport receipt |
| `resolve` | `target`, `home`, `project` | absolute `path` (host revalidates approved roots) |
| `document` | `content`, serialized `rule`, `action` | `content`, boolean `compliant` |

Document actions are `apply`, `check`, `constraints`. `check` validates the full postcondition without changing content; `constraints` ignores the desired value and checks only constraints. `apply` returns proposed content and pre-application compliance. Host validates final postconditions across all rules. Pure document/target methods must not perform network or filesystem writes. Sources are capped at 16 MiB decoded.

## Official extensions

- `http`: HTTP(S) GET, conditional PUT for policy publishing, POST for encrypted report submission. Redirects are rejected; enroll the final endpoint. Optional `--credential access_token=ENV_NAME`. Authenticated non-loopback endpoints must use HTTPS. A server must actually implement ETag/If-Match for conditional publishing.
- `git`: `git+https://github.com/org/repo.git?ref=main#path/policy.hcl` or `git+ssh://git@github.com/org/repo.git?ref=main#path/policy.hcl`. Requires installed Git. Fetches into a temporary bare repository, without checkout/hooks. No embedded HTTPS passwords and no interactive prompts. SSH agent auth is available; Git global/system configuration is disabled. Read-only in v1; publish using normal Git workflows.
- `json`: strict JSON field/array patches and constraints.
- `structured` (v0.2.0 development): YAML, TOML, HCL, XML, JSONC, JSON5, INI, dotenv, Java properties, CSV, TSV and XML plist selected-field patches. See [selectors and format guarantees](formats.md).

Service-specific transports and application aliases are outside the current
official set pending validation. Locally built extensions require explicit
installation. HTTPS sources select their transport by URI scheme; use
`--extension NAME` when explicitly enrolling a service-specific handler for an
HTTPS URL. The CLI does not infer a service provider from a hostname. Existing
enrollments with an explicit extension name retain that choice.

External APIs beyond file synchronization (for example repository settings) are a third-party extension responsibility. The generic async interface supports custom methods, but v1 CLI planning does not orchestrate arbitrary external side effects as file transactions.

References: [Rust linkage](https://doc.rust-lang.org/reference/linkage.html), [HCL](https://github.com/hashicorp/hcl), [age encryption](https://docs.rs/age/0.11.5/age/).
