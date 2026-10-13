# ADR-001: Files are the source of truth

Status: Accepted for Phase 0

Date: 2026-10-13

## Context

Selenite is a local-first desktop app whose diary, second-brain, workspace and task-management faces share one engine. A vault must remain useful without Selenite, and losing an index must never mean losing a note. Database rows, search results and backlinks must be reproducible from files that other Markdown tools can open.

## Decision

Each page is a UTF-8 Markdown file in the vault. Its filename supplies its title; YAML frontmatter contains its properties. Markdown nodes, optional block IDs and wikilinks remain in the source text. Reading, parsing and indexing do not rewrite that text.

The vault contains two distinct kinds of application files:

| Location | Role | Rebuildable from note content? |
| --- | --- | --- |
| `*.md`, including nested folders | Authoritative page content and YAML properties | No: these files are the source |
| `.selenite/templates/daily.md` | Optional user-authored daily-note template | No |
| `.selenite/schemas/<db>.toml` | Reserved authoritative database schema definitions | No; schema functionality is a later phase |
| `.selenite/views/<view>.toml` | Reserved authoritative saved-view definitions | No; view functionality is a later phase |
| `.selenite/cache/` | Disposable SQLite and Tantivy indexes and cache identity | Yes |
| `.selenite/index.lock` | Coordination file for cache access | Contains no user content |

Only the cache is disposable. The whole `.selenite` directory must not be treated as a cache: templates and future schema/view definitions need the same preservation as notes. SQLite stores derived pages, links and properties, including a copy of source text for change detection. That copy is never the input to a full rebuild or the sole copy of content. Tantivy indexes page title and body.

### Rebuild and recovery

A full rebuild enumerates vault Markdown files and reads and validates all their source before changing the previous cache. Invalid input aborts that attempt. The indexer then persists `ready = 0`, recreates the SQLite tables within a transaction, replaces the Tantivy documents, and commits SQLite with `ready = 1` only after the search update succeeds.

SQLite and Tantivy do not share a transaction. The durable readiness flag prevents a failed or interrupted update from presenting the two stores as a complete snapshot. Queries reject an incomplete cache; a subsequent rebuild derives its replacement from disk.

An exclusive filesystem lock serializes cache writers and recovery; cache readers hold a shared lock. The lock sits outside the replaceable cache directory. A cache identity rejects handles whose underlying cache has been replaced. Recovery retains recognized corrupt or incompatible caches in a sibling `.selenite/cache-retired-<timestamp>` directory before creating a fresh cache. Permission and other unrelated I/O failures remain errors. Retired caches are diagnostic copies, not authoritative content or rebuild input.

Vault access uses relative paths, checks containment, and rejects symbolic-link page paths. Cache preparation also checks its directory and file paths. Rebuilds never create, rename or change a Markdown page.

### Incremental indexing and links

Filesystem notifications are hints to reconcile the vault, not a history of authoritative edits. The watcher registers observation before its initial refresh, debounces events and runs one indexing worker. A refresh enumerates pages and compares their complete source against the previous cache; only changed pages are parsed and written to the indexes. Missing pages are removed from the cache. Link resolution is revisited when the set of paths changes, including when duplicate basenames become ambiguous or cease to be ambiguous.

Refresh uses the same readiness and locking protocol as rebuild. Malformed changed source aborts before cache mutation. Native watcher errors remain visible, and an explicit flush reconciles disk even without a delivered event. This is eventual reconciliation with external file edits, not an atomic filesystem-wide snapshot; it still reads unchanged files to establish whether they changed.

Links and backlinks are derived from Markdown. Ambiguous or missing link targets are not guessed. Rename support in the core produces a pure plan of source edits and the proposed path change; it does not perform filesystem writes. Its edits preserve unrelated source bytes and link fragments and aliases. Applying a multi-file rename is outside Phase 0.

### Architectural boundary

`selenite-core` owns the vault and Markdown primitives and has no UI, networking or dependency on another workspace crate. `selenite-index` builds caches from those primitives. The desktop shell and CLI use the engine; diary-specific behavior stays in the desktop layer.

The desktop shell opens or exclusively creates `Journal/YYYY-MM-DD.md` using the local calendar date and a Markdown template. Existing note bytes are preserved. Its preview is read-only. Database schemas and queries, editing, sync, plugins, AI and MCP are not implemented by this decision. The database, sync, plugin-host, AI and MCP crates are foundations or stubs for later phases, not claims that those features work.

## Consequences

- Vault content remains plain Markdown and YAML and can be edited by other tools without an export step.
- Backups must include notes and authoritative application metadata; indexes can be omitted and rebuilt.
- A full rebuild reads and parses the whole vault, and an incremental refresh still reads all page source. This favors correctness over premature timestamp-based shortcuts.
- Cache interruption fails visibly instead of silently returning a mixed SQLite/Tantivy state. Retained damaged caches consume disk space until deliberately removed.
- Future features must continue to persist their authoritative content or definitions in the vault. An index-only content store would require changing this decision.

## Verification

The core tests cover source-preserving parsing and rename plans. Index tests cover disk-only rebuilds, malformed-source rollback, missing and corrupt cache recovery, cache replacement, incremental changes, link ambiguity and watcher reconciliation. Desktop vault tests cover template creation, existing-note preservation and path validation. Performance and native-launch measurements are reported separately in the [README](../../README.md); this ADR makes no runtime performance claim.

Dependency choices and approved license exceptions are recorded in [Dependencies](../dependencies.md).
