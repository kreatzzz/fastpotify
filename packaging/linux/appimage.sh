#!/usr/bin/env bash
# Assemble an AppDir and turn it into an AppImage on a native Linux runner.
set -euo pipefail

usage() {
    cat >&2 <<'EOF'
usage: appimage.sh --binary PATH --target TARGET --version VERSION --output PATH
       --appimage-tool PATH [--linuxdeploy-tool PATH]
EOF
    exit 2
}

binary=''
target=''
version=''
output=''
appimage_tool=''
linuxdeploy_tool=''

while (($# > 0)); do
    case "$1" in
        --binary|--target|--version|--output|--appimage-tool|--linuxdeploy-tool)
            [[ $# -ge 2 ]] || usage
            case "$1" in
                --binary) binary=$2 ;;
                --target) target=$2 ;;
                --version) version=$2 ;;
                --output) output=$2 ;;
                --appimage-tool) appimage_tool=$2 ;;
                --linuxdeploy-tool) linuxdeploy_tool=$2 ;;
            esac
            shift 2
            ;;
        *)
            usage
            ;;
    esac
done

[[ -n "$binary" && -n "$target" && -n "$version" && -n "$output" && -n "$appimage_tool" ]] || usage
[[ -f "$binary" && -x "$appimage_tool" ]] || {
    echo 'AppImage input binary and appimagetool must exist' >&2
    exit 1
}
[[ "$target" == *-unknown-linux-gnu ]] || {
    echo "unsupported Linux target: $target" >&2
    exit 2
}

arch=${target%%-*}
case "$arch" in
    x86_64|aarch64|armv7)
        ;;
    *)
        echo "no AppImage tool mapping for target architecture: $arch" >&2
        exit 2
        ;;
esac

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
stage_parent="$(mktemp -d "${TMPDIR:-/tmp}/woofer-appimage.XXXXXXXX")"
trap 'rm -rf "$stage_parent"' EXIT
appdir="$stage_parent/Woofer.AppDir"
mkdir -p "$appdir/usr/bin" "$appdir/usr/share/applications" \
    "$appdir/usr/share/icons/hicolor/scalable/apps"
cp "$binary" "$appdir/usr/bin/woofer"
chmod 755 "$appdir/usr/bin/woofer"
cp "$repo_root/packaging/applications/woofer.desktop" \
    "$appdir/usr/share/applications/woofer.desktop"
cp "$repo_root/packaging/applications/woofer.desktop" "$appdir/woofer.desktop"
cp "$repo_root/packaging/icons/woofer.svg" \
    "$appdir/usr/share/icons/hicolor/scalable/apps/woofer.svg"
cp "$repo_root/packaging/icons/woofer.svg" "$appdir/woofer.svg"
cp "$repo_root/README.md" "$repo_root/LICENSE" "$appdir/"
cat > "$appdir/AppRun" <<'EOF'
#!/bin/sh
# Resolve the mounted AppImage before starting the bundled executable.
HERE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
exec "$HERE/usr/bin/woofer" "$@"
EOF
chmod 755 "$appdir/AppRun"

# AppImage tools include file metadata, so keep it independent of runner time.
find "$appdir" -exec touch -t 198001010000 {} +
mkdir -p "$(dirname "$output")"
rm -f "$output"
export PATH="$(dirname "$appimage_tool"):$PATH"
export APPIMAGETOOL="$appimage_tool"

if [[ -n "$linuxdeploy_tool" ]]; then
    [[ -x "$linuxdeploy_tool" ]] || {
        echo "linuxdeploy is not executable: $linuxdeploy_tool" >&2
        exit 1
    }
    # Linuxdeploy copies non-glibc shared libraries while preserving system
    # glibc compatibility; appimagetool then writes the final image.
    workdir="$stage_parent/output"
    mkdir -p "$workdir"
    (
        cd "$workdir"
        ARCH="$arch" APPIMAGE_EXTRACT_AND_RUN=1 "$linuxdeploy_tool" \
            --appdir "$appdir" --executable "$appdir/usr/bin/woofer" \
            --desktop-file "$appdir/usr/share/applications/woofer.desktop" \
            --icon-file "$appdir/usr/share/icons/hicolor/scalable/apps/woofer.svg" \
            --output appimage
    )
    generated=$(find "$workdir" -maxdepth 1 -type f -name '*.AppImage' -print -quit)
    [[ -n "$generated" ]] || {
        echo 'linuxdeploy did not produce an AppImage' >&2
        exit 1
    }
    mv "$generated" "$output"
else
    # A caller may provide only appimagetool when distro libraries are already
    # known to be available; CI supplies linuxdeploy for a self-contained image.
    ARCH="$arch" APPIMAGE_EXTRACT_AND_RUN=1 "$appimage_tool" "$appdir" "$output"
fi

[[ -s "$output" ]] || {
    echo "AppImage was not created: $output" >&2
    exit 1
}
chmod 755 "$output"
printf '%s\n' "$output"
