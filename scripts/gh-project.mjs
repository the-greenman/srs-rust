#!/usr/bin/env node
// gh-project — story-driven priority management for the SRS GitHub Project (#5).
//
// Single file, zero dependencies. Wraps the `gh` CLI (which must be installed and
// authenticated). Works inside an isolated single-repo checkout: every operation hits
// the GitHub API, so nothing here depends on a sibling repo being present on disk.
//
// Priority model: user stories (label `user-story`, in muDemocracy.org) carry a MoSCoW
// value on the board; implementation issues are their sub-issues (native GitHub
// sub-issues) and inherit a derived `priority: Pn` label (highest served story, bumped
// one tier if gate-blocking). Bugs floor at P1 even without a story; unlinked non-bug
// issues are flagged, never lost.
//
// Usage: node gh-project.mjs <command> [options]   (see `help`)

import { execFileSync } from "node:child_process";

// ---------------------------------------------------------------------------
// Configuration (overridable via env)
// ---------------------------------------------------------------------------
const OWNER = process.env.GHP_OWNER || "the-greenman";
const PROJECT_NUMBER = Number(process.env.GHP_PROJECT || 5);
const STORY_REPO = process.env.GHP_STORY_REPO || "muDemocracy.org";
const STORY_LABEL = "user-story";

const MOSCOW_TO_P = { Must: "P0", Should: "P1", Could: "P2", "Won't": null };
const P_ORDER = ["P0", "P1", "P2"]; // index 0 = highest
const pRank = (p) => { const i = P_ORDER.indexOf(p); return i < 0 ? 99 : i; }; // unset sorts last
const BUG_FLOOR = "P1"; // bugs are fixed ASAP even with no story
// Explicit, auditable bump signals (label names). A match bumps one tier (cap P0).
const BUMP_LABELS = new Set(["critical-path", "blocks-gate", "regression"]);

// ---------------------------------------------------------------------------
// Small shell / GraphQL helpers
// ---------------------------------------------------------------------------
function gh(args, { input } = {}) {
  return execFileSync("gh", args, {
    encoding: "utf8",
    input,
    maxBuffer: 64 * 1024 * 1024,
  });
}

function ghJson(args, opts) {
  const out = gh(args, opts).trim();
  return out ? JSON.parse(out) : null;
}

// Run a GraphQL query. `vars` values may be string|number|boolean; numbers/bools use -F.
function graphql(query, vars = {}) {
  const args = ["api", "graphql", "-f", `query=${query}`];
  for (const [k, v] of Object.entries(vars)) {
    if (v === null || v === undefined) continue;
    args.push(typeof v === "string" ? "-f" : "-F", `${k}=${v}`);
  }
  const res = ghJson(args); // gh wraps GraphQL responses in { data, errors }
  return res?.data ?? res;
}

const die = (msg) => {
  console.error(`gh-project: ${msg}`);
  process.exit(1);
};

// ---------------------------------------------------------------------------
// Project metadata (discovered, not hardcoded) — cached for the process
// ---------------------------------------------------------------------------
let _meta = null;
function meta() {
  if (_meta) return _meta;
  const projFields = `projectV2(number:$number){
      id title
      fields(first:50){ nodes{
        __typename
        ... on ProjectV2FieldCommon { id name }
        ... on ProjectV2SingleSelectField { id name options { id name } }
        ... on ProjectV2IterationField { id name
          configuration { iterations { id title } completedIterations { id title } } }
      } }
    }`;
  // Owner may be a user or an organization. Query separately so the wrong kind
  // (which errors and makes `gh` exit non-zero) never breaks the right one.
  const ask = (kind) =>
    graphql(`query($owner:String!,$number:Int!){ ${kind}(login:$owner){ ${projFields} } }`,
      { owner: OWNER, number: PROJECT_NUMBER });
  let proj = null;
  try { proj = ask("user")?.user?.projectV2 ?? null; } catch { /* not a user */ }
  if (!proj) { try { proj = ask("organization")?.organization?.projectV2 ?? null; } catch { /* not an org */ } }
  if (!proj) die(`project #${PROJECT_NUMBER} not found for ${OWNER}`);
  const fields = {};
  for (const f of proj.fields.nodes) fields[f.name] = f;
  _meta = { id: proj.id, title: proj.title, fields };
  return _meta;
}

function field(name) {
  const f = meta().fields[name];
  if (!f) die(`project field "${name}" not found`);
  return f;
}

function optionId(fieldName, optionName) {
  const f = field(fieldName);
  const o = (f.options || []).find(
    (x) => x.name.toLowerCase() === optionName.toLowerCase()
  );
  if (!o) die(`option "${optionName}" not found on field "${fieldName}"`);
  return o.id;
}

function iterationId(title) {
  const cfg = field("Iteration").configuration;
  const all = [...cfg.iterations, ...cfg.completedIterations];
  const it = all.find((x) => x.title.toLowerCase() === title.toLowerCase());
  if (!it) die(`iteration "${title}" not found (iterations are UI-only)`);
  return it.id;
}

// ---------------------------------------------------------------------------
// Board read (correct pagination, deduped)
// ---------------------------------------------------------------------------
let _board = null;
function board() {
  if (_board) return _board;
  const q = `query($owner:String!,$number:Int!,$endCursor:String){
    user(login:$owner){ projectV2(number:$number){ items(first:100, after:$endCursor){
      pageInfo{ hasNextPage endCursor }
      nodes{
        id
        status:   fieldValueByName(name:"Status")   { ... on ProjectV2ItemFieldSingleSelectValue { name } }
        priority: fieldValueByName(name:"Priority")  { ... on ProjectV2ItemFieldSingleSelectValue { name } }
        moscow:   fieldValueByName(name:"MoSCoW")    { ... on ProjectV2ItemFieldSingleSelectValue { name } }
        release:  fieldValueByName(name:"Release")   { ... on ProjectV2ItemFieldSingleSelectValue { name } }
        iteration:fieldValueByName(name:"Iteration") { ... on ProjectV2ItemFieldIterationValue { title } }
        content{ ... on Issue {
          number state title repository { name }
          labels(first:30){ nodes{ name } }
        } }
      }
    } } }
  }`;
  const byKey = new Map();
  const seen = new Set();
  let cursor = null;
  do {
    const data = graphql(q, { owner: OWNER, number: PROJECT_NUMBER, endCursor: cursor });
    const items = data.user.projectV2.items;
    for (const n of items.nodes) {
      if (seen.has(n.id)) continue;
      seen.add(n.id);
      const c = n.content;
      if (!c || c.number == null) continue;
      const key = `${c.repository.name}#${c.number}`;
      byKey.set(key, {
        itemId: n.id,
        key,
        repo: c.repository.name,
        num: c.number,
        state: c.state,
        title: c.title,
        labels: c.labels.nodes.map((l) => l.name),
        status: n.status?.name ?? null,
        priority: n.priority?.name ?? null,
        moscow: n.moscow?.name ?? null,
        release: n.release?.name ?? null,
        iteration: n.iteration?.title ?? null,
      });
    }
    cursor = items.pageInfo.hasNextPage ? items.pageInfo.endCursor : null;
  } while (cursor);
  _board = byKey;
  return _board;
}

// ---------------------------------------------------------------------------
// Sub-issue graph: map every descendant impl issue -> set of ancestor stories
// ---------------------------------------------------------------------------
const _subCache = new Map(); // "owner/repo#num" -> child issue objects
function subIssues(owner, repo, num) {
  const k = `${owner}/${repo}#${num}`;
  if (_subCache.has(k)) return _subCache.get(k);
  let res = [];
  try {
    res = ghJson(["api", "--paginate", `repos/${owner}/${repo}/issues/${num}/sub_issues`]) || [];
  } catch (e) {
    console.error(`gh-project: warning: could not read sub-issues of ${k}: ${(e.stderr ? String(e.stderr) : e.message).trim()}`);
  }
  _subCache.set(k, res);
  return res;
}

// owner+repo for a child issue object (nested repository, else repository_url).
function ownerRepoOf(c) {
  if (c.repository?.owner?.login && c.repository?.name)
    return { owner: c.repository.owner.login, repo: c.repository.name };
  const m = /\/repos\/([^/]+)\/([^/]+)$/.exec(c.repository_url || "");
  return m ? { owner: m[1], repo: m[2] } : { owner: OWNER, repo: null };
}

// Returns Map<"repo#num", Set<storyNumber>> of descendants per story.
function storyDescendants(stories) {
  const map = new Map();
  for (const story of stories) {
    const visited = new Set();
    const stack = subIssues(OWNER, STORY_REPO, story.num).map((c) => ({ c, root: story.num }));
    while (stack.length) {
      const { c, root } = stack.pop();
      const { owner, repo } = ownerRepoOf(c);
      if (!repo) continue;
      if (owner !== OWNER) {
        console.error(`gh-project: warning: skipping cross-owner sub-issue ${owner}/${repo}#${c.number}`);
        continue;
      }
      const key = `${repo}#${c.number}`;
      if (visited.has(key)) continue; // cycle/diamond guard within a story
      visited.add(key);
      if (!map.has(key)) map.set(key, new Set());
      map.get(key).add(root);
      for (const k of subIssues(owner, repo, c.number)) stack.push({ c: k, root });
    }
  }
  return map;
}

// ---------------------------------------------------------------------------
// Stories (the human layer)
// ---------------------------------------------------------------------------
function openStories() {
  return (
    ghJson([
      "issue", "list", "--repo", `${OWNER}/${STORY_REPO}`,
      "--label", STORY_LABEL, "--state", "open", "--limit", "200",
      "--json", "number,title",
    ]) || []
  ).map((s) => ({ num: s.number, title: s.title }));
}

// ---------------------------------------------------------------------------
// Mutations (idempotent)
// ---------------------------------------------------------------------------
function setSingleSelect(itemId, fieldName, optionName, dryRun) {
  const fid = field(fieldName).id;
  const oid = optionId(fieldName, optionName);
  if (dryRun) return;
  graphql(
    `mutation($p:ID!,$i:ID!,$f:ID!,$o:String!){
       updateProjectV2ItemFieldValue(input:{projectId:$p,itemId:$i,fieldId:$f,value:{singleSelectOptionId:$o}}){ projectV2Item{ id } } }`,
    { p: meta().id, i: itemId, f: fid, o: oid }
  );
}

function clearField(itemId, fieldName, dryRun) {
  if (dryRun) return;
  graphql(
    `mutation($p:ID!,$i:ID!,$f:ID!){
       clearProjectV2ItemFieldValue(input:{projectId:$p,itemId:$i,fieldId:$f}){ projectV2Item{ id } } }`,
    { p: meta().id, i: itemId, f: field(fieldName).id }
  );
}

function setIteration(itemId, title, dryRun) {
  if (dryRun) return;
  graphql(
    `mutation($p:ID!,$i:ID!,$f:ID!,$v:String!){
       updateProjectV2ItemFieldValue(input:{projectId:$p,itemId:$i,fieldId:$f,value:{iterationId:$v}}){ projectV2Item{ id } } }`,
    { p: meta().id, i: itemId, f: field("Iteration").id, v: iterationId(title) }
  );
}

function ensureOnBoard(repo, num, dryRun) {
  const key = `${repo}#${num}`;
  const existing = board().get(key);
  if (existing) return existing.itemId;
  const node = ghJson(["api", `repos/${OWNER}/${repo}/issues/${num}`, "--jq", "{id:.node_id}"]);
  const nodeId = node?.id;
  if (!nodeId) die(`could not resolve issue node id for ${key}`);
  if (dryRun) return null;
  const res = graphql(
    `mutation($p:ID!,$c:ID!){ addProjectV2ItemById(input:{projectId:$p,contentId:$c}){ item{ id } } }`,
    { p: meta().id, c: nodeId }
  );
  return res.addProjectV2ItemById.item.id;
}

function setPriorityLabel(repo, num, p, dryRun) {
  // p is "P0"|"P1"|"P2"|null. Ensure exactly one priority: label.
  const add = p ? [`priority: ${p}`] : [];
  const remove = P_ORDER.filter((x) => x !== p).map((x) => `priority: ${x}`);
  if (dryRun) return;
  ensureLabels(repo);
  const args = ["issue", "edit", String(num), "--repo", `${OWNER}/${repo}`];
  for (const l of add) args.push("--add-label", l);
  for (const l of remove) args.push("--remove-label", l);
  if (add.length || remove.length) {
    try { gh(args); } catch { /* label may not be present; non-fatal */ }
  }
}

const _labelled = new Set();
function ensureLabels(repo) {
  if (_labelled.has(repo)) return;
  for (const [p, color] of [["P0", "B60205"], ["P1", "D93F0B"], ["P2", "FBCA04"]]) {
    try {
      gh(["label", "create", `priority: ${p}`, "--repo", `${OWNER}/${repo}`, "--color", color, "--force"]);
    } catch { /* exists */ }
  }
  _labelled.add(repo);
}

// ---------------------------------------------------------------------------
// Rollup engine
// ---------------------------------------------------------------------------
const higher = (a, b) => (a && (!b || P_ORDER.indexOf(a) < P_ORDER.indexOf(b)) ? a : b);

// Derive an issue's priority AND record every stage of the calculation, so the
// reasoning is explainable (see `summary` / `explain`).
function derivePriority(row, served, storiesByNum) {
  const isBug = row.labels.includes("bug");
  const servedArr = served ? [...served] : [];

  // Stage 1–2: served stories and each one's MoSCoW → P.
  const storyValues = servedArr.map((sn) => {
    const moscow = storiesByNum.get(sn)?.moscow ?? null;
    return { story: sn, moscow, p: moscow ? MOSCOW_TO_P[moscow] : null };
  });

  // Stage 3: base = highest (most urgent) P across served stories.
  let base = null;
  for (const sv of storyValues) base = higher(sv.p, base);

  // Stage 4: bug floor — a bug is never weaker than P1.
  let p = base;
  let bugFloor = null;
  if (isBug) {
    const to = higher(base, BUG_FLOOR); // raise to P1 if base is weaker/absent
    bugFloor = { applied: to !== base, from: base, to };
    p = to;
  }

  // Stage 5: bump one tier (cap P0) if a bump-signal label is present.
  const bumpLabels = row.labels.filter((l) => BUMP_LABELS.has(l));
  let bump = { labels: bumpLabels, applied: false, from: p, to: p };
  if (p && bumpLabels.length) {
    const to = P_ORDER[Math.max(0, P_ORDER.indexOf(p) - 1)];
    bump = { labels: bumpLabels, applied: to !== p, from: p, to };
    p = to;
  }

  const kind = servedArr.length ? "story-derived" : isBug ? "bug-floor" : "unlinked";
  return { p, stages: { kind, isBug, served: servedArr, storyValues, base, bugFloor, bump, final: p } };
}

function computeRollup() {
  const b = board();
  const stories = openStories();
  // Story board rows carry the MoSCoW value.
  const storiesByNum = new Map();
  for (const s of stories) {
    const row = b.get(`${STORY_REPO}#${s.num}`);
    storiesByNum.set(s.num, { num: s.num, title: s.title, moscow: row?.moscow ?? null, onBoard: !!row });
  }
  const descendants = storyDescendants([...storiesByNum.values()]);

  const derived = []; // {row, p, stages, basis}
  const bugs = [];
  const unlinked = [];
  for (const row of b.values()) {
    if (row.state !== "OPEN") continue;
    if (row.repo === STORY_REPO) continue; // skip stories/epics themselves
    if (row.labels.includes(STORY_LABEL) || row.labels.includes("epic") || row.labels.includes("plan")) continue;
    const { p, stages } = derivePriority(row, descendants.get(row.key), storiesByNum);
    if (stages.kind === "story-derived")
      derived.push({ row, p, stages, basis: `stories ${stages.served.map((n) => "#" + n).join(",")}` });
    else if (stages.kind === "bug-floor") bugs.push({ row, p, stages, basis: "bug floor (no story)" });
    else unlinked.push({ row, stages });
  }
  const uncovered = [...storiesByNum.values()].filter(
    (s) => ![...descendants.values()].some((set) => set.has(s.num))
  );
  return { derived, bugs, unlinked, uncovered, storiesByNum };
}

function applyPriority(entry, dryRun) {
  const { row, p } = entry;
  const want = p ? `priority: ${p}` : null;
  const have = row.labels.find((l) => l.startsWith("priority: ")) || null;
  const boardP = row.priority;
  const labelStale = (want || null) !== (have || null);
  const boardStale = (p || null) !== (boardP || null);
  if (!labelStale && !boardStale) return false;
  setPriorityLabel(row.repo, row.num, p, dryRun);
  if (p) setSingleSelect(row.itemId, "Priority", p, dryRun);
  else if (boardP) clearField(row.itemId, "Priority", dryRun); // don't leave a stale board value
  return true;
}

// ---------------------------------------------------------------------------
// Release (swimlane) field — derived from milestone:* labels + story parentage
// ---------------------------------------------------------------------------
const RELEASE_FROM_LABEL = {
  "milestone:decision-logger-v1": "Decision Logger v1",
  "milestone:safe-to-try": "Safe to try",
  "milestone:future": "Future",
};
function releaseFromLabels(labels) {
  for (const [lbl, rel] of Object.entries(RELEASE_FROM_LABEL)) if (labels.includes(lbl)) return rel;
  return null;
}
function createReleaseField() {
  graphql(
    `mutation($p:ID!){ createProjectV2Field(input:{projectId:$p,dataType:SINGLE_SELECT,name:"Release",
      singleSelectOptions:[
        {name:"Decision Logger v1",color:GREEN,description:""},
        {name:"Safe to try",color:BLUE,description:""},
        {name:"Future",color:GRAY,description:""}
      ]}){ projectV2Field{ ... on ProjectV2SingleSelectField { id name } } } }`,
    { p: meta().id }
  );
  _meta = null; // refresh field cache so field("Release") resolves
}
function allStoriesFull() {
  return (
    ghJson([
      "issue", "list", "--repo", `${OWNER}/${STORY_REPO}`, "--label", STORY_LABEL,
      "--state", "all", "--limit", "300", "--json", "number,title,labels,state",
    ]) || []
  ).map((s) => ({ num: s.number, labels: (s.labels || []).map((l) => l.name) }));
}
function cmdReleaseSync(dryRun) {
  if (!meta().fields["Release"]) {
    if (dryRun) console.log("[dry-run] would create Release field (Decision Logger v1 / Safe to try / Future)");
    else { createReleaseField(); console.log("Created Release field."); }
  }
  const b = board();
  // key -> release, story first then inherited by descendants
  const targets = new Map();
  const assign = (key, rel) => { if (rel && !targets.has(key)) targets.set(key, rel); };
  for (const s of allStoriesFull()) {
    const rel = releaseFromLabels(s.labels);
    if (!rel) continue;
    assign(`${STORY_REPO}#${s.num}`, rel);
    const visited = new Set();
    const stack = subIssues(OWNER, STORY_REPO, s.num).map((c) => ({ c }));
    while (stack.length) {
      const { c } = stack.pop();
      const { owner, repo } = ownerRepoOf(c);
      if (!repo || owner !== OWNER) continue;
      const key = `${repo}#${c.number}`;
      if (visited.has(key)) continue;
      visited.add(key);
      assign(key, rel);
      for (const k of subIssues(owner, repo, c.number)) stack.push({ c: k });
    }
  }
  assign("muDemocracy.org#30", "Decision Logger v1");
  assign("muDemocracy.org#36", "Safe to try");
  let set = 0, off = 0;
  for (const [key, rel] of targets) {
    const row = b.get(key);
    if (!row) { off++; continue; }
    if (row.release === rel) continue;
    console.log(`${dryRun ? "[dry-run] " : ""}Release ${key} = ${rel}`);
    if (!dryRun) setSingleSelect(row.itemId, "Release", rel, false);
    set++;
  }
  console.log(`${targets.size} targets · ${set} ${dryRun ? "would be " : ""}set · ${off} not on board`);
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------
const fmt = (o) => JSON.stringify(o, null, 2);

function cmdFields() {
  const m = meta();
  const out = { project: { id: m.id, title: m.title }, fields: {} };
  for (const [name, f] of Object.entries(m.fields)) {
    out.fields[name] = {
      id: f.id,
      options: f.options?.map((o) => ({ name: o.name, id: o.id })),
      iterations: f.configuration?.iterations?.map((i) => ({ title: i.title, id: i.id })),
    };
  }
  console.log(fmt(out));
}

function cmdEnsureFields(dryRun) {
  const m = meta();
  if (m.fields["MoSCoW"]) {
    console.log("MoSCoW field already present.");
  } else if (dryRun) {
    console.log("[dry-run] would create single-select field MoSCoW (Must/Should/Could/Won't).");
  } else {
    graphql(
      `mutation($p:ID!){ createProjectV2Field(input:{projectId:$p,dataType:SINGLE_SELECT,name:"MoSCoW",
        singleSelectOptions:[
          {name:"Must",color:RED,description:""},
          {name:"Should",color:ORANGE,description:""},
          {name:"Could",color:YELLOW,description:""},
          {name:"Won't",color:GRAY,description:""}
        ]}){ projectV2Field{ ... on ProjectV2SingleSelectField { id name } } } }`,
      { p: m.id }
    );
    console.log("Created MoSCoW field.");
  }
  for (const req of ["Status", "Priority", "Iteration"]) {
    if (!m.fields[req]) console.log(`WARNING: required field "${req}" missing.`);
  }
}

function parseFilters(argv) {
  const f = {};
  for (let i = 0; i < argv.length; i++) {
    if (argv[i] === "--repo") f.repo = argv[++i];
    else if (argv[i] === "--status") f.status = argv[++i];
    else if (argv[i] === "--iteration") f.iteration = argv[++i];
    else if (argv[i] === "--open") f.open = true;
  }
  return f;
}

function cmdBoard(argv) {
  const f = parseFilters(argv);
  let rows = [...board().values()];
  if (f.open) rows = rows.filter((r) => r.state === "OPEN");
  if (f.repo) rows = rows.filter((r) => r.repo === f.repo);
  if (f.status) rows = rows.filter((r) => r.status === f.status);
  if (f.iteration) rows = rows.filter((r) => r.iteration === f.iteration);
  rows.sort(
    (a, b) =>
      pRank(a.priority) - pRank(b.priority) ||
      a.repo.localeCompare(b.repo) ||
      a.num - b.num
  );
  console.log(fmt(rows.map((r) => ({
    key: r.key, status: r.status, priority: r.priority,
    release: r.release, iteration: r.iteration, moscow: r.moscow, title: r.title,
  }))));
}

function cmdTree(storyNum) {
  if (!storyNum) die("usage: tree <story#>");
  const seen = new Set();
  const render = (owner, repo, num, depth) => {
    for (const k of subIssues(owner, repo, num)) {
      const { owner: o, repo: r } = ownerRepoOf(k);
      if (!r) continue;
      const key = `${o}/${r}#${k.number}`;
      const st = k.state?.toUpperCase?.() || "";
      console.log(`${"  ".repeat(depth)}- ${r}#${k.number} [${st}] ${k.title}`);
      if (seen.has(key)) continue;
      seen.add(key);
      render(o, r, k.number, depth + 1);
    }
  };
  console.log(`${STORY_REPO}#${storyNum} (story)`);
  render(OWNER, STORY_REPO, storyNum, 1);
}

function cmdStoriesSync(dryRun) {
  const stories = openStories();
  let added = 0;
  for (const s of stories) {
    const key = `${STORY_REPO}#${s.num}`;
    if (board().get(key)) continue;
    console.log(`${dryRun ? "[dry-run] " : ""}add ${key}`);
    if (!dryRun) ensureOnBoard(STORY_REPO, s.num, false);
    added++;
  }
  console.log(`${stories.length} stories, ${added} added to board.`);
}

function cmdStorySet(argv, dryRun) {
  const num = argv[0];
  if (!num) die("usage: story set <num> --moscow <M> [--release <milestone>]");
  let moscow, release;
  for (let i = 1; i < argv.length; i++) {
    if (argv[i] === "--moscow") moscow = argv[++i];
    else if (argv[i] === "--release") release = argv[++i];
  }
  const itemId = ensureOnBoard(STORY_REPO, num, dryRun);
  if (moscow) {
    console.log(`${dryRun ? "[dry-run] " : ""}MoSCoW ${STORY_REPO}#${num} = ${moscow}`);
    if (!dryRun) setSingleSelect(itemId, "MoSCoW", moscow, false);
  }
  if (release) {
    console.log(`${dryRun ? "[dry-run] " : ""}milestone ${STORY_REPO}#${num} = ${release}`);
    if (!dryRun) gh(["issue", "edit", String(num), "--repo", `${OWNER}/${STORY_REPO}`, "--milestone", release]);
  }
}

function cmdRollup(argv) {
  const dryRun = !argv.includes("--fix");
  const r = computeRollup();
  const lines = [];
  lines.push("## Story-derived");
  for (const e of [...r.derived].sort((a, b) => pRank(a.p) - pRank(b.p))) {
    const changed = applyPriority(e, dryRun);
    lines.push(`  ${e.row.key} -> ${e.p ?? "(none)"}  [${e.basis}]${changed ? (dryRun ? " (would change)" : " (changed)") : ""}`);
  }
  lines.push("## Bugs — fix ASAP (no story)");
  for (const e of r.bugs) {
    const changed = applyPriority(e, dryRun);
    lines.push(`  ${e.row.key} -> ${e.p}  [${e.basis}]${changed ? (dryRun ? " (would change)" : " (changed)") : ""}`);
  }
  lines.push("## Unlinked — could get lost (non-bug, no story)");
  for (const u of r.unlinked) lines.push(`  ${u.row.key}  ${u.row.title}`);
  lines.push("## Uncovered stories (no implementation children)");
  for (const s of r.uncovered) lines.push(`  ${STORY_REPO}#${s.num}  ${s.title}`);
  console.log(lines.join("\n"));
  if (dryRun) console.log("\n(dry-run; pass --fix to write labels + board Priority)");
}

function cmdCoverage() {
  const r = computeRollup();
  console.log(fmt({
    bugs_fix_asap: r.bugs.map((e) => ({ key: e.row.key, p: e.p, title: e.row.title })),
    unlinked_could_get_lost: r.unlinked.map((u) => ({ key: u.row.key, title: u.row.title })),
    uncovered_stories: r.uncovered.map((s) => ({ key: `${STORY_REPO}#${s.num}`, title: s.title })),
  }));
}

// Compact "moscow→base" cell, e.g. "Must,Should→P0" or "—".
function moscowCell(stages) {
  if (!stages.storyValues.length) return "—";
  const ms = stages.storyValues.map((sv) => sv.moscow ?? "?").join(",");
  return `${ms}→${stages.base ?? "none"}`;
}

const STAGE_LEGEND = [
  ["1 served stories", "walk the sub-issue graph up to the user stories an issue serves"],
  ["2 MoSCoW → P", "Must→P0 · Should→P1 · Could→P2 · Won't→(none)"],
  ["3 base", "highest (most urgent) P across the served stories"],
  ["4 bug floor", "a `bug` is never weaker than P1 (even with no story)"],
  ["5 bump", "+1 tier (cap P0) if a label is in {" + [...BUMP_LABELS].join(", ") + "}"],
  ["6 final", "the derived priority (written as the `priority: Pn` label + board mirror)"],
];

function cmdSummary(argv) {
  const brief = argv.includes("--brief");
  let fRepo, fRelease;
  for (let i = 0; i < argv.length; i++) {
    if (argv[i] === "--repo") fRepo = argv[++i];
    else if (argv[i] === "--release") fRelease = argv[++i];
  }
  const r = computeRollup();
  const keep = (row) => (!fRepo || row.repo === fRepo) && (!fRelease || row.release === fRelease);
  const estimates = [...r.derived, ...r.bugs].filter((e) => keep(e.row)).sort((a, b) => pRank(a.p) - pRank(b.p));

  const L = [];
  L.push("PRIORITY ESTIMATE — how it is calculated");
  L.push("");
  for (const [k, v] of STAGE_LEGEND) L.push(`  Stage ${k.padEnd(16)} ${v}`);
  L.push("");

  // Totals
  const cnt = { P0: 0, P1: 0, P2: 0 };
  for (const e of estimates) if (e.p) cnt[e.p]++;
  L.push(
    `TOTALS   P0×${cnt.P0}  P1×${cnt.P1}  P2×${cnt.P2}   ·   ` +
      `bugs(no story)×${r.bugs.filter((e) => keep(e.row)).length}  ` +
      `unlinked×${r.unlinked.filter((u) => keep(u.row)).length}  ` +
      `uncovered-stories×${r.uncovered.length}`
  );

  // By release
  if (!fRelease) {
    const byRel = new Map();
    for (const e of estimates) {
      const rel = e.row.release ?? "(no release)";
      const m = byRel.get(rel) ?? { P0: 0, P1: 0, P2: 0 };
      if (e.p) m[e.p]++;
      byRel.set(rel, m);
    }
    if (byRel.size) {
      L.push("");
      L.push("BY RELEASE");
      for (const [rel, m] of byRel) L.push(`  ${rel.padEnd(22)} P0×${m.P0}  P1×${m.P1}  P2×${m.P2}`);
    }
  }

  if (!brief) {
    L.push("");
    L.push("ESTIMATES (stage by stage)");
    L.push(`  ${"issue".padEnd(16)} ${"served".padEnd(14)} ${"moscow→base".padEnd(16)} ${"floor".padEnd(6)} ${"bump".padEnd(14)} final`);
    for (const e of estimates) {
      const s = e.stages;
      const served = s.served.length ? s.served.map((n) => "#" + n).join(",") : e.stages.isBug ? "—(bug)" : "—";
      const floor = s.bugFloor?.applied ? `→${s.bugFloor.to}` : "—";
      const bumpc = s.bump.applied ? `→${s.bump.to}(${s.bump.labels.join(",")})` : "—";
      L.push(
        `  ${e.row.key.padEnd(16)} ${served.slice(0, 14).padEnd(14)} ${moscowCell(s).padEnd(16)} ${floor.padEnd(6)} ${bumpc.slice(0, 14).padEnd(14)} ${e.p ?? "none"}`
      );
    }
  }
  console.log(L.join("\n"));
}

function cmdExplain(argv) {
  const repo = argv[0];
  const num = argv[1];
  if (!repo || !num) die("usage: explain <repo> <issue#>");
  const key = `${repo}#${num}`;
  const r = computeRollup();
  const entry =
    r.derived.find((e) => e.row.key === key) ||
    r.bugs.find((e) => e.row.key === key) ||
    r.unlinked.find((u) => u.row.key === key);
  if (!entry) die(`${key} is not an open, prioritisable issue on the board (is it a story/epic, closed, or absent?)`);
  const { row, stages: s } = entry;
  const p = entry.p ?? null;

  const L = [];
  L.push(`${row.key} — ${row.title}`);
  L.push(`final priority: ${p ?? "(none)"}${row.release ? ` · release: ${row.release}` : ""}`);
  L.push("");
  L.push("Stage 1 · served stories (sub-issue graph)");
  L.push(s.served.length ? s.served.map((n) => `    ${STORY_REPO}#${n}`).join("\n") : "    (none — this issue serves no user story)");
  L.push("Stage 2 · story value (MoSCoW → P)");
  L.push(
    s.storyValues.length
      ? s.storyValues.map((sv) => `    #${sv.story}  ${sv.moscow ?? "no MoSCoW set"} → ${sv.p ?? "(none)"}`).join("\n")
      : "    n/a"
  );
  L.push("Stage 3 · base = highest served story");
  L.push(`    ${s.base ?? "(none)"}`);
  L.push("Stage 4 · bug floor (a bug is never weaker than P1)");
  L.push(
    !s.isBug
      ? "    n/a — not a bug"
      : s.bugFloor.applied
        ? `    applied: ${s.bugFloor.from ?? "(none)"} → ${s.bugFloor.to}`
        : `    not needed — base ${s.bugFloor.from} already ≥ P1`
  );
  L.push(`Stage 5 · bump (labels in {${[...BUMP_LABELS].join(", ")}})`);
  L.push(
    s.bump.labels.length
      ? s.bump.applied
        ? `    applied: ${s.bump.from} → ${s.bump.to}  (label: ${s.bump.labels.join(", ")})`
        : `    signal present (${s.bump.labels.join(", ")}) but already at P0`
      : "    none"
  );
  L.push("Stage 6 · final");
  const label = row.labels.find((l) => l.startsWith("priority: ")) ?? "(no label)";
  const sync = (p ? `priority: ${p}` : "(none)") === label && (p ?? null) === (row.priority ?? null);
  L.push(`    ${p ?? "(none)"}   [board: ${row.priority ?? "—"} · label: ${label}]  ${sync ? "✓ in sync" : "⚠ stale — run rollup --fix"}`);
  console.log(L.join("\n"));
}

function cmdSet(argv) {
  const dryRun = argv.includes("--dry-run");
  const repo = argv[0];
  const num = argv[1];
  if (!repo || !num) die("usage: set <repo> <issue#> [--status S --priority P --iteration N] [--dry-run]");
  let status, priority, iteration;
  for (let i = 2; i < argv.length; i++) {
    if (argv[i] === "--status") status = argv[++i];
    else if (argv[i] === "--priority") priority = argv[++i];
    else if (argv[i] === "--iteration") iteration = argv[++i];
  }
  const itemId = ensureOnBoard(repo, num, dryRun);
  if (status) { console.log(`${dryRun ? "[dry-run] " : ""}Status ${repo}#${num} = ${status}`); setSingleSelect(itemId, "Status", status, dryRun); }
  if (priority) { console.log(`${dryRun ? "[dry-run] " : ""}Priority ${repo}#${num} = ${priority}`); setSingleSelect(itemId, "Priority", priority, dryRun); setPriorityLabel(repo, num, priority, dryRun); }
  if (iteration) { console.log(`${dryRun ? "[dry-run] " : ""}Iteration ${repo}#${num} = ${iteration}`); setIteration(itemId, iteration, dryRun); }
}

function cmdAdd(argv) {
  const dryRun = argv.includes("--dry-run");
  const [repo, num] = argv;
  if (!repo || !num) die("usage: add <repo> <issue#>");
  const id = ensureOnBoard(repo, num, dryRun);
  console.log(`${dryRun ? "[dry-run] " : ""}${repo}#${num} on board${id ? " (" + id + ")" : ""}`);
}

function cmdReconcile(argv) {
  const dryRun = !argv.includes("--fix");
  const r = computeRollup();
  const issues = [];
  // Closed-but-not-Done
  for (const row of board().values()) {
    if (row.state === "CLOSED" && row.status && row.status !== "Done") {
      issues.push(`closed-not-done: ${row.key} (Status=${row.status})`);
      if (!dryRun) setSingleSelect(row.itemId, "Status", "Done", false);
    }
  }
  // Rollup-stale priorities
  for (const e of [...r.derived, ...r.bugs]) {
    const want = e.p ? `priority: ${e.p}` : null;
    const have = e.row.labels.find((l) => l.startsWith("priority: ")) || null;
    if ((want || null) !== (have || null)) {
      issues.push(`rollup-stale: ${e.row.key} label=${have ?? "—"} want=${want ?? "—"}`);
      if (!dryRun) applyPriority(e, false);
    }
  }
  // Open bug with no priority
  for (const e of r.bugs) {
    if (!e.row.priority && !e.row.labels.some((l) => l.startsWith("priority: ")))
      issues.push(`bug-unprioritised: ${e.row.key}`);
  }
  // Unlinked non-bug
  for (const u of r.unlinked) issues.push(`unlinked-could-get-lost: ${u.row.key}`);
  console.log(issues.length ? issues.join("\n") : "no drift");
  if (dryRun && issues.length) console.log("\n(dry-run; pass --fix to repair closed-not-done + rollup-stale)");
}

function help() {
  console.log(`gh-project — story-driven priority for SRS Project #${PROJECT_NUMBER} (${OWNER})

  fields                          dump project field/option/iteration IDs
  ensure-fields [--dry-run]       create the MoSCoW field if missing
  board [--repo R --status S --iteration N --open]
  add <repo> <issue#> [--dry-run]
  stories sync [--dry-run]        add open user-story issues to the board
  story set <num> --moscow <M> [--release <ms>]
  tree <story#>                   print story -> sub-issue tree
  rollup [--fix]                  derive impl priority from stories (dry-run by default)
  summary [--repo R --release X --brief]   priority estimates with the calculation stages
  explain <repo> <issue#>         stage-by-stage derivation for one issue
  coverage                        bugs-ASAP + unlinked + uncovered-stories audit (JSON)
  release-sync [--dry-run]        set the Release field from milestone labels + story parentage
  set <repo> <issue#> [--status --priority --iteration] [--dry-run]
  reconcile [--fix]               report/repair board drift

Priority stages: served stories → MoSCoW→P → base(max) → bug floor(P1) → bump(+1) → final.
Env: GHP_OWNER, GHP_PROJECT, GHP_STORY_REPO. Requires an authenticated \`gh\`.`);
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------
const [cmd, ...rest] = process.argv.slice(2);
const dry = rest.includes("--dry-run");
try {
  switch (cmd) {
    case "fields": cmdFields(); break;
    case "ensure-fields": cmdEnsureFields(dry); break;
    case "board": cmdBoard(rest); break;
    case "add": cmdAdd(rest); break;
    case "stories": rest[0] === "sync" ? cmdStoriesSync(dry) : die("usage: stories sync"); break;
    case "story": rest[0] === "set" ? cmdStorySet(rest.slice(1), dry) : die("usage: story set <num> ..."); break;
    case "tree": cmdTree(rest[0]); break;
    case "rollup": cmdRollup(rest); break;
    case "coverage": cmdCoverage(); break;
    case "summary": cmdSummary(rest); break;
    case "explain": cmdExplain(rest); break;
    case "release-sync": cmdReleaseSync(dry); break;
    case "set": cmdSet(rest); break;
    case "reconcile": cmdReconcile(rest); break;
    case "help": case "--help": case "-h": case undefined: help(); break;
    default: die(`unknown command "${cmd}" (try \`help\`)`);
  }
} catch (e) {
  die(e.stderr ? String(e.stderr) : e.message);
}
