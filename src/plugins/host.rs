//! The sandbox a plugin runs in.
//!
//! A plugin computes; the host does everything else. The module is compiled
//! once and instantiated per run inside wasmi, with fuel to burn and memory
//! to grow — and nothing else. Every fetch a plugin plans is made by the
//! host itself, only to domains the manifest allows, and the answers are
//! handed back for the plugin to read. A trap, an empty fuel tank, a missed
//! deadline, or a URL the manifest never allowed is one thing to the host:
//! this plugin failed this run.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use wasmi::{Config, Engine, Module};

use crate::plugins::{ABI_VERSION, PROVIDER_CAPABILITY, PluginManifest};
use crate::translate::Translation;

/// The compute a plugin run may burn, in wasmi fuel: enough for tens of
/// millions of instructions, so a module that plans and reads back a whole
/// song's worth of JSON fits with room to spare, and one that loops does
/// not.
const PLUGIN_FUEL: u64 = 200_000_000;

/// The linear memory a plugin may grow to, from the design: a runaway
/// allocation fails the run, never the app.
const PLUGIN_MEMORY: usize = 64 * 1024 * 1024;

/// The wall clock a whole run may take, fuel and fetches included. Fuel
/// already bounds the compute; this only catches a fetch chain gone
/// pathological, whose own requests time out far sooner than this.
const RUN_DEADLINE: Duration = Duration::from_secs(120);

/// The room one host fetch may hand the plugin, from the design: a body
/// bigger than this is an answer nobody asked for.
const MAX_RESPONSE: usize = 5 * 1024 * 1024;

/// The engine every plugin runs on, with fuel metering on so a runaway
/// module is slowed, capped, then stopped.
fn engine() -> &'static Engine {
    static ENGINE: OnceLock<Engine> = OnceLock::new();
    ENGINE.get_or_init(|| {
        let mut config = Config::default();
        config.consume_fuel(true);
        Engine::new(&config)
    })
}

/// Modules already compiled, by plugin id. wasmi's `Module` is `Send` and
/// `Sync`, so one compile serves every run; a reinstalled plugin with the
/// same id but new bytes is told apart by their length.
type Modules = HashMap<String, (usize, Arc<Module>)>;

static MODULES: OnceLock<Mutex<Modules>> = OnceLock::new();

/// The compiled module for `id`, from the cache when the bytes have not
/// changed, freshly compiled when they have or when the last compile
/// failed — a module that cannot load is simply not kept.
fn module(wasm: &[u8], id: &str) -> Result<Arc<Module>, String> {
    let modules = MODULES.get_or_init(Mutex::default);
    let mut modules = modules
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((len, module)) = modules.get(id)
        && *len == wasm.len()
    {
        return Ok(Arc::clone(module));
    }
    let module = Arc::new(
        Module::new(engine(), &mut &wasm[..])
            .map_err(|error| format!("the plugin {id} is not a loadable module: {error}"))?,
    );
    modules.insert(id.to_string(), (wasm.len(), Arc::clone(&module)));
    Ok(module)
}

/// What a live instance keeps: the store, whose data is only the resource
/// limits, and the module's instance in it.
struct Machine {
    store: wasmi::Store<Limits>,
    instance: wasmi::Instance,
}

#[derive(Default)]
struct Limits {
    limits: wasmi::StoreLimits,
}

fn machine(module: &Module) -> Result<Machine, String> {
    let mut store = wasmi::Store::new(
        engine(),
        Limits {
            limits: wasmi::StoreLimitsBuilder::new()
                .memory_size(PLUGIN_MEMORY)
                .build(),
        },
    );
    store.limiter(|limits: &mut Limits| &mut limits.limits);
    store
        .add_fuel(PLUGIN_FUEL)
        .map_err(|error| format!("the sandbox refused to stock fuel: {error}"))?;
    let linker = wasmi::Linker::<Limits>::new(engine());
    let instance = linker
        .instantiate(&mut store, module)
        .and_then(|pre| pre.start(&mut store))
        .map_err(|error| format!("the plugin failed to instantiate: {error}"))?;
    Ok(Machine { store, instance })
}

/// Runs a provider plugin over the lines: the plugin plans its requests,
/// the host fetches them itself, and the plugin reads them back. `Ok(None)`
/// is the miss the ABI defines — the plugin has nothing for this input —
/// and the host's error is everything else: a trap, an empty fuel tank, a
/// missed deadline, or a URL the manifest does not allow.
pub async fn run_translation(
    wasm: &[u8],
    manifest: &PluginManifest,
    kind: &str,
    target: &str,
    lines: &[&str],
) -> Result<Option<Translation>, String> {
    let wasm = wasm.to_vec();
    let manifest = manifest.clone();
    let kind = kind.to_string();
    let target = target.to_string();
    let lines: Vec<String> = lines.iter().map(|line| line.to_string()).collect();
    off_thread(move || {
        let input = serde_json::json!({ "kind": kind, "target": target, "lines": lines });
        let output = run_provider(&wasm, &manifest, &kind, &input)?;
        into_translation(output, input["lines"].as_array().map_or(0, Vec::len))
    })
    .await
}

/// The track a lyrics provider is asked about: artist, title, album, and
/// the length in milliseconds, `0` when nobody knows it.
pub async fn run_lyrics(
    wasm: &[u8],
    manifest: &PluginManifest,
    query: &crate::lyrics::Query,
) -> Result<Option<crate::lyrics::Lyrics>, String> {
    let wasm = wasm.to_vec();
    let manifest = manifest.clone();
    let input = lyrics_input(query);
    off_thread(move || {
        let output = run_provider(&wasm, &manifest, "lyrics", &input)?;
        into_lyrics(output)
    })
    .await
}

/// The one input shape every provider kind is asked with: `lines` carries
/// what the kind needs, and for lyrics that is artist, title, album, and
/// the duration in milliseconds, as strings — `""` for what is not known.
/// `target` is the language to aid into, always empty for lyrics.
pub(crate) fn lyrics_input(query: &crate::lyrics::Query) -> serde_json::Value {
    serde_json::json!({
        "kind": "lyrics",
        "target": "",
        "lines": [
            query.artist,
            query.title,
            query.album,
            if query.duration_ms > 0 {
                query.duration_ms.to_string()
            } else {
                String::new()
            }
        ]
    })
}

/// Takes a blocking run off the runtime's worker threads, under the one
/// wall-clock deadline a whole run — fuel and fetches included — may take.
async fn off_thread<T: Send + 'static>(
    run: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    match tokio::time::timeout(RUN_DEADLINE, tokio::task::spawn_blocking(run)).await {
        Ok(Ok(answer)) => answer,
        Ok(Err(error)) => Err(format!("the plugin task died: {error}")),
        Err(_) => Err("the plugin missed its deadline".to_string()),
    }
}

/// The shared spine of every provider run: prove what the module claims,
/// let it plan, fetch what it asked for, and let it read the answers back.
/// The output is the plugin's own JSON, whatever shape its kind speaks.
fn run_provider(
    wasm: &[u8],
    manifest: &PluginManifest,
    kind: &str,
    input: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let module = module(wasm, &manifest.id)?;
    let mut machine = machine(&module)?;
    let declared = declared_manifest(&mut machine)?;
    let capability = PluginManifest::provider_capability(kind);
    if !declared
        .capabilities
        .iter()
        .any(|claimed| claimed.as_str() == capability)
    {
        return Err(format!("the plugin claims no {capability} capability"));
    }
    let planned: Plan = serde_json::from_value(call_json(&mut machine, "plan", input, None)?)
        .map_err(|error| format!("the plugin's plan is not the JSON the ABI wants: {error}"))?;
    let urls: Vec<String> = planned
        .requests
        .into_iter()
        .map(|request| request.url)
        .collect();
    for url in &urls {
        ensure_allowed(url, manifest)?;
    }
    let what = format!("the plugin {}", manifest.id);
    let responses: Vec<PluginResponse> = urls
        .iter()
        .map(|url| fetch_one(url, &what))
        .collect::<Result<Vec<_>, String>>()?;
    let answers = serde_json::json!({ "responses": responses });
    call_json(&mut machine, "fulfil", input, Some(&answers))
}

/// Loads a plugin far enough to hear what it is: the ABI version and the
/// manifest it declares, validated against this host. Used when a plugin
/// is installed, so nothing lands on disk before it has answered for
/// itself.
pub fn validate(wasm: &[u8]) -> Result<PluginManifest, String> {
    let module = Module::new(engine(), &mut &wasm[..])
        .map_err(|error| format!("the plugin is not a loadable module: {error}"))?;
    let mut machine = machine(&module)?;
    let declared = declared_manifest(&mut machine)?;
    if !declared
        .capabilities
        .iter()
        .any(|claimed| claimed.starts_with(PROVIDER_CAPABILITY))
    {
        return Err("the plugin claims no provider capability".to_string());
    }
    Ok(declared)
}

/// The module's own answer for who it is. The manifest on disk may say one
/// thing and the wasm another; the wasm is what actually runs, so what it
/// declares is what gets checked.
fn declared_manifest(machine: &mut Machine) -> Result<PluginManifest, String> {
    let abi = machine
        .instance
        .get_typed_func::<(), i32>(&machine.store, "abi_version")
        .map_err(|error| format!("the plugin would not say which ABI it speaks: {error}"))?;
    let abi = abi
        .call(&mut machine.store, ())
        .map_err(|error| format!("the plugin trapped in abi_version: {error}"))?;
    if abi != ABI_VERSION {
        return Err(format!(
            "the plugin speaks ABI {abi}; this host speaks {ABI_VERSION}"
        ));
    }
    let declared = {
        let manifest = machine
            .instance
            .get_typed_func::<(), i64>(&machine.store, "manifest")
            .map_err(|error| format!("the plugin exports no callable manifest: {error}"))?;
        let packed = manifest
            .call(&mut machine.store, ())
            .map_err(|error| format!("the plugin trapped in manifest: {error}"))?;
        read_packed(machine, packed)?
    };
    let declared: PluginManifest = serde_json::from_slice(&declared)
        .map_err(|error| format!("the plugin's manifest is not the JSON the ABI wants: {error}"))?;
    declared
        .validate()
        .map_err(|error| format!("the plugin's manifest is unusable: {error}"))?;
    Ok(declared)
}

/// Writes `payload` into the plugin's memory and calls the export `name`
/// with it; the packed answer is read back out and read as JSON.
fn call_json(
    machine: &mut Machine,
    name: &str,
    payload: &serde_json::Value,
    extra: Option<&serde_json::Value>,
) -> Result<serde_json::Value, String> {
    let bytes = serde_json::to_vec(payload)
        .map_err(|error| format!("cannot encode the request for the plugin: {error}"))?;
    let extra_bytes = extra
        .map(|extra| {
            serde_json::to_vec(extra)
                .map_err(|error| format!("cannot encode the answers for the plugin: {error}"))
        })
        .transpose()?;
    let answer = call_packed(machine, name, &bytes, extra_bytes.as_deref())?;
    serde_json::from_slice(&answer).map_err(|error| {
        format!("the plugin's answer from {name} is not the JSON the ABI wants: {error}")
    })
}

fn call_packed(
    machine: &mut Machine,
    name: &str,
    payload: &[u8],
    extra: Option<&[u8]>,
) -> Result<Vec<u8>, String> {
    if payload.len() > i32::MAX as usize {
        return Err(format!(
            "the request for the plugin is {} bytes; no call carries that",
            payload.len()
        ));
    }
    let memory = memory(machine)?;
    let alloc = machine
        .instance
        .get_typed_func::<i32, i32>(&machine.store, "alloc")
        .map_err(|error| format!("the plugin exports no callable alloc: {error}"))?;
    let ptr = alloc
        .call(&mut machine.store, payload.len() as i32)
        .map_err(|error| format!("the plugin trapped in alloc: {error}"))?;
    if ptr < 0 {
        return Err("the plugin refused to allocate room for its request".to_string());
    }
    memory
        .write(&mut machine.store, ptr as usize, payload)
        .map_err(|error| format!("the plugin's memory would not take the request: {error}"))?;
    let answer = match extra {
        // A two-argument export: the whole call is the one buffer.
        None => machine
            .instance
            .get_typed_func::<(i32, i32), i64>(&machine.store, name)
            .map_err(|error| format!("the plugin exports no callable {name}: {error}"))?
            .call(&mut machine.store, (ptr, payload.len() as i32)),
        // The four-argument form carries a second buffer; an empty one
        // arrives as the zero pointer the ABI reads as nothing.
        Some(extra) => {
            let (extra_ptr, extra_len) = if extra.is_empty() {
                (0, 0)
            } else {
                let ptr = alloc
                    .call(&mut machine.store, extra.len() as i32)
                    .map_err(|error| format!("the plugin trapped in alloc: {error}"))?;
                if ptr < 0 {
                    return Err("the plugin refused to allocate room for its answers".to_string());
                }
                memory
                    .write(&mut machine.store, ptr as usize, extra)
                    .map_err(|error| {
                        format!("the plugin's memory would not take the answers: {error}")
                    })?;
                (ptr, extra.len() as i32)
            };
            machine
                .instance
                .get_typed_func::<(i32, i32, i32, i32), i64>(&machine.store, name)
                .map_err(|error| format!("the plugin exports no callable {name}: {error}"))?
                .call(
                    &mut machine.store,
                    (ptr, payload.len() as i32, extra_ptr, extra_len),
                )
        }
    }
    .map_err(|error| format!("the plugin trapped in {name}: {error}"))?;
    read_packed(machine, answer)
}

/// Unpacks the plugin's `(ptr, len)` answer and copies it out of its
/// memory, giving the room back afterwards when the plugin offers a way.
fn read_packed(machine: &mut Machine, packed: i64) -> Result<Vec<u8>, String> {
    let packed = packed as u64;
    let ptr = (packed >> 32) as u32;
    let len = packed as u32;
    if len as usize > PLUGIN_MEMORY {
        return Err(format!(
            "the plugin answered with {len} bytes; the sandbox allows {PLUGIN_MEMORY}"
        ));
    }
    let memory = memory(machine)?;
    let mut buffer = vec![0u8; len as usize];
    memory
        .read(&machine.store, ptr as usize, &mut buffer)
        .map_err(|error| format!("the plugin's answer lies outside its memory: {error}"))?;
    if let Ok(dealloc) = machine
        .instance
        .get_typed_func::<(i32, i32), ()>(&machine.store, "dealloc")
    {
        let _ = dealloc.call(&mut machine.store, (ptr as i32, len as i32));
    }
    Ok(buffer)
}

fn memory(machine: &Machine) -> Result<wasmi::Memory, String> {
    machine
        .instance
        .get_memory(&machine.store, "memory")
        .ok_or_else(|| "the plugin exports no memory".to_string())
}

/// Whether the plugin may ask for `url`: an https URL on a domain its
/// manifest allowlists, by exact host. Anything else fails the whole run —
/// the plugin can ask, the host decides.
fn ensure_allowed(url: &str, manifest: &PluginManifest) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|error| format!("the plugin asked for a malformed URL {url:?}: {error}"))?;
    if parsed.scheme() != "https" {
        return Err(format!(
            "the plugin asked for {url:?} over {}; only https is allowed",
            parsed.scheme()
        ));
    }
    let host = parsed
        .host_str()
        .unwrap_or_default()
        .trim_end_matches('.')
        .to_ascii_lowercase();
    let allowed = manifest.domains.iter().any(|domain| {
        domain
            .trim()
            .trim_end_matches('.')
            .eq_ignore_ascii_case(&host)
    });
    if !allowed {
        return Err(format!(
            "the plugin asked for {host:?}, which its manifest does not allow"
        ));
    }
    Ok(())
}

/// The client the host fetches a plugin's planned requests with: the app's
/// own identity, one timeout per request. The retries are the host's, and
/// the domains are the manifest's.
fn http() -> &'static reqwest::blocking::Client {
    static HTTP: OnceLock<reqwest::blocking::Client> = OnceLock::new();
    HTTP.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .user_agent(concat!("woofer/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(30))
            .build()
            .expect("unable to build the plugin fetch client")
    })
}

/// One GET, tried again on a fault the way the built-in translator tries:
/// a little later each time, with a dash of randomness so concurrent runs
/// do not knock together, and for as long as the server's own `Retry-After`
/// asks when it sends one. The plugin is the one who decides what an
/// answered refusal means, so every status that arrives is handed over —
/// only a request that never gets an answer is tried again.
fn fetch_retrying(url: &str, what: &str) -> Result<reqwest::blocking::Response, String> {
    use crate::translate::{ATTEMPTS, FIRST_RETRY, MAX_RETRY};
    let mut wait = FIRST_RETRY;
    for attempt in 0..ATTEMPTS {
        let asked;
        match http().get(url).send() {
            Ok(response) => {
                let status = response.status();
                if !status.is_server_error() || attempt + 1 == ATTEMPTS {
                    return Ok(response);
                }
                asked = response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.parse::<u64>().ok())
                    .map(Duration::from_secs);
                log::debug!("{what} answered {status}; trying again in {wait:?}");
            }
            Err(error) => {
                if attempt + 1 == ATTEMPTS {
                    return Err(format!("cannot reach {what}: {error}"));
                }
                asked = None;
                log::debug!("cannot reach {what} yet: {error}");
            }
        }
        let pause = asked.unwrap_or(wait) + Duration::from_millis(rand::random::<u64>() % 250);
        std::thread::sleep(pause);
        wait = (wait * 2).min(MAX_RETRY);
    }
    unreachable!("the loop returns or errors on its last attempt")
}

/// Fetches one planned request and reads the answer within the response
/// cap, as the `{"status", "body"}` the ABI hands the plugin.
fn fetch_one(url: &str, what: &str) -> Result<PluginResponse, String> {
    let response = fetch_retrying(url, what)?;
    let status = response.status().as_u16();
    if let Some(length) = response.content_length()
        && usize::try_from(length).is_ok_and(|length| length > MAX_RESPONSE)
    {
        return Err(format!(
            "{what} offered {length} bytes; the host fetches at most {MAX_RESPONSE}"
        ));
    }
    let body = response
        .bytes()
        .map_err(|error| format!("{what} would not hand over its answer: {error}"))?;
    if body.len() > MAX_RESPONSE {
        return Err(format!(
            "{what} offered {} bytes; the host fetches at most {MAX_RESPONSE}",
            body.len()
        ));
    }
    Ok(PluginResponse {
        status,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

/// The plugin's fulfil answer becomes per-line aids, aligned with the
/// song: a short answer pads with `None`, a long one is cut, an
/// `{"error": …}` is the failure it says it is, and a `{"miss": true}` is
/// the quiet "nothing for this input" the chain walks past.
fn into_translation(
    output: serde_json::Value,
    count: usize,
) -> Result<Option<Translation>, String> {
    if let Some(error) = output.get("error") {
        let message = error.as_str().unwrap_or("unknown error");
        return Err(format!("the plugin failed: {message}"));
    }
    if is_miss(&output) {
        return Ok(None);
    }
    let output: PluginOutput = serde_json::from_value(output)
        .map_err(|error| format!("the plugin's answer is not the JSON the ABI wants: {error}"))?;
    Ok(Some(Translation {
        romanized: aligned(output.romanized, count),
        translated: aligned(output.translated, count),
    }))
}

/// Whether the plugin answered the miss the ABI defines: a literal
/// `"miss": true`, "I have nothing for this input" — distinct from an
/// error, which means the plugin is broken, not empty.
fn is_miss(output: &serde_json::Value) -> bool {
    output
        .get("miss")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

/// The plugin's fulfil answer becomes lyrics for the track, or a miss when
/// it has none. An answer whose lines came back empty is a miss too: the
/// panel has nothing to draw from it, and the chain has somewhere to go.
fn into_lyrics(output: serde_json::Value) -> Result<Option<crate::lyrics::Lyrics>, String> {
    if let Some(error) = output.get("error") {
        let message = error.as_str().unwrap_or("unknown error");
        return Err(format!("the plugin failed: {message}"));
    }
    if is_miss(&output) {
        return Ok(None);
    }
    let output: LyricsOutput = serde_json::from_value(output)
        .map_err(|error| format!("the plugin's answer is not the JSON the ABI wants: {error}"))?;
    let Some(found) = output.lyrics else {
        return Err("the plugin's answer carries no lyrics".to_string());
    };
    let lines: Vec<crate::lyrics::Line> = found
        .lines
        .into_iter()
        .map(|line| crate::lyrics::Line {
            at_ms: line.at_ms,
            text: line.text,
        })
        .collect();
    if lines.is_empty() {
        return Ok(None);
    }
    // A synced answer with a line missing its stamp is not one the panel
    // can follow, so it reads as plain.
    let synced = found.synced && lines.iter().all(|line| line.at_ms.is_some());
    Ok(Some(crate::lyrics::Lyrics {
        lines,
        synced,
        instrumental: false,
    }))
}

fn aligned(mut lines: Vec<Option<String>>, count: usize) -> Vec<Option<String>> {
    lines.truncate(count);
    lines.resize(count, None);
    lines
}

#[derive(Deserialize)]
struct Plan {
    requests: Vec<PlannedRequest>,
}

#[derive(Deserialize)]
struct PlannedRequest {
    url: String,
}

#[derive(Serialize)]
struct PluginResponse {
    status: u16,
    body: String,
}

#[derive(Default, Deserialize)]
struct PluginOutput {
    #[serde(default)]
    romanized: Vec<Option<String>>,
    #[serde(default)]
    translated: Vec<Option<String>>,
}

#[derive(Deserialize)]
struct LyricsOutput {
    lyrics: Option<PluginLyrics>,
}

#[derive(Deserialize)]
struct PluginLyrics {
    #[serde(default)]
    synced: bool,
    #[serde(default)]
    lines: Vec<PluginLine>,
}

#[derive(Deserialize)]
struct PluginLine {
    at_ms: Option<u32>,
    text: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn manifest(domains: &[&str]) -> PluginManifest {
        PluginManifest {
            domains: domains.iter().map(|domain| domain.to_string()).collect(),
            ..PluginManifest::default()
        }
    }

    #[test]
    fn a_url_on_an_allowed_domain_passes() {
        let manifest = manifest(&["clients5.google.com"]);
        assert!(
            ensure_allowed(
                "https://clients5.google.com/translate_a/single?q=x",
                &manifest
            )
            .is_ok()
        );
        // Case and a trailing dot are spelling, not another domain.
        assert!(ensure_allowed("https://CLIENTS5.google.com./x", &manifest).is_ok());
    }

    #[test]
    fn a_foreign_domain_fails_the_run() {
        let manifest = manifest(&["clients5.google.com"]);
        let error =
            ensure_allowed("https://evil.example.com/steal?lyrics=1", &manifest).unwrap_err();
        assert!(error.contains("evil.example.com"), "{error}");
        // A lookalike subdomain is still another host.
        assert!(
            ensure_allowed("https://clients5.google.com.evil.example.com/x", &manifest).is_err()
        );
    }

    #[test]
    fn plaintext_and_nonsense_never_reach_the_wire() {
        let manifest = manifest(&["clients5.google.com"]);
        assert!(
            ensure_allowed("http://clients5.google.com/x", &manifest)
                .unwrap_err()
                .contains("only https")
        );
        assert!(ensure_allowed("not a url", &manifest).is_err());
    }

    #[test]
    fn a_fulfil_answer_lands_on_its_lines() {
        let found = into_translation(
            json!({"romanized": ["konnichiwa", null], "translated": ["hello", "world", "extra"]}),
            3,
        )
        .unwrap()
        .expect("an ordinary answer is data");
        assert_eq!(
            found.romanized,
            vec![Some("konnichiwa".to_string()), None, None]
        );
        assert_eq!(
            found.translated,
            vec![
                Some("hello".to_string()),
                Some("world".to_string()),
                Some("extra".to_string())
            ]
        );
        // And a song shorter than the answer cuts the tail off.
        let cut = into_translation(json!({"translated": ["hello", "world"]}), 1)
            .unwrap()
            .unwrap();
        assert_eq!(cut.translated, vec![Some("hello".to_string())]);
    }

    #[test]
    fn a_plugin_error_is_a_failure_and_nothing_else() {
        let error = into_translation(json!({"error": "the upstream refused"}), 2).unwrap_err();
        assert!(error.contains("the upstream refused"), "{error}");
        // Missing fields mean every line keeps the original.
        let found = into_translation(json!({}), 2).unwrap().unwrap();
        assert_eq!(found.romanized, vec![None, None]);
        assert_eq!(found.translated, vec![None, None]);
    }

    #[test]
    fn a_miss_is_no_data_not_a_failure() {
        assert_eq!(into_translation(json!({"miss": true}), 2).unwrap(), None);
        // Only the literal true reads as a miss; anything else the shape
        // parser judges.
        let found = into_translation(json!({"miss": false}), 2)
            .unwrap()
            .unwrap();
        assert_eq!(found.translated, vec![None, None]);
    }

    #[test]
    fn a_lyrics_answer_maps_into_the_panels_lyrics() {
        let found = into_lyrics(json!({
            "lyrics": {"synced": true, "lines": [
                {"at_ms": 12345, "text": "first"},
                {"at_ms": 67890, "text": "second"},
            ]}
        }))
        .unwrap()
        .expect("a full answer is lyrics");
        assert!(found.synced);
        assert_eq!(found.lines[0].at_ms, Some(12_345));
        assert_eq!(found.lines[1].text, "second");
    }

    #[test]
    fn a_lyrics_answer_that_cannot_be_followed_reads_as_plain_or_a_miss() {
        // A synced flag with unstamped lines is honest only as plain.
        let plain = into_lyrics(json!({
            "lyrics": {"synced": true, "lines": [{"at_ms": null, "text": "a"}]}
        }))
        .unwrap()
        .unwrap();
        assert!(!plain.synced);
        // No lines at all is a miss in a shape, and the chain walks past it.
        assert_eq!(
            into_lyrics(json!({"lyrics": {"synced": true, "lines": []}})).unwrap(),
            None
        );
        assert_eq!(into_lyrics(json!({"miss": true})).unwrap(), None);
        let error = into_lyrics(json!({"error": "no such track"})).unwrap_err();
        assert!(error.contains("no such track"), "{error}");
    }

    #[test]
    fn the_lyrics_input_names_the_track_in_strings() {
        let query = crate::lyrics::Query {
            artist: "Artist".into(),
            title: "Song".into(),
            album: String::new(),
            duration_ms: 201_000,
        };
        let input = lyrics_input(&query);
        assert_eq!(input["kind"], "lyrics");
        assert_eq!(input["target"], "");
        assert_eq!(
            input["lines"],
            json!(["Artist", "Song", "", "201000"]),
            "artist, title, album, duration — the unknowns as empty strings"
        );
        let unknown = lyrics_input(&crate::lyrics::Query {
            duration_ms: 0,
            ..query
        });
        assert_eq!(unknown["lines"][3], "");
    }

    #[tokio::test]
    async fn a_module_that_cannot_load_fails_its_run_without_a_panic() {
        let manifest =
            PluginManifest::parse(crate::plugins::BUNDLED.first().unwrap().manifest).unwrap();
        // Placeholder bytes are not wasm; whatever they ever are, the run
        // must end in an error the host can fall back on.
        let error = run_translation(b"placeholder", &manifest, "translate", "en", &["hello"])
            .await
            .unwrap_err();
        assert!(!error.is_empty());
    }
}
