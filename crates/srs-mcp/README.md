# srs-mcp

MCP (Model Context Protocol) adapter over `srs-repository` services. Ships inside
the `srs` binary as `srs mcp serve` — a stdio MCP server exposing one SRS
repository to any MCP client (Claude Code, Cursor, Copilot, Goose, …).

Governed by [ADR-037](../../docs/adr/037-mcp-adapter-surface.md): this crate is
the sole owner of the `rmcp`/`tokio` dependencies, every handler is exactly one
service call, and no business logic lives here.

## Surface

**Resources** (read):

| URI | Content |
|---|---|
| `srs://<repositoryId>/map` | Repo map: counts, package info, relation summary (JSON) |
| `srs://<repositoryId>/navigation` | Identity record + ordered navigation sections (JSON) |
| `srs://<repositoryId>/record/{instanceId}` | One record, any tier (JSON; resource template) |
| `srs://<repositoryId>/container/<containerId>` | Container resolve-view: authored columns + ordered members (JSON) |
| `srs://<repositoryId>/view/<documentViewId>` | Rendered document view (markdown) |
| `srs://<repositoryId>/type/{typeId}` | Type authoring schema: fieldIds, required flags, aiGuidance (JSON; enumerated + template) |

The `srs://` scheme is implementation tooling, not spec — every component is an
existing SRS identifier (see ADR-037 §6).

**Tools**: `repo_validate`, `find`, `record_create`, `relation_create`,
`note_create`, `type_schema` — the validated write workflows plus discovery.
Read a type's schema (`type_schema` or the `type/{typeId}` resource) before
authoring: each property's `x-srs-field-id` is the UUID `record_create`
needs. Rejected writes return
`isError: true` with the service diagnostics; nothing is written on rejection.
`repo_validate` reports validation findings as data: `summary.errors == 0`
(equivalently, no `error` diagnostics) means the repository is consistent.
Warnings are non-blocking but should be reviewed. An empty `diagnostics` array
means the repository is completely clean, with neither errors nor warnings.
Tool descriptions live as `pub const` items in [`src/tools.rs`](src/tools.rs)
(single source; the `srs-usage.md` MCP section mirrors them).

## Client configuration

Claude Code (`.mcp.json` in your project):

```json
{
  "mcpServers": {
    "my-srs-repo": {
      "command": "srs",
      "args": ["mcp", "serve", "--repo", "/absolute/path/to/repo"]
    }
  }
}
```

One repository per server process — register one entry per repo. `--repo`
defaults to auto-detection from the working directory.

**Scope limits (first cut):** file-backed repositories only (`.srsj`/JsonStore
repos are not servable — ADR-037 §5); the CLI's `--dir` storage override is not
exposed; global `--pretty`/`--container` parse but are ignored.
