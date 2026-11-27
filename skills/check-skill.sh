#!/usr/bin/env bash
# Rules 4 and 5 of MAINTAINING.md: the size cap, and every tool and command the skill names exists.
# Usage: check-skill.sh <SKILL.md> <srs binary> <crates/srs-mcp-core/src/tools.rs>
# In CI the binary is the one built from the same commit as the skill.
# ponytail: one-word tools (find, read, similar, neighbours) and flags are not checked; add when one is renamed.
set -u
f=$1 srs=$2 tools=$3 bad=0

[ "$(wc -c <"$f")" -lt 12000 ] || { echo "over the 12,000 byte cap: $(wc -c <"$f")"; bad=1; }

known=$(grep -oE 'TOOL_[A-Z_]+: &str = "[a-z_]+"' "$tools" | grep -oE '"[a-z_]+"' | tr -d '"')
for t in $(grep -oE '`[a-z]+(_[a-z]+)+`' "$f" | tr -d '`' | sort -u); do
  echo "$known" | grep -qx "$t" || { echo "no MCP tool: $t"; bad=1; }
done

grep -oE '`srs [a-z][a-z-]*( [a-z][a-z-]*)*' "$f" | cut -c6- | sort -u | while read -r c; do
  # shellcheck disable=SC2086
  "$srs" $c --help >/dev/null 2>&1 || echo "no command: srs $c"
done | grep . && bad=1

exit $bad
