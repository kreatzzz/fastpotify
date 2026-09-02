//! Which plugins are installed, and the order each kind asks them in.
//!
//! The app ships none: plugins arrive from the catalog as files, and the
//! built-in engines — never listed here — stand behind every chain. Each
//! kind walks its own chain, in the order the user set, and the first
//! provider with data wins.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use crate::paths::AppDirs;
use crate::plugins::PluginManifest;
use crate::translate::Translation;
use sha2::{Digest, Sha256};

/// The on-disk sidecar carries the module's identity plus a content digest.
/// `sha256` is optional only for old installs from before integrity pinning;
/// newly installed plugins always write it and are checked against it.
#[derive(Default, Deserialize, Serialize)]
#[serde(default)]
struct StoredManifest {
    #[serde(flatten)]
    manifest: PluginManifest,
    sha256: Option<String>,
}

fn parse_stored_manifest(text: &str) -> Result<StoredManifest, String> {
    let stored: StoredManifest =
        serde_json::from_str(text).map_err(|error| format!("unreadable manifest: {error}"))?;
    stored
        .manifest
        .validate()
        .map_err(|error| format!("unusable manifest: {error}"))?;
    if let Some(sha256) = stored.sha256.as_deref()
        && !valid_sha256(sha256)
    {
        return Err("the manifest carries a malformed sha256".to_string());
    }
    Ok(stored)
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|character| character.is_ascii_hexdigit())
}

/// The lowercase SHA-256 used for both sidecar verification and cache
/// identity. Keeping this in the manager gives every install path the same
/// spelling and avoids length-only cache keys.
pub fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

static INSTALL_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Three consecutive sandbox/network failures take one plugin out of the
/// current session's provider chains. The state is intentionally in-memory:
/// a transient outage should not become a persisted opt-out across restarts.
pub const PLUGIN_FAILURE_LIMIT: u8 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginStatus {
    Healthy,
    Failing(u8),
    Disabled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginHealthChange {
    None,
    Failure(u8),
    Disabled(u8),
    AlreadyDisabled,
    Recovered,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FailureState {
    Failing(u8),
    Disabled,
}

/// Thread-safe health for provider runs. The key includes the wasm digest,
/// so replacing a plugin's contents starts a fresh failure streak.
#[derive(Clone, Default)]
pub struct PluginHealth {
    states: Arc<Mutex<HashMap<(String, String), FailureState>>>,
}

impl PluginHealth {
    fn key(plugin: &Plugin) -> (String, String) {
        (plugin.id.clone(), plugin.sha256.clone())
    }

    pub fn status(&self, plugin: &Plugin) -> PluginStatus {
        let state = self
            .states
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&Self::key(plugin))
            .copied();
        match state {
            Some(FailureState::Failing(count)) => PluginStatus::Failing(count),
            Some(FailureState::Disabled) => PluginStatus::Disabled,
            None => PluginStatus::Healthy,
        }
    }

    pub fn enabled(&self, plugin: &Plugin) -> bool {
        !matches!(self.status(plugin), PluginStatus::Disabled)
    }

    /// Forget the runtime failure state for a freshly installed module.
    /// Installing the same digest again is an explicit user action to trust
    /// it once more, so a previous session-local disable must not survive the
    /// successful publish.
    pub fn reset(&self, plugin: &Plugin) {
        self.reset_identity(&plugin.id, &plugin.sha256);
    }

    /// Reset one installed identity without requiring a plugin object on the
    /// backend command path. The digest keeps a replacement's state distinct
    /// from an older module with the same id.
    pub fn reset_identity(&self, id: &str, sha256: &str) {
        self.states
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&(id.to_string(), sha256.to_string()));
    }

    pub fn success(&self, plugin: &Plugin) -> PluginHealthChange {
        let key = Self::key(plugin);
        let mut states = self
            .states
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match states.get(&key).copied() {
            Some(FailureState::Failing(_)) => {
                states.remove(&key);
                PluginHealthChange::Recovered
            }
            // An in-flight call may finish after another call disabled the
            // plugin. It must not silently re-enable that plugin.
            Some(FailureState::Disabled) => PluginHealthChange::None,
            None => {
                // A replacement with a new digest is a fresh module. Clear
                // an older disabled identity once the replacement answers,
                // so the app's visible status follows the current bytes.
                let replaced_disabled = states.iter().any(|(identity, state)| {
                    identity.0 == plugin.id
                        && identity.1 != plugin.sha256
                        && matches!(state, FailureState::Disabled)
                });
                if replaced_disabled {
                    states.retain(|identity, _| identity.0 != plugin.id);
                    PluginHealthChange::Recovered
                } else {
                    PluginHealthChange::None
                }
            }
        }
    }

    pub fn failure(&self, plugin: &Plugin) -> PluginHealthChange {
        let key = Self::key(plugin);
        let mut states = self
            .states
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let state = states.entry(key).or_insert(FailureState::Failing(0));
        match *state {
            FailureState::Disabled => PluginHealthChange::AlreadyDisabled,
            FailureState::Failing(count) => {
                let next = count.saturating_add(1).min(PLUGIN_FAILURE_LIMIT);
                if next >= PLUGIN_FAILURE_LIMIT {
                    *state = FailureState::Disabled;
                    PluginHealthChange::Disabled(next)
                } else {
                    *state = FailureState::Failing(next);
                    PluginHealthChange::Failure(next)
                }
            }
        }
    }
}

fn temporary_path(dir: &Path, id: &str, suffix: &str) -> PathBuf {
    let serial = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    dir.join(format!(".{id}.{suffix}.{}-{serial}", std::process::id()))
}

fn write_temp(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("cannot stage {}: {error}", path.display()))?;
    if let Err(error) = file.write_all(bytes).and_then(|_| file.sync_all()) {
        let _ = fs::remove_file(path);
        return Err(format!("cannot stage {}: {error}", path.display()));
    }
    Ok(())
}

/// Publishes both files as one checked installation. Each final rename is
/// atomic, and an error during the second rename restores the previous pair,
/// so readers never accept a half-written plugin as valid.
fn write_pair_atomic(dir: &Path, id: &str, wasm: &[u8], manifest: &[u8]) -> Result<(), String> {
    let _guard = INSTALL_LOCK
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let wasm_path = dir.join(format!("{id}.wasm"));
    let manifest_path = dir.join(format!("{id}.json"));
    let tmp_wasm = temporary_path(dir, id, "wasm");
    let tmp_manifest = temporary_path(dir, id, "json");
    write_temp(&tmp_wasm, wasm)?;
    if let Err(error) = write_temp(&tmp_manifest, manifest) {
        let _ = fs::remove_file(&tmp_wasm);
        return Err(error);
    }

    let old_wasm = temporary_path(dir, id, "old-wasm");
    let old_manifest = temporary_path(dir, id, "old-json");
    let had_wasm = wasm_path.exists();
    let had_manifest = manifest_path.exists();
    let mut wasm_published = false;
    let mut manifest_published = false;
    let result = (|| {
        if had_wasm {
            fs::rename(&wasm_path, &old_wasm)
                .map_err(|error| format!("cannot stage the existing plugin: {error}"))?;
        }
        if had_manifest && let Err(error) = fs::rename(&manifest_path, &old_manifest) {
            if had_wasm {
                let _ = fs::rename(&old_wasm, &wasm_path);
            }
            return Err(format!("cannot stage the existing manifest: {error}"));
        }
        fs::rename(&tmp_wasm, &wasm_path)
            .map_err(|error| format!("cannot publish the plugin: {error}"))?;
        wasm_published = true;
        fs::rename(&tmp_manifest, &manifest_path)
            .map_err(|error| format!("cannot publish the manifest: {error}"))?;
        manifest_published = true;
        // Directory fsync is supported on Unix and harmlessly unavailable
        // on some Windows filesystems; the rename is still atomic there.
        if let Ok(directory) = File::open(dir) {
            let _ = directory.sync_all();
        }
        Ok(())
    })();

    if let Err(error) = result {
        if manifest_published {
            let _ = fs::remove_file(&manifest_path);
        }
        if wasm_published {
            let _ = fs::remove_file(&wasm_path);
        }
        if had_manifest {
            let _ = fs::rename(&old_manifest, &manifest_path);
        }
        if had_wasm {
            let _ = fs::rename(&old_wasm, &wasm_path);
        }
        let _ = fs::remove_file(&tmp_wasm);
        let _ = fs::remove_file(&tmp_manifest);
        let _ = fs::remove_file(&old_wasm);
        let _ = fs::remove_file(&old_manifest);
        return Err(error);
    }
    let _ = fs::remove_file(&old_wasm);
    let _ = fs::remove_file(&old_manifest);
    Ok(())
}

/// A plugin as the interface sees it: who it is and what it may do.
#[derive(Clone, Debug, PartialEq)]
pub struct Plugin {
    pub id: String,
    pub name: String,
    pub publisher: String,
    pub version: String,
    pub homepage: String,
    pub capabilities: Vec<String>,
    pub domains: Vec<String>,
    /// The SHA-256 of the wasm currently on disk. Cache keys include this,
    /// so replacing a module cannot reuse an answer from its predecessor.
    pub sha256: String,
}

impl Plugin {
    fn from_manifest(manifest: PluginManifest, sha256: String) -> Self {
        Self {
            id: manifest.id,
            name: manifest.name,
            publisher: manifest.publisher,
            version: manifest.version,
            homepage: manifest.homepage,
            capabilities: manifest.capabilities,
            domains: manifest.domains,
            sha256,
        }
    }

    /// The kinds this plugin can answer for, with the `provider:` prefix
    /// stripped: `lyrics`, `translate`, `romanize`.
    pub fn provider_kinds(&self) -> Vec<&str> {
        self.capabilities
            .iter()
            .filter_map(|capability| capability.strip_prefix(crate::plugins::PROVIDER_CAPABILITY))
            .collect()
    }

    /// The manifest the host believes about this plugin — the allowlist the
    /// user agreed to when it arrived, whatever a run of the module itself
    /// might claim.
    pub fn manifest(&self) -> PluginManifest {
        PluginManifest {
            id: self.id.clone(),
            name: self.name.clone(),
            publisher: self.publisher.clone(),
            version: self.version.clone(),
            api: crate::plugins::ABI_VERSION,
            capabilities: self.capabilities.clone(),
            domains: self.domains.clone(),
            homepage: self.homepage.clone(),
        }
    }
}

/// Every plugin installed on disk, sorted by id. A plugin whose manifest
/// is missing or unreadable is skipped with a note, never a failure — one
/// bad file must not cost the rest their page.
pub fn list(dirs: &AppDirs) -> Vec<Plugin> {
    let _guard = INSTALL_LOCK
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    list_impl(dirs, true)
}

/// The metadata needed to draw provider rows, without instantiating every
/// wasm module. Runtime callers use this path first and let the host runner
/// validate the selected module only when it is actually asked to answer.
/// Integrity-pinned sidecars supply the digest without rereading their wasm;
/// legacy sidecars still need one read to derive a cache identity.
pub fn list_metadata(dirs: &AppDirs) -> Vec<Plugin> {
    let _guard = INSTALL_LOCK
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    list_impl(dirs, false)
}

fn list_impl(dirs: &AppDirs, verify_modules: bool) -> Vec<Plugin> {
    let dir = dirs.plugins_dir();
    let mut plugins = Vec::new();
    let mut ids: Vec<String> = fs::read_dir(&dir)
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|entry| {
                    let path = entry.path();
                    (path
                        .extension()
                        .is_some_and(|extension| extension == "wasm")
                        && path.is_file())
                    .then(|| {
                        path.file_stem()
                            .map(|stem| stem.to_string_lossy().into_owned())
                    })
                    .flatten()
                })
                .collect()
        })
        .unwrap_or_default();
    ids.sort();
    ids.dedup();
    let mut seen = HashSet::new();
    for id in ids {
        if PluginManifest::validate_id(&id).is_err() {
            log::warn!("the plugin file {id:?} has an unusable name; leaving it out of the list");
            continue;
        }
        let wasm_path = dir.join(format!("{id}.wasm"));
        let manifest_path = dir.join(format!("{id}.json"));
        let Ok(text) = std::fs::read_to_string(&manifest_path) else {
            log::warn!("the plugin {id} has no readable manifest; leaving it out of the list");
            continue;
        };
        let stored = match parse_stored_manifest(&text) {
            Ok(stored) => stored,
            Err(error) => {
                log::warn!("the manifest of the plugin {id} is unusable: {error}");
                continue;
            }
        };
        if stored.manifest.id != id {
            log::warn!(
                "the manifest of the plugin {id} names {}; leaving it out of the list",
                stored.manifest.id
            );
            continue;
        }
        let digest = if verify_modules {
            let Ok(wasm) = fs::read(&wasm_path) else {
                log::warn!("the plugin {id} has no readable wasm; leaving it out");
                continue;
            };
            let digest = sha256(&wasm);
            if let Some(expected) = stored.sha256.as_deref() {
                if !valid_sha256(expected) || !digest.eq_ignore_ascii_case(expected) {
                    log::warn!("the plugin {id} failed its wasm integrity check; leaving it out");
                    continue;
                }
                // An integrity-pinned sidecar is checked against the module's
                // own manifest as well. This prevents a tampered domain or
                // capability list from changing what the host will execute.
                match crate::plugins::host::validate(&wasm) {
                    Ok(declared) if declared == stored.manifest => {}
                    Ok(_) => {
                        log::warn!(
                            "the plugin {id} sidecar disagrees with its wasm; leaving it out"
                        );
                        continue;
                    }
                    Err(error) => {
                        log::warn!("the plugin {id} is not a valid ABI module: {error}");
                        continue;
                    }
                }
            }
            digest
        } else if let Some(expected) = stored.sha256.as_deref() {
            // The sidecar was written by our atomic installer. The actual
            // bytes are checked by `wasm_bytes` immediately before a run.
            expected.to_ascii_lowercase()
        } else {
            let Ok(wasm) = fs::read(&wasm_path) else {
                log::warn!("the plugin {id} has no readable wasm; leaving it out");
                continue;
            };
            sha256(&wasm)
        };
        if !seen.insert(stored.manifest.id.clone()) {
            log::warn!(
                "duplicate plugin id {}; keeping the first entry",
                stored.manifest.id
            );
            continue;
        }
        plugins.push(Plugin::from_manifest(stored.manifest, digest));
    }
    plugins
}

static METADATA_CACHE: OnceLock<Mutex<HashMap<PathBuf, Vec<Plugin>>>> = OnceLock::new();

/// The app and backend share one metadata snapshot, so a track change does
/// not reread and reparse every sidecar. Installs/removals invalidate it.
pub fn list_metadata_cached(dirs: &AppDirs) -> Vec<Plugin> {
    let dir = dirs.plugins_dir();
    let _install_guard = INSTALL_LOCK
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(found) = METADATA_CACHE
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&dir)
        .cloned()
    {
        return found;
    }
    let found = list_impl(dirs, false);
    METADATA_CACHE
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(dir, found.clone());
    found
}

/// Drops the process-local metadata snapshot after a successful install or
/// removal. A failed operation leaves the previous snapshot usable.
pub fn invalidate_metadata_cache(dirs: &AppDirs) {
    let dir = dirs.plugins_dir();
    if let Some(cache) = METADATA_CACHE.get() {
        cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&dir);
    }
}

/// Resolves a chain against the known plugins: each id in the order the
/// chain names it, keeping only the plugins that claim `provider:{kind}`,
/// and skipping ids nobody has heard of — a stale entry never stops the
/// ones behind it.
pub fn chain_plugins<'a>(all: &'a [Plugin], chain: &[String], kind: &str) -> Vec<&'a Plugin> {
    let wanted = PluginManifest::provider_capability(kind);
    let mut seen = HashSet::new();
    chain
        .iter()
        .filter(|id| seen.insert(id.as_str()))
        .filter_map(|id| all.iter().find(|plugin| &plugin.id == id))
        .filter(|plugin| plugin.capabilities.contains(&wanted))
        .collect()
}

/// Validates a plugin and installs it: the wasm must load, speak ABI 1,
/// and claim a provider capability before anything is written, and it
/// lands as `plugins/<id>.wasm` with its manifest beside it.
///
/// This is the synchronous form, for tests and callers with no runtime.
/// The interface goes through [`install`], which runs it off-thread so a
/// slow module never holds the frame.
pub fn install_blocking(dirs: &AppDirs, wasm: &[u8]) -> Result<Plugin, String> {
    let manifest = crate::plugins::host::validate(wasm)?;
    PluginManifest::validate_id(&manifest.id)?;
    let dir = dirs.plugins_dir();
    fs::create_dir_all(&dir).map_err(|error| format!("cannot open the plugins folder: {error}"))?;
    let digest = sha256(wasm);
    let text = serde_json::to_string_pretty(&StoredManifest {
        manifest: manifest.clone(),
        sha256: Some(digest.clone()),
    })
    .map_err(|error| format!("cannot encode the manifest: {error}"))?;
    write_pair_atomic(&dir, &manifest.id, wasm, text.as_bytes())?;
    invalidate_metadata_cache(dirs);
    Ok(Plugin::from_manifest(manifest, digest))
}

/// Installs a plugin, off the runtime's blocking threads: the interface
/// hands this the bytes of a `.wasm` file and gets a listed plugin back.
pub async fn install(dirs: &AppDirs, wasm: Vec<u8>) -> Result<Plugin, String> {
    let dirs = dirs.clone();
    tokio::task::spawn_blocking(move || install_blocking(&dirs, &wasm))
        .await
        .map_err(|error| format!("the install task died: {error}"))?
}

/// Removes an installed plugin's two files. Chains are the settings', so
/// dropping the id from them is the caller's half of an uninstall.
pub fn remove(dirs: &AppDirs, id: &str) {
    if PluginManifest::validate_id(id).is_err() {
        log::warn!("refusing to remove a plugin with an unsafe id {id:?}");
        return;
    }
    let _guard = INSTALL_LOCK
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = dirs.plugins_dir();
    for name in [format!("{id}.wasm"), format!("{id}.json")] {
        if let Err(error) = std::fs::remove_file(dir.join(&name))
            && error.kind() != std::io::ErrorKind::NotFound
        {
            log::warn!("unable to remove the plugin file {name}: {error}");
        }
    }
    invalidate_metadata_cache(dirs);
}

/// The wasm a plugin runs from: its installed file.
pub fn wasm_bytes(dirs: &AppDirs, plugin: &Plugin) -> Result<Vec<u8>, String> {
    let dir = dirs.plugins_dir();
    let wasm_path = dir.join(format!("{}.wasm", plugin.id));
    let wasm = fs::read(&wasm_path)
        .map_err(|error| format!("cannot read the plugin {}: {error}", plugin.id))?;
    let digest = sha256(&wasm);
    if !digest.eq_ignore_ascii_case(&plugin.sha256) {
        return Err(format!(
            "the plugin {} changed after it was listed; refusing to run it",
            plugin.id
        ));
    }
    let manifest_text = fs::read_to_string(dir.join(format!("{}.json", plugin.id)))
        .map_err(|error| format!("cannot read the plugin {} manifest: {error}", plugin.id))?;
    let stored = parse_stored_manifest(&manifest_text)?;
    if stored.manifest != plugin.manifest() {
        return Err(format!(
            "the plugin {} manifest changed after it was listed; refusing to run it",
            plugin.id
        ));
    }
    if let Some(expected) = stored.sha256.as_deref()
        && (!valid_sha256(expected) || !digest.eq_ignore_ascii_case(expected))
    {
        return Err(format!(
            "the plugin {} failed its wasm integrity check",
            plugin.id
        ));
    }
    Ok(wasm)
}

/// The cache file of a plugin's answer: keyed by the plugin first, the
/// tongue and the words after, so one plugin's answer can never be served
/// for another's, and kept beside the built-in translator's own files.
pub fn cache_path(cache_dir: &Path, id: &str, target: &str, lines: &[&str]) -> PathBuf {
    cache_dir.join(format!("{}.json", cache_key(id, target, lines)))
}

/// The digest naming a plugin's cache file.
pub fn cache_key(id: &str, target: &str, lines: &[&str]) -> String {
    cache_key_for(id, "", "", target, lines)
}

/// The content-aware cache key for a provider result. The old helpers stay
/// available for callers that only have an id; runtime provider paths use
/// this form so a version bump or a same-length wasm replacement cannot
/// reuse an answer from the old module.
pub fn cache_key_for(
    id: &str,
    version: &str,
    wasm_sha256: &str,
    target: &str,
    lines: &[&str],
) -> String {
    let mut payload = String::from("woofer-plugin-cache-v2");
    for field in [id, version, wasm_sha256, target] {
        payload.push('\n');
        payload.push_str(&field.len().to_string());
        payload.push(':');
        payload.push_str(field);
    }
    payload.push('\n');
    payload.push_str(&lines.len().to_string());
    for line in lines {
        payload.push('\n');
        payload.push_str(&line.len().to_string());
        payload.push(':');
        payload.push_str(line);
    }
    crate::translate::cache_digest(&payload)
}

pub fn cache_path_for(
    cache_dir: &Path,
    id: &str,
    version: &str,
    wasm_sha256: &str,
    target: &str,
    lines: &[&str],
) -> PathBuf {
    cache_dir.join(format!(
        "{}.json",
        cache_key_for(id, version, wasm_sha256, target, lines)
    ))
}

/// Reads a plugin's cached answer, while it is fresh.
pub fn read_cached(
    cache_dir: &Path,
    id: &str,
    target: &str,
    lines: &[&str],
) -> Option<Option<Translation>> {
    crate::translate::read_cached(&cache_path(cache_dir, id, target, lines))
}

pub fn read_cached_for(
    cache_dir: &Path,
    id: &str,
    version: &str,
    wasm_sha256: &str,
    target: &str,
    lines: &[&str],
) -> Option<Option<Translation>> {
    crate::translate::read_cached(&cache_path_for(
        cache_dir,
        id,
        version,
        wasm_sha256,
        target,
        lines,
    ))
}

/// Remembers a plugin's answer on disk, `None` included.
pub fn store_cached(
    cache_dir: &Path,
    id: &str,
    target: &str,
    lines: &[&str],
    found: &Option<Translation>,
) {
    crate::translate::store_cached(&cache_path(cache_dir, id, target, lines), found);
}

pub fn store_cached_for(
    cache_dir: &Path,
    id: &str,
    version: &str,
    wasm_sha256: &str,
    target: &str,
    lines: &[&str],
    found: &Option<Translation>,
) {
    crate::translate::store_cached(
        &cache_path_for(cache_dir, id, version, wasm_sha256, target, lines),
        found,
    );
}

/// The cache file of a lyrics plugin's answer: a new directory beside the
/// built-in lyrics cache, keyed by the plugin first and the track after,
/// so one provider's answer can never be served for another's.
pub fn lyrics_cache_path(
    lyrics_cache_dir: &Path,
    id: &str,
    query: &crate::lyrics::Query,
) -> PathBuf {
    lyrics_cache_dir.join("lyrics_plugins").join(format!(
        "{}.json",
        crate::translate::cache_digest(&format!(
            "{id}\n{}|{}|{}|{}",
            query.artist, query.title, query.album, query.duration_ms
        ))
    ))
}

/// The content-aware cache path for a lyrics provider. Version and wasm
/// digest are part of the identity for the same reason as translation
/// answers above.
pub fn lyrics_cache_path_for(
    lyrics_cache_dir: &Path,
    id: &str,
    version: &str,
    wasm_sha256: &str,
    query: &crate::lyrics::Query,
) -> PathBuf {
    lyrics_cache_dir.join("lyrics_plugins").join(format!(
        "{}.json",
        crate::translate::cache_digest(&format!(
            "woofer-lyrics-plugin-v2\n{id}\n{version}\n{wasm_sha256}\n{}|{}|{}|{}",
            query.artist, query.title, query.album, query.duration_ms
        ))
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A private app directory set, alone until something is written into it.
    fn dirs(name: &str) -> AppDirs {
        let root =
            std::env::temp_dir().join(format!("woofer-plugins-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        AppDirs {
            config: root.join("config"),
            state: root.join("state"),
            cache: root.join("cache"),
        }
    }

    fn manifest_text(id: &str, capability: &str) -> String {
        format!(
            r#"{{"id":"{id}","name":"{id}","publisher":"kreatzzz","version":"1.0.0","api":1,
                "capabilities":["{capability}"],"domains":["clients5.google.com"],
                "homepage":"https://example.com/{id}"}}"#
        )
    }

    fn install_manually(dirs: &AppDirs, id: &str, capability: &str) {
        let dir = dirs.plugins_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{id}.wasm")), b"placeholder").unwrap();
        std::fs::write(
            dir.join(format!("{id}.json")),
            manifest_text(id, capability),
        )
        .unwrap();
    }

    /// A listed plugin without a folder behind it, for the pure lookups.
    fn plugin(id: &str, capability: &str) -> Plugin {
        Plugin::from_manifest(
            PluginManifest::parse(&manifest_text(id, capability)).unwrap(),
            String::new(),
        )
    }

    #[test]
    fn an_empty_folder_lists_nothing() {
        let dirs = dirs("empty");
        assert!(list(&dirs).is_empty());
        let _ = std::fs::remove_dir_all(dirs.state.parent().unwrap());
    }

    #[test]
    fn installed_plugins_are_listed_sorted_by_id() {
        let dirs = dirs("order");
        install_manually(&dirs, "deepl", "provider:translate");
        install_manually(&dirs, "acme", "provider:romanize");
        let plugins = list(&dirs);
        let ids: Vec<&str> = plugins.iter().map(|plugin| plugin.id.as_str()).collect();
        assert_eq!(ids, vec!["acme", "deepl"]);
        let _ = std::fs::remove_dir_all(dirs.state.parent().unwrap());
    }

    #[test]
    fn metadata_listing_defers_wasm_instantiation_until_a_run() {
        let dirs = dirs("metadata-only");
        let dir = dirs.plugins_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let wasm = b"not a wasm module";
        let digest = sha256(wasm);
        std::fs::write(dir.join("acme.wasm"), wasm).unwrap();
        let mut sidecar: serde_json::Value =
            serde_json::from_str(&manifest_text("acme", "provider:translate")).unwrap();
        sidecar["sha256"] = serde_json::Value::String(digest);
        std::fs::write(dir.join("acme.json"), sidecar.to_string()).unwrap();

        // The page can read a pinned identity without compiling arbitrary
        // bytes; execution still goes through the strict path.
        assert_eq!(list_metadata(&dirs).len(), 1);
        assert!(list(&dirs).is_empty());
        let _ = std::fs::remove_dir_all(dirs.state.parent().unwrap());
    }

    #[test]
    fn a_chain_resolves_in_its_own_order_and_skips_what_it_cannot_use() {
        let deepl = plugin("deepl", "provider:translate");
        let acme = plugin("acme", "provider:translate");
        let lyrics = plugin("words", "provider:lyrics");
        let all = vec![deepl, acme, lyrics];
        let chain = chain_plugins(
            &all,
            &["gone".to_string(), "acme".to_string(), "deepl".to_string()],
            "translate",
        );
        let ids: Vec<&str> = chain.iter().map(|plugin| plugin.id.as_str()).collect();
        // An id nobody has heard of never stops the ones behind it, and a
        // plugin of another kind is not for this chain.
        assert_eq!(ids, vec!["acme", "deepl"]);
        // The lyrics plugin only answers its own kind's call.
        let lyrics_chain = chain_plugins(&all, &["words".to_string()], "translate");
        assert!(lyrics_chain.is_empty());
    }

    #[test]
    fn a_kind_of_no_claimant_resolves_to_nothing() {
        let all = vec![plugin("deepl", "provider:translate")];
        assert!(chain_plugins(&all, &["deepl".to_string()], "romanize").is_empty());
    }

    #[test]
    fn a_plugin_announces_the_kinds_it_answers_for() {
        assert_eq!(
            plugin("deepl", "provider:translate").provider_kinds(),
            vec!["translate"]
        );
        assert_eq!(
            plugin("words", "provider:lyrics").provider_kinds(),
            vec!["lyrics"]
        );
        // A capability outside the provider taxonomy is not a kind.
        assert!(plugin("odd", "panel:sidebar").provider_kinds().is_empty());
    }

    #[test]
    fn a_manifestless_plugin_is_skipped_without_costing_the_rest() {
        let dirs = dirs("broken");
        let dir = dirs.plugins_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("lonely.wasm"), b"placeholder").unwrap();
        std::fs::write(dir.join("bad.json"), "not json").unwrap();
        let plugins = list(&dirs);
        assert!(plugins.is_empty());
        let _ = std::fs::remove_dir_all(dirs.state.parent().unwrap());
    }

    #[test]
    fn placeholder_bytes_are_refused_and_nothing_is_written() {
        let dirs = dirs("refused");
        assert!(install_blocking(&dirs, b"placeholder").is_err());
        assert!(!dirs.plugins_dir().exists());
        let _ = std::fs::remove_dir_all(dirs.state.parent().unwrap());
    }

    #[test]
    fn removing_takes_both_files() {
        let dirs = dirs("remove");
        install_manually(&dirs, "deepl", "provider:translate");
        let dir = dirs.plugins_dir();
        remove(&dirs, "deepl");
        assert!(!dir.join("deepl.wasm").exists());
        assert!(!dir.join("deepl.json").exists());
        let _ = std::fs::remove_dir_all(dirs.state.parent().unwrap());
    }

    #[test]
    fn a_plugin_cache_key_is_prefixed_by_its_id() {
        let lines = ["こんにちは", "世界"];
        assert_ne!(
            cache_key("translate", "en", &lines),
            cache_key("romanize", "en", &lines)
        );
        // Neither collides with the built-in translator's own key, whole
        // or split in halves.
        assert_ne!(
            cache_key("translate", "en", &lines),
            crate::translate::cache_key(None, "en", &lines)
        );
        assert_ne!(
            cache_key("translate", "en", &lines),
            crate::translate::cache_key(Some("translation"), "en", &lines)
        );
        assert_eq!(
            cache_key("deepl", "en", &lines),
            cache_key("deepl", "en", &lines)
        );
    }

    #[test]
    fn a_provider_cache_key_changes_with_version_and_content() {
        let lines = ["こんにちは", "世界"];
        let digest_a = "a".repeat(64);
        let digest_b = "b".repeat(64);
        let first = cache_key_for("translate", "1.0.0", &digest_a, "en", &lines);
        assert_ne!(
            first,
            cache_key_for("translate", "1.1.0", &digest_a, "en", &lines)
        );
        assert_ne!(
            first,
            cache_key_for("translate", "1.0.0", &digest_b, "en", &lines)
        );
        // Newline-containing lines are length-framed, not aliases for two
        // separate lyric lines.
        assert_ne!(
            first,
            cache_key_for("translate", "1.0.0", &digest_a, "en", &["こんにちは\n世界"])
        );
    }

    #[test]
    fn duplicate_chain_entries_are_only_asked_once() {
        let plugin = plugin("translate", "provider:translate");
        let all = vec![plugin];
        let chain = chain_plugins(&all, &["translate".into(), "translate".into()], "translate");
        assert_eq!(chain.len(), 1);
    }

    #[test]
    fn a_plugin_disables_after_three_failures() {
        let plugin = plugin("translate", "provider:translate");
        let health = PluginHealth::default();
        assert_eq!(health.status(&plugin), PluginStatus::Healthy);
        assert_eq!(health.failure(&plugin), PluginHealthChange::Failure(1));
        assert_eq!(health.failure(&plugin), PluginHealthChange::Failure(2));
        assert_eq!(health.failure(&plugin), PluginHealthChange::Disabled(3));
        assert!(!health.enabled(&plugin));
        assert_eq!(health.status(&plugin), PluginStatus::Disabled);
        assert_eq!(health.failure(&plugin), PluginHealthChange::AlreadyDisabled);
    }

    #[test]
    fn reinstalling_a_digest_resets_its_disabled_health() {
        let plugin = plugin("translate", "provider:translate");
        let health = PluginHealth::default();
        for _ in 0..PLUGIN_FAILURE_LIMIT {
            let _ = health.failure(&plugin);
        }
        assert_eq!(health.status(&plugin), PluginStatus::Disabled);

        health.reset(&plugin);

        assert_eq!(health.status(&plugin), PluginStatus::Healthy);
        assert_eq!(health.failure(&plugin), PluginHealthChange::Failure(1));
    }

    #[test]
    fn a_success_resets_only_a_failing_streak() {
        let plugin = plugin("translate", "provider:translate");
        let health = PluginHealth::default();
        assert_eq!(health.failure(&plugin), PluginHealthChange::Failure(1));
        assert_eq!(health.success(&plugin), PluginHealthChange::Recovered);
        assert_eq!(health.status(&plugin), PluginStatus::Healthy);
        assert_eq!(health.failure(&plugin), PluginHealthChange::Failure(1));

        let replacement = Plugin::from_manifest(
            PluginManifest::parse(&manifest_text("translate", "provider:translate")).unwrap(),
            "different-content".into(),
        );
        // A new module identity gets its own streak.
        assert_eq!(health.status(&replacement), PluginStatus::Healthy);
        assert_eq!(health.failure(&replacement), PluginHealthChange::Failure(1));
    }

    #[test]
    fn a_replacement_module_starts_healthy_after_an_old_one_was_disabled() {
        let plugin = plugin("translate", "provider:translate");
        let health = PluginHealth::default();
        for _ in 0..PLUGIN_FAILURE_LIMIT {
            let _ = health.failure(&plugin);
        }
        assert_eq!(health.status(&plugin), PluginStatus::Disabled);
        let replacement = Plugin::from_manifest(
            PluginManifest::parse(&manifest_text("translate", "provider:translate")).unwrap(),
            "different-content".into(),
        );
        assert!(health.enabled(&replacement));
        assert_eq!(health.success(&replacement), PluginHealthChange::Recovered);
        assert_eq!(health.status(&plugin), PluginStatus::Healthy);
    }

    #[test]
    fn an_installed_pair_rejects_sidecar_tampering() {
        let Some(wasm) = std::fs::read(
            "plugins/translate/target/wasm32-unknown-unknown/release/woofer_plugin_translate.wasm",
        )
        .ok() else {
            // The plugin crates are optional in a plain host checkout.
            return;
        };
        let dirs = dirs("integrity");
        install_blocking(&dirs, &wasm).expect("the built plugin installs");
        assert_eq!(list(&dirs).len(), 1);
        let manifest_path = dirs.plugins_dir().join("translate.json");
        let text = std::fs::read_to_string(&manifest_path).unwrap();
        let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
        value["domains"] = serde_json::json!(["evil.example"]);
        std::fs::write(&manifest_path, value.to_string()).unwrap();
        assert!(list(&dirs).is_empty());
        let _ = std::fs::remove_dir_all(dirs.state.parent().unwrap());
    }

    #[test]
    fn a_lyrics_plugin_caches_beside_the_built_in_lyrics_never_among_them() {
        let query = crate::lyrics::Query {
            artist: "Artist".into(),
            title: "Song".into(),
            album: String::new(),
            duration_ms: 201_000,
        };
        let cache_dir =
            std::env::temp_dir().join(format!("woofer-lyrics-cache-{}", std::process::id()));
        let path = lyrics_cache_path(&cache_dir, "acme", &query);
        assert!(path.starts_with(cache_dir.join("lyrics_plugins")));
        assert_eq!(path.extension().and_then(|ext| ext.to_str()), Some("json"));
        // One plugin's key is another's stranger, and the track is in it.
        let other = lyrics_cache_path(
            &cache_dir,
            "other",
            &crate::lyrics::Query {
                artist: "Artist".into(),
                ..query.clone()
            },
        );
        assert_ne!(path, other);
        assert_ne!(
            path,
            lyrics_cache_path(
                &cache_dir,
                "acme",
                &crate::lyrics::Query {
                    title: "Other song".into(),
                    ..query
                }
            )
        );
    }
}
