#!/usr/bin/env bash
# Runs inside the image after packages are installed (the file is removed afterwards).
set -e
cat > /usr/lib/os-release <<'OSR'
NAME="Safora Linux"
PRETTY_NAME="Safora Linux"
ID=safora
ID_LIKE=arch
BUILD_ID=rolling
HOME_URL="https://github.com/tazrog/safora-linux"
TAGLINE="Simple by Nature, Private by default"
ANSI_COLOR="38;2;61;214;208"
OSR
