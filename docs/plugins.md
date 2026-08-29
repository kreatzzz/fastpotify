# Woofer Plugin System — Design

Status: v1 shipped — the wasmi host, the SDK, and the bundled Translate
and Romanize plugins are live. The catalog is the `woofer-plugins`
repository, deployed on Vercel from it; its public address lands here
once the first deploy is done. Runtime: wasmi (wasm32-unknown-unknown),
pure compute, no imports.

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

A plugin is one `.wasm` module (`wasm32-unknown-unknown`, no imports)
plus a manifest:

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

## 6. Surfaces and their arities

Conflicts are prevented, not resolved after the fact: every extension
surface declares its **arity** — how many plugins may hold it, and who
breaks a tie — at API-design time.

| Arity | Meaning | Ties are broken by |
| --- | --- | --- |
| **single-active** | One plugin holds the slot; it answers a question | The user, via an explicit picker |
| **multi-active** | Many plugins coexist, each in a namespaced slot | The user, for order and visibility only |
| **merged** | Many contribute; the host merges by fixed policy | The host, deterministically (reserved; none in v1) |

### The catalog, by version

**v1 — data and control**

| Surface | Arity |
| --- | --- |
| Lyrics provider | single-active |
| Translation provider (translate) | single-active |
| Translation provider (romanize) | single-active |
| Commands (tray, palette, control-CLI verbs) | multi-active |
| Settings entries | additive, namespaced |
| Storage | private |

**v1.5 — UI surfaces and events**

| Surface | Arity |
| --- | --- |
| Sidebar panels | multi-active, stacked |
| Track context-menu items | multi-active |
| Player-bar extras | multi-active, fixed-size slots |
| `on_event` (now-playing changes) | multi-active |
| Polling scheduler | multi-active, host floor of 1 s |

**v2 — real-time and trust**

| Surface | Arity |
| --- | --- |
| Websocket relay (host-owned sockets) | multi-active |
| Secrets (host-managed tokens) | private |
| Now-playing "featured" widget | single-active |

### The right sidebar, conflict-free

The sidebar is not a slot to grab; it is a **stack of sections**, one per
`panel:sidebar` plugin, in the manner of a code editor's side dock:

- **Order** is the user's: drag to reorder, persisted; install order is
  the default. No plugin can fight for position.
- **Space** is guarded: sections collapse, and a collapsed section does
  not execute its plugin at all. Per-section caps — a widget-count
  budget and a nesting depth — keep five installed panels from making
  the interface sweat.
- **Visibility** is declarative: a plugin may say "only while something
  is playing", and the host evaluates the condition without running it.

### The `panel` widget vocabulary (v1.5, first cut)

Eleven widgets, each a JSON node; the host owns rendering, fonts,
spacing, and the image cache.

`row` · `column` · `list` · `text` (weight, size) · `badge` ·
`avatar` (URL, fetched and cached by the host's art loader) · `button`
(action lands back as a command or an event) · `text-input` ·
`progress` · `divider` · `spacer`

Hard limits: at most 200 widgets per section, nesting at most 5 deep,
strings bounded, images only through the host loader. A friends sidebar
(a list of rows with avatars, badges, and one button each) fits in
roughly 60 widgets for 20 friends — the vocabulary is sized so the
interesting plugins are expressible and the expensive ones are not.

### What gets built (the catalog that seeds the marketplace)

1. **Providers** — lyrics, translation (DeepL with the user's own key,
   LibreTranslate self-hosted), romanization.
2. **Zero-interface utilities** — Last.fm scrobbling, Discord rich
   presence, a sleep timer, lyric export: events, secrets, and HTTP
   with no UI at all.
3. **Enrichment cards** — song meanings, chords for the current track,
   concert alerts by polling.
4. **Panels** — a friends sidebar with status and "Listen along",
   language-learning flashcards over the translation data, listening
   statistics.
5. **Integrations** — commands for launchers and hotkey daemons through
   the control CLI.

One deliberate non-feature: plugins cannot call each other in v1. No
dependency graph, no cross-plugin API; a plugin that wants translated
data makes its own requests. Composition is a later question, refused
until the catalog demands it.

## 7. Clash resolution — when plugins overlap

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

## 8. Degree of change — what a plugin may and may not do

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

## 9. Keeping the app unbreakable — failure isolation

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

## 10. Marketplace (website) and in-app management

- **Catalog repo** `woofer-plugins`: `plugins/<slug>/{manifest.json,
  plugin.wasm, icon, README}`. Approval = reviewed PR merge; CI
  validates the schema, the digests, and the api version, and
  regenerates `registry.json`.
- **Website**: static, generated by CI, hosted on GitHub Pages
  (`woofer-plugins.vercel.app` once deployed; a custom domain is a later,
  5-minute change). Catalog page + per-plugin pages: description,
  publisher, version, declared domains, install, source. "Open in
  Woofer" uses the `woofer://install?plugin=&v=&sig=` deep link; the
  scheme is registered by the packages already shipped (Info.plist,
  .desktop, Inno Setup) and lands through the existing single-instance
  socket into a confirmation dialog.
- **In-app** (Settings → Plugins): list with enable/disable (instant),
  delete (wasm + storage), visit link, version, publisher, domains;
  "Install from file…" for sideloading; per-capability active pickers.

## 11. The first plugins

- **Romanize** (`translation-provider:romanize`): per-line `dt=rm`
  requests, ASCII lines skipped, identity results discarded.
- **Translate** (`translation-provider:translate`): newline-batched
  `dt=t`, chunked to the URL budget, source==target skip.
- Both ship **pre-installed and disable-able**; both keep the existing
  disk-cache discipline in plugin storage. Lyrics (LRCLIB) follows as
  the third plugin, completing the dogfood of every v1 capability.
