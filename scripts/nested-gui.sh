#!/bin/bash
# usage: nested-gui.sh <app> <shot.png> [settings-page]
cd /src/target/debug
export PATH=$PWD:$PATH XDG_CONFIG_HOME=/tmp/cfg-$$ LIBGL_ALWAYS_SOFTWARE=1 RUST_LOG=warn
mkdir -p $XDG_CONFIG_HOME/sekuro
printf '[desktop]\nwallpaper = "/distro/profile/airootfs/usr/share/backgrounds/sekuro/sekuro-default.png"\n' > $XDG_CONFIG_HOME/sekuro/config.toml
export SEKURO_SETTINGS_PAGE=$3
sekurowm --winit --exec "$1" >/src/wm-gui.log 2>&1 &
WM=$!
sleep 5
sekuroctl screenshot "$2"
sleep 1
sekuroctl quit
wait $WM; pkill -x sekuro-settings; pkill -x sekuro-bar; true
