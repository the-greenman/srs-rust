#!/usr/bin/env bash
# check-corpus-conformance.sh — validate first-party SRS corpora with a given `srs` binary.
#
# The release gate (the-greenman/srs#392 row 2). Every merge to master auto-tags a release that
# humans and CI pull; nothing checked that the binary in it can still read the corpora it exists to
# read. That has bitten by hand more than once — build.285 rejects a pre-cutover corpus loudly, and
# a binary that reads a corpus as empty is the quieter twin. Each was caught by a person and
# recorded as a ledger warning. This makes them a red X.
#
# The silent-emptiness half is deliberately NOT attributed to a specific build here: build.276,
# which an earlier draft named, does not reproduce it — against today's migrated corpora it fails
# loudly with `missing field 'instanceIndex'`. The zero-instance check below stands on its own
# merit (nothing else distinguishes an empty read from a clean one), not on that incident.
#
# Usage:
#   ./scripts/check-corpus-conformance.sh [--srs /path/to/srs] <label>=<repo-path> [...]
#   (or SRS=/path/to/srs in the environment; --srs wins)
#
# MIGRATION PENDING (srs-rust#1145). A data-model revision bump ships a binary that REFUSES the
# previous revision and offers a registry migration for it. Validating the corpora as-is then
# deadlocks the release (build.417, RFC-043 rev 8, had to be published by hand): the corpora cannot
# migrate until the release is published, and the release is not published until they validate.
# So the question this gate asks is "can this binary load the corpus, OR migrate it through the
# registry and then validate it?":
#
#   loads                        -> validated as-is (errors fail, as before);
#   refused (ok=false)           -> the corpus is COPIED to a temp dir, every `needed` migration
#                                   is applied there in registry order with the same CLI commands a
#                                   user runs (`srs repo migrations` / `srs repo apply-migration`,
#                                   no gate-only logic), and the result is validated. 0 errors is
#                                   a pass reported as "migration pending: <ids>";
#   neither loads nor migrates   -> fails (a migration that errors, or none on offer, is a red X).
#
# The checked-out corpora are never written to.
#
set -uo pipefail

SRS="${SRS:-srs}"
if [ "${1:-}" = "--srs" ]; then
  SRS="${2:?--srs needs a path}"
  shift 2
fi

if [ "$#" -eq 0 ]; then
  echo "usage: $0 [--srs /path/to/srs] <label>=<repo-path> [...]" >&2
  exit 2
fi

echo "srs binary: $SRS"
"$SRS" --version || true
sha256sum "$SRS" 2>/dev/null || true
echo

# Append a line to the GitHub job summary when there is one; a no-op locally.
summary() {
  if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then echo "$*" >> "$GITHUB_STEP_SUMMARY"; fi
  return 0
}

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# validate <label> <path> — 0 pass, 1 fail (already reported), 3 load refused (ok=false). A refusal
# is NOT reported here: the caller decides whether it is final. Its diagnostics land in $refusal.
refusal=""
validate() {
  local label="$1" path="$2" out ok errors checked warnings
  # `|| true`: a non-zero exit still carries the JSON envelope on stdout, and the envelope is what
  # decides. An empty stdout is handled below and is fatal.
  out="$("$SRS" repo validate --repo "$path" 2>/dev/null)" || true

  if [ -z "${out//[[:space:]]/}" ]; then
    echo "::error::$label: srs repo validate produced no output for $path"
    return 1
  fi

  ok="$(printf '%s' "$out" | jq -r 'if has("ok") then .ok else "MISSING" end' 2>/dev/null)" || ok="UNPARSEABLE"

  case "$ok" in
    UNPARSEABLE|MISSING|"")
      echo "::error::$label: srs repo validate returned no readable \`ok\` field — the output is not the CLI envelope this gate understands."
      printf '%s\n' "$out" | head -c 2000
      return 1
      ;;
    false)
      # Load failure: top-level `diagnostics` is an array of strings, each naming a file.
      refusal="$(printf '%s' "$out" | jq -r '.diagnostics[]?')"
      return 3
      ;;
    true)
      errors="$(printf '%s' "$out" | jq -r '.payload.summary.errors' 2>/dev/null)"
      checked="$(printf '%s' "$out" | jq -r '.payload.summary.checked' 2>/dev/null)"
      warnings="$(printf '%s' "$out" | jq -r '.payload.summary.warnings' 2>/dev/null)"
      if ! [[ "$errors" =~ ^[0-9]+$ ]] || ! [[ "$checked" =~ ^[0-9]+$ ]] || ! [[ "$warnings" =~ ^[0-9]+$ ]]; then
        echo "::error::$label: ok=true but payload.summary is not readable (checked=$checked errors=$errors warnings=$warnings)."
        echo "  The envelope changed shape; this gate has stopped checking rather than gone green."
        return 1
      fi
      # An empty corpus is the silent-failure mode this gate exists for: a binary that reads a
      # migrated repository as zero instances and reports success. Zero checked is never a pass.
      #
      # KNOWN CEILING — this catches TOTAL emptiness only. A binary that silently drops *some*
      # instance family (say every `governance/decision` record) would report a smaller non-zero
      # count and pass. The obvious fix — a recorded minimum per corpus — was rejected: the counts
      # move with ordinary authoring, so a baseline would go red on content changes that are not
      # regressions, and a gate whose failures are usually wrong gets ignored or disabled, which
      # costs more than the case it adds. The per-corpus count IS printed on every run, so a drop
      # from 32 to 11 is visible in the log and in the diff between two runs; making it fail
      # automatically needs a stable expected value this repository does not have. Revisit if the
      # corpora ever carry a declared instance count.
      if [ "$checked" -eq 0 ]; then
        echo "::error::$label: srs repo validate checked 0 instances — the corpus loaded as EMPTY."
        echo "  This is the silent-emptiness failure the gate exists to catch, not a clean result."
        return 1
      fi
      echo "$label: checked $checked, $errors errors, $warnings warnings"
      # Defensive, and deliberately kept: today the CLI emits the ok:false envelope whenever the
      # validation report is not ok, so an ok:true envelope always carries errors == 0 and this
      # branch does not fire. It is the assertion that that stays true — if a future binary starts
      # reporting per-instance errors inside a successful load, this reports them instead of
      # printing "0 errors" from a field nobody checked.
      if [ "$errors" -ne 0 ]; then
        printf '%s' "$out" \
          | jq -r '.payload.diagnostics[]? | select(.severity == "error") | "::error::'"$label"': \(.path // "?"): \(.message)"'
        return 1
      fi
      return 0
      ;;
    *)
      echo "::error::$label: srs repo validate returned an unexpected \`ok\` value: $ok"
      return 1
      ;;
  esac
}

# migrate <label> <copy> — apply every `needed` registry migration to the temp copy, earliest in
# registry order first, re-listing after each apply (a migration's status depends on the ones
# before it). A migration still reported `needed` after it was applied is a failure, not a retry,
# so the loop ends. Applied ids land in $applied.
applied=""
migrate() {
  local label="$1" copy="$2" list next again res
  applied=""
  while :; do
    list="$("$SRS" repo migrations --repo "$copy" 2>/dev/null)" || true
    if [ "$(printf '%s' "$list" | jq -r '.ok' 2>/dev/null)" != "true" ]; then
      echo "::error::$label: srs repo migrations failed on the temp copy — the registry cannot be read for this corpus."
      printf '%s' "$list" | jq -r '.diagnostics[]? | "::error::'"$label"': \(.)"' 2>/dev/null || printf '%s\n' "$list" | head -c 2000
      return 1
    fi
    again="$(printf '%s' "$list" | jq -r --arg done " $applied " \
      '[.payload.migrations[] | select(.status.needed) | .id
        | select(. as $id | $done | contains(" " + $id + " "))][0] // empty')"
    if [ -n "$again" ]; then
      echo "::error::$label: srs repo apply-migration --id $again succeeded but the registry still reports it needed — the migration does not converge."
      return 1
    fi
    next="$(printf '%s' "$list" | jq -r '[.payload.migrations[] | select(.status.needed) | .id][0] // empty')"
    [ -z "$next" ] && return 0
    res="$("$SRS" repo apply-migration --repo "$copy" --id "$next" 2>/dev/null)" || true
    if [ "$(printf '%s' "$res" | jq -r '.ok' 2>/dev/null)" != "true" ]; then
      echo "::error::$label: srs repo apply-migration --id $next failed — the corpus cannot be migrated by this binary."
      printf '%s' "$res" | jq -r '.diagnostics[]? | "::error::'"$label"': \(.)"' 2>/dev/null || printf '%s\n' "$res" | head -c 2000
      return 1
    fi
    echo "  applied $next"
    applied="${applied:+$applied }$next"
  done
}

summary "### Corpus conformance"
failed=0
pending=0
n=0

for spec in "$@"; do
  label="${spec%%=*}"
  path="${spec#*=}"
  n=$((n + 1))

  if [ ! -d "$path" ]; then
    echo "::error::$label: repository path does not exist: $path"
    echo "  A corpus that vanished is not a corpus that passed — check the checkout step."
    summary "- ✗ **$label**: repository path does not exist"
    failed=1
    continue
  fi

  validate "$label" "$path"
  rc=$?
  if [ "$rc" -eq 0 ]; then
    summary "- ✓ **$label**: validates as-is"
    continue
  fi
  if [ "$rc" -ne 3 ]; then
    summary "- ✗ **$label**: fails validation"
    failed=1
    continue
  fi

  # Refused as-is. Is there a registry path from here?
  echo "$label: this binary refuses the corpus as-is:"
  printf '%s\n' "$refusal" | sed 's/^/  /'
  echo "$label: migrating a temporary copy through the registry"
  copy="$work/$n"
  cp -a "$path" "$copy"
  rev_from="$(jq -r '.dataModelRevision // 0' "$copy/manifest.json" 2>/dev/null || echo '?')"
  if ! migrate "$label" "$copy"; then
    summary "- ✗ **$label**: refused, and the registry migration failed"
    failed=1
    continue
  fi
  if [ -z "$applied" ]; then
    echo "::error::$label: srs repo validate reported ok=false and the registry offers no needed migration — the binary cannot load this corpus."
    printf '%s\n' "$refusal" | sed "s/^/::error::$label: /"
    summary "- ✗ **$label**: refused, no registered migration"
    failed=1
    continue
  fi

  validate "$label (after migration)" "$copy"
  rc=$?
  if [ "$rc" -eq 3 ]; then
    echo "::error::$label: still refused after applying: $applied"
    printf '%s\n' "$refusal" | sed "s/^/::error::$label: /"
  fi
  if [ "$rc" -ne 0 ]; then
    summary "- ✗ **$label**: fails after migration ($applied)"
    failed=1
    continue
  fi
  # The registry does not say which migrations bump the data-model revision (some are structural,
  # e.g. a path rename), so report the manifest's revision change alongside the applied ids.
  rev_to="$(jq -r '.dataModelRevision // 0' "$copy/manifest.json" 2>/dev/null || echo '?')"
  if [ "$rev_from" = "$rev_to" ]; then revnote="dataModelRevision $rev_to unchanged"; else revnote="dataModelRevision $rev_from -> $rev_to"; fi
  echo "::notice::$label: migration pending: $applied ($revnote) — validates after migration; migrate the corpus repository."
  summary "- ⚠ **$label**: migration pending: \`$applied\` ($revnote) — the corpus is not broken; it validates once migrated"
  pending=$((pending + 1))
done

echo
if [ "$failed" -ne 0 ]; then
  echo "✗ Corpus conformance FAILED for this build."
  exit 1
fi
if [ "$pending" -ne 0 ]; then
  echo "✓ All first-party corpora validate against this build ($pending only after a pending registry migration)."
  exit 0
fi
echo "✓ All first-party corpora validate against this build."
