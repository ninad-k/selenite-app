# GTD templates

These files configure the same database and view engine used by any other workflow. There is no GTD-specific query or row type.

Install `schemas/gtd.toml` under the vault's `.selenite/schemas/`, and the six files from `views/` under `.selenite/views/`. The built-in installer creates only missing metadata files and preserves any existing definitions. Installation can be repeated after interruption; it is not an all-or-nothing transaction. A customized schema may require corresponding changes to the bundled views.

Every page with the YAML tag `gtd` is a row, wherever its Markdown file lives. The tag may be a string or a list item; matching is case-sensitive. Required `status` and `kind` fields make a row's purpose explicit. The [task](pages/task.md) and [project](pages/project.md) files are starting points to copy into ordinary vault Markdown pages; substitute `{{title}}` yourself when using them outside an application template action.

| View | Selection | Layout |
| --- | --- | --- |
| Inbox | `status = inbox` | Table |
| Next actions | `status = next` and `kind = action` | Board grouped by contexts |
| Projects | `kind = project` | Gallery using the optional attachment |
| Waiting for | `status = waiting` | Table grouped by waiting-for person |
| Someday / maybe | `status = someday` | Table |
| Weekly review | All four active statuses | Calendar using `review_date` |

Set `review_date` to a `YYYY-MM-DD` date when you want a row on the review calendar. The template does not invent a schedule or modify dates automatically. A row with several contexts appears in each corresponding board group. Empty properties remain distinct from `false` and `0`.

Properties use YAML strings, numbers, booleans and lists. Dates are ISO date strings; a relation is a page target or `[[wikilink]]`, or a list of those values; URLs use HTTP(S); files use vault-relative paths. Select values must appear in the schema's options. `$title` and `$path` are read-only query fields, not YAML properties. Filters and text matching are case-sensitive; range endpoints are inclusive; sort ties use path order and empty values sort last.

Row edits preserve the Markdown body exactly and preserve unknown frontmatter values, but YAML comments, ordering and formatting may change when frontmatter is reserialized. The complete previous source is required for compare-and-swap writes, so stale edits cannot silently overwrite another edit.
