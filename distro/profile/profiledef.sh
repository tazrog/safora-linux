#!/usr/bin/env bash
# shellcheck disable=SC2034

iso_name="steady"
iso_label="STEADY_$(date --date="@${SOURCE_DATE_EPOCH:-$(date +%s)}" +%Y%m)"
iso_publisher="Steady Linux"
iso_application="Steady Linux live installer"
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
  ["/usr/bin/steady-session"]="0:0:755"
  ["/usr/bin/steady-install-claude"]="0:0:755"
  ["/usr/bin/steadywm"]="0:0:755"
  ["/usr/bin/steadyctl"]="0:0:755"
  ["/usr/bin/steady-bar"]="0:0:755"
  ["/usr/bin/steady-lock"]="0:0:755"
  ["/usr/bin/steady-settings"]="0:0:755"
  ["/usr/bin/steady-installer"]="0:0:755"
  ["/usr/lib/steady/steady-install-backend"]="0:0:755"
  ["/usr/lib/steady/steady-sysconf"]="0:0:755"
  ["/usr/lib/initcpio/install/steadybt"]="0:0:755"
  ["/usr/lib/initcpio/hooks/steadybt"]="0:0:755"
)
