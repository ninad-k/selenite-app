# Expanded implementation verification

Date: 2026-10-13. Work is on `codex/all-phases`, entirely inside the authorized
checkout. The user authorized all later tracks and configurable services/paid
capabilities. No remote push, hosted publication or account connection occurred.

## Implemented scope

| Track | Local implementation |
| --- | --- |
| Editing and knowledge | Lazy CodeMirror editor, exact-source saves, retained history, journaled renames, wikilink navigation, note/block previews, backlinks and graph |
| Databases and faces | Typed YAML properties, folder/tag databases, TOML schemas/views, filters/sorts/groups, table/board/calendar/gallery, reusable GTD templates |
| Sync | Google Drive and OneDrive adapters, immutable remote versions, reviewed plans, preserved conflicts, explicit merged resolution and retry checkpoints |
| AI | Configurable OpenAI-compatible models, environment-variable credentials, BM25/vector retrieval, current-source validation and citations |
| MCP | Bounded stdio JSON-RPC vault tools, read-only default and explicit opt-in CAS writes |
| Plugins | Wasmtime without WASI, explicit grants, fuel/memory bounds, signed catalogs, content-addressed installation and original SDK examples |
| Pro | Configurable signing keys and capability names, signed entitlement verification; community features remain available |
| Desktop/CLI | Native editing and database workbench, explicit optional service controls, headless commands and recovery tools |

See the component guides for concrete APIs, settings, bounds and recovery behavior.
These implementations do not establish that external services are deployed or
that every provider integration has been tested against a live account.

## Acceptance evidence

Windows checks use Rust 1.95.0 and Node.js 24.21.0. The final implementation passes
`cargo fmt --all --check`, `cargo test --workspace --locked` (**136 tests**),
`cargo clippy --workspace --all-targets --locked -- -D warnings` (**zero warnings**)
and `cargo build --workspace --locked`. The six frontend helper tests and the
Svelte/Vite production build pass. Vite reports an advisory for the lazily loaded
CodeMirror chunk size. Logs are retained in `.local/all-phases-verification/`.

Regression coverage includes compare-and-swap writes, coherent index snapshots,
multi-file rename recovery, typed relations, schema preconditions, journal-note
renames, unavailable embeds, real MCP subprocess IO, sync retries and merged
heads, real WASM execution, signed catalogs and changed plugin review inputs.

Earlier foundation measurements remain in [Phase 0 verification](phase-0-verification.md);
they are historical evidence, not measurements of the expanded executable.

The native debug WebView passed **18 functional checks**, with zero unhandled
exceptions and zero provider connection attempts. Evidence and screenshots are
retained at `.local/workbench-smoke/1791893496817-111944/`. This run exercised
CodeMirror saves, unsaved navigation/window-close protection, external-edit
conflicts, search/backlinks/graph, ordinary and daily-note renames, all four
database layouts, TOML editing, disabled services and entitlement rejection.
Its timing was collected during a heavily loaded release build and is not used
as startup-performance evidence. Native screenshots were visually inspected.

### Final release measurements

The final release CLI, desktop and benchmark example built successfully. After
compilation finished, the 5,000-note CLI rebuild completed in **2,340.207 ms**
(5,000 pages, 5,000 links, 25,000 properties). Six query workloads, each with five
warmups and 50 measurements, produced a worst query-API **p95 of 3.610 ms** and an
absolute maximum of **4.413 ms**. Cache opening took 4.362 ms. The complete ranked
snapshot matched the retained Phase 0 baseline, including paths, titles, score
bits and order; all Markdown bytes matched the generator and remained unchanged.

Three end-to-end CLI searches took **683.255, 649.945 and 671.534 ms**. They include
process startup and full disk reconciliation, so the 100 ms target is met by the
query API, **not by the complete CLI invocation**. Evidence is retained as
`rebuild.txt`, `search.metrics.txt`, `search.tsv` and `cli-search-ms.txt` inside
`.local/all-phases-verification/`.

Five release Tauri launches with the preselected 5,001-note desktop fixture
rendered its tree and today's page in **692.673, 723.870, 667.623, 627.078 and
701.629 ms**, measured from process spawn to the frontend readiness report.
All were below one second. The first used a fresh WebView profile; the remaining
runs reused it. OS caches were not cleared, and human folder selection was not
timed. The native checks also verified navigation/cancellation and rejection of
paths outside the vault. Evidence and a screenshot are retained at
`.local/native-smoke/1791893736542-42064/`.

### Acceptance status

| Original foundation criterion | Expanded implementation result |
| --- | --- |
| Windows build/tests/zero Clippy warnings | Passed; 136 Rust tests, six frontend tests, 18 native functional checks |
| 5,000-note rebuild and identical cache recovery | Rebuild and exact baseline equality passed; prior absent-cache recovery passed; literal deletion remains unperformed |
| Search below 100 ms | Query API passed at 3.610 ms worst p95; full CLI remains above 100 ms |
| Desktop opens vault and today's note below 1 s | Passed in five local release launches, 627–724 ms |
| Vault opens unchanged in Obsidian | Plain Markdown/YAML and byte preservation checked; actual Obsidian opening remains deferred |

The next acceptance tasks are live provider/MCP-client checks, execution of the
three-platform CI matrix after an authorized push, and the deferred Obsidian
interoperability check. These are validation/deployment work, not claims that a
hosted marketplace or paid service was launched.

## Remaining live acceptance work

- Execute the configured Windows/Linux/macOS GitHub Actions matrix after an
  authorized push. Windows local checks do not verify the other operating systems.
- Connect real Google Drive and OneDrive archive folders and validate their
  credentials, pagination, conflicts and retries. Current transport tests use
  local HTTP servers and in-memory fixtures. OAuth login/refresh is external.
- Configure a real model endpoint and verify model-specific API compatibility,
  embedding dimensions, citation quality, latency and cost. No real note content
  has been sent to a model during implementation.
- Connect the stdio MCP server to the intended desktop client. Tests include a
  real spawned CLI process, but no Claude Desktop session was available.
- Deploy a signed catalog and entitlement issuer if wanted. Hosting, billing,
  payment processing, OAuth registration and a paid feature list are deployment
  decisions; none is fabricated by the application.
- Open the generated plain Markdown/YAML vault in Obsidian when convenient.
  Source-byte checks pass; the user could not perform the actual opening check.
- The earlier literal cache-deletion check was blocked by automatic approval
  review. Retaining the cache elsewhere and rebuilding from an absent cache
  verified identical ranked results without deleting files.

The Rust guest SDK example is source-only in this environment; the two original
WAT examples execute in automated Wasmtime tests. Wasmtime fuel bounds guest
execution, not compilation cost. Local writes coordinate cooperating Selenite
processes; unrelated editors do not honor these locks. Per-file atomic writes
are not a distributed transaction. No user-content garbage collection is built.
