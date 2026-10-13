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

The local implementation includes CodeMirror editing, conflict-aware saves,
reviewed renames, wikilinks and note/block previews, backlinks, a graph, typed
databases and all four view layouts. Reusable templates provide diary and GTD
behavior. SQLite/Tantivy indexes and AI vectors are rebuildable caches.

Optional integrations include Google Drive/OneDrive version archives, cited
hybrid AI retrieval with BYO models, MCP stdio tools, a Wasmtime plugin sandbox,
signed plugin catalogs and configurable signed entitlements. Services are
disabled until configured and explicitly invoked. No hosted marketplace,
billing service, model service or cloud account is deployed by this repository.

## Architecture

```text
crates/
  selenite-core/         Markdown pages, blocks, properties, links, vault access
  selenite-index/        SQLite, Tantivy, rebuild, incremental file watcher
  selenite-db/           Typed schemas, saved views, filters and property writes
  selenite-sync/         SyncProvider adapters, reviewed plans, conflict recovery
  selenite-plugin-host/  Wasmtime sandbox, signed catalogs and entitlements
  selenite-ai/           BYO providers, hybrid retrieval and source citations
  selenite-mcp/          Bounded read-only/opt-in-write stdio vault tools
  selenite-cli/          Headless vault commands
apps/desktop/           Tauri 2 + Svelte shell
sdk/                    Original plugin ABI examples and authoring guide
docs/adr/               Architecture decisions
templates/              Plain Markdown templates
.github/workflows/      Windows, Linux, macOS checks
```

Apps and CLI compose the index, database and optional service crates; index and db depend on core. Core has no
dependency on another workspace crate, no UI, and no network access. Daily-note
calendar behavior belongs to the desktop shell.

See [ADR-001](docs/adr/ADR-001-files-are-source-of-truth.md) for persistence and
recovery rules, [dependency choices](docs/dependencies.md) for versions and
licenses, and [Phase 0 verification](docs/phase-0-verification.md) for measured
results and remaining acceptance checks. The [parallel implementation plan](docs/implementation-plan.md)
records the authorized later scope; [expanded verification](docs/all-phases-verification.md)
records its checks and remaining live acceptance work. Guides cover [databases](docs/databases.md),
[sync](docs/sync.md), [AI](docs/ai.md), [MCP](docs/mcp.md), [plugins](docs/plugins.md),
and [optional entitlements](docs/pro.md).

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
npm --prefix apps/desktop test
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
cargo run -p selenite-cli --bin selenite -- init-gtd "path-to-vault"
cargo run -p selenite-cli --bin selenite -- views "path-to-vault"

# With no argument, choose Open a vault on the welcome screen.
cargo run -p selenite-desktop --release -- "path-to-vault"
```

The desktop also accepts `SELENITE_VAULT`. It opens the local calendar day's
`Journal/YYYY-MM-DD.md`, creating it if absent. Put an optional custom template at
`<vault>/.selenite/templates/daily.md`; `{{date}}` and `{{title}}` are substituted.
Otherwise it uses [the bundled template](templates/daily.md). Existing notes are
never overwritten. The folder chooser can switch vaults; the file tree opens
other notes. Choose Edit for CodeMirror; Ctrl+S saves. Unsaved edits are protected
during navigation and window closing. Raw HTML is escaped and Markdown images
remain omitted; wikilinks navigate locally and standalone note/block embeds show
bounded previews without recursive expansion.
The Resource panel can display an explicitly reviewed HTTPS embed beside the
note. External sites may disallow framing; no resource loads automatically.

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
Preserve `.selenite/templates`, schema/view definitions and service settings when managing
caches. A malformed note reports an error; it is never silently rewritten or
dropped. Symlinked pages and directories are excluded, and path traversal is
rejected.

Renames show a review before applying source-preserving link updates. Saves use
exact-source checks, synced staging and atomic per-file publication. Previous
sources and multi-file rename journals are retained under `.selenite`; an
interrupted rename can be explicitly resumed with `selenite recover-renames`.
Atomic creation requires local storage supporting hard links. Locks coordinate
Selenite processes, while unrelated editors can still race; changed source bytes
cause a conflict instead of a silent overwrite. Use Refresh from disk to pick up
external changes; unsaved drafts remain protected.

Run `selenite --help` for database, AI, sync, plugin and MCP commands. Service
settings are vault-local TOML with environment-variable names for secrets. Sync
uses immutable remote JSON version archives, not an editable folder mirror.
Signed plugin catalogs and paid capability names are deployment configuration;
all community note, database and search functions remain available without an
entitlement. Real cloud accounts, model providers and hosted commerce still need
operator configuration and live acceptance tests.

No account or hosted service is required. See the existing repository
[LICENSE](LICENSE).
