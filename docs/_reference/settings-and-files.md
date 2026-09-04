---
title: Settings & Files
description: Where Woofer keeps configuration, credentials, and caches, and what is safe to delete.
nav_order: 0
---

# Settings and files

## Where things live

Woofer follows each platform's conventions. On Linux:

| What | Where | Safe to delete? |
| --- | --- | --- |
| Settings | `~/.config/woofer/settings.json` | Yes, you lose preferences |
| Winamp skins | `~/.config/woofer/skins/` | Yes, you add them again |
| Shared Web API sign-in | `~/.local/state/woofer/shared_web_api_token.json` | Yes, you sign in again |
| Personal Web API sign-in | `~/.local/state/woofer/personal_web_api_token.json` | Yes, personal acceleration is removed |
| Playback credential | `~/.local/state/woofer/credentials/` | Yes, you approve playback again |
| Last session | `~/.local/state/woofer/session.json` | Yes |
| Audio cache | `~/.cache/woofer/audio/` | Always |
| Artwork cache | `~/.cache/woofer/art/` | Always |
| Lyrics cache | `~/.cache/woofer/lyrics/` | Always |
| Translations cache | `~/.cache/woofer/translations/` | Always |
| Account-scoped playlist cache | `~/.cache/woofer/playlists/<account-id>/` | Always |
| Installed plugins | `~/.local/state/woofer/plugins/` | No, they are uninstalled |
| Last run's log | `~/.local/state/woofer/woofer.log` | Always |
| Crash log | `~/.local/state/woofer/panic.log` | Always |

Clearing caches never signs you out; credentials live in *state*, not
*cache*. Web API token files are written with owner-only permissions.
Signing out from Settings deletes both Web API grants and the separate
playback credential.

On macOS, settings, state, and the logs are in
`~/Library/Application Support/me.kreatzzz.woofer` and the caches in
`~/Library/Caches/me.kreatzzz.woofer`. On Windows, settings are in
`%APPDATA%\kreatzzz\woofer\config`, state and the logs in
`%LOCALAPPDATA%\kreatzzz\woofer\data`, and the caches in
`%LOCALAPPDATA%\kreatzzz\woofer\cache`.

## settings.json

Settings are stored in one readable JSON file and written atomically. Its
main fields are:

| Field | Default | Meaning |
| --- | --- | --- |
| `device_name` | `Woofer` | Name on Spotify Connect |
| `bitrate` | `320` | 96, 160, or 320 kbps |
| `normalisation` | `false` | Volume normalisation |
| `autoplay` | `true` | Keep playing similar music at the end |
| `gapless` | `true` | Gapless playback |
| `audio_backend` | platform | `pulseaudio` or `rodio` on Linux |
| `audio_cache_mb` | `1024` | On-disk audio cache budget |
| `theme` | `dark` | `dark`, `light`, or `system` |
| `accent_from_art` | `true` | Tint pages with album art |
| `winamp_window` | `false` | The window is the Winamp mini player |
| `skin` | none | A file or folder name in the skins folder; the built-in skin when absent |
| `skin_scale` | by display | Screen pixels per skin pixel, 1 to 4 |
| `winamp_on_top` | `false` | Keep the mini player above other windows |
| `vis` | `bars` | The mini player's visualiser: `bars`, `scope`, or `off` |
| `playlist_open` | `false` | The playlist window is open under the mini player |
| `playlist_height` | `174` | The playlist window's height in skin pixels |
| `eq_open` | `false` | The equalizer window is open under the mini player |
| `eq_on` | `false` | The equalizer shapes local playback |
| `eq_preamp_db` | `0` | The preamp, in decibels, never above zero |
| `eq_bands_db` | ten zeros | The bands from 60 Hz to 16 kHz, in decibels, -12 to 12 |
| `balance` | `0` | Left to right, -1 to 1, for local playback |
| `mono` | `false` | Play both channels the same |
| `playlist_shaded` | `false` | The playlist window is rolled up to its title bar |
| `winamp_shaded` | `false` | The main window is rolled up to its title bar |
| `keep_playing_in_background` | `true` | Close to tray |
| `check_for_updates` | `true` | Ask GitHub once a day for a newer release |
| `web_client_id` | none | Optional personal Spotify app id used alongside shared coverage |
| `lyrics_language` | `en` | Language the lyrics panel translates into |
| `lyrics_show_translation` | `false` | Echo each lyric line in your language |
| `lyrics_romanize` | `false` | Write lyric lines in Latin letters |

## Command line

```
woofer [OPTIONS]

  --device-name <NAME>  Spotify Connect name for this session
  -v, --verbose         More logs from librespot and the API client
```

`woofer.log` in the state directory is what to attach to a bug report:
it contains the last run's output, including the additional lines printed by
`woofer -v`. If the app crashed, attach `panic.log` from the same directory
as well.

## Demo mode

Builds made with `cargo build --features demo` accept `--demo`, which fills
the interface with sample data, useful for screenshots, theming, and
interface work. Demo mode never writes settings.

`--demo-page` opens a page, such as `home`, `playlist:pl1`, or `artist:art0`,
and `--demo-show` adds surfaces on top of it: a comma separated list of
`queue`, `devices`, `lyrics`, `shortcuts`, `premium`, `create`, `light`, `focus`, `winamp`,
`playlist`, `eq`, and `eq-shade`.

`--demo-shot <PATH>` writes the window to a PNG and exits, which is how the
screenshots in these pages are made:

```
cargo run --release --features demo -- \
  --demo-shot docs/screenshot.png --demo-page playlist:pl1 --demo-show queue
```

The shot is the window's own frame buffer, so it comes out at whatever size
the window is. `--demo-shot-delay <MS>` sets how long cover art has to arrive
before the frame is taken.
