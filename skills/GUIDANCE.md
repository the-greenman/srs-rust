# Repository guidance: the baseline

A repository's guidance (`manifest.aiGuidance`, set with `srs repo guidance set`) is appended to the MCP server's instructions. It is the one text about **this** repository that every agent receives on connect, whether or not its client loads the srs skill. It is channel 3 in `MAINTAINING.md`, and it is maintained under the same rules: every hint has a reason, no bare numbers, one home per lesson.

## What it holds

| Part | Says | Size |
|---|---|---|
| `summary` | What the repository holds, who writes it, and what to read first. | 1 to 3 sentences, under 400 characters |
| `navigationHints` | One string. In order: the areas in reading order; how this repository uses types, relation keys and lifecycle (only what differs from the obvious, with exact keys); where new things go; the repository's own traps. | under 1,000 characters |
| `suggestedEntryPoints` | The identity record, then 1 to 3 records an agent should open first (section anchors, a maintainer's note). Give ids; the command stores paths. | 2 to 4 |

Keep the whole under 1,500 characters. The server caps the note at about 2 KB and cuts the rest.

## What it leaves out

- **Generic SRS lessons.** How to search, copy relation keys, validate, update whole objects: the server instructions and the skill teach these to every repository. Repeating them here costs every agent on every connect.
- **Counts and dates that change.** "2,500 records" is out of date next week. Name the structure, not its size.
- **Rules owned elsewhere.** A repository's governance lives in its records or its project instructions. Guidance points to the record; it does not restate it.
- **Anything private** in a repository served to others.

## Template

```json
{
  "summary": "<What this holds>, written by <who>. Read <the identity record / the guide> first.",
  "navigationHints": "Areas, in order: <A>, <B>, <C>. <Kind> records are type <ns/name>; <state meaning> is their lifecycle, never a field. <X> points at <Y> with <relation key>, so neighbours on a <Y> gives <what>. New <things> go in <container>. Here, <the trap this repository has>.",
  "suggestedEntryPoints": ["<identity record id>", "<section anchor id>"]
}
```

## Keeping it right

- Change the guidance in the same PR that changes what it describes: an area added or renamed, a type or relation key that matters, where new things go.
- A cold agent given only the repository (task T1 of the cold-agent set) is the test: it should name the areas and open the right first record without searching blindly.
- `repo validate` will warn when guidance is unset (srs-rust#1686). Until then, `srs repo guidance get` is the check.
