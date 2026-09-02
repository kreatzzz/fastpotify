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
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use wasmi::{Config, Engine, Module};

use crate::plugins::{ABI_VERSION, PROVIDER_CAPABILITY, PluginManifest};
use crate::translate::Translation;

/// The compute a plugin run may burn, in wasmi fuel: enough for a normal
/// JSON request, while keeping an accidental infinite loop bounded even if
/// the runtime deadline cannot interrupt an individual wasm call.
const PLUGIN_FUEL: u64 = 200_000_000;

/// The linear memory a plugin may grow to, from the design: a runaway
/// allocation fails the run, never the app.
const PLUGIN_MEMORY: usize = 64 * 1024 * 1024;

/// The wall clock a whole run may take, fuel and fetches included. The
/// blocking worker also receives this deadline so a timed-out future cannot
/// leave a request retrying for minutes in the background.
const RUN_DEADLINE: Duration = Duration::from_secs(10);

/// The room one host fetch may hand the plugin, from the design: a body
/// bigger than this is an answer nobody asked for.
const MAX_RESPONSE: usize = 5 * 1024 * 1024;
/// Keep compilation itself bounded before wasmi has to inspect a module.
const MAX_WASM: usize = 64 * 1024 * 1024;

/// A provider cannot make an unbounded request fan-out or smuggle a huge
/// URL through the host before the response cap gets a chance to apply.
const MAX_REQUESTS: usize = 64;
/// Planned requests are independent, but keeping a small upper bound avoids
/// turning one provider run into an unbounded connection burst.
const MAX_CONCURRENT_REQUESTS: usize = 5;
const MAX_URL: usize = 16 * 1024;
const MAX_ABI_BUFFER: usize = PLUGIN_MEMORY;
const MAX_LYRICS_LINES: usize = 10_000;
const MAX_LYRIC_TEXT: usize = 1024 * 1024;

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
/// `Sync`, so one compile serves every run; the content digest, rather than
/// a cheap length check, distinguishes a replacement with the same size.
type Modules = HashMap<String, ([u8; 32], Arc<Module>)>;

static MODULES: OnceLock<Mutex<Modules>> = OnceLock::new();

/// The compiled module for `id`, from the cache when the bytes have not
/// changed, freshly compiled when they have or when the last compile
/// failed — a module that cannot load is simply not kept.
fn module(wasm: &[u8], id: &str) -> Result<Arc<Module>, String> {
    if wasm.len() > MAX_WASM {
        return Err(format!(
            "the plugin is {} bytes; the host allows at most {MAX_WASM}",
            wasm.len()
        ));
    }
    let digest: [u8; 32] = Sha256::digest(wasm).into();
    let modules = MODULES.get_or_init(Mutex::default);
    let modules = modules
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((cached, module)) = modules.get(id)
        && *cached == digest
    {
        return Ok(Arc::clone(module));
    }
    drop(modules);
    let module = Arc::new(
        Module::new(engine(), &mut &wasm[..])
            .map_err(|error| format!("the plugin {id} is not a loadable module: {error}"))?,
    );
    let mut modules = MODULES
        .get()
        .expect("the module cache was initialized above")
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    // Another thread may have compiled the same bytes while this one was
    // outside the lock. Reuse that winner instead of replacing it.
    if let Some((cached, existing)) = modules.get(id)
        && *cached == digest
    {
        return Ok(Arc::clone(existing));
    }
    modules.insert(id.to_string(), (digest, Arc::clone(&module)));
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
    off_thread(move |cancelled, deadline| {
        let input = serde_json::json!({ "kind": kind, "target": target, "lines": lines });
        let output = run_provider(&wasm, &manifest, &kind, &input, cancelled, deadline)?;
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
    off_thread(move |cancelled, deadline| {
        let output = run_provider(&wasm, &manifest, "lyrics", &input, cancelled, deadline)?;
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
/// A cancellation bit is shared with the worker because Tokio cannot abort a
/// `spawn_blocking` closure once it has started. The worker checks it between
/// wasm calls and HTTP attempts, while fuel bounds a call already in wasm.
async fn off_thread<T: Send + 'static>(
    run: impl FnOnce(&AtomicBool, Instant) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = Arc::clone(&cancelled);
    let deadline = Instant::now() + RUN_DEADLINE;
    let task = tokio::task::spawn_blocking(move || run(&worker_cancelled, deadline));
    match tokio::time::timeout(RUN_DEADLINE, task).await {
        Ok(Ok(answer)) => answer,
        Ok(Err(error)) => Err(format!("the plugin task died: {error}")),
        Err(_) => {
            cancelled.store(true, Ordering::Release);
            Err("the plugin missed its deadline".to_string())
        }
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
    cancelled: &AtomicBool,
    deadline: Instant,
) -> Result<serde_json::Value, String> {
    check_deadline(cancelled, deadline)?;
    let module = module(wasm, &manifest.id)?;
    let mut machine = machine(&module)?;
    required_exports(&machine)?;
    let declared = declared_manifest(&mut machine)?;
    if declared != *manifest {
        return Err("the module's manifest differs from the installed manifest".to_string());
    }
    let capability = PluginManifest::provider_capability(kind);
    if !declared
        .capabilities
        .iter()
        .any(|claimed| claimed.as_str() == capability)
    {
        return Err(format!("the plugin claims no {capability} capability"));
    }
    check_deadline(cancelled, deadline)?;
    let planned: Plan = serde_json::from_value(call_json(&mut machine, "plan", input, None)?)
        .map_err(|error| format!("the plugin's plan is not the JSON the ABI wants: {error}"))?;
    if planned.requests.len() > MAX_REQUESTS {
        return Err(format!(
            "the plugin planned {} requests; the host allows at most {MAX_REQUESTS}",
            planned.requests.len()
        ));
    }
    let urls: Vec<String> = planned
        .requests
        .into_iter()
        .map(|request| request.url)
        .collect();
    for url in &urls {
        if url.len() > MAX_URL {
            return Err(format!(
                "the plugin asked for a {size}-byte URL; the host allows at most {MAX_URL}",
                size = url.len()
            ));
        }
        ensure_allowed(url, manifest)?;
    }
    let what = format!("the plugin {}", manifest.id);
    check_deadline(cancelled, deadline)?;
    // A translation/romanization song commonly plans one request per line.
    // Fetch those independent requests in small batches, while collecting
    // answers in the planner's order so fulfil sees the same ABI shape as it
    // would for a sequential host.
    let responses = bounded_parallel(&urls, MAX_CONCURRENT_REQUESTS, |url| {
        let response = fetch_one(url, &what, cancelled, deadline);
        if response.is_err() {
            // A failed request is a failed run. Tell sibling workers to stop
            // retrying as soon as the first one reports that failure.
            cancelled.store(true, Ordering::Release);
        }
        response
    })?;
    let answers = serde_json::json!({ "responses": responses });
    check_deadline(cancelled, deadline)?;
    call_json(&mut machine, "fulfil", input, Some(&answers))
}

/// Loads a plugin far enough to hear what it is: the ABI version and the
/// manifest it declares, validated against this host. Used when a plugin
/// is installed, so nothing lands on disk before it has answered for
/// itself.
pub fn validate(wasm: &[u8]) -> Result<PluginManifest, String> {
    if wasm.len() > MAX_WASM {
        return Err(format!(
            "the plugin is {} bytes; the host allows at most {MAX_WASM}",
            wasm.len()
        ));
    }
    let module = Module::new(engine(), &mut &wasm[..])
        .map_err(|error| format!("the plugin is not a loadable module: {error}"))?;
    let mut machine = machine(&module)?;
    required_exports(&machine)?;
    let declared = declared_manifest(&mut machine)?;
    if !declared.capabilities.iter().any(|claimed| {
        claimed
            .strip_prefix(PROVIDER_CAPABILITY)
            .is_some_and(|kind| !kind.is_empty())
    }) {
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

/// Checks the complete v1 export surface before a module is accepted. Doing
/// this at install time turns a missing `plan` or `fulfil` into a clear
/// rejection instead of discovering it halfway through a user's first run.
fn required_exports(machine: &Machine) -> Result<(), String> {
    memory(machine)?;
    for (name, result) in [
        (
            "alloc",
            machine
                .instance
                .get_typed_func::<i32, i32>(&machine.store, "alloc")
                .map(|_| ()),
        ),
        (
            "dealloc",
            machine
                .instance
                .get_typed_func::<(i32, i32), ()>(&machine.store, "dealloc")
                .map(|_| ()),
        ),
        (
            "abi_version",
            machine
                .instance
                .get_typed_func::<(), i32>(&machine.store, "abi_version")
                .map(|_| ()),
        ),
        (
            "manifest",
            machine
                .instance
                .get_typed_func::<(), i64>(&machine.store, "manifest")
                .map(|_| ()),
        ),
        (
            "plan",
            machine
                .instance
                .get_typed_func::<(i32, i32), i64>(&machine.store, "plan")
                .map(|_| ()),
        ),
        (
            "fulfil",
            machine
                .instance
                .get_typed_func::<(i32, i32, i32, i32), i64>(&machine.store, "fulfil")
                .map(|_| ()),
        ),
    ] {
        result.map_err(|error| format!("the plugin exports no callable {name}: {error}"))?;
    }
    Ok(())
}

/// Returns a stable failure for a run whose worker was cancelled or whose
/// shared wall-clock budget has expired. Both checks are needed: a timeout
/// future may set the bit while the worker is between two clock reads.
fn check_deadline(cancelled: &AtomicBool, deadline: Instant) -> Result<(), String> {
    if cancelled.load(Ordering::Acquire) || Instant::now() >= deadline {
        Err("the plugin missed its deadline".to_string())
    } else {
        Ok(())
    }
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
    if payload.len() > MAX_ABI_BUFFER || payload.len() > i32::MAX as usize {
        return Err(format!(
            "the request for the plugin is {} bytes; the host allows at most {MAX_ABI_BUFFER}",
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
    if ptr <= 0 {
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
            if extra.len() > MAX_ABI_BUFFER || extra.len() > i32::MAX as usize {
                return Err(format!(
                    "the answers for the plugin are {} bytes; the host allows at most {MAX_ABI_BUFFER}",
                    extra.len()
                ));
            }
            let (extra_ptr, extra_len) = if extra.is_empty() {
                (0, 0)
            } else {
                let ptr = alloc
                    .call(&mut machine.store, extra.len() as i32)
                    .map_err(|error| format!("the plugin trapped in alloc: {error}"))?;
                if ptr <= 0 {
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
    if ptr == 0 || ptr > i32::MAX as u32 {
        return Err("the plugin answered with an invalid pointer".to_string());
    }
    if len as usize > MAX_ABI_BUFFER {
        return Err(format!(
            "the plugin answered with {len} bytes; the sandbox allows {MAX_ABI_BUFFER}"
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
    if parsed.username() != "" || parsed.password().is_some() {
        return Err("the plugin asked for a URL carrying user credentials".to_string());
    }
    if parsed.port().is_some_and(|port| port != 443) {
        return Err(format!(
            "the plugin asked for {url:?} on a non-HTTPS port; only port 443 is allowed"
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
            // A provider is allowed to fetch only the host it declared. A
            // redirect could otherwise move the request to an unallowlisted
            // host after this check, so redirects stay explicit failures.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("unable to build the plugin fetch client")
    })
}

/// Runs independent planned work in bounded batches and returns each answer
/// in input order. Scoped threads let the blocking HTTP client run without
/// occupying a Tokio worker, while the batch boundary keeps the maximum
/// number of in-flight requests explicit and cheap to reason about.
fn bounded_parallel<T, U, F>(items: &[T], max_parallel: usize, task: F) -> Result<Vec<U>, String>
where
    T: Sync,
    U: Send,
    F: Fn(&T) -> Result<U, String> + Sync,
{
    let max_parallel = max_parallel.max(1);
    let mut answers = Vec::with_capacity(items.len());
    for batch in items.chunks(max_parallel) {
        let batch_answers = std::thread::scope(|scope| {
            let handles: Vec<_> = batch
                .iter()
                .map(|item| scope.spawn(|| task(item)))
                .collect();
            handles
                .into_iter()
                .map(|handle| {
                    handle
                        .join()
                        .map_err(|_| "the plugin request worker panicked".to_string())?
                })
                .collect::<Result<Vec<_>, String>>()
        })?;
        answers.extend(batch_answers);
    }
    Ok(answers)
}

/// One GET, tried again on a fault the way the built-in translator tries:
/// a little later each time, with a dash of randomness so concurrent runs
/// do not knock together, and for as long as the server's own `Retry-After`
/// asks when it sends one. The plugin is the one who decides what an
/// answered refusal means, so every status that arrives is handed over —
/// only a request that never gets an answer is tried again.
fn fetch_retrying(
    url: &str,
    what: &str,
    cancelled: &AtomicBool,
    deadline: Instant,
) -> Result<reqwest::blocking::Response, String> {
    use crate::translate::{ATTEMPTS, FIRST_RETRY, MAX_RETRY};
    let mut wait = FIRST_RETRY;
    for attempt in 0..ATTEMPTS {
        check_deadline(cancelled, deadline)?;
        let asked;
        let remaining = deadline.saturating_duration_since(Instant::now());
        match http().get(url).timeout(remaining).send() {
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
        let pause = pause.min(deadline.saturating_duration_since(Instant::now()));
        std::thread::sleep(pause);
        wait = (wait * 2).min(MAX_RETRY);
    }
    unreachable!("the loop returns or errors on its last attempt")
}

/// Fetches one planned request and reads the answer within the response
/// cap, as the `{"status", "body"}` the ABI hands the plugin.
fn fetch_one(
    url: &str,
    what: &str,
    cancelled: &AtomicBool,
    deadline: Instant,
) -> Result<PluginResponse, String> {
    let response = fetch_retrying(url, what, cancelled, deadline)?;
    let status = response.status().as_u16();
    if let Some(length) = response.content_length()
        && usize::try_from(length).is_ok_and(|length| length > MAX_RESPONSE)
    {
        return Err(format!(
            "{what} offered {length} bytes; the host fetches at most {MAX_RESPONSE}"
        ));
    }
    // Read at most one byte past the response cap. `Response::bytes()` would
    // buffer an untrusted body in full before the size check could run.
    let mut body = Vec::with_capacity(MAX_RESPONSE.min(64 * 1024));
    response
        .take((MAX_RESPONSE + 1) as u64)
        .read_to_end(&mut body)
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
    if found.lines.len() > MAX_LYRICS_LINES {
        return Err(format!(
            "the plugin answered with {} lyric lines; the host allows at most {MAX_LYRICS_LINES}",
            found.lines.len()
        ));
    }
    let lines: Vec<crate::lyrics::Line> = found
        .lines
        .into_iter()
        .map(|line| {
            if line.text.len() > MAX_LYRIC_TEXT {
                return Err(format!(
                    "the plugin answered with a {}-byte lyric line; the host allows at most {MAX_LYRIC_TEXT}",
                    line.text.len()
                ));
            }
            Ok(crate::lyrics::Line {
                at_ms: line.at_ms,
                text: line.text,
            })
        })
        .collect::<Result<_, String>>()?;
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
    fn credentials_and_nonstandard_ports_are_not_allowed() {
        let manifest = manifest(&["clients5.google.com"]);
        assert!(ensure_allowed("https://user:pass@clients5.google.com/x", &manifest).is_err());
        assert!(ensure_allowed("https://clients5.google.com:8443/x", &manifest).is_err());
        assert!(ensure_allowed("https://clients5.google.com:443/x", &manifest).is_ok());
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

    #[test]
    fn planned_work_is_bounded_and_answers_keep_their_order() {
        use std::sync::atomic::AtomicUsize;

        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let items: Vec<usize> = (0..12).collect();
        let answers = bounded_parallel(&items, MAX_CONCURRENT_REQUESTS, |item| {
            let active_now = active.fetch_add(1, Ordering::AcqRel) + 1;
            peak.fetch_max(active_now, Ordering::AcqRel);
            // Sleeping makes overlap observable even on a lightly loaded
            // single-core CI runner without involving the network.
            std::thread::sleep(Duration::from_millis(10));
            active.fetch_sub(1, Ordering::AcqRel);
            Ok(item * 2)
        })
        .expect("all bounded work should answer");

        assert_eq!(answers, (0..12).map(|item| item * 2).collect::<Vec<_>>());
        assert!(peak.load(Ordering::Acquire) > 1, "work did not overlap");
        assert!(
            peak.load(Ordering::Acquire) <= MAX_CONCURRENT_REQUESTS,
            "peak exceeded the request bound"
        );
    }

    #[tokio::test]
    async fn a_module_that_cannot_load_fails_its_run_without_a_panic() {
        let manifest = PluginManifest::parse(
            r#"{"id":"acme","name":"Acme","publisher":"kreatzzz","version":"1.0.0","api":1,
                "capabilities":["provider:translate"],"domains":["clients5.google.com"]}"#,
        )
        .unwrap();
        // Placeholder bytes are not wasm; whatever they ever are, the run
        // must end in an error the host can fall back on.
        let error = run_translation(b"placeholder", &manifest, "translate", "en", &["hello"])
            .await
            .unwrap_err();
        assert!(!error.is_empty());
    }
}
