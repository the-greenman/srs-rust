// Unit tests for the plain-label mirror set (#335). Hermetic — imports the pure
// helpers from gh-project.mjs; the module's main-guard means importing it does NOT
// run the CLI or shell out to `gh`. Run: `node --test scripts/gh-project.test.mjs`
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  MIRROR_LABELS, MIRROR_REPOS, labelCreateArgs,
  STATUS_LABEL_MAP, STATUS_MIRROR_LABELS, statusMirrorWant,
  planPromotions, PROMOTE_INTENT_LABEL, epicRank, epicFeedRank, epicRoadmapSeq, startedEpics,
  bandTargets, SIZE_WEIGHT, derivePriority, parseIssueRef, MOSCOW_DEFAULT, EPIC_PRIORITY_DEFAULT,
  planStaleClaims, STALE_CLAIM_HOURS_DEFAULT,
  planTopup, TOPUP_TARGET_DEFAULT,
  isWorkItem, isConsumableReady, countConsumableReady,
  hasOpenBlockers, isBlocked, blockedLabelWant,
} from "./gh-project.mjs";

// Blocking via native blocked-by dependencies (#671). Ownership rule: edges present
// ⇒ the `blocked` label is derived (tool-owned); no edges ⇒ human-owned, left alone.
const brow = (o = {}) => ({ repo: "srs-rust", num: 1, key: "srs-rust#1", state: "OPEN", status: "Backlog", labels: [], blockedBy: [], ...o });

test("isBlocked: an open blocker edge blocks, regardless of the label", () => {
  assert.equal(isBlocked(brow({ blockedBy: [{ key: "srs#10", state: "OPEN" }] })), true);
  assert.equal(isBlocked(brow({ blockedBy: [{ key: "srs#10", state: "OPEN" }], labels: [] })), true);
});

test("isBlocked: edges are authoritative — all blockers closed unblocks even with a stale label", () => {
  // Auto-unblock: the feed must not wait for the label mirror to catch up.
  assert.equal(isBlocked(brow({ blockedBy: [{ key: "srs#10", state: "CLOSED" }], labels: ["blocked"] })), false);
});

test("isBlocked: no edges — the hand-set label governs (external, non-issue blocks)", () => {
  assert.equal(isBlocked(brow({ labels: ["blocked"] })), true);
  assert.equal(isBlocked(brow({ labels: [] })), false);
  assert.equal(isBlocked(brow({ blockedBy: undefined, labels: ["blocked"] })), true); // rows without the field (tests/fixtures)
});

test("isBlocked: mixed blockers — one still-open blocker keeps it blocked", () => {
  assert.equal(isBlocked(brow({ blockedBy: [{ key: "srs#10", state: "CLOSED" }, { key: "srs-web#5", state: "OPEN" }] })), true);
});

test("blockedLabelWant: derives only when edges exist; null means leave the label alone", () => {
  assert.equal(blockedLabelWant(brow()), null);                                                    // no edges → human-owned
  assert.equal(blockedLabelWant(brow({ blockedBy: [{ key: "srs#10", state: "OPEN" }] })), true);    // set it
  assert.equal(blockedLabelWant(brow({ blockedBy: [{ key: "srs#10", state: "CLOSED" }] })), false); // clear it
});

test("hasOpenBlockers tolerates rows without the blockedBy field", () => {
  assert.equal(hasOpenBlockers({ labels: [] }), false);
});

// The labels the scheduled cloud routines read/write. Missing any one hard-fails a
// `gh issue edit --add-label` in a repo that lacks it — the whole point of #335.
const REQUIRED = ["ready", "priority: P0", "priority: P1", "priority: P2", "status: in progress", "promote:ready", "blocked"];

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
  for (const r of ["srs", "srs-rust", "srs-web", "srs-vscode"]) assert.ok(MIRROR_REPOS.includes(r), `missing repo: ${r}`);
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

test("planPromotions promotes an OFF-board intent (status null) — the judge may label an issue not yet on the board", () => {
  // Regression: `promote` must handle issues discovered by label search that aren't project items
  // yet (status null). These get promoted (ensureOnBoard adds them when Status=Ready is set).
  const plan = planPromotions([row({ num: 30, status: null, labels: [PROMOTE_INTENT_LABEL] })]);
  assert.deepEqual(plan.map((p) => [p.action, p.promote]), [["promoted", true]]);
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

// Epic continuity (#664): the WORK FEED drains a started epic before opening the next
// one in the same Priority tier — the board must not fill with half-finished epics.
test("epicFeedRank: a started epic beats an untouched one in the same tier", () => {
  const startedEpic = { priority: "P1", num: 90 };
  const untouched = { priority: "P1", num: 10 };
  const started = new Set([90]);
  assert.ok(epicFeedRank(startedEpic, started) < epicFeedRank(untouched, started),
    "started wins within the tier even with a higher issue number");
});

test("epicFeedRank: Priority tier still dominates — a started P1 epic never beats an untouched P0", () => {
  const startedP1 = { priority: "P1", num: 1 };
  const untouchedP0 = { priority: "P0", num: 99 };
  const started = new Set([1]);
  assert.ok(epicFeedRank(untouchedP0, started) < epicFeedRank(startedP1, started));
});

test("epicFeedRank: neither started — falls back to issue number within the tier", () => {
  const a = { priority: "P2", num: 5 };
  const b = { priority: "P2", num: 6 };
  assert.ok(epicFeedRank(a, new Set()) < epicFeedRank(b, new Set()));
});

// Roadmap prefix (#711): the "NN" the owner hand-maintains on Release names and
// "Epic NN:" titles is the intended epic sequence — issue numbers are filing
// chronology and get it wrong.
test("epicRoadmapSeq: reads the Release-name prefix, falls back to the title prefix", () => {
  assert.equal(epicRoadmapSeq({ release: "04 Generic Semantic Editor", title: "whatever" }), 4);
  assert.equal(epicRoadmapSeq({ release: null, title: "Epic 07: Offline editor" }), 7);
  assert.equal(epicRoadmapSeq({ release: "12 SRS VS Code Extension", title: "SRS VS Code Extension" }), 12);
  // Release wins over a conflicting title prefix (Release is the curated identity).
  assert.equal(epicRoadmapSeq({ release: "03 Workflow editor", title: "Epic 99: old name" }), 3);
  assert.equal(epicRoadmapSeq({ release: null, title: "SRS VS Code Extension" }), null);
});

test("epicFeedRank: roadmap prefix beats issue number — the real Epic 07/08 inversion", () => {
  // Epic 08 is issue #60, Epic 07 is issue #94: number order would run 08 before 07.
  const epic08 = { priority: "P2", num: 60, release: "08 AI Assisted Decision log" };
  const epic07 = { priority: "P2", num: 94, release: "07 Offline editor" };
  assert.ok(epicFeedRank(epic07, new Set()) < epicFeedRank(epic08, new Set()),
    "roadmap 07 must precede roadmap 08 regardless of issue numbers");
});

test("epicFeedRank: an unnumbered epic sorts after numbered ones in its tier", () => {
  const numbered = { priority: "P2", num: 999, release: "11 Snapshot export" };
  const unnumbered = { priority: "P2", num: 1, release: null, title: "Some epic" };
  assert.ok(epicFeedRank(numbered, new Set()) < epicFeedRank(unnumbered, new Set()));
});

test("epicFeedRank: started still beats a lower roadmap prefix within the tier", () => {
  // Continuity outranks sequence: a begun epic is drained before an earlier-numbered
  // untouched one opens.
  const startedLater = { priority: "P1", num: 83, release: "04 Generic Semantic Editor" };
  const untouchedEarlier = { priority: "P1", num: 76, release: "03 Workflow editor" };
  const started = new Set([83]);
  assert.ok(epicFeedRank(startedLater, started) < epicFeedRank(untouchedEarlier, started));
});

test("startedEpics: claimed/in-review/done/closed descendants start an epic; Ready does not", () => {
  const desc = new Map([
    ["srs-rust#1", 100], // In progress → starts 100
    ["srs-rust#2", 200], // Ready → queued is not begun
    ["srs-rust#3", 300], // CLOSED → starts 300
    ["srs-rust#4", 400], // not on the board → unknown, does not start
  ]);
  const b = new Map([
    ["srs-rust#1", { state: "OPEN", status: "In progress" }],
    ["srs-rust#2", { state: "OPEN", status: "Ready" }],
    ["srs-rust#3", { state: "CLOSED", status: "Done" }],
  ]);
  assert.deepEqual([...startedEpics(desc, b)].sort(), [100, 300]);
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

// derivePriority is the pure rollup core: stories → base, epic fallback, bug floor, bump.
const prow = (labels = []) => ({ repo: "srs-rust", num: 1, key: "srs-rust#1", labels });
const stories = new Map([[21, { moscow: "Must" }], [22, { moscow: "Could" }]]);

test("derivePriority: story value wins — epic fallback never fires when a story serves", () => {
  const { p, stages } = derivePriority(prow(), new Set([21, 22]), stories, { num: 30, priority: "P2" });
  assert.equal(p, "P0"); // highest served story (Must) — not degraded by the P2 epic
  assert.equal(stages.kind, "story-derived");
  assert.equal(stages.epicFallback.applied, false);
});

test("derivePriority: epic fallback inherits epic Priority one tier down, floored at P2", () => {
  for (const [epicP, want] of [["P0", "P1"], ["P1", "P2"], ["P2", "P2"]]) {
    const { p, stages } = derivePriority(prow(), null, stories, { num: 30, priority: epicP });
    assert.equal(p, want, `epic ${epicP} → ${want}`);
    assert.equal(stages.kind, "epic-derived");
    assert.equal(stages.epicFallback.applied, true);
  }
});

// Blank-means-Could (#664): a blank must never make an issue invisible to the feed.
// Only an explicit Won't on every served story excludes.

test("derivePriority: blank story MoSCoW defaults to Could → P2 (never null)", () => {
  const blankStories = new Map([[40, { moscow: null }]]);
  const { p, stages } = derivePriority(prow(), new Set([40]), blankStories, null);
  assert.equal(p, "P2");
  assert.equal(stages.kind, "story-derived");
  assert.equal(stages.storyValues[0].defaulted, true);
  assert.equal(MOSCOW_DEFAULT, "Could");
});

test("derivePriority: a story missing from the story map (unlabelled/off-list) still defaults to Could", () => {
  const { p } = derivePriority(prow(), new Set([999]), new Map(), null);
  assert.equal(p, "P2");
});

test("derivePriority: epic without a Priority now counts as P2 — its work still flows", () => {
  const { p, stages } = derivePriority(prow(), null, stories, { num: 30, priority: null });
  assert.equal(p, "P2"); // blank epic Priority ⇒ P2, one tier down ⇒ still P2
  assert.equal(stages.kind, "epic-derived");
  assert.equal(stages.epicFallback.applied, true);
  assert.equal(stages.epicFallback.defaulted, true);
  assert.equal(EPIC_PRIORITY_DEFAULT, "P2");
});

test("derivePriority: no story, no epic — orphaned but P2-floored (flagged, never lost)", () => {
  const { p, stages } = derivePriority(prow(), null, stories, null);
  assert.equal(p, "P2"); // default floor: the orphan enters the feed at the bottom
  assert.equal(stages.kind, "orphaned"); // …but stays flagged for linking hygiene
  assert.equal(stages.defaultFloor.applied, true);
});

test("derivePriority: explicit Won't on every served story excludes — no fallback, no floor", () => {
  const wontStories = new Map([[50, { moscow: "Won't" }]]);
  // Even under a P0 epic, an explicit Won't is the one deliberate opt-out.
  const { p, stages } = derivePriority(prow(), new Set([50]), wontStories, { num: 30, priority: "P0" });
  assert.equal(p, null);
  assert.equal(stages.wontExcluded, true);
  assert.equal(stages.epicFallback.applied, false);
  assert.equal(stages.defaultFloor.applied, false);
});

test("derivePriority: Won't mixed with a blank story — the blank's Could default wins", () => {
  const mixed = new Map([[50, { moscow: "Won't" }], [51, { moscow: null }]]);
  const { p, stages } = derivePriority(prow(), new Set([50, 51]), mixed, null);
  assert.equal(p, "P2");
  assert.equal(stages.wontExcluded, false);
});

test("derivePriority: a bug under a Won't story keeps the P1 floor — bugs are never lost", () => {
  const wontStories = new Map([[50, { moscow: "Won't" }]]);
  const { p } = derivePriority(prow(["bug"]), new Set([50]), wontStories, null);
  assert.equal(p, "P1");
});

test("derivePriority: bump applies on top of the orphan floor (P2 → P1)", () => {
  const { p, stages } = derivePriority(prow(["critical-path"]), null, stories, null);
  assert.equal(stages.defaultFloor.applied, true);
  assert.equal(p, "P1");
});

test("derivePriority: bug floor still applies on top of the epic fallback", () => {
  // bug under no epic → P1 floor
  const noEpic = derivePriority(prow(["bug"]), null, stories, null);
  assert.equal(noEpic.p, "P1");
  assert.equal(noEpic.stages.kind, "bug-floor");
  // bug under a P2 epic: fallback P2, floor raises to P1
  const underEpic = derivePriority(prow(["bug"]), null, stories, { num: 30, priority: "P2" });
  assert.equal(underEpic.p, "P1");
  assert.equal(underEpic.stages.bugFloor.applied, true);
});

test("derivePriority: bump raises an epic-derived issue back up one tier", () => {
  const { p, stages } = derivePriority(prow(["critical-path"]), null, stories, { num: 30, priority: "P0" });
  assert.equal(stages.epicFallback.to, "P1");
  assert.equal(p, "P0"); // bump undoes the discount for gate-blocking work
});

test("parseIssueRef parses repo#num (dots and dashes in repo names) and rejects junk", () => {
  assert.deepEqual(parseIssueRef("muDemocracy.org#48"), { repo: "muDemocracy.org", num: "48" });
  assert.deepEqual(parseIssueRef("srs-web#116"), { repo: "srs-web", num: "116" });
  assert.equal(parseIssueRef("116"), null);
  assert.equal(parseIssueRef("srs-web#"), null);
  assert.equal(parseIssueRef(undefined), null);
});

test("SIZE_WEIGHT orders effort small < medium < large < xl", () => {
  assert.ok(SIZE_WEIGHT.small < SIZE_WEIGHT.medium);
  assert.ok(SIZE_WEIGHT.medium < SIZE_WEIGHT.large);
  assert.ok(SIZE_WEIGHT.large < SIZE_WEIGHT.xl);
});

// Stale-claim recovery (the missing half of the promotion pipeline): nothing else ever revisits an
// `In progress` issue, so a claim whose holder crashed/timed out would sit invisible forever without
// this planner. planStaleClaims is the pure core; claimedAt()'s REST lookup is not unit-tested here.
const HOUR = 3600 * 1000;
const srow = (o) => ({ repo: "srs-rust", num: o.num ?? 1, key: `srs-rust#${o.num ?? 1}`, itemId: `item-${o.num ?? 1}`, state: "OPEN", status: "In progress", claimedAtMs: null, ...o });

test("STALE_CLAIM_HOURS_DEFAULT is a sane default (long enough for real work, same-day recovery)", () => {
  assert.ok(STALE_CLAIM_HOURS_DEFAULT >= 1 && STALE_CLAIM_HOURS_DEFAULT <= 48);
});

test("planStaleClaims ignores anything not OPEN + In progress", () => {
  const rows = [
    srow({ num: 1, state: "CLOSED", claimedAtMs: 0 }),
    srow({ num: 2, status: "Ready", claimedAtMs: 0 }),
    srow({ num: 3, status: "Backlog", claimedAtMs: 0 }),
    srow({ num: 4, status: null, claimedAtMs: 0 }),
  ];
  assert.deepEqual(planStaleClaims(rows, 100 * HOUR, 24 * HOUR), []);
});

test("planStaleClaims reclaims a claim older than the threshold, leaves a fresh one alone", () => {
  const now = 100 * HOUR;
  const rows = [
    srow({ num: 10, claimedAtMs: now - 48 * HOUR }), // well past a 24h threshold
    srow({ num: 11, claimedAtMs: now - 1 * HOUR }),  // just claimed
  ];
  const plan = planStaleClaims(rows, now, 24 * HOUR);
  assert.deepEqual(plan.map((p) => [p.num, p.action]), [[10, "reclaim"], [11, "fresh"]]);
});

test("planStaleClaims treats age exactly at the threshold as stale (>=, not >)", () => {
  const now = 100 * HOUR;
  const rows = [srow({ num: 20, claimedAtMs: now - 24 * HOUR })];
  const plan = planStaleClaims(rows, now, 24 * HOUR);
  assert.equal(plan[0].action, "reclaim");
});

test("planStaleClaims reports unresolvable claim times as unknown, never silently drops or reclaims them", () => {
  const rows = [srow({ num: 30, claimedAtMs: null })];
  const plan = planStaleClaims(rows, 100 * HOUR, 24 * HOUR);
  assert.deepEqual(plan.map((p) => [p.num, p.action]), [[30, "unknown"]]);
});

test("planStaleClaims carries itemId through so the caller can write the board field without a second lookup", () => {
  const rows = [srow({ num: 40, itemId: "PVTI_abc", claimedAtMs: 0 })];
  const plan = planStaleClaims(rows, 100 * HOUR, 24 * HOUR);
  assert.equal(plan[0].itemId, "PVTI_abc");
});

// Auto-topup: keeps the Ready queue at a target depth by writing `promote:ready` to the
// highest-priority unblocked Backlog leaves. planTopup is the pure core — no filtering, no
// side effects; the caller pre-filters and pre-sorts candidates before passing them in.
const trow = (o) => ({ repo: "srs-rust", num: o.num ?? 1, key: `srs-rust#${o.num ?? 1}`, ...o });

test("TOPUP_TARGET_DEFAULT is a positive integer", () => {
  assert.ok(Number.isInteger(TOPUP_TARGET_DEFAULT) && TOPUP_TARGET_DEFAULT > 0,
    "default target must be a positive integer");
});

test("planTopup returns empty toNominate when queue is already at target", () => {
  const candidates = [trow({ num: 1 }), trow({ num: 2 })];
  const result = planTopup(candidates, 3, 3);
  assert.deepEqual(result.toNominate, []);
  assert.equal(result.deficit, 0);
  assert.equal(result.currentReady, 3);
  assert.equal(result.target, 3);
});

test("planTopup nominates up to deficit rows in order", () => {
  const candidates = [trow({ num: 10 }), trow({ num: 20 }), trow({ num: 30 }), trow({ num: 40 }), trow({ num: 50 })];
  const result = planTopup(candidates, 1, 3); // deficit = 2
  assert.equal(result.deficit, 2);
  assert.equal(result.toNominate.length, 2);
  assert.deepEqual(result.toNominate.map((r) => r.num), [10, 20]); // first two in order
});

test("planTopup clamps when fewer candidates than deficit", () => {
  const candidates = [trow({ num: 1 }), trow({ num: 2 })];
  const result = planTopup(candidates, 0, 5); // deficit = 5, only 2 available
  assert.equal(result.deficit, 5);
  assert.equal(result.toNominate.length, 2); // all available, no error
});

test("planTopup returns correct metadata fields", () => {
  const result = planTopup([trow({ num: 1 })], 2, 4);
  assert.equal(result.deficit, 2);
  assert.equal(result.currentReady, 2);
  assert.equal(result.target, 4);
  assert.equal(result.toNominate.length, 1);
});

test("planTopup returns empty toNominate when queue exceeds target (overprovisioned)", () => {
  // If ready count is already above target, deficit is 0 and nothing is nominated.
  const candidates = [trow({ num: 1 })];
  const result = planTopup(candidates, 5, 3);
  assert.deepEqual(result.toNominate, []);
  assert.equal(result.deficit, 0);
});

// countConsumableReady is the queue-depth measure topup uses. It must match exactly what the hourly
// consumer can pick up — else topup mis-measures. Regression: it once counted stories/epics/claimed
// items, so a queue of 5 stories + 1 epic + 2 claimed read "full" (8) while 0 consumable work
// remained, and topup never refilled (and merges never rehydrated the queue).
const crow = (o) => ({ repo: "srs-rust", num: o.num ?? 1, key: `srs-rust#${o.num ?? 1}`, state: "OPEN", status: null, labels: [], ...o });

test("isConsumableReady: a plain Ready work-item counts", () => {
  assert.equal(isConsumableReady(crow({ status: "Ready" })), true);
});

test("isConsumableReady: a Backlog item carrying promote:ready counts (about to be Ready)", () => {
  assert.equal(isConsumableReady(crow({ status: "Backlog", labels: ["promote:ready"] })), true);
});

test("isConsumableReady: epics, stories, and plans never count even when Ready", () => {
  assert.equal(isConsumableReady(crow({ status: "Ready", labels: ["epic"] })), false);
  assert.equal(isConsumableReady(crow({ status: "Ready", labels: ["user-story"] })), false);
  assert.equal(isConsumableReady(crow({ status: "Ready", labels: ["plan"] })), false);
});

test("isConsumableReady: an already-claimed (status: in progress) Ready item does not count", () => {
  assert.equal(isConsumableReady(crow({ status: "Ready", labels: ["status: in progress"] })), false);
});

test("isConsumableReady: CLOSED and non-Ready items do not count", () => {
  assert.equal(isConsumableReady(crow({ status: "Ready", state: "CLOSED" })), false);
  assert.equal(isConsumableReady(crow({ status: "Backlog" })), false);
});

test("countConsumableReady mirrors the real drain scenario: 5 stories + 1 epic + 2 claimed = 0 consumable", () => {
  const rows = [
    crow({ num: 1, status: "Ready", labels: ["user-story"] }),
    crow({ num: 2, status: "Ready", labels: ["user-story"] }),
    crow({ num: 3, status: "Ready", labels: ["user-story"] }),
    crow({ num: 4, status: "Ready", labels: ["user-story"] }),
    crow({ num: 5, status: "Ready", labels: ["user-story"] }),
    crow({ num: 6, status: "Ready", labels: ["plan", "epic"] }),
    crow({ num: 7, status: "Ready", labels: ["ready", "status: in progress"] }),
    crow({ num: 8, status: "Ready", labels: ["ready", "status: in progress"] }),
  ];
  assert.equal(countConsumableReady(rows), 0);
});

test("countConsumableReady counts only the genuinely pickable leaves in a mixed board", () => {
  const rows = [
    crow({ num: 1, status: "Ready" }),                                  // ✓
    crow({ num: 2, status: "Backlog", labels: ["promote:ready"] }),     // ✓ intent
    crow({ num: 3, status: "Ready", labels: ["user-story"] }),          // ✗ story
    crow({ num: 4, status: "Ready", labels: ["status: in progress"] }), // ✗ claimed
    crow({ num: 5, status: "Backlog" }),                                // ✗ not ready
  ];
  assert.equal(countConsumableReady(rows), 2);
});

// D3: a claim is the `status: in progress` LABEL (routines can't set board Status), so stale-claims
// must recover a zombie claim whose board Status never advanced past "Ready".
test("planStaleClaims recovers a label-only claim (board still Ready) that is stale", () => {
  const now = 100 * HOUR;
  const rows = [
    srow({ num: 50, status: "Ready", labels: ["ready", "status: in progress"], claimedAtMs: now - 48 * HOUR }),
  ];
  const plan = planStaleClaims(rows, now, 24 * HOUR);
  assert.deepEqual(plan.map((p) => [p.num, p.action]), [[50, "reclaim"]]);
});

test("planStaleClaims ignores a Ready item that is NOT claimed (no in-progress label)", () => {
  const rows = [srow({ num: 51, status: "Ready", labels: ["ready"], claimedAtMs: 0 })];
  assert.deepEqual(planStaleClaims(rows, 100 * HOUR, 24 * HOUR), []);
});
