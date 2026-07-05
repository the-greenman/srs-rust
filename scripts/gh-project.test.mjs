// Unit tests for the plain-label mirror set (#335). Hermetic — imports the pure
// helpers from gh-project.mjs; the module's main-guard means importing it does NOT
// run the CLI or shell out to `gh`. Run: `node --test scripts/`
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  MIRROR_LABELS, MIRROR_REPOS, labelCreateArgs,
  STATUS_LABEL_MAP, STATUS_MIRROR_LABELS, statusMirrorWant,
  planPromotions, PROMOTE_INTENT_LABEL, epicRank,
  bandTargets, SIZE_WEIGHT,
} from "./gh-project.mjs";

// The labels the scheduled cloud routines read/write. Missing any one hard-fails a
// `gh issue edit --add-label` in a repo that lacks it — the whole point of #335.
const REQUIRED = ["ready", "priority: P0", "priority: P1", "priority: P2", "status: in progress", "promote:ready"];

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

// Promotion pipeline: a REST-only judge marks unblocked issues `promote:ready`; the privileged
// `promote` command converts that intent to board Status=Ready. planPromotions is the pure core.
const row = (o) => ({ repo: "srs-rust", num: o.num ?? 1, key: `srs-rust#${o.num ?? 1}`, state: "OPEN", status: null, labels: [], ...o });

test("promote intent label is in the mirror set (so ensureLabels creates it everywhere)", () => {
  assert.equal(PROMOTE_INTENT_LABEL, "promote:ready");
  assert.ok(MIRROR_LABELS.map((l) => l.name).includes(PROMOTE_INTENT_LABEL), "intent label must be creatable");
  // It must NOT be a Status mirror label — the judge owns it, the mirror owns `ready`.
  assert.ok(!STATUS_MIRROR_LABELS.includes(PROMOTE_INTENT_LABEL), "intent must stay distinct from `ready`");
});

test("planPromotions ignores rows without the intent label", () => {
  const plan = planPromotions([row({ num: 1, labels: [] }), row({ num: 2, labels: ["ready"] })]);
  assert.equal(plan.length, 0);
});

test("planPromotions promotes only OPEN Backlog/unset issues", () => {
  const rows = [
    row({ num: 10, status: "Backlog", labels: [PROMOTE_INTENT_LABEL] }),
    row({ num: 11, status: null,      labels: [PROMOTE_INTENT_LABEL] }),
  ];
  const plan = planPromotions(rows);
  assert.deepEqual(plan.map((p) => [p.num, p.action, p.promote]), [
    [10, "promoted", true],
    [11, "promoted", true],
  ]);
});

test("planPromotions never demotes/re-opens: advanced, ready, and closed intents are just cleared", () => {
  const rows = [
    row({ num: 20, status: "In progress", labels: [PROMOTE_INTENT_LABEL] }),
    row({ num: 21, status: "In review",   labels: [PROMOTE_INTENT_LABEL] }),
    row({ num: 22, status: "Done",        labels: [PROMOTE_INTENT_LABEL] }),
    row({ num: 23, status: "Ready",       labels: [PROMOTE_INTENT_LABEL] }),
    row({ num: 24, status: "Backlog", state: "CLOSED", labels: [PROMOTE_INTENT_LABEL] }),
  ];
  const plan = planPromotions(rows);
  assert.ok(plan.every((p) => p.promote === false), "none of these may be promoted");
  assert.deepEqual(plan.map((p) => p.action),
    ["skip-advanced", "skip-advanced", "skip-advanced", "already-ready", "cleared-closed"]);
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

// bandTargets slices an ordered weight stream into N ~equal-effort bands, preserving order.
test("bandTargets fills equal-effort bands in order", () => {
  // 10 unit weights, 5 bands → 2 per band.
  assert.deepEqual(bandTargets([1, 1, 1, 1, 1, 1, 1, 1, 1, 1], 5), [0, 0, 1, 1, 2, 2, 3, 3, 4, 4]);
  // Heavier items fill a band sooner; order is never reordered, only grouped.
  const a = bandTargets([3, 1, 1, 3, 1, 1], 3); // total 10, target ~3.33
  assert.equal(a.length, 6);
  assert.deepEqual([...a].sort((x, y) => x - y), a, "band index is non-decreasing (order preserved)");
  assert.equal(a[0], 0);
  assert.ok(a[a.length - 1] <= 2, "never exceeds n-1 bands");
  // Fewer items than bands: each item in its own band, remainder empty.
  assert.deepEqual(bandTargets([1, 1], 5), [0, 1]);
});

test("SIZE_WEIGHT orders effort small < medium < large < xl", () => {
  assert.ok(SIZE_WEIGHT.small < SIZE_WEIGHT.medium);
  assert.ok(SIZE_WEIGHT.medium < SIZE_WEIGHT.large);
  assert.ok(SIZE_WEIGHT.large < SIZE_WEIGHT.xl);
});
