# Woofer Plugin System — Design

Status: v1 shipped — the wasmi host, the SDK, and the first two plugins,
Translate and Romanize, published on the catalog at
[usewoofer.com](https://usewoofer.com). The app bundles nothing: the
built-in engines answer until a plugin is installed. Runtime: wasmi
(wasm32-unknown-unknown), pure compute, no imports.

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
  "capabilities": ["provider:romanize"],
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
| `plan(input) -> requests` | pure: decide WHAT to fetch |
| `fulfil(input, responses) -> output` | pure: parse the answers |
| `on_event(event) -> ()` | playback state changes (roadmap) |
| `command_run(id, context) -> result` | a registered command fired (roadmap) |

| Host function (import) | Purpose |
| --- | --- |
| `storage_get / storage_put` | per-plugin namespaced KV, quota-capped |
| `log(level, message)` | lands in the app log under the plugin's name |
| `settings_get` | the values the user set for this plugin |

Everything else — HTTP, timing, caching policy, retries — is host work.

### The one input shape

Every provider kind is asked with the same object, `plan` and `fulfil`
alike:

```json
{ "kind": "translate", "target": "en", "lines": ["…", "…"] }
```

- `kind` names the provider kind: `translate`, `romanize`, or `lyrics`.
- `target` is the language to aid into; always `""` for lyrics.
- `lines` carries what the kind needs. For translate and romanize it is
  the lyric lines. For lyrics it is exactly four strings — artist, title,
  album, and the track length in milliseconds — with `""` standing for
  anything the app does not know:

  ```json
  { "kind": "lyrics", "target": "", "lines": ["Artist", "Song", "Album", "201000"] }
  ```

`plan` answers `{"requests":[{"url":…}, …]}`. `fulfil` receives the same
input with the host's answers attached as
`"responses":[{"status":200,"body":"…"}]`, in plan order.

### The answer: data, miss, error

`fulfil` answers one of three ways, and the host treats each differently:

- **Data** — the capability's own output shape (below). The chain stops
  here.
- **Miss** — `{"miss":true}`: *"I have nothing for this input."* A miss is
  not a malfunction; it is an answer, and it moves the chain to the next
  provider exactly as data from a higher link already would.
- **Error** — `{"error":"…"}`, or a trap, an empty fuel tank, or a missed
  deadline: the plugin failed this call. Also moves the chain down, with a
  note in the log.

A handler returning `Err` becomes `{"error":…}` at the ABI, so the host
has one failure shape, not two.

### Output shapes, per kind

**`provider:translate` / `provider:romanize`** — per-line aids, aligned
with the input lines (short answers pad with `null`, long ones are cut):

```json
{ "translated": ["bonjour", null], "romanized": [null, null] }
```

**`provider:lyrics`** — either a miss, or `lyrics` with a `synced` flag
and lines whose `at_ms` is a millisecond stamp (synced) or `null` (plain):

```json
{ "lyrics": { "synced": true,
              "lines": [ { "at_ms": 12345, "text": "…" }, { "at_ms": null, "text": "…" } ] } }
```

A `synced` answer with any line missing its stamp reads as plain. An
answer with no lines reads as a miss. Lyrics hits are cached by the host
(30 days, keyed by plugin and track), misses included.

## 5. Capabilities

Data capabilities are named `provider:` plus the kind, and a plugin
claims one or more of them:

- **`provider:lyrics`** — (artist, title, album, duration) → synced or
  plain lines. Sits **behind** the built-ins: Spotify's own words and
  LRCLIB always answer first, and the lyrics chain only fills the gaps
  they leave.
- **`provider:translate`** — (lines, target) → per-line words.
- **`provider:romanize`** — (lines, target) → per-line Latin spellings.

The two built-in engines — the Google `clients5` translator in the app
and the built-in LRCLIB client — are not plugins and cannot be displaced:
they stand permanently behind the last link of their kinds' chains.
- **commands** — additive and namespaced (`romanize.play`); many allowed.
- **settings** — schema-driven entries rendered by the host on the
  plugin's page. Values live in the host's settings file.
- **storage** — namespaced KV, 50 MB quota; the plugin's cache lives
  here.

## 6. Chains — the arity of a provider

Every provider kind is an **ordered fallback chain**. The host walks a
kind's chain in order; the first provider that answers with data wins. A
miss or a failure advances to the next; when the chain runs out, the
kind's built-in engine answers — always the last resort, never asked
inside the chain.

Rules the chains obey:

- **Installing appends.** A new provider takes the back of every chain it
  can answer for. Installing never displaces a provider the user has
  already seated.
- **An empty chain is the built-ins' own.** A kind with no ordered chain
  runs on the built-in engines alone — complete on their own for every
  kind today. The built-ins wait behind the chain regardless.
- **Membership replaces enable/disable.** A provider answers only the
  kinds whose chains name it; removing it from a chain is the off switch.
- **Stale ids are skipped.** An id in a chain that no plugin claims — an
  uninstalled provider, a manifest gone bad — costs its slot nothing: the
  walk passes it and moves on.
- **Order is the user's.** Up and down buttons on the Plugins page swap
  adjacent links; the order persists in the settings file.

The chains are one setting, stored readably:

```json
{ "provider_chains": { "lyrics": [], "translate": ["deepl", "translate"], "romanize": ["romanize"] } }
```

### Surfaces and their arities

Conflicts are prevented, not resolved after the fact: every extension
surface declares its **arity** — how many plugins may hold it, and who
breaks a tie — at API-design time.

| Arity | Meaning | Ties are broken by |
| --- | --- | --- |
| **chain** | The kind asks each of its providers in the user's order, then the built-in | The user, via the ordering controls |
| **multi-active** | Many plugins coexist, each in a namespaced slot | The user, for order and visibility only |
| **merged** | Many contribute; the host merges by fixed policy | The host, deterministically (reserved; none in v1) |

### The catalog, by version

**v1 — data and control**

| Surface | Arity |
| --- | --- |
| Lyrics providers | chain, behind the built-in lyrics flow |
| Translation providers | chain, behind the built-in translator |
| Romanization providers | chain, behind the built-in romanizer |
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

1. **Data capabilities are chained, user-ordered.** Installing a second
   lyrics provider never displaces the first: it joins the back of the
   chain, and the Plugins page shows the order with controls to change
   it. The first installed keeps its seat until the user moves it.
2. **Uninstall or removal from a chain** falls back to the next link by
   order; with none, to the kind's built-in engine — the feature never
   degrades to an error, at worst to its honest empty state.
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
- **Website**: static, generated by CI, hosted on Vercel at
  [usewoofer.com](https://usewoofer.com). Catalog page + per-plugin
  pages: description, publisher, version, declared domains, install,
  source. "Open in
  Woofer" uses the `woofer://install?plugin=&v=&sig=` deep link; the
  scheme is registered by the packages already shipped (Info.plist,
  .desktop, Inno Setup) and lands through the existing single-instance
  socket into a confirmation dialog.
- **In-app** (Plugins page): each kind's chain with up/down ordering
  (instant, persisted), delete (wasm + its seat in every chain), visit
  link, version, publisher, domains; "Install from file…" for
  sideloading.

## 11. The first plugins

- **Romanize** (`provider:romanize`): per-line `dt=rm` requests, ASCII
  lines skipped, identity results discarded.
- **Translate** (`provider:translate`): newline-batched `dt=t`, chunked
  to the URL budget, source==target skip.
- Both are **published on the catalog**, installed separately from it
  (`woofer://install?plugin=translate` and `…plugin=romanize`); the
  built-in engines answer until then and whenever they are absent;
  both keep the existing disk-cache discipline in plugin storage. Lyrics
  (`provider:lyrics`) follows as the third plugin, taking the gaps the
  built-in lyrics flow leaves, and completing the dogfood of every v1
  capability.
