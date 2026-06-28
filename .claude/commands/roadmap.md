---
description: Sequence a program's issues into iterations by gate/phase, bounded by story release windows.
argument-hint: "<program: an epic #N or milestone>"
allowed-tools: Bash, Read
---

# /roadmap — sequence a program into iterations

Program: **$ARGUMENTS** (an epic issue or a milestone).

Iterations are delivery windows on Project #5. **GitHub has no API to create iterations** — if a
phase needs a window that doesn't exist yet, stop and ask the human to add it in the project UI.
See "Project & priority management" in this repo's `CLAUDE.md`.

## Fetch the tool

```bash
gh release download --repo the-greenman/srs-rust --pattern gh-project.mjs \
  --output /tmp/gh-project.mjs --clobber
```

## Stage 1 — Map the program

- Read the epic/milestone and its child issues (`gh issue view`, `node /tmp/gh-project.mjs tree <story#>`).
- Identify the program's **gate/phase structure** (e.g. Phase 0 RFC → Gate A → Gate B …) from the
  epic body and milestones.
- Inspect existing iterations: `node /tmp/gh-project.mjs fields` (Iteration options).

## Stage 2 — Sequence

Assign each child issue to an iteration by phase, respecting dependencies and **bounded by the
served story's release window** (don't schedule a `decision-logger-v1` child past that release):

```bash
node /tmp/gh-project.mjs set <repo> <issue#> --iteration "Iteration N"
```

Keep priorities story-derived — run `/triage` (or `rollup --fix`) rather than hand-setting priority.

## Stage 3 — Report

Emit a per-iteration table for the program. **Flag any phase with no available iteration** and ask
the human to create it in the UI before that phase can be scheduled.
