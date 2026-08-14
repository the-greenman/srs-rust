# ADR-044: Navigation identity is optional, never inferred

- **Status:** accepted
- **Date:** 2026-08-14
- **Supersedes:** —
- **Superseded by:** —

## Context

`repository_navigation` returns the repository's identity node and its ordered navigation sections
(RFC-013 Change B: the identity is excluded from the section list). Its result struct declared
`identity: NavigationNode` — non-optional — which forced the service to produce an identity node for
every repository, including those that have none.

It met that obligation by inference: when the root container carried no `identityInstanceId`, the
first `rootInstanceIds` entry was promoted to the identity node. Because navigation then excluded
the identity from `sections`, an ordinary section record was simultaneously presented to every
client as the repository's identity object *and* removed from navigation. No diagnostic was emitted.

RFC-029 line 104 is explicit: a root container with a valid root container and no
`identityInstanceId` is **valid**. R5's type constraint applies only when the field is present.
So the inference did not paper over corruption — it fabricated an answer for a supported state, and
the fabrication was indistinguishable from a genuine identity in the payload.

The same shape appeared in the sibling branch: when `manifest.container` was absent entirely, the
service returned `NavigationNode::default()` — an identity node whose every field is the empty
string. Different mechanism, same category of untruth.

srs-rust#834's delete cascade (which clears `identityInstanceId` when the record it names is deleted,
rather than leaving a dangling pointer that made `validate` report I-81 at error severity) made the
identity-less state substantially easier to reach, which is what surfaced this. The inference itself
predates it and fired for any identity-less root container.

Three shapes were available: drop the fallback and make the field optional; keep the fallback but
emit a diagnostic naming the value as inferred; or keep the fallback and also leave the promoted
record in `sections`. The latter two keep a record labelled as the repository's identity when it
never claimed to be one — a diagnostic makes the misrepresentation *documented*, not *absent*, and
every client that renders `identity` without reading `diagnostics` still shows the wrong thing.

## Decision

**A derived payload field with no source in the underlying data is reported absent and accompanied
by a diagnostic. It is never inferred from an unrelated record.**

Concretely, in `repository_navigation_service`:

- `RepositoryNavigation.identity` is `Option<NavigationNode>`, serialized with
  `#[serde(skip_serializing_if = "Option::is_none")]`. When there is no identity, the `identity` key
  is **omitted** from the JSON object rather than emitted as an empty or substituted node.
- `identity` is populated from `manifest.container.identityInstanceId` and from nothing else. The
  `rootInstanceIds.first()` fallback is removed.
- An absent `identityInstanceId` is **not an error**. The previous
  `RepositoryError::NotFound { path: "manifest.container.identityInstanceId" }` is removed; the
  service returns `Ok` with `identity: None` and a diagnostic naming the absence and citing RFC-029.
- Every root-container member is a navigation section unless it *is* the identity. With no identity,
  nothing is excluded — all roots appear in `sections`.
- The `manifest.container`-absent branch returns `identity: None`, not `NavigationNode::default()`.

The generalisation is deliberate and binding on **identity-shaped** derived fields — any field whose
value asserts *what a thing is* rather than merely selecting a representative element. If a service
cannot answer such a question from the data, the payload says so. Filling the hole with the nearest
plausible value is the failure mode this ADR exists to prevent: it converts a recoverable "not set"
into an unrecoverable "set to the wrong thing", and the client has no way to tell them apart.

The clause is scoped that narrowly on purpose. Two other `root_instance_ids.first()` selections
survive in `srs-repository` and are **not** governed by this ADR:

- `container_view_service.rs:191-196` — picks the first root to populate `ContainerView.root`.
- `view_service.rs:369-376` (`document_views_for_container`) — picks the first root to match a
  DocumentView.

Both select *a representative root* from an ordered list, which is a defensible reading of an
ordered `rootInstanceIds`; neither claims the chosen record *is* something it is not. They are noted
here so a future reader does not mistake this ADR for a sweep that already covered them. If either
is later found to misrepresent rather than merely select, it is separately tracked work, not a
violation of this decision.

Adapters (`srs-cli`, `srs-bindings`, `srs-mcp`) serialize the service struct verbatim per ADR-011,
ADR-013, and ADR-037 respectively. None of them reconstructs, defaults, or infers an identity to
restore the old shape.

## Consequences

**Positive:**

- A repository that has not named an identity is represented honestly. Clients can distinguish
  "this repo has no identity record" from "this repo's identity is X", which was previously
  impossible.
- No record silently disappears from navigation. The bug where an ordinary section vanished the
  moment the identity was cleared is structurally gone, not merely diagnosed.
- A container can stand alone. The identity node is a navigational convenience, not a precondition
  for a navigable repository — which matches what RFC-029 already permits at the data layer.
- The error path shrinks: an absent optional field no longer produces `NotFound`, so
  `repo navigation` stops failing outright on a repository the spec considers valid.

**Negative / trade-offs:**

- This is a payload contract change. A consumer outside this monorepo that assumes `identity` is
  always present will need to handle its absence. In-tree consumers were checked and need no
  change: `srs-web` already reads `raw.identity ?? {}`; `srs-vscode` has no consumer.
- Clients that want a headline for an identity-less repository must now choose their own fallback at
  the presentation layer. That is the correct place for it — a presentation default is reversible
  and local, whereas a service-level default is neither.

**Neutral:**

- The committed golden schema `crates/srs-cli/schemas/payload/repo-navigation.json` does not change,
  because `RepoNavigationPayload` embeds `RepositoryNavigation` opaquely (`"navigation": true`) under
  ADR-011's rule that external service types serialize as `serde_json::Value`. The contract change is
  therefore recorded here and in the service's tests rather than in a schema diff.
- The RFC-029 Tier-0-identity migration grace path is untouched. It applies when
  `identityInstanceId` is *present* and points at a note; this ADR governs only its absence.
- `srs repo validate` behaviour is unchanged. An absent `identityInstanceId` was valid before this
  ADR and remains valid after it — the change is to what navigation *reports*, not to what the
  repository *is*.
