# site/: engine.semanticops.com

Static files for the srs engine, served as-is by an assets-only Cloudflare Worker (`wrangler.jsonc`), deployed by Workers Builds on push to master with root directory `site`.

- `public/install.sh`, `public/start.md`, `public/llms.txt`: hand-written.
- `public/cli/*.md`: generated from `srs --help`. After changing a command, run `node scripts/site/build-site.mjs` (it uses `target/release/srs`, else `target/debug/srs`, or the path you pass). CI runs it with `--check` and fails on drift.
