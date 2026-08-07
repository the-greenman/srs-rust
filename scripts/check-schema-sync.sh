#!/usr/bin/env bash
# check-schema-sync.sh
#
# Verifies that every JSON schema file in srs/docs/schema/2.0/ has an
# identical copy in crates/srs-schema/schemas/2.0/.
#
# Exits 0 if all schemas in every mirror actually checked are in sync.
# Exits 1 if any schema is missing from a checked mirror or has diverged.
#
# The srs-vscode mirror is only checked when its tree is present alongside
# this workspace (a monorepo checkout) — every cloud worker container is a
# single-repo checkout, so that mirror is normally skipped. Pass
# --require-vscode to turn a skipped srs-vscode mirror into an error, for
# environments (e.g. a release gate) that must have it present.
#
# Usage (from srs-rust/ workspace root):
#   bash scripts/check-schema-sync.sh [--require-vscode]

set -euo pipefail

REQUIRE_VSCODE=0
for arg in "$@"; do
  case "$arg" in
    --require-vscode) REQUIRE_VSCODE=1 ;;
    *)
      echo "ERROR: unknown argument: $arg" >&2
      exit 1
      ;;
  esac
done

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

checked_mirrors=("crates/srs-schema/schemas/2.0")

VSCODE_SCHEMA_DIR="$(dirname "$WORKSPACE_ROOT")/srs-vscode/schemas/2.0"

if [[ ! -d "$VSCODE_SCHEMA_DIR" ]]; then
  if [[ "$REQUIRE_VSCODE" -eq 1 ]]; then
    echo "ERROR: --require-vscode set but srs-vscode schema directory not found: $VSCODE_SCHEMA_DIR" >&2
    errors=$((errors + 1))
  else
    echo "WARN: srs-vscode schema directory not found (non-monorepo environment?): $VSCODE_SCHEMA_DIR — skipping that mirror" >&2
  fi
else
  checked_mirrors+=("srs-vscode/schemas/2.0")
  for spec_file in "$SPEC_SCHEMA_DIR"/*.json; do
    filename="$(basename "$spec_file")"
    vscode_file="$VSCODE_SCHEMA_DIR/$filename"

    if [[ ! -f "$vscode_file" ]]; then
      echo "MISSING: $filename exists in spec schemas but not in srs-vscode/schemas/2.0/" >&2
      errors=$((errors + 1))
      continue
    fi

    spec_sha="$(sha256sum "$spec_file" | cut -d' ' -f1)"
    vscode_sha="$(sha256sum "$vscode_file" | cut -d' ' -f1)"

    if [[ "$spec_sha" != "$vscode_sha" ]]; then
      echo "DIVERGED: $filename — spec and srs-vscode copies have different content" >&2
      echo "  spec:    $spec_sha  ($spec_file)" >&2
      echo "  vscode:  $vscode_sha  ($vscode_file)" >&2
      errors=$((errors + 1))
    fi
  done
fi

mirror_list="$(IFS=', '; echo "${checked_mirrors[*]}")"

if [[ $errors -eq 0 ]]; then
  echo "OK: all $(ls "$SPEC_SCHEMA_DIR"/*.json | wc -l | tr -d ' ') spec schemas are in sync with the following mirror(s): $mirror_list"
  exit 0
else
  echo "FAIL: $errors schema sync error(s) found (mirrors checked: $mirror_list)" >&2
  exit 1
fi
