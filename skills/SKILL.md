# SRS — Semantic Record System
version: 1
description: Work with SRS repositories using the srs CLI

---

## Quick Start

Install the `srs` CLI (Linux x86_64):

```bash
curl -fsSL https://github.com/the-greenman/srs-rust/releases/latest/download/srs-x86_64-unknown-linux-gnu.tar.gz | tar -xz
./srs --help
```

Orient before touching anything:

```bash
srs repo agent-index --repo <path>  # one-page agent-readable summary of the repository
srs repo validate --repo <path> --pretty  # check integrity; fix all errors before writing
```

---

## Overview

SRS (Semantic Record System) is a typed, relational record store for governance, specification,
and structured knowledge. Data lives in an SRS repository: a directory with a `.srs/` marker and
a `manifest.json` instance index.

**Data model in brief:**

- **Field** — atomic semantic unit with stable UUID, `namespace/name`, `version`, and `valueType`
  (string | text | number | boolean | date | url | select | multiselect)
- **Type** — named, versioned composition of Fields
- **Record tiers:**
  - Tier 0 (Note) — free-text sections, no type binding
  - Tier 1 (TypedRecord) — named fields with values, no Type binding
  - Tier 2 (Record) — instantiated Type; `typeId` + `fieldValues[]`
- **Relation** — typed edge between two instance UUIDs
  (canonical types: `contains`, `depends-on`, `supersedes`, `refines`, `derived-from`, `evidences`, `precedes`)
- **Container** — lightweight grouping boundary; its `containerId` is distinct from instance IDs

The CLI is the only stable interface. Never read or write SRS JSON files directly.

---

## Key Workflows

### Orient on an unknown repository

```bash
srs repo agent-index --repo <path>      # llms.txt-style summary: title, counts, types, sections
srs repo map --repo <path> --pretty     # detailed stats
srs type list --repo <path> --pretty    # all installed types (namespace/name/version)
```

### Create and read records

```bash
# Create a Tier 2 record (requires a Type)
srs record create --repo <path> --type <namespace>/<type-name> --input - <<'EOF'
{ "fieldValues": [{ "fieldId": "<uuid>", "value": "..." }] }
EOF

# List records by type
srs record list --repo <path> --type <namespace>/<type-name> --pretty

# Get a single record
srs record get --repo <path> --id <instance-id> --pretty
```

### Validate

```bash
srs repo validate --repo <path> --pretty  # must be 0 errors before any commit
```

### Inspect types and fields

```bash
srs type list --repo <path> --pretty
srs type get --repo <path> --name <namespace>/<type-name> --pretty
srs field list --repo <path> --pretty
```

### Relations

```bash
srs relation create --repo <path> --input - <<'EOF'
{ "sourceId": "<uuid>", "targetId": "<uuid>", "type": "depends-on" }
EOF
srs relation list --repo <path> --pretty
```

---

## Agentic Usage

**Authoritative rules for agents** are in `srs-usage.md`, published alongside the SRS
specification at `https://github.com/the-greenman/srs`. Download it once per session if
you need the full ruleset:

```bash
curl -fsSL https://raw.githubusercontent.com/the-greenman/srs/master/srs-usage.md
```

**The single most important rule:** run `srs repo validate --repo <path>` before and after
every write operation. A repository with non-zero diagnostics is in a degraded state — do not
commit or proceed without resolving them.

### Session start protocol

```bash
srs repo agent-index --repo <path>     # fast overview; read rendered field in the JSON output
srs repo validate --repo <path>        # confirm zero errors
```

The `agent-index` command returns a `payload.rendered` field with a compact markdown document
covering repository identity, type inventory, navigation sections, and suggested entry points —
enough context for an agent to plan its next steps without reading every record.

### Discovery ladder (before any write)

```bash
srs repo map --repo <path> --pretty           # counts + entry points
srs type list --repo <path> --pretty          # available types
srs record list --repo <path> --pretty        # all instances (paginate for large repos)
```

Never create a record for a type that doesn't exist. If the type is absent, ask the user
or create the type first via `srs type create`.

---

*Full agentic rules, edge cases, and workflow patterns: `srs-usage.md` in the `srs` repository.*
