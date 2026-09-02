#!/usr/bin/env bash
# Write one sorted sha256 manifest for a release directory.
set -euo pipefail

dist=${1:-}
[[ -n "$dist" && -d "$dist" ]] || {
    echo "usage: checksums.sh DIST-DIRECTORY" >&2
    exit 2
}

checksum_file="$dist/checksums.txt"
tmp="$(mktemp "${TMPDIR:-/tmp}/woofer-checksums.XXXXXXXX")"
trap 'rm -f "$tmp"' EXIT

files=()
while IFS= read -r path; do
    files+=("${path##*/}")
done < <(
    find "$dist" -maxdepth 1 -type f \
        ! -name 'checksums.txt' ! -name '*.asc' -print | LC_ALL=C sort
)
(( ${#files[@]} > 0 )) || {
    echo "no release files found in $dist" >&2
    exit 1
}

for file in "${files[@]}"; do
    if command -v sha256sum >/dev/null 2>&1; then
        (cd "$dist" && sha256sum "$file") >> "$tmp"
    else
        (cd "$dist" && shasum -a 256 "$file") >> "$tmp"
    fi
done
mv "$tmp" "$checksum_file"
trap - EXIT
printf '%s\n' "$checksum_file"
