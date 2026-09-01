#!/usr/bin/env bash
# sync-schemas-from-spec.test.sh
#
# Self-check for sync-schemas-from-spec.sh's --local source (srs-rust#874):
# a diverged/foreign-branch checkout must be REFUSED (the exact false-green
# trap this fix exists for), a checkout that is a clean ancestor of its
# origin/master must be ACCEPTED and print its provenance, and passing both
# --local and $SRS_SPEC_DIR must be refused as a conflicting source.
#
# Builds throwaway git repos under a tmpdir — no network, no gh CLI required
# (the release-asset default path isn't exercised here; it needs a real
# release and is covered by ordinary manual/CI use of the script).
#
# Usage: bash scripts/sync-schemas-from-spec.test.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SYNC_SCRIPT="${SCRIPT_DIR}/sync-schemas-from-spec.sh"
WORKSPACE_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

WORK="$(mktemp -d)"
trap 'rm -rf "${WORK}"' EXIT

fail() {
    echo "FAIL: $1" >&2
    exit 1
}

# --- Build a fake "srs" spec repo with an origin remote -------------------
ORIGIN="${WORK}/origin-srs.git"
git init --quiet --bare "${ORIGIN}"

SEED="${WORK}/seed"
git init --quiet "${SEED}"
git -C "${SEED}" config user.email "test@example.com"
git -C "${SEED}" config user.name "Test"
mkdir -p "${SEED}/docs/schema/2.0"
echo '{"seed":true}' > "${SEED}/docs/schema/2.0/seed.json"
git -C "${SEED}" add -A
git -C "${SEED}" commit --quiet -m "seed schema"
git -C "${SEED}" branch -M master
git -C "${SEED}" remote add origin "${ORIGIN}"
git -C "${SEED}" push --quiet origin master

# A clean checkout, exactly at origin/master.
CLEAN="${WORK}/clean-clone"
git clone --quiet "${ORIGIN}" "${CLEAN}"
git -C "${CLEAN}" checkout --quiet master 2>/dev/null || true

# --- Case 1: clean checkout (HEAD == origin/master) is ACCEPTED -----------
DST_1="${WORK}/dst1/crates/srs-schema/schemas/2.0"
mkdir -p "$(dirname "${DST_1}")"
FAKE_WORKSPACE_1="${WORK}/fake-workspace-1"
mkdir -p "${FAKE_WORKSPACE_1}/scripts" "${FAKE_WORKSPACE_1}/crates/srs-schema/schemas/2.0"
cp "${SYNC_SCRIPT}" "${FAKE_WORKSPACE_1}/scripts/sync-schemas-from-spec.sh"
git init --quiet "${FAKE_WORKSPACE_1}" >/dev/null 2>&1 || true

OUT_1="$(bash "${FAKE_WORKSPACE_1}/scripts/sync-schemas-from-spec.sh" --local "${CLEAN}" 2>&1)" \
    || fail "clean ancestor checkout was refused, expected acceptance:\n${OUT_1:-}"
echo "${OUT_1}" | grep -q "= origin/master" \
    || fail "acceptance output did not report '= origin/master' provenance:\n${OUT_1}"
[[ -f "${FAKE_WORKSPACE_1}/crates/srs-schema/schemas/2.0/seed.json" ]] \
    || fail "seed.json was not copied on the accepted path"
echo "PASS: clean checkout (HEAD == origin/master) accepted, provenance printed"

# --- Case 2: diverged/foreign-branch checkout is REFUSED (RED case) -------
# Simulate the real #874 incident: a checkout on its own branch with a local
# commit never merged to master (e.g. a long-lived docs branch).
STALE="${WORK}/stale-clone"
git clone --quiet "${ORIGIN}" "${STALE}"
git -C "${STALE}" checkout --quiet -b docs/design-captures-tier0
git -C "${STALE}" config user.email "test@example.com"
git -C "${STALE}" config user.name "Test"
echo '{"diverged":true}' > "${STALE}/docs/schema/2.0/seed.json"
git -C "${STALE}" add -A
git -C "${STALE}" commit --quiet -m "diverged docs-branch edit"

# Advance origin/master too, so the stale clone is neither ahead-only nor
# behind-only of the *current* origin/master — a true divergence.
git -C "${SEED}" checkout --quiet master
echo '{"seed":true,"v2":true}' > "${SEED}/docs/schema/2.0/seed.json"
git -C "${SEED}" commit --quiet -am "advance master"
git -C "${SEED}" push --quiet origin master

FAKE_WORKSPACE_2="${WORK}/fake-workspace-2"
mkdir -p "${FAKE_WORKSPACE_2}/scripts" "${FAKE_WORKSPACE_2}/crates/srs-schema/schemas/2.0"
cp "${SYNC_SCRIPT}" "${FAKE_WORKSPACE_2}/scripts/sync-schemas-from-spec.sh"

if OUT_2="$(bash "${FAKE_WORKSPACE_2}/scripts/sync-schemas-from-spec.sh" --local "${STALE}" 2>&1)"; then
    fail "diverged checkout was ACCEPTED — the exact silent-stale-sibling trap srs-rust#874 fixes:\n${OUT_2}"
fi
echo "${OUT_2}" | grep -q "is not an ancestor of its origin/master" \
    || fail "refusal message did not name the ancestor check:\n${OUT_2}"
[[ ! -f "${FAKE_WORKSPACE_2}/crates/srs-schema/schemas/2.0/seed.json" ]] \
    || fail "seed.json was copied despite refusal — diverged content leaked into the mirror"
echo "PASS: diverged/foreign-branch checkout refused, nothing copied"

# --- Case 3: --local and $SRS_SPEC_DIR both set is refused ----------------
FAKE_WORKSPACE_3="${WORK}/fake-workspace-3"
mkdir -p "${FAKE_WORKSPACE_3}/scripts"
cp "${SYNC_SCRIPT}" "${FAKE_WORKSPACE_3}/scripts/sync-schemas-from-spec.sh"

if OUT_3="$(SRS_SPEC_DIR="${CLEAN}" bash "${FAKE_WORKSPACE_3}/scripts/sync-schemas-from-spec.sh" --local "${CLEAN}" 2>&1)"; then
    fail "conflicting --local + \$SRS_SPEC_DIR was accepted, expected refusal:\n${OUT_3}"
fi
echo "${OUT_3}" | grep -q "pick one explicit source" \
    || fail "conflict refusal message missing:\n${OUT_3}"
echo "PASS: conflicting --local + \$SRS_SPEC_DIR refused"

echo "OK: all sync-schemas-from-spec.sh --local source checks passed"
