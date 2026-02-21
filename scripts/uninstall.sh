#!/bin/bash
# Uninstall Legion binaries from /usr/local/bin

set -e

BINARIES="legion legion-dispatch legion-check legion-report legion-status legion-stop"
INSTALL_DIR="/usr/local/bin"

echo "Removing Legion binaries from $INSTALL_DIR..."
for bin in $BINARIES; do
    if [ -f "$INSTALL_DIR/$bin" ]; then
        rm -f "$INSTALL_DIR/$bin"
        echo "  Removed $bin"
    fi
done
echo "Done."
