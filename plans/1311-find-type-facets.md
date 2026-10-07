# #1311 find facets: byType typeId + request-level cap

Spec gate: `srs/docs/schema/2.0/discovery.json` has no facets; they are a tooling payload (#1219), so no RFC.

Change (one mechanism, additive):
- `FacetCount.typeId` (byType values only).
- `FindPage.by_type_limit` (None = 20, Some(0) = every type); `other` stays correct.
- CLI `--by-type-limit`, MCP `byTypeLimit`, bindings trailing optional `by_type_limit`; golden `find.json` regenerated.
- Tests: service (cap/other/typeId/sum), CLI, MCP, bindings; `docs/dogfooding.md` S52 updated.
