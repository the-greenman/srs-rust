# SRS Rust — Staged TDD Implementation Plan

## Principle

**Library first. CLI is one consumer.**

```
srs-core        — types + validation. No I/O. WASM/FFI safe.
srs-repository  — filesystem I/O. Detects repos, loads/writes instances.
srs-cli         — arg parsing + dispatch only. No logic.
```

Each stage follows the same sequence:
1. **Contract** — define public types and function signatures
2. **Stubs** — implement empty `todo!()` bodies so everything compiles
3. **Tests** — write tests that fail against stubs
4. **Implementation** — make tests pass

---

## Error handling contract

- `srs-core`: `CoreError` via `thiserror`. No `unwrap()`, no `anyhow`.
- `srs-repository`: `RepositoryError` via `thiserror`. No `unwrap()`, no `anyhow`.
- `srs-cli`: `anyhow::Result`. Library errors convert via `From` automatically.

---

## CLI output envelope

All commands return a fixed envelope shape:

```json
{ "ok": true, "command": "note list", "version": "0.1.0", "payload": { ... } }
{ "ok": false, "command": "note get", "version": "0.1.0", "diagnostics": ["..."] }
```

The payload is **always nested under `"payload"`**, not flattened into the envelope. This keeps the envelope shape fixed regardless of command, avoids key collisions with `ok`/`command`/`version`/`diagnostics`, and is easier to type in any consumer language.

Exit code `0` always. Non-zero only for invocation failures (bad args, runtime panic). Validation errors and not-found responses use `ok: false` with diagnostics, exit `0`.

---

## Stage 1 — Core Types (`srs-core`) ✓ COMPLETE

All 8 tests pass. `cargo test -p srs-core` green.

### Files

| File | Status |
|---|---|
| `crates/srs-core/src/error.rs` | Done |
| `crates/srs-core/src/types/mod.rs` | Done |
| `crates/srs-core/src/types/note.rs` | Done |
| `crates/srs-core/src/validation/mod.rs` | Done |
| `crates/srs-core/src/validation/note.rs` | Done |
| `crates/srs-core/src/extensions/mod.rs` | Done (empty) |
| `crates/srs-core/src/lib.rs` | Done |

### Contracts

**`error.rs`**
```rust
pub enum CoreError {
    DuplicateSectionName { name: String },
    EmptyTag,
    Json(#[from] serde_json::Error),
}
```
Also implements `PartialEq` for test assertions.

**`types/note.rs`**
```rust
pub struct Note {
    pub instance_id: String,
    pub title: Option<String>,
    pub tags: Option<Vec<String>>,
    pub sections: Vec<NoteSection>,
    pub graduated_at: Option<String>,
    pub source_refs: Option<Vec<SourceReference>>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub meta: Option<serde_json::Value>,
}

pub struct NoteSection {
    pub name: String,
    pub label: Option<String>,
    pub content: String,
    pub content_hint: Option<ContentHint>,
    pub tags: Option<Vec<String>>,
}

// serde(rename_all = "kebab-case")
pub enum ContentHint { Text, Markdown, Plain }

pub struct SourceReference {
    pub source_type: SourceType,
    pub source_id: String,
    pub source_standard: Option<String>,
    pub stream_id: Option<String>,
    pub relation_type: Option<RelationType>,
    pub confidence: Option<f64>,
    pub note: Option<String>,
}

// serde(rename_all = "kebab-case") — produces "transcript-chunk", "repository-document", etc.
pub enum SourceType { TranscriptChunk, TranscriptSegment, ExternalDocument, RepositoryDocument }

// serde(rename_all = "kebab-case") — produces "derived-from", "quoted-from", etc.
pub enum RelationType { Evidence, DerivedFrom, QuotedFrom, InspiredBy, SupersedesContext }
```
All structs: `serde(rename_all = "camelCase")`, `skip_serializing_if = "Option::is_none"` on optional fields.

**`validation/note.rs`**
```rust
pub fn validate_note(note: &Note) -> Result<(), CoreError>
```
Checks: section names unique (invariant 18), all tags non-empty at Note and Section level (invariant 18a).

### Tests (all passing)

`types/note.rs`: `test_note_roundtrip_json`, `test_origin_purpose_deserializes` (sections.len() == 6), `test_source_type_serializes_hyphenated`, `test_relation_type_serializes_hyphenated`, `test_content_hint_serializes_lowercase`

`validation/note.rs`: `test_valid_note_passes`, `test_duplicate_section_name_fails`, `test_empty_tag_on_note_fails`, `test_empty_tag_on_section_fails`

---

## Stage 2 — Repository Detect + Manifest (`srs-repository`) ✓ COMPLETE

All tests pass. `cargo test -p srs-repository` green.

### Files

| File | Status |
|---|---|
| `crates/srs-repository/src/error.rs` | Done |
| `crates/srs-repository/src/detect.rs` | Done |
| `crates/srs-repository/src/index.rs` | Done |
| `crates/srs-repository/src/manifest.rs` | Done |
| `crates/srs-repository/src/lib.rs` | Done |
| `crates/srs-repository/Cargo.toml` | Done (uuid added) |

### Contracts

**`error.rs`**
```rust
pub enum RepositoryError {
    NotFound { path: PathBuf },
    ManifestMissing { path: PathBuf },
    ManifestParse { path: PathBuf, source: serde_json::Error },
    NoteLoad { path: PathBuf, source: serde_json::Error },
    NoteValidation { path: PathBuf, source: srs_core::error::CoreError },
    NoteWrite { path: PathBuf, source: std::io::Error },
    Io { path: PathBuf, source: std::io::Error },
    Serialize { path: PathBuf, source: serde_json::Error },
}
```
Also implements `PartialEq` for test assertions.

**`detect.rs`**
```rust
pub fn find_repo_root(start: &Path) -> Result<PathBuf, RepositoryError>
```
Walks `start.ancestors()`, returns first ancestor containing `.srs/`.

**`index.rs`**
```rust
#[serde(untagged)]
pub enum InstanceIndexEntry { Path(String), Object(InstanceIndexObject) }

pub struct InstanceIndexObject {
    pub instance_id: String,
    pub tier: u8,
    pub path: String,
    pub title: Option<serde_json::Value>,
    pub tags: Option<Vec<String>>,
}

impl InstanceIndexEntry {
    pub fn path(&self) -> &str
    pub fn instance_id(&self) -> Option<&str>
    pub fn tier(&self) -> Option<u8>
    pub fn title(&self) -> Option<String>
    pub fn is_note(&self) -> bool  // tier == 0; legacy entries assumed tier 0
}
```

**`manifest.rs`**
```rust
pub struct Manifest {
    pub instance_index: Vec<InstanceIndexEntry>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
    #[serde(skip)]
    pub root: PathBuf,
}

pub fn load_manifest(repo_root: &Path) -> Result<Manifest, RepositoryError>
```

### Tests (all passing)

`detect.rs`: `test_find_repo_root_from_nested_path`, `test_find_repo_root_not_found`

`index.rs`: `test_legacy_string_entry_deserializes`, `test_object_entry_deserializes`, `test_is_note_for_tier_0`, `test_is_note_for_non_zero_tier`, `test_is_note_for_legacy_path`

`manifest.rs`: `test_load_live_manifest` (live repo, first entry == `"records/notes/origin-purpose.json"`), `test_legacy_index_round_trips`

---

## Stage 3 — Note Loader + Writer (`srs-repository`) ✓ COMPLETE

All tests pass. `cargo test -p srs-repository` green.

### Files

| File | Status |
|---|---|
| `crates/srs-repository/src/loader.rs` | Done |
| `crates/srs-repository/src/writer.rs` | Done |

### Contracts

**`loader.rs`**
```rust
pub fn load_note(path: &Path) -> Result<Note, RepositoryError>
pub fn load_note_relative(repo_root: &Path, relative_path: &str) -> Result<Note, RepositoryError>
```
`load_note` always calls `validate_note` after deserialization. Validation failure → `NoteValidation`.

**`writer.rs`**
```rust
pub fn new_instance_id() -> String
pub fn write_note(note: &Note, path: &Path) -> Result<(), RepositoryError>
pub fn upsert_index_entry(manifest: &mut Manifest, note: &Note, relative_path: &str)
pub fn write_manifest(manifest: &Manifest) -> Result<(), RepositoryError>
```
`write_note` serializes to JSON, injects `$schema` header, writes pretty-printed. Uses `Serialize` error variant (not `NoteWrite`) for serialization failures; `NoteWrite` for filesystem write failures.

`upsert_index_entry` builds an `InstanceIndexObject` with `tier: 0`. Replaces existing entry with matching `instance_id()` or appends.

`write_manifest` round-trips all `extra` fields unchanged.

### Tests (all passing)

`loader.rs`: `test_load_origin_purpose` (sections.len() == 6), `test_load_validates_on_read`

`writer.rs`: `test_new_instance_id_produces_unique_uuids`, `test_write_note_roundtrip`, `test_upsert_index_entry_adds_new`, `test_upsert_index_entry_replaces_existing`, `test_write_manifest_preserves_extra_fields`

---

## Stage 4 — CLI Wiring (`srs-cli`) ✓ COMPLETE

`cargo build -p srs` succeeds. Read-only integration tests pass.

### Files

| File | Status |
|---|---|
| `crates/srs-cli/src/output.rs` | Done |
| `crates/srs-cli/src/commands/mod.rs` | Done |
| `crates/srs-cli/src/commands/note.rs` | Done |
| `crates/srs-cli/src/main.rs` | Done |
| `crates/srs-cli/Cargo.toml` | Done (clap added) |
| `Cargo.toml` (workspace) | Done (clap added) |

### Contracts

**`output.rs`**
```rust
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn ok(command: &str, payload: serde_json::Value) -> String
// → { "ok": true, "command": "...", "version": "...", "payload": { ... } }

pub fn err(command: &str, diagnostics: Vec<String>) -> String
// → { "ok": false, "command": "...", "version": "...", "diagnostics": ["..."] }
```

**`commands/mod.rs`**
```rust
#[derive(Parser)] pub struct Cli { command: Commands }
#[derive(Subcommand)] pub enum Commands { Note(NoteCommand) }
pub fn dispatch(cli: Cli) -> anyhow::Result<String>
```

**`commands/note.rs`**

`NoteCommand` subcommands: `List { repo?, tag?, json }`, `Get { repo?, id, json }`, `Create { repo?, json }`, `Tag { repo?, id, add_tag, json }`. `--json` is a silent no-op on all subcommands; output is always JSON.

```rust
fn resolve_repo(repo: Option<PathBuf>) -> anyhow::Result<PathBuf>
// Some(path) → return as-is. None → find_repo_root(current_dir())

fn cmd_note_list(repo: Option<PathBuf>, tag: Option<String>) -> anyhow::Result<String>
// payload: { "notes": [{ "instanceId", "path", "title" }] }
// filters to is_note() entries only; if --tag, loads note to check tags

fn cmd_note_get(repo: Option<PathBuf>, id: String) -> anyhow::Result<String>
// finds entry by instance_id(); rejects non-notes; payload: { "note": <full Note> }
// not found or non-note → ok:false + diagnostic

fn cmd_note_create(repo: Option<PathBuf>) -> anyhow::Result<String>
// reads JSON from stdin; mints instance_id if empty; validates; slugifies title for filename
// writes to records/notes/<slug>.json; upserts + writes manifest
// payload: { "note": <created Note> }

fn cmd_note_tag(repo: Option<PathBuf>, id: String, add_tag: String) -> anyhow::Result<String>
// adds tag if not present; writes note back; upserts + writes manifest
// payload: { "note": <updated Note> }
```

**Known gap in current implementation:** `cmd_note_tag` updates the note file but does not call `upsert_index_entry` + `write_manifest` to persist the updated tags to the manifest index.

---

## Stage 5 — Integration Tests ⚠ PARTIAL

5 of 7 tests passing. 2 write tests remain ignored pending temp repo fixture.

### Location

`crates/srs-cli/tests/integration_tests.rs` — discovered automatically by Cargo as a crate integration test (no workspace `[[test]]` entry needed).

### Helper functions

```rust
fn run_srs(args: &[&str]) -> serde_json::Value
// asserts exit 0, parses JSON, panics otherwise

fn run_srs_stdin(args: &[&str], stdin: &str) -> serde_json::Value
// pipes stdin, asserts exit 0, parses JSON
```

Both use `env!("CARGO_BIN_EXE_srs")` and run with cwd = `/home/greenman/dev/semanticops/srs`.

### Tests

| Test | Status |
|---|---|
| `test_note_list_ok` | ✓ passing |
| `test_note_list_contains_known_note` | ✓ passing — origin-purpose id present in payload.notes |
| `test_note_list_filter_by_tag` | ✓ passing — `--tag purpose` returns ≥1 result |
| `test_note_get_by_id` | ✓ passing — sections.len() == 6 |
| `test_note_get_unknown_id_returns_error` | ✓ passing — ok:false, non-empty diagnostics |
| `test_note_create_and_retrieve` | ⚠ ignored — needs temp repo with `.srs/` |
| `test_note_tag_adds_tag` | ⚠ ignored — needs temp repo with `.srs/` |

### Remaining work for Stage 5

The two write tests need a temp dir fixture that:
1. Creates a `.srs/` marker directory
2. Copies `manifest.json` from the live repo (or writes a minimal one)
3. Runs `note create` and `note tag` against the temp dir
4. Cleans up after the test

Fix `cmd_note_tag` manifest update gap before enabling these tests.

---

## File index

| File | Stage | Status |
|---|---|---|
| `crates/srs-core/src/error.rs` | 1 | ✓ Done |
| `crates/srs-core/src/types/mod.rs` | 1 | ✓ Done |
| `crates/srs-core/src/types/note.rs` | 1 | ✓ Done |
| `crates/srs-core/src/validation/mod.rs` | 1 | ✓ Done |
| `crates/srs-core/src/validation/note.rs` | 1 | ✓ Done |
| `crates/srs-core/src/extensions/mod.rs` | 1 | ✓ Done (empty) |
| `crates/srs-core/src/lib.rs` | 1 | ✓ Done |
| `crates/srs-repository/src/error.rs` | 2 | ✓ Done |
| `crates/srs-repository/src/detect.rs` | 2 | ✓ Done |
| `crates/srs-repository/src/index.rs` | 2 | ✓ Done |
| `crates/srs-repository/src/manifest.rs` | 2 | ✓ Done |
| `crates/srs-repository/src/lib.rs` | 2+3 | ✓ Done |
| `crates/srs-repository/Cargo.toml` | 2 | ✓ Done |
| `crates/srs-repository/src/loader.rs` | 3 | ✓ Done |
| `crates/srs-repository/src/writer.rs` | 3 | ✓ Done |
| `crates/srs-cli/src/output.rs` | 4 | ✓ Done |
| `crates/srs-cli/src/commands/mod.rs` | 4 | ✓ Done |
| `crates/srs-cli/src/commands/note.rs` | 4 | ✓ Done |
| `crates/srs-cli/src/main.rs` | 4 | ✓ Done |
| `crates/srs-cli/Cargo.toml` | 4 | ✓ Done |
| `Cargo.toml` (workspace) | 4 | ✓ Done |
| `crates/srs-cli/tests/integration_tests.rs` | 5 | ⚠ Partial |
