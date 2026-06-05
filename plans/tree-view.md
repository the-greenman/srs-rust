# Plan: `srs tree` — hierarchical record tree view

## Summary

There is no way to visualise the parent-child structure of an SRS repository from the CLI. Records form a hierarchy via `contains` relations (source → child), and ordering within a level is expressed via `precedes` relations. Without a tree view, understanding document structure requires chaining multiple `relation list` and `record list` calls. This plan adds `srs tree` — a command that auto-detects top-level records, traverses `contains` edges, and outputs both a structured JSON payload and a pre-rendered ASCII tree.

This plan also fixes a scattered inconsistency: there is no canonical function for resolving a record's display label. The VS Code extension shows only `typeName`; `render_service.rs` extracts titles only when explicitly configured per view. A new `record_label.rs` module establishes the canonical implementation that all consumers can use.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Codex |
| Refactor + Shared Modules | Codex |
| Tree Service + CLI | Codex |
| Verification | Codex |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new ADRs required. This plan:
- Implements the ADR-010 handler pattern (one service call per handler, ≤15 lines)
- Implements the ADR-011 payload contract (named struct, generate-schemas run, golden file committed)
- Introduces two new utility modules in `srs-repository` that fix existing code duplication

---

## Contracts

### CLI output contract (ADR-011)

This plan **adds two new commands** (`tree`) and **two new payload structs** (`TreePayload`, `TreeNodePayload`).

Required actions:
- Add `TreePayload` and `TreeNodePayload` to `crates/srs-cli/src/payload.rs`
- Wire handler to use `output::serialize()`
- Run `cargo run --bin generate-schemas`
- Commit new `crates/srs-cli/schemas/payload/tree.json`

Verification: `cargo test --test payload_contracts` must pass.

### Entity schema sync (check-schema-sync.sh)

No new entity schemas. No action required.

---

## Scope

- New `srs-repository/src/relation_graph.rs` — extracted, `pub(crate)` traversal primitives shared by render and tree services
- New `srs-repository/src/record_label.rs` — canonical `build_field_name_index` + `record_display_label`; `pub(crate)` (VS Code consumes JSON via CLI, not the Rust API directly)
- New `srs-repository/src/tree_service.rs` — `build_tree()` producing structured data
- New `srs-cli/src/commands/tree.rs` — CLI handler + ASCII renderer
- Refactor `render_service.rs` to use `relation_graph` instead of its private copies (pure behaviour-preserving refactor)
- Payload structs `TreePayload` + `TreeNodePayload`
- Command `srs tree` with flags `--from`, `--container`, `--relation-type`, `--depth`, `--type`

**Out of scope:**
- Implementing `--format text` globally (text rendering is in the payload's `text` field, accessible via `jq -r '.payload.text'`)
- Enriching `RecordListPayload` with display labels for the VS Code extension (follow-on task)
- Tree traversal for relation types other than `contains` is supported but not the primary tested path
- Memoization of shared DAG subtrees — adversarial deep diamond DAGs will traverse shared subtrees once per path (potentially exponential). Practical SRS repos do not have deeply shared hierarchies. Document this as a known limitation; add memoization as a follow-on if benchmarks show it matters.

**Flag precedence rule:** `--from` and `--container` are mutually exclusive — `build_tree` returns an error if both are set. If neither is set, root auto-detection runs.

**Type filter ordering:** root auto-detection runs against ALL records (before type filter). A filtered-type record that is a child of an unfiltered parent will appear as a root in the output if its parent is filtered out. This is the defined behaviour — document in tests.

---

## Phases

### Phase 1: Shared utilities — `relation_graph.rs` and `record_label.rs`

**Goal:** Two new `pub(crate)` modules exist in `srs-repository`, `render_service.rs` uses them, and all existing render tests pass.

**Agent:** Refactor + Shared Modules

#### Tasks

- [ ] Create `crates/srs-repository/src/relation_graph.rs` with:
  - `pub(crate) fn sort_by_precedes_chain(records: Vec<Record>, relations: &[Relation]) -> Vec<Record>` — identical logic to current private copy in `render_service.rs:898`
  - `pub(crate) fn children_by_relation_type(source_id: &str, relation_type: &str, all_relations: &[Relation], store: &dyn RepositoryStore) -> Result<Vec<Record>, RepositoryError>` — returns `Vec<Record>` (consistent with `sort_by_precedes_chain`), loads child records by ID, applies `sort_by_precedes_chain` before returning; abstracts the pattern in `collect_subsections`
- [ ] Create `crates/srs-repository/src/record_label.rs` with:
  - `pub(crate) fn build_field_name_index(store: &dyn RepositoryStore) -> Result<HashMap<String, String>, RepositoryError>` — loads all field definitions from the package, returns `field_id → field_name` map
  - `pub(crate) fn record_display_label(record: &Record, field_name_index: &HashMap<String, String>) -> String` — searches `field_values` for first entry named `"title"`, then `"name"`, then `"label"`; falls back to `record.type_name`
- [ ] Register both modules in `crates/srs-repository/src/lib.rs`
- [ ] Refactor `render_service.rs`: delete private `sort_by_precedes_chain` (line 898) and `collect_subsections` (line 985); replace call-sites (lines 346, 1003, 1037, 1390) with `relation_graph::` equivalents — no behaviour change. The test at line 2671 calls the private function directly and must be updated to call `relation_graph::sort_by_precedes_chain`.

#### Acceptance Criteria

- [ ] `relation_graph::sort_by_precedes_chain` and `relation_graph::children_by_relation_type` exist and are `pub(crate)`; `children_by_relation_type` returns `Vec<Record>`
- [ ] `record_label::build_field_name_index` and `record_label::record_display_label` exist and are `pub(crate)`
- [ ] `render_service.rs` contains no private copies of `sort_by_precedes_chain` or `collect_subsections`
- [ ] All existing `render_service` tests pass unchanged — the refactor is behaviour-preserving

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write or verify:

- `relation_graph::tests::sort_by_precedes_chain_basic` — single chain sorts correctly
- `relation_graph::tests::sort_by_precedes_chain_cycle` — cycle does not loop; falls back to created_at for unreached nodes
- `record_label::tests::display_label_finds_title_field` — record with a `title` field returns its value
- `record_label::tests::display_label_finds_name_field` — record with no `title` but a `name` field returns its value
- `record_label::tests::display_label_fallback` — record with neither field returns `type_name`

#### Milestone gate

1. Verify all acceptance criteria above are met — check each checkbox.
2. Confirm every test listed in the Testing section exists in the codebase and passes.
3. Run lint and tests:

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

4. Update the plan file: mark completed task checkboxes `[x]` and acceptance criteria `[x]`.
5. Commit:

```bash
git commit
```

Do not start Phase 2 until this gate passes.

---

### Phase 2: `tree_service.rs` — structured tree data

**Goal:** `tree_service::build_tree` correctly traverses `contains` relations, resolves labels, detects cycles, and returns a structured `TreeResult`.

**Agent:** Tree Service + CLI

#### Tasks

- [ ] Create `crates/srs-repository/src/tree_service.rs` with:
  ```rust
  pub struct TreeOptions {
      pub root_ids: Option<Vec<String>>,   // None = auto-detect
      pub container_id: Option<String>,
      pub relation_type: String,           // default "contains"
      pub max_depth: Option<u32>,
      pub type_filter: Option<String>,     // "namespace/name"
  }

  pub struct TreeNode {
      pub instance_id: String,
      pub label: String,
      pub type_namespace: String,
      pub type_name: String,
      pub lifecycle_state: Option<String>,
      pub depth: u32,
      pub children: Vec<TreeNode>,
      pub cycle_pruned: bool,
  }

  pub struct TreeResult {
      pub roots: Vec<TreeNode>,
      pub diagnostics: Vec<String>,
  }

  pub fn build_tree(store: &dyn RepositoryStore, options: TreeOptions)
      -> Result<TreeResult, RepositoryError>
  ```
- [ ] Validate mutual exclusion: return `Err` if both `options.root_ids.is_some()` and `options.container_id.is_some()`
- [ ] Root auto-detection (neither flag set): load all relations, collect every `target_instance_id` from `options.relation_type` relations, roots = manifest instances not appearing as a target
- [ ] Container resolution: if `options.container_id` is set, call `container_service::list_roots` to get starting IDs; use those as `root_ids`
- [ ] Type filter applies to nodes, not to root detection — auto-detection runs against ALL manifest instances; filtered-type nodes that are children of unfiltered parents appear as roots if the parent is filtered out (defined behaviour, covered by test)
- [ ] Recursive traversal: pass `ancestors: HashSet<String>` (per-path) down; if child ID ∈ ancestors, emit `cycle_pruned: true` leaf; a node reachable via multiple paths is expanded each time (DAG behaviour — see DAG traversal note in Scope)
- [ ] Order children using `relation_graph::children_by_relation_type` (returns `Vec<Record>` already sorted by precedes chain)
- [ ] Label each node using `record_label::record_display_label` with the field name index built once before traversal starts
- [ ] Register module in `crates/srs-repository/src/lib.rs`

#### Acceptance Criteria

- [ ] No-arg call on a repo with a `contains`-structured hierarchy returns a correctly nested `TreeResult`
- [ ] A cycle in `contains` relations produces `cycle_pruned: true` on the back-edge node and does not loop
- [ ] A node reachable from two different parents appears under both parents (DAG supported)
- [ ] Passing both `--from` and `--container` returns an error
- [ ] `--container` flag correctly uses `rootInstanceIds` as starting nodes
- [ ] `--type` filter excludes non-matching nodes from the output; a filtered-type child of an unfiltered parent appears as a root when the parent is also filtered
- [ ] `--depth 1` returns only roots and their immediate children
- [ ] Labels resolve to title field value where present, fall back to `type_name`

#### Testing

```bash
cargo test -p srs-repository tree_service
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write or verify:

- `tree_service::tests::flat_repo_with_no_contains_returns_all_as_roots` — each instance is its own root
- `tree_service::tests::contains_hierarchy_nested_correctly` — parent→child nesting
- `tree_service::tests::cycle_detection_prunes_back_edge` — cycle produces `cycle_pruned: true` without hanging
- `tree_service::tests::diamond_dag_allows_duplicate_appearances` — shared child appears under both parents
- `tree_service::tests::depth_limit_respected` — `max_depth: Some(1)` returns only roots + immediate children
- `tree_service::tests::type_filter_excludes_non_matching` — filtered nodes absent from result
- `tree_service::tests::type_filter_orphans_become_roots` — filtered-type child with unfiltered parent appears as root
- `tree_service::tests::from_and_container_conflict_returns_error` — both flags set → `Err`

#### Milestone gate

1. Verify all acceptance criteria above are met — check each checkbox.
2. Confirm every test listed in the Testing section exists and passes.
3. Run:

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

4. Update plan checkboxes.
5. Commit.

---

### Phase 3: CLI command + payload contract

**Goal:** `srs tree` is a working command; golden schema file is committed; ASCII text renders correctly.

**Agent:** Tree Service + CLI

#### Tasks

- [ ] Add `TreePayload` and `TreeNodePayload` to `crates/srs-cli/src/payload.rs`:
  ```rust
  pub struct TreePayload {
      pub roots: Vec<TreeNodePayload>,
      pub text: String,
      pub diagnostics: Vec<String>,
  }

  pub struct TreeNodePayload {
      pub instance_id: String,
      pub label: String,
      pub type_namespace: String,
      pub type_name: String,
      pub lifecycle_state: Option<String>,
      pub depth: u32,
      #[schemars(with = "Vec<serde_json::Value>")]
      pub children: Vec<TreeNodePayload>,
      pub cycle_pruned: bool,
  }
  ```
- [ ] Create `crates/srs-cli/src/commands/tree.rs` with:
  - `TreeArgs` struct (clap `Parser`). The `--type` flag is a Rust keyword; use `#[clap(long = "type")] pub type_filter: Option<String>`. Full struct:
    ```rust
    #[derive(Parser)]
    pub struct TreeArgs {
        #[arg(long = "from")]
        pub from: Option<Vec<String>>,
        #[arg(long)]
        pub container: Option<String>,
        #[arg(long = "relation-type", default_value = "contains")]
        pub relation_type: String,
        #[arg(long)]
        pub depth: Option<u32>,
        #[arg(long = "type")]
        pub type_filter: Option<String>,
    }
    ```
  - `dispatch(ctx: CliContext, args: TreeArgs) -> Result<String>` (≤15 lines)
  - `render_ascii_tree(roots: &[tree_service::TreeNode]) -> String` — recursive prefix walker emitting `├──` / `└──` / `│   `; cycle nodes show `↻ cycle`; lifecycle state shown in brackets when present
- [ ] Add `pub mod tree;` to `crates/srs-cli/src/commands/mod.rs`
- [ ] Add `Tree(TreeArgs)` variant to `Commands` enum in `mod.rs`
- [ ] Add match arm `Commands::Tree(args) => tree::dispatch(ctx, args)` in `dispatch()`
- [ ] Run `cargo run --bin generate-schemas` and commit new `crates/srs-cli/schemas/payload/tree.json`

#### Acceptance Criteria

- [ ] `srs tree --repo <path>` returns valid JSON envelope with `ok: true`
- [ ] `srs tree --repo <path> | jq -r '.payload.text'` prints a legible ASCII tree
- [ ] `srs tree --repo <path> --type <namespace/name>` filters to that type only
- [ ] `srs tree --repo <path> --depth 1` returns only roots and immediate children
- [ ] `cargo test --test payload_contracts` passes
- [ ] `srs tree --repo ../srs/srs` on the spec repo produces a tree with nested sections

#### Testing

```bash
cargo run --bin generate-schemas
cargo test --test payload_contracts
cargo clippy -p srs-cli -- -D warnings

# Smoke against the live spec repo
cargo run --bin srs -- tree --repo ../srs/srs --pretty
cargo run --bin srs -- tree --repo ../srs/srs | jq -r '.payload.text'
```

Specific tests to write or verify:

- `tree::tests::render_ascii_tree_simple` — single root with two children renders with `├──` / `└──`
- `tree::tests::render_ascii_tree_cycle_node` — `cycle_pruned: true` node renders `↻ cycle`

#### Milestone gate

1. Verify all acceptance criteria above are met — check each checkbox.
2. Confirm every test listed in the Testing section exists and passes.
3. Run:

```bash
cargo test
cargo clippy -- -D warnings
```

4. Update plan checkboxes.
5. Commit.

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes
- [ ] `crates/srs-cli/schemas/payload/tree.json` committed
- [ ] `render_service.rs` no longer contains private `sort_by_precedes_chain` or `collect_subsections`
- [ ] `srs tree --repo ../srs/srs | jq -r '.payload.text'` produces a readable tree of spec sections
- [ ] `srs tree --repo ../srs/srs --container <id>` scopes to container roots correctly

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.
- Verification Agent runs after each major phase and before final sign-off.

## Assumptions

- The spec repo (`../srs/srs`) has `contains` relations between sections and subsections; this is the primary smoke-test target.
- `schemars` recursive type limitation for `TreeNodePayload.children` is handled by `#[schemars(with = "Vec<serde_json::Value>")]` as used elsewhere in `payload.rs`.
- `record_display_label` priority order (`title` > `name` > `label` > `type_name`) is sufficient for the spec repo; can be extended via field name list if needed.
