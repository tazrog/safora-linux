#!/usr/bin/env bash
# shellcheck disable=SC2034

iso_name="sekuro"
iso_label="SEKURO_$(date --date="@${SOURCE_DATE_EPOCH:-$(date +%s)}" +%Y%m)"
iso_publisher="Sekuro Linux"
iso_application="Sekuro Linux live installer"
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
  ["/usr/bin/sekuro-session"]="0:0:755"
  ["/usr/bin/sekuro-install-claude"]="0:0:755"
  ["/usr/bin/sekurowm"]="0:0:755"
  ["/usr/bin/sekuroctl"]="0:0:755"
  ["/usr/bin/sekuro-bar"]="0:0:755"
  ["/usr/bin/sekuro-settings"]="0:0:755"
  ["/usr/bin/sekuro-installer"]="0:0:755"
  ["/usr/lib/sekuro/sekuro-install-backend"]="0:0:755"
  ["/usr/lib/sekuro/sekuro-sysconf"]="0:0:755"
  ["/usr/lib/initcpio/install/sekurobt"]="0:0:755"
  ["/usr/lib/initcpio/hooks/sekurobt"]="0:0:755"
)
