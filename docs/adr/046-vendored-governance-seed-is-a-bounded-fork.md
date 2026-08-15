# ADR-046: The vendored governance seed is a bounded fork, not a byte-copy

- **Status:** accepted
- **Date:** 2026-08-15
- **Supersedes:** —
- **Superseded by:** —

## Context

`crates/srs-gov/assets/governance-seed.srsj` is the seed `srs-gov` installs when it creates a
governance repository. Its stated provenance is a byte-copy of the published
`com.mudemocracy.governance` package's seed artifact
(`srs/packages/com.mudemocracy.governance/<version>/seed/empty-governance-document.srsj`), produced
upstream by `scripts/build-governance-seed.mjs` and copied here unchanged.

It has not been a byte-copy for some time, and the record of that was scattered. The doc comment on
`GOVERNANCE_SEED` (`crates/srs-gov/src/main.rs`) says so and attributes it to the RFC-032/039
migrations carried through in srs-rust#783 Phase 4, naming the upstream reseed (Governance Epic 15,
muDemocracy.org#136) as the convergence point. srs-rust#826 describes a different divergence set and
a different convergence point. Neither is complete, one is now stale — the published 1.0.0 and 1.1.0
seeds are themselves on `srsj: "2"`, `dataModelRevision: 2` and the `fieldType` carrier, so the
migration half of the divergence has already been resolved upstream — and both cite "ADR-017" for
the byte-copy contract, which ADR-017 does not carry: it is *Deterministic `.srsj` serialization via
`BTreeMap`*. The contract lived only in the plans that followed it.

### What actually diverges

Baseline: `packages/com.mudemocracy.governance/**1.0.0**/seed/empty-governance-document.srsj`, the
artifact the vendoring recipe names. Of the 39 entries the two seeds share, 26 differ, and every
differing property falls in one of these rows — the inventory is complete, not illustrative:

| # | Divergence | Vendored | Published 1.0.0 |
|---|---|---|---|
| 1a | document-view section sources (3 views: `articles-and-roles`, `decision-deliberation`, `governance-document`) | `container-subset` (`containerId`) | `type-query` + `semanticObjectType` |
| 1b | `decision-log`'s section source | `type-query`, `containerScope: explicit` + `containerIds` | `type-query`, `containerScope: repository` |
| 2 | `aiGuidance.purpose` on 8 Fields | authored text, verbatim from each field's `description` | `""` |
| 3 | definition `$schema` | present on all 39 shared entries | present on 18 |
| 4 | ~~`package/fields/externallinks-fc434475.json` `fieldType`~~ — **REPAIRED**, srs-rust#850 | ~~`{datatype: string, format: uri}`~~ → now `{cardinality: "list", datatype: string, format: uri}` | `{cardinality: "list", …}` |
| 5 | `package/package.json` index + identity: `title` renamed (`governance` vs `MuDemocracy Governance`), `description` emptied, `themes`/`vocabularies` keys added, `fields` 22 vs 20, `types` +1, `lifecycles`/`protocols` entries | — | — |
| 6 | `package/protocols/decision-7a088176.json` | absent | present |
| 7 | lifecycle entry key | `governancelifecycle-3c504040.json` | `governance-lifecycle-3c504040.json` |
| 8 | `manifest.meta.upstreamPackage` | `{}` | populated |

sha256: vendored `1c5ebb4f…` (was `28a41897…` as first measured, before row 4 was repaired — see
below), published `24817893…`. Top-level `manifest.upstreamPackage` is present and equal in both —
only the `meta` copy was emptied. To regenerate this inventory, diff the two files' `data` maps
entry by entry; nothing else in the tree derives it.

Rows 1a–3 are the fork proper (below). Rows 4–8 are **unintended** and converge by being fixed, not
preserved — row 4 in particular was data loss: the vendored copy dropped `cardinality: "list"`, so
`externallinks` was single-valued in what `srs-gov` installs and repeatable in what the package
publishes.

**Row 4 is now repaired** (#850). `cardinality: "list"` was restored on the vendored entry — and on
`crates/srs-repository/tests/fixtures/governance-seed.srsj`, which is byte-identical to it by
construction — bringing the vendored `fieldType` to exactly what 1.0.0 and 1.1.0 both publish. Both
published versions carry the correct cardinality in the seed *and* in
`package/fields/externallinks-fc434475.json`, so the loss happened at vendor time and there is
nothing to fix upstream. Repairing data loss is inside this fork's contract by decision 1 below:
row 4 was already classified as unintended, and restoring it narrows the fork rather than widening
it. A re-audit at the time of the repair found it to be the *only* remaining `fieldType`
divergence across every shared entry, and no Type carries an assignment-level
`repeatable`/`minItems`/`maxItems` divergence either — the vendored and published seeds now agree
on cardinality entirely. Do not "converge" this row by copying the vendored value upstream.

1.1.0 is also published and matches 1.0.0 on all eight rows, so the choice of baseline does not
change the fork's shape — but it additionally revises authored text (`rationale`'s
`aiGuidance.purpose`, `alternativesconsidered`'s examples), which the vendored copy predates.

Rows 1a, 1b and 2 are repairs the mechanical transform could not make, not staleness:

- **`aiGuidance.purpose`.** RFC-038 [R7] requires it non-empty. The published seed's are empty
  strings, so a byte-copy of it does not load.
- **The document-view sources.** `rebind_document_views_to_scaffold` keeps a `TypeQuery` section
  whose `container_ids` is `None` (the `_ => true` arm) and drops a `ContainerSubset` section whose
  container does not exist in a fresh install. So the published `type-query` shape leaves a view in
  place with nothing bindable behind it, while the vendored `container-subset` shape lets it be
  removed — which is the release-1 behaviour
  `governance_scaffold_service::scaffold_rebinds_document_views_to_created_containers` and
  `srs-gov`'s `repo_create_document_views_bind_to_scaffolded_containers` are written against. Both
  fail on the published seed; that is why srs-rust#826's re-vendor attempt was reverted.
- **`decision-log` (row 1b).** It stays `type-query` on both sides, so it is easy to read as
  untouched — it is not. `rebind_document_views_to_scaffold` only rewrites a `TypeQuery` section
  that carries `Some(container_ids)`; the published `containerScope: repository` form has none, so
  it is passed through unbound — the view survives scaffolding pointing at nothing in particular,
  and `scaffold_rebinds_document_views_to_created_containers` fails on its rebind assertion. A
  re-vendor that repaired rows 1a and 2 alone would still fail there.

The document-view row sits on the open `semanticObjectType` question, ruled upstream in
the-greenman/srs#383: the collapse **will** execute, sequenced inside epic srs#256's remaining spine
(#272/#273), and `semanticObjectType` is sanctioned-until-collapsed in the interim. Regenerating and
republishing the governance package now would mint a version the collapse churns again inside the
same epic. The interim invariant srs#383 does impose is **no silent divergence** — and while the
divergence was not silent, it was recorded in three places that disagree.

## Decision

The vendored seed is a **deliberate, bounded fork** of the published package, not a byte-copy, for
as long as the published package fails [R7] and the `semanticObjectType` collapse (srs#383) is
pending. This ADR is the single record of that; `GOVERNANCE_SEED`'s doc comment now points here
rather than carrying its own account.

1. **The fork is bounded by the inventory above**, which is exhaustive as measured. A change to the
   vendored seed that is not on it, or that widens a row on it, is out of contract: fix it upstream
   in the published package and re-vendor, do not edit the asset. A divergence found later that the
   inventory does not cover is a defect in this ADR — add the row and say which kind it is, rather
   than treating the omission as permission.
2. **The convergence point is a republished package that needs no repair** — all four document
   views on sources `srs-rust` binds (after srs#383's collapse) and `aiGuidance.purpose` populated per
   [R7], which is Governance Epic 15's remit (muDemocracy.org#136). At that point the seed is
   re-vendored **byte-for-byte**, this fork ends, and this ADR is superseded. Rows 4–8 converge by
   being repaired upstream or dropped, not by being carried forward.
3. **`cp` alone is not a re-vendor while the fork stands.** Copying the published seed over the
   vendored one reverts the two repairs and breaks the two scaffold tests. Until convergence, any
   re-vendor repeats them, and `build-governance-seed.mjs --check` (which proved the upstream seed
   rebuilds byte-for-byte, muDemocracy.org#38) will not match the vendored copy.

The seed bytes were unchanged by this ADR as first accepted. They have since changed exactly once,
for the row-4 repair recorded above (#850) — a one-line restoration of `cardinality: "list"` that
narrows the fork. Rows 1a–3 and 5–8 are untouched.

## Consequences

**Positive:**
- One record instead of three that disagree, and it is discoverable from the tree rather than from
  an issue thread — which is what srs#383's "no silent divergence" invariant asks for.
- The two repairs are attributable. Anyone diffing the seed against the published package finds the
  reason instead of reading it as drift and "fixing" it.
- The vendoring contract is stated for the first time in a document that is actually about
  vendoring, rather than cited to an ADR about `BTreeMap` ordering.

**Negative / trade-offs:**
- The vendored seed and the published package stay out of sync until Epic 15 republishes, so a
  consumer reading the published package does not see what `srs-gov` installs. Accepted: the
  alternative is republishing a package version the same epic then churns again.
- The bound is enforced by review against the table, not by a check. A drift check would have to
  encode the exceptions, which is only worth building if the fork outlives the collapse.

**Neutral:**
- ADR-017 is untouched. It never carried this contract; the citations of it in srs-rust#826,
  `GOVERNANCE_SEED`'s doc comment and `plans/306-migrate-gov-repo-create.md` were mis-references,
  and the plans are historical records left as written.
- The fork is only the runtime asset. `crates/srs-repository/tests/fixtures/governance-seed.srsj` is
  byte-identical to it and tracks it for the same reasons — now asserted by
  `scaffold::fixture_seed_is_byte_identical_to_the_shipped_asset`, since the scaffold tests that
  evidence the repairs run off the fixture and would have stayed green through fixture drift.
