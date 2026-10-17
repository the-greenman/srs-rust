---
name: srs
description: Work with an SRS (Semantic Record System) repository through the srs CLI or MCP server: orient in an unfamiliar repository, find and read records, write them safely, validate, and keep project memory in one. Use whenever a directory has a `.srs/` marker and `manifest.json`, when asked to read, create or change typed records, relations or containers, or when asked to record decisions and findings so the next agent starts informed.
---

# SRS: Semantic Record System

An SRS repository is a directory with a `.srs/` marker and a `manifest.json`. It holds typed records, the relations between them, and containers that group and order them. Records are the source of truth; any markdown rendered from them is a projection. The `srs` CLI (or the same operations over MCP) is the only supported way to read and write it. Never edit its JSON files by hand.

Install the CLI (Linux x86_64): `curl -fsSL https://github.com/the-greenman/srs-rust/releases/latest/download/srs-x86_64-unknown-linux-gnu.tar.gz | tar -xz`. `srs --version` prints the same string for every build, so check a capability instead: `srs find --help` lists `--match` on a current build.

If the project has its own instruction file (CLAUDE.md, AGENTS.md) or `srs-usage.md`, it wins over this skill.

## Data model in brief

- **Field**: an atomic, immutable unit of meaning with a stable UUID, a snake_case `name` and a `fieldType`.
- **Type**: a named, versioned composition of Fields. Records of a Type carry `fieldValues`, an object keyed by `Field.name`.
- **Record** (Tier 2): an instance of a Type. **Note** (Tier 0): free text with no Type. There is no Tier 1.
- **Relation**: a typed edge between two instance ids (`relationType`, `sourceInstanceId`, `targetInstanceId`). Common types: `contains`, `depends-on`, `supersedes`, `refines`, `derived-from`, `evidences`, `precedes`.
- **Container**: a grouping boundary with an ordered list of member entries. Its `containerId` is not an instance id and never appears in a relation.
- **createdBy**: the session actor, stamped at creation. It is testimony, never authority.

Every command answers with a JSON envelope: check `ok`, read `payload`, and read `diagnostics`/`errors` when `ok` is false.

## 1. Orient (a few calls)

```bash
srs repo agent-index --repo <path>     # one page: identity, counts, types, sections (markdown at payload.rendered)
srs repo validate --repo <path>        # baseline: judge by payload.summary.errors == 0, not by ok
srs type list --repo <path> --pretty   # installed types
srs repo map --repo <path> --pretty    # counts and entry points
```

Then scope to the task:

```bash
srs find --repo <path> --text "<the question as you would ask it>" --match any --tier 2 --limit 10
srs find --repo <path> --type-name <name> --limit 20        # all records of a type
srs find --repo <path> --similar <instanceId>               # more like this
srs record get --repo <path> <instanceId>
srs relation neighbours --repo <path> <instanceId>          # edges around one record
srs context record --repo <path> <instanceId> --markdown    # a record plus everything related, compact
```

`--match any` ranks on any significant word, so a whole question works; without it every word must match. `find` is lexical, not semantic. `--tier 2` leaves out guide notes. For a type you have not used, read its schema first (`srs type get <typeId>`, or the MCP `type_schema` tool): properties are keyed by `Field.name` and carry `aiGuidance`.

The same operations are available over MCP with `srs mcp serve --repo <path>` (add `--profile context` for a smaller tool set, `--profile read` for none that write). Warm MCP reads are much cheaper than repeated CLI calls, since each CLI call loads the repository again.

## 2. Write rules

1. **Validate before and after every batch.** A repository with errors is degraded; do not write to it or commit it until the errors are explained.
2. **Look before you create.** Run `find --text <keywords> --match any --type-name <type>` and `find --similar`. Update or relate to a near duplicate instead of adding another.
3. **Never create a record for a Type that does not exist.** Check `type list`. Ask, or create the Type first, rather than inventing one.
4. **Set the actor.** `export SRS_ACTOR='{"kind":"ai","id":"agent:<role>","name":"<name>"}'` with an id that is stable per role, so `createdBy` says who wrote what.
5. **`record update` replaces the whole record.** `record get` first, change one thing, send every field back.
6. **Change by succession, never by erasure.** `srs record successor --id <uuid>` makes a new version; `srs record transition` moves a record through its lifecycle (only Types that define one). Do not delete what others may cite.
7. **Add to a container with `srs container members add`.** Never `container update` to add one member: it replaces the whole membership.
8. **If the tools cannot express what you need, that is a finding.** File an issue against srs-rust and stop. Do not hand-edit the files to get around it, because others will copy the workaround.

Verified payloads (JSON on stdin):

```bash
echo '{"fieldValues":{"comment_text":"..."}}' \
  | srs record create --repo <path> --type <namespace>/<type-name>

echo '{"relationType":"derived-from","sourceInstanceId":"<uuid>","targetInstanceId":"<uuid>"}' \
  | srs relation create --repo <path>

echo '{"byTransition":"resolve"}' | srs record transition --repo <path> --id <uuid>

srs container members add --repo <path> <containerId> <instanceId> --depth 1
```

Pass `--container <containerId>` to `record create` or `relation create` to place the result in a container as it is created. Rendered output (`srs render composition`) is regenerated from the records, never edited.

## 3. Project memory in an SRS repository

An SRS repository also makes good long-lived memory for a codebase: decisions, conventions, components and traps that a newcomer cannot cheaply rediscover from the code. Instruction files hold the rules you must always follow; the memory holds what you should know before you touch something. If they disagree, the instruction file wins: fix the record and say so.

**Before changing a component:** `find` for it, then read its context (`context record <id> --markdown`) so every decision, convention and finding that constrains it comes back at once. Records are pointers and summaries; follow `code_location`, `canonical_ref` and `source_ref` to the detail.

**Before finishing, write only what the next agent could not cheaply rediscover:**

| You... | Write |
|---|---|
| made or found a design decision that constrains future work | a `decision`, related to the component(s) it applies to, with `canonical_ref` if a document exists |
| hit a trap, a missing capability, or left a question open | a `finding` whose impact ends with what to do instead |
| learned a working rule no single file shows | a `convention` with its enforcement and source |
| changed what a component owns or where it starts | update the `component` and its `last_verified` |
| are unsure whether it will last | a note in a journal container |

Never store anything derivable from code or git, task progress, secrets, or copies of instruction files (point to them). Cite a record that changed what you did in your PR body, and fix any that proved wrong.

## 4. Commit hygiene

Validate, then commit the records with the change that motivated them. In a separate memory repository, use a branch named after the code branch and open its pull request beside the code's.
