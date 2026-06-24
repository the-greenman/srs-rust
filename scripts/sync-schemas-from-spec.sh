#!/usr/bin/env bash
# Sync canonical schemas from the spec repo into the srs-schema artifact crate.
#
# Resolution order:
#   1. Local sibling checkout: $SRS_SPEC_DIR or ../srs (default)
#   2. GitHub release asset:   gh release download from the-greenman/srs
#      (used automatically when the local checkout is absent — e.g. web sessions)
#
# Usage: scripts/sync-schemas-from-spec.sh [SRS_SPEC_DIR]
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
SPEC_DIR="${SRS_SPEC_DIR:-${WORKSPACE_DIR}/../srs}"
SRC="${SPEC_DIR}/docs/schema/2.0"
DST="${WORKSPACE_DIR}/crates/srs-schema/schemas/2.0"

# Detect the GitHub repo owner from the current remote so this works across forks.
_owner() {
    git -C "${WORKSPACE_DIR}" remote get-url origin 2>/dev/null \
        | sed 's|.*github\.com[:/]\([^/]*\)/.*|\1|'
}

if [[ ! -d "${SRC}" ]]; then
    echo "Local srs checkout not found at ${SRC}." >&2
    echo "Falling back to GitHub release asset..." >&2

    if ! command -v gh &>/dev/null; then
        echo "ERROR: gh CLI not found. Either:" >&2
        echo "  • Set SRS_SPEC_DIR to a local srs checkout, or" >&2
        echo "  • Install the gh CLI and authenticate: gh auth login" >&2
        exit 1
    fi

    OWNER="$(_owner)"
    SRS_REPO="${OWNER}/srs"
    TMPDIR="$(mktemp -d)"
    trap 'rm -rf "${TMPDIR}"' EXIT

    echo "Downloading schemas-2.0.tar.gz from ${SRS_REPO} (latest release)..." >&2
    gh release download \
        --repo "${SRS_REPO}" \
        --pattern "schemas-2.0.tar.gz" \
        --dir "${TMPDIR}"

    mkdir -p "${TMPDIR}/extracted"
    tar -xzf "${TMPDIR}/schemas-2.0.tar.gz" -C "${TMPDIR}/extracted"
    SRC="${TMPDIR}/extracted"
fi

mkdir -p "${DST}"
cp "${SRC}"/*.json "${DST}/"

cd "${DST}"
# IMPORTANT: plain `sort` (sorts by hash), NOT `sort -k2` (sorts by filename).
# check-schema-drift.sh validates SHA256SUMS with the same `sha256sum *.json | sort`
# and will fail if the sort order differs. Never regenerate SHA256SUMS manually —
# always use this script.
sha256sum *.json | sort > SHA256SUMS

echo "Synced $(ls "${DST}"/*.json | wc -l) schemas + SHA256SUMS from ${SRC}"
