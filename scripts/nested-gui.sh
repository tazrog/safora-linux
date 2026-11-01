#!/bin/bash
# usage: nested-gui.sh <app> <shot.png> [settings-page]
cd /src/target/debug
export PATH=$PWD:$PATH XDG_CONFIG_HOME=/tmp/cfg-$$ LIBGL_ALWAYS_SOFTWARE=1 RUST_LOG=warn
mkdir -p $XDG_CONFIG_HOME/steady
printf '[desktop]\nwallpaper = "/distro/profile/airootfs/usr/share/backgrounds/steady/steady-default.png"\n' > $XDG_CONFIG_HOME/steady/config.toml
export STEADY_SETTINGS_PAGE=$3
steadywm --winit --exec "$1" >/src/wm-gui.log 2>&1 &
WM=$!
sleep 5
steadyctl screenshot "$2"
sleep 1
steadyctl quit
wait $WM; pkill -x steady-settings; pkill -x steady-bar; true
