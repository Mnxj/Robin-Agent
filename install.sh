#!/usr/bin/env bash
set -euo pipefail

# One-click installer for robin CLI from GitHub Releases.
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/<owner>/<repo>/main/install.sh | bash
#   REPO=sausheong/robin VERSION=v0.1.0 bash install.sh

REPO="${REPO:-sausheong/robin}"
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
curl -fL "$URL" -o "${TMP_DIR}/${ASSET}"
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
