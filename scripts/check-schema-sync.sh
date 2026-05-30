#!/usr/bin/env bash
# check-schema-sync.sh
#
# Verifies that every JSON schema file in srs/docs/schema/2.0/ has an
# identical copy in crates/srs-schema/schemas/2.0/.
#
# Exits 0 if all schemas are in sync.
# Exits 1 if any schema is missing from the embedded copy or has diverged.
#
# Usage (from srs-rust/ workspace root):
#   bash scripts/check-schema-sync.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(dirname "$SCRIPT_DIR")"
SPEC_SCHEMA_DIR="$(dirname "$WORKSPACE_ROOT")/srs/docs/schema/2.0"
EMBEDDED_SCHEMA_DIR="$WORKSPACE_ROOT/crates/srs-schema/schemas/2.0"

if [[ ! -d "$SPEC_SCHEMA_DIR" ]]; then
  echo "ERROR: spec schema directory not found: $SPEC_SCHEMA_DIR" >&2
  exit 1
fi

if [[ ! -d "$EMBEDDED_SCHEMA_DIR" ]]; then
  echo "ERROR: embedded schema directory not found: $EMBEDDED_SCHEMA_DIR" >&2
  exit 1
fi

errors=0

for spec_file in "$SPEC_SCHEMA_DIR"/*.json; do
  filename="$(basename "$spec_file")"
  embedded_file="$EMBEDDED_SCHEMA_DIR/$filename"

  if [[ ! -f "$embedded_file" ]]; then
    echo "MISSING: $filename exists in spec schemas but not in crates/srs-schema/schemas/2.0/" >&2
    errors=$((errors + 1))
    continue
  fi

  spec_sha="$(sha256sum "$spec_file" | cut -d' ' -f1)"
  embedded_sha="$(sha256sum "$embedded_file" | cut -d' ' -f1)"

  if [[ "$spec_sha" != "$embedded_sha" ]]; then
    echo "DIVERGED: $filename — spec and embedded copies have different content" >&2
    echo "  spec:     $spec_sha  ($spec_file)" >&2
    echo "  embedded: $embedded_sha  ($embedded_file)" >&2
    errors=$((errors + 1))
  fi
done

if [[ $errors -eq 0 ]]; then
  echo "OK: all $(ls "$SPEC_SCHEMA_DIR"/*.json | wc -l | tr -d ' ') spec schemas are in sync with embedded copies"
  exit 0
else
  echo "FAIL: $errors schema sync error(s) found" >&2
  exit 1
fi
