# Structure a repository for people

A repository can be valid and still unreadable: hundreds of records with nothing to say where to start. Structure it as a **tree of documents that gets deeper**. A person or a cold agent finds a top-level introduction, reads an overview, and drills down into parts, then into the records that back them. Each level answers "what is this, and where do I go next?" before it shows any detail.

This page covers the shape of that tree, the commands that build it, and how to check it. It assumes you can already create records and containers ([first five minutes](https://engine.semanticops.com/start.md)).

## The shape

```
root container          the front door: the purpose record, then a few areas in reading order
└─ area                 a document (overview) that anchors its own section container
   └─ part              a document that anchors its own section container
      └─ record         the evidence, the decisions, the detail
```

- **Every level with children is a document, not a label.** Its anchor record says, in prose: what this part is about (in short), the story so far, the parts below it, and what is still open. A heading with a list under it is not a document.
- **Every document with children anchors its own container.** That is what lets navigation descend: a container member that is the `anchorInstanceId` of another container becomes an expandable section.
- **Depth stays shallow.** Three or four levels is usually enough. If a level has more than about a dozen children, it needs another level of parts.
- **Leaves are the records you already have.** Put a record in a section by membership. Never copy it to make it appear twice: one record can be a member of several sections.

## The front door

- **The identity record is the reading guide.** The root container's `identityInstanceId` names the repository's purpose record. Write it for a first-time reader:
  - what the repository holds;
  - the areas, in the order to read them;
  - what the lifecycle states mean, if records carry them;
  - where to go next.
- **One document is the front door.** Clients open a composition first. The SemanticOps editor opens the first composition in the package, so install that one first. Make it render the reading guide, then the top of each area.
- **Order the root deliberately.** The root container's entry order is the navigation order. Put the areas in the order a newcomer should read them. When the order is itself information, such as a ranking, say so in the reading guide.

## Build it

Create the overview document, then its section, anchored on it, with the parts below:

```sh
srs note create <<'EOF'
{"title": "The graph: why we need one",
 "sections": [{"name": "summary", "label": "In short", "content": "…"},
              {"name": "story",   "label": "The story so far", "content": "…"},
              {"name": "parts",   "label": "The parts", "content": "1. … 2. …"},
              {"name": "open",    "label": "Open questions", "content": "…"}]}
EOF

srs container create <<'EOF'
{"title": "The graph", "anchorInstanceId": "<overview id>", "identityInstanceId": "<overview id>",
 "memberInstanceIds": [{"instanceId": "<overview id>"},
                       {"instanceId": "<part 1 id>", "depth": 1},
                       {"instanceId": "<part 2 id>", "depth": 1}]}
EOF

srs container members add <area container> <record id> --depth 1   # a record joins a section; nothing is copied
srs container members add <root container> <overview id>            # the area appears in navigation
```

- **Anchor every root member.** `srs repo validate` warns when a root container member anchors no container (RFC-013 I-82). Treat that warning as a navigation dead end and fix it.
- **Nest within a section with `depth`.** An entry's parent is the nearest entry above it with a smaller depth, and depth rises by at most one per entry.
- **Keep meaning in relations.** A finding `evidences` a problem, and a part is `derived-from` the conversation it came from. Membership only arranges records for reading.

## Make records read as text

- **Labels:** give every field assignment a `displayLabel` ("Felt by", not `who_feels_it`).
- **Reading views:** one view per type for reading (`srs view create`).
  - The title becomes the heading through the section's `titleFieldId`.
  - Long prose fields use `labelMode: "none"`.
  - Ids, paths and locators are left out of the view. They stay in the record.
- **Documents:** one composition per reading purpose, such as the whole map, one area's dossier, or a review queue.
  - Sections over a container use `"ordering": {"source": "arranged"}` with `"containerScope": "explicit"`. Arranged order cannot be combined with `subtree`.
  - Route each type to its reading view with `typeDispatch`.

## Keep it readable as it grows

- **Rewrite overviews; don't append to them.** An overview that grows as a log stops being an overview. Move new detail into a part, and update the overview's story.
- **Settled structure stays put.** Add to the tree consistently. Bring a reorganisation of the areas forward as a deliberate change, and record what it replaced.
- **Keep counter-evidence beside the claim it weakens.** A section that only collects support reads as advocacy.

## Check it

- **Walk it:** `srs repo navigation --pretty`. From the root you should reach every area and part without searching.
- **Read it:** `srs render composition --view <front door id>`. Read it top to bottom as someone who did not write it.
- **Cold test:** give an agent with no context only the repository, and ask it to explain the repository and find one specific record.

Known gaps in today's tools:
- Navigation output does not yet keep entry order or depth inside a section (srs-rust#1490). The stored outline is still the source of truth.
- The editor's container pane lists search results rather than the outline (srs-web#668).
