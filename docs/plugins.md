# Woofer Plugin System — Design

Status: approved plan · API target: v1 · Runtime: wasmi (wasm32-wasip1)

## 1. Goals and non-goals

**Goals**

- Keep Woofer bloat-free: the binary grows once (the interpreter), then
  shrinks as features move out of core into on-demand plugins.
- A plugin is a sandboxed `.wasm` file. It computes; the host does
  everything else. Downloading a plugin from the catalog must never
  endanger the machine.
- One plugin build runs on Linux, macOS, and Windows.
- Approval-only catalog: nothing reaches the website without a reviewed
  merge.

**Non-goals (v1)**

- Freeform UI from plugins. Plugins contribute data; the host renders.
- Async inside plugins. The host orchestrates; plugins are pure functions.
- Native/dylib plugins, background daemons, audio DSP.

## 2. Architecture

```
┌─────────────────────────── Woofer host (native, egui) ─────────────────────────┐
│  UI · playback (librespot) · Web API · caches                                  │
│  ┌──────────────────────┐   ┌────────────────────────────────────────────────┐ │
│  │ Capability registry  │   │ Plugin host (wasmi)                              │ │
│  │ lyrics / translation │◄──┤ instantiate · call · fuel · memory cap · timeouts│ │
│  │ romanize / commands  │   └───────────────┬────────────────────────────────┘ │
│  └──────────┬───────────┘                   │ two-step request pattern         │
│             │                        ┌──────▼───────┐                          │
│  ┌──────────▼───────────┐            │  plugin.wasm │  pure compute, no I/O    │
│  │ Domain-scoped HTTP   │───────────►│  (sandboxed) │                          │
│  │ executor (native)    │◄───────────┤              │                          │
│  └──────────────────────┘            └──────────────┘                          │
└─────────────────────────────────────────────────────────────────────────────────┘
```

## 3. Plugin format

A plugin is one `.wasm` module (`wasm32-wasip1`) plus a manifest:

```json
{
  "id": "romanize",
  "name": "Romanize",
  "publisher": "kreatzzz",
  "version": "1.0.0",
  "api": 1,
  "capabilities": ["translation-provider:romanize"],
  "domains": ["clients5.google.com"],
  "homepage": "https://github.com/kreatzzz/woofer-plugin-romanize",
  "sha256": "…wasm digest…",
  "license": "MIT"
}
```

- `api` gates loading: the host refuses plugins newer than itself.
- `domains` is enforced by the host's HTTP executor — the plugin can
  *ask*, the host decides.
- `sha256` is pinned at install time and re-verified on every launch.

## 4. Host API v1

Wire format is JSON over a flat ABI. The plugin exports these functions
and imports only the host functions listed; it has no clock, no socket,
no file, and no thread.

| Export (plugin) | Purpose |
| --- | --- |
| `manifest() -> json` | identity, capabilities, settings schema, domains |
| `build_request(capability, input) -> request` | pure: decide WHAT to fetch |
| `handle_response(capability, input, response) -> output` | pure: parse it |
| `on_event(event) -> ()` | playback state changes (v1: now-playing only) |
| `command_run(id, context) -> result` | a registered command fired |

| Host function (import) | Purpose |
| --- | --- |
| `storage_get / storage_put` | per-plugin namespaced KV, quota-capped |
| `log(level, message)` | lands in the app log under the plugin's name |
| `settings_get` | the values the user set for this plugin |

Everything else — HTTP, timing, caching policy, retries — is host work.

## 5. Capabilities

- **lyrics-provider** — query (artist, title, album, duration) →
  synced/plain lines. Multiple installed; one active.
- **translation-provider** — (lines, target, kind: translate|romanize) →
  per-line outputs. Multiple installed; one active per kind.
- **commands** — additive and namespaced (`romanize.play`); many allowed.
- **settings** — schema-driven entries rendered by the host on the
  plugin's page. Values live in the host's settings file.
- **storage** — namespaced KV, 50 MB quota; the plugin's cache lives
  here.

## 6. Clash resolution — when plugins overlap

1. **Data capabilities are single-active, user-arbitrated.** Installing a
   second lyrics provider never silently replaces the first: the panel
   and the Plugins page surface a picker ("Lyrics from: LRCLIB / …").
   The first installed stays active until the user changes it.
2. **Uninstall or disable of the active provider** falls back to the next
   installed by install order; with none, the feature degrades to its
   honest empty state ("No lyrics plugin active"), never an error.
3. **Commands are additive** and auto-namespaced by plugin id, so two
   plugins registering `play` still coexist (`foo.play`, `bar.play`).
4. **Settings and storage are namespaced** (`plugin.<id>.*`) — plugins
   cannot read or write each other's data, so they cannot clash there.
5. **Domains are additive** and per-plugin; overlaps are harmless.
6. Determinism rule: *the user is the only arbiter; the host never
   resolves clashes silently.*

## 7. Degree of change — what a plugin may and may not do

**May (v1)** — contribute data (lyrics, translations, romanization),
register commands, contribute schema-driven settings, use its own
storage, ask the host for domain-scoped HTTP, log.

**May (roadmap, additive)** — `panel`: schema-driven sidebar widget
trees (text, lists, avatars, buttons) fed by polling and `on_event`;
`websocket relay`: host-owned sockets streaming events into
`handle_event`; `secrets`: host-managed tokens for a plugin's own
service.

**May never** — draw UI directly or replace core views; touch the
playback engine or the Spotify session; access the filesystem or the
network directly; spawn processes; read other plugins' storage or
settings; change the theme beyond schema tokens the host exposes; run
in the background or outlive the app; ship native code.

The line to remember: *plugins decorate Woofer; they never become
Woofer.* The host stays fully functional with zero plugins installed.

## 8. Keeping the app unbreakable — failure isolation

Layers, outermost first:

1. **Sandbox**: wasmi isolation — no syscalls, no shared memory. A
   plugin trap is contained by construction.
2. **Resources**: 64 MB linear memory cap, fuel metering per call, a 2 s
   wall-clock deadline per call, a 5 MB response-size cap on host
   fetches, a 50 MB storage quota. A runaway plugin is slowed, capped,
   then killed — never the app.
3. **Error domains**: every plugin call returns a Result at the ABI.
   Trap, timeout, `Err`, or OOM is the same thing to the host: *this
   plugin failed this call*.
4. **Degradation ladder**: one failed call → that capability quietly
   falls back (next provider, or the honest empty state). Three
   consecutive failures → the plugin is auto-disabled with a toast and
   marked "crashed" on its Plugins page; the user can re-enable. This
   mirrors the app's existing `Loadable::Failed` pattern.
5. **Startup**: instantiation is lazy — a bad plugin cannot slow or
   break launch; the app starts, and plugins come up when first needed.
6. **Updates**: a plugin update re-verifies sha256; the previous wasm is
   kept for one version so an update can roll back.
7. **Catalog**: approval-only merges, manifest schema CI, a source link
   required, sha256 pinned — by the time a plugin reaches users it has
   been read once by a human.

## 9. Marketplace (website) and in-app management

- **Catalog repo** `woofer-plugins`: `plugins/<slug>/{manifest.json,
  plugin.wasm, icon, README}`. Approval = reviewed PR merge; CI
  validates the schema, the digests, and the api version, and
  regenerates `registry.json`.
- **Website**: static, generated by CI, hosted on GitHub Pages
  (`kreatzzz.github.io/woofer-plugins`; a custom domain is a later,
  5-minute change). Catalog page + per-plugin pages: description,
  publisher, version, declared domains, install, source. "Open in
  Woofer" uses the `woofer://install?plugin=&v=&sig=` deep link; the
  scheme is registered by the packages already shipped (Info.plist,
  .desktop, Inno Setup) and lands through the existing single-instance
  socket into a confirmation dialog.
- **In-app** (Settings → Plugins): list with enable/disable (instant),
  delete (wasm + storage), visit link, version, publisher, domains;
  "Install from file…" for sideloading; per-capability active pickers.

## 10. The first plugins

- **Romanize** (`translation-provider:romanize`): per-line `dt=rm`
  requests, ASCII lines skipped, identity results discarded.
- **Translate** (`translation-provider:translate`): newline-batched
  `dt=t`, chunked to the URL budget, source==target skip.
- Both ship **pre-installed and disable-able**; both keep the existing
  disk-cache discipline in plugin storage. Lyrics (LRCLIB) follows as
  the third plugin, completing the dogfood of every v1 capability.
