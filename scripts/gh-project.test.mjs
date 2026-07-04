// Unit tests for the plain-label mirror set (#335). Hermetic — imports the pure
// helpers from gh-project.mjs; the module's main-guard means importing it does NOT
// run the CLI or shell out to `gh`. Run: `node --test scripts/`
import { test } from "node:test";
import assert from "node:assert/strict";
import { MIRROR_LABELS, MIRROR_REPOS, labelCreateArgs } from "./gh-project.mjs";

// The labels the scheduled cloud routines read/write. Missing any one hard-fails a
// `gh issue edit --add-label` in a repo that lacks it — the whole point of #335.
const REQUIRED = ["ready", "priority: P0", "priority: P1", "priority: P2", "status: in progress"];

test("mirror set covers every label the routines read/write", () => {
  const names = MIRROR_LABELS.map((l) => l.name);
  for (const req of REQUIRED) assert.ok(names.includes(req), `missing mirror label: ${req}`);
});

test("every mirror label has a 6-hex color and a description", () => {
  for (const l of MIRROR_LABELS) {
    assert.match(l.color, /^[0-9A-Fa-f]{6}$/, `${l.name} color`);
    assert.ok(l.description && l.description.length > 0, `${l.name} description`);
  }
});

test("labelCreateArgs is idempotent (--force) and repo-scoped", () => {
  const spec = MIRROR_LABELS.find((l) => l.name === "ready");
  const args = labelCreateArgs("srs-web", spec);
  assert.deepEqual(args.slice(0, 3), ["label", "create", "ready"]);
  assert.ok(args.includes("--force"), "must pass --force to be idempotent");
  const ri = args.indexOf("--repo");
  assert.equal(args[ri + 1], "the-greenman/srs-web");
  assert.equal(args[args.indexOf("--color") + 1], spec.color);
});

test("MIRROR_REPOS covers the routine-touched ecosystem repos", () => {
  for (const r of ["srs", "srs-rust", "srs-web"]) assert.ok(MIRROR_REPOS.includes(r), `missing repo: ${r}`);
  assert.ok(MIRROR_REPOS.some((r) => r.toLowerCase() === "mudemocracy.org"), "missing story repo");
});
