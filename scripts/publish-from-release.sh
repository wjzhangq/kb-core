#!/usr/bin/env bash
#
# Publish pre-packed .tgz tarballs (built by the Release workflow) to npm.
#
# Usage:
#   scripts/publish-from-release.sh <dir> [--dry-run]
#
# <dir> is a directory containing the .tgz files, e.g. produced by:
#   gh release download v0.0.1 --pattern '*.tgz' --dir dist
#
# Behavior:
#   - Native sub-packages (filenames containing "native") are published FIRST,
#     because the root package's optionalDependencies must already resolve.
#   - The root package is published LAST.
#   - Versions that already exist on npm are skipped (not treated as failure),
#     so the script is safe to re-run after a partial failure.
#
# Auth: relies on npm being already authenticated (e.g. ~/.npmrc with an
# _authToken, or `npm login`). This script does not configure credentials.

set -euo pipefail

DIR="${1:-}"
DRY_RUN=""

for arg in "$@"; do
  case "$arg" in
    --dry-run) DRY_RUN="--dry-run" ;;
  esac
done

if [[ -z "$DIR" || ! -d "$DIR" ]]; then
  echo "error: pass a directory containing .tgz files" >&2
  echo "usage: $0 <dir> [--dry-run]" >&2
  exit 1
fi

shopt -s nullglob

# Split tarballs: native sub-packages first, root package last.
native_pkgs=()
root_pkgs=()
for f in "$DIR"/*.tgz; do
  case "$(basename "$f")" in
    *native*) native_pkgs+=("$f") ;;
    *)        root_pkgs+=("$f") ;;
  esac
done

if [[ ${#native_pkgs[@]} -eq 0 && ${#root_pkgs[@]} -eq 0 ]]; then
  echo "error: no .tgz files found in $DIR" >&2
  exit 1
fi

# publish_one <tarball>: publish, skipping if the version already exists.
publish_one() {
  local tgz="$1"
  echo "==> publishing $(basename "$tgz")"

  local out
  if out=$(npm publish "$tgz" --access public $DRY_RUN 2>&1); then
    echo "$out"
    echo "    ok"
    return 0
  fi

  echo "$out"
  # Already-published version → skip, don't fail the whole run.
  if echo "$out" | grep -qiE "cannot publish over|previously published|EPUBLISHCONFLICT|403 Forbidden"; then
    echo "    skip: version already published"
    return 0
  fi

  echo "    FAILED: $(basename "$tgz")" >&2
  return 1
}

failed=0

# Native sub-packages first so root optionalDependencies resolve.
for f in "${native_pkgs[@]}"; do
  publish_one "$f" || failed=1
done

# Root package last.
for f in "${root_pkgs[@]}"; do
  publish_one "$f" || failed=1
done

if [[ "$failed" -ne 0 ]]; then
  echo "one or more packages failed to publish (see FAILED lines above)" >&2
  exit 1
fi

echo "all packages published (or already present)"
