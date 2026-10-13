# Phase 0 dependencies

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

`serde_yaml` is deprecated and its upstream README states that it is no longer maintained. It is retained in Phase 0 because the agreed parser stack explicitly requested it. Its use is limited to frontmatter parsing and cache serialization; any future replacement must preserve vault YAML compatibility. No replacement dependency is introduced here.

## Desktop JavaScript

| Dependency | Resolved version | License | Purpose |
| --- | --- | --- | --- |
| `svelte` | `5.57.2` | MIT | Render the welcome screen, navigable file tree and read-only note preview. |
| `@tauri-apps/api` | `2.12.3` | Apache-2.0 OR MIT | Invoke the Rust desktop commands from Svelte. |
| `vite` | `7.3.7` | MIT | Run the development server and build the local frontend assets. |
| `@sveltejs/vite-plugin-svelte` | `6.2.4` | MIT | Compile Svelte components during Vite development and builds. |
| `@tauri-apps/cli` | `2.12.2` | Apache-2.0 OR MIT | Run the native desktop development and build commands. |

The frontend uses system fonts and original inline SVGs, with no remote assets or additional UI dependency. Node.js 24 is the configured CI runtime.

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
