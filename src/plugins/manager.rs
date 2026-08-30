//! Which plugins are installed, which are built in, and the order each
//! kind asks them in.
//!
//! The app carries two plugins inside itself; users may add more as files.
//! Every listing puts the installed ones first, sorted by id, then the
//! built-ins. Picking who answers a question is no longer a single seat:
//! each kind walks its own chain, in the order the user set, and the first
//! provider with data wins — with the built-in engines, never asked here,
//! standing behind the last link.

use std::path::{Path, PathBuf};

use crate::paths::AppDirs;
use crate::plugins::{BUNDLED, BUNDLED_IDS, PluginManifest};
use crate::settings::ProviderChains;
use crate::translate::Translation;

/// A plugin as the interface sees it: who it is, what it may do, and
/// whether it came with the app.
#[derive(Clone, Debug, PartialEq)]
pub struct Plugin {
    pub id: String,
    pub name: String,
    pub publisher: String,
    pub version: String,
    pub homepage: String,
    pub capabilities: Vec<String>,
    pub domains: Vec<String>,
    pub bundled: bool,
}

impl Plugin {
    fn from_manifest(manifest: PluginManifest, bundled: bool) -> Self {
        Self {
            id: manifest.id,
            name: manifest.name,
            publisher: manifest.publisher,
            version: manifest.version,
            homepage: manifest.homepage,
            capabilities: manifest.capabilities,
            domains: manifest.domains,
            bundled,
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

/// Every plugin the app knows: the ones installed on disk first, sorted by
/// id, then the ones it was built with. A plugin whose manifest is missing
/// or unreadable is skipped with a note, never a failure — one bad file
/// must not cost the rest their page.
pub fn list(dirs: &AppDirs) -> Vec<Plugin> {
    let dir = dirs.plugins_dir();
    let mut plugins = Vec::new();
    let mut ids: Vec<String> = std::fs::read_dir(&dir)
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|entry| {
                    entry
                        .path()
                        .file_stem()
                        .map(|stem| stem.to_string_lossy().into_owned())
                })
                .filter(|stem| dir.join(format!("{stem}.wasm")).is_file())
                .collect()
        })
        .unwrap_or_default();
    ids.sort();
    ids.dedup();
    for id in ids {
        let manifest_path = dir.join(format!("{id}.json"));
        let Ok(text) = std::fs::read_to_string(&manifest_path) else {
            log::warn!("the plugin {id} has no readable manifest; leaving it out of the list");
            continue;
        };
        match PluginManifest::parse(&text) {
            Ok(manifest) => plugins.push(Plugin::from_manifest(manifest, false)),
            Err(error) => log::warn!("the manifest of the plugin {id} is unusable: {error}"),
        }
    }
    for bundled in BUNDLED {
        match PluginManifest::parse(bundled.manifest) {
            Ok(manifest) => plugins.push(Plugin::from_manifest(manifest, true)),
            Err(error) => log::warn!("a built-in plugin has an unusable manifest: {error}"),
        }
    }
    plugins
}

/// The chain a kind asks when the user has ordered none: the bundled
/// plugins stand in, and lyrics — whose permanent first link is the
/// built-in flow itself — starts with no plugin at all.
fn default_chain(kind: &str) -> Vec<String> {
    match kind {
        "translate" => vec!["translate".to_string()],
        "romanize" => vec!["romanize".to_string()],
        _ => Vec::new(),
    }
}

/// The ids kind `kind` asks, in order: the user's chain when they have
/// one, the bundled default when they have not.
pub fn chain_ids(chains: &ProviderChains, kind: &str) -> Vec<String> {
    let held = chains.for_kind(kind);
    if held.is_empty() {
        default_chain(kind)
    } else {
        held.clone()
    }
}

/// Resolves a chain against the known plugins: each id in the order the
/// chain names it, keeping only the plugins that claim `provider:{kind}`,
/// and skipping ids nobody has heard of — a stale entry never stops the
/// ones behind it.
pub fn chain_plugins<'a>(all: &'a [Plugin], chain: &[String], kind: &str) -> Vec<&'a Plugin> {
    let wanted = PluginManifest::provider_capability(kind);
    chain
        .iter()
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
    if BUNDLED_IDS.contains(&manifest.id.as_str()) {
        return Err(format!(
            "the id {} belongs to a plugin built into the app",
            manifest.id
        ));
    }
    if manifest.id.contains(['/', '\\', '\0']) || manifest.id == ".." {
        return Err(format!(
            "the id {:?} cannot be a plugin's file name",
            manifest.id
        ));
    }
    let dir = dirs.plugins_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("cannot open the plugins folder: {error}"))?;
    let text = serde_json::to_string_pretty(&manifest)
        .map_err(|error| format!("cannot encode the manifest: {error}"))?;
    std::fs::write(dir.join(format!("{}.wasm", manifest.id)), wasm)
        .map_err(|error| format!("cannot write the plugin: {error}"))?;
    if let Err(error) = std::fs::write(dir.join(format!("{}.json", manifest.id)), text) {
        // The wasm alone answers no question; take it back out.
        let _ = std::fs::remove_file(dir.join(format!("{}.wasm", manifest.id)));
        return Err(format!("cannot write the manifest: {error}"));
    }
    Ok(Plugin::from_manifest(manifest, false))
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
/// dropping the id from them is the caller's half of an uninstall; the
/// built-ins have no files to remove.
pub fn remove(dirs: &AppDirs, id: &str) {
    if BUNDLED_IDS.contains(&id) {
        log::warn!("the plugin {id} is built into the app; it has no files to remove");
        return;
    }
    let dir = dirs.plugins_dir();
    for name in [format!("{id}.wasm"), format!("{id}.json")] {
        if let Err(error) = std::fs::remove_file(dir.join(&name))
            && error.kind() != std::io::ErrorKind::NotFound
        {
            log::warn!("unable to remove the plugin file {name}: {error}");
        }
    }
}

/// The wasm a plugin runs from: the bytes the app was built with for a
/// built-in, the installed file for everything else.
pub fn wasm_bytes(dirs: &AppDirs, plugin: &Plugin) -> Result<Vec<u8>, String> {
    for bundled in BUNDLED {
        if let Ok(manifest) = PluginManifest::parse(bundled.manifest)
            && manifest.id == plugin.id
        {
            return Ok(bundled.wasm.to_vec());
        }
    }
    std::fs::read(dirs.plugins_dir().join(format!("{}.wasm", plugin.id)))
        .map_err(|error| format!("cannot read the plugin {}: {error}", plugin.id))
}

/// The cache file of a plugin's answer: keyed by the plugin first, the
/// tongue and the words after, so one plugin's answer can never be served
/// for another's, and kept beside the built-in translator's own files.
pub fn cache_path(cache_dir: &Path, id: &str, target: &str, lines: &[&str]) -> PathBuf {
    cache_dir.join(format!("{}.json", cache_key(id, target, lines)))
}

/// The digest naming a plugin's cache file.
pub fn cache_key(id: &str, target: &str, lines: &[&str]) -> String {
    crate::translate::cache_digest(&format!("{id}\n{target}\n{}", lines.join("\n")))
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
            false,
        )
    }

    #[test]
    fn an_empty_folder_leaves_the_built_ins() {
        let dirs = dirs("empty");
        let plugins = list(&dirs);
        assert_eq!(plugins.len(), 2);
        assert!(plugins.iter().all(|plugin| plugin.bundled));
        let _ = std::fs::remove_dir_all(dirs.state.parent().unwrap());
    }

    #[test]
    fn installed_plugins_come_first_sorted_and_the_built_ins_follow() {
        let dirs = dirs("order");
        install_manually(&dirs, "deepl", "provider:translate");
        install_manually(&dirs, "acme", "provider:romanize");
        let plugins = list(&dirs);
        let ids: Vec<&str> = plugins.iter().map(|plugin| plugin.id.as_str()).collect();
        assert_eq!(ids, vec!["acme", "deepl", "translate", "romanize"]);
        assert!(!plugins[0].bundled);
        assert!(plugins[2].bundled);
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
    fn an_empty_chain_falls_to_the_bundled_defaults() {
        let chains = ProviderChains::default();
        assert_eq!(chain_ids(&chains, "translate"), vec!["translate"]);
        assert_eq!(chain_ids(&chains, "romanize"), vec!["romanize"]);
        // Lyrics starts with no plugin: the built-in flow is its first
        // link, and no plugin asks after it until one is installed.
        assert_eq!(chain_ids(&chains, "lyrics"), Vec::<String>::new());
        // An ordered chain is the user's, not the default's.
        let chains = ProviderChains {
            translate: vec!["deepl".to_string()],
            ..ProviderChains::default()
        };
        assert_eq!(chain_ids(&chains, "translate"), vec!["deepl"]);
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
        assert!(plugins.iter().all(|plugin| plugin.bundled));
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
    fn removing_takes_both_files_and_never_touches_a_builtin() {
        let dirs = dirs("remove");
        install_manually(&dirs, "deepl", "provider:translate");
        let dir = dirs.plugins_dir();
        // A built-in has no files, but a stray impostor must survive a
        // remove aimed at the real one.
        std::fs::write(dir.join("translate.wasm"), b"impostor").unwrap();
        remove(&dirs, "translate");
        assert!(dir.join("translate.wasm").exists());
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
