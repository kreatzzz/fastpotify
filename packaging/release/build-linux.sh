#!/usr/bin/env bash
# Build the portable Linux archive and AppImage for one Rust target.
set -euo pipefail

usage() {
    cat >&2 <<'EOF'
usage: build-linux.sh --binary PATH --target TARGET --version VERSION --output-dir DIR
       [--appimage-tool PATH] [--linuxdeploy-tool PATH]
EOF
    exit 2
}

binary=''
target=''
version=''
output_dir=''
appimage_tool="${APPIMAGETOOL:-}"
linuxdeploy_tool="${LINUXDEPLOY:-}"

while (($# > 0)); do
    case "$1" in
        --binary|--target|--version|--output-dir|--appimage-tool|--linuxdeploy-tool)
            [[ $# -ge 2 ]] || usage
            case "$1" in
                --binary) binary=$2 ;;
                --target) target=$2 ;;
                --version) version=$2 ;;
                --output-dir) output_dir=$2 ;;
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

[[ -n "$binary" && -n "$target" && -n "$version" && -n "$output_dir" ]] || usage
[[ "$target" == *-unknown-linux-gnu ]] || {
    echo "unsupported Linux target: $target" >&2
    exit 2
}
[[ -x "$appimage_tool" ]] || {
    echo "an executable appimagetool is required (--appimage-tool)" >&2
    exit 1
}

name="woofer-v${version}-${target}"
mkdir -p "$output_dir"
bash "$(dirname "${BASH_SOURCE[0]}")/archive.sh" \
    --binary "$binary" --name "$name" --format tar.gz --platform linux \
    --output-dir "$output_dir"

appimage_args=(
    --binary "$binary"
    --target "$target"
    --version "$version"
    --output "$output_dir/$name.AppImage"
    --appimage-tool "$appimage_tool"
)
if [[ -n "$linuxdeploy_tool" ]]; then
    appimage_args+=(--linuxdeploy-tool "$linuxdeploy_tool")
fi
bash "$(dirname "${BASH_SOURCE[0]}")/../linux/appimage.sh" "${appimage_args[@]}"
