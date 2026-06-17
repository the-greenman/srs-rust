# SCDS Governance Application Profile

**Version:** 0.1-draft  
**Status:** example profile  
**Conformance target:** `SCDS 2.0 Core + ext:lifecycle + ext:protocol + ext:schema + ext:views-l1 + ext:views-l2 + ext:addressability + ext:recommended-relations + ext:repeatable-fields + ext:field-groups`

## 1. Purpose

This profile defines a reusable governance vocabulary for groups that need to deliberate, record decisions, preserve unresolved thinking, and maintain durable governance documents over time.

It is intended for:

- team decision logs
- cooperative governance records
- project foundation documents
- policy registers
- meeting-to-decision workflows
- conversational document creation
- AI-assisted facilitation and review

The profile is not a governance method. It does not require consensus, voting, consent, sociocracy, direct democracy, or delegated authority. It provides semantic structures that allow a group to record what happened, what was decided, why it was decided, what was not chosen, who had authority, and when the matter should be revisited.

## 2. Core principle

Governance records should separate four things that are often collapsed into one document:

1. **Exercise** — live thinking, unresolved questions, tensions, and partial understanding.
2. **Decision** — a settled commitment or direction of action.
3. **Ratification** — the act that gives a decision legitimacy in a particular governance context.
4. **Article / Policy** — a durable rule, boundary, or standing commitment.

A meeting is not the owner of a decision. A meeting is a context in which a decision may be discussed, formed, ratified, deferred, or revisited.

## 3. Required SCDS extensions

### Required

| Extension | Reason |
|---|---|
| `ext:lifecycle` | Decisions, exercises, articles, and ratifications need explicit states. |
| `ext:protocol` | Governance work is deliberative and stage-based, not just form filling. |
| `ext:schema` | A governance document needs a definition of what record types it contains. |
| `ext:views-l1` | Individual records need standard renderings and editing views. |
| `ext:views-l2` | Governance documents are assembled from multiple records. |
| `ext:addressability` | Conversation chunks need to be linked to active record and field focus. |
| `ext:recommended-relations` | Supersession, derivation, ratification, and containment need interoperable relation types. |

### Recommended

| Extension | Reason |
|---|---|
| `ext:repeatable-fields` | Decisions may have multiple affected groups, alternatives, owners, or review triggers. |
| `ext:field-groups` | Contacts, participants, votes, objections, and approvals may need structured repeated entries. |
| `ext:cross-field-validation` | Useful for policy constraints, dates, and conditional fields. |
| `ext:import-tracking` | Useful when importing standard governance packages. |
| `ext:registry` | Useful for publishing shared governance vocabularies. |

## 4. Type catalogue

This profile defines the following core Types:

| Type | Semantic object |
|---|---|
| `governance/article` | A durable governance rule, boundary, or standing commitment. |
| `governance/role` | A named authority boundary held by a person, group, or function. |
| `governance/decision` | A settled commitment, choice, policy, or direction of action. |
| `governance/exercise` | Unresolved thinking or deliberation still in motion. |
| `governance/ratification` | The act by which a decision is confirmed or legitimised. |
| `governance/agenda_item` | A topic brought into a session for discussion. |
| `governance/review` | A scheduled or triggered reassessment of a record. |
| `governance/agent_note` | A human or AI-authored comment, recommendation, warning, or challenge. |
| `governance/policy_reference` | A reference to external policy, law, precedent, or organisational rule. |

## 5. Shared Fields

These Fields are reused across multiple Types.

### `governance/title`

```yaml
name: title
valueType: string
description: Short human-readable name for the record.
```

### `core/description`

```yaml
name: description
valueType: text
description: Short explanation of what this record concerns.
```

### `governance/status`

```yaml
name: status
valueType: select
selectOptions:
  - draft
  - active
  - proposed
  - deferred
  - superseded
  - closed
  - rejected
  - archived
description: Current governance status of the record.
```

### `governance/context`

```yaml
name: context
valueType: text
description: Background that makes the matter meaningful.
aiGuidance:
  purpose: Capture the relevant background, tension, or situation that led to this record.
  extraction: Summarise only the context required to understand the governance issue. Do not include the decision itself unless it is needed to explain the background.
```

### `governance/rationale`

```yaml
name: rationale
valueType: text
description: Reasoning behind a decision, article, role, or recommendation.
aiGuidance:
  purpose: Capture why this record exists or why the proposed course was chosen.
  extraction: Extract the reasons, forces, values, constraints, and trade-offs that support the record.
  negativeGuidance: Do not invent reasons that were not present in the source material.
```

### `governance/alternatives_considered`

```yaml
name: alternatives_considered
valueType: text
description: Options that were considered but not chosen.
aiGuidance:
  purpose: Preserve the meaningful alternatives, rejected options, or counter-proposals.
  extraction: List alternatives that were seriously considered, including why they were not chosen where available.
```

### `governance/revisit_when`

```yaml
name: revisit_when
valueType: text
description: Conditions, dates, or triggers that should cause the record to be reviewed.
aiGuidance:
  purpose: Capture when this record should be revisited, reviewed, or challenged.
  extraction: Extract explicit review dates, triggers, expiry conditions, or evidence that would change the decision.
```

### `governance/owner`

```yaml
name: owner
valueType: string
description: Person, role, group, or function responsible for stewardship of the record.
```

### `governance/source_summary`

```yaml
name: source_summary
valueType: text
description: Short summary of the source material that informed the record.
```

### `governance/friction`

```yaml
name: friction
valueType: text
description: The specific pain, conflict, or gap that makes a decision necessary now.
aiGuidance:
  purpose: Capture what is not working and why acting matters. Distinct from background context — friction is the pressure, not the history.
  extraction: Extract the specific gap between current state and desired state, or the conflict or issue forcing a choice. Do not conflate with background or context.
  negativeGuidance: Do not include history or constraints here. Those belong in context.
```

### `governance/decision_question`

```yaml
name: decision_question
valueType: string
description: The single concrete question this decision answers, in a form that admits different answers.
aiGuidance:
  purpose: Make the exact choice being made explicit and testable. A good decision question can be answered in more than one way.
  extraction: Extract or synthesise the specific question. Prefer "Should we… or…?" form. If the source material only states a conclusion, reconstruct the question that conclusion answers.
  negativeGuidance: Do not phrase as a task or action item. "Should we adopt X or Y?" not "Implement X."
```

### `governance/key_requirements`

```yaml
name: key_requirements
valueType: text
description: The standards, values, constraints, and trade-offs used to evaluate options. Includes non-negotiables and recorded disagreements.
aiGuidance:
  purpose: Record what a good decision looks like for this problem — not just criteria but the values applied and the trade-offs accepted.
  extraction: Extract constraints (budget, deadline, legal), values applied (e.g. cost vs quality), trade-offs accepted, and significant points of disagreement among participants.
```

### `governance/next_steps`

```yaml
name: next_steps
valueType: text
description: Actions or tasks that emerge from the decision. May be empty if the decision itself is the action.
aiGuidance:
  purpose: Capture what still needs to happen after the decision is made.
  extraction: List named actions with owners where stated. If none are present, leave empty.
```

### `governance/article_text`

```yaml
name: article_text
valueType: text
contentFormat: plain
description: The operative text of the article — what the article actually says.
aiGuidance:
  purpose: Capture the durable, authoritative wording of the governance rule or commitment.
  negativeGuidance: Do not summarise or paraphrase. Article text is the thing itself, not a description of it.
```

### `governance/article_number`

```yaml
name: article_number
valueType: string
description: Stable local reference identifier for the article, e.g. A-001. Used for citations and cross-references within the repository.
```

### `governance/amendment_rule`

```yaml
name: amendment_rule
valueType: text
contentFormat: plain
description: The process or conditions under which this article may be amended or repealed.
aiGuidance:
  purpose: Make the constraints on changing this article explicit, so future amendments follow the agreed process.
```

### `governance/protected_status`

```yaml
name: protected_status
valueType: string
description: Whether this article is protected from ordinary amendment by the group — for example, because it requires external consent.
aiGuidance:
  purpose: Flag articles that require consent beyond the normal group process to amend, such as a Party Utility agreement or external authority.
```

### `governance/role_holder`

```yaml
name: role_holder
valueType: string
description: The person, team, group, or function currently holding this role.
aiGuidance:
  purpose: Name who holds this authority boundary at the time of writing. Roles may be unoccupied or transferred without amendment.
```

### `governance/authority`

```yaml
name: authority
valueType: text
contentFormat: plain
description: What this role may decide or act on without requiring group agreement.
aiGuidance:
  purpose: Define the scope of unilateral authority. What is this role holder empowered to decide alone?
  extraction: Extract the specific decisions, actions, or domains this role holder can act on independently.
  negativeGuidance: Do not conflate with boundary (what the role cannot do). Authority is what they can do.
```

### `governance/boundary`

```yaml
name: boundary
valueType: text
contentFormat: plain
description: The limits of this role — where the role's authority ends or must be shared.
aiGuidance:
  purpose: Define what the role holder cannot decide alone, and where authority overlaps with or defers to others.
  negativeGuidance: Do not restate the authority here. Boundary is what the role cannot do unilaterally.
```

### `governance/source_of_authority`

```yaml
name: source_of_authority
valueType: string
description: How this authority was conferred — inherited, delegated, elected, appointed, constitutional, etc.
aiGuidance:
  purpose: Make the legitimacy of the role visible. Where does this authority come from?
```

## 6. Type definitions

## 6.1 Article

An Article is a durable governance rule, boundary, constitutional clause, or standing commitment.

### Fields

| Field | Required | Notes |
|---|---:|---|
| `governance/title` | yes | Article title. |
| `governance/article_number` | recommended | Stable local reference, e.g. `A-005`. |
| `governance/article_text` | yes | The operative article text. |
| `governance/rationale` | no | Why the article exists. |
| `governance/amendment_rule` | recommended | How this article can be amended. |
| `governance/protected_status` | no | Whether the article is protected from ordinary amendment. |
| `governance/status` | yes | Usually `active`, `superseded`, or `archived`. |
| `governance/revisit_when` | no | Review condition, if any. |

### Lifecycle

```text
draft → proposed → active → superseded
```

An active Article should not be edited silently if the change alters meaning. A material change creates a new Article Record and a `supersedes` or `amends` Relation.

## 6.2 Role

A Role defines an authority boundary. It is not necessarily a person record.

### Fields

| Field | Required | Notes |
|---|---:|---|
| `governance/title` | yes | Role title. |
| `governance/role_holder` | no | Person, team, group, or function currently holding the role. |
| `governance/authority` | yes | What this role may decide or act on. |
| `governance/boundary` | yes | Limits of the role. |
| `governance/source_of_authority` | recommended | Inherited, delegated, elected, appointed, constitutional, etc. |
| `governance/status` | yes | Draft, proposed, active, superseded, closed. |
| `governance/revisit_when` | no | When the role should be reviewed. |

### Lifecycle

```text
draft → proposed → active → closed
```

If the authority boundary changes materially, create a new Role Record and link it to the old one with `supersedes` or `refines`.

## 6.3 Exercise

An Exercise Record captures unresolved thinking. It is not a failed decision.

### Fields

| Field | Required | Notes |
|---|---:|---|
| `governance/title` | yes | Topic under consideration. |
| `governance/thinking_reached` | yes | Where the group’s thinking has got to. |
| `governance/tensions` | no | Tensions, disagreements, or competing concerns. |
| `governance/unresolved_questions` | no | What remains open. |
| `governance/blocking` | no | What prevents closure. |
| `governance/next_action` | recommended | Next step or owner. |
| `governance/status` | yes | Open, deferred, converted, closed. |
| `governance/revisit_when` | no | When to return to the exercise. |

### Lifecycle

```text
open → deferred → converted → closed
```

A Decision may be `derived-from` an Exercise. The Exercise remains part of the deliberation record.

## 6.4 Decision

A Decision is a settled commitment, choice, policy, or direction of action.

### Fields

| Field | Required | Notes |
|---|---:|---|
| `governance/title` | yes | Short decision title. |
| `governance/decision_question` | recommended | The single concrete question this decision answers. |
| `governance/context` | recommended | History, constraints, and triggering event. |
| `governance/friction` | recommended | The specific pain or gap that made this decision necessary. |
| `governance/alternatives_considered` | recommended | What was not chosen, including doing nothing. |
| `governance/key_requirements` | recommended | Criteria, values, trade-offs, and recorded disagreements used to evaluate options. |
| `governance/decision_statement` | yes | What was decided. Active voice, one sentence. |
| `governance/rationale` | recommended | The deciding factor; why this option won over the others. |
| `governance/dissent_or_reservations` | no | Calibrated dissent, unresolved concerns, or standing-aside notes. |
| `governance/revisit_when` | recommended | Review dates, triggers, or conditions that would invalidate the decision. |
| `governance/next_steps` | no | Actions or tasks that emerge from this decision. |
| `governance/owner` | no | Person, role, or group responsible for follow-through. |
| `governance/status` | yes | Draft, proposed, ratified, closed, superseded, rejected. |

### Lifecycle

```text
draft → proposed → ratified → closed → superseded
```

Closed Decisions are immutable in this profile. A material change creates a new Decision Record linked with `supersedes`, `amends`, or `refines`.

### Minimal decision

A minimal Decision only needs:

```text
title
decision_statement
status
```

A durable Decision should include context, friction, decision question, alternatives considered, key requirements, rationale, and revisit conditions.

## 6.5 Ratification

A Ratification Record captures how a Decision became legitimate in a particular governance context.

### Fields

| Field | Required | Notes |
|---|---:|---|
| `governance/ratification_method` | yes | Vote, consensus, consent, delegated authority, unanimous agreement, etc. |
| `governance/ratification_outcome` | yes | Passed, failed, deferred, withdrawn. |
| `governance/threshold` | no | Required threshold or consent rule. |
| `governance/eligible_participants` | no | Who was entitled to participate. |
| `governance/result_summary` | recommended | Human-readable summary of the result. |
| `governance/ratified_at` | recommended | Date/time of ratification. |
| `governance/status` | yes | Proposed, active, superseded, rejected. |

### Relations

```text
Ratification --ratifies--> Decision
```

Ratification may be embedded as a Field Group for lightweight use, but should be a separate Record where auditability matters.

## 6.6 Agenda Item

An Agenda Item is a session topic. It may produce Exercises, Decisions, Reviews, or no durable record.

### Fields

| Field | Required | Notes |
|---|---:|---|
| `governance/title` | yes | Agenda topic. |
| `governance/context` | no | Background. |
| `governance/intended_outcome` | no | Discuss, decide, review, inform, explore. |
| `governance/status` | yes | Open, in-progress, deferred, closed. |

### Relations

```text
AgendaItem --contains--> Exercise
AgendaItem --contains--> Decision draft
Decision --derived-from--> AgendaItem
Exercise --derived-from--> AgendaItem
```

Agenda Items are session scaffolding. Durable Decisions belong to the team or governance Container, not to the meeting.

## 6.7 Review

A Review Record captures an explicit revisit of an earlier record.

### Fields

| Field | Required | Notes |
|---|---:|---|
| `governance/title` | yes | Review title. |
| `governance/review_subject` | yes | Human-readable reference to the thing reviewed. |
| `governance/review_reason` | yes | Scheduled review, trigger, challenge, expiry, new evidence. |
| `governance/review_finding` | yes | Continue, amend, supersede, close, reopen. |
| `governance/next_action` | no | Follow-up. |
| `governance/status` | yes | Draft, active, closed. |

### Relations

```text
Review --reviews--> Decision | Article | Role | Exercise
Review --produces--> Decision | Article | Exercise
```

`reviews` and `produces` are custom profile relation types unless adopted as common relations.

## 6.8 Agent Note

An Agent Note captures an AI or human comment, recommendation, warning, challenge, or proposed improvement.

### Fields

| Field | Required | Notes |
|---|---:|---|
| `governance/title` | yes | Short note title. |
| `governance/note_body` | yes | The recommendation, challenge, or observation. |
| `governance/note_type` | yes | Clarification, policy warning, split suggestion, contradiction, evidence request, improvement. |
| `governance/agent_identity` | no | AI agent, human participant, policy system, clarity coach. |
| `governance/status` | yes | Proposed, accepted, rejected, resolved. |

### Relations

```text
AgentNote --relates-to--> Decision draft
AgentNote --supports--> Field value
AgentNote --contradicts--> Field value
```

A lightweight annotation may be enough for transient comments. Use Agent Note Records when the comment may affect semantic state or should remain auditable.

## 7. Recommended relation types

Use SCDS canonical relation types where possible:

| Relation | Use |
|---|---|
| `contains` | Agenda contains discussion items; project contains records. |
| `derived-from` | Decision derived from Exercise or transcript-supported Note. |
| `refines` | Newer, clearer version of a rougher record. |
| `supersedes` | New record replaces an older authoritative record. |
| `depends-on` | One action, decision, or article depends on another. |
| `precedes` | Temporal or procedural order. |
| `evidences` | Source material supports a claim. |

Profile-specific relation types:

| Relation | Meaning |
|---|---|
| `governance/amends` | Source record amends target record without replacing it fully. |
| `governance/ratifies` | Ratification confirms a Decision. |
| `governance/delegates` | Article, Decision, or Role grants authority to another Role. |
| `governance/reviews` | Review assesses a prior Record. |
| `governance/produces` | Review, Protocol, or Agenda Item produces a resulting Record. |
| `governance/challenges` | Agent Note or participant note challenges a Record or Field value. |

## 8. Protocols

## 8.1 Brain Dump Protocol

Loose protocol for getting raw thinking out before structure is known.

### Stages

| Stage | Question | Output |
|---|---|---|
| `open_space` | What is on people’s minds about this topic? | Note |
| `cluster` | What themes or areas are emerging? | Typed Records or candidate Records |
| `name_components` | What are the major components we need to return to? | Component Notes / Agenda Items |
| `next_focus` | Which component should be worked on next? | Agenda Item or Exercise |

## 8.2 Exercise Review Protocol

Used when returning to an unresolved matter.

### Stages

| Stage | Question | Output |
|---|---|---|
| `recap` | What was previously established? | Exercise update |
| `check_change` | What has changed since then? | Exercise update |
| `identify_blockers` | What is still unresolved or blocking closure? | Exercise update |
| `choose_next_step` | Are we ready to decide, defer, split, or close? | Decision draft / updated Exercise |

## 8.3 Decision Capture Protocol

Used when a decision is already clear and just needs recording.

### Stages

| Stage | Question | Output |
|---|---|---|
| `state_decision` | What exactly did we decide? | Decision statement |
| `check_scope` | Is this one decision or several? | Decision draft or split suggestion |
| `record_reason` | What minimal reason should future readers know? | Rationale |
| `set_revisit` | What would make us revisit this? | Revisit condition |
| `ratify` | How was this confirmed? | Ratification Record or ratification fields |

## 8.4 Decision Deliberation Protocol

Used when a decision is not yet clear. This protocol moves a group through a reliable sequence that produces a Decision Record with enough recorded reasoning to remain legible months later.

### Stages

| Stage | Guiding question | Field | Output |
|---|---|---|---|
| `background` | What happened before? | `governance/context` | History, constraints, and the trigger that started the conversation. |
| `friction` | What's the problem we need to fix? | `governance/friction` | The specific pain or gap that makes acting necessary. |
| `decision_question` | What exactly are we deciding? | `governance/decision_question` | One concrete answerable question. |
| `alternatives` | What were our other choices? | `governance/alternatives_considered` | All genuine options considered, including doing nothing. |
| `key_requirements` | What matters most to us here? | `governance/key_requirements` | Criteria, values, trade-offs, and recorded disagreements. |
| `verdict` | What did we decide? | `governance/decision_statement` | One clear active-voice sentence. |
| `why` | Why did this option win? | `governance/rationale` | The deciding factor; how to explain it to someone not present. |
| `review_triggers` | When should we look at this again? | `governance/revisit_when` | Specific dates, conditions, or signals that would reopen the question. |
| `next_steps` | What's still left to do? | `governance/next_steps` | Actions with owners; may be empty. |

After `next_steps`, ratification follows the standard pattern: the group confirms the decision using their chosen ratification method, producing a Ratification Record if auditability requires it.

### Facilitation notes

**If the discussion goes in circles:** move to `decision_question` and name the actual choice: "It sounds like we are stuck — what is the specific option we are choosing between right now?"

**If the group says "we need more information":** surface this as a strategic decision rather than a blocker. "Are we deciding to delay until we have more information? If so, what is the review trigger — what information, by when?" This produces a Decision to defer, not a failed session. Record it as a Decision with `governance/revisit_when` set, or as an Exercise if the matter is genuinely unresolved.

## 8.5 Article Amendment Protocol

Used when changing a durable governance article or standing rule.

### Stages

| Stage | Question | Output |
|---|---|---|
| `identify_article` | Which article or rule is being changed? | Relation to Article |
| `state_problem` | Why is the current wording no longer sufficient? | Rationale |
| `draft_change` | What new wording is proposed? | New Article draft |
| `check_authority` | Who is allowed to approve this change? | Ratification requirement |
| `compare_versions` | What changes between old and new? | Review / amendment summary |
| `ratify` | Has the amendment been validly approved? | Ratification |
| `supersede` | Does this replace or amend the old article? | `supersedes` or `governance/amends` Relation |

## 8.6 Founding Document Protocol

Used when a new governance repository is being established — typically at or shortly after a founding meeting. Articles created through this protocol are constitutive acts written by the designated clerk: they express what the group agreed in spirit, not what a transcript literally contains. Source is `"human"` throughout; `sourceRefs` are not expected.

### Stages

| Stage | Guiding question | Output |
|---|---|---|
| `identify_scope` | What kind of entity is this, and what does it govern? | Article 1 draft — what this is |
| `state_purpose` | What are we here to do, and what are we explicitly not? | Article 2 draft — purpose |
| `name_members` | Who is the founding group, and how do new members join? | Article 3 draft — members; Role records |
| `define_authorities` | Who can decide what, and on what basis? | Role records for each authority boundary |
| `set_commitments` | What standing obligations apply — to the space, to others, to the work? | Article 4–5 drafts — care and commitments |
| `name_end` | What happens if this ends? How do we close down cleanly? | Article 6 draft — dissolution clause |

### Facilitation notes

- The clerk writes Articles after the founding discussion, not during it. The group agrees on the spirit; the clerk gives it form.
- Roles should be created in parallel with Article 3 and 4. Each authority boundary named in an article should have a corresponding Role record.
- Protected articles (those requiring external consent to amend) should have `governance/protected_status` set and `governance/amendment_rule` reflect the external constraint.
- If the group cannot agree on the dissolution clause (Article 6), record the unresolved question as an Exercise rather than forcing a premature commitment.
- After drafting, read all Articles aloud or share for group review before marking them `active`.

## 9. Schemas

## 9.1 Governance Foundation Document Schema

Defines a full governance document consisting of Articles, Roles, Decisions, and Exercises.

### Root Types

```text
Article
Role
Decision
Exercise
```

### Expected Relations

```text
Decision --supersedes--> Decision
Decision --governance/amends--> Article
Ratification --governance/ratifies--> Decision
Role --governance/delegates--> Role
Decision --derived-from--> Exercise
Exercise --derived-from--> AgendaItem
```

### Completeness

A minimally complete governance foundation document contains:

```text
at least one Article
at least one Role or authority statement
one Document View defining how the records are rendered
```

A mature governance foundation document should also contain:

```text
decision log
exercise log
review rules
ratification method
supersession policy
```

## 9.2 Meeting Summary Schema

Defines a batch summary output for one meeting.

### Root Types

```text
AgendaItem
Exercise
Decision
Ratification
AgentNote
```

### Expected Relations

```text
AgendaItem --contains--> Exercise
AgendaItem --contains--> Decision
Decision --derived-from--> Exercise
Ratification --governance/ratifies--> Decision
AgentNote --relates-to--> Decision
```

### Completeness

A minimally complete meeting summary contains:

```text
meeting title/date from the host system
agenda items discussed
decisions reached
exercises left open
next actions or review points
```

## 10. Views

## 10.1 Decision Log Entry View

Renders one Decision Record.

```markdown
### {{decision_id}} — {{title}}

**Decided**  
{{decision_statement}}

{{#if rationale}}
**Why**  
{{rationale}}
{{/if}}

{{#if alternatives_considered}}
**Not chosen**  
{{alternatives_considered}}
{{/if}}

{{#if dissent_or_reservations}}
**Reservations**  
{{dissent_or_reservations}}
{{/if}}

{{#if revisit_when}}
**Revisit when**  
{{revisit_when}}
{{/if}}

**Status**  
{{status}}
```

## 10.2 Exercise Minutes View

```markdown
### {{exercise_id}} — {{title}}

**Thinking reached**  
{{thinking_reached}}

{{#if tensions}}
**Tensions**  
{{tensions}}
{{/if}}

{{#if unresolved_questions}}
**Open questions**  
{{unresolved_questions}}
{{/if}}

{{#if blocking}}
**Blocking**  
{{blocking}}
{{/if}}

{{#if next_action}}
**Next action**  
{{next_action}}
{{/if}}

**Status**  
{{status}}
```

## 10.3 Article View

```markdown
## Article {{article_number}} — {{title}}

{{article_text}}

{{#if amendment_rule}}
**Amendment rule**  
{{amendment_rule}}
{{/if}}

{{#if protected_status}}
**Protected status**  
{{protected_status}}
{{/if}}
```

## 11. Document Views

## 11.1 Governance Foundation Document View

Sections:

| Section | Source | Render View |
|---|---|---|
| Articles | type-query: `governance/article` | Article View |
| Roles | type-query: `governance/role` | Role View |
| Decision Log | type-query: `governance/decision` | Decision Log Entry View |
| Exercise Book | type-query: `governance/exercise` | Exercise Minutes View |

Ordering:

```text
Articles: article_number ascending
Roles: role_id ascending
Decisions: decision_id ascending
Exercises: exercise_id ascending
```

## 11.2 Meeting Summary View

Sections:

| Section | Source | Render View |
|---|---|---|
| Agenda | type-query: `governance/agenda_item` | Agenda Item View |
| Decisions Made | type-query: `governance/decision`, lifecycleState: `ratified` or `closed` | Decision Log Entry View |
| Exercises / Open Matters | type-query: `governance/exercise`, status: `open` or `deferred` | Exercise Minutes View |
| Ratifications | type-query: `governance/ratification` | Ratification View |
| Agent Notes Accepted | type-query: `governance/agent_note`, status: `accepted` | Agent Note View |

## 12. Conversational workflow

## 12.1 Live decision session

1. Session opens against a governance Container.
2. Facilitator selects or creates an Agenda Item.
3. AttentionState points to the Agenda Item.
4. Transcript chunks are tagged with the active AttentionState.
5. The discussion produces an Exercise, Decision draft, or Agent Note.
6. Facilitator focuses a specific Field.
7. Participant View zooms to the focused Field and shows human guidance.
8. AI drafts the Field value using:
   - Type guidance
   - Field guidance
   - current Record value
   - Revision history
   - transcript chunks tagged to the Field
   - related Records
9. Facilitator edits or accepts the draft.
10. Accepted value creates a Revision with SourceReferences.
11. If ratified, Decision moves to `ratified` or `closed`.
12. Closed Decisions are immutable. Future changes create a new Record and a `supersedes` or `amends` Relation.

## 12.2 Continuing from an earlier meeting

1. Facilitator opens the prior Exercise.
2. Exercise Review Protocol summarises previous thinking.
3. Group decides whether to continue exercising, split the issue, or begin Decision Deliberation Protocol.
4. New transcript chunks are tagged to the active Exercise or Decision context.
5. If a Decision is made, it links back to the Exercise with `derived-from`.

## 12.3 Batch meeting summary

1. Transcript is uploaded.
2. Meeting Summary Schema defines target records to extract.
3. AI extracts Agenda Items, Decisions, Exercises, Ratifications, and Agent Notes.
4. Human reviewer approves, edits, or rejects extracted Records.
5. Meeting Summary Document View renders a consistent summary.

## 13. Immutability and supersession policy

This profile recommends:

```text
Draft Records may be edited.
Proposed Records may be edited under facilitator control.
Ratified Records may be corrected only for non-semantic errors.
Closed Records are immutable.
Material change creates a new Record.
The new Record links to the old Record using supersedes, amends, or refines.
```

A closed Decision should never be silently overwritten.

## 14. AI agent participation

AI agents may propose content but should not silently author the governance record.

Recommended agent pattern:

```text
AI output → Agent Note / Proposed Revision → human accept/edit/reject → Record update
```

Agent types:

| Agent | Role |
|---|---|
| Clarity Coach | Suggests splitting, tightening, or clarifying decisions. |
| Policy Agent | Checks against policy, law, organisational precedent, or external documents. |
| Evidence Agent | Finds transcript chunks and source material supporting a field value. |
| Consistency Agent | Detects contradiction with existing Records. |
| Review Agent | Identifies Records due for revisit. |

Accepted Agent Notes become context for later regeneration. Rejected Agent Notes remain available for audit if the implementation preserves them.

## 15. Historical rationale

This profile encodes a recurring pattern found in durable governance traditions:

- decisions preserve reasoning, not just outcomes
- alternatives and dissent are recorded in calibrated form
- deliberation records are separated from decision records
- approved decisions become immutable
- revision happens by explicit supersession
- revisit conditions are named
- decisions are validated through appropriate institutional process

The profile does not imitate any single tradition. It provides a modern semantic structure for the same underlying problem: writing decisions so that future participants can understand what was decided, why, what was not chosen, and when the matter should be reopened.

