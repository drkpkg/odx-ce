#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"

usage() {
  cat <<'EOF'
Usage: bump-and-release.sh [options] <patch|minor|major|X.Y.Z>

Bump odoo-cli version in Cargo.toml, commit, create annotated tag vX.Y.Z.
Use --push to publish branch and tag (triggers GitHub Actions release on v*).

Options:
  --dry-run   Print planned version and commands; do not modify files or git.
  --push      After commit and tag, run git push origin <branch> and git push origin vX.Y.Z
  --no-verify Pass --no-verify to git commit (skip hooks)
  -h, --help  Show this help

Examples:
  ./scripts/release/bump-and-release.sh patch
  ./scripts/release/bump-and-release.sh 0.3.0 --dry-run
  ./scripts/release/bump-and-release.sh minor --push
EOF
}

dry_run=0
do_push=0
no_verify=()
bump_arg=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run) dry_run=1; shift ;;
    --push) do_push=1; shift ;;
    --no-verify) no_verify=(--no-verify); shift ;;
    -h|--help) usage; exit 0 ;;
    -*)
      echo "Unknown option: $1" >&2
      usage >&2
      exit 1
      ;;
    *)
      if [[ -n "$bump_arg" ]]; then
        echo "Unexpected extra argument: $1" >&2
        exit 1
      fi
      bump_arg="$1"
      shift
      ;;
  esac
done

if [[ -z "$bump_arg" ]]; then
  echo "Missing patch|minor|major|X.Y.Z" >&2
  usage >&2
  exit 1
fi

if ! command -v git >/dev/null 2>&1; then
  echo "git is required" >&2
  exit 1
fi

if [[ ! -f "$repo_root/Cargo.toml" ]]; then
  echo "Cargo.toml not found at repo root" >&2
  exit 1
fi

current="$("$repo_root/scripts/release/version.sh")"
if [[ -z "$current" ]]; then
  echo "Could not read current version" >&2
  exit 1
fi

is_semver() {
  [[ "$1" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]]
}

compute_next() {
  local kind="$1"
  local cur="$2"
  local major minor patch
  IFS='.' read -r major minor patch <<<"$cur"
  if [[ -z "${major:-}" || -z "${minor:-}" || -z "${patch:-}" ]]; then
    echo "Invalid current version: $cur" >&2
    return 1
  fi
  case "$kind" in
    patch) patch=$((patch + 1)) ;;
    minor)
      minor=$((minor + 1))
      patch=0
      ;;
    major)
      major=$((major + 1))
      minor=0
      patch=0
      ;;
    *)
      echo "Internal error: bad kind $kind" >&2
      return 1
      ;;
  esac
  echo "${major}.${minor}.${patch}"
}

if [[ "$bump_arg" == patch || "$bump_arg" == minor || "$bump_arg" == major ]]; then
  new_version="$(compute_next "$bump_arg" "$current")" || exit 1
elif is_semver "$bump_arg"; then
  new_version="$bump_arg"
else
  echo "Invalid argument: expected patch, minor, major, or semver X.Y.Z (got: $bump_arg)" >&2
  exit 1
fi

if [[ "$new_version" == "$current" ]]; then
  echo "New version equals current ($current); nothing to do." >&2
  exit 1
fi

if [[ "$dry_run" -eq 1 ]]; then
  echo "Dry run: would bump $current -> $new_version"
  echo "Would run: cargo check"
  echo "Would commit: [FEAT] Release $new_version"
  echo "Would tag: v$new_version"
  if [[ "$do_push" -eq 1 ]]; then
    echo "Would push: git push origin <current-branch> && git push origin v$new_version"
  else
    echo "Then run: git push origin \$(git branch --show-current) && git push origin v$new_version"
  fi
  exit 0
fi

if [[ -z "$(git rev-parse --git-dir 2>/dev/null)" ]]; then
  echo "Not a git repository" >&2
  exit 1
fi

if [[ -n "$(git status --porcelain -uno 2>/dev/null)" ]]; then
  echo "Working tree has staged or unstaged changes to tracked files. Commit or stash first." >&2
  git status --short -uno >&2
  exit 1
fi

branch="$(git branch --show-current 2>/dev/null || true)"
if [[ -z "$branch" ]]; then
  echo "Detached HEAD: checkout a branch before releasing." >&2
  exit 1
fi

tmpfile="$(mktemp)"
trap 'rm -f "$tmpfile"' EXIT
if ! cargo check --quiet 2>"$tmpfile"; then
  cat "$tmpfile" >&2
  echo "cargo check failed; fix before releasing." >&2
  exit 1
fi

sed -i "s/^version = \".*\"/version = \"${new_version}\"/" "$repo_root/Cargo.toml"

if ! cargo check --quiet 2>"$tmpfile"; then
  sed -i "s/^version = \".*\"/version = \"${current}\"/" "$repo_root/Cargo.toml"
  git checkout -- "$repo_root/Cargo.lock" 2>/dev/null || true
  cat "$tmpfile" >&2
  echo "cargo check failed after version bump; reverted Cargo.toml (and Cargo.lock if it was modified)." >&2
  exit 1
fi

git add "$repo_root/Cargo.toml"
if [[ -f "$repo_root/Cargo.lock" ]]; then
  # -f: still stage if Cargo.lock was listed in .gitignore in older clones
  git add -f "$repo_root/Cargo.lock"
fi

if git diff --cached --quiet; then
  echo "No changes to commit (unexpected)." >&2
  exit 1
fi

git commit "${no_verify[@]}" -m "[FEAT] Release ${new_version}" -m "- Bump package version to ${new_version}"

git tag -a "v${new_version}" -m "Release v${new_version}"

echo "Created commit and tag v${new_version} on branch $branch."

if [[ "$do_push" -eq 1 ]]; then
  git push origin "$branch"
  git push origin "v${new_version}"
  echo "Pushed branch $branch and tag v${new_version}."
else
  echo "Push with:"
  echo "  git push origin $branch && git push origin v${new_version}"
fi
