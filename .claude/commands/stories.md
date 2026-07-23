---
description: Maintain the user-story layer that drives priority — sync, surface missing MoSCoW, audit coverage.
argument-hint: "[story #N to inspect its tree]"
allowed-tools: Bash, Read
---

# /stories — maintain the story layer

User stories (label `user-story`, in `muDemocracy.org`) are the human value layer that drives all
implementation priority. See "Project & priority management" in this repo's `CLAUDE.md`.

## Fetch the tool

```bash
gh release download --repo the-greenman/srs-rust --pattern gh-project.mjs \
  --output /tmp/gh-project.mjs --clobber
```

## Sync stories onto the board

```bash
node /tmp/gh-project.mjs stories sync
```

## Coverage audit (the point of this command)

```bash
node /tmp/gh-project.mjs coverage
```

Report four things and what to do about each:
1. **Uncovered stories** — open stories with no implementation children. Either they're not
   started, or implementation issues exist but aren't linked as sub-issues. Propose linking or
   filing the first task.
2. **Unlinked — could get lost** — non-bug implementation issues with no parent story. For each,
   propose the story it should be a sub-issue of (or flag it as unjustified work).
3. **Bugs — fix ASAP** — bugs carry a P1 floor regardless of story; confirm they're `Ready`.
4. **Orphan stories (no epic)** — `orphan_stories_no_epic`: stories under no epic, so their Release
   can't be derived (epics are releases). Propose the epic each belongs to and link it:
   `node /tmp/gh-project.mjs epic add-story <epic#> <story#>`.
5. **Unsized issues** — `unsized_issues`: leaf work items with no `size:` label. Size drives the
   implementation-order bands; assign one from scope:
   `node /tmp/gh-project.mjs size <repo> <issue#> <small|medium|large|xl>`.

Also flag any **story missing a MoSCoW value** — priority can't be derived until a human sets it.
You may propose a MoSCoW (Must/Should/Could/Won't) per story for the human to confirm in the UI.

## Epics (= releases)

```bash
node /tmp/gh-project.mjs epics
```

Lists epics in feed order (Priority → started → roadmap `NN` prefix) with coverage flags
(`missing-priority`, `no-descendants`, `missing-roadmap-number`). An epic **is** a release: its
`Epic NN:` title is identity + roadmap sequence (retitle the issue to renumber); a human sets its
Priority (`epic set <#> --priority P`). There is no Release field — membership is the sub-issue graph.

## Inspect one story's tree

If given a story number ($ARGUMENTS):

```bash
node /tmp/gh-project.mjs tree $ARGUMENTS
```

Show the story → epics → leaf implementation issues, noting open/closed state.
