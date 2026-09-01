#!/usr/bin/env bash
# Sync canonical schemas from the spec repo into the srs-schema artifact crate.
#
# One sync source, chosen once (srs-rust#874): the release asset, by default.
# A silent local-sibling fallback previously mirrored a stale checkout
# (`../srs` sitting on a non-master branch behind master) with a green exit
# code — the exact false-green trap PR #879 also had to harden against on the
# test side (crates/srs-projection/tests/rfc_035_parity.rs's spec_repo()).
#
# `--local <path>` (or the equivalent $SRS_SPEC_DIR) opts into a local
# checkout explicitly, and only when its HEAD is provably an ancestor of (or
# equal to) that checkout's own origin/master — never a diverged or foreign
# branch. The resolved source (release tag, or local commit + ancestor
# relationship) is always printed, so a PR diff shows provenance.
#
# Usage:
#   scripts/sync-schemas-from-spec.sh                 # release asset (default)
#   scripts/sync-schemas-from-spec.sh --local <path>   # explicit local checkout, ancestor-checked
#
# $SRS_SPEC_DIR is the environment-variable spelling of --local — set one or
# the other, never both.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
DST="${WORKSPACE_DIR}/crates/srs-schema/schemas/2.0"

LOCAL_PATH=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --local)
            if [[ $# -lt 2 ]]; then
                echo "ERROR: --local requires a path argument" >&2
                exit 1
            fi
            LOCAL_PATH="$2"
            shift 2
            ;;
        *)
            echo "ERROR: unknown argument '$1'" >&2
            echo "Usage: ${0} [--local <path>]" >&2
            exit 1
            ;;
    esac
done

if [[ -n "${LOCAL_PATH}" && -n "${SRS_SPEC_DIR:-}" ]]; then
    echo "ERROR: both --local and \$SRS_SPEC_DIR are set — pick one explicit source, not two." >&2
    exit 1
fi
LOCAL_PATH="${LOCAL_PATH:-${SRS_SPEC_DIR:-}}"

# Detect the GitHub repo owner from the current remote so this works across forks.
_owner() {
    git -C "${WORKSPACE_DIR}" remote get-url origin 2>/dev/null \
        | sed 's|.*github\.com[:/]\([^/]*\)/.*|\1|'
}

SRC=""

if [[ -n "${LOCAL_PATH}" ]]; then
    if [[ ! -e "${LOCAL_PATH}/.git" ]]; then
        echo "ERROR: --local ${LOCAL_PATH} is not a git checkout (no .git)." >&2
        exit 1
    fi
    if [[ ! -d "${LOCAL_PATH}/docs/schema/2.0" ]]; then
        echo "ERROR: --local ${LOCAL_PATH} has no docs/schema/2.0/ — not an srs spec checkout." >&2
        exit 1
    fi

    echo "Verifying --local ${LOCAL_PATH} against its origin/master (fetching)..." >&2
    if ! git -C "${LOCAL_PATH}" fetch --quiet origin master; then
        echo "ERROR: could not fetch origin/master in ${LOCAL_PATH} — cannot verify it isn't stale." >&2
        echo "       Drop --local and use the release asset, or fix the checkout's 'origin' remote." >&2
        exit 1
    fi

    HEAD_SHA="$(git -C "${LOCAL_PATH}" rev-parse HEAD)"
    MASTER_SHA="$(git -C "${LOCAL_PATH}" rev-parse origin/master)"

    if ! git -C "${LOCAL_PATH}" merge-base --is-ancestor "${HEAD_SHA}" "${MASTER_SHA}"; then
        echo "ERROR: ${LOCAL_PATH} HEAD (${HEAD_SHA}) is not an ancestor of its origin/master (${MASTER_SHA})." >&2
        echo "       This is srs-rust#874's exact trap: a diverged or foreign-branch local checkout" >&2
        echo "       would silently mirror the wrong schemas with a green exit code. Rebase/merge" >&2
        echo "       the checkout onto origin/master, or drop --local and use the release asset." >&2
        exit 1
    fi

    SRC="${LOCAL_PATH}/docs/schema/2.0"
    if [[ "${HEAD_SHA}" == "${MASTER_SHA}" ]]; then
        echo "Source: local checkout ${LOCAL_PATH} @ ${HEAD_SHA} (= origin/master)"
    else
        echo "Source: local checkout ${LOCAL_PATH} @ ${HEAD_SHA} (ancestor of origin/master @ ${MASTER_SHA})"
    fi
else
    if ! command -v gh &>/dev/null; then
        echo "ERROR: gh CLI not found. Either:" >&2
        echo "  • Install the gh CLI and authenticate: gh auth login, or" >&2
        echo "  • Use --local <path> with a checkout whose HEAD is an ancestor of its origin/master" >&2
        exit 1
    fi

    OWNER="$(_owner)"
    SRS_REPO="${OWNER}/srs"
    TMPDIR="$(mktemp -d)"
    trap 'rm -rf "${TMPDIR}"' EXIT

    RELEASE_TAG="$(gh release view --repo "${SRS_REPO}" --json tagName -q .tagName)"
    echo "Downloading schemas-2.0.tar.gz from ${SRS_REPO}@${RELEASE_TAG} (latest release)..." >&2
    gh release download "${RELEASE_TAG}" \
        --repo "${SRS_REPO}" \
        --pattern "schemas-2.0.tar.gz" \
        --dir "${TMPDIR}"

    mkdir -p "${TMPDIR}/extracted"
    tar -xzf "${TMPDIR}/schemas-2.0.tar.gz" -C "${TMPDIR}/extracted"
    SRC="${TMPDIR}/extracted"
    echo "Source: release asset ${SRS_REPO}@${RELEASE_TAG}"
fi

mkdir -p "${DST}"
cp "${SRC}"/*.json "${DST}/"

cd "${DST}"
# IMPORTANT: plain `sort` (sorts by hash), NOT `sort -k2` (sorts by filename).
# check-schema-drift.sh validates SHA256SUMS with the exact same command —
# using `sort -k2` or any other variant will cause "SHA256SUMS mismatch" in
# CI. Never regenerate SHA256SUMS manually — always use this script.
sha256sum *.json | sort > SHA256SUMS

echo "Synced $(ls "${DST}"/*.json | wc -l) schemas + SHA256SUMS from ${SRC}"
