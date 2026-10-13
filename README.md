# Selenite

Selenite is a local-first desktop app for notes, journaling, databases, and tasks.
One versatile core engine is shaped through templates and views into four faces:

| Face | Intended experience |
| --- | --- |
| Diary | Daily notes, with today's page opened on launch |
| Second brain | Wikilinks, backlinks, and a graph of connected notes |
| Workspace | Databases with table, board, calendar, and gallery views, plus rich embeds |
| GTD task manager | Inbox, next actions, contexts, projects, waiting-for, someday/maybe, and weekly review as database views |

The Community edition is free and open source. Plain Markdown files on disk are
the source of truth. A page is one `.md` file; its filename supplies its title.
Properties live in YAML frontmatter, blocks may carry `^block-id`, and links use
`[[page]]`, `![[page]]`, or `[[page#^block]]`.

Phase 0 implements parsing, backlink resolution, source-preserving rename plans,
rebuildable SQLite/Tantivy indexes, debounced file watching, three CLI commands,
and a Tauri 2 + Svelte vault browser with a read-only daily-note preview.
The table above describes the product direction: database schemas/views, graph
UI, CodeMirror editing, sync, plugins, AI, and MCP are future work. Provider traits
and placeholder crates establish boundaries without implementing those features.

## Architecture

```text
crates/
  selenite-core/         Markdown pages, blocks, properties, links, vault access
  selenite-index/        SQLite, Tantivy, rebuild, incremental file watcher
  selenite-db/           Schema/view/query placeholder
  selenite-sync/         SyncProvider trait stub
  selenite-plugin-host/  Placeholder
  selenite-ai/           LlmProvider trait stub
  selenite-mcp/          Placeholder
  selenite-cli/          Headless vault commands
apps/desktop/           Tauri 2 + Svelte shell
sdk/                    Reserved SDK directory
docs/adr/               Architecture decisions
templates/              Plain Markdown templates
.github/workflows/      Windows, Linux, macOS checks
```

Apps and CLI depend on index/db/core; index and db depend on core. Core has no
dependency on another workspace crate, no UI, and no network access. Daily-note
calendar behavior belongs to the desktop shell.

See [ADR-001](docs/adr/ADR-001-files-are-source-of-truth.md) for persistence and
recovery rules, [dependency choices](docs/dependencies.md) for versions and
licenses, and [Phase 0 verification](docs/phase-0-verification.md) for measured
results and remaining acceptance checks.

## Build and run

Use stable Rust **1.90 or newer**, Node.js **24**, and npm. Windows also needs the
MSVC C++ build tools and WebView2 Runtime. Linux needs the desktop libraries listed
in [CI](.github/workflows/ci.yml); macOS needs Xcode command-line tools.

From the repository root, this PowerShell setup keeps build caches and temporary
files inside the checkout:

```powershell
$env:CARGO_HOME = Join-Path $PWD '.local/cargo'
$env:TEMP = Join-Path $PWD '.local/tmp'
$env:TMP = $env:TEMP
$env:npm_config_cache = Join-Path $PWD '.local/npm'
$env:npm_config_userconfig = Join-Path $PWD '.local/npmrc'
New-Item -ItemType Directory -Force $env:CARGO_HOME, $env:TEMP, $env:npm_config_cache | Out-Null

npm --prefix apps/desktop ci
npm --prefix apps/desktop run build
cargo build --workspace --locked
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

Build the frontend before the Rust workspace: Tauri embeds its generated assets.
On Linux/macOS, use the same npm/Cargo commands after setting any desired cache
environment variables with your shell's syntax. CI is configured to build the
full workspace on all three operating systems.

```powershell
# Use an existing folder as a vault, or create a new empty folder first.
cargo run -p selenite-cli --bin selenite -- rebuild "path-to-vault"
cargo run -p selenite-cli --bin selenite -- search "path-to-vault" "ownership"
cargo run -p selenite-cli --bin selenite -- backlinks "path-to-vault" "Projects/Rust.md"

# With no argument, choose Open a vault on the welcome screen.
cargo run -p selenite-desktop --release -- "path-to-vault"
```

The desktop also accepts `SELENITE_VAULT`. It opens the local calendar day's
`Journal/YYYY-MM-DD.md`, creating it if absent. Put an optional custom template at
`<vault>/.selenite/templates/daily.md`; `{{date}}` and `{{title}}` are substituted.
Otherwise it uses [the bundled template](templates/daily.md). Existing notes are
never overwritten. The folder chooser can switch vaults; the file tree opens
other notes. Raw HTML is escaped and embedded images are omitted in this preview.

For frontend development, run `npm --prefix apps/desktop run dev` in one terminal
and `cargo run -p selenite-desktop --no-default-features` in another. Apply the
repository-local environment setup in both terminals if you want local caches.
Release builds use thin LTO and one codegen unit to reduce native executable size.

## Vault and CLI behavior

CLI search emits tab-separated score, vault-relative path, and title. Backlinks
emit source paths and optional fragments. Tabs/newlines/backslashes in fields are
escaped; errors go to stderr with a nonzero exit status. Bare page names must be
unambiguous; explicit paths identify a page directly. Link resolution is
case-sensitive on every platform and accepts unique basenames, vault-relative
paths, and explicit `./` or `../` paths relative to the source note.

Every CLI query reconciles the cache with disk first. Long-lived consumers can
use `VaultWatcher` and `Index::search` independently. The benchmark measures the
query API; process startup and a full CLI reconciliation are separate costs.

SQLite and Tantivy files live under `<vault>/.selenite/cache`. `selenite rebuild`
derives both from Markdown, including recovery from incompatible/corrupt caches.
Such caches are retained in a sibling `cache-retired-*` directory for inspection.
Preserve `.selenite/templates` and future schema/view definitions when managing
caches. A malformed note reports an error; it is never silently rewritten or
dropped. Symlinked pages and directories are excluded, and path traversal is
rejected.

Rename planning currently returns reviewed source edits in memory; applying a
multi-file rename is future work. New-note creation uses exclusive creation and
never replaces an existing file; an I/O failure during the initial write may
leave a partial newly created note for inspection. External edits appear when
the desktop reopens a page or refreshes today's view; automatic preview refresh
is not part of this foundation.

No account or hosted service is required. See the existing repository
[LICENSE](LICENSE).
