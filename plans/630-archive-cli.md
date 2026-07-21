# Plan: CLI srs archive pack / unpack + ADR-036

## Summary

The `.srs` archive engine (`archive_pack` / `archive_unpack` in `srs-repository`) is fully implemented and tested (ADR-033, golden + roundtrip tests in #276/#277), but has no CLI surface. This plan wires the two handlers into `srs-cli`, adds typed payload structs per ADR-011, regenerates golden schemas, and authors ADR-036 (`.srs` is the default working/interchange format; `.srsj` is legacy). Closes srs-rust#630.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Claude (self) |
| CLI Worker | Claude (self) |
| Verification | Claude (self) |

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-cli-handler-pattern.md) | Handler = arg parsing + one service call + `output::ok`. No business logic in handlers. | accepted |
| [ADR-011](../docs/adr/011-cli-payload-contract.md) | Every CLI command output is a named struct in `payload.rs`; golden schemas regenerated after any change. | accepted |
| [ADR-033](../docs/adr/033-srs-archive-format.md) | `archive_pack`/`archive_unpack` are the authoritative archive functions; note that "a future `srs archive pack/unpack` CLI handler will be thin wrappers". | accepted |
| [ADR-036](../docs/adr/036-srs-is-default-working-format.md) | `.srs` (SRSzip) is the default working/interchange format; `.srsj` is a legacy lightweight projection. | proposed → accepted on ship |

No new architectural decisions beyond ADR-036 (which is seeded in the issue body as an owner decision).

---

## Contracts

### CLI output contract (ADR-011)

Two new commands are added:

- `srs archive pack` → `ArchivePackPayload { output_path: String, file_size_bytes: u64 }`
- `srs archive unpack` → `ArchiveUnpackPayload { target_dir: String, repository_id: String }`

Both structs must be added to `crates/srs-cli/src/payload.rs`. After adding: run `cargo run --bin generate-schemas` and commit the new `schemas/payload/archive-pack.json` and `schemas/payload/archive-unpack.json` files.

Verification: `cargo test --test payload_contracts` must pass.

### Entity schema sync (check-schema-sync.sh)

No changes to `srs/docs/schema/2.0/` — this plan adds CLI surface only. No action required.

---

## Scope

- Add `ArchiveCommand { Pack, Unpack }` enum to `crates/srs-cli/src/commands/mod.rs`
- Add `Commands::Archive` variant and dispatch arm to `crates/srs-cli/src/commands/mod.rs`
- Add `crates/srs-cli/src/commands/archive.rs` with two handlers
- Add `ArchivePackPayload` and `ArchiveUnpackPayload` structs to `crates/srs-cli/src/payload.rs`
- Run `cargo run --bin generate-schemas` and commit updated golden files
- Author `docs/adr/036-srs-is-default-working-format.md` (status: accepted, content from issue body)

**Out of scope:**

- WASM binding for archive CLI (WASM binding exists already via `archive_to_vec` / `JsonStore::from_archive` per ADR-033; a dedicated CLI-level WASM binding is deferred)
- `.srsj` deprecation warnings or migration tooling (deferred — noted in ADR-036)
- Slice support (srs#194 ext:slices RFC not yet accepted)

---

## Phases

### Phase 1: CLI handlers + payload structs + golden schemas

**Goal:** `srs archive pack --output repo.srs` and `srs archive unpack repo.srs --target dir` compile, work end-to-end, and `cargo test --test payload_contracts` passes.

**Agent:** CLI Worker

#### Tasks

- [ ] Add `ArchivePackPayload` and `ArchiveUnpackPayload` to `crates/srs-cli/src/payload.rs` (before the `#[cfg(test)]` block at line 1959)
- [ ] Add `ArchiveCommand` subcommand enum and `Commands::Archive` variant to `crates/srs-cli/src/commands/mod.rs`
- [ ] Add dispatch arm `Commands::Archive(cmd) => archive::dispatch(ctx, cmd)` in `dispatch()` function
- [ ] Create `crates/srs-cli/src/commands/archive.rs` with:
  - `cmd_archive_pack`: resolves `--output` path, calls `archive_pack` via `with_store`, returns `ArchivePackPayload`
  - `cmd_archive_unpack`: opens positional `.srs` file, creates `FileStore::new(target)`, calls `archive_unpack`, returns `ArchiveUnpackPayload`
- [ ] Add `pub mod archive;` to `crates/srs-cli/src/commands/mod.rs`
- [ ] Run `cargo run --bin generate-schemas` and verify two new schema files appear under `crates/srs-cli/schemas/payload/`
- [ ] Commit: `feat(cli): srs archive pack / unpack commands (#630)`

#### Implementation details

**Handler signatures (ADR-010 — ≤15 lines each):**

`cmd_archive_pack`:
```rust
fn cmd_archive_pack(ctx: CliContext, output: PathBuf) -> Result<String> {
    let mut file = std::fs::File::create(&output)
        .map_err(|e| anyhow::anyhow!("cannot create output file {:?}: {}", output, e))?;
    with_store(&ctx, |store| {
        srs_repository::archive_pack(store, &mut file)
            .map_err(anyhow::Error::from)
    })?;
    let file_size_bytes = std::fs::metadata(&output)
        .map(|m| m.len())
        .unwrap_or(0);
    output::serialize("archive pack", ArchivePackPayload {
        output_path: output.to_string_lossy().into_owned(),
        file_size_bytes,
    })
}
```

`cmd_archive_unpack`:
```rust
fn cmd_archive_unpack(ctx: CliContext, source: PathBuf, target: PathBuf) -> Result<String> {
    let file = std::fs::File::open(&source)
        .map_err(|e| anyhow::anyhow!("cannot open archive {:?}: {}", source, e))?;
    let target_store = srs_repository::FileStore::new(&target);
    srs_repository::archive_unpack(file, &target_store)
        .map_err(anyhow::Error::from)?;
    let manifest = target_store.load_manifest()
        .map_err(anyhow::Error::from)?;
    output::serialize("archive unpack", ArchiveUnpackPayload {
        target_dir: target.to_string_lossy().into_owned(),
        repository_id: manifest.repository_id,
    })
}
```

Note: `archive_unpack` does NOT go through `with_store` for the target — the target is always a new `FileStore` (not an existing repo). The source `srs` file is opened directly (thin I/O glue in the CLI layer, same pattern as `render export-bundle` in `render.rs`). The `ctx` is still used to satisfy the context pattern, but `with_store` is not called for the target. There is no spec-mandated reason to constrain the unpack target to a FileStore only; this matches the only unpack use-case (creating a new directory-based repo from an archive).

**Payload structs:**

```rust
// ── Archive payloads ──────────────────────────────────────────────────────────

/// Payload for `srs archive pack`.
/// Output format is `.srs` (SRSzip, deterministic ZIP). See ADR-033, ADR-036.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ArchivePackPayload {
    pub output_path: String,
    pub file_size_bytes: u64,
}

/// Payload for `srs archive unpack`.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveUnpackPayload {
    pub target_dir: String,
    pub repository_id: String,
}
```

**Subcommand enum:**

```rust
#[derive(Subcommand)]
pub enum ArchiveCommand {
    /// Pack the current repository into a deterministic .srs archive (SRSzip).
    /// Output is byte-identical across runs on the same repository state (ADR-033, ADR-036).
    Pack {
        /// Output file path for the .srs archive
        #[arg(long)]
        output: PathBuf,
    },
    /// Unpack a .srs archive into a new repository at the target directory.
    Unpack {
        /// Path to the .srs archive file to unpack
        source: PathBuf,
        /// Target directory to unpack the repository into
        #[arg(long)]
        target: PathBuf,
    },
}
```

#### Acceptance Criteria

- [ ] `srs archive pack --output /tmp/test.srs` runs on a FileStore repo, produces a non-empty file
- [ ] `srs archive pack` twice on the same repo produces byte-identical output
- [ ] `srs archive unpack /tmp/test.srs --target /tmp/unpacked` runs, creates directory, populates it
- [ ] `srs repo validate --repo /tmp/unpacked` exits with zero diagnostics
- [ ] `cargo test --test payload_contracts` passes
- [ ] Handler for `pack` is ≤15 lines; handler for `unpack` is ≤15 lines (ADR-010)

#### Testing

```bash
cargo test -p srs-cli
cargo test --test payload_contracts
cargo clippy -p srs-cli -- -D warnings
```

Specific tests to write or verify:
- `payload_contracts` golden test — verifies `archive-pack.json` and `archive-unpack.json` schema files are in sync
- Manual CLI smoke tests in dogfooding step

#### Milestone gate

1. Verify all acceptance criteria above are met — check each checkbox.
2. Run `cargo test --test payload_contracts` — must be green.
3. Run lint:

```bash
cargo test -p srs-cli
cargo clippy -p srs-cli -- -D warnings
```

4. Update plan checkboxes `[x]`.
5. Commit: `feat(cli): srs archive pack / unpack commands (#630)`

---

### Phase 2: ADR-036

**Goal:** `docs/adr/036-srs-is-default-working-format.md` committed with status `accepted`.

**Agent:** CLI Worker

#### Tasks

- [ ] Create `docs/adr/036-srs-is-default-working-format.md` using `ADR-TEMPLATE.md`
- [ ] Content from issue body + context from ADR-033 (format decision) and ADR-035 (distinct from presentation bundles)
- [ ] Status: `accepted` (owner decision 2026-07-18)
- [ ] Commit: `docs(adr): ADR-036 .srs is the default working format (#630)`

#### Acceptance Criteria

- [ ] File exists at `docs/adr/036-srs-is-default-working-format.md`
- [ ] Status is `accepted`
- [ ] References ADR-033 (archive format), ADR-035 (flat export bundle — distinct)
- [ ] Covers: `.srsj` supported for reading + srs-web until WASM surface lands; slices valid `.srs`; presentation bundles are ADR-035

#### Milestone gate

```bash
cat docs/adr/036-srs-is-default-working-format.md  # verify content
```

Commit: `docs(adr): ADR-036 .srs is the default working format (#630)`

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schema changes — expected to pass trivially)
- [ ] `srs archive pack --output repo.srs` + `srs archive unpack repo.srs --target dir` + `srs repo validate --repo dir` chain succeeds on the spec repo fixture at `../srs/srs`
- [ ] Pack twice → byte-identical output confirmed
- [ ] ADR-036 committed with status `accepted`

## Coordination Rules

- Single agent executing all phases sequentially.
- Commit at each milestone gate.
- Do not start Phase 2 until Phase 1 milestone gate passes.

## Assumptions

- `archive_pack` / `archive_unpack` are exported from `srs-repository` (confirmed at `lib.rs:66`).
- `FileStore::new(path)` does not require the directory to exist before calling `archive_unpack` — the store will create it. (If not, the handler creates the directory first.)
- The `--target` for unpack should always be a `FileStore` (directory). No use case for unpacking into a JsonStore.
