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
#
# revisions.json's entry is retired (srs-rust#866): rfc-decision-2a1e1590's
# code-removal call is now made — the write path, the mirror, and this
# tolerance are all clean-cut together. Empty for now; the next entry follows
# the same per-item pattern when its issue makes the same call.
DECLARED_EXTRA_ALLOWLIST=(
    # srs-rust#910: the DocumentView -> Composition rename (rfc-decision-92d2da05)
    # lands here first per the binary-first choreography (CLAUDE.md "Gates and
    # choreography" — support lands + releases before the spec PR merges). Our
    # mirror already carries composition.json; the canonical srs spec schema
    # still ships document-view.json until srs#523 merges. Remove this entry
    # when srs#523 merges and the next sync-schemas-from-spec.sh run picks up
    # the renamed canonical file.
    "composition.json"
)
is_allowlisted() {
    local needle="$1"
    for entry in "${DECLARED_EXTRA_ALLOWLIST[@]}"; do
        [[ "${entry}" == "${needle}" ]] && return 0
    done
    return 1
}

# Declared transitional allowlist for the reverse direction: a file the
# canonical spec still ships that our mirror has already renamed away from
# (same srs-rust#910 rename, same expiry — srs#523 merging). Symmetric to
# DECLARED_EXTRA_ALLOWLIST; add an entry here only when the mirror renames a
# schema ahead of the spec's own rename landing.
DECLARED_MISSING_ALLOWLIST=(
    "document-view.json"
)
is_missing_allowlisted() {
    local needle="$1"
    for entry in "${DECLARED_MISSING_ALLOWLIST[@]}"; do
        [[ "${entry}" == "${needle}" ]] && return 0
    done
    return 1
}

for src_file in "${SRC}"/*.json; do
    filename="$(basename "${src_file}")"
    dst_file="${DST}/${filename}"
    if [[ ! -f "${dst_file}" ]]; then
        if is_missing_allowlisted "${filename}"; then
            echo "MISSING in artifact (declared transitional, see script comment): ${filename}"
        else
            echo "MISSING in artifact: ${filename}"
            DRIFT=1
        fi
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
