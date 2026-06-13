# Plan: JsonStore drops package vocabularies and lifecycles (#115)

## Summary

`JsonStore::load_package` hardcodes `vocabularies: vec![]` and `lifecycles: vec![]`, making any `.srsj` snapshot invalid when the source repo uses `Type.lifecycleRef` or field `vocabularyRef`. Additionally, `repository_portability.rs` does not export or import vocabularies/lifecycles during copy, so they are silently absent from `.srsj` files. Both need to be fixed together for the round-trip to work.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Repository Service Worker | — |
| Verification | — |

## Architecture Decisions

No new architectural decisions — extends the existing portability snapshot pattern already used for fields, types, views, themes, and blueprints.

---

## Contracts

### CLI output contract (ADR-011)

No CLI payload changes — this is a repository loading bug fix.

### Entity schema sync

No entity schema changes.

---

## Scope

- Fix `JsonStore::PackageMetadata` to include `vocabularies` and `lifecycles` paths
- Fix `JsonStore::load_package_from_prefix` to deserialize Vocabulary and Lifecycle objects
- Fix `JsonStore::load_package` to populate `vocabularies` and `lifecycles` on the returned `Package`
- Fix `PackageBoundarySnapshot` to carry `vocabularies: Vec<Vocabulary>` and `lifecycles: Vec<Lifecycle>`
- Fix `RawPackageMetadata` in portability to include `vocabularies` and `lifecycles` paths
- Fix `export_package_boundary` to load vocabularies/lifecycles from source
- Fix `import_package_boundary` to write vocabularies/lifecycles to target and include paths in `package.json`

**Out of scope:** `lifecycle create` command (tracked in #116); vocabulary/lifecycle update/delete.

---

## Phases

### Phase 1: Fix JsonStore package loading

**Goal:** `JsonStore::load_package` returns a `Package` with vocabularies and lifecycles populated from the `.srsj` data map.

**Agent:** Repository Service Worker

#### Tasks

- [x] Add `vocabularies: Vec<String>` and `lifecycles: Vec<String>` to `PackageMetadata` in `json_store.rs`
- [x] Add loading of `Vec<Vocabulary>` and `Vec<Lifecycle>` in `load_package_from_prefix`; extend return tuple
- [x] Update `load_package` to use loaded vocabularies/lifecycles instead of `vec![]`

#### Acceptance Criteria

- [x] A JsonStore repo with vocabularies/lifecycles in package.json loads them correctly
- [x] `srs repo validate --repo <file-repo>` and `srs repo validate --repo <copy>.srsj` produce same vocabulary/lifecycle diagnostics

### Phase 2: Fix portability snapshot to carry vocabularies and lifecycles

**Goal:** `repo copy --from <file-repo> --to <copy>.srsj` produces a `.srsj` that validates with 0 V7 diagnostics when the source validates cleanly.

**Agent:** Repository Service Worker

#### Tasks

- [x] Add `vocabularies: Vec<Vocabulary>` and `lifecycles: Vec<Lifecycle>` to `PackageBoundarySnapshot` in `repository_portability.rs`
- [x] Add `vocabularies: Vec<String>` and `lifecycles: Vec<String>` to `RawPackageMetadata` in portability
- [x] Update `export_package_boundary` to load vocabularies/lifecycles from source
- [x] Update `import_package_boundary` to write vocabulary/lifecycle JSON files and include their paths in the generated `package.json`

#### Acceptance Criteria

- [x] `srs repo copy --from <file-repo> --to <copy>.srsj` produces a `.srsj` whose validation matches source
- [x] Vocabulary and lifecycle JSON files appear in the `.srsj` data map under `package/vocabularies/` and `package/lifecycles/`

#### Milestone gate

```bash
cargo test -p srs-repository
cargo clippy -- -D warnings
```

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `srs repo validate --repo docs/spec/examples/gallery-project-v2` returns 0 diagnostics
- [ ] `srs repo copy --from docs/spec/examples/gallery-project-v2 --to /tmp/g.srsj && srs repo validate --repo /tmp/g.srsj` returns 0 diagnostics

## Assumptions

- `docs/spec/examples/gallery-project-v2` exists and has vocabulary/lifecycle references (as described in the issue)
- `Vocabulary` and `Lifecycle` JSON serialize/deserialize correctly (round-trip tested elsewhere)
