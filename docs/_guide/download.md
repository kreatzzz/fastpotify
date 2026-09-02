---
title: Download
description: Get Woofer for macOS, Windows, or Linux, with install instructions for each.
---

# Download Woofer

Woofer is currently version **0.4.0 in the source tree**. There is no public
binary release yet: publishing is paused while the release workflow and
package-manager channels are prepared. This page will grow direct, checksum-
verified downloads as soon as a tagged release is available.

For now, the reliable way to try Woofer is to build it from source. The
application itself is ready for Linux, macOS, and Windows; the release status
is the part that is still in motion.

## Build from source

You need a stable [Rust](https://rustup.rs) toolchain (1.95 or newer):

```sh
git clone https://github.com/kreatzzz/woofer
cd woofer
cargo install --path .
```

On Linux, install the desktop and audio development libraries first. On Arch:

```sh
sudo pacman -S --needed alsa-lib libpulse libxkbcommon wayland
```

On Debian or Ubuntu:

```sh
sudo apt install libasound2-dev libpulse-dev libxkbcommon-dev libwayland-dev libgl1-mesa-dev
```

The repository's [Nix development shell](https://nixos.org) provides the
same libraries and the pinned toolchain.

## What the release will include

Once a release is tagged, the GitHub release will carry:

- a universal macOS DMG for Apple Silicon and Intel;
- Windows installers and portable archives for x86_64 and ARM64;
- Linux archives for x86_64 and ARM64;
- `checksums.txt` with a SHA-256 entry for every file.

Watch the [Woofer releases](https://github.com/kreatzzz/woofer/releases) page
for the first published build. The [release plan](/dev/release-plan) records
the package-manager order and the unsigned macOS first-open note.

## Platform notes

### macOS

The first public DMG may be unsigned while Apple credentials are being
configured. An unsigned build asks you to approve Woofer once in **System
Settings → Privacy & Security**; later launches work normally. A signed and
notarized DMG skips that first-open warning when the release workflow's full
Apple contract is configured.

### Windows

The installer needs no administrator rights. SmartScreen may warn about an
unknown publisher the first time; choose **More info → Run anyway** after
checking the checksum from the release.

### Linux

The archive includes the binary and desktop integration files. Runtime needs
are the ordinary desktop libraries: ALSA, PulseAudio or PipeWire, and Wayland
or X11. Arch users can use the AUR once the first package is published.

## Package managers

Homebrew, AUR, and winget packages are planned in that order. They are not
published yet, so commands such as `brew install` and `yay -S` are intentionally
not presented as working instructions.
