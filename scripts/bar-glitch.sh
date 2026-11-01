#!/bin/bash
cd /src/target/debug
export PATH=$PWD:$PATH XDG_CONFIG_HOME=/tmp/cfg-g LIBGL_ALWAYS_SOFTWARE=1 RUST_LOG=warn
mkdir -p $XDG_CONFIG_HOME/steady
printf '[desktop]\nwallpaper = "/distro/profile/airootfs/usr/share/backgrounds/steady/steady-default.png"\n' > $XDG_CONFIG_HOME/steady/config.toml
steadywm --winit >/src/wm-g.log 2>&1 &
WM=$!
sleep 14
steadyctl screenshot /src/glitch.png
sleep 1; steadyctl quit; wait $WM; pkill -x steady-bar; true
