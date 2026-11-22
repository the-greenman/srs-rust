---
name: srs
description: Work with an SRS (Semantic Record System) repository through its MCP server or the srs CLI: orient in an unfamiliar repository, find and read records cheaply, write them safely, validate, and shape a repository so people and agents can both read it. Use whenever an MCP server offers `srs://` resources or tools such as `find`, `record_create` and `repo_validate`, when a directory has a `.srs/` marker and `manifest.json`, or when asked to read, create or change typed records, relations or containers.
---

# SRS: Semantic Record System

An SRS repository holds **records** (typed, with named fields), **notes** (free text, no type), **relations** (typed links from one record or note to another) and **containers** (ordered outlines that group them). You read and write it through an MCP server or the `srs` CLI. Both run the same engine and enforce the repository's type and relation contracts.

This file is complete on its own. Tool names are the MCP tools. CLI equivalents are at the end. A project instruction file (CLAUDE.md, AGENTS.md) or a project skill wins over this one.

## Five rules

1. **Never edit the repository's files.** Use the tools. If no tool does what you need, stop and say so. A JSON file that looks like a record is not a record.
2. **Never guess an id, a URI, a type name or a relation type.** List it, then copy it. Repository ids are in the server's instructions and in every hit's `uri`. There is no `srs://default/`. Ids are full UUIDs, never a prefix. An id you have seen has only the role the response gave it.
3. **Never state a link, an order or a state from a summary.** A relation is read with `neighbours`. An order is read from a container outline. A state is the record's `lifecycleState`. A field called `status` is prose and can be out of date.
4. **Zero hits does not mean nothing is there.** Check the match mode, the type name and `diagnostics` before you conclude (see Find).
5. **Look before you create, change by succession, validate after you write.**

## Orient

In order. Stop when you know enough.

1. Read the server's instructions. They list the repository ids, mark the read-only ones, and carry the repository's own guidance when it has set any.
2. `find` with `limit: 0` and no filters. It returns no hits, the `total`, and `facets`: counts by type, by tag, notes, and the values of closed fields. This is the cheapest map.
3. `read srs://<repositoryId>/agent-index`: the identity, the types with one line each, the sections and the entry points. Read the identity record it names. That record is the repository's reading guide.
4. `container_outline` on a container for its entries with `depth` and `parentInstanceId`. The outline is the document's real order and nesting. It takes a container id, never a record id. A `full` hit lists its `containerIds`.
5. `read srs://<repositoryId>/relation-types` before you read or write any relation, and use each type's `key` exactly as written: `depends-on`, or `namespace/name` for a custom one. `map` counts the keys in use.

Three traps. `agent-index` lists sections flat, so take nesting from the outline. `navigation` and `tree` have no size bound over MCP and are cut off on a large repository, so read one section or one subtree at a time. `tree` follows `contains` relations only, so a repository arranged by containers shows every record as a root there. That means "arranged another way", not "unstructured".

## Find

- **A question typed as a sentence needs `match: "any"`.** The default requires every word, and a sentence then returns zero hits with no diagnostic. `any` ranks by relevance.
- Use `projection: "card"` (id, label, type, state, score, snippet) while you scan. Add `fields: ["name", …]` for the few values you need. Read a full record only when you will use it.
- Filters combine with AND: `typeName` with `typeNamespace`, `tag`, `lifecycleState`, `containerId`, `tier`. A type or lifecycle filter leaves out notes. Use `tier: 0` for notes.
- An unknown type or container gives zero hits and a warning in `diagnostics`. Read the warning.
- Relations are not found with `find`. Use `neighbours` (`direction: "out"` or `"in"`, optional `relationType`). It returns edges, so read the record at the other end if you need its content. A `relationType` that is not an exact key returns zero edges and no warning.
- `similar` with an `instanceId` gives more like this one. Use it before creating, to catch a duplicate.
- For everything about one record, read `srs://<repositoryId>/context/<containerId>/<instanceId>`. For a record with many edges, use `neighbours` with a `limit`.
- A hit's `uri` is the address to read. Copy it.

## Write

1. **Search first.** If a record already says it, update or succeed it. Duplicates are the most common damage in our repositories.
2. **Read the type before you fill it.** `type_schema` returns the properties keyed by field name, with guidance for each. `fieldValues` in `record_create` uses those same names.
3. **`record_update` replaces all of `fieldValues`.** Read the record, change what you mean to change, send the whole set. Leaving `tags` out keeps them. Sending `[]` clears them.
4. **Change a settled record by succession.** `record_successor` creates the new record and the `supersedes` or `refines` link. The old one stays. Do not delete to correct.
5. **Change state with a transition.** `record_allowed_transitions`, then `record_transition`. Never by editing a field.
6. **Relations are stored in the forward form only.** `supersedes` runs newer to older. `contains` runs whole to part. `depends-on` runs dependent to needed. `precedes` runs earlier to later. `derived-from` runs the new thing to its source.
7. **A container id is never a relation endpoint.** Put a record in a container with `container_member_add`, and move it with `container_member_move`. Never replace a container's whole membership to add one entry.
8. **Unsure what something is? Write a note.** `note_create` now, `note_graduate` later. It becomes a typed record linked `derived-from` the note, and the note is kept.
9. **Never send `createdBy`.** The server stamps it from the session.
10. **Finish with `repo_validate`.** The repository is consistent when `summary.errors` is 0. Read warnings. Over the CLI, check `ok` and `errors[].code`, never the exit code or the message text.

## When the tool cannot do it

Fields, types, compositions and lifecycles are authored with the CLI, not over MCP. If the CLI is not available either, do not imitate the result in a file. Record what you needed and why, as a note or in the project's findings type, and report the gap. The same goes for an error you cannot explain: quote its `code`.

A running server keeps the binary it started with. If a tool named here is missing, or a documented behaviour differs, the server is probably older than this file. Say so.

## Shape a repository so it can be read

**Give it a front door.**
- The root container's identity record is a reading guide: what the repository holds, the areas in reading order, what the states mean, where to go next.
- Set the repository's guidance (`srs repo guidance set`: a summary, navigation hints, entry points). The server hands it to every agent on connect. It is the one instruction an agent always receives.
- Keep 2 to 9 areas at the top. Each is a document that anchors its own section container. No level has more than about 15 children.

**Choose the right thing for each job.**

| You want to say | Use | Not |
|---|---|---|
| what kind of thing this is | a type | a tag |
| where it stands in its life | lifecycle state | a `status` field, a tag, or a container per state |
| it is about, supports or replaces that | a relation | a tag carrying an id, or a name in prose |
| read these together, in this order | a container | a `precedes` chain beside the container order |
| a loose label to help search | a tag | anything a reader must rely on |

**Keep it connected.**
- Every record links to something: the part of the system it is about, the source it came from, the thing it supports. A hub record for each subject, with everything about it pointing in, makes one `neighbours` call answer "what do we know about this".
- Keep the chain from raw to settled: note, then record `derived-from` the note, then the decision that settles it.
- One record for one meaning. A record can be a member of several containers. Never copy it to show it twice.

**Keep it small enough to read.**
- Short fields, each saying one thing, with a title-like field on every type. Long prose goes in a few named fields, or in a container of paragraph records.
- A container that grows without limit is a queue nobody reads. Give it parts.
- A type with no records is a guess. Add a type when a second record needs it.
- Cap any layer that is always read first. Promote into it and demote out of it. Do not append.

**Across repositories.**
- A relation cannot point into another repository. Name a foreign record in prose, with its repository and id.
- Share meaning by sharing definitions: put the common types in a package and let each repository depend on it. Do not define the same type name twice with different ids.
- One server can serve several repositories. Name the repository on each call. A write goes to that one only.

## People and agents read differently

A person reads a rendered document, front to back, and wants titles, order and few words. An agent reads by search and by link, pays for every byte, and treats whatever it is shown as true. Build for both:

- Order and titles for the person. Types, relations and states for the agent. The outline carries the first, the records carry the second, and neither replaces the other.
- A summary helps a person and tempts an agent to stop. Write summaries that say where the detail is.
- Ids are noise in a page and essential in a tool call. Keep them out of reading views and in the records.
- A person tolerates a long document. An agent's first read should be small: the guidance, the identity record, a count.

## CLI equivalents

Every command prints one JSON envelope: `{"ok", "command", "version", "payload"}`. Add `--repo <path>`. Writes read JSON on stdin.

| MCP | CLI |
|---|---|
| `find` | `srs find --text "…" --match any --projection card --limit 10` (`--limit 0` for the map) |
| `similar` | `srs find --similar <id>` |
| `read …/map`, `/agent-index`, `/navigation` | `srs repo map`, `srs repo agent-index`, `srs repo navigation --depth 0` |
| `container_outline` | `srs container members outline <containerId>` |
| `neighbours` | `srs relation neighbours <id>` |
| `read …/context/…` | `srs context record <id> --markdown` |
| `type_schema` | `srs type schema <typeId>` |
| `record_create`, `record_update` | `srs record create --type <namespace/name>`, `srs record update <id>` |
| `record_successor` | `srs record successor --id <id>` |
| `record_allowed_transitions`, `record_transition` | `srs record allowed-transitions --id <id>`, `srs record transition --id <id>` |
| `note_create`, `note_graduate` | `srs note create`, `srs note graduate --type <namespace/name> <id>` |
| `relation_create` | `srs relation create` |
| `container_member_add`, `container_member_move` | `srs container members add`, `srs container members move` |
| `repo_validate` | `srs repo validate` |
| none | `srs repo guidance get`, `srs repo guidance set` |

`srs <command> --help` is the reference for flags and stdin shapes. A repository written for an older data model answers `ok: false` with `container-migration-needed`. Apply the migrations it lists with `srs repo apply-migration --id <id>`, in order.

## More, if you can fetch it

- Structure checks with thresholds: https://engine.semanticops.com/structure.md
- CLI reference: https://engine.semanticops.com/cli/index.md
- Project memory in an SRS repository: https://skill.semanticops.com/srs-memory/SKILL.md
- The specification: https://srs.semanticops.com/llms.txt
