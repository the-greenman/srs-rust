# Plan: Fix srs-gov repo-create root container scaffolding (#297)

## Summary

`srs-gov repo-create` scaffolds a governance `.srsj` repository but produces an invalid
root container. PR #304 wrote `manifest.container` inline as raw JSON with `rootInstanceIds`
but never saved the container to the containers/ store. As a result `srs repo navigation`
fails with `ContainerNotFound` because `repository_navigation_service` calls
`get_container(store, containerId)` to read `memberInstanceIds` — a key that is never
written. This plan replaces the raw-JSON hack with proper CLI calls so the root container
is both saved to `containers/` and has the correct `memberInstanceIds` for navigation.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| srs-gov Worker (srs-gov only) | — |
| Verification | — |

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-001](../docs/adr/001-library-first-architecture.md) | Governs crate authority; srs-gov delegates to srs CLI, not direct store access | accepted |
| [ADR-008](../docs/adr/008-repository-lifecycle-and-portability.md) | Repository lifecycle is adapter-owned; srs-gov must not write raw JSON for lifecycle concerns | accepted |
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | srs-gov invokes srs CLI subprocesses (not raw JSON patching) for all store mutations | accepted |

No new ADRs needed — this fix brings `cmd_repo_create` into conformance with existing ADRs.

---

## Contracts

### CLI output contract (ADR-011)

No new CLI command output shapes. `srs-gov repo-create` output is produced by
`render::repo_created` (not via `payload.rs`). No payload structs change. No schema
regeneration required.

### Entity schema sync (check-schema-sync.sh)

No entity schema files are added or modified. `bash scripts/check-schema-sync.sh` will
continue to pass.

---

## Scope

- Fix `cmd_repo_create` in `crates/srs-gov/src/main.rs` to create the root container via
  `srs container create`, add members via `srs container members add`, and write the
  minimal `manifest.container` embed (`containerId` + `identityInstanceId` only).
- Add `srs_members_add` helper in `crates/srs-gov/src/main.rs` (mirrors `srs_roots_add`).
- Update the existing `repo_create_produces_valid_srsj` test: replace stale
  `manifest.container.rootInstanceIds` assertions with assertions on the full container
  in `data["containers/{id}.json"]`.
- Add new test `repo_create_navigation_works` in `crates/srs-gov/tests/flow.rs` that runs
  `srs repo navigation` on a freshly created governance repo and asserts the identity and
  decision-log section are returned correctly.

**Out of scope:**
- Changes to the srs-gov TUI or any other srs-gov commands.
- Adding support for multi-section governance repos (articles, roles).
- Changes to the `srs` CLI or `srs-repository` services.
- Updating the seed artifact (`governance-seed.srsj`).

---

## Phases

### Phase 1: Fix root container scaffolding and tests

**Goal:** `srs-gov repo-create` produces a fully conformant root container, `srs repo navigation`
succeeds on the created repo, and all tests pass.

**Agent:** srs-gov Worker

#### Tasks

- [ ] In `crates/srs-gov/src/main.rs`, add helper function `srs_members_add` after
  `srs_roots_add` (line ~754):
  ```rust
  fn srs_members_add(repo: &str, container_id: &str, instance_id: &str) -> Result<()> {
      srs::run_srs_write(
          &["container", "members", "add", container_id, instance_id],
          repo,
          "",
      )?;
      Ok(())
  }
  ```

- [ ] In `cmd_repo_create` (line ~707), replace step 4 (lines 711–719) with:
  ```rust
  // 4. Required root container (RFC-013): identity + top-level section navigation.
  //    Create via the CLI so the container is saved to containers/ and containerIndex.
  //    No containerType on the root container — it is the structural root of the repo,
  //    not a domain-typed section (matches the test fixture pattern in
  //    repository_navigation_service.rs where root_container has container_type: None).
  let root_input = serde_json::json!({ "title": title });
  let root_payload =
      srs::run_srs_write(&["container", "create"], output, &root_input.to_string())?;
  let root_container_id = root_payload["container"]["containerId"]
      .as_str()
      .map(String::from)
      .ok_or_else(|| anyhow::anyhow!("root container create returned no containerId"))?;
  // Members: identity note + decision-log root record (navigation reads memberInstanceIds).
  srs_members_add(output, &root_container_id, &intent_id)?;
  srs_members_add(output, &root_container_id, &dl_root_id)?;
  // Identity is the structural root.
  srs_roots_add(output, &root_container_id, &intent_id)?;
  // Write the minimal manifest.container embed (containerId + identityInstanceId only).
  let mut repo_json: serde_json::Value =
      serde_json::from_str(&std::fs::read_to_string(out_path)?).context("re-read new repo")?;
  repo_json["manifest"]["container"] = serde_json::json!({
      "containerId": root_container_id,
      "identityInstanceId": intent_id,
  });
  std::fs::write(out_path, serde_json::to_string_pretty(&repo_json)?)?;
  ```

- [ ] In `crates/srs-gov/tests/flow.rs`, update the RFC-013 section of
  `repo_create_produces_valid_srsj` (lines 326–352). Replace the `rootInstanceIds`
  assertions with assertions that check the full container in the store:
  ```rust
  // RFC-013: a required root container is scaffolded
  let container_embed = &content["manifest"]["container"];
  assert!(container_embed.is_object(), "manifest.container missing");
  let identity = container_embed["identityInstanceId"].as_str().unwrap_or("");
  assert!(!identity.is_empty(), "container has no identityInstanceId");
  let container_id = container_embed["containerId"].as_str().unwrap_or("");
  assert!(!container_id.is_empty(), "container has no containerId");

  // identity must resolve in the instance index
  let index: std::collections::HashSet<&str> = content["manifest"]["instanceIndex"]
      .as_array()
      .unwrap()
      .iter()
      .filter_map(|e| e["instanceId"].as_str())
      .collect();
  assert!(index.contains(identity), "identity does not resolve in index");

  // full container must be saved to the store with memberInstanceIds
  let container_key = format!("containers/{container_id}.json");
  let full_container = &content["data"][&container_key];
  assert!(
      full_container.is_object(),
      "root container not found in data.containers"
  );
  let member_ids: Vec<&str> = full_container["memberInstanceIds"]
      .as_array()
      .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
      .unwrap_or_default();
  assert!(
      !member_ids.is_empty(),
      "root container memberInstanceIds is empty"
  );
  assert!(
      member_ids.contains(&identity),
      "identity not in root container memberInstanceIds"
  );
  // Root container must also contain at least one section (the decision-log root).
  assert!(
      member_ids.len() >= 2,
      "root container memberInstanceIds should contain identity + at least one section, got {:?}",
      member_ids
  );
  ```

- [ ] Add new test `repo_create_navigation_works` in `crates/srs-gov/tests/flow.rs`
  immediately after `repo_create_produces_valid_srsj`:
  ```rust
  #[test]
  fn repo_create_navigation_works() {
      let tmp = std::env::temp_dir().join(format!(
          "srs-gov-nav-test-{}.srsj",
          std::time::SystemTime::now()
              .duration_since(std::time::UNIX_EPOCH)
              .unwrap()
              .subsec_nanos()
      ));
      let path = tmp.to_string_lossy().into_owned();

      let gov = srs_gov_bin();
      let srs = srs_bin();

      // Create a governance repo
      let out = std::process::Command::new(&gov)
          .env("SRS_BIN", &srs)
          .args([
              "repo-create",
              "--output",
              &path,
              "--title",
              "Nav Test Governance",
          ])
          .output()
          .expect("run srs-gov repo-create");
      assert!(
          out.status.success(),
          "repo-create failed: {}",
          String::from_utf8_lossy(&out.stderr)
      );

      // srs repo navigation must succeed and return the intent note as identity
      // with the decision-log root as the single section.
      let nav_out = std::process::Command::new(&srs)
          .args(["repo", "navigation", "--repo", &path])
          .output()
          .expect("run srs repo navigation");
      assert!(
          nav_out.status.success(),
          "srs repo navigation failed: {}",
          String::from_utf8_lossy(&nav_out.stderr)
      );
      let nav: serde_json::Value =
          serde_json::from_slice(&nav_out.stdout).expect("navigation output is not JSON");
      assert_eq!(
          nav["ok"].as_bool(),
          Some(true),
          "navigation returned error: {}",
          nav
      );
      let payload = &nav["payload"];

      // identity must be the intent note (non-empty instanceId)
      let identity_id = payload["identity"]["instanceId"].as_str().unwrap_or("");
      assert!(!identity_id.is_empty(), "navigation identity instanceId is empty");

      // exactly one section: the decision-log root
      let sections = payload["sections"].as_array().expect("sections is not array");
      assert_eq!(sections.len(), 1, "expected 1 section, got {}", sections.len());

      // no diagnostics
      let diagnostics = payload["diagnostics"].as_array().expect("diagnostics not array");
      assert!(
          diagnostics.is_empty(),
          "navigation returned diagnostics: {:?}",
          diagnostics
      );

      std::fs::remove_file(&tmp).ok();
  }
  ```

#### Acceptance Criteria

- [ ] `srs-gov repo-create` completes without error
- [ ] The created `.srsj` has `manifest.container` with `containerId` and `identityInstanceId`
- [ ] The root container is present at `data["containers/{containerId}.json"]`
- [ ] `memberInstanceIds` on the root container contains at least 2 IDs (intent note + decision-log root), verified by `member_ids.len() >= 2`
- [ ] `srs repo navigation` returns `ok: true` on the created repo
- [ ] `navigation.identity.instanceId` is non-empty
- [ ] `navigation.sections` has exactly 1 entry (the decision-log root)
- [ ] `navigation.diagnostics` is empty
- [ ] `repo_create_produces_valid_srsj` passes with updated assertions
- [ ] `repo_create_navigation_works` passes
- [ ] No regressions in other flow tests

#### Testing

```bash
cargo test -p srs-gov
cargo clippy -p srs-gov -- -D warnings
```

Specific tests to verify:

- `repo_create_produces_valid_srsj` — RFC-013 conformance (updated assertions)
- `repo_create_navigation_works` — end-to-end navigation on a freshly created governance repo

#### Milestone gate

1. Verify all acceptance criteria above are met — check each checkbox.
2. Confirm `repo_create_produces_valid_srsj` and `repo_create_navigation_works` pass.
3. Run lint and tests:

```bash
cargo test -p srs-gov
cargo clippy -p srs-gov -- -D warnings
```

4. Update the plan file: mark completed task checkboxes `[x]` and acceptance criteria `[x]`.
5. Commit:

```bash
git commit
```

Do not proceed to the next phase until the milestone gate passes.

---

## Final Acceptance

All of the following must be true before this plan is closed:

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] CLI output format unchanged (integration tests pass)
- [ ] `cargo test --test payload_contracts` passes (no payload structs changed)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [ ] `srs repo navigation` succeeds on a repo created by `srs-gov repo-create`
- [ ] `repo_create_navigation_works` test passes

## Coordination Rules

- Worker keeps to `crates/srs-gov/` only.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of the phase:** verify all acceptance criteria, confirm planned tests exist and pass, update plan checkboxes, then commit.

## Assumptions

- `srs container create` persists the container to `containers/{id}.json` within the `.srsj`
  data blob and adds it to `manifest.containerIndex`. This is verified by the existing
  container service tests and the `create_navigation_repo` integration test fixture.
- `srs container members add <container_id> <instance_id>` updates `memberInstanceIds` on
  the stored container. This is verified by `container_service` unit tests.
- The `.srsj` data blob stores container data under flat key `containers/{id}.json` within
  the `data` top-level object. This is the `JsonStore` convention (verified in json_store.rs).
- No spec change is required. RFC-013 already specifies the root container requirement;
  this plan implements it correctly in srs-gov.
- The root container has no `containerType`. Domain-typed section containers
  (`decision_log`, etc.) carry a type; the structural root does not. This matches the
  unit test fixture in `repository_navigation_service.rs` (root container has
  `container_type: None`) and the `create_navigation_repo` integration test fixture.
  The `srs container create` service accepts input without `containerType` — the field
  is optional in the Container schema.
