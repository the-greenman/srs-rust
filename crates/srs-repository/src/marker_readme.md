# About SRS

This directory marks a **Semantic Record System (SRS) repository**: structured,
typed, versioned records and the relations between them, stored as plain JSON.

**The records are the source of truth.** Rendered documents and exports are
projections of them, never the source.

## Where to look

- `manifest.json`: the repository's identity and settings
- `records/`, `relations/`, `package/`: the authoritative data and its definitions

The file tree is authoritative; there is no separate index to consult.
Nothing inside `.srs/` is authoritative. It is implementation-private and
this file is only orientation.

## Use the tools, do not hand-edit

SRS JSON has strict identity, versioning and validation rules. Writing it by
hand, or by imitating neighbouring files, is the failure mode the tools exist
to prevent. Both surfaces validate writes:

- **MCP server** (preferred): `srs mcp serve`, mounted per repository. List its tools with `tools/list`.
- **CLI** (fallback): the `srs` command. List commands with `srs --help`.

Run `srs repo validate` after changes.

## Learn more

<https://srs.semanticops.com> (canonical JSON Schemas live under `/schema/2.0/`).
