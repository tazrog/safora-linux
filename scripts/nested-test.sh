#!/bin/bash
# Runs inside the sandbox: start nested sekurowm, the bar and terminals, then screenshot.
cd /src/target/debug
export PATH=$PWD:$PATH
export XDG_CONFIG_HOME=/tmp/cfg; mkdir -p $XDG_CONFIG_HOME/sekuro
cat > $XDG_CONFIG_HOME/sekuro/config.toml <<EOC
[desktop]
wallpaper = "/distro/profile/airootfs/usr/share/backgrounds/sekuro/sekuro-default.png"
[windows]
corner_radius = ${RADIUS:-12}
opacity_inactive = 0.9
EOC
export LIBGL_ALWAYS_SOFTWARE=1 RUST_LOG=warn,sekurowm=info
sekurowm --winit >/src/wm-nested.log 2>&1 &
WM=$!
sleep 4
sekuroctl spawn "foot -T one"; sleep 1.5
sekuroctl spawn "foot -T two"; sleep 1.5
sekuroctl spawn "foot -T three"; sleep 1.5
sekuroctl spawn "foot -T four"; sleep 2
sekuroctl screenshot /src/shot-smart.png; sleep 1
sekuroctl quit; wait $WM
pkill -f foot; true
