#!/bin/bash
# Enter the unprivileged Arch build sandbox (bwrap, single-uid user namespace).
# Usage: sandbox.sh [command...]   (default: interactive bash)
ROOT=${SAFORA_SANDBOX:-$HOME/.local/share/safora/archroot}   # kept outside the project folder (it is full of symlinks)
SRC=/home/tazrog/NewLinux/src
ARGS=(--unshare-user --uid 0 --gid 0 --bind "$ROOT" / --dev-bind /dev /dev --proc /proc --ro-bind /sys /sys
      --tmpfs /tmp --tmpfs /run --dir /run/user/0 --bind "$SRC" /src --ro-bind /home/tazrog/NewLinux/scripts /scripts --bind /home/tazrog/NewLinux/distro /distro --setenv HOME /root --setenv LANG C.UTF-8 --unshare-uts --hostname saforabuild)
if [ -n "$XDG_RUNTIME_DIR" ] && [ -n "$WAYLAND_DISPLAY" ] && [ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ]; then
  ARGS+=(--bind "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" /run/user/0/$WAYLAND_DISPLAY --setenv XDG_RUNTIME_DIR /run/user/0 --setenv WAYLAND_DISPLAY "$WAYLAND_DISPLAY")
fi
[ $# -eq 0 ] && set -- bash
exec bwrap "${ARGS[@]}" "$@"
