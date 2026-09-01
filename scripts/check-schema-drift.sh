#!/usr/bin/env bash
# Check whether crates/srs-schema/schemas/2.0/ has drifted from the canonical spec schemas.
# Exits non-zero if any schema file or SHA256SUMS differs.
# Usage: scripts/check-schema-drift.sh [SRS_SPEC_DIR]
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
# Positional arg wins (as the usage line and CI invocation promise), then env, then sibling.
SPEC_DIR="${1:-${SRS_SPEC_DIR:-${WORKSPACE_DIR}/../srs}}"
SRC="${SPEC_DIR}/docs/schema/2.0"
DST="${WORKSPACE_DIR}/crates/srs-schema/schemas/2.0"

if [[ ! -d "${SRC}" ]]; then
    echo "ERROR: Canonical schema directory not found: ${SRC}" >&2
    echo "       Set SRS_SPEC_DIR to the path of the srs spec repo." >&2
    exit 1
fi

DRIFT=0

# Declared transitional allowlist (srs-rust#877 caboose repair): these schemas
# were retired from the canonical spec per rfc-decision-4f1e12e5's "attested
# removals" pass (srs#443, srs#444) but still back live, in-progress srs-rust
# functionality that a mirror-sync alone cannot resolve — code removal is a
# separate, deliberate call, not something to do silently as drift cleanup.
# Each entry names the tracking issue that owns the reconciliation. Remove an
# entry here only when its issue closes with the schema (and its consuming
# code) actually deleted.
#   - revisions.json: ext:addressability's Revision sidecar; srs-rust#866
#     tracks it — explicitly "the timing and shape of any actual code removal
#     is an srs-rust maintainer call" (rfc-decision-2a1e1590's return trigger),
#     not yet made.
#   - typed-record.json: srs#505 retired Tier 1 (TypedRecord) from the spec
#     (rfc-decision-53635966), but #883 deliberately kept the Tier-1 raw-JSON
#     handling code paths live (catalog classification, discovery text
#     projection, container-view display) since a rev-3 repository may still
#     carry real Tier-1 content — srs-rust#888 tracks the actual code removal;
#     remove this entry together with that removal, not separately.
DECLARED_EXTRA_ALLOWLIST=(
    "revisions.json"
    "typed-record.json"
)
is_allowlisted() {
    local needle="$1"
    for entry in "${DECLARED_EXTRA_ALLOWLIST[@]}"; do
        [[ "${entry}" == "${needle}" ]] && return 0
    done
    return 1
}

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
        if is_allowlisted "${filename}"; then
            echo "EXTRA in artifact (declared transitional, see script comment): ${filename}"
        else
            echo "EXTRA in artifact (not in spec): ${filename}"
            DRIFT=1
        fi
    fi
done

EXPECTED_SUMS="${DST}/SHA256SUMS"
if [[ ! -f "${EXPECTED_SUMS}" ]]; then
    echo "MISSING: ${EXPECTED_SUMS}"
    DRIFT=1
else
    TMPFILE="$(mktemp)"
    # Plain `sort` (by hash) — must match sync-schemas-from-spec.sh exactly.
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
