# ADR-054: Fork into a container, carrying relations

- **Status:** accepted
- **Date:** 2026-10-08
- **Supersedes:** —
- **Superseded by:** —

## Context

`record fork` (srs-rust#1136) clones a record and swaps the clone into one container, linked
`derived-from` its original. Two gaps showed up when a human layer was forked from an agent-suggested
layer (srs-programme "affirm", semanticops.com#29): the original must already be a member of the
container (so "fork into Affirmed" is two calls, not one), and the fork drops every other link, such as
the persona a problem is `held-by` and the cluster that `contains` it, which are what make the record
meaningful. Fork is implementation-level (in no RFC or decision record), so this is an ADR, not a spec door.

## Decision

1. **One core.** `fork_service::fork_subtree` takes `ForkOptions { target_container, carry_relations }`.
   CLI, WASM and MCP are adapters over it (capability-layering); none re-implements either option.
2. **`target_container`.** The fork goes into that container; if the original is not a member it is
   appended to the container's outline first. It wins over the plain container argument. Failure
   removes everything created, including that appended entry (best effort, ADR-024, as for forks).
3. **`carry_relations: none | outgoing | all`.** Default `none` is today's behaviour. `outgoing`
   re-creates relations whose source is a forked original; `all` also those whose target is. The same
   `relationType` is created with the fork substituted, preserving `notes`, `sourceRefs` and `meta`.
   An end that is also forked in the same call is re-pointed to that fork, so a relation between two
   forked records is created once, between both forks.
4. **Excluded.** `derived-from` is never carried (the fork already has its own to the original;
   copying would assert the fork derived from the original's sources). The original's relations are
   never modified or removed.
5. **Attribution.** Carried relations go through `create_relation_auto`: relation type validation
   applies and `createdBy` is stamped from the session actor (RFC-046). The carried edges are testimony
   of whoever forked, not of whoever made the original's.
6. **Surface.** CLI `record fork <id> --into <container> --carry-relations none|outgoing|all`;
   `--into` and the global `--container` name the same thing (both allowed only if equal, else an
   error). WASM `fork_record(containerId, instanceId, optionsJson?)` with
   `{targetContainer?, carryRelations?}`. MCP `record_fork` gains `targetContainerId` and
   `carryRelations`; `containerId` becomes optional when `targetContainerId` is given. The write
   guard checks the target container. Payload gains `forks[].carriedRelationIds` and `carriedRelations`,
   both omitted when empty so the default payload is byte-identical.

## Consequences

- "Affirm" is one operation. Carrying `precedes` edges may collide with chain rules; that surfaces as a
  normal relation validation error and rolls the fork back.
- Not atomic against a crash, like all fork rollback today.
