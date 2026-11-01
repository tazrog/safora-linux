#!/bin/bash
cd /src/target/debug
export PATH=$PWD:$PATH XDG_CONFIG_HOME=/tmp/cfg-g LIBGL_ALWAYS_SOFTWARE=1 RUST_LOG=warn
mkdir -p $XDG_CONFIG_HOME/sekuro
printf '[desktop]\nwallpaper = "/distro/profile/airootfs/usr/share/backgrounds/sekuro/sekuro-default.png"\n' > $XDG_CONFIG_HOME/sekuro/config.toml
sekurowm --winit >/src/wm-g.log 2>&1 &
WM=$!
sleep 14
sekuroctl screenshot /src/glitch.png
sleep 1; sekuroctl quit; wait $WM; pkill -x sekuro-bar; true
