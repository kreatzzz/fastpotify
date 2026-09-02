#!/usr/bin/env bash
# Check release names and archive safety before an asset can be published.
set -euo pipefail

dist=${1:-}
version=${2:-}
[[ -n "$dist" && -n "$version" && -d "$dist" ]] || {
    echo "usage: verify-artifacts.sh DIST-DIRECTORY VERSION" >&2
    exit 2
}

linux_x64="woofer-v${version}-x86_64-unknown-linux-gnu"
linux_arm="woofer-v${version}-aarch64-unknown-linux-gnu"
windows_x64="woofer-v${version}-x86_64-pc-windows-msvc"
windows_arm="woofer-v${version}-aarch64-pc-windows-msvc"
macos="woofer-v${version}-macos-universal"

expected=(
    "$linux_x64.tar.gz"
    "$linux_x64.AppImage"
    "$linux_arm.tar.gz"
    "$linux_arm.AppImage"
    "$windows_x64.zip"
    "$windows_x64-setup.exe"
    "$windows_arm.zip"
    "$windows_arm-setup.exe"
    "$macos.dmg"
)

for file in "${expected[@]}"; do
    [[ -s "$dist/$file" ]] || {
        echo "missing or empty release asset: $file" >&2
        exit 1
    }
done

archive_entries_are_safe() {
    local archive=$1
    local listing
    listing=$(tar -tzf "$archive")
    while IFS= read -r entry; do
        [[ "$entry" != /* && "$entry" != *'../'* && "$entry" != ../* ]] || {
            echo "unsafe path in $archive: $entry" >&2
            return 1
        }
    done <<< "$listing"
    grep -Eq '/(woofer|README\.md|LICENSE)$' <<< "$listing"
}

for archive in "$dist/$linux_x64.tar.gz" "$dist/$linux_arm.tar.gz"; do
    archive_entries_are_safe "$archive"
done

for archive in "$dist/$windows_x64.zip" "$dist/$windows_arm.zip"; do
    if command -v unzip >/dev/null 2>&1; then
        unzip -Z1 "$archive" | awk '
            BEGIN { bad=0; found=0 }
            { if ($0 ~ /^\// || $0 ~ /(^|\/)\.\.\//) bad=1; if ($0 ~ /(^|\/)woofer\.exe$/) found=1 }
            END { if (bad || !found) exit 1 }
        '
    elif command -v 7z >/dev/null 2>&1; then
        7z t "$archive" >/dev/null
    else
        echo 'zip verification requires unzip or 7z' >&2
        exit 1
    fi
done

# The remaining checks intentionally stay format-level: DMG and AppImage are
# made on their native runners and are inspected there before upload.
file "$dist/$linux_x64.AppImage" "$dist/$linux_arm.AppImage" \
    "$dist/$macos.dmg" "$dist/$windows_x64-setup.exe" \
    "$dist/$windows_arm-setup.exe" >/dev/null

printf '%s\n' 'release assets passed naming and archive-safety checks'
