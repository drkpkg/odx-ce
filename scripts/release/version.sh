#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

version_line="$(grep -E '^version[[:space:]]*=[[:space:]]*"' "$repo_root/Cargo.toml" | head -n 1)"
version="$(echo "$version_line" | sed -E 's/^version[[:space:]]*=[[:space:]]*"([^"]+)".*$/\1/')"

if [ -z "${version}" ]; then
  echo "Failed to read version from Cargo.toml" >&2
  exit 1
fi

echo "$version"
