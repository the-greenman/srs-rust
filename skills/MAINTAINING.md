# Maintaining the srs skill

`SKILL.md` is the one file a release attaches (`release.yml`), so it must stand alone. This file is the practice for changing it. It is not shipped.

## What the skill is for

It teaches an agent that has never seen SRS to use a repository well through the tools it has. It is the last teaching channel, not the first. An agent is taught, in this order, by:

1. **The tool itself**: its behaviour, its error codes, its diagnostics.
2. **The tool description** (`srs-mcp-core`), read on every call.
3. **The server instructions**, always delivered, with the repository's own guidance (`manifest.aiGuidance`) appended.
4. **The repository**: its identity record, its outline, its type guidance.
5. **The skill**, only where the client has skills and loaded this one.

**Fix a lesson at the lowest number that can hold it.** A rule in the skill is a debt: it is paid by every agent on every load, and only by agents that have the skill.

## Rules

1. **Every rule has evidence.** Each line of `SKILL.md` that tells an agent what to do traces to a row in the ledger below: a finding id, a dated measurement or a failed test. No evidence, no rule.
2. **No bare numbers.** A figure in the skill carries its date and source, or it is left out. (The srs-context skill quoted recall 0.89 from 2026-10-07 after it had fallen to 0.667 by 2026-11-14.)
3. **One home per lesson.** A lesson that holds for any SRS repository lives here. A lesson about one repository lives in that repository's guidance or its project skill. The other place links; it does not restate. When the project skill and this one disagree, that is a bug in one of them.
4. **A size cap.** `SKILL.md` stays under 12,000 bytes. To add a rule over the cap, remove one or merge two. CI checks it.
5. **Every tool it names exists.** CI checks each MCP tool name and each `srs` subcommand in `SKILL.md` against the tool catalogue and the CLI built from the same commit. A rename fails the build. `check-skill.sh` beside this file runs rules 4 and 5. Its first run (2026-11-22, against the binary pinned for srs-context) failed on `srs repo guidance`: the command is on master and not in the pinned build.
6. **A cold agent tests every change.** Before a change merges, run the cold-agent set (below) with the old file and the new one. The new one must not score lower or cost more failed calls. Record the scores in the PR. A change that only adds calls needs a reason.
7. **Promote slowly.** A finding becomes a skill rule when it has cost an agent a wrong answer or a failed write at least twice, in two sessions, and cannot be fixed in channels 1 to 4 this month. Otherwise file the fix against the tool.
8. **Retire quickly.** When the tool fixes the cause, the rule goes in the same release, and its ledger row is marked retired with the fixing issue. Each rule names what would retire it.
9. **Reconcile contradictions before shipping.** Two records that disagree about tool behaviour are tested against the current binary, and the loser is superseded. (Findings 74561bed and 670655d6 disagree on exit codes, and `start.md` sides with one.)
10. **Measure, then word.** Wording that tells an agent to "search first" has been measured to change nothing (finding 25af154a). Putting the right record in front of the agent did. Prefer changes that alter what the agent sees over changes that alter what it is told.

## The cold-agent set

A small fixed set, cheap to run (16 Haiku runs, about 100k tokens each). Each task has a known answer, so scoring is not a judgement.

| Task | Tests | Corpus |
|---|---|---|
| T1 Orient: what is this repository, its top-level parts in order, its depth, a named record, and the record after it | reading the outline, not the summary | handbook (92 records) |
| T2 Recall: a question typed as a sentence, then the parts it applies to | match mode, reading edges | context (321) |
| T3 Plan a replacement: old id and state, the new record's field names, the call sequence | schema before write, succession, lifecycle, validate | context |
| T4 Survey a large repository: what it is, its structure, its concepts in prerequisite order, the state of one type, three safe next calls | size limits, relation keys, order read from edges | spec (738) |

Score each run on: answer correct; no guessed id or URI (count failed calls and empty results); every claim traceable to a call; calls used. Put the prompts, the answers and the scores in the PR that changes the skill.

**T1 to T3 are a smoke test. T4 is the only task that separates a good run from a poor one.** Add a task when a new failure class appears. Demote a task to smoke when two releases pass it at ceiling. Tasks still to write: a real write on a scratch repository (a plan is not a write); a repository whose summary is wrong; a question that spans two repositories.

The harness must give both arms the same tools and the same context. An agent run inside a project that already explains SRS is not cold: say so in the result. A cold harness is one `srs mcp serve` over a single repository, with no project instructions in the agent's context.

### First run, 2026-11-22

Old file (18 lines of links, no web access, so in effect no skill) against the new draft. Haiku, two runs per cell, read-only, through the registry server. **Not cold**: every agent carried this project's instructions. Call logs are self-reported.

| Task | Arm | Correct | Failed calls | Empty results | Calls |
|---|---|---|---|---|---|
| T1 | old | 2 of 2 | 0 | 0 | 4, 2 |
| T1 | new | 2 of 2 (one misstated a depth) | 0 | 0 | 5, 5 |
| T2 | old | 2 of 2 | 0 | 0 | 2, 3 |
| T2 | new | 2 of 2 | 0 | 0 | 3, 2 |
| T3 | old | 2 of 2 (one left the transition "not established") | 0 | 0 | 4, 6 |
| T3 | new | 2 of 2 (both read the allowed transitions and the existing edges) | 0 | 0 | 6, 6 |
| T4 | old | partial: prerequisite order not established (1), read from `precedes` (1) | 2, 2 | 0 | 7, 10 |
| T4 | new | partial: prerequisites read from `depends-on` (1), structure from `contains` (1) | 1, 2 | 4, 1 | 15, 13 |

What it shows:

- **The new file did not raise a score.** T1 to T3 sat at ceiling in both arms. Agents already use `match: "any"`, `neighbours` and `type_schema`, because the tool descriptions and the server instructions teach them. That is rule "fix at the lowest channel" working.
- **The new file changed behaviour, and cost calls.** With it, agents read relations before stating an order (T4) and checked transitions before planning a write (T3). They also made 5 empty relation reads, because the draft told them to read `relation-types` and did not say to copy the `key`. The draft was corrected.
- **The test found tool gaps no skill should carry.** Every T4 run failed on `navigation` (268,918 bytes on the spec; the read tool cuts at 96,000 and the client still refused the result; the CLI has `--depth`, the resource does not). `neighbours` with an unknown relation type returns zero edges and no warning. `container_outline` on a record id says only "container not found".
- **No run invented an id or a fact.** The earlier zero-context failure (bbe9faf9) did not recur, with either file.

Recorded as findings 3dfc2f9e (the result), f602af00 (silent failures) and 0381f65a (`navigation`) in srs-context.

## The loop

1. **Capture.** An agent that hits friction records a finding in the project's memory, with a `friction_category` and the error code. (Only 17 of 153 findings carried a category on 2026-11-20, and the usage log keeps no error code. Both need fixing for this loop to have data.)
2. **Classify, monthly.** For each new finding about using SRS: which channel should have taught it? Tool, description, instructions, repository, or skill.
3. **Fix at the lowest channel.** File the issue there.
4. **Promote** to the skill only under rule 7.
5. **Test** with the cold-agent set.
6. **Retire** under rule 8.

The dream cycle in srs-context is the natural place for step 2: it already re-verifies records on a schedule.

## Where this sits in the readability research

This practice is one part of the thread "Readable repositories, tested" in srs-programme's Research section (363fc3c3). It does not restate that thread.

- The cold-agent set is the audit's check H2 made repeatable. Each run is recorded there as a dated finding that evidences the problem it bears on (first run: f316ae52).
- The thresholds in the skill's "Shape" section (2 to 9 areas, about 15 children) are the audit's S1 and S4. Their home is `structure.md`. The skill carries the short form and changes when that page does.
- Those thresholds were chosen, not measured (part 6 of the thread, 683cf65a). A cold-agent task that passes on a repository the audit fails, or fails on one it passes, is evidence for moving a threshold. Record it there.
- Shaping one repository is the audit scorecard's work (part 2, 0872ba27). This file only governs what the skill says to every repository.

## Ledger

One row per rule in `SKILL.md`. "Retire when" names the lower-channel fix.

| Rule | Evidence | Retire when |
|---|---|---|
| Never edit files; no look-alike JSON | finding 6cd8fdeb (mimicry) | never: it is the contract |
| Never guess an id, URI, type or relation type | zero-context test bbe9faf9 (3 of 7 calls on `srs://default/`; core package id reported as a composition id), 2026-11 | the `srs://default/` error lists the valid ids; `map` labels each id's role |
| Never state a link, order or state from a summary | zero-context test bbe9faf9 (order asserted with no relation read); A/B 2026-10-07 task 4 (wrong answer from the brief, 0 calls); finding c6832790 (38 ratified decisions read `status: proposed`) | not expected |
| Sentence queries need `match: "any"` | finding 085233e1; recall@5 0.083 to 0.89, 2026-10-06/07; spot-check 2026-11-22 (0 hits, no diagnostic, against 138) | `find` warns when an all-words query of 4 or more words returns zero, or defaults to `any` |
| `find` with `limit: 0` is the map | spot-check 2026-11-22 | never: it is a feature |
| `card` projection, `fields` | findings 030997a9, 03eb5d28, 221efa89 (response size); card about 65% of full, 2026-11-22 | `card` becomes the default |
| Relations are not found with `find` | zero-context test bbe9faf9; spot-check 2026-11-22 | the `find` warning for `typeName: "relation"` points to `neighbours` |
| Relation type is its `key`, copied exactly | cold run 2026-11-22: 5 empty `neighbours` reads from `com.semanticops.srs/depends-on` (the key is `depends-on`) | `neighbours` warns on an unknown relation type |
| `navigation` and `tree` are cut off on a large repository | cold run 2026-11-22: 4 of 4 runs failed on the spec's `navigation` | the resource takes a depth bound (srs-rust#1490) |
| `container_outline` takes a container id | cold run 2026-11-22: "container not found" on a section record's id | the error says the id is a record and names its container |
| Ids are full UUIDs | spot-check 2026-11-22: `neighbours` on an 8-character prefix returned "not found" | a prefix resolves, or the error says so |
| `agent-index` is flat; `tree` follows `contains` only | spot-checks 2026-11-22 (handbook: 92 roots in `tree`, depth 0 to 2 in the outline) | `agent-index` carries depth; `tree` says when a repository has no `contains` edges |
| Whole-object update | srs-context skill; finding fced5d15 (container membership replaced wholesale) | a patch form exists |
| Succession, not deletion; transitions, not a status field | finding c6832790 | not expected |
| Search before create | survey 2026-11-21: programme 395 titles shared by 795 instances | `record_create` warns on a same-type same-title record |
| Never send `createdBy` | RFC-046 (`actor-supplied`) | never |
| Validate by `summary.errors == 0`; check `ok`, not the exit code | findings 74561bed, 670655d6 | exit codes are reconciled |
| Server may be older than this file | finding e6f9b6ba; check 2026-11-22: `srs repo guidance` missing from the pinned build | the server reports its version in `initialize` and the skill names a minimum |
| Set the repository's guidance (what it says: `GUIDANCE.md`) | scan 2026-11-22: set in 1 of 9 served repositories (finding 731e1662) | `repo validate` warns when it is empty |
| Type, state, relation, container, tag table | survey 2026-11-21: tags as entities (2,516 distinct in one corpus), a container per state (721 and 410 members), `from:<repo>:<id>` tags as edges | validation warns on each |
| Hubs and no isolated records | survey 2026-11-21: 0% isolated in the derivation-chain corpus, 57% and 86% in the flat ones; usage log: 55% of tier-2 records never read | not expected |
| Short fields, bounded containers, no unused types | survey 2026-11-21: fields of 97,712 and 75,453 chars; 40 unused types | `repo doctor` reports them |
| No relation across repositories; share types by package | RFC-049 R2 and R11; survey: `decision` defined with 3 type ids in 5 repositories; prototype 2026-11-22: a relation to a record in another repository is refused (`relation-endpoint-unknown`), a slice of one repository will not merge into another (`merge-repository-mismatch`), and one exported package installed in two repositories with the dependency reported `satisfied` | a cross-repository relation is accepted in the spec |
