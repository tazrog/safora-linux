#!/usr/bin/env bash
# Runs inside the image after packages are installed (the file is removed afterwards).
set -e
cat > /usr/lib/os-release <<'OSR'
NAME="Sekuro Linux"
PRETTY_NAME="Sekuro Linux"
ID=sekuro
ID_LIKE=arch
BUILD_ID=rolling
ANSI_COLOR="38;2;79;209;165"
OSR
