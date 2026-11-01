#!/usr/bin/env bash
# Verify the ISO draws only from official Arch repositories (no AUR, no third-party repos).
set -Eeuo pipefail
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
conf="$HERE/profile/pacman.conf"

repos=$(grep -E '^\[.+\]$' "$conf" | tr -d '[]' | grep -v '^options$' | tr '\n' ' ')
for r in $repos; do
    case $r in core|extra) ;; *) echo "FAIL: non-official repository [$r] in $conf" >&2; exit 1 ;; esac
done
echo "repositories: $repos(official only)"

if grep -R -n -i -E 'yay|paru|aur\.archlinux|makepkg|git\+https' "$HERE/profile/packages.x86_64" "$HERE/installer" "$HERE/profile/airootfs/usr/bin" >/dev/null 2>&1; then
    echo "FAIL: AUR tooling referenced in the profile" >&2
    grep -R -n -i -E 'yay|paru|aur\.archlinux|makepkg|git\+https' "$HERE/profile/packages.x86_64" "$HERE/installer" "$HERE/profile/airootfs/usr/bin" >&2
    exit 1
fi
echo "no AUR helpers or sources referenced"

# Every listed package must resolve from the official sync databases (run where pacman is available).
if command -v pacman >/dev/null && [ -d /var/lib/pacman/sync ]; then
    missing=0
    while read -r pkg; do
        [[ -z $pkg || $pkg == \#* ]] && continue
        pacman -Si "$pkg" >/dev/null 2>&1 || { echo "FAIL: $pkg is not in core/extra" >&2; missing=1; }
    done < "$HERE/profile/packages.x86_64"
    [[ $missing == 0 ]] || exit 1
    echo "all packages resolve from core/extra"
fi
