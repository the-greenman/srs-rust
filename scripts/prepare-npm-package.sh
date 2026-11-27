#!/usr/bin/env bash
# prepare-npm-package.sh <release-tag> <package-dir>
#
# Rewrites the package.json that `wasm-pack build --target web` generated in
# <package-dir> into the public npm package `@semanticops/srs-bindings`
# (srs-rust#819, srs-programme decision 012d961f: bindings are published to npm
# from promoted releases only).
#
# The version comes from the release tag: `v<major>.<minor>.<patch>-build.<N>`
# becomes `0.<N>.0` (pre-1.0, breaking-in-minor, build number is the minor).
# Prints the resulting version on stdout. Exits non-zero on a malformed tag or a
# package directory with no package.json.

set -euo pipefail

tag="${1:?usage: prepare-npm-package.sh <release-tag> <package-dir>}"
dir="${2:?usage: prepare-npm-package.sh <release-tag> <package-dir>}"

if [[ ! "${tag}" =~ ^v[0-9]+\.[0-9]+\.[0-9]+-build\.([0-9]+)$ ]]; then
  echo "prepare-npm-package: tag '${tag}' is not v<semver>-build.<N>" >&2
  exit 1
fi
build="${BASH_REMATCH[1]}"
version="0.${build}.0"

pkg="${dir}/package.json"
[[ -f "${pkg}" ]] || { echo "prepare-npm-package: ${pkg} not found" >&2; exit 1; }

tmp="$(mktemp)"
jq --arg v "${version}" '
  .name = "@semanticops/srs-bindings"
  | .version = $v
  | .description = "SRS engine bindings (WebAssembly): read and write SRS repositories from JavaScript"
  | .license = "MIT OR Apache-2.0"
  | .repository = {type: "git", url: "git+https://github.com/the-greenman/srs-rust.git"}
  | .publishConfig = {access: "public"}
  | .sideEffects = false
' "${pkg}" > "${tmp}"
mv "${tmp}" "${pkg}"
echo "${version}"
