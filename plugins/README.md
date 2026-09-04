# Woofer plugins

A plugin is one `wasm32-unknown-unknown` module: it computes and asks; the
host fetches, retries, caches, and decides. Nothing here touches a socket,
a file, or the clock. The product overview lives in `docs/plugins.md`; the
complete ABI and sandbox contract lives in `docs/dev/plugin-architecture.md`.
This is the handbook for writing one.

    plugins/
      sdk/         woofer-plugin-sdk — the ABI, once, and the test harness
      translate/   provider:translate
      romanize/    provider:romanize

Every crate is standalone (an empty `[workspace]` table of its own), builds
its own module, and tests it on the same interpreter the host runs
(wasmi 0.31).

## The ABI, in one breath

Version 1: no imports, JSON everywhere, strings packed into one `i64` as
`(ptr as u64) << 32 | len as u64`. The module exports `memory`, `alloc`,
`dealloc`, `abi_version`, `manifest`, `plan`, and `fulfil`. `plan` gets
`{"kind":…,"target":…,"lines":[…]}` and answers `{"requests":[{"url":…}]}`.
`fulfil` gets the same input with the host's answers attached —
`"responses":[{"status":200,"body":"…"}]`, in plan order — and answers
`{"error":…}`, `{"miss":true}` ("I have nothing for this input"), or the
capability's own output. A handler's `Err` becomes `{"error":…}` at the
ABI, so the host has one failure shape, not two. Each kind asks its
providers in the user's order, and the first with data wins.

## Writing one

Create `plugins/<id>/` with an empty `[workspace]` table,
`[lib] crate-type = ["cdylib"]`, and:

```toml
[dependencies]
woofer-plugin-sdk = { path = "../sdk" }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

then declare the manifest and the two handlers, and let the macro wire the
rest:

```rust
const MANIFEST: &str = r#"{
    "id": "my-plugin",
    "name": "My Plugin",
    "publisher": "kreatzzz",
    "version": "1.0.0",
    "api": 1,
    "capabilities": ["provider:translate"],
    "domains": ["example.com"],
    "homepage": "https://github.com/kreatzzz/woofer-plugin-my-plugin"
}"#;

fn plan(input: &str) -> Result<String, String> { … }   // decide what to fetch
fn fulfil(input: &str) -> Result<String, String> { … } // parse what came back

woofer_plugin_sdk::register_plugin! {
    manifest = MANIFEST,
    plan = plan,
    fulfil = fulfil,
}
```

The macro generates every export and hides the packing, the allocator, and
the buffer dance. Memory is the standard allocator under a thin wrapper:
`alloc(len)` yields 16-byte-aligned room (sentinel `16` for empty, `0` for
out of memory); `dealloc` must see the same `len` the room was asked for.
The generated functions are named `woofer_alloc`, `woofer_plan`, … in Rust,
so they cannot collide with the handlers.

## Testing one

Take the SDK with its `harness` feature as a dev-dependency:

```toml
[dev-dependencies]
woofer-plugin-sdk = { path = "../sdk", features = ["harness"] }
```

The harness finds the release module under `target/wasm32-unknown-unknown/`
(cargo-builds it when it is missing, or takes `PLUGIN_WASM` from the
environment), loads it with wasmi, checks `abi_version`, and drives it
exactly like the host — offline, with canned answers:

```rust
let mut plugin = Plugin::from_local_artifact("woofer-plugin-my-plugin").unwrap();
let planned = plugin.plan(&input.to_string()).unwrap();
let answered = plugin.fulfil(&input.to_string(), &[Response { status: 200, body: … }]).unwrap();
```

## Building and submitting to the catalog

    cd plugins/<id>
    cargo test                                            # the harness suite
    cargo build --release --target wasm32-unknown-unknown

The resulting module stays in the crate's `target/` directory for local
testing. Woofer does not embed plugin modules in `assets/` or in the
application binary. The official Translate and Romanize modules are reviewed
and published separately in the catalog repository, under
`plugins/<id>/plugin.wasm`.

When submitting a catalog entry, keep the module manifest's
`capabilities` in the `provider:<kind>` form, give the catalog a matching
SHA-256 digest, and use absolute HTTPS URLs for the module and its source
link. The catalog CI verifies the manifest and regenerates `registry.json`;
the app then verifies that digest again before installing the module.
