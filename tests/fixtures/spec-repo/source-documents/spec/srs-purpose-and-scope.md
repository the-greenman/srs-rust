# SRS — Purpose & Scope

**Version**: 0.1-draft
**Status**: draft
**Kind**: charter / foundational document
**Intended home**: a top-level section family of the SRS specification (the "srs srs")

> **Projection note** — This is a single authored document, staged in `source-documents/`
> so it can be migrated into SRS records later. Once migrated, the records become the source
> of truth and this file becomes a projection of them.

---

## About this document (its semantic structure)

> **Purpose** — Declare the shape of this document up front, so its migration into SRS
> records is mechanical rather than interpretive. This section is scaffolding; it is not
> part of the charter itself.

This document is written to be moved into SRS form with no re-reading required. It is a
sequence of **sections**, and every section is authored as a future record of the existing
`com.semanticops.spec/section` type, extended with one added facet — **Purpose**:

| Facet | Meaning | Migrates to |
|---|---|---|
| **Title** | The name of the section | `section-title` field |
| **Purpose** | The job this section does — the question it answers, the reason it exists | a new `section-purpose` field (see *How this becomes SRS*) |
| **Content** | The prose of the section | `content` field |

The **Purpose** facet is deliberate and load-bearing. A section whose purpose cannot be
stated in one sentence is a section that does not belong. This is the document's own defence
against the scope creep it warns about: purpose first, prose second.

Ordering is by position here and will be fixed by `section-sequence` relations on migration.

---

## 1. Purpose

> **Purpose** — State, in one place, the single reason SRS exists, so that every downstream
> decision — every feature, every extension, every line of the spec — can be checked against
> it. This is the yardstick.

**SRS builds portable semantic documents that both humans and AI can understand and use.**

Its reason for being is to **transfer a complex unit of knowledge** — intact — from one mind
or system to another. Not a file, not a message, not a row in a database: a *unit of
knowledge*, with its meaning attached, so that the receiver understands it the way the author
did.

Transferring knowledge well has two demands that pull against each other, and SRS exists to
hold both at once:

- **Depth of context** — the receiver needs enough to genuinely understand, not a lossy
  summary.
- **Without overload** — no one, human or AI, can absorb everything at once.

SRS resolves the tension by making knowledge **layered and drillable**: a meaningful surface
that can be entered progressively, deeper on demand. Everything else in the system is in
service of this.

---

## 2. What SRS is

> **Purpose** — Give one shared definition, so "SRS" means the same thing to every reader and
> contributor before any detail is discussed.

SRS (Semantic Record System) is a **portable semantic system**: an open way to break knowledge
into small, named, typed, addressable units and to record how those units relate.

A wall of prose can only be interpreted by a human, and only whole. SRS instead captures the
same knowledge as **records** — each carrying explicit meaning (a stable identity, a value
type, guidance for AI) — wired together by first-class **relations**. The result is knowledge
a tool can query, validate, re-render, reason over, and hand to an AI, rather than merely
display.

One principle governs the whole system:

> **Records are the source of truth. Rendered documents are projections of those records —
> derived, never authoritative.**

"Portable" is not decoration. The units, their definitions, and their relations travel
together and can be understood by any conforming implementation, with no dependency on the
tool that produced them.

---

## 3. What SRS must do

> **Purpose** — Enumerate the small, closed set of jobs SRS is responsible for. This is the
> functional charter: capability that does not serve one of these jobs is, by definition, out
> of scope.

SRS has exactly two responsibilities. Everything the system offers should reduce to one of
them.

### 3.1 Transfer a complex unit of knowledge, in layers

Present knowledge so it can be received without overload: a meaningful surface first, with
depth available on demand. Layers can be **drilled into** — the reader chooses how far down to
go, and the structure guarantees that going deeper adds detail rather than changing the story.
This is how SRS delivers *depth of context without overload*.

### 3.2 Structure knowledge so its meaning can be shared

Meaning does not transfer on its own; it transfers when sender and receiver share the same
building blocks. So SRS lets those building blocks be **defined and shared**, and lets a
structure built from them:

- **require that specific questions are answered** — a Type declares which fields must be
  present, so a unit of knowledge cannot be transferred half-formed;
- **let the quality of those answers be evaluated** — the structure makes it possible to
  judge whether an answer is present, complete, and well-formed, not just whether text exists;
- **enable synthesis and whole-context sharing** — because units are typed and related, they
  can be composed into a larger whole and transferred together, as one coherent context, not
  as scattered fragments.

---

## 4. The deeper goal and its principles

> **Purpose** — Record the long-term intent behind SRS, because it decides trade-offs that the
> immediate use case cannot. When two designs both "work," this is what chooses between them.

The initial use case is a means; the goal is larger.

The goal is a **generic, portable semantic system for human–AI collaboration that carries no
built-in, engineered control hierarchy.** Most collaboration tools quietly encode the
structure of the organisation that built them — **Conway's law** made permanent in software.
SRS is designed *not* to. Its structure comes from the knowledge being shared, not from a
chain of command baked into the format.

Three principles follow and constrain every design decision:

- **No engineered control hierarchy.** The model must not presume who is in charge, who
  approves, or how a group is organised. Structure describes knowledge, never authority.
- **Data sovereignty.** The people who create knowledge own it. It is portable and readable
  without permission from any central service or vendor.
- **Decision sovereignty.** Groups decide for themselves how they organise, govern, and act on
  their knowledge. SRS supplies the substrate for those decisions; it does not make them, and
  it does not lock them in.

The **openness of the spec is the mechanism** for all three. An open, implementable standard is
what makes the data portable and the decisions the group's own.

---

## 5. Initial use case

> **Purpose** — Name the one concrete application SRS is being built for first, so the general
> goal has a real workload to prove itself against and design stays grounded in something
> shippable.

The first use case is **sharing collaboration context**: the semantic context a group creates
when its members work together.

When people collaborate, they produce more than documents. They make **decisions**, those
decisions create **structure**, and that structure carries meaning that is usually lost the
moment it leaves the room — trapped in chat logs, meeting notes, and individual memory. SRS
targets exactly this: capturing the decisions and the structure they create as portable
semantic records, so the shared understanding of a group can be **transferred** — to a new
member, to another team, or to an AI collaborator — intact.

This use case exercises every responsibility in §3: it demands depth without overload (a
newcomer should not have to read everything), it demands shared building blocks (so a decision
means the same thing to everyone), and it demands synthesis (so the whole context transfers
together).

---

## 6. Scope

> **Purpose** — Draw the boundary explicitly. This is the section that exists to stop scope
> creep: if something is not here under "in scope," building it needs a deliberate decision,
> not drift.

**In scope** — SRS defines, and only defines:

- the **semantic building blocks** of knowledge (fields, types) and how they are defined,
  versioned, and shared;
- **records** — instances of that structure that hold real knowledge, at graduated levels of
  maturity;
- **relations** between records, as first-class typed edges;
- **containers** that group records into boundaries;
- **views** — projections that present the layered, drillable surface;
- the **portability contract**: how all of the above travel together and are understood by any
  conforming implementation.

**Out of scope (non-goals)** — SRS deliberately does *not* provide:

- **an org chart, roles, or an approval/permission hierarchy** — this is the Conway's-law trap
  of §4; SRS describes knowledge, not authority;
- **a workflow or process engine** — it structures the knowledge decisions produce, not the
  business logic that acts on it;
- **a specific application or UI** — those are clients built *on* SRS; the model stays
  presentation-free;
- **a storage engine, database, or sync service** — SRS is a portable format, not
  infrastructure;
- **a messaging or transport protocol** — it defines what travels, not the wire it travels on;
- **governance decisions themselves** — SRS is the substrate for a group's decisions
  (decision sovereignty), never the decision-maker.

The test for any proposed capability: *does it help transfer a unit of knowledge with its
meaning intact (§3), without encoding who is in charge (§4)?* If not, it is out of scope.

---

## 7. Implementation patterns

> **Purpose** — Give the high-level vocabulary of the model, so every reader shares the same
> six constructs before meeting the normative detail. This section names the parts; the
> specification defines them.

SRS does all of its work with a small, fixed set of constructs. Each is introduced here at the
level of *what it is for*; precise rules live in the specification.

### 7.1 Field — the atomic unit of meaning

> **Purpose** — Establish the smallest shareable building block, the thing that makes meaning
> reusable and unambiguous.

A **Field** is the smallest unit of meaning: a stable identity, a value type, and optional
guidance for how an AI should read or produce its value. Field semantics are **immutable** — a
field means the same thing everywhere it is used, which is what lets meaning be *shared* rather
than re-invented per document.

### 7.2 Type — a composition of fields

> **Purpose** — Explain how building blocks compose into a shape that can *require questions be
> answered* (§3.2).

A **Type** is a named, versioned composition of fields. By declaring which fields a record must
carry, a Type is how SRS "asks the questions": it defines what a well-formed unit of this kind
of knowledge looks like, and thereby makes it possible to evaluate whether an instance actually
answers them.

### 7.3 Record — knowledge, in three tiers

> **Purpose** — Show how real knowledge is captured, and how it can start rough and mature
> without being lost — the mechanism behind "depth without premature structure."

A **Record** is concrete knowledge. SRS recognises three tiers of semantic maturity so capture
is never blocked by structure:

- **Tier 0 — Note**: free text, no type binding. For raw capture.
- **Tier 1 — Typed Record**: named fields with values, no bound Type yet.
- **Tier 2 — Record**: bound to a published Type and fully validatable.

Knowledge can graduate up the tiers as it firms up; nothing is discarded on the way.

### 7.4 Container — a grouping boundary

> **Purpose** — Provide a way to bound a set of records into a coherent whole, enabling
> *whole-context sharing* (§3.2) without inventing a hierarchy.

A **Container** groups records into a boundary — "everything belonging to this project," "this
decision and its consequences." It organises; it is not part of the semantic graph and asserts
no authority over what it holds. It is how a whole context is packaged so it can travel
together.

### 7.5 Relation — a typed edge between records

> **Purpose** — Establish that structure lives in *connections between* records, not in a
> containing hierarchy — the design that keeps the model free of engineered control (§4).

A **Relation** is a first-class, typed edge between two records — `contains`, `depends-on`,
`supersedes`, `refines`, `precedes`, and so on. Relations are **semantic claims**, not
ownership or command lines. The structure of a body of knowledge *is* its relations, which is
precisely why SRS needs no imposed hierarchy to express structure.

### 7.6 View — the layered, drillable surface

> **Purpose** — Name the construct that delivers §3.1: turning the record graph into a surface
> a reader can enter progressively and drill into.

A **View** is a projection of records into a presentation — the layered surface a reader
actually meets. Views are how "depth of context without overload" is delivered: a view can show
a meaningful summary layer and let the reader **drill down** into the records beneath it. A view
is always derived; it never holds authoritative knowledge of its own.

---

## 8. How this document becomes SRS

> **Purpose** — Record the intended migration path, so moving this charter into the srs srs is
> a known, mechanical step and not a fresh design exercise.

This document is staged as a source document (`source-documents/spec/`) with a `.meta.json`
sidecar. When it is migrated:

1. Each `## n.` heading becomes a `com.semanticops.spec/section` record; each `### n.m`
   becomes a `com.semanticops.spec/subsection`.
2. The **Title** and **Content** facets map directly onto the existing `section-title` and
   `content` fields.
3. The **Purpose** facet needs a new field — `com.semanticops.spec/section-purpose` (a `text`
   field, likely `required`) added to the `section`/`subsection` types. Capturing purpose as a
   first-class field is the point of authoring the document this way.

   > **Deferred, 2026-08-10.** This field is deliberately **not** minted during formation. The
   > purpose concept is unconverged — four overlapping Fields already exist across three
   > namespaces (`srs/summary`, `spec/summary`, `core/statement`, `srs/purpose`), none carrying
   > the yardstick semantics this facet means — and Field identity is permanent. The discipline
   > continues in prose; the decision is revisited before the first full public release. See the
   > decision record **"Defer minting `section-purpose` until the purpose concept converges"** in
   > the [RFC decision log](../../../docs/spec/rfcs/rfc-decision-log.md), and the-greenman/srs#329.
4. `section-sequence` / `subsection-sequence` relations fix the ordering shown here.
5. Every migrated record carries a `sourceRef` back to this document's `documentId`, preserving
   provenance.

Until then, the prose above is authoritative and this file is the unit of work.
