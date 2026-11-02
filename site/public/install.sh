#!/bin/sh
# Install the srs engine (the `srs` CLI and MCP server) from the latest release.
#
#   curl -fsSL https://engine.semanticops.com/install.sh | sh
#
# Installs to $SRS_INSTALL_DIR (default ~/.local/bin). Linux and macOS, x86_64 and arm64.
set -eu

repo="https://github.com/the-greenman/srs-rust"
dir="${SRS_INSTALL_DIR:-$HOME/.local/bin}"
from_source="cargo install --git $repo srs"

fail() {
  echo "srs install: $1" >&2
  echo "Build it from source instead (needs Rust): $from_source" >&2
  exit 1
}

case "$(uname -s)/$(uname -m)" in
  Linux/x86_64 | Linux/amd64) target=x86_64-unknown-linux-gnu ;;
  Linux/aarch64 | Linux/arm64) target=aarch64-unknown-linux-gnu ;;
  Darwin/arm64 | Darwin/aarch64) target=aarch64-apple-darwin ;;
  Darwin/x86_64) target=x86_64-apple-darwin ;;
  *) fail "no prebuilt srs for $(uname -s) $(uname -m)" ;;
esac

if command -v sha256sum >/dev/null 2>&1; then
  sha() { sha256sum "$1" | cut -d' ' -f1; }
elif command -v shasum >/dev/null 2>&1; then
  sha() { shasum -a 256 "$1" | cut -d' ' -f1; }
else
  fail "need sha256sum or shasum to verify the download"
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
archive="srs-$target.tar.gz"
url="$repo/releases/latest/download/$archive"

echo "Downloading $url"
curl -fsSL "$url" -o "$tmp/$archive" || fail "no $archive in the latest release yet"
curl -fsSL "$url.sha256" -o "$tmp/$archive.sha256" || fail "no $archive.sha256 in the latest release"
[ "$(sha "$tmp/$archive")" = "$(cut -d' ' -f1 "$tmp/$archive.sha256")" ] || fail "checksum mismatch for $archive"

tar -xzf "$tmp/$archive" -C "$tmp" srs
mkdir -p "$dir"
mv "$tmp/srs" "$dir/srs"
chmod +x "$dir/srs"
echo "Installed $("$dir/srs" --version) to $dir/srs"

case ":$PATH:" in
  *":$dir:"*) ;;
  *) echo "Note: $dir is not on your PATH. Add it, e.g. export PATH=\"$dir:\$PATH\"" ;;
esac
echo "Next: https://engine.semanticops.com/start.md"
