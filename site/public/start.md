# srs engine: the first five minutes

`srs` is the reference engine for SRS (Semantic Record System): one binary that is both the CLI and an MCP server over an SRS repository. Every step below was run against a fresh build.

## 1. Install

```bash
curl -fsSL https://engine.semanticops.com/install.sh | sh
```

It picks the prebuilt binary for your OS and CPU from the [latest release](https://github.com/the-greenman/srs-rust/releases/latest), checks its `.sha256`, and installs it to `~/.local/bin` (set `SRS_INSTALL_DIR` to change that). With no prebuilt binary for your platform, build from source (needs Rust):

```bash
cargo install --git https://github.com/the-greenman/srs-rust srs
```

## 2. Get a repository

Open the sample, a meeting note, the decision taken from it and two tasks:

```bash
curl -fsSLO https://semanticops.com/try/meeting.srs
srs archive unpack meeting.srs --target meeting
```

Or start an empty one:

```bash
srs repo create --repo notes --namespace com.example.notes --title "My notes"
```

## 3. Validate and look around

```bash
srs repo validate --repo meeting
srs repo map --repo meeting --pretty
```

The sample validates as it is. A repository written for an older data model answers `"ok": false` with `errors[0].code` = `container-migration-needed`. Apply the migrations it lists in `errors[0].details.pending`, in order, then validate again:

```bash
srs repo apply-migration --repo meeting --id <id>
```

`srs repo agent-index --repo meeting` gives one page of orientation (identity, counts, types, sections).

## 4. Connect an agent over MCP

Claude Code:

```bash
claude mcp add srs -- srs mcp serve --repo /path/to/meeting
```

Any other MCP client, in `.mcp.json` or its equivalent:

```json
{"mcpServers":{"srs":{"command":"srs","args":["mcp","serve","--repo","/path/to/meeting"]}}}
```

`srs mcp serve --profile context` advertises a smaller tool set; `--profile read` advertises no tool that writes. Start an agent at the `srs://<repositoryId>/agent-index` resource.

## 5. The output contract

Every command prints one JSON envelope: `{"ok", "command", "version", "payload"}`.

- **Check `ok` first, not the exit code.** When `ok` is `false`, the command did not do its job, and the reasons are in `errors[]`, one `{code, message, details?}` per problem. The exit code alone does not tell you: `record get` of a missing id exits 0 with `"ok": false`.
- **Branch on `errors[].code`**, a stable kebab-case identifier. Never on the message text.
- **Exit 0 does not mean valid.** A command that ran reports problems with the data in `payload.diagnostics`; `repo validate` adds `payload.summary`. The repository is clean when `payload.summary.errors` is 0.
- MCP tool errors carry the same `code` and `details` as `structuredContent`.

Never edit a repository's JSON files by hand: the CLI and the MCP tools enforce its type and relation contracts.

## Where next

- [CLI reference](https://engine.semanticops.com/cli/index.md): every command's `--help`.
- [The SRS specification](https://srs.semanticops.com/llms.txt): the data model, one page per concept.
- [The srs-memory skill](https://skill.semanticops.com/srs-memory/SKILL.md): keep a codebase's decisions and traps in an SRS repository.
- [The SemanticOps editor](https://app.semanticops.com): open and edit a repository in the browser.
