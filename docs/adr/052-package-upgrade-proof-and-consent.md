# ADR-052: Package upgrade: proof by prior bundle and per-definition consent

- **Status:** accepted
- **Date:** 2026-10-07
- **Supersedes:** —
- **Superseded by:** —
- **Related:** ADR-030 (import tracking and reference copies), ADR-050 (the `.srspkg` codec),
  `plans/1152-package-upgrade.md`, srs-rust#1325.

## Context

`package upgrade` (#1152) overwrites a same-id, same-version definition whose upstream changed
only when it can prove the installed file unedited: it equals the install-time reference copy at
`<boundary>/.srs-import/refs/<rel>` (ADR-030). Where the copy is missing (installs made before
the copies were written, kinds older code skipped, a best-effort write that failed) the
definition is reported as a `no-reference-copy` conflict and kept, with no way forward. The live
essay repo (essay 1.5.0 installed, 1.7.0 bundled) hit this for document-state v2, comment,
`comment_text` and paragraph.

## Decision

Governing spec: RFC-014 Change E option (b) (reference content re-fetched from the published
source) and `rfc-decision-e5828aa8` (a local edit is protected).

1. **Proof by prior bundle.** `UpgradeOptions.prior_bundles` carries earlier published bundles
   of the same package (a different `packageId` is `InvalidInput`). They are proof only, never
   installed. A `no-reference-copy` definition whose installed JSON equals a prior bundle's
   definition of the same (kind, id, version), by the same value equality the classifier already
   uses, is clean: it is overwritten, its reference copy is written, and it is listed in
   `updated` with `provenBy` = that prior bundle's version.
2. **Per-definition consent.** `UpgradeOptions.adopt` lists definition ids. A definition still
   unproven after step 1 and named in `adopt` is overwritten (reference copy written) and listed
   in the new `adopted` list. Consent is per definition, never a global flag, because adoption
   discards content nothing can show to be unmodified.
   Adopting a `no-reference-copy` conflict deliberately overwrites any unknown local edit: that
   is the consent trade-off. `adopt` matches by id and covers every version of that id in the
   bundle. An id that matched nothing adoptable gets a note `adopt <id>: nothing to adopt`.
3. **Never adoptable.** A `local-edit` (a reference copy proves the file was edited) and a
   `key-collision` stay conflicts even if named in `adopt`; a `notes` entry says
   `not adoptable: ...`. That is the rfc-decision-e5828aa8 protection: consent cannot be given
   for something the system knows is the user's work.
   **Trust assumption:** prior bundles must be published artifacts the caller verified (e.g. by
   sha256). A bundle exported from the user's own edited boundary would "prove" edits clean.
   The engine only refuses a prior whose version is not older than the new bundle's or is newer
   than the installed version (`InvalidInput`), as does a different `packageId`.
4. A dry run classifies identically and writes nothing. A re-run converges (reference copies now
   exist).

A conflict is adoptable exactly when `conflictKind == "no-reference-copy"`; clients key on that,
never on note text.

Result shape: `updated` items gain optional `provenBy`; `adopted` is a new list of the same item
shape; `conflicts` is unchanged (`conflictKind` stays `local-edit | no-reference-copy |
key-collision`). Adapters map inputs only: CLI `--prior-bundle <file>` / `--adopt <id>`
(repeatable), MCP `priorBundles` / `adopt`, WASM options `{priorBundles: [bundleText], adopt: [id]}`.

## Consequences

**Positive:** old installs upgrade without losing the edit protection; the unproven residue is an
explicit, auditable choice.

**Negative / trade-offs:** a client must ship the earlier bundles it wants to prove against
(github.com release downloads have no CORS header). Proof fails for a version that was never
published as a bundle.

## Implementation charter (ADR-048)

- [x] **Spec-first** — RFC-014 Change E (b); rfc-decision-e5828aa8.
- [x] **Layer test** — core service (`package_install_service`); adapters map inputs only.
- [x] **One way per goal** — extends the one upgrade classifier; no second upgrade path.
- [x] **Parity and mirror obligations** — payload twin + golden `package-upgrade` schema
      regenerated; no spec schema mirror change.
- [x] **Decision mode** — complicated (owner ruled 2026-10-07).
