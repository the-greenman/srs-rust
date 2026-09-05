# Agents — read `CLAUDE.md` first; it is canonical for this repo. This file carries only what a non-Claude agent needs beyond it.

- **Current work**: the-greenman/srs#580 is the queue. Read it before starting anything — it holds the ordered remaining units and the merge-rights contract, not this file.
- **Signing**: run `ssh-add -l | grep -q "SHA256:vHuO6si5w3RLL4IJZofWbyvEi42WA2fYX7bM"` before ANY commit. STOP and report if missing — never `--no-gpg-sign`, never `--no-verify`. Use plain `git commit`.
- **Do not invoke or follow `.claude/commands/*`** — those are Claude-specific. `epic-worker`/`epic-coordinator` are retired (srs#451). The process is `CLAUDE.md` + srs#580.
- **PRs**: state **mode, cell and door** in the body, plus `Closes #N` / `Refs #N`. Label `gate:auto-merge` only when mode is clear/complicated AND the change is Door 1 (cite the `rfc-decision-*` id) or non-normative; otherwise `gate:owner-merge` and never merge.
- **Gates**: `cargo build --workspace`, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, each by exit code. `SRS_SPEC_DIR` must point at a fresh clone of `srs` `origin/master` — see `CLAUDE.md`'s "Gates and choreography" section.
