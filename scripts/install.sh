#!/usr/bin/env sh
set -eu

VERSION="${1:-v0.1.0}"
INSTALL_DIR="${2:-/usr/local/bin}"
OWNER="${SPATIAD_GITHUB_OWNER:-zubeyralmaho}"
REPO="${SPATIAD_GITHUB_REPO:-spatiad}"

OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH_RAW="$(uname -m)"

case "$ARCH_RAW" in
  x86_64|amd64) ARCH="x86_64" ;;
  arm64|aarch64) ARCH="aarch64" ;;
  *)
    echo "Unsupported architecture: $ARCH_RAW" >&2
    exit 1
    ;;
esac

case "$OS" in
  linux|darwin) ;;
  *)
    echo "Unsupported OS: $OS" >&2
    exit 1
    ;;
esac

ASSET="spatiad-${VERSION}-${OS}-${ARCH}.tar.gz"
URL="https://github.com/${OWNER}/${REPO}/releases/download/${VERSION}/${ASSET}"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

echo "Downloading ${URL}"

if command -v curl >/dev/null 2>&1; then
  curl -fsSL "$URL" -o "$TMP_DIR/spatiad.tar.gz"
elif command -v wget >/dev/null 2>&1; then
  wget -qO "$TMP_DIR/spatiad.tar.gz" "$URL"
else
  echo "Neither curl nor wget is available" >&2
  exit 1
fi

tar -xzf "$TMP_DIR/spatiad.tar.gz" -C "$TMP_DIR"

if [ ! -f "$TMP_DIR/spatiad-bin" ]; then
  echo "Archive does not contain spatiad-bin" >&2
  exit 1
fi

mkdir -p "$INSTALL_DIR"
install -m 0755 "$TMP_DIR/spatiad-bin" "$INSTALL_DIR/spatiad-bin"

echo "Installed spatiad-bin to ${INSTALL_DIR}/spatiad-bin"
echo "Run: ${INSTALL_DIR}/spatiad-bin"
