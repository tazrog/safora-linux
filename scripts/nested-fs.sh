#!/bin/bash
cd /src/target/debug
export PATH=$PWD:$PATH XDG_CONFIG_HOME=/tmp/cfg-fs LIBGL_ALWAYS_SOFTWARE=1 RUST_LOG=warn
mkdir -p $XDG_CONFIG_HOME/safora
printf '[desktop]\nwallpaper = "/distro/profile/airootfs/usr/share/backgrounds/safora/safora-default.png"\n' > $XDG_CONFIG_HOME/safora/config.toml
saforawm --winit >/src/wm-fs.log 2>&1 &
WM=$!
sleep 4
saforactl spawn "foot -T one"; sleep 2
saforactl fullscreen; sleep 3
saforactl screenshot /src/fs1.png; sleep 1
saforactl quit; wait $WM; pkill -x foot; pkill -x safora-bar; true
