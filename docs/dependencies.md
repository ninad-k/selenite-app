# Dependency inventory

This inventory records the resolved direct dependencies in [Cargo.lock](../Cargo.lock) and the desktop [package-lock.json](../apps/desktop/package-lock.json) on 2026-10-13. Cargo license values were checked against the downloaded package manifests; npm license values come from the lockfile. First-party workspace crates are omitted from the external dependency tables.

Each direct dependency below has a permissive license. `OR` denotes the alternatives declared by its upstream package. The five explicitly approved MPL-2.0 transitive exceptions are listed separately. This document does not choose or change the repository's [LICENSE](../LICENSE).

## Rust

| Dependency | Resolved version | License | Purpose |
| --- | --- | --- | --- |
| `anyhow` | `1.0.104` | MIT OR Apache-2.0 | Attach context to CLI, index and desktop errors without coupling the core to application error handling. |
| `pulldown-cmark` | `0.13.4` | MIT | Parse Markdown events and source ranges in the core and render filtered preview events in the desktop shell. |
| `serde_yaml` | `0.9.34+deprecated` | MIT OR Apache-2.0 | Parse YAML frontmatter and serialize derived property values for the SQLite cache. |
| `rusqlite` | `0.40.2` | MIT | Provide transactional SQLite access with bundled SQLite for consistent builds. |
| `tantivy` | `0.26.2` | MIT | Index title and body for local full-text search, using the enabled mmap and LZ4 features. |
| `notify` | `8.2.0` | CC0-1.0 | Observe native filesystem changes that trigger debounced vault reconciliation. |
| `serde` | `1.0.229` | MIT OR Apache-2.0 | Derive serialization for the native desktop command responses. |
| `serde_json` | `1.0.154` | MIT OR Apache-2.0 | Write the opt-in native startup measurement report as JSON. |
| `chrono` | `0.4.45` | MIT OR Apache-2.0 | Determine the local calendar date and validate daily-note dates in the desktop layer. |
| `tauri` | `2.12.3` | Apache-2.0 OR MIT | Host the native window and webview and expose Rust-owned vault commands. |
| `tauri-build` | `2.7.2` | Apache-2.0 OR MIT | Generate native build resources and validate Tauri configuration and capabilities. |
| `tauri-plugin-dialog` | `2.8.2` | Apache-2.0 OR MIT | Provide the native vault-folder picker through the Rust command boundary. |
| `toml` | `1.1.8+spec-1.1.0` | MIT OR Apache-2.0 | Parse and serialize authoritative database schemas, saved views and optional service settings. |
| `url` | `2.5.8` | MIT OR Apache-2.0 | Validate property URLs and configured network endpoints without hand-written URL parsing. |
| `reqwest` | `0.13.5` | MIT OR Apache-2.0 | Provide bounded HTTP requests with Rustls for explicitly enabled model, sync and catalog adapters. |
| `sha2` | `0.10.9` | MIT OR Apache-2.0 | Identify source revisions, disposable embeddings and immutable plugin packages by SHA-256. |
| `base64` | `0.22.1` | MIT OR Apache-2.0 | Decode public keys/signatures and encode retained WASM packages. |
| `ed25519-dalek` | `2.2.0` | BSD-3-Clause | Verify signed catalogs and configurable entitlements using established Ed25519 implementations. |
| `wasmtime` | `36.0.17` | Apache-2.0 WITH LLVM-exception | Execute capability-limited WASM with fuel and memory bounds and no WASI imports. |
| `wat` (development) | `1.236.1` | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | Compile original readable WASM fixtures and SDK examples for executable ABI tests. |

`serde_yaml` is deprecated and its upstream README states that it is no longer maintained. It is retained because the agreed parser stack explicitly requested it. It handles frontmatter parsing, typed property edits and cache serialization; any future replacement must preserve vault YAML compatibility. No replacement dependency is introduced here. Wasmtime is pinned to the 36.0.17 LTS patch to keep the workspace Rust 1.90 minimum; its optional WASI support is disabled.

## Desktop JavaScript

| Dependency | Resolved version | License | Purpose |
| --- | --- | --- | --- |
| `svelte` | `5.57.2` | MIT | Render the welcome screen, navigable file tree and read-only note preview. |
| `@tauri-apps/api` | `2.12.3` | Apache-2.0 OR MIT | Invoke the Rust desktop commands from Svelte. |
| `vite` | `7.3.7` | MIT | Run the development server and build the local frontend assets. |
| `@sveltejs/vite-plugin-svelte` | `6.2.4` | MIT | Compile Svelte components during Vite development and builds. |
| `@tauri-apps/cli` | `2.12.2` | Apache-2.0 OR MIT | Run the native desktop development and build commands. |
| `@codemirror/state` | `6.7.6` | MIT | Manage editor documents and transactions. |
| `@codemirror/view` | `6.43.14` | MIT | Render the lazily loaded Markdown editor. |
| `@codemirror/commands` | `6.11.1` | MIT | Provide editing commands, keymaps and undo history. |
| `@codemirror/language` | `6.13.1` | MIT | Provide syntax highlighting and language support. |
| `@codemirror/lang-markdown` | `6.5.2` | MIT | Parse Markdown for editor syntax support. |
| `@codemirror/search` | `6.7.2` | MIT | Provide find and replace within the editor. |

The frontend uses system fonts and original inline SVGs. An optional HTTPS resource frame loads only after explicit URL/origin review. Node.js 24 is the configured CI runtime. Existing `serde`, `serde_json`, `chrono` and `anyhow` dependencies also support the new serialization, date validation and service boundaries.

## Approved transitive license exceptions

The user explicitly approved these five MPL-2.0-only packages on 2026-10-13 to enable the agreed Tauri 2 desktop stack. They are transitive dependencies, not new direct application dependencies. This approval is specific to the listed packages and does not authorize future non-permissive additions.

| Transitive dependency | Resolved version | License | Upstream purpose |
| --- | --- | --- | --- |
| `cssparser` | `0.37.0` | MPL-2.0 | Parse CSS syntax used by the desktop dependency graph. |
| `cssparser-macros` | `0.7.1` | MPL-2.0 | Supply procedural macros for `cssparser`. |
| `dtoa-short` | `0.3.5` | MPL-2.0 | Format floating-point values with controlled precision. |
| `selectors` | `0.38.0` | MPL-2.0 | Match CSS selectors used by the desktop dependency graph. |
| `option-ext` | `0.2.0` | MPL-2.0 | Supply additional operations on Rust `Option` values. |

Lockfiles pin the full transitive graph; these tables are a focused inventory of direct choices and the approved exceptions, not a complete third-party notice bundle. Dependency updates should recheck licenses and remain consistent with [ADR-001](adr/ADR-001-files-are-source-of-truth.md).
