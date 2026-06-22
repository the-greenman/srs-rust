# ADR-017: Deterministic `.srsj` serialization via `BTreeMap`

- **Status:** accepted
- **Date:** 2026-06-22
- **Supersedes:** —
- **Superseded by:** —

## Context

`JsonStore` (the `.srsj` single-file repository backend) keeps all repository entities in an
in-memory `data` map keyed by relative path, and serialises that map directly into the `.srsj`
envelope via `to_srsj_string` (the export primitive established by ADR-015, shared by `flush()`
and `srs-bindings::export_srsj`).

`data` was a `std::collections::HashMap`. HashMap iteration order is non-deterministic (randomised
per process), so every write re-emitted the entries in a different order even when the semantic
content was unchanged. Consequences observed in issue #171:

- A one-field edit (e.g. `srs type update`) rewrote the *entire* `.srsj` — ~1400+ changed lines for
  a single-entry change, with only one entry actually different.
- Writes were **not idempotent**: running the same no-op write twice produced two different files.
- Committed `.srsj` fixtures could not be regenerated or reviewed with a minimal diff, and CLI- and
  app-exported files never matched byte-for-byte despite identical content.

serde_json's `preserve_order` feature is **not** enabled in this workspace (confirmed: `serde_json = "1"`
with no features in the workspace `Cargo.toml`), so `serde_json::Value::Object` is backed by serde_json's
`Map`, which aliases `BTreeMap` — values serialise with sorted keys. The manifest is deterministic in the
write path because `to_srsj_string` converts it via `serde_json::to_value` first, which normalises its
flattened `extra` HashMap into a sorted object. Entity values (Field/Type/Container/etc., each carrying a
flattened `extra` HashMap) are likewise normalised because they are stored into `data` via
`serde_json::to_value` at save time. The sole remaining source of non-determinism was the top-level `data`
map itself, which serde serialises in raw iteration order.

This property complements ADR-007 (file-before-index write ordering): ADR-007 keeps the index internally
consistent across interrupted writes; ADR-017 keeps the serialised bytes stable for unchanged content. Together
they make the `.srsj` file deterministic in both membership and byte layout after any operation.

## Decision

The `.srsj` envelope's `data` map (both `JsonStoreFile.data` and the in-memory `JsonStoreState.data`)
is a `BTreeMap<String, serde_json::Value>`, not a `HashMap`. This yields deterministic,
minimal-diff, idempotent `.srsj` writes: entries are always emitted in sorted key order, so a no-op
content write reproduces the file byte-for-byte and a single-entry change produces a single-entry
diff.

Deterministic serialisation is a **required property** of the `.srsj` format. Two rules enforce it:

1. **Envelope maps must be ordered.** Any map serialised directly into the `.srsj` envelope (currently
   `data`) must be an ordered container (`BTreeMap`) or be sorted at serialisation time. Reintroducing
   `HashMap` for the serialised `data`, or relying on serde_json `preserve_order` for envelope-level
   ordering, is rejected.
2. **Entity values must be produced via `serde_json::to_value`.** Values inserted into `data` must come
   from `serde_json::to_value(entity)` (which sorts nested keys through the `BTreeMap`-backed
   `serde_json::Value`), not from a hand-built `serde_json::json!()` literal populated from a HashMap-typed
   source. This guarantees nested key ordering inside each entry is deterministic, not just the top-level
   keys. Existing save paths already follow this; new ones must too.

Both rules depend on serde_json's `preserve_order` feature staying **disabled**. With it disabled,
`serde_json::Map` aliases `BTreeMap` (sorted keys); enabling it — in the workspace `Cargo.toml` *or* via any
crate-level `serde_json = { features = ["preserve_order"] }` override — would switch `Map` to insertion-order
`IndexMap` and silently break the nested-key guarantee in rule 2. Do not enable `preserve_order` without
revisiting this ADR.

## Consequences

**Positive:**
- `.srsj` writes are idempotent and produce minimal, reviewable diffs.
- Committed `.srsj` fixtures can be deterministically regenerated and verified.
- CLI-exported and app-exported (`export_srsj`) `.srsj` files match byte-for-byte for identical
  content.

**Negative / trade-offs:**
- `BTreeMap` lookups/inserts are O(log n) vs. HashMap's amortised O(1). The `data` map is small
  (one entry per repository file) and not on a hot path, so the cost is negligible.

**Neutral:**
- This is the first time a `.srsj` byte-ordering guarantee is stated explicitly. It does not change
  the `.srsj` *content* contract — only its ordering — so existing readers are unaffected (any
  conforming JSON reader is order-insensitive).
- The one-time reordering of existing committed `.srsj` fixtures into sorted order is expected and
  is a single, reviewable diff.
