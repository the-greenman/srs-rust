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
# tolerance are all clean-cut together. composition.json's entry is retired
# (srs-rust#910): srs#523 merged, and the next sync-schemas-from-spec.sh run
# picked up the renamed canonical file — the mirror and spec now carry the
# same composition.json bytes.
#
# relations-collection.json (srs-rust#929): srs#527 deleted this schema from
# the canonical spec outright ("dead shape with no successor" — RFC-038 moved
# relations to one-file-per-relation under relations/). But srs-rust's RFC-038
# [R11]-exempt legacy migration surface (relation_service.rs's
# load_relations_collection/schema_validate_relation, gated on
# store.rfc038_exempt()) still reads and schema-validates real
# dataModelRevision < 2 repositories' relations-collection.json files during
# migration — code that is still live, not dead. The schema stays as a
# permanently-retained legacy validator until that migration surface itself
# is retired (the same full-removal call already made for Tier-1/TypedRecord
# in #913 and ext:federation in #912) — not something a mirror-sync should
# ever silently decide.
DECLARED_EXTRA_ALLOWLIST=("relations-collection.json")
is_allowlisted() {
    local needle="$1"
    for entry in "${DECLARED_EXTRA_ALLOWLIST[@]}"; do
        [[ "${entry}" == "${needle}" ]] && return 0
    done
    return 1
}

# Declared transitional allowlist for the reverse direction: a file the
# canonical spec still ships that our mirror has already renamed away from
# (same srs-rust#910 rename). Retired: srs#523 merged, document-view.json is
# gone from the canonical spec too. Symmetric to DECLARED_EXTRA_ALLOWLIST; add
# an entry here only when the mirror renames a schema ahead of the spec's own
# rename landing.
#
# srsj-envelope.json's entry is retired (srs-rust#937): the file is now
# mirrored and registered in srs-schema (SRSJ_ENVELOPE_SCHEMA_ID,
# registry-only — see that constant's doc comment for why no runtime code
# path validates a `.srsj` document against it).
DECLARED_MISSING_ALLOWLIST=()
is_missing_allowlisted() {
    local needle="$1"
    for entry in "${DECLARED_MISSING_ALLOWLIST[@]}"; do
        [[ "${entry}" == "${needle}" ]] && return 0
    done
    return 1
}

# Declared transitional allowlist for CONTENT drift: a file present on both
# sides whose bytes differ because our mirror leads the spec on an already-
# ratified rename (srs-rust#910: the Composition rename, rfc-decision-92d2da05,
# and the semanticObjectType collapse, owner ruling on #383/rfc-decision-
# c8704763) — every entry here is prose or a key rename inside a file that
# also exists (unlike the whole-file EXTRA/MISSING pairs above). Retired:
# srs#523/#524 merged and the next sync-schemas-from-spec.sh run picked up the
# renamed canonical content — the mirror and spec are byte-identical again.
#
# srs-rust#924's entry is retired: srs#525 merged (PR #540, commit 781ffb9),
# and the next sync-schemas-from-spec.sh run picked up the same
# composition.json/discovery.json/manifest.json/view.json bytes from the
# canonical spec — the mirror and spec are byte-identical again.
#
# field.json's entry is retired (srs-rust#932): the mirror is synced (widened
# valueRange enum + allowedValues.items.type) and srs-core's FieldType
# validation (R2/R3/R9) now implements both new capabilities. The frozen
# seed's own `$defs.FieldType.allOf` gap this surfaced (rangeType wasn't
# permitted for the map-of-ref shape) was filed upstream as srs#551 and fixed
# there (PR #555, re-synced here) — the map-of-ref shapes now live in
# `every_field_type_shape_passes_the_schema_contract` alongside every other
# FieldType shape, not a separate pending test.
#
# protocol.json's entry is retired: srs-rust#930 landed the reference-
# implementation rename (Rust structs, catalog.rs, CLI/MCP payload call
# sites, fixtures) alongside this mirror bump, so the mirror and
# srs-core's actual Protocol/ProtocolStage shape are byte-identical again.
DECLARED_CONTENT_DRIFT_ALLOWLIST=()
is_content_drift_allowlisted() {
    local needle="$1"
    for entry in "${DECLARED_CONTENT_DRIFT_ALLOWLIST[@]}"; do
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
        if is_content_drift_allowlisted "${filename}"; then
            echo "DRIFT detected (declared transitional, see script comment): ${filename}"
        else
            echo "DRIFT detected: ${filename}"
            DRIFT=1
        fi
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
