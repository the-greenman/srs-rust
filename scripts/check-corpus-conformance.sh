#!/usr/bin/env bash
# check-corpus-conformance.sh — validate first-party SRS corpora with a given `srs` binary.
#
# The release gate (the-greenman/srs#392 row 2). Every merge to master auto-tags a release that
# humans and CI pull; nothing checked that the binary in it can still read the corpora it exists to
# read. Both directions have bitten: build.276 read migrated repositories as *silently empty*, and
# build.285 rejected pre-cutover ones loudly. Both were caught by hand and recorded as ledger
# warnings. This makes them a red X.
#
# Usage:
#   SRS=/path/to/srs ./scripts/check-corpus-conformance.sh <label>=<repo-path> [...]
#
# Two envelopes, neither signalled by the exit code — this is the whole reason the check is a script
# and not a bare `srs repo validate` invocation:
#
#   {"ok": false, "diagnostics": ["[path] message", ...]}                  # load failed, NO payload
#   {"ok": true,  "payload": {"summary": {"errors": N, ...}, ...}}         # loaded, N diagnostics
#
# `srs repo validate` exits 0 for both. Reading only `.payload.summary.errors` misreads the first as
# `null` and reports the binary's envelope as the problem rather than the corpus, so both are
# handled. A summary that is not a number is a failure, never a pass: a changed envelope under a
# future binary must not read as a clean corpus nobody looked at.

set -uo pipefail

SRS="${SRS:-srs}"

if [ "$#" -eq 0 ]; then
  echo "usage: SRS=/path/to/srs $0 <label>=<repo-path> [...]" >&2
  exit 2
fi

echo "srs binary: $SRS"
"$SRS" --version || true
sha256sum "$SRS" 2>/dev/null || true
echo

failed=0

for spec in "$@"; do
  label="${spec%%=*}"
  path="${spec#*=}"

  if [ ! -d "$path" ]; then
    echo "::error::$label: repository path does not exist: $path"
    echo "  A corpus that vanished is not a corpus that passed — check the checkout step."
    failed=1
    continue
  fi

  # `|| true`: a non-zero exit still carries the JSON envelope on stdout, and the envelope is what
  # decides. An empty stdout is handled below and is fatal.
  out="$("$SRS" repo validate --repo "$path" 2>/dev/null)" || true

  if [ -z "${out//[[:space:]]/}" ]; then
    echo "::error::$label: srs repo validate produced no output for $path"
    failed=1
    continue
  fi

  ok="$(printf '%s' "$out" | jq -r 'if has("ok") then .ok else "MISSING" end' 2>/dev/null)" || ok="UNPARSEABLE"

  case "$ok" in
    UNPARSEABLE|MISSING|"")
      echo "::error::$label: srs repo validate returned no readable \`ok\` field — the output is not the CLI envelope this gate understands."
      printf '%s\n' "$out" | head -c 2000
      failed=1
      ;;
    false)
      # Load failure: top-level `diagnostics` is an array of strings, each naming a file.
      echo "::error::$label: srs repo validate reported ok=false — the binary cannot load this corpus."
      printf '%s' "$out" | jq -r '.diagnostics[]? | "::error::'"$label"': \(.)"'
      failed=1
      ;;
    true)
      errors="$(printf '%s' "$out" | jq -r '.payload.summary.errors' 2>/dev/null)"
      checked="$(printf '%s' "$out" | jq -r '.payload.summary.checked' 2>/dev/null)"
      warnings="$(printf '%s' "$out" | jq -r '.payload.summary.warnings' 2>/dev/null)"
      if ! [[ "$errors" =~ ^[0-9]+$ ]] || ! [[ "$checked" =~ ^[0-9]+$ ]] || ! [[ "$warnings" =~ ^[0-9]+$ ]]; then
        echo "::error::$label: ok=true but payload.summary is not readable (checked=$checked errors=$errors warnings=$warnings)."
        echo "  The envelope changed shape; this gate has stopped checking rather than gone green."
        failed=1
        continue
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
        failed=1
        continue
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
        failed=1
      fi
      ;;
    *)
      echo "::error::$label: srs repo validate returned an unexpected \`ok\` value: $ok"
      failed=1
      ;;
  esac
done

echo
if [ "$failed" -ne 0 ]; then
  echo "✗ Corpus conformance FAILED for this build."
  exit 1
fi
echo "✓ All first-party corpora validate against this build."
