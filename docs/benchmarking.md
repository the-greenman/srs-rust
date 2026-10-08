# Scaled-corpus benchmark

How the engine scales with repository size (srs-rust#1194). It is the yardstick for the perf work under #1193.

```bash
scripts/bench-scale.sh                 # pinned muSrs archive, x1 and x10
scripts/bench-scale.sh <archive.srs> 5 # another archive, x1 and x5
```

The script unpacks the archive, builds an xN copy, and prints a table. Source:
`crates/srs-repository/examples/scale_bench.rs` (`gen` and `bench` subcommands; use them directly on an
exploded repository). The default corpus is the vendored `tests/fixtures/discovery-eval/musrs-pinned.srs`.

## What the generator does

For each copy 1..N-1 it clones every `.json` file under `records/`, `relations/` and `containers/`, remapping every
`instanceId` / `relationId` / `containerId` to a deterministic UUID (SHA-256 of copy index + old id, with v5
version/variant bits) and rewriting every reference to them inside the cloned files. A relation's filename stem is its
new id; any other file keeps its slug and takes the first eight characters of its new id. Manifest, packages and
source documents are copied once. The xN output loads without catalog or validation errors (the bench fails otherwise).

## Reading the table

Each cell is the best of 3 runs in milliseconds; every run opens a **cold** `FileStore`, so the point-lookup rows
include the catalog build. Compare columns for the scaling factor: a row near 10x between x1 and x10 is linear in
corpus size.

| Row | Exercises |
|---|---|
| `catalog::build_checked` | full tree enumeration and [R24] checks |
| `find_instance`, `get_note_by_id` | point lookup on a cold store |
| `write + read` | `save_note` then `find_instance` on the same store (memo invalidation) |
| `list_records_filtered (all)` | unfiltered record listing |
| `validate_repository` | schema validation of every instance |

Reference numbers (release build, cloud container, muSrs, 4093 / 40921 entities): catalog 55 / 569 ms, write+read 118 /
1169 ms, validate 282 / 3345 ms. Timings are machine-dependent; compare runs on one machine. The benchmark does not
gate CI.
