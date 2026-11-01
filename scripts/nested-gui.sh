#!/bin/bash
# usage: nested-gui.sh <app> <shot.png> [settings-page]
cd /src/target/debug
export PATH=$PWD:$PATH XDG_CONFIG_HOME=/tmp/cfg-$$ LIBGL_ALWAYS_SOFTWARE=1 RUST_LOG=warn
mkdir -p $XDG_CONFIG_HOME/safora
printf '[desktop]\nwallpaper = "/distro/profile/airootfs/usr/share/backgrounds/safora/safora-default.png"\n' > $XDG_CONFIG_HOME/safora/config.toml
export SAFORA_SETTINGS_PAGE=$3
saforawm --winit --exec "$1" >/src/wm-gui.log 2>&1 &
WM=$!
sleep 5
saforactl screenshot "$2"
sleep 1
saforactl quit
wait $WM; pkill -x safora-settings; pkill -x safora-bar; true
