#!/bin/bash
# Legion Build Script - builds for your local chip architecture
# Usage: ./build-mac.sh

set -e

# Detect architecture
ARCH=$(uname -m)
case "$ARCH" in
    arm64|aarch64)
        TARGET="aarch64-apple-darwin"
        ARCH_NAME="arm64"
        ;;
    x86_64)
        TARGET="x86_64-apple-darwin"
        ARCH_NAME="x86_64"
        ;;
    *)
        echo "Unknown architecture: $ARCH"
        exit 1
        ;;
esac

# Get version from Cargo.toml
VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')

echo "==> Building Legion $VERSION for $ARCH_NAME ($TARGET)..."

# Build for target architecture
cargo build --release --target "$TARGET"

# Package
PKG_ID="com.legion.cli"
INSTALL_DIR="/usr/local/bin"
PKG_NAME="legion-${VERSION}-${ARCH_NAME}.pkg"

echo "==> Packaging $PKG_NAME..."
rm -rf target/pkg-root
mkdir -p target/pkg-root${INSTALL_DIR}

for bin in legion legion-dispatch legion-check legion-report legion-status legion-stop legion-deps; do
    cp target/${TARGET}/release/$bin target/pkg-root${INSTALL_DIR}/
done

pkgbuild \
    --root target/pkg-root \
    --identifier "$PKG_ID" \
    --version "$VERSION" \
    --install-location / \
    "$PKG_NAME"

rm -rf target/pkg-root

echo "==> Created $PKG_NAME"
ls -lh "$PKG_NAME"
