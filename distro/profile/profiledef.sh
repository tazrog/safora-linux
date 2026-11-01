#!/usr/bin/env bash
# shellcheck disable=SC2034

iso_name="safora"
iso_label="SAFORA_$(date --date="@${SOURCE_DATE_EPOCH:-$(date +%s)}" +%Y%m)"
iso_publisher="Safora Linux"
iso_application="Safora Linux live installer"
iso_version="$(date --date="@${SOURCE_DATE_EPOCH:-$(date +%s)}" +%Y.%m.%d)"
install_dir="arch"
buildmodes=('iso')
bootmodes=('bios.syslinux'
           'uefi.systemd-boot')
arch="x86_64"
pacman_conf="pacman.conf"
airootfs_image_type="squashfs"
airootfs_image_tool_options=('-comp' 'zstd' '-Xcompression-level' '15' '-b' '1M')
file_permissions=(
  ["/etc/shadow"]="0:0:400"
  ["/etc/sudoers.d/99-live"]="0:0:440"
  ["/etc/sudoers.d/10-wheel"]="0:0:440"
  ["/root"]="0:0:750"
  ["/root/customize_airootfs.sh"]="0:0:755"
  ["/usr/bin/safora-session"]="0:0:755"
  ["/usr/bin/safora-install-claude"]="0:0:755"
  ["/usr/bin/saforawm"]="0:0:755"
  ["/usr/bin/saforactl"]="0:0:755"
  ["/usr/bin/safora-bar"]="0:0:755"
  ["/usr/bin/safora-lock"]="0:0:755"
  ["/usr/bin/safora-settings"]="0:0:755"
  ["/usr/bin/safora-installer"]="0:0:755"
  ["/usr/lib/safora/safora-install-backend"]="0:0:755"
  ["/usr/lib/safora/safora-sysconf"]="0:0:755"
  ["/usr/lib/safora/saforabt-firmware"]="0:0:755"
  ["/usr/lib/initcpio/install/saforabt"]="0:0:755"
  ["/usr/lib/initcpio/hooks/saforabt"]="0:0:755"
)
