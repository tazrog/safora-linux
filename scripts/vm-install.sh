#!/bin/bash
# Drive the Sekuro installer in a VirtualBox VM with keystrokes only (encrypted install of /dev/sda).
#   vm-install.sh [VM] [claude]
VM=${1:-Sekuro-Test-uefi}; K=$(dirname "$0")/vmkeys.sh
NEXT() { VBoxManage controlvm $VM keyboardputscancode 38 e0 4d e0 cd b8; sleep 1.5; }
NEXT                                             # welcome -> network/bluetooth
NEXT                                             # -> disk
NEXT                                             # -> encryption
$K $VM tab "text:sekuropass-9876" tab tab "text:sekuropass-9876" stab "text:sekuropass-9876"; NEXT
$K $VM tab "text:Test User" tab "text:tester" tab tab "text:userpw123" tab "text:userpw123"; NEXT
[ "$2" = claude ] && $K $VM tab space           # tick "Install Claude Code"
NEXT                                             # extras -> confirm
$K $VM tab space; NEXT                           # confirm + Install
