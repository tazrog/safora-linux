# USB Burner

A small GTK 4 app for writing Linux distro images to USB drives so you can boot from them.

## Features
- Writes `.iso`, `.img`, and `.raw` files, plus compressed `.xz`, `.gz`, `.bz2`, and `.zst` images (decompressed while writing).
- Lists removable/USB drives only. It never shows the drive your system is running from.
- Can check the image's SHA-256 against the checksum published by the distro before writing.
- Reads the drive back after writing to confirm the data matches (catches bad or fake sticks).
- Shows progress, speed, and time left. You can cancel at any point.
- Unmounts the drive's partitions automatically. It refuses a drive with an open LUKS or LVM volume.
- The window runs as your normal user. Only the small `usbburner-helper` runs as root, through `sudo`. The app asks for your password itself, so it doesn't need a polkit agent.

## Requirements
```sh
sudo pacman -S python-gobject gtk4 sudo
```
Your user must be allowed to use `sudo` (for example, in the `wheel` group with `%wheel ALL=(ALL:ALL) ALL` enabled in sudoers).

## Run without installing
```sh
./usbburner.py
```

## Install
```sh
sudo ./install.sh              # installs to /usr/local and adds it to your app menu
sudo ./install.sh --uninstall
```

## Notes
- Linux ISOs are hybrid images, so a raw write is the right method. Windows ISOs need a different tool, such as WoeUSB.
- The helper checks the target again as root: it must be a whole, writable, removable disk, and none of its partitions may be mounted at a system path.
