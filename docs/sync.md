# Sync: reviewed immutable remote archives

Selenite sync preserves local Markdown as the source of truth. Google Drive and
OneDrive adapters store an **append-only Selenite version archive**, not a folder
mirror intended for editing in another app. Each immutable JSON object contains
the complete original UTF-8 source, its portable vault path, its parent revisions,
and a SHA-256 revision identifier. The original `.md` files remain on disk.

The implementation never issues remote deletes or replaces existing remote
content. A filename change or a missing file is reported for manual review; it
does not erase or silently recreate the other side. Retained remote versions and
local journals consume storage. Garbage collection is not implemented.

## Configuration and explicit execution

Create `.selenite/sync.toml`. Sync is disabled by default; an existing remote
folder and an access token obtained outside Selenite are required. Tokens are
read from the named environment variable when constructing the provider. Their
values are never written to the configuration or sync journal.

```toml
enabled = false
provider = "google_drive"
endpoint = "https://www.googleapis.com/drive/v3/"
# Optional; the default is the corresponding /upload/drive/v3/ endpoint.
# upload_endpoint = "https://www.googleapis.com/upload/drive/v3/"
token_env = "SELENITE_SYNC_TOKEN"
folder_id = "existing-folder-id"
# drive_id = "shared-drive-id" # Only when using a Google shared drive.
include_metadata = false
```

For OneDrive, set `provider = "one_drive"` and
`endpoint = "https://graph.microsoft.com/v1.0/"`. Omit `upload_endpoint`.
`folder_id` is an existing folder's item ID. Optional `drive_id` selects a drive;
otherwise the Graph `/me/drive` endpoints are used. This is an archive folder,
not the local vault folder. Configure endpoint URLs only for trusted services:
the configured API receives the access token. HTTPS is required except for
loopback HTTP used in transport tests.

`SyncConfig::load(&vault)` reads the configuration.
`configured_provider(&config)` constructs the selected adapter.
`plan(&vault, provider.as_ref(), config.include_metadata)` reads the local vault,
remote versions, and previous baseline, returning a serializable `SyncPlan`.
Review its actions and issues before calling
`run(&vault, provider.as_ref(), &plan)`. Run recomputes and compares the entire
plan before effects, including source bytes, remote revisions, and baseline.
Modified IPC payloads and stale reviews fail. Planning creates only local lock
bookkeeping; it does not publish remote versions or change notes/configuration.

Only Markdown pages are included by default. `include_metadata = true` also
includes valid `.selenite/schemas/**/*.toml`, `views/**/*.toml`, and
`templates/**/*.md`. Service settings, credentials, plugins, cache, history,
sync state, and journals are excluded. Transport/source validation rejects
absolute paths, traversal, symlinks at filesystem boundaries, reserved Windows
names, noncanonical separators, malformed Markdown frontmatter, invalid TOML,
and paths that collide by case. Sources are limited to 8 MiB and encoded remote
objects/responses to 12 MiB; an archive is limited to 100,000 listed versions.

## Conflicts and recovery

`.selenite/sync/state/<provider-id>.json` stores hashes and known remote revisions.
The baseline compares local changes with remote changes. A change on only one
side is transferred. Divergent content on both sides, or multiple remote heads,
produces ordinary Markdown copies under `Conflicts/`. Local original bytes remain
unchanged. Metadata conflicts are preserved in Markdown code blocks so neither
configuration is silently activated. Conflict filenames are deterministic, and
an externally edited conflict copy is never overwritten.

Remote publication is immutable. A concurrent change after plan validation can
create another head rather than destroying either version. A fresh plan reports
these heads. To resolve a conflict, review its local bytes and all remote heads,
then call `resolve_conflict(&vault, provider.as_ref(), &plan, path, merged_source)`.
The desktop IPC contract is `sync_resolve({plan, path, source}) -> SyncReport`.
Remote source text is already present in `plan.snapshot.remote`; use the selected
action's `revisions` to identify its current heads. A resolution must target a
reviewed conflict and provide valid Markdown or TOML for that path.

Resolution validates the complete plan again, retains the local original and all
remote variants as ordinary Markdown conflict copies, journals the merge intent,
and compare-and-swap writes the merged source to its authoritative local file
before publishing. The new immutable version names **all reviewed remote heads**
as parents. A successful report lists the path in `resolved`; it does not erase
the retained conflict copies. A later concurrent head remains a separate
conflict for another review. Existing conflict copies that users have edited are
never replaced.

If a merge upload fails, the chosen merged bytes remain on disk. A pending record
keeps that path classified as a conflict, even if the chosen text equals the old
baseline; ordinary sync cannot then download over that explicit choice. Obtain
a fresh plan and resolve again. If an upload succeeded but its acknowledgement or
checkpoint was lost, the retry recognizes the immutable pending revision and
finishes without another upload. Pending records are marked complete, not
deleted. Invalid merges, changed local sources, and changed remote heads reject
the stale review before note changes or publication.

A separate OS file lock serializes cooperating sync runs. Local application uses
core compare-and-swap writes. Every explicit run retains its reviewed plan under
`.selenite/sync/journals/`, checkpoints the baseline after each successful action,
and records successful completion separately. If a process or transport fails,
request and review a **fresh** plan. Already published identical versions converge
without replacement; unchanged local/remote bytes establish a new baseline if a
checkpoint was interrupted. Missing history, changed conflict copies, malformed
remote data, and authorization errors stop or report the affected work without
deleting either side. Core history retains pre-write bytes. Local storage must
support the core's atomic rename and exclusive hard-link publication.

OS locks coordinate Selenite processes; unrelated editors and cloud clients do
not participate in those locks. Atomic per-file publication is not a distributed
transaction. Local compare-and-swap checks and immutable remote versions keep
their limitations explicit.

## Provider protocol

Google Drive lists only the configured parent, follows `nextPageToken`, rejects
`incompleteSearch`, and reads version content with `alt=media`. Private
`appProperties` identify the archive format and a hash of the logical path; the
full path is in the version object. Multipart creation uploads metadata and
content as a new file; duplicate names with identical revisions are deduplicated
on read. Use the same OAuth application across clients because private properties
belong to that application. See Google's [file listing reference](https://developers.google.com/workspace/drive/api/reference/rest/v3/files/list),
[custom property limits](https://developers.google.com/workspace/drive/api/guides/properties),
and [multipart upload guide](https://developers.google.com/workspace/drive/api/guides/manage-uploads).

OneDrive lists the configured folder, follows same-endpoint/same-folder
`@odata.nextLink` pages, and downloads version objects. A preauthenticated content
redirect receives no bearer token. New immutable names use an upload session with
`@microsoft.graph.conflictBehavior = "fail"`; an existing object is accepted only
after verifying identical version bytes. Upload URLs also receive no bearer
token. See Microsoft's [folder listing](https://learn.microsoft.com/en-us/graph/api/driveitem-list-children?view=graph-rest-1.0),
[content download](https://learn.microsoft.com/en-us/graph/api/driveitem-get-content?view=graph-rest-1.0),
and [upload session](https://learn.microsoft.com/en-us/graph/api/driveitem-createuploadsession?view=graph-rest-1.0)
references.

These adapters do not perform OAuth login or refresh expired tokens. Configured
tokens must already have access to the archive folder. OAuth scopes and
application registration remain deployment responsibilities. No real Google or
Microsoft account was contacted during development: tests use an in-memory
transport and local HTTP mock servers. Live-account integration remains
unverified.

## Verification

Run `cargo test -p selenite-sync` and
`cargo clippy -p selenite-sync --all-targets -- -D warnings` from the repository,
with `CARGO_HOME`, `TEMP`, and `TMP` pointing inside `.local` as in the project
verification instructions. Tests cover three-way transfers, simultaneous edits,
create races, conflict copies, partial-run checkpoints, stale/tampered plans,
metadata exclusions, malicious paths, provider pagination, token-safe download
redirects, fail-on-conflict uploads, authorization failures, and missing remotes.
Resolution tests also cover multiple-head convergence on a second device,
invalid and stale merges, metadata validation, upload failure, lost
acknowledgements, and a new remote head arriving during publication.
