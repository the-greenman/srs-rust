#!/usr/bin/env bash
# check-corpus-conformance.test.sh
#
# Self-check for check-corpus-conformance.sh's migration path (srs-rust#1145). Proves:
#   1. a current-revision corpus passes as-is (no "migration pending");
#   2. an older-revision corpus the binary refuses passes as "migration pending: <ids>", and the
#      corpus on disk is left untouched;
#   3. a corpus that migrates but fails validation afterwards fails;
#   4. a corpus that neither loads nor migrates fails, and so does a vanished path.
#
# The older-revision corpus is scripts/fixtures/corpus-gate-rev7: the srs gallery example
# (docs/spec/examples/gallery-project-v2) at dataModelRevision 7, copied from srs@0a808811 — the
# last commit before srs#852 migrated it to revision 8. When the binary stops offering a
# migration path from revision 7, refresh this fixture to the oldest revision it still migrates.
#
# Usage: SRS=/path/to/srs bash scripts/check-corpus-conformance.test.sh
#        (defaults to target/debug/srs — run `cargo build` first)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GATE="${SCRIPT_DIR}/check-corpus-conformance.sh"
FIXTURE="${SCRIPT_DIR}/fixtures/corpus-gate-rev7"
SRS="${SRS:-${SCRIPT_DIR}/../target/debug/srs}"
[ -x "$SRS" ] || { echo "FAIL: no srs binary at $SRS (cargo build, or set SRS)" >&2; exit 1; }

WORK="$(mktemp -d)"
trap 'rm -rf "${WORK}"' EXIT

fail() {
  echo "FAIL: $1" >&2
  [ -n "${2:-}" ] && printf '%s\n' "$2" >&2
  exit 1
}

# gate <expected-exit> <label>=<path> ... — run the gate, assert its exit code, echo its output.
gate() {
  local want="$1" out rc
  shift
  rc=0
  out="$(GITHUB_STEP_SUMMARY="${WORK}/summary.md" bash "$GATE" --srs "$SRS" "$@" 2>&1)" || rc=$?
  [ "$rc" -eq "$want" ] || fail "expected exit $want, got $rc for: $*" "$out"
  printf '%s' "$out"
}

before="$(cd "$FIXTURE" && find . -type f -exec sha256sum {} + | sort)"

# 2. Older revision -> migration pending, pass, fixture untouched.
out="$(gate 0 rev7="$FIXTURE")"
grep -q "rev7: migration pending: rfc043-container-entries" <<<"$out" \
  || fail "rev-7 corpus was not reported as migration pending" "$out"
grep -q "migration pending" "${WORK}/summary.md" || fail "job summary does not report migration pending"
after="$(cd "$FIXTURE" && find . -type f -exec sha256sum {} + | sort)"
[ "$before" = "$after" ] || fail "the gate wrote to the corpus it was checking"
echo "ok - older-revision corpus passes as migration pending; corpus untouched"

# 1. Current revision (the fixture migrated for real, via the CLI) -> passes as-is.
cp -a "$FIXTURE" "${WORK}/current"
"$SRS" repo apply-migration --repo "${WORK}/current" --id rfc043-container-entries >/dev/null
"$SRS" repo apply-migration --repo "${WORK}/current" --id rfc046-actor-provenance >/dev/null
out="$(gate 0 current="${WORK}/current")"
grep -q "current: checked [1-9]" <<<"$out" || fail "current corpus did not validate as-is" "$out"
grep -q "migration pending" <<<"$out" && fail "current corpus reported migration pending" "$out"
echo "ok - current-revision corpus passes as-is"

# 3. Migrates cleanly, then fails validation (a field value of the wrong datatype).
cp -a "$FIXTURE" "${WORK}/invalid"
rec="${WORK}/invalid/records/tier-2/article-c503fae5.json"
jq '.fieldValues.article_number = 12345' "$rec" > "${WORK}/rec.json" && mv "${WORK}/rec.json" "$rec"
out="$(gate 1 invalid="${WORK}/invalid")"
grep -q "invalid: still refused after applying: rfc043-container-entries" <<<"$out" \
  || fail "invalid corpus did not fail at validation after migration" "$out"
grep -q "fails after migration" "${WORK}/summary.md" || fail "job summary does not report the post-migration failure"
echo "ok - corpus invalid after migration fails"

# 4. Neither loads nor migrates; vanished path; and one failure fails a mixed run.
mkdir -p "${WORK}/junk/.srs"
echo '{"dataModelRevision": 7, "container": {"rootInstanceIds": []}}' > "${WORK}/junk/manifest.json"
gate 1 junk="${WORK}/junk" >/dev/null
gate 1 gone="${WORK}/does-not-exist" >/dev/null
gate 1 rev7="$FIXTURE" gone="${WORK}/does-not-exist" >/dev/null
echo "ok - unloadable/unmigratable and vanished corpora fail"

echo "PASS: check-corpus-conformance.sh"
