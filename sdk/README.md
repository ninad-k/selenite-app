# Selenite plugin SDK

These original examples target Selenite's core WebAssembly JSON ABI v1. Plugins have no WASI, filesystem, networking, environment-variable or clock imports. The host exposes only the capabilities described in [Plugin hosting](../docs/plugins.md).

## Build the WAT examples

From the repository root, with Cargo caches and temporary directories configured as in the main build instructions:

```powershell
cargo run -p selenite-plugin-host --example compile_wat -- sdk/wat/echo.wat .local/plugin-examples/echo.wasm
cargo run -p selenite-plugin-host --example compile_wat -- sdk/wat/list-pages.wat .local/plugin-examples/list-pages.wasm
```

The compiler helper creates a new output file and refuses to overwrite an existing file. Use another output filename for a later build. Matching manifests are beside the WAT sources. The echo example has no capabilities. The list example requires the manifest request and an explicit `vault_list` grant; installation alone does not grant access.

The [Rust echo example](rust/echo.rs) can be compiled with an already installed `wasm32-unknown-unknown` target:

```powershell
rustc --edition=2024 --crate-type=cdylib --target wasm32-unknown-unknown sdk/rust/echo.rs -o .local/plugin-examples/rust-echo.wasm
```

Use a new output filename when retaining previous builds. This is guest code, not a new application workspace crate. The host owns instance lifetime, so its small allocator deliberately keeps allocations until that invocation ends.

## ABI

| Direction | Function | Meaning |
| --- | --- | --- |
| Guest export | `memory` | One 32-bit linear memory |
| Guest export | `alloc(length: i32) -> i32` | Allocate a writable buffer for the host's input JSON |
| Guest export | `run(pointer: i32, length: i32) -> i64` | Process input and return an output pointer in the high 32 bits and byte length in the low 32 bits |
| Optional guest import | `selenite.call(request_ptr: i32, request_len: i32, output_ptr: i32, output_capacity: i32) -> i32` | Execute one granted host request and return its JSON response length |

All strings are UTF-8 and lengths are bytes. The output of `run` must be valid JSON. `selenite.call` requires an output buffer whose capacity equals the configured `max_message_bytes`; the list example uses the default 1 MiB budget. The host checks ranges before dispatch, including before any write. Invalid ABI calls trap. Application errors, denied capabilities and conflicts return `{"ok":false,"error":"..."}`; success returns `{"ok":true,"data":...}`. Instances and grants are fresh for every run.

Host request examples:

```json
{"op":"list","offset":0,"limit":100}
{"op":"read","path":"Journal/2026-10-13.md"}
{"op":"write","path":"Note.md","expected_source":"Exact old bytes","source":"Exact replacement bytes"}
{"op":"write","path":"New.md","expected_source":null,"source":"# A new note\n"}
```

`list` returns `paths` and optional `next_offset`. `read` returns `path` and `source`. `write` with a non-null expected source performs compare-and-swap; null requests exclusive creation and cannot overwrite an existing page. The host validates Markdown/frontmatter and preserves replaced content in the core's history. A guest cannot read or write `.selenite` metadata, rename or delete files, or execute a shell.

The byte and operation limits are explicit: default fuel 5,000,000, memory 16 MiB, module 2 MiB, message 1 MiB and 64 host calls. Listings stop above 20,000 filesystem entries or four message budgets of accumulated path bytes; each page-list request returns at most 1,000 paths. Page read responses reserve the worst-case JSON escaping budget, so readable source size is `(max_message_bytes - 512) / 6`. Plugin page paths use forward slashes and are at most 256 bytes. Fuel controls guest execution, not JIT compilation or disk latency.

For distribution, publish exact binary bytes and a signed catalog entry with their SHA-256 digest. See the [catalog protocol](../docs/plugins.md#signed-catalog-and-package-installation). No public catalog, signing key or publisher service is configured by the repository.
