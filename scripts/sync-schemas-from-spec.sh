#!/usr/bin/env bash
# Sync canonical schemas from the spec repo into the srs-schema artifact crate.
# Usage: scripts/sync-schemas-from-spec.sh [SRS_SPEC_DIR]
# SRS_SPEC_DIR defaults to ../srs (sibling checkout of this workspace).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
SPEC_DIR="${SRS_SPEC_DIR:-${WORKSPACE_DIR}/../srs}"
SRC="${SPEC_DIR}/docs/schema/2.0"
DST="${WORKSPACE_DIR}/crates/srs-schema/schemas/2.0"

if [[ ! -d "${SRC}" ]]; then
    echo "ERROR: Canonical schema directory not found: ${SRC}" >&2
    echo "       Set SRS_SPEC_DIR to the path of the srs spec repo." >&2
    exit 1
fi

mkdir -p "${DST}"
cp "${SRC}"/*.json "${DST}/"

cd "${DST}"
sha256sum *.json | sort > SHA256SUMS

echo "Synced $(ls "${DST}"/*.json | wc -l) schemas + SHA256SUMS from ${SRC}"
