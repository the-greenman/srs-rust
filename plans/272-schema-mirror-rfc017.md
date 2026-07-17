# Plan: Schema mirror sync from RFC-017 (srs-rust#272)

## Summary

RFC-017 Rev 3 (accepted, srs#101) adds `"attaches"` to the `SourceReference.sourceRole` enum in all four SourceReference-bearing schemas (`record.json`, `note.json`, `typed-record.json`, `relations-collection.json`) and clarifies that `contentPath` in `source-document-meta.json` permits forward-slash subdirectory segments. The canonical spec JSON schemas in `srs/docs/schema/2.0/` need these edits first, then `sync-schemas-from-spec.sh` mirrors them into `crates/srs-schema/schemas/2.0/` and regenerates `SHA256SUMS`. This is a pure mirror sync — no Rust code changes, no payload struct changes, no CLI surface changes.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Verification | Verification Agent |

No worker agents needed — single-phase schema edit with no code changes.

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new architectural decisions — schema sync is governed by the existing mirror model in `srs-rust/CLAUDE.md` (Schema Sync section) and the constraint that `crates/srs-schema/schemas/2.0/` is a mirror of `srs/docs/schema/2.0/`, never edited directly.

---

## Contracts

### CLI output contract (ADR-011)

No CLI commands added or changed. No payload structs modified. No schema regeneration via `generate-schemas` needed.

### Entity schema sync (check-schema-sync.sh)

Yes — 5 schema files in `srs/docs/schema/2.0/` are modified. After edits, `sync-schemas-from-spec.sh` must be run and `bash scripts/check-schema-sync.sh` must exit 0.

---

## Scope

- Edit `srs/docs/schema/2.0/record.json`: add `"attaches"` to `SourceReference.sourceRole` enum
- Edit `srs/docs/schema/2.0/note.json`: same
- Edit `srs/docs/schema/2.0/typed-record.json`: same
- Edit `srs/docs/schema/2.0/relations-collection.json`: same
- Edit `srs/docs/schema/2.0/source-document-meta.json`: extend `contentPath` description to state forward-slash sub-path segments are permitted
- Run `scripts/sync-schemas-from-spec.sh` (from srs-rust/) to copy all five updated schema files into `crates/srs-schema/schemas/2.0/` and regenerate `SHA256SUMS`
- Verify with `bash scripts/check-schema-sync.sh`

**Out of scope:**
- Any Rust code changes (validator, parser, model types) — those come in subsequent Gate A issues
- srs-vscode schema sync — that repo syncs itself from the srs release artifact in its own pipeline
- Changes to the deprecated legacy `relationType` alias — RFC-017 Rev 3 explicitly says it does not gain `"attaches"`
- RFC record authoring / spec re-render — the RFC document is already committed in srs

---

## Phases

### Phase 1: Edit canonical spec schemas + sync mirror

**Goal:** All five `srs/docs/schema/2.0/` files are edited, `crates/srs-schema/schemas/2.0/` is synced, SHA256SUMS is regenerated, and `check-schema-sync.sh` exits 0.

**Agent:** Lead Integrator (direct edit — no worker delegation needed for 5-line schema edits)

#### Tasks

- [ ] In `srs/docs/schema/2.0/record.json`: find `$defs.SourceReference.properties.sourceRole.enum` (currently `["evidence", "extracted-from", "quoted-from", "inspired-by"]`) and append `"attaches"`
- [ ] In `srs/docs/schema/2.0/note.json`: same edit — `$defs.SourceReference.properties.sourceRole.enum` → append `"attaches"`
- [ ] In `srs/docs/schema/2.0/typed-record.json`: same edit
- [ ] In `srs/docs/schema/2.0/relations-collection.json`: same edit
- [ ] In `srs/docs/schema/2.0/source-document-meta.json`: update `properties.contentPath.description` to state that forward-slash sub-path segments are permitted and resolved relative to `sourceDocumentsPath`
- [ ] Run `bash /home/user/srs-rust/scripts/sync-schemas-from-spec.sh` (from the srs-rust workspace root; uses local `../srs` checkout)
- [ ] Run `bash /home/user/srs-rust/scripts/check-schema-sync.sh` — must exit 0 (srs-vscode drift warning is acceptable; only srs-rust parity is required for this PR)

#### Acceptance Criteria

- [ ] `$defs.SourceReference.properties.sourceRole.enum` in all four SourceReference-bearing schemas contains exactly `["evidence", "extracted-from", "quoted-from", "inspired-by", "attaches"]`
- [ ] `source-document-meta.json` `contentPath` description explicitly mentions forward-slash sub-path segments
- [ ] `crates/srs-schema/schemas/2.0/` files match `srs/docs/schema/2.0/` exactly (diff is empty for each of the 5 changed files)
- [ ] `SHA256SUMS` is regenerated and consistent with the updated files
- [ ] `bash scripts/check-schema-sync.sh` exits 0 for the srs-rust portion (srs-vscode divergence is a separate pipeline concern)

#### Testing

```bash
# Verify enum values in all four files
python3 -c "
import json
for f in ['record.json','note.json','typed-record.json','relations-collection.json']:
    d = json.load(open(f'crates/srs-schema/schemas/2.0/{f}'))
    e = d['\$defs']['SourceReference']['properties']['sourceRole']['enum']
    assert 'attaches' in e, f'{f}: missing attaches'
    print(f'{f}: OK — {e}')
"

# Verify contentPath description
python3 -c "
import json
d = json.load(open('crates/srs-schema/schemas/2.0/source-document-meta.json'))
desc = d['properties']['contentPath']['description']
assert 'sub-path' in desc or 'subdirectory' in desc or 'forward-slash' in desc, 'description not updated'
print('contentPath description OK:', desc)
"

# Sync check
bash scripts/check-schema-sync.sh
```

No Rust test changes needed — this is a JSON file edit only.

#### Milestone gate

1. Verify all acceptance criteria above are met — check each checkbox.
2. Confirm the `python3` verification commands above pass.
3. Run sync check:

```bash
bash scripts/check-schema-sync.sh
```

4. Update the plan file: mark completed task checkboxes `[x]` and acceptance criteria `[x]`.
5. Commit srs-rust changes:

```bash
git commit -m "chore(schema): sync schemas from RFC-017 — add attaches to sourceRole (#272)"
```

---

## Final Acceptance

All of the following must be true before this plan is closed:

- [ ] `cargo test` passes with no failures (schema files are embedded at compile time; compile failure = broken schema)
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes (no payload structs changed, but run to confirm no compile breakage)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 for srs-rust (srs-vscode parity is a separate pipeline)
- [ ] All four SourceReference-bearing schemas contain `"attaches"` in `sourceRole.enum`
- [ ] `source-document-meta.json` `contentPath` description reflects subdirectory support

## Coordination Rules

- Lead Integrator edits both `srs/docs/schema/2.0/` (canonical) and `crates/srs-schema/schemas/2.0/` (mirror via sync script).
- Do not edit `crates/srs-schema/schemas/2.0/` files directly — use `sync-schemas-from-spec.sh` only.
- srs-vscode drift is not in scope — do not reach into `../srs-vscode`.

## Assumptions

- The local `srs` checkout at `../srs` (relative to srs-rust) is on the RFC-017 schema branch when `sync-schemas-from-spec.sh` runs, so the script picks up the updated canonical files.
- srs PR (for canonical schema changes, closing srs#101) is opened simultaneously with this srs-rust PR; the srs PR merges AFTER this one.
