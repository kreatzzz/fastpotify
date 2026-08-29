//! Plugins: sandboxed modules that contribute what a data provider can,
//! never what playback or the machine need.
//!
//! A plugin is one `.wasm` file (pure compute, no imports) plus a manifest.
//! The host [`host`] runs it inside wasmi with fuel and a memory cap, does
//! every fetch itself, and refuses anything the manifest does not allow.
//! [`manager`] knows which plugins are installed, which are built in, and
//! which one answers a given question. The app starts fully functional with
//! none of them.

pub mod host;
pub mod manager;

use serde::{Deserialize, Serialize};

/// The plugin API this host speaks. A module wanting another version is
/// refused at the door, not mid-run.
pub const ABI_VERSION: i32 = 1;

/// The capability prefix every translation plugin must claim, followed by
/// the kind it answers: `translate` or `romanize`.
pub const TRANSLATION_CAPABILITY: &str = "translation-provider:";

/// Who a plugin is, as its manifest says it. The host-side copy is the
/// source of truth: the allowlisted domains are the ones the user saw when
/// the plugin arrived, whatever a run of the module itself claims.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    pub publisher: String,
    pub version: String,
    pub api: i32,
    pub capabilities: Vec<String>,
    pub domains: Vec<String>,
    pub homepage: String,
}

impl PluginManifest {
    /// Reads a manifest, refusing what this host cannot load: unreadable
    /// JSON, no id, no capabilities, or an API it does not speak.
    pub fn parse(text: &str) -> Result<Self, String> {
        let manifest: Self =
            serde_json::from_str(text).map_err(|error| format!("unreadable manifest: {error}"))?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Whether the manifest says something this host is willing to load.
    pub fn validate(&self) -> Result<(), String> {
        if self.id.trim().is_empty() {
            return Err("the manifest names no id".to_string());
        }
        if self.api != ABI_VERSION {
            return Err(format!(
                "the manifest wants plugin API {}; this build speaks {ABI_VERSION}",
                self.api
            ));
        }
        if self.capabilities.is_empty() {
            return Err("the manifest claims no capabilities".to_string());
        }
        Ok(())
    }

    /// The capability a translation plugin of `kind` must claim.
    pub fn translation_capability(kind: &str) -> String {
        format!("{TRANSLATION_CAPABILITY}{kind}")
    }
}

/// A plugin compiled into the app: its wasm and the manifest the host
/// believes about it, both frozen at build time.
pub(crate) struct Bundled {
    pub wasm: &'static [u8],
    pub manifest: &'static str,
}

/// The translation plugin that ships with the app. The wasm is compiled
/// separately and dropped in at `assets/plugins/translate.wasm`; until it
/// is, the placeholder bytes fail to load and the built-in translator
/// answers instead.
const TRANSLATE: Bundled = Bundled {
    wasm: include_bytes!("../../assets/plugins/translate.wasm"),
    manifest: r#"{"id":"translate","name":"Translate","publisher":"kreatzzz","version":"1.0.0","api":1,"capabilities":["translation-provider:translate"],"domains":["clients5.google.com"],"homepage":"https://github.com/kreatzzz/woofer-plugin-translate"}"#,
};

/// The romanization plugin that ships with the app, alongside `translate`.
const ROMANIZE: Bundled = Bundled {
    wasm: include_bytes!("../../assets/plugins/romanize.wasm"),
    manifest: r#"{"id":"romanize","name":"Romanize","publisher":"kreatzzz","version":"1.0.0","api":1,"capabilities":["translation-provider:romanize"],"domains":["clients5.google.com"],"homepage":"https://github.com/kreatzzz/woofer-plugin-romanize"}"#,
};
/// Every plugin the app carries inside itself.
pub(crate) const BUNDLED: &[Bundled] = &[TRANSLATE, ROMANIZE];

/// The ids the bundled manifests claim, for callers that must never touch
/// an installed file belonging to one of them.
pub(crate) const BUNDLED_IDS: &[&str] = &["translate", "romanize"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_plugins_load_and_say_who_they_are() {
        for bundled in BUNDLED {
            let loaded = host::validate(bundled.wasm).expect("a bundled plugin loads");
            let claimed = PluginManifest::parse(bundled.manifest).expect("sidecar parses");
            assert_eq!(loaded.api, ABI_VERSION);
            assert_eq!(loaded, claimed, "the module and its sidecar must agree");
        }
    }

    #[test]
    #[ignore = "speaks to the live endpoint"]
    fn the_bundled_plugins_answer_over_the_network() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let input = ["こんにちは、世界"];
        for bundled in BUNDLED {
            let manifest = PluginManifest::parse(bundled.manifest).unwrap();
            let kind = match manifest.capabilities.first().map(String::as_str) {
                Some(capability) => capability
                    .strip_prefix(TRANSLATION_CAPABILITY)
                    .expect("a translation capability")
                    .to_string(),
                None => panic!("bundled {} claims no capability", manifest.id),
            };
            let found = runtime.block_on(host::run_translation(
                bundled.wasm,
                &manifest,
                &kind,
                "en",
                &input,
            ));
            let translation = found.expect("the plugin answers");
            let own = match kind.as_str() {
                "translate" => &translation.translated,
                _ => &translation.romanized,
            };
            assert!(
                own.iter()
                    .any(|line| line.as_deref().is_some_and(|line| !line.is_empty())),
                "{kind} produced nothing: {translation:?}"
            );
        }
    }

    #[test]
    fn the_bundled_manifests_are_well_formed() {
        for bundled in BUNDLED {
            let manifest = PluginManifest::parse(bundled.manifest).unwrap();
            assert_eq!(manifest.api, ABI_VERSION);
            assert!(!manifest.capabilities.is_empty());
            for capability in &manifest.capabilities {
                assert!(
                    capability.starts_with(TRANSLATION_CAPABILITY),
                    "a bundled translation plugin claims a translation capability"
                );
            }
        }
    }

    #[test]
    fn the_bundled_id_list_matches_the_bundled_manifests() {
        for bundled in BUNDLED {
            let manifest = PluginManifest::parse(bundled.manifest).unwrap();
            assert!(
                BUNDLED_IDS.contains(&manifest.id.as_str()),
                "{} is bundled but not listed as such",
                manifest.id
            );
        }
    }

    #[test]
    fn a_manifest_is_refused_when_it_asks_for_another_api() {
        let text = r#"{"id":"x","api":2,"capabilities":["translation-provider:translate"]}"#;
        let error = PluginManifest::parse(text).unwrap_err();
        assert!(error.contains("plugin API 2"));
    }

    #[test]
    fn a_manifest_is_refused_when_it_names_nothing() {
        assert!(PluginManifest::parse(r#"{"api":1,"capabilities":["x"]}"#).is_err());
        assert!(PluginManifest::parse(r#"{"id":"x","api":1}"#).is_err());
        assert!(PluginManifest::parse("not json").is_err());
    }

    #[test]
    fn a_capability_is_named_for_its_kind() {
        assert_eq!(
            PluginManifest::translation_capability("romanize"),
            "translation-provider:romanize"
        );
    }
}
