#!/usr/bin/env bash
set -euo pipefail

# One-click installer for robin CLI from GitHub Releases.
# Usage:

default_repo() {
  if command -v git >/dev/null 2>&1; then
    if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
      local url
      url="$(git remote get-url origin 2>/dev/null || true)"
      if [[ -n "$url" ]]; then
        # Supports:
        #   https://github.com/<owner>/<repo>.git
        #   git@github.com:<owner>/<repo>.git
        if [[ "$url" =~ github\.com[:/]+([^/]+)/([^/]+?)(\.git)?$ ]]; then
          echo "${BASH_REMATCH[1]}/${BASH_REMATCH[2]}"
          return 0
        fi
      fi
    fi
  fi
  echo "Mnxj/Robin-Agent"
}

REPO="${REPO:-$(default_repo)}"
VERSION="${VERSION:-latest}"
INSTALL_DIR="${INSTALL_DIR:-/usr/local/bin}"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

detect_target() {
  local os arch
  os="$(uname -s | tr '[:upper:]' '[:lower:]')"
  arch="$(uname -m)"

  case "$os" in
    linux) os="unknown-linux-gnu" ;;
    darwin) os="apple-darwin" ;;
    *) echo "unsupported OS: $os" >&2; exit 1 ;;
  esac

  case "$arch" in
    x86_64|amd64) arch="x86_64" ;;
    arm64|aarch64) arch="aarch64" ;;
    *) echo "unsupported arch: $arch" >&2; exit 1 ;;
  esac

  echo "${arch}-${os}"
}

TARGET="$(detect_target)"
ASSET="robin-${TARGET}.tar.gz"

if [[ "$VERSION" == "latest" ]]; then
  URL="https://github.com/${REPO}/releases/latest/download/${ASSET}"
else
  URL="https://github.com/${REPO}/releases/download/${VERSION}/${ASSET}"
fi

echo "==> downloading ${URL}"
if ! curl -fL "$URL" -o "${TMP_DIR}/${ASSET}"; then
  echo "" >&2
  echo "download failed (repo=${REPO} version=${VERSION} asset=${ASSET})" >&2
  echo "" >&2
  echo "Possible fixes:" >&2
  echo "  - If this is your fork: create a tag like v0.1.0 and push it so the release workflow uploads dist/robin-<target>.tar.gz." >&2
  echo "  - Or run: REPO=<owner>/<repo> VERSION=<tag> bash install.sh" >&2
  echo "  - Or build from source: cargo build --release -p robin && install -m 0755 target/release/robin ${INSTALL_DIR}/robin" >&2
  exit 1
fi
tar -xzf "${TMP_DIR}/${ASSET}" -C "${TMP_DIR}"

if [[ ! -f "${TMP_DIR}/robin" ]]; then
  echo "release archive does not contain robin binary" >&2
  exit 1
fi

echo "==> installing robin to ${INSTALL_DIR}"
if [[ ! -w "$INSTALL_DIR" ]]; then
  sudo install -m 0755 "${TMP_DIR}/robin" "${INSTALL_DIR}/robin"
else
  install -m 0755 "${TMP_DIR}/robin" "${INSTALL_DIR}/robin"
fi

echo "==> installed: $(${INSTALL_DIR}/robin version || true)"
