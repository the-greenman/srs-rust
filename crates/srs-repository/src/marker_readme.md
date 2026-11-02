# About SRS

This directory marks a **Semantic Record System (SRS) repository**: structured,
typed, versioned records and the relations between them, stored as plain JSON.

**The records are the source of truth.** Rendered documents and exports are
projections of them, never the source.

## Where to look

- `manifest.json`: the repository's identity and settings
- `records/`, `relations/`, `package/`: the authoritative data and its definitions

The file tree is authoritative; there is no separate index to consult.
This file is not authoritative: it is only orientation, and `.srs/` is
implementation-private.

## Use the tools, do not hand-edit

SRS JSON has strict identity, versioning and validation rules. Writing it by
hand, or by imitating neighbouring files, is the failure mode the tools exist
to prevent. Both surfaces validate writes:

- **MCP server** (preferred): `srs mcp serve`, mounted per repository. List its tools with `tools/list`.
- **CLI** (fallback): the `srs` command. List commands with `srs --help`.

Run `srs repo validate` after changes.

## Get running

`srs` is the engine: one binary that is both the CLI and the MCP server.

- Install the srs engine: `curl -fsSL https://engine.semanticops.com/install.sh | sh`
- [First five minutes](https://engine.semanticops.com/start.md): CLI, MCP server, output contract
- [SemanticOps sites](https://semanticops.com/llms.txt): every site and what it is for

## Learn more

<https://srs.semanticops.com/llms.txt>: the specification, for agents
(canonical JSON Schemas live under `https://srs.semanticops.com/schema/2.0/`).
