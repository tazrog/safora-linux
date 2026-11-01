#!/bin/bash
cd /src/target/debug
export PATH=$PWD:$PATH XDG_CONFIG_HOME=/tmp/cfg-fs LIBGL_ALWAYS_SOFTWARE=1 RUST_LOG=warn
mkdir -p $XDG_CONFIG_HOME/sekuro
printf '[desktop]\nwallpaper = "/distro/profile/airootfs/usr/share/backgrounds/sekuro/sekuro-default.png"\n' > $XDG_CONFIG_HOME/sekuro/config.toml
sekurowm --winit >/src/wm-fs.log 2>&1 &
WM=$!
sleep 4
sekuroctl spawn "foot -T one"; sleep 2
sekuroctl fullscreen; sleep 3
sekuroctl screenshot /src/fs1.png; sleep 1
sekuroctl quit; wait $WM; pkill -x foot; pkill -x sekuro-bar; true
