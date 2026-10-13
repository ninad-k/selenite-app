# Databases are views over Markdown

A database schema lives at `.selenite/schemas/<id>.toml`. Its rows are regular
Markdown pages selected by a recursive folder or a YAML `tags` value. A saved
view lives at `.selenite/views/<id>.toml`; it selects, sorts and groups rows without
copying them into another content store.

The schema supports text, number, date, select, multi-select, checkbox, relation,
URL and file properties. Values are validated against the schema. Missing values
remain distinct from zero or an unchecked checkbox. Relations are page links;
missing targets remain representable so a vault can be incrementally authored.

```toml
id = "reading"
name = "Reading"
[source]
kind = "folder"
path = "Reading"
[[fields]]
name = "status"
type = "select"
options = ["Queued", "Reading", "Finished"]
[[fields]]
name = "due"
type = "date"
```

A board view of the same pages:

```toml
id = "reading-board"
name = "Reading board"
database = "reading"
layout = "board"
group_by = "status"
[[sort]]
field = "$title"
descending = false
```

Layouts are `table`, `board`, `calendar` and `gallery`. Calendar views specify a
date `calendar_field`; gallery views may specify `gallery_field`. Filters compose
`all` and `any` with typed `eq`, `contains`, `range` and `is_empty` operations.
Definitions are validated before saving and use exact-source conflict checks.
Rows are always reread from disk. Equal sort values use relative paths as stable
tie-breakers; empty cells sort last.

Use Views in the desktop to install GTD templates, create/edit definitions, open
pages and edit their typed properties. `selenite views <vault>` lists definitions;
`selenite query <vault> <view-id>` returns the selected rows as JSON.
`selenite init-gtd <vault>` installs the reusable task schema and saved views for
inbox, next actions, projects, waiting-for, someday/maybe and weekly review.
Existing customizations are preserved. GTD is ordinary schema/view configuration,
not special logic in the query engine.

Property edits preserve Markdown body bytes and unknown YAML values, but YAML
comments, key ordering and scalar formatting may change during serialization.
Use the Markdown editor when exact frontmatter formatting matters.

Reviewed page renames also update schema-typed relation values. Scalar and list
relations preserve the identity of unambiguous targets, including relative
references from moved rows and basename collisions introduced by a rename.
Untyped text, unknown properties and unresolved/ambiguous relations are left
unchanged. The final YAML and Markdown edits share one core recovery journal;
the desktop rejects a changed preview before publishing the rename.
