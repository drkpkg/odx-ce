#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
dist_dir="$repo_root/dist"

# Clean previous build outputs before building
rm -f "$dist_dir"/odx-*-windows-x86_64-installer.exe
mkdir -p "$dist_dir"

version="$("$repo_root/scripts/release/version.sh")"

image="odx-packaging-windows:local"
docker build -t "$image" -f "$repo_root/packaging/windows/Dockerfile" "$repo_root/packaging/windows"

docker run --rm \
  -v "$repo_root:/work" \
  -w /work \
  "$image" \
  bash -c "set -euo pipefail; cargo build --release --target x86_64-pc-windows-gnu; cp -v target/x86_64-pc-windows-gnu/release/odx.exe /tmp/odx.exe; cp -v packaging/windows/odx.nsi /tmp/odx.nsi; cd /tmp; makensis -V2 odx.nsi; cp -v odx-installer.exe /work/dist/odx-${version}-windows-x86_64-installer.exe"

echo "Windows installer written to: $dist_dir/odx-${version}-windows-x86_64-installer.exe"
