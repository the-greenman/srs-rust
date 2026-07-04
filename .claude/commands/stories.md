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

Also flag any **story missing a MoSCoW value** — priority can't be derived until a human sets it.
You may propose a MoSCoW (Must/Should/Could/Won't) per story for the human to confirm in the UI.

## Epics (= releases)

```bash
node /tmp/gh-project.mjs epics
```

Lists epics by Priority with their Release identity and coverage flags (`missing-release`,
`missing-priority`, `no-descendants`). An epic **is** a release: a human sets its Release + Priority
(`epic set <#> --priority P --release R`); descendants inherit Release via `release-sync`.

## Inspect one story's tree

If given a story number ($ARGUMENTS):

```bash
node /tmp/gh-project.mjs tree $ARGUMENTS
```

Show the story → epics → leaf implementation issues, noting open/closed state.
