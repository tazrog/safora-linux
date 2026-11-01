#!/bin/bash
# Runs inside the sandbox: start nested saforawm, the bar and terminals, then screenshot.
cd /src/target/debug
export PATH=$PWD:$PATH
export XDG_CONFIG_HOME=/tmp/cfg; mkdir -p $XDG_CONFIG_HOME/safora
cat > $XDG_CONFIG_HOME/safora/config.toml <<EOC
[desktop]
wallpaper = "/distro/profile/airootfs/usr/share/backgrounds/safora/safora-default.png"
[windows]
corner_radius = ${RADIUS:-12}
opacity_inactive = 0.9
EOC
export LIBGL_ALWAYS_SOFTWARE=1 RUST_LOG=warn,saforawm=info
saforawm --winit >/src/wm-nested.log 2>&1 &
WM=$!
sleep 4
saforactl spawn "foot -T one"; sleep 1.5
saforactl spawn "foot -T two"; sleep 1.5
saforactl spawn "foot -T three"; sleep 1.5
saforactl spawn "foot -T four"; sleep 2
saforactl screenshot /src/shot-smart.png; sleep 1
saforactl quit; wait $WM
pkill -f foot; true
