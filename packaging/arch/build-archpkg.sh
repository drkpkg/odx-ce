#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
dist_dir="$repo_root/dist"
work_dir="$repo_root/packaging/arch/.work"

# Clean build dirs before building
rm -rf "$work_dir"
rm -f "$dist_dir"/*.pkg.tar.zst
mkdir -p "$dist_dir" "$work_dir"

version="$("$repo_root/scripts/release/version.sh")"

# Create source tarball that PKGBUILD expects
tarball="$work_dir/odx-${version}.tar.gz"
# Use working tree (includes uncommitted files like LICENSE during development)
tar -czf "$tarball" \
  --exclude=".git" \
  --exclude="target" \
  --exclude=".testing" \
  --exclude="dist" \
  --exclude="packaging/arch/.work" \
  --transform="s,^,odx-${version}/," \
  -C "$repo_root" .

# Prepare PKGBUILD with correct version
cp "$repo_root/packaging/arch/PKGBUILD" "$work_dir/PKGBUILD"
sed -i "s/^pkgver=.*/pkgver=${version}/" "$work_dir/PKGBUILD"

image="odx-packaging-arch:local"
docker build -t "$image" -f "$repo_root/packaging/arch/Dockerfile" "$repo_root/packaging/arch"

docker run --rm \
  -v "$work_dir:/work" \
  -w /work \
  "$image" \
  bash -c "chown -R builder:builder /work && sudo -u builder bash -lc 'cd /work && makepkg -sf --noconfirm'"

# Copy package(s) to dist
cp -v "$work_dir"/*.pkg.tar.zst "$dist_dir/"

echo "Arch package(s) written to: $dist_dir"
