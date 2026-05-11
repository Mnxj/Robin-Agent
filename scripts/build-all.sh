#!/usr/bin/env bash
set -euo pipefail

# Build and package release binaries for multiple targets.
# Artifacts are written to ./dist

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST_DIR="${DIST_DIR:-${ROOT_DIR}/dist}"
BUILD_TOOL="${BUILD_TOOL:-cargo}" # cargo | cross
PROFILE="${PROFILE:---release}"

TARGETS=(
  "x86_64-unknown-linux-gnu"
  "aarch64-unknown-linux-gnu"
  "x86_64-apple-darwin"
  "aarch64-apple-darwin"
  "x86_64-pc-windows-msvc"
)

mkdir -p "$DIST_DIR"

if [[ "$BUILD_TOOL" == "cross" ]] && ! command -v cross >/dev/null 2>&1; then
  echo "cross not found, fallback to cargo"
  BUILD_TOOL="cargo"
fi

build_target() {
  local target="$1"
  echo "==> building $target with $BUILD_TOOL"
  "$BUILD_TOOL" build $PROFILE -p robin --target "$target"

  local bin_dir="${ROOT_DIR}/target/${target}/release"
  local robin_name="robin"
  [[ "$target" == *"windows"* ]] && robin_name="robin.exe"

  if [[ ! -f "${bin_dir}/${robin_name}" ]]; then
    echo "missing binary: ${bin_dir}/${robin_name}" >&2
    exit 1
  fi

  local stage_dir="${DIST_DIR}/stage-${target}"
  rm -rf "$stage_dir"
  mkdir -p "$stage_dir"
  cp "${bin_dir}/${robin_name}" "${stage_dir}/${robin_name}"

  local archive_base="${DIST_DIR}/robin-${target}"
  if [[ "$target" == *"windows"* ]]; then
    (cd "$stage_dir" && zip -q -r "${archive_base}.zip" .)
  else
    tar -czf "${archive_base}.tar.gz" -C "$stage_dir" .
  fi
  rm -rf "$stage_dir"
}

for target in "${TARGETS[@]}"; do
  build_target "$target"
done

echo "==> done, artifacts in ${DIST_DIR}"
