#!/usr/bin/env bash
# Build the Safora Linux live/installer ISO.
#
#   sudo ./distro/build-iso.sh
#
# Needs root because mkarchiso installs packages into a chroot and mounts /proc, /sys and /dev.
# Everything happens inside a throw-away Arch Linux root unpacked from the official bootstrap
# tarball, so nothing on the host system is modified (apart from /var/tmp/safora-build, which is
# removed on success).
set -Eeuo pipefail

[[ $EUID -eq 0 ]] || { echo "Run this script with sudo." >&2; exit 1; }

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
TOP=$(dirname "$HERE")
BOOTSTRAP=${BOOTSTRAP:-$TOP/work/dl/bootstrap.tar.zst}
BIN=$TOP/src/target/release
WORK=${WORK:-/var/tmp/safora-build}
OUT=$TOP/out
OWNER=${SUDO_USER:-root}

for b in saforawm saforactl safora-bar safora-lock safora-settings safora-installer safora-viewer; do
    [[ -x $BIN/$b ]] || { echo "Missing $BIN/$b — build the release binaries first (see README.md)." >&2; exit 1; }
done
[[ -f $BOOTSTRAP ]] || { echo "Missing Arch bootstrap tarball: $BOOTSTRAP" >&2; exit 1; }

ROOT=$WORK/root
CACHE=${CACHE:-/var/tmp/safora-pkg-cache}   # package downloads survive between builds
# FAST=1 is for development iterations only: skips signature checks and compresses lightly.
FAST=${FAST:-0}
mounted=0
umount_all() {
    [[ $mounted == 1 ]] || return 0
    for m in var/cache/pacman/pkg dev/pts dev sys proc; do umount -lf "$ROOT/$m" 2>/dev/null || true; done
    mounted=0
}
trap umount_all EXIT

echo "==> Preparing build root in $ROOT"
umount -lf "$ROOT"/{var/cache/pacman/pkg,dev/pts,dev,sys,proc} 2>/dev/null || true
rm -rf "$WORK"
mkdir -p "$ROOT"
tar --zstd -xpf "$BOOTSTRAP" -C "$ROOT" --strip-components=1

echo 'Server = https://geo.mirror.pkgbuild.com/$repo/os/$arch' > "$ROOT/etc/pacman.d/mirrorlist"
sed -i 's/^#DisableSandbox/DisableSandbox/; s/^CheckSpace/#CheckSpace/; s/^#ParallelDownloads.*/ParallelDownloads = 5/' "$ROOT/etc/pacman.conf"
cp -L /etc/resolv.conf "$ROOT/etc/resolv.conf"

mount --bind /proc "$ROOT/proc"
mount --bind /sys "$ROOT/sys"
mount --bind /dev "$ROOT/dev"
mount --bind /dev/pts "$ROOT/dev/pts"
mkdir -p "$CACHE" "$ROOT/var/cache/pacman/pkg"
mount --bind "$CACHE" "$ROOT/var/cache/pacman/pkg"
mounted=1

in_root() { chroot "$ROOT" /usr/bin/env -i HOME=/root TERM="${TERM:-dumb}" LANG=C.UTF-8 PATH=/usr/local/sbin:/usr/local/bin:/usr/bin "$@"; }

echo "==> Installing archiso"
in_root pacman-key --init
in_root pacman-key --populate archlinux
in_root pacman -Syu --noconfirm --needed archiso

echo "==> Assembling the profile"
PROFILE=$ROOT/build/profile
mkdir -p "$PROFILE" "$ROOT/build/out"
cp -a "$HERE/profile/." "$PROFILE/"
# mkarchiso keeps file modes: make sure a restrictive umask on the checkout (e.g. 077) can't produce
# root-only system directories or files. Special cases are set by file_permissions in profiledef.sh.
find "$PROFILE" -type d -exec chmod 755 {} +
find "$PROFILE" -type f -exec chmod go+r,go-w {} +
# Symlinks are kept out of the source tree (so the folder can be copied and backed up anywhere);
# distro/symlinks.txt lists them as "<path in airootfs> <target>" and they are created here.
while read -r link target; do
    [[ -n $link ]] || continue
    mkdir -p "$PROFILE/airootfs/$(dirname "$link")"
    ln -sfn "$target" "$PROFILE/airootfs/$link"
done < "$HERE/symlinks.txt"
install -Dm755 "$BIN/saforawm"          "$PROFILE/airootfs/usr/bin/saforawm"
install -Dm755 "$BIN/saforactl"         "$PROFILE/airootfs/usr/bin/saforactl"
install -Dm755 "$BIN/safora-lock"         "$PROFILE/airootfs/usr/bin/safora-lock"
install -Dm755 "$BIN/safora-bar"        "$PROFILE/airootfs/usr/bin/safora-bar"
install -Dm755 "$BIN/safora-settings"   "$PROFILE/airootfs/usr/bin/safora-settings"
install -Dm755 "$BIN/safora-installer"  "$PROFILE/airootfs/usr/bin/safora-installer"
install -Dm755 "$BIN/safora-viewer"     "$PROFILE/airootfs/usr/bin/safora-viewer"
# USB Burner (distro/usbburner): the GUI finds its helper next to its real path, so /usr/bin/usbburner is a
# symlink (distro/symlinks.txt) to /usr/lib/usbburner/usbburner.py.
install -Dm755 "$HERE/usbburner/usbburner.py"      "$PROFILE/airootfs/usr/lib/usbburner/usbburner.py"
install -Dm755 "$HERE/usbburner/usbburner-helper"  "$PROFILE/airootfs/usr/lib/usbburner/usbburner-helper"
install -Dm644 "$HERE/usbburner/usbburner.desktop" "$PROFILE/airootfs/usr/share/applications/usbburner.desktop"
install -Dm644 "$HERE/usbburner/usbburner.svg"     "$PROFILE/airootfs/usr/share/icons/hicolor/scalable/apps/usbburner.svg"
install -Dm755 "$HERE/installer/safora-install-backend" "$PROFILE/airootfs/usr/lib/safora/safora-install-backend"
cp "$HERE/profile/pacman.conf" "$PROFILE/pacman.conf"
if [[ $FAST == 1 ]]; then
    echo "==> FAST mode: signature checks off, light compression (do not ship this ISO)"
    sed -i 's/^SigLevel .*/SigLevel = Never/' "$PROFILE/pacman.conf"
    sed -i "s/'-Xcompression-level' '15'/'-Xcompression-level' '3'/" "$PROFILE/profiledef.sh"
fi
# The live image's own pacman needs the same mirrors as the build root.
cp "$ROOT/etc/pacman.d/mirrorlist" "$ROOT/build/mirrorlist.build"

echo "==> Building the ISO (this takes a while: it downloads ~2 GB and compresses the image)"
in_root mkarchiso -v -w /build/work -o /build/out /build/profile

mkdir -p "$OUT"
cp "$ROOT"/build/out/*.iso "$OUT/"
chown -R "$OWNER": "$OUT" 2>/dev/null || true
ls -lh "$OUT"

echo "==> Cleaning up"
umount_all
rm -rf "$WORK"
echo "Done: $(ls "$OUT"/*.iso)"
