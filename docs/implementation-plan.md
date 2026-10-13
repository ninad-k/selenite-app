# Parallel implementation plan

Authorized on 2026-10-13: implement all previously deferred phases concurrently
where their dependencies permit. Keep service endpoints and paid capabilities
configurable. This supersedes the original Phase-0-only scope; the filesystem,
license, deletion and remote-push rules remain in force.

The implementation stays in the agreed workspace. Markdown/YAML holds content;
TOML files hold authoritative configuration, schemas, saved views and plugin
permissions. Search/vector indexes remain disposable. No hosted service, cloud
account, purchase, publication or remote push is performed by this development
work. Existing Phase 0 verification limitations remain recorded separately.

## Work tracks

| Track | Deliverable | Depends on |
| --- | --- | --- |
| 1. Editing and links | Conflict-aware durable saves, reviewed rename application, CodeMirror editor, backlinks, graph and navigation | Existing core |
| 2. Databases and faces | Typed TOML schemas, saved filter/sort/group queries; table, board, calendar and gallery; reusable diary/workspace/GTD templates | Core persistence |
| 3. Sync | SyncProvider contract, Google Drive and OneDrive adapters, conflict-preserving synchronization, configurable credentials/endpoints | Core persistence |
| 4. AI and MCP | BYO provider configuration, hybrid lexical/vector retrieval with source citations, stdio MCP vault tools | Core, index and DB |
| 5. Plugins and distribution | Wasmtime sandbox, explicit capabilities, configurable catalog/install workflow, SDK example, configurable Pro entitlements | Core persistence |
| 6. Integration and release checks | Desktop/CLI commands, service settings, documentation, native and workspace regression checks | Tracks 1–5 |

Provider-backed integrations will be tested with deterministic local transports
and fixtures. Actual cloud-account authentication, model-provider requests,
marketplace publication and payment settlement require operator configuration
and separate live verification. A working configurable adapter is distinct from
a claim of a deployed marketplace or paid service.

## Working process

Subagents own disjoint files, coordinate API contracts before integration, and
return focused tests and license inventories. The lead integrates reviewed
changes and creates small conventional commits. Shared manifest/lockfile edits
and Cargo builds are coordinated. No feature is labelled complete solely because
a stub or mock exists. Acceptance evidence and remaining limitations are updated
as each track lands.
