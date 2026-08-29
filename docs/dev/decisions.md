# Decisions log

Every decision worth remembering, newest last. Dates are 2026.

## The fork

**Fork from crmne/fastpotify, renamed to Woofer** (Aug 29). The upstream
maintainer closes outside feature PRs and reimplements the ideas in his own
style; the fork carries what he does not want: a plugin marketplace and the
translation feature. MIT license inherited; his name stays in the license
file. The GitHub repo was renamed `kreatzzz/fastpotify` → `kreatzzz/woofer`
(old URLs redirect; the two open upstream PRs remain valid).

**Identity**: bundle id `me.kreatzzz.woofer`, binary `woofer`, settings in
`~/.config/woofer` (macOS: `~/Library/Application Support/me.kreatzzz.woofer`).
`src/paths.rs` migrates the old `fastpotify` directories on first launch —
rename when possible, cross-volume copy as fallback, tested.

**The Web API application is ours.** `DEFAULT_WEB_CLIENT_ID`
(`src/auth.rs`) is `1f3066de96254093865b614999a5847e`, registered at
developer.spotify.com (Web API only; redirect `http://127.0.0.1:8989/login`).
The playback grant still uses Spotify's own desktop identity
(`PLAYBACK_CLIENT_ID`) — that one is not ours to replace. The Settings
override stays.

**macOS ships unsigned** until the user buys an Apple Developer
subscription; the release workflow handles both (secrets absent → unsigned
DMG; users right-click → Open).

**Upstream policy**: weekly fetch, cherry-pick fixes, keep sending small
fixes upstream. Never merge wholesale: upstream `main` does not compile
off Linux (missing tokio `fs` feature — our one-line PR #60 fixes it), and
a wholesale merge would drag in their naming.

## The lyrics features

**Translation + romanization in the lyrics panel** (before plugins existed;
now the plugins' job, with the built-in as fallback): keyless Google
endpoint `clients5.google.com/translate_a/single` (`client=dict-chrome-ex`,
`dt=t&dt=rm`) — no account, no key. Verified facts that shaped the code:
translation preserves newlines across response segments (batchable);
romanization merges lines (per-line requests only, ≤5 in flight);
`response[2]` is the detected source language; source==target skips
everything. Disk cache, 30 days, keyed by target+lines; retries with
backoff, `Retry-After` honored, jitter. Lingva is the last-resort
translation fallback (unverified live — both mirrors 500ed during the
build).

**Romanize replaces the main line; translation echoes under it.** Toggles
live in the panel header and in Settings → Lyrics; the language picker
defaults to English.

## The plugin system (v1 shipped)

**wasmi, not wasmtime**: an interpreter (~2-5 MB) over a JIT (20-40 MB);
the workloads are I/O-bound text, so JIT speed is wasted. Plugin modules
are `wasm32-unknown-unknown` with **no imports** — pure compute; the host
does every fetch itself and enforces each manifest's `domains`.

**The two-step request pattern**: `plan(input) -> {"requests":[{url}]}`
then the host fetches, then `fulfil(input, responses) -> output`. Plugins
are two pure functions; nothing async crosses the sandbox.

**ABI v1** (pinned; additive-only changes): exports `memory`, `alloc`,
`dealloc`, `abi_version`, `manifest`, `plan`, `fulfil`; `fulfil` takes
four arguments (input + responses buffers); strings packed as
`(ptr << 32) | len`; JSON payloads. Host limits: fuel 200M per run, 64 MB
memory, 5 MB per response, wall-clock backstop.

**Bundled plugins**: Translate and Romanize ship inside the binary
(`include_bytes!` from `assets/plugins/*.wasm`, committed build artifacts)
and are disable-able, so a fresh install works with zero setup. Their
sidecar manifests in `src/plugins/mod.rs` must match each module's own
`manifest()` — a test enforces it.

**Arities over conflict resolution**: data capabilities are single-active
and user-arbitrated (never a silent swap); commands and panels are
multi-active and namespaced; the host merges nothing silently. Plugins
never draw UI — they contribute data and schema-driven settings; the
v1.5 `panel` vocabulary is drafted in `docs/plugins.md` §6. Plugins
cannot call each other in v1.

**The catalog is approval-only and is a website, not an app surface**:
`kreatzzz/woofer-plugins` (git) → static site on Vercel at
**usewoofer.com** (domain bought Aug 29; DNS: apex A `76.76.21.21`, `www`
CNAME `cname.vercel-dns.com`). A PR merge is the review; `registry.json`
carries sha256-pinned wasm. In-app: the Plugins page (top-bar puzzle
icon) installs from URL or drag-and-drop, enables/disables, deletes,
visits source — no in-app browsing.

**Fallbacks**: plugin fails or is disabled → the built-in translator
(keyless Google) → Lingva (translation only). The app is fully functional
with zero plugins.

## Packaging

**Homebrew tap → AUR → winget, in that order**, after a tagged release.
The release workflow builds Linux x64+arm64 tarballs, Windows x64+arm64
(zip + Inno Setup installer), a macOS universal DMG, and checksums, on a
`v*` tag. See `docs/dev/release-plan.md` for the full runbook — currently
**halted at the user's request** (the `v0.3.0` tag was pushed and deleted
before anything built).
