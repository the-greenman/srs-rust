#!/usr/bin/env bash
# Check whether crates/srs-schema/schemas/2.0/ has drifted from the canonical spec schemas.
# Exits non-zero if any schema file or SHA256SUMS differs.
# Usage: scripts/check-schema-drift.sh [SRS_SPEC_DIR]
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

DRIFT=0

for src_file in "${SRC}"/*.json; do
    filename="$(basename "${src_file}")"
    dst_file="${DST}/${filename}"
    if [[ ! -f "${dst_file}" ]]; then
        echo "MISSING in artifact: ${filename}"
        DRIFT=1
    elif ! diff -q "${src_file}" "${dst_file}" > /dev/null; then
        echo "DRIFT detected: ${filename}"
        DRIFT=1
    fi
done

for dst_file in "${DST}"/*.json; do
    filename="$(basename "${dst_file}")"
    if [[ ! -f "${SRC}/${filename}" ]]; then
        echo "EXTRA in artifact (not in spec): ${filename}"
        DRIFT=1
    fi
done

EXPECTED_SUMS="${DST}/SHA256SUMS"
if [[ ! -f "${EXPECTED_SUMS}" ]]; then
    echo "MISSING: ${EXPECTED_SUMS}"
    DRIFT=1
else
    TMPFILE="$(mktemp)"
    bash -c "cd '${DST}' && sha256sum *.json | sort > '${TMPFILE}'"
    if ! diff -q "${EXPECTED_SUMS}" "${TMPFILE}" > /dev/null; then
        echo "SHA256SUMS mismatch in artifact directory"
        DRIFT=1
    fi
    rm -f "${TMPFILE}"
fi

if [[ "${DRIFT}" -ne 0 ]]; then
    echo ""
    echo "Schema drift detected. Run scripts/sync-schemas-from-spec.sh to update." >&2
    exit 1
fi

echo "OK: No schema drift detected."
