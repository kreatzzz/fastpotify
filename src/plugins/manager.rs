//! Which plugins are installed, which are built in, and which one answers.
//!
//! The app carries two plugins inside itself; users may add more as files.
//! Every listing puts the installed ones first, sorted by id, then the
//! built-ins, and marks each with whether it is switched on. Picking who
//! answers a question is a pure lookup: the first enabled plugin claiming
//! the capability, and behind it — never asked here — the built-in
//! translator the panel has always had.

use std::path::{Path, PathBuf};

use crate::paths::AppDirs;
use crate::plugins::{BUNDLED, BUNDLED_IDS, PluginManifest};
use crate::translate::Translation;

/// A plugin as the interface sees it: who it is, what it may do, and
/// whether the user wants it answering.
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
    pub enabled: bool,
}

impl Plugin {
    fn from_manifest(manifest: PluginManifest, bundled: bool, disabled: &[String]) -> Self {
        let enabled = !disabled.iter().any(|id| id == &manifest.id);
        Self {
            id: manifest.id,
            name: manifest.name,
            publisher: manifest.publisher,
            version: manifest.version,
            homepage: manifest.homepage,
            capabilities: manifest.capabilities,
            domains: manifest.domains,
            bundled,
            enabled,
        }
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
pub fn list(dirs: &AppDirs, disabled: &[String]) -> Vec<Plugin> {
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
            Ok(manifest) => plugins.push(Plugin::from_manifest(manifest, false, disabled)),
            Err(error) => log::warn!("the manifest of the plugin {id} is unusable: {error}"),
        }
    }
    for bundled in BUNDLED {
        match PluginManifest::parse(bundled.manifest) {
            Ok(manifest) => plugins.push(Plugin::from_manifest(manifest, true, disabled)),
            Err(error) => log::warn!("a built-in plugin has an unusable manifest: {error}"),
        }
    }
    plugins
}

/// The first enabled plugin claiming the capability `kind` asks for —
/// installed before built-in, and none at all when the user has disabled
/// every claimant. Whoever it is, the host falls back to the built-in
/// translator behind it.
pub fn active<'a>(plugins: &'a [Plugin], kind: &str) -> Option<&'a Plugin> {
    let wanted = PluginManifest::translation_capability(kind);
    plugins
        .iter()
        .find(|plugin| plugin.enabled && plugin.capabilities.contains(&wanted))
}

/// Validates a plugin and installs it: the wasm must load, speak ABI 1,
/// and claim a translation capability before anything is written, and it
/// lands as `plugins/<id>.wasm` with its manifest beside it.
///
/// This is the synchronous form, for tests and callers with no runtime.
/// The interface goes through [`install`], which runs it off-thread so a
/// slow module never holds the frame.
pub fn install_blocking(dirs: &AppDirs, wasm: &[u8]) -> Result<Plugin, String> {
    let manifest = crate::plugins::host::validate(wasm)?;
    if BUNDLED_IDS.contains(&manifest.id.as_str()) {
        return Err(format!(
            "the id {} belongs to a plugin built into the app; disable that one instead",
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
    Ok(Plugin::from_manifest(manifest, false, &[]))
}

/// Installs a plugin, off the runtime's blocking threads: the interface
/// hands this the bytes of a `.wasm` file and gets a listed plugin back.
pub async fn install(dirs: &AppDirs, wasm: Vec<u8>) -> Result<Plugin, String> {
    let dirs = dirs.clone();
    tokio::task::spawn_blocking(move || install_blocking(&dirs, &wasm))
        .await
        .map_err(|error| format!("the install task died: {error}"))?
}

/// Removes an installed plugin's two files. The built-ins have no files to
/// remove; disabling is their off switch.
pub fn remove(dirs: &AppDirs, id: &str) {
    if BUNDLED_IDS.contains(&id) {
        log::warn!("the plugin {id} is built into the app; disable it instead of removing it");
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

    #[test]
    fn an_empty_folder_leaves_the_built_ins() {
        let dirs = dirs("empty");
        let plugins = list(&dirs, &[]);
        assert_eq!(plugins.len(), 2);
        assert!(
            plugins
                .iter()
                .all(|plugin| plugin.bundled && plugin.enabled)
        );
        let _ = std::fs::remove_dir_all(dirs.state.parent().unwrap());
    }

    #[test]
    fn installed_plugins_come_first_sorted_and_the_built_ins_follow() {
        let dirs = dirs("order");
        install_manually(&dirs, "deepl", "translation-provider:translate");
        install_manually(&dirs, "acme", "translation-provider:romanize");
        let plugins = list(&dirs, &[]);
        let ids: Vec<&str> = plugins.iter().map(|plugin| plugin.id.as_str()).collect();
        assert_eq!(ids, vec!["acme", "deepl", "translate", "romanize"]);
        assert!(!plugins[0].bundled);
        assert!(plugins[2].bundled);
        let _ = std::fs::remove_dir_all(dirs.state.parent().unwrap());
    }

    #[test]
    fn a_disabled_plugin_stays_listed_but_switched_off() {
        let dirs = dirs("disabled");
        install_manually(&dirs, "deepl", "translation-provider:translate");
        let plugins = list(&dirs, &["deepl".to_string()]);
        let deepl = plugins.iter().find(|plugin| plugin.id == "deepl").unwrap();
        assert!(!deepl.enabled);
        // With deepl off, the built-in translate plugin is who answers.
        assert_eq!(active(&plugins, "translate").unwrap().id, "translate");
        let _ = std::fs::remove_dir_all(dirs.state.parent().unwrap());
    }

    #[test]
    fn the_active_plugin_is_the_first_enabled_one_for_its_kind() {
        let dirs = dirs("active");
        install_manually(&dirs, "deepl", "translation-provider:translate");
        let plugins = list(&dirs, &[]);
        assert_eq!(active(&plugins, "translate").unwrap().id, "deepl");
        assert_eq!(active(&plugins, "romanize").unwrap().id, "romanize");
        // A capability nobody claims has no claimant.
        let lyrics_only = vec![Plugin::from_manifest(
            PluginManifest::parse(&manifest_text("x", "lyrics-provider:lines")).unwrap(),
            false,
            &[],
        )];
        assert!(active(&lyrics_only, "translate").is_none());
        let _ = std::fs::remove_dir_all(dirs.state.parent().unwrap());
    }

    #[test]
    fn a_manifestless_plugin_is_skipped_without_costing_the_rest() {
        let dirs = dirs("broken");
        let dir = dirs.plugins_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("lonely.wasm"), b"placeholder").unwrap();
        std::fs::write(dir.join("bad.json"), "not json").unwrap();
        let plugins = list(&dirs, &[]);
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
        install_manually(&dirs, "deepl", "translation-provider:translate");
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
}
