//! Plugins: sandboxed modules that contribute what a data provider can,
//! never what playback or the machine need.
//!
//! A plugin is one `.wasm` file (pure compute, no imports) plus a manifest.
//! The host [`host`] runs it inside wasmi with fuel and a memory cap, does
//! every fetch itself, and refuses anything the manifest does not allow.
//! [`manager`] knows which plugins are installed and the order each kind
//! asks them in. The app ships none: the built-in engines answer when no
//! plugin is installed, and the catalog at usewoofer.com is where plugins
//! come from.

pub mod catalog;
pub mod host;
pub mod manager;

use serde::{Deserialize, Serialize};

/// The plugin API this host speaks. A module wanting another version is
/// refused at the door, not mid-run.
pub const ABI_VERSION: i32 = 1;

/// The capability prefix every provider plugin must claim, followed by the
/// kind it answers: `lyrics`, `translate`, or `romanize`.
pub const PROVIDER_CAPABILITY: &str = "provider:";

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

    /// The capability a provider plugin of `kind` must claim.
    pub fn provider_capability(kind: &str) -> String {
        format!("{PROVIDER_CAPABILITY}{kind}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wasm the plugin crates last built, when they did: the live
    /// test answers through those. Nothing in the app carries plugins,
    /// so the test skips quietly when the artifacts are not around.
    fn built_artifact(id: &str) -> Option<Vec<u8>> {
        std::fs::read(format!(
            "plugins/{id}/target/wasm32-unknown-unknown/release/woofer_plugin_{id}.wasm"
        ))
        .ok()
    }

    #[test]
    #[ignore = "speaks to the live endpoint, and needs the plugin crates built"]
    fn the_built_plugins_answer_over_the_network() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let input = ["こんにちは、世界"];
        for id in ["translate", "romanize"] {
            let Some(wasm) = built_artifact(id) else {
                eprintln!("the {id} crate has no built wasm; skipping");
                continue;
            };
            let manifest = host::validate(&wasm).expect("the plugin loads");
            let kind = match manifest.capabilities.first().map(String::as_str) {
                Some(capability) => capability
                    .strip_prefix(PROVIDER_CAPABILITY)
                    .expect("a provider capability")
                    .to_string(),
                None => panic!("the {id} plugin claims no capability"),
            };
            let found =
                runtime.block_on(host::run_translation(&wasm, &manifest, &kind, "en", &input));
            let translation = found.expect("the plugin answers").expect("not a miss");
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
    fn a_manifest_is_refused_when_it_asks_for_another_api() {
        let text = r#"{"id":"x","api":2,"capabilities":["provider:translate"]}"#;
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
            PluginManifest::provider_capability("romanize"),
            "provider:romanize"
        );
        assert_eq!(
            PluginManifest::provider_capability("lyrics"),
            "provider:lyrics"
        );
    }
}
