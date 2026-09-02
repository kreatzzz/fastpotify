//! The registry `woofer://install?plugin=…` deep links resolve against.
//!
//! The marketplace site publishes a `registry.json`; a link arriving from
//! the OS names a plugin by its slug, and this module turns that slug into
//! the facts the app needs to ask an honest question: who published it,
//! what it can reach, and the hash the wasm it downloads must carry. The
//! catalog is approval-only, so what it says is the trust contract the
//! confirmation dialog shows.
//!
//! Both halves are checked for shape before anything is trusted: a slug is
//! short, lowercase, and hyphenated, and an entry's hash is a real sha256
//! digest with an absolute wasm address. A catalog that cannot say where
//! its wasm lives is refused here rather than mid-download.

use serde::Deserialize;

use std::time::Duration;

use crate::plugins::PluginManifest;

/// The catalog the deep links resolve against; Settings may override it
/// later, so keep this the single place the default lives.
pub const DEFAULT_CATALOG_URL: &str = "https://usewoofer.com/registry.json";

/// One plugin as the catalog lists it. Only what an install needs is
/// carried; the manifest inside the wasm stays the source of truth the
/// host believes once the bytes are here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogEntry {
    pub id: String,
    pub name: String,
    pub publisher: String,
    pub version: String,
    pub capabilities: Vec<String>,
    pub domains: Vec<String>,
    pub homepage: String,
    /// Where the wasm is downloaded from. Absolute, checked below.
    pub wasm: String,
    /// The wasm's size in bytes, as the catalog says; the download is not
    /// held to it, it is carried for a future "is this plausible" look.
    pub size: u64,
    /// The hex sha256 the downloaded wasm must hash to. Normalized to
    /// lowercase when the entry is read.
    pub sha256: String,
}

/// The registry document. Tolerant of the two shapes a hand-maintained
/// list drifts between: a bare array, or one under a `plugins` key.
#[derive(Deserialize)]
#[serde(untagged)]
enum Registry {
    Wrapped { plugins: Vec<Entry> },
    Bare(Vec<Entry>),
}

impl Registry {
    fn entries(self) -> Vec<Entry> {
        match self {
            Registry::Wrapped { plugins } => plugins,
            Registry::Bare(entries) => entries,
        }
    }
}

/// One catalog entry as the JSON carries it. Everything is defaulted so a
/// half-written entry reaches [`Entry::validate`], where the missing piece
/// gets named, instead of failing the whole registry's parse.
#[derive(Default, Deserialize)]
#[serde(default)]
struct Entry {
    id: String,
    name: String,
    publisher: String,
    version: String,
    capabilities: Vec<String>,
    domains: Vec<String>,
    homepage: String,
    wasm: String,
    size: u64,
    sha256: String,
}

impl Entry {
    /// Whether this entry is something an install may act on.
    fn validate(self) -> Result<CatalogEntry, String> {
        if self.id.trim().is_empty() {
            return Err("the catalog lists an entry with no id".to_string());
        }
        if self.name.trim().is_empty() {
            return Err(format!("the catalog entry {} names no plugin", self.id));
        }
        let hex = |c: char| c.is_ascii_hexdigit();
        if self.sha256.len() != 64 || !self.sha256.chars().all(hex) {
            return Err(format!(
                "the catalog entry {} carries a malformed sha256",
                self.id
            ));
        }
        let wasm_url = reqwest::Url::parse(&self.wasm).ok();
        if wasm_url.as_ref().is_none_or(|url| {
            url.scheme() != "https"
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.port().is_some_and(|port| port != 443)
        }) {
            return Err(format!(
                "the catalog entry {} has no secure absolute wasm address",
                self.id
            ));
        }
        Ok(CatalogEntry {
            sha256: self.sha256.to_ascii_lowercase(),
            id: self.id,
            name: self.name,
            publisher: self.publisher,
            version: self.version,
            capabilities: self.capabilities,
            domains: self.domains,
            homepage: self.homepage,
            wasm: self.wasm,
            size: self.size,
        })
    }
}

/// The plugin slug inside a `woofer://install?plugin=…` link, exactly the
/// shape the OS hands over as an argv. The query is read by hand -- the
/// `url` crate is not a dependency and a two-field query does not earn
/// one -- and the slug is percent-decoded before it is shaped.
pub fn parse_install_url(url: &str) -> Result<String, String> {
    let rest = url
        .strip_prefix("woofer://")
        .ok_or_else(|| format!("not a woofer:// link: {url}"))?;
    let (target, query) = rest
        .split_once('?')
        .ok_or_else(|| format!("the link carries no query: {url}"))?;
    if target.trim_end_matches('/') != "install" {
        return Err(format!("unsupported woofer link: woofer://{target}"));
    }
    for pair in query.split('&') {
        if let Some((key, value)) = pair.split_once('=')
            && key == "plugin"
        {
            return slug(value);
        }
    }
    Err(format!("the link names no plugin: {url}"))
}

/// A slug: percent-decoded, then held to the shape the catalog and the
/// file system agree on. Anything else is refused rather than fetched.
fn slug(value: &str) -> Result<String, String> {
    let decoded = percent_encoding::percent_decode_str(value)
        .decode_utf8()
        .map_err(|_| "the plugin slug is not valid UTF-8".to_string())?
        .into_owned();
    if decoded.is_empty() {
        return Err("the link names no plugin".to_string());
    }
    let shaped = decoded.len() <= 64
        && decoded
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !shaped {
        return Err(format!("“{decoded}” is not a plugin slug"));
    }
    Ok(decoded)
}

/// Looks a slug up in the catalog. Split from the network so tests can
/// feed the private `find` helper a registry on paper.
pub fn resolve(slug: &str, catalog_url: &str) -> Result<CatalogEntry, String> {
    let body = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|error| error.to_string())?
        .get(catalog_url)
        .send()
        .and_then(|response| response.error_for_status())
        .and_then(|response| response.text())
        .map_err(|error| format!("the catalog at {catalog_url} did not answer: {error}"))?;
    find(slug, &body)
}

/// The catalog entry for `slug`, from the registry's own text.
fn find(slug: &str, body: &str) -> Result<CatalogEntry, String> {
    let registry: Registry =
        serde_json::from_str(body).map_err(|error| format!("unreadable catalog: {error}"))?;
    let entry = registry
        .entries()
        .into_iter()
        .find(|entry| entry.id == slug)
        .ok_or_else(|| format!("no plugin called “{slug}” in the catalog"))?;
    entry.validate()
}

/// Whether `bytes` hash to the digest the catalog published. Case does not
/// matter; the entry's hash is normalized on the way in, and a catalog
/// that spelled it in capitals still gets an honest comparison.
pub fn matches_sha256(bytes: &[u8], sha256: &str) -> bool {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    hex.eq_ignore_ascii_case(sha256)
}

/// Checks that the manifest a downloaded module declares is the same trust
/// contract the user saw in the catalog. The manager performs its ordinary
/// ABI validation again before publishing; this comparison belongs here so
/// a catalog offer cannot silently install a different publisher, version,
/// capability, or domain allowlist.
pub fn manifest_matches(entry: &CatalogEntry, manifest: &PluginManifest) -> Result<(), String> {
    let mismatch =
        |field: &str| format!("the downloaded plugin manifest disagrees with the catalog {field}");
    if manifest.id != entry.id {
        return Err(mismatch("id"));
    }
    if manifest.name != entry.name {
        return Err(mismatch("name"));
    }
    if manifest.publisher != entry.publisher {
        return Err(mismatch("publisher"));
    }
    if manifest.version != entry.version {
        return Err(mismatch("version"));
    }
    if manifest.capabilities != entry.capabilities {
        return Err(mismatch("capabilities"));
    }
    if manifest.domains != entry.domains {
        return Err(mismatch("domains"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_install_link_yields_its_slug() {
        // #given / #when / #then
        assert_eq!(
            parse_install_url("woofer://install?plugin=translate"),
            Ok("translate".to_owned())
        );
        assert_eq!(
            parse_install_url("woofer://install?plugin=romanize-jp"),
            Ok("romanize-jp".to_owned())
        );
        // A trailing slash on the target is decoration, not a different
        // link; escapes in the slug are decoded before it is shaped.
        assert_eq!(
            parse_install_url("woofer://install/?plugin=romanize%2Djp"),
            Ok("romanize-jp".to_owned())
        );
        assert_eq!(
            parse_install_url("woofer://install?plugin=translate&ref=banner"),
            Ok("translate".to_owned())
        );
    }

    /// Links that are not ours to open are refused, as are install links
    /// that name nothing or name it badly.
    #[test]
    fn a_link_that_is_not_an_install_of_a_slug_is_refused() {
        // Another scheme entirely: the browser's business, never ours.
        assert!(parse_install_url("https://usewoofer.com/install?plugin=translate").is_err());
        assert!(parse_install_url("spotify:track:trk1").is_err());
        // A woofer link for something else, or for nothing.
        assert!(parse_install_url("woofer://uninstall?plugin=translate").is_err());
        assert!(parse_install_url("woofer://install").is_err());
        assert!(parse_install_url("woofer://install?ref=banner").is_err());
        assert!(parse_install_url("woofer://install?plugin=").is_err());
        // Slugs are short, lowercase, and hyphenated; paths and spaces
        // and capitals are not slugs.
        assert!(parse_install_url("woofer://install?plugin=../etc/passwd").is_err());
        assert!(parse_install_url("woofer://install?plugin=Translate").is_err());
        assert!(parse_install_url("woofer://install?plugin=a%20b").is_err());
        assert!(parse_install_url(&format!("woofer://install?plugin={}", "a".repeat(80))).is_err());
    }

    /// A registry on paper: the wrapped shape the site publishes, and the
    /// bare array a hand edit produces, both find the same entry.
    #[test]
    fn a_slug_finds_its_entry_in_the_registry() {
        let entry = r#"
            {
                "id": "translate",
                "name": "Translate",
                "publisher": "kreatzzz",
                "version": "1.0.0",
                "capabilities": ["provider:translate"],
                "domains": ["clients5.google.com"],
                "homepage": "https://github.com/kreatzzz/woofer-plugin-translate",
                "wasm": "https://usewoofer.com/plugins/translate.wasm",
                "size": 54321,
                "sha256": "AA11bb22cc33dd44ee55ff6600112233445566778899aabbccddeeff00112233"
            }
        "#;
        let wrapped = format!(r#"{{"plugins": [{entry}]}}"#);
        let bare = format!(r#"[{entry}]"#);
        for registry in [&wrapped, &bare] {
            let found = find("translate", registry).expect("the entry is found");
            assert_eq!(found.name, "Translate");
            assert_eq!(found.domains, vec!["clients5.google.com"]);
            // The hash comes out lowercase, whatever the catalog typed.
            assert_eq!(
                found.sha256,
                "aa11bb22cc33dd44ee55ff6600112233445566778899aabbccddeeff00112233"
            );
        }
        // Another slug, or an empty registry, is a miss with its name on it.
        assert!(find("nope", &wrapped).unwrap_err().contains("nope"));
        assert!(find("translate", "[]").is_err());
    }

    #[test]
    fn a_registry_that_cannot_be_read_or_trusted_is_refused() {
        assert!(
            find("translate", "not json")
                .unwrap_err()
                .contains("unreadable")
        );
        let sha = "aa11bb22cc33dd44ee55ff6600112233445566778899aabbccddeeff00112233";
        let base = |wasm: &str, sha256: &str| {
            format!(
                r#"[{{"id":"translate","name":"Translate","wasm":"{wasm}","sha256":"{sha256}"}}]"#
            )
        };
        // A hash that is not a sha256 digest, and a wasm address that is
        // not somewhere to fetch from, are both refused here rather than
        // surfacing mid-install.
        assert!(find("translate", &base("https://usewoofer.com/p.wasm", "abc")).is_err());
        assert!(
            find(
                "translate",
                &base(
                    "https://usewoofer.com/p.wasm",
                    "zz11bb22cc33dd44ee55ff6600112233445566778899aabbccddeeff00112233"
                )
            )
            .is_err()
        );
        assert!(find("translate", &base("/plugins/translate.wasm", sha)).is_err());
        // No name, no offer: the dialog would show an empty row.
        assert!(
            find(
                "translate",
                r#"[{"id":"translate","wasm":"https://x/p.wasm","sha256":""}]"#
            )
            .is_err()
        );
    }

    #[test]
    fn the_hash_verdict_is_the_catalogs_to_pass_or_fail() {
        let bytes = b"the wasm, honest and true";
        let digest = "9f2b0a55b8c7d6c17e1a3b5f4c8d2e6a7b9c0d1e2f3a4b5c6d7e8f9a0b1c2d3e";
        assert!(!matches_sha256(bytes, digest));
        // The real digest of those bytes, upper- and lowercase both accepted.
        let real: String = {
            use sha2::{Digest, Sha256};
            Sha256::digest(bytes)
                .iter()
                .map(|b| format!("{b:02X}"))
                .collect()
        };
        assert_eq!(real.len(), 64);
        assert!(matches_sha256(bytes, &real));
        assert!(matches_sha256(bytes, &real.to_ascii_lowercase()));
        assert!(!matches_sha256(bytes, &format!("{real}0")));
    }

    #[test]
    fn a_downloaded_manifest_must_match_the_catalog_offer() {
        let entry = CatalogEntry {
            id: "translate".into(),
            name: "Translate".into(),
            publisher: "kreatzzz".into(),
            version: "1.0.0".into(),
            capabilities: vec!["provider:translate".into()],
            domains: vec!["clients5.google.com".into()],
            homepage: String::new(),
            wasm: "https://usewoofer.com/plugins/translate.wasm".into(),
            size: 1,
            sha256: "a".repeat(64),
        };
        let manifest = PluginManifest {
            id: "translate".into(),
            name: "Translate".into(),
            publisher: "kreatzzz".into(),
            version: "1.0.0".into(),
            api: crate::plugins::ABI_VERSION,
            capabilities: vec!["provider:translate".into()],
            domains: vec!["clients5.google.com".into()],
            homepage: String::new(),
        };
        assert!(manifest_matches(&entry, &manifest).is_ok());
        for mutate in [
            |manifest: &mut PluginManifest| manifest.id = "other".into(),
            |manifest: &mut PluginManifest| manifest.publisher = "someone-else".into(),
            |manifest: &mut PluginManifest| manifest.version = "2.0.0".into(),
            |manifest: &mut PluginManifest| manifest.capabilities = vec!["provider:lyrics".into()],
            |manifest: &mut PluginManifest| manifest.domains = vec!["evil.example".into()],
        ] {
            let mut changed = manifest.clone();
            mutate(&mut changed);
            assert!(manifest_matches(&entry, &changed).is_err());
        }
    }
}
