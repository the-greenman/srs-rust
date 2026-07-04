// Unit tests for the plain-label mirror set (#335). Hermetic — imports the pure
// helpers from gh-project.mjs; the module's main-guard means importing it does NOT
// run the CLI or shell out to `gh`. Run: `node --test scripts/`
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  MIRROR_LABELS, MIRROR_REPOS, labelCreateArgs,
  STATUS_LABEL_MAP, STATUS_MIRROR_LABELS, statusMirrorWant, epicRank,
} from "./gh-project.mjs";

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

test("Status→label mirror maps the two routine-relevant statuses to defined labels", () => {
  assert.equal(STATUS_LABEL_MAP["Ready"], "ready");
  assert.equal(STATUS_LABEL_MAP["In progress"], "status: in progress");
  const names = MIRROR_LABELS.map((l) => l.name);
  for (const l of STATUS_MIRROR_LABELS) assert.ok(names.includes(l), `status-mirror label not in set: ${l}`);
});

test("statusMirrorWant: only OPEN issues in a mirrored status carry a label", () => {
  assert.equal(statusMirrorWant({ state: "OPEN", status: "Ready" }), "ready");
  assert.equal(statusMirrorWant({ state: "OPEN", status: "In progress" }), "status: in progress");
  assert.equal(statusMirrorWant({ state: "OPEN", status: "Backlog" }), null);
  assert.equal(statusMirrorWant({ state: "OPEN", status: "Done" }), null);
  // A closed issue never carries a status-mirror label, even if its board Status lags.
  assert.equal(statusMirrorWant({ state: "CLOSED", status: "Ready" }), null);
});

// Epics (= releases) order the roadmap by Priority, then by issue number. The same
// ranking breaks diamond ties so a shared descendant is claimed by the higher-priority
// epic deterministically.
test("epicRank orders by Priority first, then issue number", () => {
  const p0b = { priority: "P0", num: 99 };
  const p0a = { priority: "P0", num: 30 };
  const p1  = { priority: "P1", num: 1 };
  const none = { priority: null, num: 2 };
  const sorted = [p1, none, p0b, p0a].sort((a, b) => epicRank(a) - epicRank(b));
  assert.deepEqual(sorted.map((e) => e.num), [30, 99, 1, 2]);
  assert.ok(epicRank(p0a) < epicRank(p0b), "same priority: lower # wins");
  assert.ok(epicRank(p1) < epicRank(none), "set priority beats unset");
});
