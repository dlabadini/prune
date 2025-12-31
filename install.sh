#!/bin/bash
# Prune installer script
# Usage: curl -fsSL https://prunecode.ai/install.sh | bash

set -e

PRUNE_VERSION="${PRUNE_VERSION:-latest}"
PRUNE_INSTALL_DIR="${PRUNE_INSTALL_DIR:-$HOME/.prune/bin}"

# Detect platform
OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
    Linux)
        PLATFORM="linux"
        ;;
    Darwin)
        PLATFORM="darwin"
        ;;
    *)
        echo "Unsupported OS: $OS"
        exit 1
        ;;
esac

case "$ARCH" in
    x86_64)
        ARCH="x86_64"
        ;;
    aarch64|arm64)
        ARCH="aarch64"
        ;;
    *)
        echo "Unsupported architecture: $ARCH"
        exit 1
        ;;
esac

BINARY_NAME="prune-${PLATFORM}-${ARCH}"

echo "Installing Prune for ${PLATFORM}-${ARCH}..."

# Create install directory
mkdir -p "$PRUNE_INSTALL_DIR"

# Download binary
if [ "$PRUNE_VERSION" = "latest" ]; then
    DOWNLOAD_URL="https://github.com/dlabadini/prune/releases/latest/download/${BINARY_NAME}.tar.gz"
else
    DOWNLOAD_URL="https://github.com/dlabadini/prune/releases/download/v${PRUNE_VERSION}/${BINARY_NAME}.tar.gz"
fi

echo "Downloading from $DOWNLOAD_URL..."
curl -fsSL "$DOWNLOAD_URL" | tar -xz -C "$PRUNE_INSTALL_DIR"

# Make executable
chmod +x "$PRUNE_INSTALL_DIR/prune"

echo ""
echo "Prune installed successfully to $PRUNE_INSTALL_DIR/prune"
echo ""
echo "Add to your PATH by adding this to your shell config:"
echo ""
echo "  export PATH=\"\$PATH:$PRUNE_INSTALL_DIR\""
echo ""
echo "Then run 'prune' to get started!"
echo ""
echo "Start your 14-day free trial at https://prunecode.ai/trial"
