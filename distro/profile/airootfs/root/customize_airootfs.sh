#!/usr/bin/env bash
# Runs inside the image after packages are installed (the file is removed afterwards).
set -e
cat > /usr/lib/os-release <<'OSR'
NAME="Steady Linux"
PRETTY_NAME="Steady Linux"
ID=steady
ID_LIKE=arch
BUILD_ID=rolling
HOME_URL="https://github.com/tazrog/steady-linux"
TAGLINE="Less Complexity, More Confidence"
ANSI_COLOR="38;2;242;184;75"
OSR
