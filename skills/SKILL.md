---
name: srs
description: Work with an SRS (Semantic Record System) repository through the srs CLI or MCP server: orient in an unfamiliar repository, find and read records, write them safely, validate, and keep project memory in one. Use whenever a directory has a `.srs/` marker and `manifest.json`, when asked to read, create or change typed records, relations or containers, or when asked to record decisions and findings so the next agent starts informed.
---

# SRS: Semantic Record System

An SRS repository is a directory with a `.srs/` marker and a `manifest.json`: typed records, the relations between them, and containers that group and order them. The `srs` CLI, or the same operations over MCP (`srs mcp serve --repo <path>`), is the only supported way to read and write it. Never edit its JSON files by hand.

Everything you need to start is at **https://engine.semanticops.com/start.md**: install (`curl -fsSL https://engine.semanticops.com/install.sh | sh`), open or create a repository, validate, connect MCP, and the output contract (check `ok`, branch on `errors[].code`, and judge validity by `payload.summary.errors == 0`, not by the exit code).

- CLI reference, every command's `--help`: https://engine.semanticops.com/cli/index.md
- The engine's index for agents: https://engine.semanticops.com/llms.txt
- Keeping a codebase's decisions, conventions and traps in an SRS repository: https://skill.semanticops.com/srs-memory/SKILL.md
- The specification: https://srs.semanticops.com/llms.txt

If the project has its own instruction file (CLAUDE.md, AGENTS.md) or `srs-usage.md`, it wins over this skill.
