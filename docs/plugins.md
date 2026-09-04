---
title: Plugins
description: Add reviewed lyrics, translation, and romanization providers to Woofer without giving them control of the app.
---

# Plugins, without the takeover

Woofer plugins are small, reviewed WebAssembly modules that extend the app's
lyrics tools. They can provide lyrics, translate a song, or write its words in
the Latin alphabet. Woofer keeps ownership of the interface, network, files,
and playback.

The catalog is live at [usewoofer.com](https://usewoofer.com). Woofer itself is
tagged and verified at **v0.4.0**, but its public binaries have not been
published yet. Until they are, plugin installation is for people running a
[source build](/download).

## What you can add today

The first reviewed providers are:

- **Translate** — adds a translation beneath each lyric line and skips songs
  already written in your chosen language.
- **Romanize** — writes non-Latin lyrics in Latin characters, one line at a
  time.

The host also supports lyrics providers, although the catalog does not list
one yet. Woofer's own Spotify and LRCLIB lyrics flow remains available.

[Browse the plugin catalog](https://usewoofer.com/plugins) ·
[Read the complete architecture](/dev/plugin-architecture)

## How installation works

1. Choose a plugin in the catalog and select **Open in Woofer**.
2. Woofer shows the publisher, version, capabilities, and requested domains.
3. Confirm the install. Woofer verifies the downloaded module against the
   catalog's SHA-256 digest before storing it.
4. Reorder providers from Woofer's Plugins page when more than one can answer
   the same request.

You can also install a local `.wasm` file from the Plugins page when developing
or reviewing a provider.

## A deliberately narrow sandbox

A plugin is pure computation. It cannot open a socket, read your files, spawn a
process, inspect Spotify credentials, draw arbitrary interface elements, or
touch the playback engine. When it needs remote data, it describes the request
and Woofer performs it only for domains declared in the manifest.

Every call has fuel, memory, response-size, and time limits. A provider that
misses or fails passes the request to the next provider. After three consecutive
failures, Woofer disables that module for the session and keeps the rest of the
chain running.

## Built to work without plugins

No plugin ships inside the application. Translation, romanization, Spotify
lyrics, and LRCLIB remain built-in fallbacks, so a clean installation is useful
on its own. Removing every plugin returns Woofer to that baseline.

## Build a provider

The repository includes a Rust SDK, an offline wasmi test harness, and the
source for the official Translate and Romanize providers. Start with the
[plugin SDK guide](https://github.com/kreatzzz/woofer/tree/main/plugins) and use
the [full ABI and sandbox specification](/dev/plugin-architecture) when
implementing or reviewing a module.

Catalog submissions are approval-only. Each entry links to its source, declares
its network domains, and is checked before the catalog is regenerated.
