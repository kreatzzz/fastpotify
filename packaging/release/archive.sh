#!/usr/bin/env bash
# Make a deterministic portable archive for one target.
set -euo pipefail

usage() {
    cat >&2 <<'EOF'
usage: archive.sh --binary PATH --name NAME --format tar.gz|zip --platform linux|windows --output-dir DIR
EOF
    exit 2
}

binary=''
name=''
format=''
platform=''
output_dir=''

while (($# > 0)); do
    case "$1" in
        --binary)
            [[ $# -ge 2 ]] || usage
            binary=$2
            shift 2
            ;;
        --name)
            [[ $# -ge 2 ]] || usage
            name=$2
            shift 2
            ;;
        --format)
            [[ $# -ge 2 ]] || usage
            format=$2
            shift 2
            ;;
        --platform)
            [[ $# -ge 2 ]] || usage
            platform=$2
            shift 2
            ;;
        --output-dir)
            [[ $# -ge 2 ]] || usage
            output_dir=$2
            shift 2
            ;;
        *)
            usage
            ;;
    esac
done

[[ -n "$binary" && -n "$name" && -n "$format" && -n "$platform" && -n "$output_dir" ]] || usage
[[ "$name" != */* && "$name" != .* ]] || {
    echo "archive name must be a plain directory name: $name" >&2
    exit 2
}
[[ "$format" == tar.gz || "$format" == zip ]] || usage
[[ "$platform" == linux || "$platform" == windows ]] || usage
[[ -f "$binary" ]] || {
    echo "release binary does not exist: $binary" >&2
    exit 1
}

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
mkdir -p "$output_dir"
output_dir="$(cd "$output_dir" && pwd)"
stage_parent="$(mktemp -d "${TMPDIR:-/tmp}/woofer-archive.XXXXXXXX")"
trap 'rm -rf "$stage_parent"' EXIT
stage="$stage_parent/$name"
mkdir -p "$stage"

if [[ "$platform" == linux ]]; then
    # Keep the executable at the archive root so a download can be run as-is.
    cp "$binary" "$stage/woofer"
    chmod 755 "$stage/woofer"
    mkdir -p "$stage/packaging"
    cp -R "$repo_root/packaging/applications" "$stage/packaging/"
    cp -R "$repo_root/packaging/icons" "$stage/packaging/"
else
    cp "$binary" "$stage/woofer.exe"
fi
cp "$repo_root/README.md" "$repo_root/LICENSE" "$stage/"

# A fixed file date removes host-clock noise from zip central-directory entries.
find "$stage" -exec touch -t 198001010000 {} +

archive="$output_dir/$name.$format"
rm -f "$archive"
if [[ "$format" == tar.gz ]]; then
    epoch="${SOURCE_DATE_EPOCH:-0}"
    [[ "$epoch" =~ ^[0-9]+$ ]] || epoch=0
    if tar --help 2>&1 | grep -q -- '--sort=name'; then
        # GNU tar gives stable order, ownership, and mtimes in CI.
        tar --sort=name --mtime="@${epoch}" --owner=0 --group=0 \
            --numeric-owner -cf - -C "$stage_parent" "$name" | gzip -n > "$archive"
    else
        # This fallback keeps local BSD tar smoke tests useful; CI uses GNU
        # tar. Feed an uncompressed stream to gzip -n so its header does not
        # carry the current clock time.
        tar -cf - -C "$stage_parent" "$name" | gzip -n > "$archive"
    fi
else
    if command -v 7z >/dev/null 2>&1; then
        # 7-Zip is available on the Windows runner and can omit NTFS metadata.
        (cd "$stage_parent" && 7z a -tzip -mx=9 -mtc=off -mta=off -mtm=off \
            "$archive" "$name" >/dev/null)
    elif command -v zip >/dev/null 2>&1; then
        # The sorted file list keeps zip output stable on Unix hosts.
        (cd "$stage_parent" && LC_ALL=C find "$name" -print | sort | \
            zip -X -q -9 "$archive" -@)
    else
        echo 'zip creation requires 7z or zip' >&2
        exit 1
    fi
fi

[[ -s "$archive" ]] || {
    echo "archive was not created: $archive" >&2
    exit 1
}
printf '%s\n' "$archive"
