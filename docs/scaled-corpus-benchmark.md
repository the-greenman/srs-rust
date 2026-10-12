# Scaled-corpus benchmark

Measures how the engine scales with corpus size (srs-rust#1194, yardstick for the #1193 perf
sub-issues). Source: `crates/srs-repository/examples/scaled_bench.rs`.

## Run

```bash
cargo run --release -p srs-repository --example scaled_bench -- --scales 1,10 --iters 5
```

Always `--release`. Options: `--source <repo>` (any valid directory repository; default is the
vendored `tests/fixtures/exploded-basic`, per CLAUDE.md "no live sibling checkout"),
`--scales`, `--iters`. To produce a corpus without timing it (e.g. to point the CLI at):

```bash
cargo run --release -p srs-repository --example scaled_bench -- --emit /tmp/x10 --scale 10
srs repo validate --repo /tmp/x10          # 0 errors
```

## How the ×N corpus is built

Copy 0 is the source. Each further copy clones every file under `records/`, `relations/` and
`containers/` with deterministic new ids (SHA-256 of copy index + old id, UUID version/variant
bits set), rewrites every reference to a remapped id inside the cloned files, and renames files
to the canonical stems (`{slug}-{id8}` / `{relationId}`). The manifest's identity instance and
anything referencing it are not cloned; type/field/package definitions are shared. The run
asserts `validate_repository` reports 0 errors at every scale.

## Reading the table

Cells are the median wall time in ms over `--iters` runs, each on a fresh `FileStore` unless noted.

| Row | Measures |
|---|---|
| `catalog::build_checked` | cold catalog build with [R24] fatality |
| `find_instance + load_record_by_id` | one point read, including the catalog it triggers |
| `write then read, same store` | `save_record` + `find_instance` after a warm catalog (memo invalidation) |
| `list_records_filtered` | unfiltered record listing |
| `validate_repository` | the in-process equivalent of `repo validate` |

Compare ×1 against ×N: a row growing roughly linearly in N is expected; super-linear growth is
the signal. The small default fixture gives a cheap smoke run; for meaningful numbers pass a
larger `--source` (e.g. a copy of the muSrs corpus). Process startup cost (e.g.
`SchemaRegistry::global()`, srs-rust#1195) is deliberately outside these in-process timings.
