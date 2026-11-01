#!/bin/bash
cd /src/target/debug
export PATH=$PWD:$PATH XDG_CONFIG_HOME=/tmp/cfg-g LIBGL_ALWAYS_SOFTWARE=1 RUST_LOG=warn
mkdir -p $XDG_CONFIG_HOME/safora
printf '[desktop]\nwallpaper = "/distro/profile/airootfs/usr/share/backgrounds/safora/safora-default.png"\n' > $XDG_CONFIG_HOME/safora/config.toml
saforawm --winit >/src/wm-g.log 2>&1 &
WM=$!
sleep 14
saforactl screenshot /src/glitch.png
sleep 1; saforactl quit; wait $WM; pkill -x safora-bar; true
