#!/usr/bin/env bash
# Create (or recreate) a VirtualBox VM for testing the Sekuro ISO.
#   ./distro/vbox-vm.sh [uefi|bios] [path/to/sekuro.iso]
set -Eeuo pipefail
FW=${1:-uefi}
TOP=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
ISO=${2:-$(ls -t "$TOP"/out/sekuro-*.iso 2>/dev/null | head -1)}
NAME=Sekuro-Test-$FW
DIR="$HOME/VirtualBox VMs/$NAME"

[[ -f $ISO ]] || { echo "ISO not found: $ISO" >&2; exit 1; }

if VBoxManage showvminfo "$NAME" >/dev/null 2>&1; then
    VBoxManage controlvm "$NAME" poweroff >/dev/null 2>&1 || true
    sleep 1
    VBoxManage unregistervm "$NAME" --delete >/dev/null
fi

VBoxManage createvm --name "$NAME" --ostype ArchLinux_64 --register >/dev/null
VBoxManage modifyvm "$NAME" \
    --memory 4096 --cpus 2 --vram 128 --graphicscontroller vmsvga --accelerate3d on \
    --firmware "$([ "$FW" = uefi ] && echo efi || echo bios)" --ioapic on --rtcuseutc on \
    --mouse usbtablet --keyboard usb \
    --audio-enabled on --audio-driver default --audio-controller hda --audio-out on \
    --nic1 nat --boot1 dvd --boot2 disk --boot3 none --boot4 none \
    --clipboard-mode disabled --draganddrop disabled
VBoxManage createmedium disk --filename "$DIR/disk.vdi" --size 24576 --format VDI >/dev/null
VBoxManage storagectl "$NAME" --name SATA --add sata --controller IntelAhci --portcount 2
VBoxManage storageattach "$NAME" --storagectl SATA --port 0 --device 0 --type hdd --medium "$DIR/disk.vdi"
VBoxManage storageattach "$NAME" --storagectl SATA --port 1 --device 0 --type dvddrive --medium "$ISO"
echo "Created $NAME (firmware: $FW). Start with: VBoxManage startvm $NAME"
