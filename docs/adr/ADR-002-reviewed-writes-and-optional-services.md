# ADR-002: Reviewed writes and optional services

Status: implemented within the existing workspace architecture.

## Context

Editing, multi-file renames and remote integrations introduce concurrent writers.
The files-as-source-of-truth rule applies equally to the desktop, CLI, plugins,
AI and MCP. SQLite/Tantivy and vectors must not become a second authority.

## Decision

Core owns portable paths, symlink containment, bounded reads, exact-source
compare-and-swap writes, synced staging and retained history. Exclusive creation
publishes a complete staged inode using a hard link. Replacement uses same-volume
rename. Cooperating writers use `.selenite/write.lock`; multi-file readers take a
shared lock. External editors do not participate in this advisory protocol.

Multi-file renames record before/after sources and a complete source snapshot in
a retained journal. Publication is not presented as filesystem-wide atomicity.
Explicit recovery rolls forward only when disk bytes still match the recorded
before/after conditions. Nothing automatically discards history or conflict files.
Schema-dependent rename plans retain their exact metadata file set and source
bytes. Publication and recovery recheck these under the same exclusive lock as
the Markdown edits, so concurrent schema changes invalidate the plan.

Schemas, views and configuration remain TOML files. Property edits may reserialize
YAML formatting but preserve Markdown body bytes. Sync stores immutable remote
versions and checkpoints, reports missing history, and preserves divergent
Markdown copies. A reviewed merge creates a new child of the competing revisions.
No remote delete propagation is enabled.

Network access belongs to optional service crates and explicitly loaded embedded
content, never core. Model keys and sync tokens are environment-variable values;
configuration stores their variable names. AI sends only explicitly scoped
embedding inputs or retrieved context, returns citations, and has no write tool.
MCP writes require process-launch opt-in and source preconditions.

Wasmtime receives no WASI or ambient imports. Explicit manifest capabilities and
local grants intersect; resource budgets bound guest execution and host calls.
Catalog and entitlement signatures have separate signing domains. Installing a
plugin grants no capability. Community content operations never depend on Pro
entitlements; deployments configure any additional paid capability names.

## Consequences

Retained history, sync versions and packages consume storage; cleanup requires a
separate explicit policy. Hard-link publication excludes filesystems lacking
hard-link support. Advisory locking cannot eliminate races with arbitrary
external editors. Hosted services and credentials remain deployment work, and
local deterministic tests do not substitute for real-account acceptance tests.
