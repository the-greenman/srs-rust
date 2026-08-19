# ADR-047: `repo doctor` is an explicit, never-on-load repair surface

- **Status:** proposed
- **Date:** 2026-08-19
- **Supersedes:** —
- **Superseded by:** —

## Context

`srs-usage.md` and the write-path governance note (the-greenman/srs#397) tell an agent to write
through the tools, never by hand-editing repository JSON. Agents (and humans) will not always
comply — the natural "copy this record, then edit it" act alone produces a duplicate
`instanceId`, which is `SRS038-R12-DUPLICATE-ID`, an [R24]-fatal diagnostic that bricks the whole
repository load. A hand-edited container, a manually renamed relation file, or a corpus still
carrying RFC-038's retired manifest keys land in the same place: the repository loads for no
command at all, including the commands that would otherwise fix it.

srs-rust#844 named the specific gap this closes: `container roots/members list` cannot read a
bricked repository, so even the one repair path that already existed (`container roots remove` /
`container members remove`, ADR-045) was driven blind, off `repo validate`'s diagnostics alone,
with no way to see a container's current membership before or after acting on it.

srs-rust#857 generalizes this: manual edits and raw adds **will** happen, and the core should
accommodate them by repair, never by silent tolerance. This ADR is the decided design for that
surface — `srs repo doctor`.

## Decision

### One explicit command, dry-run by default

`repo doctor` (`doctor_service::doctor`) is a single command with one flag: `--fix`. Without it,
doctor only reports — every finding names what `--fix` would do, and nothing on disk changes
(the dry-run-changes-nothing property is enforced by test: a byte-compare of the whole tree
before and after a dry run). `--fix` applies every repair the current pass can decide
deterministically.

**Never on any load path.** Every load-time surface — `store.catalog()`, `load_manifest()`, the
CLI/MCP/WASM open paths — keeps behaving exactly as it does today: [R24] fatality is unchanged,
[R21]/[R2] refusals are unchanged. `doctor_service::doctor` is reachable only from the `repo
doctor` CLI handler and its WASM/MCP twins, all triggered by an explicit user request. A reader
that silently repaired on open would be exactly the coercion [R21] forbids; a repair the operator
did not ask for and cannot see is not truthful diagnostics, it is undetected loss risk with a
happy exit code.

### Read path: reuse the existing unchecked seam, don't widen it

Detection reuses `RepositoryStore::catalog_unchecked` (`catalog::build`) — the diagnostics-carrying,
non-[R24]-fatal variant `repo validate` and the ADR-045 container repair seam already use. Doctor
adds no second unchecked seam; it is another consumer of the one that already exists, which is
exactly what srs-rust#844 asked for: the visibility gap it named is closed by doctor's report,
not by widening `container roots/members list`'s own checked-only contract. `repo doctor` also
closes #844 for that reason — the operator can now see full current membership (and every other
diagnosable class) on a bricked repository, without a second exemption on the read side of
container list.

One case `catalog::build` does not cover: a retired manifest property (RFC-038 Change K) or a
`dataModelRevision` below this build's floor makes `catalog::build`'s own internal
`store.load_manifest()` call fail. `build` already handles that by emitting a single
`MANIFEST_INVALID` diagnostic and returning an otherwise-empty catalog rather than failing the
whole call — but an empty catalog means every *other* diagnosable class goes dark until the
manifest-level fault is cleared. Doctor's manifest-level classes (retired keys,
below-floor generation) are therefore diagnosed from a **raw** read
(`load_manifest_raw_text`, never the checked `load_manifest`) independent of the catalog pass,
and the catalog's own `MANIFEST_INVALID` diagnostic is skipped in doctor's main loop as
redundant with the more specific raw-read finding.

If a future repair class genuinely cannot be read through `catalog_unchecked` or the raw-manifest
probe, that is a second unchecked seam and an owner decision — not something this surface invents
unilaterally (see srs-rust#857's stop condition on this point).

### The repair inventory

Each diagnosable class maps to exactly one of {deterministic repair, named manual step} — never
a guess, never two mechanisms for the same goal:

| Class | Repair | Mechanism reused |
|---|---|---|
| retired manifest keys (RFC-038 Change K) | deterministic, file-tree stores only | `rfc038_storage_migration_service::migrate_storage` — the same transform `repo apply-migration --id rfc038-storage` runs |
| `dataModelRevision` below this build's floor | manual | `repo apply-migration` (field-type, then rfc039-carrier) — generation migration is a different concern from raw-add/manual-edit repair |
| duplicate instance id ("adopt") | deterministic when the id has no incoming reference; **ambiguous** (manual) otherwise | fresh id via `writer::new_instance_id`, `save_instance_json` |
| dangling container membership ([R13]) | deterministic | `container_service::remove_member` / `remove_root` (ADR-045's repair seam) |
| dangling relation endpoint ([R13]) | deterministic — the relation is deleted | `store.delete_relations_json` |
| relation filename↔id mismatch ([R11]) | deterministic | `store.save_relation` (derives the correct path from the in-file `relationId`) + delete the old file |
| every other diagnosable class (other duplicate-id sets, dangling `FieldAssignment.fieldId`, malformed/unrecognised candidates, schema/shape failures, …) | manual | the diagnostic is reported verbatim; doctor never guesses content |

Every applied repair — and every planned-but-not-applied one under a dry run — is named in the
finding's `detail` field. This is the truthful-diagnostics contract: the caller sees exactly what
changed, or exactly why nothing did.

### Adopt's identity semantics

A duplicate instance id is the natural copy-then-edit act: `cp record.json record-2.json`, then
edit the copy, forgetting to change `instanceId`. Adopt mints a fresh id for every copy in the
duplicate group but one (the group's locators, sorted, keep the lexicographically-first path —
deterministic, not a guess about which file is "the real one"); content is otherwise preserved
byte-for-byte except the `instanceId` field itself.

Relations and container membership are deliberately **not** carried forward to the reminted
copies. They still name the id that was kept, which is correct by construction: nothing on disk
changes for the kept file, so every reference that already resolved to that shared id keeps
resolving exactly the same way it did before the group was ambiguous.

**The stop condition.** When the duplicate id has *any* incoming relation or container reference,
adopt does not run — the finding is `Ambiguous`, a manual step, even under `--fix`. The reason is
not "we don't know which file is canonical" in the abstract; it is concrete: today, both files
declare the *same* id, so a relation or container entry naming that id is symmetric across every
copy in the group — there is no per-file signal in the data that says which physical content a
given reference was written against. Picking a keeper anyway would silently decide, for the
operator, which content those existing references mean from now on. That is a semantic content
decision, and ADR-044's "explicit minting on request is not fabrication" licenses minting a fresh
id on request — it does not license silently reinterpreting what an *existing* reference points
to. Doctor reports the case and lets a human decide (srs-rust#857's own stop condition); it does
not resolve it.

The common case — a fresh verbatim copy, before anything else in the repository could reference
it — is exactly the issue's own reproduction, and it has no incoming references, so it is always
auto-repairable. Both PR review rounds and the test suite treat this as the target case; the
ambiguous case is deliberately narrower and rarer.

## Consequences

**Positive:**
- A repository bricked by the most common forms of raw-add/manual-edit damage — duplicate ids,
  dangling container or relation references, a renamed relation file, an unmigrated manifest —
  is repairable through one command, with a dry-run preview and truthful per-finding diagnostics.
- Closes srs-rust#844 without widening any checked-read surface: the visibility gap is closed by
  doctor's report, which is built on the same unchecked seam ADR-045 already carved.
- No parallel mechanism: every repair calls the same service function an ordinary write path
  would use (`remove_member`/`remove_root`, `migrate_storage`, `save_relation`), so the checked
  and repair paths cannot drift out of step with each other.
- The repair inventory table is the complete answer to "what does doctor do with class X" — no
  class is repaired by an undocumented side effect of another class's fix.

**Negative / trade-offs:**
- Doctor's read path depends on `catalog::build`'s "return an otherwise-empty catalog on a
  manifest error" behaviour to avoid a second unchecked seam. That means a repository with *both*
  a retired manifest key *and* (say) a dangling container reference needs two `repo doctor --fix`
  passes to clear both: the first pass sees only the manifest-level finding (from the raw probe)
  because the catalog-derived classes are dark until the manifest parses; the second pass, run
  after the first, sees the rest. This is documented behaviour, not a bug — `--fix` is idempotent
  and safe to run repeatedly, and the report always says exactly what is left.
- The "retired manifest keys" repair is file-tree-store only, inherited from
  `rfc038_storage_migration_service::migrate_storage`'s own pre-existing constraint (no store
  implements batch rollback — srs-rust#813). On `MemoryStore` this class stays a manual step.
- Diagnostic messages are parsed with plain string splitting (`duplicate_set_and_id`,
  `container_dangling_prop_and_id`, `relation_dangling_id` in `doctor_service.rs`), not a
  structured field on `CatalogDiagnostic`. The producer (`catalog.rs`) and consumer live in the
  same crate and are expected to change together; a unit test pins each parser against the exact
  message text `catalog.rs` emits today as a drift tripwire. A structured diagnostic payload
  would be the more robust long-term shape, but it touches `CatalogDiagnostic`'s serialized form
  and is deferred rather than bundled into this unit.

## Rejected alternatives

**A dedicated `repo repair` command, separate from `doctor`'s reporting.** Rejected on the same
grounds ADR-045 already used: report and repair are two views of the same detect-then-act
operation, and a second command name for the same goal is the parallel-mechanism drift the
codebase deliberately avoids. `--fix` is the one flag that flips the same command from reporting
to acting.

**A second unchecked-catalog seam, purpose-built for doctor.** Rejected: `catalog_unchecked`
already exists precisely for this ("validate-style consumers need the complete picture" —
`catalog::build`'s own doc comment). Widening the exemption set was explicitly named a
stop-condition trigger in srs-rust#857, reserved for an owner decision; it was not needed here.

**Guessing a canonical copy for every duplicate-id group, always.** Rejected: it would silently
redefine what an existing relation or container reference means whenever the duplicate id already
had incoming references, trading a loud, fixable [R24] fatal load for a quiet, wrong repair. The
stop condition (`Ambiguous`, manual) is the deliberately narrower alternative.

**Repairing every diagnosable catalog class doctor encounters.** Rejected for this unit: the
inventory is scoped to the classes srs-rust#857 names as the minimum set (adopt, dangling
membership/relation, filename mismatch, retired manifest keys). Other duplicate-id sets
(relation/container/source-document/definition/extension) and dangling
`FieldAssignment.fieldId` references are report-only — extending the inventory to cover them is
future work, not a defect in this one.
