# AGENTS.md — the handbook for anyone (human or model) working on Woofer

Woofer is a fork of [crmne/fastpotify](https://github.com/crmne/fastpotify)
(fastpotify.rocks upstream): a fast, native Spotify client in Rust + egui,
playing through librespot. The fork exists to carry a plugin system, a
marketplace, and a UI direction the upstream maintainer does not want. Read
`docs/dev/decisions.md` for every decision and its date, and
`docs/plugins.md` for the plugin architecture.

## The ground rules

- **Style**: lowercase narrative comments that earn their place, ending in a
  period; imperative capitalized commit subjects ("Count the fork's own work
  as 0.3.0"); tests live in the same file, in the same voice.
- **Before finishing anything**: `cargo fmt`, `cargo clippy --all-targets
  --features demo -- -D warnings`, and `cargo test` (197 tests + 1 ignored
  live-network test) all green.
- **UI verification** is by screenshot:
  `cargo run --release --features demo -- --demo --demo-page <page> --demo-shot out.png`
  then look at the PNG.
- **Never commit without being asked.** Never push to `upstream`; `origin`
  (kreatzzz/woofer) is ours, `upstream` (crmne/fastpotify) is theirs.

## Where things live

| Path | What it is |
| --- | --- |
| `src/app.rs` | The `App`: state, `Action` handling, `Event` handling, frame loop |
| `src/backend.rs` | Tokio worker thread; `Command`s in, `Event`s out; all network |
| `src/player.rs` | The librespot engine (playback, Connect) |
| `src/lyrics.rs` | LRCLIB lyrics: fetch, match, LRC parse, 30-day disk cache |
| `src/translate.rs` | Built-in translator (keyless Google `clients5` endpoint), Lingva last-resort, the retry/backoff helper, and the two narrow entry points `fetch_translation_only` / `fetch_romanization_only` the plugin fallback uses |
| `src/plugins/mod.rs` | Plugin manifest model, the `provider:` taxonomy, the two **bundled** plugins (wasm via `include_bytes!` + frozen sidecar manifests) |
| `src/plugins/host.rs` | The wasmi sandbox: fuel, memory cap, the two-step `plan`/`fulfil` ABI, the `{"miss":true}` convention, domain-enforced fetching |
| `src/plugins/manager.rs` | Installed vs bundled plugins, **provider chains** (per-kind ordered fallback lists), install/remove, cache keys |
| `src/plugins/catalog.rs` | The `woofer://install` resolver: registry fetch, slug lookup, sha256 check |
| `src/ui/` | One file per page/panel; `theme.rs` is the design system (Palette, Lucide icons, buttons) |
| `src/settings.rs` | The one JSON settings file; `SessionState` for restore |
| `src/paths.rs` | Platform directories + the one-time `fastpotify` → `woofer` migration |
| `src/single_instance.rs` | The control socket: `woofer next`, `woofer nowplaying`, … |
| `src/demo.rs` | Sample data, `--demo-page/--demo-show/--demo-shot`, and the headless render test |
| `plugins/sdk` | `woofer-plugin-sdk`: `register_plugin!` macro + offline wasmi test harness |
| `plugins/translate`, `plugins/romanize` | The two official plugins, built to `assets/plugins/*.wasm` |
| `docs/plugins.md` | The whole plugin design (ABI, arities, sandbox limits, marketplace) |
| `docs/dev/decisions.md` | Every decision, dated |
| `docs/dev/release-plan.md` | How publishing works (Homebrew / AUR / winget) — currently halted |

## Commands

```bash
cargo run --release        # the real app
cargo test                 # the suite (79 tests)
cargo test --features demo # adds the headless render of every page
cargo test --lib plugins -- --ignored --nocapture   # LIVE Google round-trip through both plugins
```

Rebuilding the bundled plugins (only after editing `plugins/*`):

```bash
rustup target add wasm32-unknown-unknown
cd plugins/translate && cargo build --release --target wasm32-unknown-unknown
cp target/wasm32-unknown-unknown/release/woofer_plugin_translate.wasm ../../assets/plugins/translate.wasm
# same for romanize; both plugins' harness tests run with plain `cargo test` in their crate
```

## The current state (2026-08-30)

- `main` carries everything, now including the **upstream 0.4.0-rc1 merge**
  (equalizer, Winamp skin mode, RTL shaping, API gateway, dual-grant auth,
  playback resume) and on top of it: **provider chains** (ordered
  per-kind fallback lists, `provider:lyrics|translate|romanize`
  taxonomy, `{"miss":true}` ABI convention, lyrics-provider machinery —
  no lyrics plugin ships yet) and **`woofer://install` deep links**
  (registered by all three packages, resolved against
  usewoofer.com/registry.json, sha256-verified, confirm dialog before
  anything installs). Version is `0.4.0` in Cargo.toml but **nothing is
  released** — publishing is deliberately paused at the user's request.
- The marketplace site (`kreatzzz/woofer-plugins`, static) is pushed and
  ready; the user owns `usewoofer.com` and still needs to deploy on Vercel
  and point DNS (A `76.76.21.21`, `www` CNAME `cname.vercel-dns.com`).
- Two PRs are open upstream: #60 (one-line tokio `fs` feature — upstream
  `main` does not compile off Linux without it) and #61 (the whole
  translation feature). After the GitHub repo rename they remain valid.
- Upstream moves fast and often closes outside feature PRs to reimplement
  them; cherry-pick fixes from `upstream/main` weekly, but **never merge
  wholesale** without re-applying the tokio `fs` fix and our identity
  changes.

## The gotchas that cost someone time

- **Demo mode reads your real `session.json`** (`last_page` restores a real
  artist page and renders "Loading…"). Always pass `--demo-page` explicitly.
- **`assets/plugins/*.wasm` are committed build artifacts.** The sidecar
  manifests in `src/plugins/mod.rs` must match each module's own
  `manifest()` output — the test `the_bundled_plugins_load_and_say_who_they_are`
  enforces it (homepages point at the plugin repos, not the main repo).
- **wasmi is pinned to 0.31 in two places** (the app's Cargo.toml and the
  SDK's harness dev-dependency). Bump both together.
- **ABI v1**: exports `memory, alloc, dealloc, abi_version, manifest,
  plan, fulfil`; `fulfil` takes FOUR arguments (input buffer + responses
  buffer); strings cross as i64 `(ptr << 32) | len`; `fulfil` may answer
  `{"miss":true}` to pass a question to the next provider in the chain.
- **The Lingva fallback is unverified live** (both mirrors answered 500
  during the build; implemented against the documented shape, tested from
  canned JSON).
- `zh-CN` and `zh-TW` count as the same language in the source==target
  skip — a known limitation, flagged in tests.
- The release workflow has **no `workflow_dispatch`** — only a `v*` tag
  push triggers it, and on this fork the tag push once failed to trigger
  anything; check the Actions tab after tagging.
- The docs folder is the upstream Jekyll site (its `CNAME` still says
  `fastpotify.rocks`); it is not deployed for the fork.
- In zsh, `$VAR` does not word-split, and `sed` via `xargs` hangs on empty
  input (reads stdin) — pipe through `grep … | xargs …` carefully.

## What is next (agreed order)

1. Publishing when the user says go: tag `v0.3.0` → verify the run →
   Homebrew tap with real hashes → AUR PKGBUILDs → winget manifests.
   Details in `docs/dev/release-plan.md`.
2. Vercel deploy + DNS for usewoofer.com (user side, ~5 min).
3. Upstream: follow up on PRs #60/#61.
4. Plugin v1.5: the `panel` capability (widget vocabulary is drafted in
   `docs/plugins.md` §6), degradation ladder with auto-disable after three
   failures, `woofer://` deep-link installs.
