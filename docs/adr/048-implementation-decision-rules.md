# ADR-048: Implementation decision rules

- **Status:** accepted
- **Date:** 2026-08-29
- **Supersedes:** —
- **Superseded by:** —

## Context

The spec repo (`srs`) has a Charter Check (the-greenman/srs#463, PR the-greenman/srs#499):
every RFC and decision names its cell, its governing preferences, and its decision mode against
`docs/charter/decision-compass.md` before drafting starts — machine-enforced for presence, never
for prose quality. That compass is the spec's own charter, ratified by decision `cce3c00e` (the
Pattern Grid and cell/axis preferences), `9ee14517` (the layer rules — three planes: Meaning,
Expression, Operation), and `7caca3a1` (decision modes: clear/complicated → rules, complex → map
consequences, chaotic → out of scope).

The owner ruling that opened this issue (the-greenman/srs-rust#875, 2026-08-26, verbatim):
*"we now have a charter for the rfc decisions. when this work comes back, lets apply an
equivalent context appropriate set of decision rules."* Per the compass's own layer rule 1
("one home" — a concern expressed in two layers is drift), this ADR does not restate the
compass — it names the implementation layer's own preference profile, consuming the compass by
link, and records where enforcement stands today.

The-greenman/srs-rust#308 and #311 are the standing lesson this ADR is built against: a
principle ruled and recorded with no mechanism that checks it decays into archaeology the next
time someone needs it. Each rule below lands with its enforcement named, even where that
enforcement is "not built yet, tracked at #N".

## Decision

### The five rules (verbatim from the-greenman/srs-rust#875)

1. **Spec-first**: name the governing spec decision, RFC, or invariant this implements. An
   implementation change that would *make* a spec ruling routes to `srs` as an RFC/decision
   instead (the ADR-004 trap generalized: the impl follows the spec, never leads it).
2. **Layer test** (Operation plane, from `docs/architecture/capability-layering.md` + spec
   decision `9ee14517`): which layer owns this — core service (once), CLI/WASM adapter (exposure
   only), client (presentation only, never semantics)? A capability landing in two layers, or
   semantics landing in an adapter/client, fails the check.
3. **One way per goal**: does this add a second mechanism for a goal that has one? Name the
   existing mechanism; collapse or supersede. Declared twins (Node emitter SSOT ↔ Rust
   byte-parity twin) are the sanctioned exception form: a twin exists only WITH its parity gate.
4. **Parity and mirror obligations named**: schema mirrors (read-only, release-asset-synced),
   payload contract sync (srs-vscode), pin choreography — each named per change, filed as
   tracking issues at landing.
5. **Decision mode** (per spec decision `7caca3a1`): clear/complicated → these rules; complex →
   map consequences before building (cross-repo cutovers like #873 are the canonical case);
   chaotic → out of scope by definition.

### Upstream — consume, don't clone

The governing source is the spec charter, referenced here by link, not duplicated:

- [`docs/charter/decision-compass.md`](https://github.com/the-greenman/srs/blob/master/docs/charter/decision-compass.md)
  in the `srs` repo — the full Pattern Grid, axis/cell preferences, layer rules, and decision
  modes.
- Spec decisions `9ee14517` (layer rules), `7caca3a1` (decision modes), `cce3c00e` (the Pattern
  Grid and cell/axis preferences) — each a `srs/records/tier-2/rfc-decision-<id>.json` record.

Nothing in the five rules above restates a compass mechanism; each cites its compass source
directly. When the compass changes, this ADR's rules are re-checked against it, not maintained
in parallel.

### Two worked examples (this week's owner-ruled precedents)

Both surfaced from the same PR (the-greenman/srs-rust#876, owner ruling 2026-08-26) and are
recorded here as the profile's first real applications, not hypotheticals:

**(a) `aiGuidance` stays typed.** `#872` (RFC-040 metamodel v1.1.0 engine sync) had collapsed
`RecordType`'s `$schema`/`aiGuidance`/`semanticObjectType`/`tags` into an untyped `extra` map.
#876 reintroduced the first three as typed fields; the owner ruled the reversal correct because
these are spec-modeled `type.json` properties, not unknown-key carries — a **core spec property
belongs in a typed field, never a carry bag** (rule 3, one-way-per-goal: "typed fields for
spec-modeled properties, `extra` only for transition carriage" is now the one mechanism, not two
competing ones).

**(b) `semanticObjectType` rides `extra` as transition carriage.** The same PR's fourth
property did *not* get the same treatment: `semanticObjectType` was ruled a duplicate of the
Type system itself (the-greenman/srs#372, #383, #422 — collapse execution still pending at
the-greenman/srs#272), so re-typing it on `RecordType` would re-entrench the exact construct
already scheduled for removal. It stays in `extra` — untyped, round-tripped, but not named Type
surface — until the-greenman/srs#383 executes the collapse. **A ruled construct removal is never
re-typed** in the interim; it rides the transitional carry bag until its collapse lands.

### Interim enforcement: the struct↔schema conformance guard

Rule enforcement here is presence-only in this unit (checklists below) — no automated rule
checker ships with this ADR. The one piece of automated enforcement named as a direct
consequence of the worked examples above is **not built here**:

`the-greenman/srs-rust#777` ("CI gate needed: `docs/schema/2.0/*.json` vs `srs-core`
struct/validator conformance — third seam") already tracks the mechanism: a CI check verifying
that `srs-core` struct definitions cover the same property set the mirrored JSON Schema
declares, so a spec-modeled property silently demoted to a carry bag (exactly the `aiGuidance`
near-miss above, caught only by owner review) is caught mechanically instead. The-greenman/srs#490
(the post-256 meaning-placement review) names this guard as the **interim defense** pending the
larger generator-inversion successor (Rust definitions generated from the metamodel, deferred
until epic #256 closes). No new issue is filed — #777 already covers this ground and is cited
here as the follow-up.

## Consequences

**Positive:** The implementation layer gets a lightweight, human-judged decision profile that
plugs directly into the spec's own charter rather than inventing a parallel one. The two worked
examples give the next ADR/PR author a concrete answer for "typed field or carry bag?" instead
of re-deriving it.

**Negative / trade-offs:** The checklist is presence-only — it cannot catch a rule cited but not
honored (the same ritualization risk the compass itself names in its observability-pressure
audit). The struct↔schema conformance guard that would mechanically enforce rule 3's worked
example stays unbuilt; until #777 lands, the "aiGuidance nearly became a carry bag" class of
mistake is caught only by review.

**Neutral:** This ADR does not change any crate boundary, service, or CLI contract — it is
process documentation, landing alongside its checklist per the-greenman/srs-rust#308/#311's
lesson that a rule without a mechanism decays.
