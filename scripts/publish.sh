#!/usr/bin/env bash
set -euo pipefail

# Publish artifacts in ./dist to GitHub Release.
# Requirements:
#   - gh CLI logged in
#   - tag already exists (e.g. v0.1.0)
#
# Usage:
#   TAG=v0.1.0 ./scripts/publish.sh

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST_DIR="${DIST_DIR:-${ROOT_DIR}/dist}"
REPO="${REPO:-sausheong/robin}"
TAG="${TAG:-$(git describe --tags --exact-match 2>/dev/null || true)}"

if [[ -z "$TAG" ]]; then
  echo "TAG is required (or current commit must be exactly on a tag)." >&2
  exit 1
fi

if ! command -v gh >/dev/null 2>&1; then
  echo "gh CLI is required: https://cli.github.com/" >&2
  exit 1
fi

shopt -s nullglob
ASSETS=("${DIST_DIR}"/*)
if [[ ${#ASSETS[@]} -eq 0 ]]; then
  echo "no artifacts found in ${DIST_DIR}" >&2
  exit 1
fi

if ! gh release view "$TAG" --repo "$REPO" >/dev/null 2>&1; then
  gh release create "$TAG" --repo "$REPO" --title "$TAG" --notes "Automated release for ${TAG}"
fi

gh release upload "$TAG" "${ASSETS[@]}" --repo "$REPO" --clobber
echo "published ${#ASSETS[@]} assets to ${REPO}@${TAG}"
