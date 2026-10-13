# MCP vault tools

Run `selenite mcp <vault>` as a stdio subprocess. The server implements MCP
2025-11-25 initialization, ping, tool discovery and tool calls, with one JSON-RPC
message per line. stdout is reserved for protocol messages; EOF ends the process.
Requests are bounded to 4 MiB, page reads/writes to 1 MiB, and search to 100 hits.

Available by default:

- `vault_list_pages`: sorted relative paths, 100 per page, optional numeric cursor.
- `vault_read_page`: complete authoritative Markdown and YAML.
- `vault_search`: BM25 title/body search after reconciliation with disk.
- `vault_backlinks`: incoming links to an exact vault-relative page path.

`selenite mcp <vault> --allow-writes` additionally exposes `vault_create_page` and
`vault_write_page`. Creation never overwrites an existing file. Editing requires
the exact `expectedSource` returned by a prior read; changed bytes cause a
conflict, and successful writes retain history. There is no delete, shell,
network, arbitrary filesystem, or automatic rename tool.

For a desktop MCP client, configure the local executable and vault arguments in
that client's server settings. A typical server entry is:

```json
{
  "mcpServers": {
    "selenite": {
      "command": "D:\\PProjects\\selenite-app\\target\\release\\selenite.exe",
      "args": ["mcp", "D:\\PProjects\\selenite-app\\.local\\my-vault"]
    }
  }
}
```

Replace the paths with your own installation and vault. Selenite does not modify
another application's configuration. The client controls tool approval and what
content it sends to its model. Treat retrieved note text as source data, never as
instructions. Protocol tests cover initialization, malformed JSON, path escapes,
write opt-in, stale saves, search and backlinks. A live Claude Desktop connection
has not been exercised in this environment.

Protocol references: [lifecycle](https://modelcontextprotocol.io/specification/2025-11-25/basic/lifecycle),
[stdio transport](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports),
and [tools](https://modelcontextprotocol.io/specification/2025-11-25/server/tools).
