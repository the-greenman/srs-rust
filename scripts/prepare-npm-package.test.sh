#!/usr/bin/env bash
# Self-check for prepare-npm-package.sh (srs-rust#819): version from tag, name and
# access rewritten, malformed tags and a missing package.json refused.
# Usage: bash scripts/prepare-npm-package.test.sh
set -euo pipefail
SCRIPT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/prepare-npm-package.sh"
WORK="$(mktemp -d)"; trap 'rm -rf "${WORK}"' EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }

echo '{"name":"srs-bindings","version":"0.1.0","files":["a.wasm"],"types":"a.d.ts"}' > "${WORK}/package.json"
v="$(bash "${SCRIPT}" v0.1.0-build.612 "${WORK}")"
[[ "${v}" == "0.612.0" ]] || fail "version was ${v}"
[[ "$(jq -r .name "${WORK}/package.json")" == "@semanticops/srs-bindings" ]] || fail "name"
[[ "$(jq -r .version "${WORK}/package.json")" == "0.612.0" ]] || fail "package version"
[[ "$(jq -r .publishConfig.access "${WORK}/package.json")" == "public" ]] || fail "access"
[[ "$(jq -r .types "${WORK}/package.json")" == "a.d.ts" ]] || fail "wasm-pack fields must survive"

for bad in v0.1.0 latest v0.1.0-build. 0.1.0-build.5; do
  if bash "${SCRIPT}" "${bad}" "${WORK}" >/dev/null 2>&1; then fail "accepted tag '${bad}'"; fi
done
if bash "${SCRIPT}" v0.1.0-build.1 "${WORK}/missing" >/dev/null 2>&1; then fail "accepted missing dir"; fi
echo "prepare-npm-package.test.sh: ok"
