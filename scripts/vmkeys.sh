#!/bin/bash
# usage: vmkeys.sh VM tab|enter|space|down|up|"text:..." ...
VM=$1; shift
for k in "$@"; do
  case $k in
    tab) VBoxManage controlvm $VM keyboardputscancode 0f 8f;;
    stab) VBoxManage controlvm $VM keyboardputscancode 2a 0f 8f aa;;
    enter) VBoxManage controlvm $VM keyboardputscancode 1c 9c;;
    space) VBoxManage controlvm $VM keyboardputscancode 39 b9;;
    down) VBoxManage controlvm $VM keyboardputscancode e0 50 e0 d0;;
    up) VBoxManage controlvm $VM keyboardputscancode e0 48 e0 c8;;
    text:*) VBoxManage controlvm $VM keyboardputstring "${k#text:}";;
  esac
  sleep 0.3
done
