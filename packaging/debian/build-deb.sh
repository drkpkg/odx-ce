#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
dist_dir="$repo_root/dist"

# Clean previous build outputs before building
rm -f "$dist_dir"/*.deb
mkdir -p "$dist_dir"

image="odx-packaging-deb:local"

docker build -t "$image" -f "$repo_root/packaging/debian/Dockerfile" "$repo_root/packaging/debian"

docker run --rm \
  -v "$repo_root:/work" \
  -w /work \
  "$image" \
  bash -c "export PATH=/usr/local/cargo/bin:\$PATH && rm -rf target/debian && cargo build --release && cargo deb --no-build && cp -v target/debian/odx_*.deb /work/dist/"

echo "Debian package(s) written to: $dist_dir"
