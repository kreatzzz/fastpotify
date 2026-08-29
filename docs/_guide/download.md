---
title: Download
description: Get Woofer for macOS, Windows, or Linux, with install instructions for each.
nav_order: 1
---

{% assign v = site.woofer_version %}
{% assign base = "https://github.com/kreatzzz/woofer/releases/download/v" | append: v %}

The current version is **v{{ v }}**. Every file below, with its SHA-256, is
listed in [checksums.txt]({{ base }}/checksums.txt); all versions live on
the [releases page](https://github.com/kreatzzz/woofer/releases).

## macOS

One download for both Apple Silicon and Intel:

- [woofer-v{{ v }}-macos-universal.dmg]({{ base }}/woofer-v{{ v }}-macos-universal.dmg)

Open it and drag **Woofer** to Applications. Or, with
[Homebrew](https://brew.sh):

```sh
brew install --cask crmne/tap/woofer
```

Homebrew installs it like any download, so the first-open steps below
apply once. To skip them, clear the quarantine flag instead:

```sh
xattr -d com.apple.quarantine /Applications/Woofer.app
```

### First open on macOS

This build is not yet notarized with Apple, so macOS blocks it the first
time. Recent macOS versions (Sequoia and later) no longer let you bypass
this with a right-click, so you open it once through Privacy & Security:

1. Double-click **Woofer** in Applications. macOS says it cannot be
   opened because Apple cannot check it for malicious software. Click
   **Done** (do **not** click Move to Trash).
2. Open **System Settings**, then **Privacy & Security**.
3. Scroll down to the **Security** section, find *"Woofer was blocked
   to protect your Mac"*, and click **Open Anyway**.
4. Authenticate, then click **Open Anyway** once more.

macOS remembers the choice: every launch after this is an ordinary
double-click. This step disappears once notarized builds ship.

## Windows

The installer adds Woofer to the Start menu and needs no administrator
rights. Almost every PC wants the first one; the second is for Windows on
ARM:

- [woofer-v{{ v }}-x86_64-pc-windows-msvc-setup.exe]({{ base }}/woofer-v{{ v }}-x86_64-pc-windows-msvc-setup.exe)
- [woofer-v{{ v }}-aarch64-pc-windows-msvc-setup.exe]({{ base }}/woofer-v{{ v }}-aarch64-pc-windows-msvc-setup.exe)

If you would rather not install anything, the same program comes as a zip:
unpack it and run `woofer.exe`.

- [woofer-v{{ v }}-x86_64-pc-windows-msvc.zip]({{ base }}/woofer-v{{ v }}-x86_64-pc-windows-msvc.zip)
- [woofer-v{{ v }}-aarch64-pc-windows-msvc.zip]({{ base }}/woofer-v{{ v }}-aarch64-pc-windows-msvc.zip)

Either way, SmartScreen may warn about an unknown publisher on first run;
choose More info, then Run anyway.

## Linux

### Arch Linux

Woofer is in the AUR, with the desktop entry and icon installed for you:

```sh
yay -S woofer          # the released build
yay -S woofer-git      # built from the latest commit
```

### Flatpak

[FlatPark](https://flatpark.org/apps/rocks.woofer.Woofer) packages
each Linux release as a sandboxed Flatpak and follows every new version:

```sh
flatpak remote-add --if-not-exists flatpark https://dl.flatpark.org/flatpark.flatpakrepo
flatpak install flatpark rocks.woofer.Woofer
```

### Other distributions

- [woofer-v{{ v }}-x86_64-unknown-linux-gnu.tar.gz]({{ base }}/woofer-v{{ v }}-x86_64-unknown-linux-gnu.tar.gz)
- [woofer-v{{ v }}-aarch64-unknown-linux-gnu.tar.gz]({{ base }}/woofer-v{{ v }}-aarch64-unknown-linux-gnu.tar.gz)

Unpack, put `woofer` on your PATH, and copy the desktop entry and icon
from the bundled `packaging/` directory if you want it in your launcher.
Runtime needs are the ordinary desktop libraries: ALSA, PulseAudio or
PipeWire, and Wayland or X11.

Or build from source: see [Getting Started](/getting-started/).
