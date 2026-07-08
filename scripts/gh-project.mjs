#!/usr/bin/env node
// gh-project — story-driven priority management for the SRS GitHub Project (#5).
//
// Single file, zero npm dependencies. Normally wraps the `gh` CLI; falls back to
// direct GitHub API calls via `curl` when `gh` is absent (set GITHUB_TOKEN or
// GH_TOKEN). Works inside an isolated single-repo checkout: every operation hits
// the GitHub API, so nothing here depends on a sibling repo being present on disk.
//
// Priority model: user stories (label `user-story`, in muDemocracy.org) carry a MoSCoW
// value on the board; implementation issues are their sub-issues (native GitHub
// sub-issues) and inherit a derived `priority: Pn` label (highest served story, bumped
// one tier if gate-blocking). An issue with no story ancestry but reachable from an
// epic inherits the epic's roadmap Priority one tier down (P0→P1, P1→P2, P2→P2) —
// engineering/enabling work rides its release's rank but sits below the release's
// story work unless a bump label raises it. Bugs floor at P1 even without a story;
// an issue under neither a story nor an epic is flagged as orphaned, never lost.
//
// Release model: an epic (label `epic`, in muDemocracy.org) IS a release. The epic
// declares its release once (its own Release field value) and ranks the roadmap via its
// Priority; every descendant inherits the epic's Release down the sub-issue graph
// (`release-sync`). Release lives only as the board field — there is no release label.
//
// Usage: node gh-project.mjs <command> [options]   (see `help`)

import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, unlinkSync } from "node:fs";
import { pathToFileURL } from "node:url";

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

// Canonical plain-label mirror set. The scheduled cloud routines read/write these
// (they can't use Projects v2 GraphQL through the web-session proxy), so every repo a
// routine touches must carry the full set — otherwise a `gh issue edit --add-label`
// hard-fails and the routine dies. This tool is the single source of truth for the set
// and creates any missing labels on demand (`ensure-labels`, and `rollup --fix` /
// `reconcile --fix`) so it can't drift. (#335)
//
// `promote:ready` is the promotion INTENT signal (not a mirror of any board field). A REST-only
// judge — the progress-review cloud routine, a human, or a future rule — marks an unblocked issue
// with it; it CANNOT set the board Status through the proxy. The `promote` command (run where
// Projects v2 is reachable: CI/local) converts that intent into the source of truth (board
// Status=Ready) and clears it. Kept distinct from `ready` so `reconcile`'s Status→label mirror
// never fights the judge: the judge writes `promote:ready`, the mirror owns `ready`.
const PROMOTE_INTENT_LABEL = "promote:ready";
const MIRROR_LABELS = [
  { name: "ready", color: "0E8A16", description: "Board Status=Ready mirror — the routines' work queue (set by `promote`/`reconcile`, not by hand)" },
  { name: "priority: P0", color: "B60205", description: "Derived priority (highest served story) — top" },
  { name: "priority: P1", color: "D93F0B", description: "Derived priority (highest served story)" },
  { name: "priority: P2", color: "FBCA04", description: "Derived priority (highest served story)" },
  { name: "status: in progress", color: "1D76DB", description: "Claimed in progress by the SRS jobs routine" },
  { name: PROMOTE_INTENT_LABEL, color: "5319E7", description: "Judged unblocked; awaiting CI promotion to board Status=Ready (write this, not `ready`)" },
  { name: "blocked", color: "E4E669", description: "Unmet prerequisites — auto-topup skips this issue; remove when unblocked" },
];
// Repos whose merges/routines depend on the mirror set existing. Overridable for tests/forks.
const MIRROR_REPOS = (process.env.GHP_MIRROR_REPOS || `srs,srs-rust,srs-web,${STORY_REPO}`)
  .split(",").map((s) => s.trim()).filter(Boolean);

// Board Status → plain-label mirror. The routines can't read Projects v2 Status through the
// proxy, so the label IS their signal: `ready` is the work queue, `status: in progress` marks a
// claim. Only these two statuses mirror to a label; all others (Backlog/In review/Done) clear both.
const STATUS_LABEL_MAP = { Ready: "ready", "In progress": "status: in progress" };
const STATUS_MIRROR_LABELS = Object.values(STATUS_LABEL_MAP);

// ---------------------------------------------------------------------------
// gh CLI detection — falls back to native HTTP (curl) when gh is absent
// ---------------------------------------------------------------------------
const GH_AVAILABLE = (() => {
  try { execFileSync("gh", ["--version"], { stdio: "pipe" }); return true; }
  catch { return false; }
})();
const GITHUB_TOKEN = process.env.GITHUB_TOKEN || process.env.GH_TOKEN || "";
if (!GH_AVAILABLE) {
  if (!GITHUB_TOKEN) {
    console.error("gh-project: neither `gh` CLI nor GITHUB_TOKEN/GH_TOKEN is available.");
    process.exit(1);
  }
  // Node 22+ native fetch must be told to honour the egress proxy; curl already does.
  process.env.NODE_USE_ENV_PROXY = "1";
  if (!process.env.NODE_EXTRA_CA_CERTS && existsSync("/root/.ccr/ca-bundle.crt"))
    process.env.NODE_EXTRA_CA_CERTS = "/root/.ccr/ca-bundle.crt";
}

// ---------------------------------------------------------------------------
// Small shell / GraphQL helpers
// ---------------------------------------------------------------------------
function gh(args, opts = {}) {
  return GH_AVAILABLE ? ghCli(args, opts) : ghHttp(args, opts);
}

function ghCli(args, { input } = {}) {
  return execFileSync("gh", args, {
    encoding: "utf8",
    input,
    maxBuffer: 64 * 1024 * 1024,
  });
}

// ---------------------------------------------------------------------------
// Native-HTTP fallback (curl-based) — used when gh CLI is absent.
// Handles exactly the gh argument patterns this script uses.
// ---------------------------------------------------------------------------
let _hdrSeq = 0;

function curlGet(url, { paginate = false } = {}) {
  const authHeaders = [
    "-H", `Authorization: Bearer ${GITHUB_TOKEN}`,
    "-H", "Accept: application/vnd.github+json",
    "-H", "X-GitHub-Api-Version: 2022-11-28",
  ];
  const results = [];
  let nextUrl = url;
  do {
    const hf = `/tmp/.gh-hdr-${process.pid}-${++_hdrSeq}`;
    let body = "";
    try {
      body = execFileSync("curl", ["-s", "-D", hf, ...authHeaders, nextUrl], {
        encoding: "utf8", maxBuffer: 64 * 1024 * 1024,
      });
    } finally {
      nextUrl = null;
      if (paginate) {
        try {
          const hdrs = readFileSync(hf, "utf8");
          const m = hdrs.match(/link:\s*[^<]*<([^>]+)>;\s*rel="next"/i);
          if (m) nextUrl = m[1];
        } catch { /* no header file */ }
      }
      try { unlinkSync(hf); } catch { /* ok */ }
    }
    if (paginate) {
      const parsed = JSON.parse(body || "[]");
      if (Array.isArray(parsed)) results.push(...parsed);
    } else {
      return body;
    }
  } while (nextUrl);
  return JSON.stringify(results);
}

function curlMutate(method, url, body) {
  const args = [
    "-s", "-X", method,
    "-H", `Authorization: Bearer ${GITHUB_TOKEN}`,
    "-H", "Accept: application/vnd.github+json",
    "-H", "X-GitHub-Api-Version: 2022-11-28",
  ];
  if (body !== undefined) {
    args.push("-H", "Content-Type: application/json", "-d", JSON.stringify(body));
  }
  args.push(url);
  return execFileSync("curl", args, { encoding: "utf8", maxBuffer: 64 * 1024 * 1024 });
}

function curlMutateStatus(method, url, body) {
  const args = [
    "-s", "-w", "\n%{http_code}", "-X", method,
    "-H", `Authorization: Bearer ${GITHUB_TOKEN}`,
    "-H", "Accept: application/vnd.github+json",
    "-H", "X-GitHub-Api-Version: 2022-11-28",
    "-H", "Content-Type: application/json",
    "-d", JSON.stringify(body),
    url,
  ];
  const raw = execFileSync("curl", args, { encoding: "utf8", maxBuffer: 64 * 1024 * 1024 });
  const nl = raw.lastIndexOf("\n");
  return { body: raw.slice(0, nl), status: parseInt(raw.slice(nl + 1), 10) };
}

function ghHttp(args, _opts = {}) {
  if (args[0] === "api")   return ghHttpApi(args.slice(1));
  if (args[0] === "issue") return ghHttpIssue(args.slice(1));
  if (args[0] === "label") return ghHttpLabel(args.slice(1));
  throw new Error(`gh-project: unsupported in no-gh mode: gh ${args[0]}`);
}

function ghHttpApi(args) {
  // GraphQL: ["graphql", "-f"/"F", "query=Q", ...vars]
  if (args[0] === "graphql") {
    let query = "";
    const variables = {};
    for (let i = 1; i < args.length; i++) {
      const flag = args[i];      // "-f" (string) or "-F" (typed)
      const kv   = args[++i];   // "key=value"
      const eq   = kv.indexOf("=");
      const key  = kv.slice(0, eq);
      const val  = kv.slice(eq + 1);
      if (key === "query") { query = val; continue; }
      if (flag === "-F") {
        if (val === "null")        variables[key] = null;
        else if (val === "true")   variables[key] = true;
        else if (val === "false")  variables[key] = false;
        else if (val !== "" && !isNaN(val)) variables[key] = Number(val);
        else                       variables[key] = val;
      } else {
        variables[key] = val;
      }
    }
    const reqBody = JSON.stringify({
      query,
      ...(Object.keys(variables).length ? { variables } : {}),
    });
    return execFileSync("curl", [
      "-s", "-X", "POST",
      "-H", `Authorization: Bearer ${GITHUB_TOKEN}`,
      "-H", "Content-Type: application/json",
      "-H", "Accept: application/vnd.github+json",
      "-H", "X-GitHub-Api-Version: 2022-11-28",
      "-d", reqBody,
      "https://api.github.com/graphql",
    ], { encoding: "utf8", maxBuffer: 64 * 1024 * 1024 });
  }

  // Paginated REST: ["--paginate", "path"]
  if (args[0] === "--paginate") {
    return curlGet(`https://api.github.com/${args[1]}?per_page=100`, { paginate: true });
  }

  // Plain REST (with optional --jq): ["path", ...flags]
  const path   = args[0];
  const jqIdx  = args.indexOf("--jq");
  const body   = curlGet(`https://api.github.com/${path}`);
  if (jqIdx !== -1 && args[jqIdx + 1] === "{id:.node_id}") {
    return JSON.stringify({ id: JSON.parse(body).node_id });
  }
  return body;
}

function ghHttpIssue(args) {
  // issue list --repo O/R [--label L] [--state S] [--limit N] [--json fields]
  if (args[0] === "list") {
    let repo, label, state = "open";
    for (let i = 1; i < args.length; i++) {
      if      (args[i] === "--repo")  repo  = args[++i];
      else if (args[i] === "--label") label = args[++i];
      else if (args[i] === "--state") state = args[++i];
      else if (args[i] === "--limit" || args[i] === "--json") ++i; // consume, not needed
    }
    const params = new URLSearchParams({ state, per_page: "100" });
    if (label) params.set("labels", label);
    const raw = curlGet(`https://api.github.com/repos/${repo}/issues?${params}`, { paginate: true });
    return JSON.stringify(JSON.parse(raw).map((i) => ({
      number: i.number,
      title:  i.title,
      state:  i.state,
      labels: (i.labels || []).map((l) => ({ name: l.name })),
    })));
  }

  // issue edit N --repo O/R [--add-label L] [--remove-label L] [--milestone M]
  if (args[0] === "edit") {
    const num = args[1];
    let repo, addLabels = [], removeLabels = [], milestone;
    for (let i = 2; i < args.length; i++) {
      if      (args[i] === "--repo")         repo = args[++i];
      else if (args[i] === "--add-label")    addLabels.push(args[++i]);
      else if (args[i] === "--remove-label") removeLabels.push(args[++i]);
      else if (args[i] === "--milestone")    milestone = args[++i];
    }
    const base = `https://api.github.com/repos/${repo}/issues/${num}`;
    if (milestone !== undefined) {
      const ms    = JSON.parse(curlGet(`https://api.github.com/repos/${repo}/milestones?per_page=100`));
      const found = ms.find((x) => x.title === milestone);
      curlMutate("PATCH", base, { milestone: found?.number ?? null });
    }
    if (addLabels.length) curlMutate("POST", `${base}/labels`, { labels: addLabels });
    for (const label of removeLabels) {
      try { curlMutate("DELETE", `${base}/labels/${encodeURIComponent(label)}`); } catch { /* ok */ }
    }
    return "";
  }

  throw new Error(`gh-project: unsupported in no-gh mode: gh issue ${args[0]}`);
}

function ghHttpLabel(args) {
  // label create NAME --repo O/R --color C [--force]
  if (args[0] === "create") {
    const name = args[1];
    let repo, color, force = false;
    for (let i = 2; i < args.length; i++) {
      if      (args[i] === "--repo")  repo  = args[++i];
      else if (args[i] === "--color") color = args[++i];
      else if (args[i] === "--force") force = true;
    }
    const url = `https://api.github.com/repos/${repo}/labels`;
    const r   = curlMutateStatus("POST", url, { name, color });
    if (r.status === 422 && force) curlMutate("PATCH", `${url}/${encodeURIComponent(name)}`, { name, color });
    return r.body;
  }
  throw new Error(`gh-project: unsupported in no-gh mode: gh label ${args[0]}`);
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
    const items = data?.user?.projectV2?.items;
    if (!items) {
      const why = data?.message || data?.errors?.[0]?.message
        || "unexpected response shape";
      die(`Projects v2 GraphQL is unavailable in this session — ${why}; run from a local machine or CI`);
    }
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

function setNumber(itemId, fieldName, value, dryRun) {
  if (dryRun) return;
  graphql(
    `mutation($p:ID!,$i:ID!,$f:ID!,$v:Float!){
       updateProjectV2ItemFieldValue(input:{projectId:$p,itemId:$i,fieldId:$f,value:{number:$v}}){ projectV2Item{ id } } }`,
    { p: meta().id, i: itemId, f: field(fieldName).id, v: value }
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

// Mirror board Status → plain label. `want` is the desired status-mirror label (or null to
// clear both). Ensures exactly one of STATUS_MIRROR_LABELS is present. Same shape as
// setPriorityLabel: the board Status can't be read by the routines, so the label is the mirror.
function setStatusLabel(repo, num, want, dryRun) {
  const add = want ? [want] : [];
  const remove = STATUS_MIRROR_LABELS.filter((l) => l !== want);
  if (dryRun) return;
  ensureLabels(repo);
  const args = ["issue", "edit", String(num), "--repo", `${OWNER}/${repo}`];
  for (const l of add) args.push("--add-label", l);
  for (const l of remove) args.push("--remove-label", l);
  try { gh(args); } catch { /* label may not be present; non-fatal */ }
}

// Desired status-mirror label for a board row: only OPEN issues in a mirrored status carry one.
function statusMirrorWant(row) {
  return row.state === "OPEN" ? (STATUS_LABEL_MAP[row.status] || null) : null;
}

// Pure promotion planner (unit-tested). Given board rows, decide what `promote` does with each
// `promote:ready` intent. Only OPEN Backlog issues (or ones with no Status set yet) actually move
// to Ready; anything already advanced (In progress / In review / Done) or closed just has the
// stale intent cleared — the intent is a one-shot request, never a demotion or a re-open.
// Returns [{ repo, num, key, action, promote, from }] — `promote` true means "set Status=Ready".
function planPromotions(rows) {
  const PROMOTABLE = new Set(["Backlog", null, undefined]);
  const out = [];
  for (const row of rows) {
    if (!(row.labels || []).includes(PROMOTE_INTENT_LABEL)) continue;
    const base = { repo: row.repo, num: row.num, key: row.key, from: row.status ?? null };
    if (row.state !== "OPEN") out.push({ ...base, action: "cleared-closed", promote: false });
    else if (row.status === "Ready") out.push({ ...base, action: "already-ready", promote: false });
    else if (PROMOTABLE.has(row.status)) out.push({ ...base, action: "promoted", promote: true });
    else out.push({ ...base, action: "skip-advanced", promote: false });
  }
  return out;
}

// Pure staleness planner (unit-tested). `status: in progress` has no board-native timestamp, so
// callers annotate each row with `claimedAtMs` (epoch ms the label was last applied, from the
// issue's REST event timeline — see `claimedAt()`) before calling this. Candidates are OPEN issues
// carrying the `status: in progress` claim label (or board Status "In progress"); nothing else in
// the pipeline ever revisits a claim (`promote` explicitly skips advanced statuses, `reconcile` only
// mirrors the label) so a dead claim would otherwise sit invisible forever. A row with no resolvable claimedAtMs is reported as
// `unknown` rather than silently left alone or wrongly reclaimed.
// Returns [{ repo, num, key, itemId, claimedAtMs, action, ageMs }] — action: reclaim|fresh|unknown.
function planStaleClaims(rows, nowMs, thresholdMs) {
  const out = [];
  const inProgLabel = STATUS_LABEL_MAP["In progress"];
  for (const row of rows) {
    // A claim is the `status: in progress` LABEL (what routines actually write — they can't set the
    // board Status through their proxy), not board Status "In progress". Keying off the label also
    // recovers zombie claims that never advanced the board (board still "Ready", label still set) —
    // previously invisible to both this recovery and the consumer. Board-Status matches still count.
    if (row.state !== "OPEN") continue;
    if (row.status !== "In progress" && !row.labels?.includes(inProgLabel)) continue;
    const base = { repo: row.repo, num: row.num, key: row.key, itemId: row.itemId, claimedAtMs: row.claimedAtMs ?? null };
    if (row.claimedAtMs == null) { out.push({ ...base, action: "unknown" }); continue; }
    const ageMs = nowMs - row.claimedAtMs;
    out.push({ ...base, action: ageMs >= thresholdMs ? "reclaim" : "fresh", ageMs });
  }
  return out;
}

// Pure topup planner (unit-tested). Receives pre-filtered, pre-sorted candidate rows (open
// workItems, Backlog/null status, not `blocked`, not `promote:ready` — caller's responsibility).
// Does NOT filter internally — do not move filtering here, it would break unit tests that pass
// raw arrays. Computes how many intents to write to fill the Ready queue to `target` depth, and
// returns the first `deficit` candidates as nominees. Caller writes `promote:ready` on each;
// `promote --fix` (run next in board-sync) converts them to board Status=Ready immediately.
// Returns { toNominate, deficit, currentReady, target }.
function planTopup(candidates, readyCount, target) {
  const deficit = Math.max(0, target - readyCount);
  return { toNominate: candidates.slice(0, deficit), deficit, currentReady: readyCount, target };
}

// Remove a single label from an issue (REST via gh). Non-fatal if absent.
function removeLabel(repo, num, label) {
  try { gh(["issue", "edit", String(num), "--repo", `${OWNER}/${repo}`, "--remove-label", label]); }
  catch { /* label not present — non-fatal */ }
}

// `gh label create` args for one mirror label in a repo. Idempotent via --force
// (creates if missing, updates color/description if present). Pure — safe to unit-test.
function labelCreateArgs(repo, { name, color, description }) {
  return ["label", "create", name, "--repo", `${OWNER}/${repo}`,
    "--color", color, "--description", description, "--force"];
}

const _labelled = new Set();
function ensureLabels(repo) {
  if (_labelled.has(repo)) return;
  for (const spec of MIRROR_LABELS) {
    try { gh(labelCreateArgs(repo, spec)); } catch { /* exists / insufficient perms — non-fatal */ }
  }
  _labelled.add(repo);
}

// ---------------------------------------------------------------------------
// Size (effort) — a first-class, maintained input. Lives as the `size: <v>` label
// (what the bands calc + routines read) and is mirrored to the board Size field.
// Not derivable — set by a human/agent at triage (see `size` command).
// ---------------------------------------------------------------------------
const SIZE_WEIGHT = { small: 1, medium: 2, large: 3, xl: 5 };
const SIZE_VALUES = Object.keys(SIZE_WEIGHT);           // small|medium|large|xl
const SIZE_TO_FIELD = { small: "S", medium: "M", large: "L", xl: "XL" }; // board Size options
const SIZE_DEFAULT_WEIGHT = SIZE_WEIGHT.medium;          // unsized items default to medium
const SIZE_LABEL_SPECS = [
  { name: "size: small",  color: "C2E0C6", description: "Effort estimate — small" },
  { name: "size: medium", color: "BFD4F2", description: "Effort estimate — medium" },
  { name: "size: large",  color: "FBCA04", description: "Effort estimate — large" },
  { name: "size: xl",     color: "E99695", description: "Effort estimate — extra large" },
];

// The `size:` value on a board row, or null.
function sizeOf(row) {
  const l = row.labels.find((x) => x.startsWith("size: "));
  return l ? l.slice("size: ".length).trim() : null;
}
// Effort weight for banding: from the size label, default medium.
function sizeWeight(row) {
  return SIZE_WEIGHT[sizeOf(row)] ?? SIZE_DEFAULT_WEIGHT;
}
// A leaf work item: not an epic (`epic`), a user story, or an engineering epic (`plan`).
function isWorkItem(row) {
  return !row.labels.includes("epic")
    && !row.labels.includes(STORY_LABEL)
    && !row.labels.includes("plan");
}
// The Ready-queue depth as the hourly *consumer* sees it: OPEN leaf work-items (no epic/story/plan)
// that are actually pickable — Status=Ready (or a `promote:ready` intent about to become Ready) and
// NOT already claimed (`status: in progress`). This MUST match what the consumer skips, or topup
// mis-measures the queue. Counting stories/epics/claimed items made it read "full" (Ready: 8) while
// zero consumable work remained, so it never refilled — and merges never rehydrated the queue.
function isConsumableReady(row) {
  return row.state === "OPEN"
    && isWorkItem(row)
    && !row.labels.includes(STATUS_LABEL_MAP["In progress"])
    && (row.status === "Ready" || row.labels.includes(PROMOTE_INTENT_LABEL));
}
function countConsumableReady(rows) {
  return rows.filter(isConsumableReady).length;
}
// Open leaf work items with no size label.
function unsizedLeaves() {
  const out = [];
  for (const row of board().values()) {
    if (row.state !== "OPEN" || !isWorkItem(row)) continue;
    if (!sizeOf(row)) out.push(row);
  }
  return out;
}

const _sizeLabelled = new Set();
function ensureSizeLabels(repo) {
  if (_sizeLabelled.has(repo)) return;
  for (const spec of SIZE_LABEL_SPECS) { try { gh(labelCreateArgs(repo, spec)); } catch { /* non-fatal */ } }
  _sizeLabelled.add(repo);
}
// Ensure exactly one `size:` label (mirror of setPriorityLabel).
function setSizeLabel(repo, num, size, dryRun) {
  const add = size ? [`size: ${size}`] : [];
  const remove = SIZE_VALUES.filter((s) => s !== size).map((s) => `size: ${s}`);
  if (dryRun) return;
  ensureSizeLabels(repo);
  const args = ["issue", "edit", String(num), "--repo", `${OWNER}/${repo}`];
  for (const l of add) args.push("--add-label", l);
  for (const l of remove) args.push("--remove-label", l);
  if (add.length || remove.length) { try { gh(args); } catch { /* label may not be present; non-fatal */ } }
}

// ---------------------------------------------------------------------------
// Rollup engine
// ---------------------------------------------------------------------------
const higher = (a, b) => (a && (!b || P_ORDER.indexOf(a) < P_ORDER.indexOf(b)) ? a : b);

// Derive an issue's priority AND record every stage of the calculation, so the
// reasoning is explainable (see `summary` / `explain`). `epicInfo` is the claiming
// epic (`{num, priority}`) from epicDescendants, or null if under no epic.
function derivePriority(row, served, storiesByNum, epicInfo = null) {
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

  // Stage 4: epic fallback — no story value, but the issue is reachable from an
  // epic: inherit the epic's roadmap Priority one tier down (P0→P1, P1→P2, P2→P2).
  // Engineering/enabling work rides its release's rank but sits below the release's
  // story work by default; a bump label (stage 6) raises a specific issue.
  let p = base;
  let epicFallback = { applied: false, epic: epicInfo?.num ?? null, epicPriority: epicInfo?.priority ?? null, to: null };
  if (p === null && epicInfo?.priority) {
    const to = P_ORDER[Math.min(P_ORDER.length - 1, P_ORDER.indexOf(epicInfo.priority) + 1)];
    epicFallback = { applied: true, epic: epicInfo.num, epicPriority: epicInfo.priority, to };
    p = to;
  }

  // Stage 5: bug floor — a bug is never weaker than P1.
  let bugFloor = null;
  if (isBug) {
    const to = higher(p, BUG_FLOOR); // raise to P1 if current value is weaker/absent
    bugFloor = { applied: to !== p, from: p, to };
    p = to;
  }

  // Stage 6: bump one tier (cap P0) if a bump-signal label is present.
  const bumpLabels = row.labels.filter((l) => BUMP_LABELS.has(l));
  let bump = { labels: bumpLabels, applied: false, from: p, to: p };
  if (p && bumpLabels.length) {
    const to = P_ORDER[Math.max(0, P_ORDER.indexOf(p) - 1)];
    bump = { labels: bumpLabels, applied: to !== p, from: p, to };
    p = to;
  }

  const kind = servedArr.length ? "story-derived" : isBug ? "bug-floor" : epicFallback.applied ? "epic-derived" : "orphaned";
  return { p, stages: { kind, isBug, served: servedArr, storyValues, base, epicFallback, bugFloor, bump, final: p } };
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

  // Epic ancestry for the fallback stage: key → claiming epic (with its Priority).
  const epics = openEpics();
  const epicByNum = new Map(epics.map((e) => [e.num, e]));
  const epicDesc = epicDescendants(epics);

  const derived = []; // {row, p, stages, basis}
  const bugs = [];
  const unlinked = []; // orphaned: under neither a story nor an epic
  for (const row of b.values()) {
    if (row.state !== "OPEN") continue;
    if (row.repo === STORY_REPO) continue; // skip stories/epics themselves
    if (row.labels.includes(STORY_LABEL) || row.labels.includes("epic") || row.labels.includes("plan")) continue;
    const claiming = epicDesc.get(row.key);
    const epicInfo = claiming != null ? { num: claiming, priority: epicByNum.get(claiming)?.priority ?? null } : null;
    const { p, stages } = derivePriority(row, descendants.get(row.key), storiesByNum, epicInfo);
    if (stages.kind === "story-derived")
      derived.push({ row, p, stages, basis: `stories ${stages.served.map((n) => "#" + n).join(",")}` });
    else if (stages.kind === "bug-floor") bugs.push({ row, p, stages, basis: "bug floor (no story)" });
    else if (stages.kind === "epic-derived")
      derived.push({ row, p, stages, basis: `epic #${stages.epicFallback.epic} ${stages.epicFallback.epicPriority}→${stages.epicFallback.to}` });
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
// Epics as releases — an epic (label `epic`, in muDemocracy.org) IS a release.
// The epic declares its release once (its own Release field value) and ranks the
// roadmap via its Priority; every descendant inherits the epic's Release down the
// native sub-issue graph. There is no separate release entity and no release label —
// Release lives only as the board field, derived here from the epic hierarchy.
// (Membership = sub-issue parent; the Release field = the derived, groupable mirror.)
// ---------------------------------------------------------------------------
const EPIC_LABEL = "epic";

// Rank an epic for roadmap order and diamond tie-breaking: Priority first, then #.
function epicRank(epic) {
  return pRank(epic.priority) * 1e6 + epic.num;
}

// Open epics in the story repo, joined to their board row (Release identity + Priority).
function openEpics() {
  const rows = (ghJson([
    "issue", "list", "--repo", `${OWNER}/${STORY_REPO}`,
    "--label", EPIC_LABEL, "--state", "open", "--limit", "100",
    "--json", "number,title",
  ]) || []).map((e) => ({ num: e.number, title: e.title }));
  const b = board();
  return rows.map((e) => {
    const row = b.get(`${STORY_REPO}#${e.num}`);
    return {
      num: e.num, title: e.title, key: `${STORY_REPO}#${e.num}`,
      release: row?.release ?? null, priority: row?.priority ?? null, onBoard: !!row,
    };
  });
}

// Map<"repo#num", epicNum> for every descendant of any epic. A node reachable from
// more than one epic is claimed by the highest-Priority epic (ties: lowest #).
function epicDescendants(epics) {
  const map = new Map();
  for (const epic of [...epics].sort((a, b) => epicRank(a) - epicRank(b))) {
    const visited = new Set();
    const stack = subIssues(OWNER, STORY_REPO, epic.num).map((c) => ({ c }));
    while (stack.length) {
      const { c } = stack.pop();
      const { owner, repo } = ownerRepoOf(c);
      if (!repo || owner !== OWNER) continue;
      const key = `${repo}#${c.number}`;
      if (visited.has(key)) continue;
      visited.add(key);
      if (!map.has(key)) map.set(key, epic.num); // higher-priority epic wins the diamond
      for (const k of subIssues(owner, repo, c.number)) stack.push({ c: k });
    }
  }
  return map;
}

// Everything the release model needs: epics, the descendant→epic map, the target
// Release per descendant, and open stories that sit under no epic.
function computeReleaseRollup() {
  const epics = openEpics();
  const desc = epicDescendants(epics);
  const relByEpic = new Map(epics.map((e) => [e.num, e.release]));
  const targets = new Map(); // key -> { release, epic }
  for (const [key, epicNum] of desc) {
    const release = relByEpic.get(epicNum);
    if (release) targets.set(key, { release, epic: epicNum });
  }
  const orphanStories = openStories().filter((s) => !desc.has(`${STORY_REPO}#${s.num}`));
  return { epics, desc, targets, orphanStories };
}

// Propagate each epic's Release down to its descendants (idempotent). Default writes;
// pass --dry-run to preview. The epic's own Release (its identity) is set by hand via
// `epic set` and is never overwritten here.
function cmdReleaseSync(dryRun) {
  if (!meta().fields["Release"])
    die("Release field not found on the board — create it in the UI first (its options are the epics).");
  const b = board();
  const { epics, targets } = computeReleaseRollup();
  for (const e of epics)
    if (!e.release) console.error(`gh-project: warning: epic ${e.key} has no Release set — its descendants can't inherit one`);
  let set = 0, ok = 0, off = 0;
  for (const [key, { release }] of targets) {
    const row = b.get(key);
    if (!row) { off++; continue; }
    if (row.release === release) { ok++; continue; }
    console.log(`${dryRun ? "[dry-run] " : ""}Release ${key} = ${release}`);
    if (!dryRun) setSingleSelect(row.itemId, "Release", release, false);
    set++;
  }
  console.log(`${targets.size} descendants under ${epics.length} epics · ${set} ${dryRun ? "would be " : ""}set · ${ok} already correct · ${off} not on board`);
  if (dryRun) console.log("(dry-run; run without --dry-run to write)");
}

// `epics` — roadmap read: epics ordered by Priority with release identity + coverage.
function cmdEpics() {
  const { epics, desc, orphanStories } = computeReleaseRollup();
  const b = board();
  const kidsByEpic = new Map(epics.map((e) => [e.num, []]));
  for (const [key, epicNum] of desc) kidsByEpic.get(epicNum)?.push(key);
  const out = [...epics].sort((a, c) => epicRank(a) - epicRank(c)).map((e) => {
    const kids = kidsByEpic.get(e.num) || [];
    const rows = kids.map((k) => b.get(k)).filter(Boolean);
    const done = rows.filter((r) => r.state === "CLOSED" || r.status === "Done").length;
    const flags = [];
    if (!e.release) flags.push("missing-release");
    if (!e.priority) flags.push("missing-priority");
    if (!kids.length) flags.push("no-descendants");
    return { epic: e.key, title: e.title, release: e.release, priority: e.priority, descendants: kids.length, done, flags };
  });
  console.log(fmt({
    epics: out,
    orphan_stories: orphanStories.map((s) => ({ key: `${STORY_REPO}#${s.num}`, title: s.title })),
  }));
}

// `epic set <n> --priority P [--release R]` — the two manual inputs on an epic: its
// roadmap rank (Priority) and its release identity (Release). No priority label written.
function cmdEpicSet(argv, dryRun) {
  const num = argv[0];
  if (!num) die("usage: epic set <num> --priority <P> [--release <R>]");
  let priority, release;
  for (let i = 1; i < argv.length; i++) {
    if (argv[i] === "--priority") priority = argv[++i];
    else if (argv[i] === "--release") release = argv[++i];
  }
  if (!priority && !release) die("nothing to set — pass --priority and/or --release");
  const itemId = ensureOnBoard(STORY_REPO, num, dryRun);
  if (priority) { console.log(`${dryRun ? "[dry-run] " : ""}Priority ${STORY_REPO}#${num} = ${priority}`); if (!dryRun) setSingleSelect(itemId, "Priority", priority, false); }
  if (release)  { console.log(`${dryRun ? "[dry-run] " : ""}Release ${STORY_REPO}#${num} = ${release}`);   if (!dryRun) setSingleSelect(itemId, "Release", release, false); }
}

// `link <parent-repo>#<num> <child-repo>#<num>` — generic native sub-issue link,
// cross-repo, plain REST (no Projects v2). This is how an agent parents a freshly
// filed engineering issue under the story or epic it serves so the rollup can derive
// its priority. Proxy-bound cloud routines can run this (or the equivalent raw
// `gh api` POST) — the sub-issues endpoints are REST, not GraphQL.
function parseIssueRef(s) {
  const m = /^([\w.-]+)#(\d+)$/.exec(s || "");
  return m ? { repo: m[1], num: m[2] } : null;
}
function cmdLink(argv, dryRun) {
  const refs = argv.filter((a) => !a.startsWith("--"));
  const parent = parseIssueRef(refs[0]);
  const child = parseIssueRef(refs[1]);
  if (!parent || !child) die("usage: link <parent-repo>#<num> <child-repo>#<num>   (e.g. link muDemocracy.org#48 srs-web#116)");
  const c = ghJson(["api", `repos/${OWNER}/${child.repo}/issues/${child.num}`, "--jq", "{id:.id}"]);
  if (!c?.id) die(`could not resolve issue id for ${child.repo}#${child.num}`);
  console.log(`${dryRun ? "[dry-run] " : ""}link ${child.repo}#${child.num} under ${parent.repo}#${parent.num}`);
  if (!dryRun) gh(["api", "-X", "POST", `repos/${OWNER}/${parent.repo}/issues/${parent.num}/sub_issues`, "-F", `sub_issue_id=${c.id}`]);
}

// `epic add-story <epic#> <story#>` — backfill: link a story under an epic as a native
// sub-issue (which epic is the owner's call; this just executes it).
function cmdEpicAddStory(argv, dryRun) {
  const [epicNum, storyNum] = argv;
  if (!epicNum || !storyNum) die("usage: epic add-story <epic#> <story#>");
  const story = ghJson(["api", `repos/${OWNER}/${STORY_REPO}/issues/${storyNum}`, "--jq", "{id:.id}"]);
  const sid = story?.id;
  if (!sid) die(`could not resolve issue id for ${STORY_REPO}#${storyNum}`);
  console.log(`${dryRun ? "[dry-run] " : ""}link ${STORY_REPO}#${storyNum} under epic ${STORY_REPO}#${epicNum}`);
  if (!dryRun) gh(["api", "-X", "POST", `repos/${OWNER}/${STORY_REPO}/issues/${epicNum}/sub_issues`, "-F", `sub_issue_id=${sid}`]);
}

// ---------------------------------------------------------------------------
// Implementation order + effort bands
// Order: epic Priority (`epicRank`) → issue MoSCoW-derived priority → sub-issue
// position. Weight: `size:` label (default medium). Sliced into N equal-effort bands.
// ---------------------------------------------------------------------------

// Ordered stream of open leaf work items across all epics (roadmap order), plus any
// open leaves under no epic (trailing `unlinkedLeaves`, never dropped).
function computeImplementationOrder() {
  const b = board();
  const { epics, desc } = computeReleaseRollup(); // desc: key -> claiming epicNum
  const roll = computeRollup();
  const pByKey = new Map();
  for (const e of [...roll.derived, ...roll.bugs, ...roll.unlinked]) pByKey.set(e.row.key, e.p ?? null);

  const sortedEpics = [...epics].sort((a, c) => epicRank(a) - epicRank(c));
  const seen = new Set();
  const ordered = [];
  for (const epic of sortedEpics) {
    const local = [];
    let pos = 0;
    const tvisited = new Set();
    const visit = (owner, repo, num) => {
      for (const k of subIssues(owner, repo, num)) {
        const { owner: o, repo: r } = ownerRepoOf(k);
        if (!r || o !== OWNER) continue;
        const key = `${r}#${k.number}`;
        const myPos = pos++;
        if (desc.get(key) === epic.num) {
          const row = b.get(key);
          if (row && row.state === "OPEN" && isWorkItem(row) && !seen.has(key)) local.push({ key, row, pos: myPos });
        }
        if (!tvisited.has(key)) { tvisited.add(key); visit(o, r, k.number); }
      }
    };
    visit(OWNER, STORY_REPO, epic.num);
    // within an epic: MoSCoW-derived priority first, then sub-issue position
    local.sort((x, y) => pRank(pByKey.get(x.key)) - pRank(pByKey.get(y.key)) || x.pos - y.pos);
    for (const it of local) {
      if (seen.has(it.key)) continue;
      seen.add(it.key);
      ordered.push({ row: it.row, epic, p: pByKey.get(it.key) ?? null });
    }
  }
  const unlinkedLeaves = [];
  for (const row of b.values()) {
    if (row.state !== "OPEN" || !isWorkItem(row) || seen.has(row.key)) continue;
    unlinkedLeaves.push({ row, epic: null, p: pByKey.get(row.key) ?? null });
  }
  return { ordered, unlinkedLeaves, epics: sortedEpics };
}

// Equal-effort banding. Pure: weights[] → band index per item (monotonic, order-preserving).
// Advances to the next band when cumulative effort crosses that band's share of the total;
// a tail guard force-advances when items run low so trailing bands aren't left empty.
// Exported for tests.
function bandTargets(weights, n) {
  const total = weights.reduce((a, w) => a + w, 0);
  const target = n > 0 ? total / n : total || 1;
  const out = [];
  let bi = 0, cum = 0;
  for (let i = 0; i < weights.length; i++) {
    out.push(bi);
    cum += weights[i];
    const remainingItems = weights.length - 1 - i;
    if (bi >= n - 1) continue;
    if (cum >= (bi + 1) * target && remainingItems > 0) bi++;      // this band has its share
    else if (remainingItems <= n - 1 - bi) bi++;                    // reserve a band per leftover item
  }
  return out;
}

function allocateBands(ordered, n) {
  const weights = ordered.map((it) => sizeWeight(it.row));
  const assign = bandTargets(weights, n);
  const bands = Array.from({ length: n }, () => ({ items: [], effort: 0 }));
  ordered.forEach((it, i) => { bands[assign[i]].items.push(it); bands[assign[i]].effort += weights[i]; });
  const total = weights.reduce((a, w) => a + w, 0);
  return { bands, total, target: n > 0 ? total / n : total };
}

// Opt-in: write each item's band index (1-based) into the "Band" number field.
// Replaces the old iteration mapping — iterations are date-bound (a band is not a
// calendar sprint), so band k → Band = k is the stable, date-free home for the order.
function assignBandsToBandField(bands, dryRun) {
  console.log("");
  if (!meta().fields["Band"]) {
    console.log('No "Band" field on the project — create a Number field named "Band" to assign bands.');
    return;
  }
  bands.forEach((band, i) => {
    console.log(`${dryRun ? "[dry-run] " : ""}Band ${i + 1} → Band = ${i + 1} (${band.items.length} issues)`);
    if (dryRun) return;
    for (const it of band.items) setNumber(it.row.itemId, "Band", i + 1, false);
  });
}

function cmdBands(argv) {
  const dryRun = argv.includes("--dry-run");
  const assign = argv.includes("--assign");
  const showTree = argv.includes("--tree");
  let n = 10;
  const ci = argv.indexOf("--count");
  if (ci !== -1 && argv[ci + 1]) n = Math.max(1, parseInt(argv[ci + 1], 10) || 10);
  const { ordered, unlinkedLeaves } = computeImplementationOrder();
  const { bands, total, target } = allocateBands(ordered, n);
  const L = [];
  if (showTree) { cmdTree(); L.push(""); }
  L.push(`IMPLEMENTATION BANDS — ${ordered.length} leaf issues · total effort ${total} · ~${target.toFixed(1)}/band · ${n} bands`);
  L.push("order: epic Priority → MoSCoW-derived priority → sub-issue position · weight: size label (default medium)");
  bands.forEach((band, i) => {
    L.push("");
    L.push(`Band ${i + 1}  (effort ~${band.effort} · ${band.items.length} issues)`);
    for (const it of band.items)
      L.push(`  ${it.row.key.padEnd(16)} [${(it.p ?? "—").padEnd(2)}][${(sizeOf(it.row) ?? "?").padEnd(6)}] (${it.epic?.release ?? "—"}) ${it.row.title.slice(0, 56)}`);
  });
  if (unlinkedLeaves.length) {
    L.push("");
    L.push(`Unlinked — under no epic (not banded): ${unlinkedLeaves.length}`);
    for (const it of unlinkedLeaves) L.push(`  ${it.row.key}  ${it.row.title.slice(0, 56)}`);
  }
  console.log(L.join("\n"));
  if (assign) assignBandsToBandField(bands, dryRun);
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------
const fmt = (o) => JSON.stringify(o, null, 2);

// `size <repo> <issue#> <small|medium|large|xl>` — the single writer of size:
// sets the `size:` label AND the board Size field (small→S · medium→M · large→L · xl→XL).
function cmdSize(argv) {
  const dryRun = argv.includes("--dry-run");
  const [repo, num, value] = argv;
  if (!repo || !num || !value) die(`usage: size <repo> <issue#> <${SIZE_VALUES.join("|")}> [--dry-run]`);
  const size = value.toLowerCase();
  if (!SIZE_VALUES.includes(size)) die(`size must be one of: ${SIZE_VALUES.join(", ")}`);
  const itemId = ensureOnBoard(repo, num, dryRun);
  console.log(`${dryRun ? "[dry-run] " : ""}Size ${repo}#${num} = ${size}`);
  setSizeLabel(repo, num, size, dryRun);
  if (!dryRun && itemId) { try { setSingleSelect(itemId, "Size", SIZE_TO_FIELD[size], false); } catch (e) { console.error(`  (board Size field not set: ${e.message})`); } }
}

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

function cmdEnsureLabels(argv) {
  const dryRun = argv.includes("--dry-run");
  let repos = MIRROR_REPOS;
  const ri = argv.indexOf("--repo");
  if (ri !== -1 && argv[ri + 1]) repos = [argv[ri + 1]];
  let created = 0;
  for (const repo of repos) {
    for (const spec of MIRROR_LABELS) {
      console.log(`${dryRun ? "[dry-run] " : ""}${repo}: ${spec.name}`);
      if (dryRun) continue;
      try { gh(labelCreateArgs(repo, spec)); created++; }
      catch (e) { console.error(`  ! ${repo}/${spec.name}: ${e.stderr ? String(e.stderr).trim() : e.message}`); }
    }
    _labelled.add(repo);
  }
  console.log(`${dryRun ? "(dry-run; pass without --dry-run to write)" : `ensured ${created} label(s) across ${repos.length} repo(s)`}`);
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

// `tree [<story#>]` — one story's sub-issue tree, or (no arg) the whole board:
// every epic in roadmap (epicRank) order with its subtree.
function cmdTree(storyNum) {
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
  if (storyNum) {
    console.log(`${STORY_REPO}#${storyNum} (story)`);
    render(OWNER, STORY_REPO, storyNum, 1);
    return;
  }
  const { epics } = computeReleaseRollup();
  for (const e of [...epics].sort((a, c) => epicRank(a) - epicRank(c))) {
    console.log(`${e.key} [${e.priority ?? "—"}] ${e.title}${e.release ? ` · ${e.release}` : ""}`);
    render(OWNER, STORY_REPO, e.num, 1);
  }
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
  if (!dryRun) for (const repo of MIRROR_REPOS) ensureLabels(repo); // self-heal mirror set (#335)
  const r = computeRollup();
  const lines = [];
  const section = (title, entries) => {
    lines.push(title);
    for (const e of [...entries].sort((a, b) => pRank(a.p) - pRank(b.p))) {
      const changed = applyPriority(e, dryRun);
      lines.push(`  ${e.row.key} -> ${e.p ?? "(none)"}  [${e.basis}]${changed ? (dryRun ? " (would change)" : " (changed)") : ""}`);
    }
  };
  section("## Story-derived", r.derived.filter((e) => e.stages.kind === "story-derived"));
  section("## Epic-derived (no story — epic Priority one tier down)", r.derived.filter((e) => e.stages.kind === "epic-derived"));
  section("## Bugs — fix ASAP (no story)", r.bugs);
  lines.push("## Orphaned — could get lost (non-bug, under no story and no epic)");
  for (const u of r.unlinked) lines.push(`  ${u.row.key}  ${u.row.title}`);
  lines.push("## Uncovered stories (no implementation children)");
  for (const s of r.uncovered) lines.push(`  ${STORY_REPO}#${s.num}  ${s.title}`);
  console.log(lines.join("\n"));
  if (dryRun) console.log("\n(dry-run; pass --fix to write labels + board Priority)");
}

function cmdCoverage() {
  const r = computeRollup();
  const { orphanStories } = computeReleaseRollup();
  console.log(fmt({
    bugs_fix_asap: r.bugs.map((e) => ({ key: e.row.key, p: e.p, title: e.row.title })),
    orphaned_could_get_lost: r.unlinked.map((u) => ({ key: u.row.key, title: u.row.title })),
    uncovered_stories: r.uncovered.map((s) => ({ key: `${STORY_REPO}#${s.num}`, title: s.title })),
    orphan_stories_no_epic: orphanStories.map((s) => ({ key: `${STORY_REPO}#${s.num}`, title: s.title })),
    unsized_issues: unsizedLeaves().map((row) => ({ key: row.key, title: row.title })),
  }));
}

// Compact "moscow→base" cell, e.g. "Must,Should→P0", "epic P0→P1", or "—".
function moscowCell(stages) {
  if (!stages.storyValues.length) {
    if (stages.epicFallback?.applied) return `epic ${stages.epicFallback.epicPriority}→${stages.epicFallback.to}`;
    return "—";
  }
  const ms = stages.storyValues.map((sv) => sv.moscow ?? "?").join(",");
  return `${ms}→${stages.base ?? "none"}`;
}

const STAGE_LEGEND = [
  ["1 served stories", "walk the sub-issue graph up to the user stories an issue serves"],
  ["2 MoSCoW → P", "Must→P0 · Should→P1 · Could→P2 · Won't→(none)"],
  ["3 base", "highest (most urgent) P across the served stories"],
  ["4 epic fallback", "no story: inherit the claiming epic's Priority one tier down (P0→P1, P1→P2, P2→P2)"],
  ["5 bug floor", "a `bug` is never weaker than P1 (even with no story)"],
  ["6 bump", "+1 tier (cap P0) if a label is in {" + [...BUMP_LABELS].join(", ") + "}"],
  ["7 final", "the derived priority (written as the `priority: Pn` label + board mirror)"],
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
      `orphaned×${r.unlinked.filter((u) => keep(u.row)).length}  ` +
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
      const served = s.served.length
        ? s.served.map((n) => "#" + n).join(",")
        : s.epicFallback?.applied ? `epic#${s.epicFallback.epic}` : e.stages.isBug ? "—(bug)" : "—";
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
  L.push("Stage 4 · epic fallback (no story: epic Priority one tier down)");
  L.push(
    s.served.length
      ? "    n/a — story-derived"
      : s.epicFallback?.applied
        ? `    applied: epic ${STORY_REPO}#${s.epicFallback.epic} Priority ${s.epicFallback.epicPriority} → ${s.epicFallback.to}`
        : s.epicFallback?.epic != null
          ? `    under epic ${STORY_REPO}#${s.epicFallback.epic}, but it has no Priority set — nothing to inherit`
          : "    n/a — not under any epic (orphaned)"
  );
  L.push("Stage 5 · bug floor (a bug is never weaker than P1)");
  L.push(
    !s.isBug
      ? "    n/a — not a bug"
      : s.bugFloor.applied
        ? `    applied: ${s.bugFloor.from ?? "(none)"} → ${s.bugFloor.to}`
        : `    not needed — base ${s.bugFloor.from} already ≥ P1`
  );
  L.push(`Stage 6 · bump (labels in {${[...BUMP_LABELS].join(", ")}})`);
  L.push(
    s.bump.labels.length
      ? s.bump.applied
        ? `    applied: ${s.bump.from} → ${s.bump.to}  (label: ${s.bump.labels.join(", ")})`
        : `    signal present (${s.bump.labels.join(", ")}) but already at P0`
      : "    none"
  );
  L.push("Stage 7 · final");
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
  if (status) { console.log(`${dryRun ? "[dry-run] " : ""}Status ${repo}#${num} = ${status}`); setSingleSelect(itemId, "Status", status, dryRun); setStatusLabel(repo, num, STATUS_LABEL_MAP[status] || null, dryRun); }
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

// Discover every issue carrying the promotion intent, across all mirrored repos, by LABEL search
// (REST) — not just board items. The judge routinely marks issues that are not on the board yet
// (a freshly-filed bug/enhancement), and those are exactly the ones we must pull onto the queue.
// Board Status is merged in where the issue is already a project item so planPromotions can tell
// Backlog from already-advanced; off-board issues have status null (→ promotable, and ensureOnBoard
// adds them to the board when we set Ready). --state all so stale intents on closed issues get cleared.
function intentRows() {
  const byKey = new Map([...board().values()].map((r) => [r.key, r]));
  const rows = [];
  for (const repo of MIRROR_REPOS) {
    const list = ghJson([
      "issue", "list", "--repo", `${OWNER}/${repo}`,
      "--label", PROMOTE_INTENT_LABEL, "--state", "all", "--limit", "200",
      "--json", "number,state,labels",
    ]) || [];
    for (const i of list) {
      const key = `${repo}#${i.number}`;
      rows.push({
        repo, num: i.number, key,
        state: String(i.state).toUpperCase(),        // gh returns OPEN|CLOSED
        status: byKey.get(key)?.status ?? null,       // board Status if on board, else null
        labels: (i.labels || []).map((l) => l.name),
      });
    }
  }
  return rows;
}

// Default staleness window for `stale-claims`: long enough that a genuinely long-running
// implementation task isn't falsely reclaimed, short enough that a dead claim recovers same-day
// given the hourly board-sync schedule.
const STALE_CLAIM_HOURS_DEFAULT = 24;

// Target depth for the Ready queue. `topup` writes `promote:ready` intents to fill the queue
// to this depth on every board-sync run. Overridable via GHP_TOPUP_TARGET env or --target flag.
const TOPUP_TARGET_DEFAULT = 3;

// When the `status: in progress` label was most recently applied — the claim's start time.
// Projects v2 has no per-field-value timestamp reachable here, so this reads the issue's REST
// event timeline (ascending order) and takes the last matching "labeled" event; a prior
// label/unlabel/re-label cycle is superseded by that most recent application. Returns epoch ms,
// or null if no such event is found (label present but timeline unreadable/incomplete).
function claimedAt(repo, num) {
  let events;
  try {
    // Use /timeline, not /issues/{n}/events: the events endpoint 404s for some issues (observed on
    // srs-rust#367) while /timeline reliably carries `labeled` events with `.label.name`+`.created_at`.
    events = ghJson(["api", "--paginate", `repos/${OWNER}/${repo}/issues/${num}/timeline`]) || [];
  } catch (e) {
    console.error(`gh-project: warning: could not read events for ${repo}#${num}: ${(e.stderr ? String(e.stderr) : e.message).trim()}`);
    return null;
  }
  const labelEvents = events.filter((e) => e.event === "labeled" && e.label?.name === "status: in progress");
  if (!labelEvents.length) return null;
  return Date.parse(labelEvents[labelEvents.length - 1].created_at);
}

// stale-claims [--hours N] [--fix] — detect (and with --fix, recover) `In progress` issues whose
// claim has gone stale: whatever claimed it (the "SRS jobs routine" or any other consumer) wrote
// `status: in progress` and never finished — crashed, timed out, or was interrupted. Nothing else
// in this pipeline ever revisits an in-progress issue: `promote` explicitly skips advanced statuses
// and `reconcile` only mirrors the label, never demotes it. Without this, a dead claim is invisible
// forever — not Backlog (so `promote` can't touch it), not Ready (so the queue consumer never sees
// it again). Recovery resets Status to Ready (mirrors the `ready` label) and leaves a comment so
// whatever actually holds the claim knows it was reclaimed. Idempotent: a claim younger than the
// threshold is left alone; an issue moved back to Ready simply won't match "In progress" next run.
function cmdStaleClaims(argv) {
  const dryRun = !argv.includes("--fix");
  let hours = STALE_CLAIM_HOURS_DEFAULT;
  for (let i = 0; i < argv.length; i++) if (argv[i] === "--hours") hours = Number(argv[++i]);
  const thresholdMs = hours * 3600 * 1000;
  const inProgLabel = STATUS_LABEL_MAP["In progress"];
  const candidates = [...board().values()].filter(
    (r) => r.state === "OPEN" && (r.status === "In progress" || r.labels.includes(inProgLabel))
  );
  const annotated = candidates.map((r) => ({ ...r, claimedAtMs: claimedAt(r.repo, r.num) }));
  const plan = planStaleClaims(annotated, Date.now(), thresholdMs);
  for (const p of plan) {
    const ageStr = p.ageMs != null ? `${(p.ageMs / 3600000).toFixed(1)}h` : "—";
    console.log(`${dryRun ? "[dry-run] " : ""}${p.action}: ${p.key} (age ${ageStr})`);
    if (dryRun || p.action !== "reclaim") continue;
    setSingleSelect(p.itemId, "Status", "Ready", false);
    setStatusLabel(p.repo, p.num, "ready", false); // mirror immediately; reconcile also keeps it in sync
    try {
      gh(["issue", "comment", String(p.num), "--repo", `${OWNER}/${p.repo}`,
        "--body", `Auto-reclaimed: this issue's \`status: in progress\` claim was stale (>${hours}h since claimed) and has been reset to **Ready** for re-pickup. If work is still genuinely in flight, re-claim it.`]);
    } catch (e) {
      console.error(`gh-project: warning: could not comment on ${p.key}: ${(e.stderr ? String(e.stderr) : e.message).trim()}`);
    }
  }
  const reclaimed = plan.filter((p) => p.action === "reclaim").length;
  const unknown = plan.filter((p) => p.action === "unknown").length;
  console.log(`${plan.length} in-progress · ${reclaimed} ${dryRun ? "would be " : ""}reclaimed · ${plan.length - reclaimed - unknown} fresh${unknown ? ` · ${unknown} unknown (no labeled event found)` : ""}`);
  if (dryRun && reclaimed) console.log("(dry-run; pass --fix to reclaim)");
}

// topup [--fix] [--target N] — keep the Ready queue at target depth by writing `promote:ready`
// intents to the highest-priority unblocked Backlog leaves. Runs before `promote --fix` in
// board-sync.yml so the intents are converted to board Status=Ready on the same run. Idempotent:
// if the queue is already at or above target, nothing is written. Issues with the `blocked` label
// are skipped. readyCount counts only *consumable* Ready leaves (countConsumableReady): OPEN
// work-items (no epic/story/plan) that are Status=Ready or carry a `promote:ready` intent and are
// NOT already claimed (`status: in progress`) — i.e. exactly what the hourly consumer can pick up.
// note: off-board issues carrying `promote:ready` are not counted (they are not in board().values()),
// so readyCount may undercount by ≤1 — over-nomination by 1 is benign.
function cmdTopup(argv) {
  const dryRun = !argv.includes("--fix");
  let target = Number(process.env.GHP_TOPUP_TARGET) || TOPUP_TARGET_DEFAULT;
  for (let i = 0; i < argv.length; i++) if (argv[i] === "--target") { target = Number(argv[++i]); if (isNaN(target)) die("topup: --target requires a numeric argument"); }
  // Early exit: target ≤ 0 means no work regardless of board state.
  if (target <= 0) {
    console.log(`target: ${target} · deficit: 0 · nominated: 0`);
    return;
  }
  if (!dryRun) for (const repo of MIRROR_REPOS) ensureLabels(repo);

  const b = board();
  const readyCount = countConsumableReady([...b.values()]);

  const candidates = [...b.values()].filter((row) =>
    row.state === "OPEN" &&
    isWorkItem(row) &&
    (row.status === "Backlog" || row.status == null) &&
    !row.labels.includes("blocked") &&
    !row.labels.includes(PROMOTE_INTENT_LABEL)
  );
  candidates.sort((a, b) => {
    const pa = a.labels.find((l) => l.startsWith("priority: "));
    const pb = b.labels.find((l) => l.startsWith("priority: "));
    return pRank(pa ? pa.replace("priority: ", "") : null) - pRank(pb ? pb.replace("priority: ", "") : null);
  });

  const result = planTopup(candidates, readyCount, target);
  for (const row of result.toNominate) {
    const p = row.labels.find((l) => l.startsWith("priority: ")) ?? "no priority";
    console.log(`${dryRun ? "[dry-run] " : ""}topup: ${row.key} (${p}) → promote:ready`);
    if (!dryRun) {
      gh(["issue", "edit", String(row.num), "--repo", `${OWNER}/${row.repo}`, "--add-label", PROMOTE_INTENT_LABEL]);
    }
  }
  console.log(`Ready: ${readyCount} · target: ${target} · deficit: ${result.deficit} · nominated: ${result.toNominate.length}`);
  if (dryRun && result.toNominate.length > 0) console.log("(dry-run; pass --fix to nominate)");
}

// promote [--fix] — convert `promote:ready` intents into board Status=Ready. This is the
// privileged half of the promotion pipeline: the judge (a proxy-bound cloud routine, a human, or
// a future rule) can only add the intent label over REST; this command, run where Projects v2 is
// reachable (the board-sync GitHub Action, or locally), does the board write it cannot. Idempotent
// and safe to re-run: already-Ready / advanced / closed issues just have the stale intent cleared.
function cmdPromote(argv) {
  const dryRun = !argv.includes("--fix");
  if (!dryRun) for (const repo of MIRROR_REPOS) ensureLabels(repo); // self-heal so the intent label exists
  const plan = planPromotions(intentRows());
  for (const p of plan) {
    console.log(`${dryRun ? "[dry-run] " : ""}${p.action}: ${p.key}${p.promote ? ` ${p.from ?? "—"}→Ready` : ` (Status=${p.from ?? "—"}, clear intent)`}`);
    if (dryRun) continue;
    if (p.promote) {
      const itemId = ensureOnBoard(p.repo, p.num, false);
      setSingleSelect(itemId, "Status", "Ready", false);
      setStatusLabel(p.repo, p.num, "ready", false);       // mirror immediately; reconcile also keeps it in sync
    }
    removeLabel(p.repo, p.num, PROMOTE_INTENT_LABEL);        // intent consumed either way
  }
  const promoted = plan.filter((p) => p.promote).length;
  console.log(`${plan.length} intent${plan.length === 1 ? "" : "s"} · ${promoted} ${dryRun ? "would be " : ""}promoted · ${plan.length - promoted} cleared`);
  if (dryRun && plan.length) console.log("(dry-run; pass --fix to promote)");
}

function cmdReconcile(argv) {
  const dryRun = !argv.includes("--fix");
  if (!dryRun) for (const repo of MIRROR_REPOS) ensureLabels(repo); // self-heal mirror set (#335)
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
  // Status-label mirror stale (board Status ≠ ready/status: in progress label). This is the
  // routines' only readiness signal — without it a board-Ready issue is invisible to them. (#335)
  for (const row of board().values()) {
    const want = statusMirrorWant(row);
    const have = row.labels.find((l) => STATUS_MIRROR_LABELS.includes(l)) || null;
    if ((want || null) !== (have || null)) {
      issues.push(`status-mirror-stale: ${row.key} label=${have ?? "—"} want=${want ?? "—"}`);
      if (!dryRun) setStatusLabel(row.repo, row.num, want, false);
    }
  }
  // Open bug with no priority
  for (const e of r.bugs) {
    if (!e.row.priority && !e.row.labels.some((l) => l.startsWith("priority: ")))
      issues.push(`bug-unprioritised: ${e.row.key}`);
  }
  // Unlinked non-bug
  for (const u of r.unlinked) issues.push(`orphaned-could-get-lost: ${u.row.key}`);
  // Stories under no epic — release can't be derived until they are linked
  for (const s of computeReleaseRollup().orphanStories)
    issues.push(`orphan-story-no-epic: ${STORY_REPO}#${s.num}`);
  // Leaf work items with no size — bands weight on this; assign one at triage (report-only)
  for (const row of unsizedLeaves()) issues.push(`unsized: ${row.key}`);
  console.log(issues.length ? issues.join("\n") : "no drift");
  if (dryRun && issues.length) console.log("\n(dry-run; pass --fix to repair closed-not-done + rollup-stale + status-mirror)");
}

function help() {
  console.log(`gh-project — story-driven priority for SRS Project #${PROJECT_NUMBER} (${OWNER})

  fields                          dump project field/option/iteration IDs
  ensure-fields [--dry-run]       create the MoSCoW field if missing
  ensure-labels [--repo R] [--dry-run]
                                  create the plain-label mirror set (ready, priority: P0/P1/P2,
                                  status: in progress) in all ecosystem repos (or one --repo)
  board [--repo R --status S --iteration N --open]
  add <repo> <issue#> [--dry-run]
  stories sync [--dry-run]        add open user-story issues to the board
  story set <num> --moscow <M> [--release <ms>]
  tree [<story#>]                 sub-issue tree — one story, or (no arg) the whole board by epic
  rollup [--fix]                  derive impl priority from stories (dry-run by default)
  summary [--repo R --release X --brief]   priority estimates with the calculation stages
  explain <repo> <issue#>         stage-by-stage derivation for one issue
  coverage                        bugs-ASAP + unlinked + uncovered + orphan-stories audit (JSON)
  epics                           roadmap: epics (= releases) by Priority, with coverage
  epic set <num> --priority P [--release R]   an epic's roadmap rank + release identity
  epic add-story <epic#> <story#>            link a story under an epic (sub-issue)
  link <parent-repo>#<n> <child-repo>#<n>    generic sub-issue link (cross-repo, REST) — parent a
                                             filed issue under the story/epic it serves
  release-sync [--dry-run]        derive each descendant's Release from its epic (writes; --dry-run previews)
  set <repo> <issue#> [--status --priority --iteration] [--dry-run]
  promote [--fix]                 promote every \`promote:ready\`-labelled issue to board Status=Ready
                                  (the privileged half of promotion; a REST-only judge adds the
                                  intent label, this converts it — run in CI/local, not the routines)
  topup [--fix] [--target N]      keep Ready queue at target depth (default 3, GHP_TOPUP_TARGET)
                                  by writing \`promote:ready\` to the highest-priority unblocked
                                  Backlog leaves; skips \`blocked\` issues; \`promote\` converts intents
  size <repo> <issue#> <small|medium|large|xl> [--dry-run]   effort estimate (label + board Size field)
  bands [--count N] [--tree] [--assign] [--dry-run]
                                  implementation order in N equal-effort bands (default 10);
                                  --assign writes band k → the "Band" number field (k = 1..N)
  reconcile [--fix]               report/repair board drift (priority + Status→label mirror)
  stale-claims [--hours N] [--fix]
                                  recover dead \`In progress\` claims (default 24h) — resets Status
                                  to Ready + comments; nothing else in the pipeline revisits a claim

Priority stages: served stories → MoSCoW→P → base(max) → epic fallback(−1 tier) → bug floor(P1) → bump(+1) → final.
Env: GHP_OWNER, GHP_PROJECT, GHP_STORY_REPO.
Auth: requires an authenticated \`gh\` CLI, or GITHUB_TOKEN/GH_TOKEN env var (curl fallback).`);
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------
// Pure helpers exported for unit tests. Importing the module must NOT run the CLI.
export { MIRROR_LABELS, MIRROR_REPOS, labelCreateArgs, STATUS_LABEL_MAP, STATUS_MIRROR_LABELS, statusMirrorWant, planPromotions, PROMOTE_INTENT_LABEL, epicRank, bandTargets, SIZE_WEIGHT, derivePriority, parseIssueRef, planStaleClaims, STALE_CLAIM_HOURS_DEFAULT, planTopup, TOPUP_TARGET_DEFAULT, isWorkItem, isConsumableReady, countConsumableReady };

// Only dispatch when run directly (`node gh-project.mjs ...`), not when imported.
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [cmd, ...rest] = process.argv.slice(2);
  const dry = rest.includes("--dry-run");
  try {
    switch (cmd) {
      case "fields": cmdFields(); break;
      case "ensure-fields": cmdEnsureFields(dry); break;
      case "ensure-labels": cmdEnsureLabels(rest); break;
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
      case "epics": cmdEpics(); break;
      case "epic":
        if (rest[0] === "set") cmdEpicSet(rest.slice(1), dry);
        else if (rest[0] === "add-story") cmdEpicAddStory(rest.slice(1), dry);
        else die("usage: epic set <num> --priority P [--release R] | epic add-story <epic#> <story#>");
        break;
      case "link": cmdLink(rest, dry); break;
      case "set": cmdSet(rest); break;
      case "topup": cmdTopup(rest); break;
      case "promote": cmdPromote(rest); break;
      case "size": cmdSize(rest); break;
      case "bands": cmdBands(rest); break;
      case "reconcile": cmdReconcile(rest); break;
      case "stale-claims": cmdStaleClaims(rest); break;
      case "help": case "--help": case "-h": case undefined: help(); break;
      default: die(`unknown command "${cmd}" (try \`help\`)`);
    }
  } catch (e) {
    die(e.stderr ? String(e.stderr) : e.message);
  }
}
