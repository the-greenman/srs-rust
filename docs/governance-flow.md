# Governance Flow — Navigation-Driven Container Resolution

## How `srs-gov` Derives Structural Navigation

`srs-gov` derives governance structure by calling `srs repo navigation` and parsing the
`navigation` payload it returns:

```json
{
  "navigation": {
    "rootContainerId": "<uuid>",
    "identity": { "instanceId": "...", "typeNamespace": "...", "typeName": "...", "displayLabel": "..." },
    "sections": [
      {
        "instanceId": "...",
        "typeNamespace": "governance",
        "typeName": "decision_log",
        "displayLabel": "Decision Log",
        "sectionContainerId": "<uuid>"
      }
    ],
    "diagnostics": []
  }
}
```

Each section in `navigation.sections` is matched against `GOVERNANCE_CONTAINERS` via
`governance::by_root_type(typeNamespace, typeName)`. This returns the `ContainerTypeDef`
that carries the CLI key, icon, and creatable child types. Sections whose type is not
in the registry are silently excluded (unknown/dormant package types).

The `sectionContainerId` from the matched section is the container ID used by
`resolve_container_id` — the authoritative path for `srs-gov decision_log list`,
`get`, and `create`.

## Gate B Exploration Flow

A typical inspection of a governance repository:

```bash
# 1. Overview: identity + precedes-ordered sections with member counts
srs-gov --repo governance.srsj

# 2. List members of the Decision Log (hides superseded/closed by default)
srs-gov decision_log list --repo governance.srsj

# 3. Show all states
srs-gov decision_log list --all --repo governance.srsj

# 4. Narrow by content or tag
srs-gov decision_log list --search "budget" --repo governance.srsj
srs-gov decision_log list --tag "ratified" --repo governance.srsj

# 5. Fetch a specific record (use IDs from step 2's "Member IDs" section)
#    If the record has source documents linked via `srs attachment link`, a
#    "Linked Attachments" section appears below the field detail showing
#    each attachment's relative path, title, document ID, and on-disk size.
srs-gov decision_log get <instance-id> --repo governance.srsj

# 6. Dry-run: see the command to create a new decision
srs-gov decision_log create decision --repo governance.srsj

# 7. Inspect the underlying srs calls
srs-gov --explain decision_log list --repo governance.srsj
```

## Why Not `containerType`?

`containerType` is a soft-deprecated hint field on SRS containers (RFC-009). It was
originally used as a discovery label for governance-specific containers, but RFC-009
introduced the UUID type chain (`typeNamespace`/`typeName` on records) as the correct
structural anchor. The `navigation` service uses the type chain — not `containerType`
— to identify section roots.

Matching on `containerType` is fragile:

- It is advisory, not canonical; any container may carry any value.
- Two containers of the same type (e.g. two `"document"` containers) cannot be
  distinguished by type alone — the old code fell back to title-matching, which
  is also fragile.
- The navigation service already resolves ordering and section containerId via the
  type chain; duplicating that logic in `srs-gov` is unnecessary.

`srs-gov` now delegates all structural resolution to `srs repo navigation` and matches
sections by `typeNamespace`/`typeName`. The `containerType` field is not read by
`main.rs` at all.

## srs-gov Call Boundary

`srs-gov` is a thin client. There are two call paths:

- **Shell-out via `run_srs()`** — used for all read operations (`cmd_top`, `cmd_list`, `cmd_get`, `resolve_container_id`). The `srs` binary's payload contract is the stable interface; no srs-repository crate dependency. `cmd_get` makes a second best-effort call to `srs attachment list` to resolve linked attachments — it degrades gracefully (with a stderr warning) if that call fails.
- **Direct library call** — used only for `cmd_repo_create`, which delegates to `srs-repository::governance_scaffold_service`. This avoids piping a complex binary payload through stdin; the scaffold is a single write-then-done operation where subprocess round-trip adds friction without benefit.

New commands should default to the shell-out path unless they require a write operation where the binary payload contract would be awkward to thread through stdin.

## TUI

The read-only terminal UI (`tui_app.rs`, `tui_data.rs`) uses `srs repo navigation` for
section resolution, matching navigation nodes by `typeNamespace`/`typeName` via
`governance::by_root_type` — the same path used by `cmd_top` and `resolve_container_id`.
The RFC-009 migration off the deprecated `containerType` hint is complete across all
srs-gov paths (#384).
