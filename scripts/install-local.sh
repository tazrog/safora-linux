#!/usr/bin/env bash
# Install this checkout's apps and system config onto the running Safora system (the same files the ISO ships).
#
#   sudo scripts/install-local.sh         # image viewer, USB Burner, launcher/default-app config, Low RAM boot entry
#   sudo scripts/install-local.sh --all   # also every other Safora binary (compositor, bar, settings, ...)
#
# Build first:  scripts/sandbox.sh bash -c 'cd /src && cargo build --release'
set -Eeuo pipefail
[[ $EUID -eq 0 ]] || { echo "Run this script with sudo." >&2; exit 1; }

TOP=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
BIN=$TOP/src/target/release
AIR=$TOP/distro/profile/airootfs

BINS=(safora-viewer)
[[ ${1:-} == --all ]] && BINS+=(saforawm saforactl safora-bar safora-lock safora-settings safora-installer)
for b in "${BINS[@]}"; do
    if [[ -x $BIN/$b ]]; then
        # install writes a new file and renames it, so running programs (the compositor, the bar) keep working.
        install -Dm755 "$BIN/$b" "/usr/bin/$b"
        echo "  /usr/bin/$b"
    fi
done

for f in usr/share/applications/safora-viewer.desktop etc/xdg/mimeapps.list etc/xdg/fuzzel/fuzzel.ini; do
    install -Dm644 "$AIR/$f" "/$f"
    echo "  /$f"
done

# USB Burner: same layout as the ISO (replaces an older /usr/local install).
install -Dm755 "$TOP/distro/usbburner/usbburner.py"      /usr/lib/usbburner/usbburner.py
install -Dm755 "$TOP/distro/usbburner/usbburner-helper"  /usr/lib/usbburner/usbburner-helper
install -Dm644 "$TOP/distro/usbburner/usbburner.desktop" /usr/share/applications/usbburner.desktop
ln -sfn /usr/lib/usbburner/usbburner.py /usr/bin/usbburner
rm -rf /usr/local/lib/usbburner /usr/local/bin/usbburner /usr/local/share/applications/usbburner.desktop
echo "  /usr/bin/usbburner"

# Low RAM boot entry (local AI): session script, unit generator and the GRUB menu entry.
install -Dm755 "$AIR/usr/bin/safora-session" /usr/bin/safora-session
install -Dm755 "$AIR/usr/lib/safora/safora-lowram-generator" /usr/lib/safora/safora-lowram-generator
ln -sfn /usr/lib/safora/safora-lowram-generator /usr/lib/systemd/system-generators/safora-lowram
ln -sfn /usr/lib/safora/safora-lowram-generator /usr/lib/systemd/user-generators/safora-lowram
install -Dm755 "$AIR/etc/grub.d/11_safora_lowram" /etc/grub.d/11_safora_lowram
if [[ -f /boot/grub/grub.cfg ]]; then
    grub-mkconfig -o /boot/grub/grub.cfg >/dev/null 2>&1 && echo "  boot menu: Safora Linux - Low RAM (local AI)" \
        || echo "note: grub-mkconfig failed; run  sudo grub-mkconfig -o /boot/grub/grub.cfg"
fi

pacman -S --needed --noconfirm xarchiver 7zip >/dev/null && echo "  xarchiver, 7zip"
update-desktop-database -q /usr/share/applications || true
echo "Done. Restart the session (or the compositor) to run the new saforawm; other apps take effect on next start."
