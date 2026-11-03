#!/bin/bash
# Test VM for the Safora ISO with QEMU/KVM, run unprivileged from the build sandbox (scripts/sandbox.sh;
# needs: pacman -S qemu-desktop edk2-ovmf inside it). The VM lives outside the project folder, in
# ~/.local/share/safora/vm.
#
#   scripts/qemu-vm.sh new [iso]     fresh 24 GB disk + UEFI vars, boot the ISO (newest out/safora-*.iso)
#   scripts/qemu-vm.sh iso [iso]     boot the ISO again, keeping the disk
#   scripts/qemu-vm.sh disk          boot the installed system from the disk
#
# USB=~/.local/share/safora/vm/testdrive.img attaches a LUKS-encrypted 64 MB test drive (passphrase
# "testdrive-pass"), for checking "Remember forever" in the file manager.
# HEADLESS=1 runs without a window. A QMP socket at ~/.local/share/safora/vm/qmp.sock lets
# scripts/qemu-drive.py type and take screenshots (automated tests).
set -euo pipefail
TOP=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
VM=${SAFORA_VM:-$HOME/.local/share/safora/vm}
SANDBOX=${SAFORA_SANDBOX:-$HOME/.local/share/safora/archroot}
MODE=${1:-new}
ISO=${2:-$(ls -t "$TOP"/out/safora-*.iso 2>/dev/null | head -1)}
mkdir -p "$VM"

in_sandbox() {
    bwrap --unshare-user --uid 0 --gid 0 --bind "$SANDBOX" / --dev-bind /dev /dev --proc /proc --ro-bind /sys /sys \
        --tmpfs /tmp --tmpfs /run --dir /run/user/0 --bind "$VM" /vm --ro-bind "$TOP/out" /out \
        --setenv HOME /root --setenv XDG_RUNTIME_DIR /run/user/0 --setenv SDL_VIDEODRIVER wayland \
        ${WAYLAND_DISPLAY:+--bind "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" "/run/user/0/$WAYLAND_DISPLAY" --setenv WAYLAND_DISPLAY "$WAYLAND_DISPLAY"} \
        "$@"
}

if [[ $MODE == new ]]; then
    rm -f "$VM/disk.qcow2" "$VM/OVMF_VARS.fd"
    in_sandbox qemu-img create -q -f qcow2 /vm/disk.qcow2 24G
    in_sandbox cp /usr/share/edk2/x64/OVMF_VARS.4m.fd /vm/OVMF_VARS.fd
fi
[[ -f $VM/disk.qcow2 ]] || { echo "No VM disk yet: run  $0 new" >&2; exit 1; }

ARGS=(-name Safora-Test -machine q35,accel=kvm -cpu host -smp 4 -m 6144
      -drive if=pflash,format=raw,readonly=on,file=/usr/share/edk2/x64/OVMF_CODE.4m.fd
      -drive if=pflash,format=raw,file=/vm/OVMF_VARS.fd
      -drive file=/vm/disk.qcow2,if=none,id=disk0,format=qcow2 -device nvme,drive=disk0,serial=SAFORA0,bootindex=1
      -device virtio-vga -device qemu-xhci -device usb-kbd -device usb-tablet
      -nic user,model=virtio-net-pci -audiodev none,id=snd -device intel-hda -device hda-duplex,audiodev=snd)
if [[ $MODE == new || $MODE == iso ]]; then
    [[ -f $ISO ]] || { echo "ISO not found: $ISO" >&2; exit 1; }
    # Attached as a USB stick, like on real hardware: with enough RAM archiso then copies the system
    # to RAM and unmounts the medium (copytoram=auto), which an optical drive would not trigger.
    ARGS+=(-drive "if=none,id=stick,format=raw,readonly=on,file=/out/$(basename "$ISO")"
           -device usb-storage,drive=stick,removable=on,bootindex=0)
fi
# USB=path/to/image attaches an extra USB drive (e.g. an encrypted test drive, see testdrive.img below).
if [[ -n ${USB:-} ]]; then
    # QEMU only sees the VM folder (as /vm): copy the image there unless it already lives there.
    [[ $(dirname "$(realpath "$USB")") == "$(realpath "$VM")" ]] || cp -f "$USB" "$VM/"
    ARGS+=(-drive "if=none,id=extra,format=raw,file=/vm/$(basename "$USB")" -device usb-storage,drive=extra,removable=on)
fi
rm -f "$VM/qmp.sock"
ARGS+=(-qmp unix:/vm/qmp.sock,server=on,wait=off)   # for scripts/qemu-drive.py (keys, screenshots)
if [[ ${HEADLESS:-0} == 1 ]]; then
    ARGS+=(-display none)
else
    ARGS+=(-display sdl)   # GTK fails here: its image loader cannot sandbox itself inside bwrap
fi
in_sandbox qemu-system-x86_64 "${ARGS[@]}"
