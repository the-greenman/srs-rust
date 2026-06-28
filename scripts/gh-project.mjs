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
function bump(p, labels) {
  if (!p) return p;
  if (!labels.some((l) => BUMP_LABELS.has(l))) return p;
  const i = P_ORDER.indexOf(p);
  return P_ORDER[Math.max(0, i - 1)];
}

function highestMoscowP(storyNums, storiesByNum) {
  let best = null; // lower index = higher
  for (const sn of storyNums) {
    const moscow = storiesByNum.get(sn)?.moscow;
    const p = moscow ? MOSCOW_TO_P[moscow] : null;
    if (!p) continue;
    if (best === null || P_ORDER.indexOf(p) < P_ORDER.indexOf(best)) best = p;
  }
  return best;
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

  const derived = []; // {row, p, basis}
  const bugs = [];
  const unlinked = [];
  for (const row of b.values()) {
    if (row.state !== "OPEN") continue;
    if (row.repo === STORY_REPO) continue; // skip stories/epics themselves
    if (row.labels.includes(STORY_LABEL) || row.labels.includes("epic")) continue;
    const served = descendants.get(row.key);
    const isBug = row.labels.includes("bug");
    if (served && served.size) {
      let p = highestMoscowP(served, storiesByNum);
      if (isBug && p && P_ORDER.indexOf(BUG_FLOOR) < P_ORDER.indexOf(p)) p = BUG_FLOOR;
      if (isBug && !p) p = BUG_FLOOR;
      p = bump(p, row.labels);
      derived.push({ row, p, basis: `stories ${[...served].map((n) => "#" + n).join(",")}` });
    } else if (isBug) {
      const p = bump(BUG_FLOOR, row.labels);
      bugs.push({ row, p, basis: "bug floor (no story)" });
    } else {
      unlinked.push({ row });
    }
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
    iteration: r.iteration, moscow: r.moscow, title: r.title,
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
  coverage                        bugs-ASAP + unlinked + uncovered-stories audit (JSON)
  set <repo> <issue#> [--status --priority --iteration] [--dry-run]
  reconcile [--fix]               report/repair board drift

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
    case "set": cmdSet(rest); break;
    case "reconcile": cmdReconcile(rest); break;
    case "help": case "--help": case "-h": case undefined: help(); break;
    default: die(`unknown command "${cmd}" (try \`help\`)`);
  }
} catch (e) {
  die(e.stderr ? String(e.stderr) : e.message);
}
