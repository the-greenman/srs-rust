# Plan: Format-driven theme auto-selection in render_service (#121)

## Summary

`resolve_active_theme` in `render_service.rs` skips theming with diagnostic `[T-2]` when the
document view's `themeRef` theme doesn't target the requested format. This causes the srs-web
blueprint editor HTML preview to render raw field labels (`item-term`, `Items`) instead of clean
prose, because the `guide-prose` theme (targets: `["markdown"]`) is skipped for format `html`.
The `DocumentView` model already has `themeVariants[]` for exactly this purpose. This plan wires
auto-selection: when the default theme doesn't target the format, try the first `themeVariant`
whose theme does. No binding signature changes needed — srs-web already passes `format = "html"`.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Repository Service Worker | Phase 1 (render_service) |
| Verification | Phase 1 gate, final acceptance |

No new agent roles needed.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-001](../docs/adr/001-library-first-architecture.md) | Theme selection logic lives in `srs-repository::render_service`, not in `srs-cli` or `srs-bindings` | accepted |
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | `resolve_active_theme` is a service-level private function — no handler changes | accepted |
| [ADR-013](../docs/adr/013-wasm-binding-strategy.md) | No binding signature change: format-driven selection is transparent to callers; WASM already passes `format` | accepted |

No new ADRs required: this plan implements existing ADR-001/010/013 patterns without introducing new architectural constraints.

---

## Contracts

### CLI output contract (ADR-011)

No new or changed CLI commands, handlers, or payload structs. `render document-view` payload shape
is unchanged (`rendered`, `diagnostics`, `projection`). No schema regeneration needed. `cargo test
--test payload_contracts` must still pass.

### Entity schema sync

No entity schemas changed. `bash scripts/check-schema-sync.sh` must still exit 0.

---

## Scope

- Modify `resolve_active_theme` in `crates/srs-repository/src/render_service.rs` to auto-select a
  format-matching `themeVariant` when the default `themeRef` doesn't target the requested format.
- Add regression tests in `render_service.rs` (inline `#[cfg(test)]` module).
- Author the `guide-prose-html` theme and add `themeVariants` entry to the `guide-body-view`
  document view in `~/dev/muDemocracy.org/muSrs` **using the `srs` CLI** (`srs theme create` +
  `srs document-view update`).
- Rebuild the WASM binary (`wasm-pack build`) and re-vendor into `srs-web/src/lib/srs_bindings/`.
- Regenerate the srs-web e2e fixture (`muSrs.srsj`) from the updated muSrs repo so both themes
  and the `themeVariants` entry are present.

**Out of scope:**
- Adding `targets` info to the `theme list` CLI payload (touches `payload.rs` + schema regen; file
  a separate enhancement issue).
- Any `section.commentary` (`commentary-term`/`commentary-body`) themed templates — only what
  `guide-prose` covers.
- srs-web JS changes (no `renderDocumentView` signature change needed).
- `ext:views-l2` spec text changes (no spec update required).

---

## Phases

### Phase 1: Format-driven theme auto-selection in render_service

**Goal:** `resolve_active_theme` auto-selects a `themeVariant` whose theme targets the requested
format when the default `themeRef` theme doesn't.

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `crates/srs-repository/src/render_service.rs`, modify `resolve_active_theme` (currently
  at ~line 639). In the `theme_variant == None` branch: after resolving the `themeRef` theme, if
  it does NOT target `format`, iterate `dv.theme_variants` and find the **first** variant whose
  resolved theme has `targets` containing `format`. Skip any variant whose theme_ref cannot be
  resolved (missing themeId, theme not in package) silently — emit no diagnostic for skipped
  variants; just try the next one. Use the first successfully-resolved matching theme.
  If >1 variant matches, use the first and push a **distinct** diagnostic (not [T-2]):
  `"[T-3] view {id}: multiple themeVariants target format {format}; using first '{name}'"`.
  If 0 variants match (current behavior), push the existing `[T-2]` diagnostic and return `None`.
- [ ] Keep the explicit `--theme-variant` branch (line ~646) completely unchanged — named variants
  still take priority and the format check at line ~694 stays as-is for that path.
- [ ] Add tests in the `#[cfg(test)]` module at the bottom of `render_service.rs`, after the last
  existing test in the module (`theme_format_mismatch_skips_theme_and_emits_diagnostic` or
  whichever is last):
  - `auto_select_theme_variant_by_format` — html format with themeRef targeting markdown and one
    html-targeting themeVariant: verify the variant's templates are applied, no `[T-2]` diagnostic.
  - `auto_select_no_variant_match_emits_t2` — html format, themeRef targets markdown, no variants
    target html: verify `[T-2]` diagnostic and no theme applied.
  - `auto_select_multiple_variants_match_uses_first` — html format, two html-targeting variants:
    verify first is used and `[T-3]` ambiguity diagnostic is emitted.
  - `explicit_variant_overrides_auto_select` — explicit `theme_variant = Some("foo")` still takes
    the explicit-variant code path, unchanged.

#### Acceptance Criteria

- [ ] `auto_select_theme_variant_by_format` passes: html render with a html-targeted variant
  applies the variant theme (no raw field-label spans, no `[T-2]` diagnostic).
- [ ] `auto_select_no_variant_match_emits_t2` passes: `[T-2]` diagnostic fired, theme is None.
- [ ] `auto_select_multiple_variants_match_uses_first` passes: first variant used, `[T-3]` ambiguity
  diagnostic emitted.
- [ ] `explicit_variant_overrides_auto_select` passes: named-variant path unchanged.
- [ ] Existing tests `theme_variant_selection_uses_matching_variant` and
  `theme_variant_not_found_falls_back_to_theme_ref` still pass.
- [ ] `cargo clippy -p srs-repository -- -D warnings` passes.

#### Testing

```bash
cargo test -p srs-repository auto_select
cargo test -p srs-repository theme_variant
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write:
- `auto_select_theme_variant_by_format` — proves html auto-selects, no T-2 diagnostic
- `auto_select_no_variant_match_emits_t2` — proves fallback still fires T-2
- `auto_select_multiple_variants_match_uses_first` — proves first-match + ambiguity diagnostic
- `explicit_variant_overrides_auto_select` — proves named path unchanged

#### Milestone gate

1. All acceptance criteria checked.
2. Four new tests exist and pass; two existing tests pass.
3. Run:
   ```bash
   cargo test -p srs-repository
   cargo clippy -p srs-repository -- -D warnings
   ```
4. Update plan checkboxes.
5. Commit: `fix(render): auto-select themeVariant by format when themeRef doesn't match (#121)`

---

### Phase 2: Author guide-prose-html theme + document-view variant in muSrs

**Goal:** The muSrs file repo at `~/dev/muDemocracy.org/muSrs` has a `guide-prose-html` theme
(targets `["html"]`) and the `guide-body-view` document view has a `themeVariants` entry pointing
to it. All operations use the `srs` CLI only — no direct file edits.

**Agent:** Repository Service Worker (data authoring)

**Note on CLI vs direct edit:** `srs theme create` and `srs document-view update` are the
canonical write path. If any CLI command proves insufficient for this authoring task, register a
GitHub issue before falling back to direct file edit.

#### Tasks

- [ ] Verify the `srs theme create` and `srs document-view update` commands support the required
  fields. Run `srs theme create --help` and `srs document-view update --help` on the local CLI
  binary.
- [ ] Author the `guide-prose-html` theme via `srs theme create --repo ~/dev/muDemocracy.org/muSrs`
  with stdin JSON. The CLI auto-generates a UUID4 for the theme id; **capture the returned id**
  from the command output for use in the next step. The theme stdin JSON (omit `id` — CLI provides it):
  ```json
  {
    "namespace": "com.mudemocracy",
    "name": "guide-prose-html",
    "version": 1,
    "description": "HTML variant of guide-prose for the srs-web blueprint editor preview. Renders fields as HTML paragraphs — no label spans. Mirrors the markdown prose theme.",
    "targets": ["html"],
    "elementTemplates": {
      "fieldRow": "<p>{{field-value}}</p>\n",
      "groupFieldRowTemplates": {
        "item-term": "<p><strong>{{field-value}}</strong></p>\n",
        "item-body": "<p>{{field-value}}</p>\n"
      }
    }
  }
  ```
  Extract the new theme id: `NEW_THEME_ID=$(... | python3 -c "import json,sys; print(json.load(sys.stdin)['payload']['theme']['id'])")`.
- [ ] Add a `themeVariants` entry to the `guide-body-view` document view
  (`id: 2aba4d85-317b-44e1-a600-d38a743b4cb4`) via `srs document-view update`. Read the
  current value with `srs document-view get 2aba4d85-317b-44e1-a600-d38a743b4cb4`, add the
  `themeVariants` key using `$NEW_THEME_ID`, and pipe the full JSON back:
  ```json
  "themeVariants": [
    { "name": "html", "themeRef": { "mode": "bundled", "themeId": "<NEW_THEME_ID>" } }
  ]
  ```
  After update, verify the round-trip: `srs document-view get 2aba4d85... --repo muSrs` must show
  all original fields (`containerType`, `sections`, `themeRef`, `preamble`, `format`) **plus**
  the new `themeVariants` entry.
- [ ] Run `srs repo validate --repo ~/dev/muDemocracy.org/muSrs` — must show 0 errors.
- [ ] Smoke-test the html render:
  ```bash
  srs --repo ~/dev/muDemocracy.org/muSrs render document-view \
    --view 2aba4d85-317b-44e1-a600-d38a743b4cb4 \
    --container 1c843817-c0f9-4ba6-b65f-c6d23af161a7 \
    --view-format html
  ```
  Confirm output contains `<p><strong>` for item-term entries and **no** `<span class="field-label">item-term` or `<span class="field-label">item-body`.
- [ ] Confirm the markdown render is unchanged (no regressions):
  ```bash
  srs --repo ~/dev/muDemocracy.org/muSrs render document-view \
    --view 2aba4d85-317b-44e1-a600-d38a743b4cb4 \
    --container 1c843817-c0f9-4ba6-b65f-c6d23af161a7 \
    --view-format markdown
  ```
  Confirm output contains `**Assign a scribe**` (bold term), no literal HTML tags.

#### Acceptance Criteria

- [ ] `srs theme list --repo muSrs` shows `guide-prose` and `guide-prose-html`.
- [ ] `srs document-view get 2aba4d85 --repo muSrs` shows `themeVariants` with the html entry AND all original fields (`containerType`, `sections`, `themeRef`, `preamble`, `format`) still present.
- [ ] `srs repo validate --repo muSrs` → 0 errors.
- [ ] html render: no `<span class="field-label">item-term` or `<span class="field-label">item-body` in output.
- [ ] html render: `<p><strong>` appears for item-term values.
- [ ] markdown render: unchanged — `**Assign a scribe**` present, no HTML tags.
- [ ] No `[T-2]` diagnostic in html render output.

#### Testing

```bash
# HTML render smoke test
srs --repo ~/dev/muDemocracy.org/muSrs --store file render document-view \
  --view 2aba4d85-317b-44e1-a600-d38a743b4cb4 \
  --container 1c843817-c0f9-4ba6-b65f-c6d23af161a7 \
  --view-format html | python3 -c \
  "import json,sys; r=json.load(sys.stdin)['payload']['rendered']; \
   assert '<span class=\"field-label\">item-term' not in r, 'raw item-term label present'; \
   assert '<p><strong>' in r, 'expected bold term paragraph'; \
   print('HTML render: OK')"

# Markdown regression check
srs --repo ~/dev/muDemocracy.org/muSrs --store file render document-view \
  --view 2aba4d85-317b-44e1-a600-d38a743b4cb4 \
  --container 1c843817-c0f9-4ba6-b65f-c6d23af161a7 \
  --view-format markdown | python3 -c \
  "import json,sys; r=json.load(sys.stdin)['payload']['rendered']; \
   assert '**Assign a scribe**' in r, 'markdown bold term missing'; \
   assert '<p>' not in r, 'HTML leaked into markdown'; \
   print('Markdown render: OK')"
```

#### Milestone gate

1. All acceptance criteria checked.
2. Smoke tests pass.
3. Commit muSrs data changes in `~/dev/muDemocracy.org/` repo:
   `feat: add guide-prose-html theme + themeVariants for html preview (#121)`

---

### Phase 3: WASM rebuild + srs-web fixture update

**Goal:** The srs-web WASM binary contains the Phase 1 render logic fix; the e2e fixture
`muSrs.srsj` contains both themes and the document-view `themeVariants`.

**Agent:** Bindings Worker + Lead Integrator

#### Tasks

- [ ] Rebuild the WASM binary from `srs-rust` worktree:
  ```bash
  wasm-pack build crates/srs-bindings --target web \
    --out-dir ../../srs-web/src/lib/srs_bindings
  ```
  This regenerates `srs_bindings.js`, `srs_bindings.d.ts`, `srs_bindings_bg.wasm`,
  `srs_bindings_bg.wasm.d.ts` in `srs-web/src/lib/srs_bindings/`.
- [ ] Regenerate the srs-web e2e fixture from the updated muSrs file repo using `srs repo copy`:
  ```bash
  srs repo copy \
    --from ~/dev/muDemocracy.org/muSrs \
    --to /home/greenman/dev/semanticops/srs-web/e2e/fixtures/muSrs.srsj \
    --to-store json
  ```
  (`--from` / `--to` / `--to-store json` confirmed from `srs repo copy --help`.)
- [ ] Stage the updated `srs_bindings/*` and `e2e/fixtures/muSrs.srsj` files in the srs-web repo.
  Run `npm run typecheck` — must pass.

#### Acceptance Criteria

- [ ] `srs-web/src/lib/srs_bindings/srs_bindings_bg.wasm` is newer than the Phase 1 commit.
- [ ] `npm run typecheck` passes in srs-web.
- [ ] Source repo `~/dev/muDemocracy.org/muSrs/package/themes/` contains a `*prose-html*.json`
  file (the theme file authored in Phase 2).
- [ ] `e2e/fixtures/muSrs.srsj` document-view entry `package/document-views/guide-body-view-2aba4d85.json`
  contains `themeVariants` key.

#### Testing

```bash
cd /home/greenman/dev/semanticops/srs-web
npm run typecheck

# Verify source file repo has the new theme
ls ~/dev/muDemocracy.org/muSrs/package/themes/ | grep prose-html || echo "ERROR: prose-html theme missing"

# Verify srsj fixture has themeVariants in document-view
python3 -c "
import json
d=json.load(open('e2e/fixtures/muSrs.srsj'))
dv=d['data']['package/document-views/guide-body-view-2aba4d85.json']
assert 'themeVariants' in dv, 'themeVariants missing from document-view in fixture'
themes=[k for k in d['data'] if '/themes/' in k]
print('Theme files in fixture:', themes)
print('Fixture: OK')
"
```

#### Milestone gate

1. All acceptance criteria checked.
2. wasm-pack build succeeded (no compile errors).
3. `npm run typecheck` passes.
4. Fixture validation script passes.
5. Commit in srs-web: `fix(wasm,fixture): rebuild WASM with themed html render + update muSrs fixture (#121)`

---

## Final Acceptance

All of the following must be true before this plan is closed:

- [ ] `cargo test` passes with no failures (in srs-rust worktree)
- [ ] `cargo clippy -- -D warnings` passes
- [ ] CLI output format unchanged — `cargo test --test payload_contracts` passes
- [ ] `bash scripts/check-schema-sync.sh` exits 0
- [ ] srs-web `npm run typecheck` passes
- [ ] `srs repo validate --repo ~/dev/muDemocracy.org/muSrs` → 0 errors
- [ ] html render of `guide-body-view` (container `1c843817`) contains no `<span class="field-label">item-term` or `field-label>item-body`
- [ ] markdown render unchanged: bold terms (`**…**`), no HTML tags
- [ ] `e2e/fixtures/muSrs.srsj` contains `guide-prose-html` theme and `themeVariants` in document-view

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- At the end of each phase: verify all acceptance criteria, confirm planned tests exist and pass,
  update plan checkboxes, then commit. Do not proceed to the next phase without completing the
  milestone gate.
- **Use the `srs` CLI for all data authoring in Phase 2.** If any CLI command proves
  insufficient, file a GitHub issue before falling back to direct file edit.

## Assumptions

- `srs theme create` accepts stdin JSON with `namespace`, `name`, `version`, `targets`,
  `elementTemplates`, and `description` fields (mirrors the existing theme struct).
- `srs document-view update` accepts a full document-view JSON on stdin (same as `type update`
  pattern). If it only accepts partial updates, adapt accordingly.
- The muSrs file repo at `~/dev/muDemocracy.org/muSrs` is on a branch that can receive commits.
- `wasm-pack` is installed and on PATH.
