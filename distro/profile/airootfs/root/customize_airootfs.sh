#!/usr/bin/env bash
# Runs inside the image after packages are installed (the file is removed afterwards).
set -e
cat > /usr/lib/os-release <<'OSR'
NAME="Safora Linux"
PRETTY_NAME="Safora Linux"
ID=safora
ID_LIKE=arch
BUILD_ID=rolling
HOME_URL="https://github.com/tazrog/safora-linux"
TAGLINE="Simple by Nature, Private by default"
ANSI_COLOR="38;2;61;214;208"
OSR

# Unlock the keyring with the login password (greetd login) and keep it in step when the password changes.
# Without this, "Remember forever" for e.g. an encrypted USB drive would have nowhere to store the password.
if ! grep -q pam_gnome_keyring /etc/pam.d/greetd; then
    sed -i '/^auth.*include.*system-local-login/a auth       optional     pam_gnome_keyring.so' /etc/pam.d/greetd
    echo 'session    optional     pam_gnome_keyring.so auto_start' >> /etc/pam.d/greetd
fi
if ! grep -q pam_gnome_keyring /etc/pam.d/passwd; then
    echo 'password   optional     pam_gnome_keyring.so' >> /etc/pam.d/passwd
fi
