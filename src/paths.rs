//! Where Woofer keeps its files.
//!
//! Configuration, durable state (Spotify credentials), and disposable caches
//! (audio, artwork) live in the platform's conventional directories, so
//! clearing a cache never signs the user out and a config backup never
//! contains a credential.

use std::path::{Path, PathBuf};

use directories::ProjectDirs;

#[derive(Clone, Debug)]
pub struct AppDirs {
    pub config: PathBuf,
    pub state: PathBuf,
    pub cache: PathBuf,
}

impl AppDirs {
    pub fn discover() -> Self {
        let project = ProjectDirs::from("me", "kreatzzz", "woofer");
        match project {
            Some(project) => {
                let dirs = Self {
                    config: project.config_dir().to_path_buf(),
                    state: project
                        .state_dir()
                        .map(|path| path.to_path_buf())
                        .unwrap_or_else(|| project.data_local_dir().to_path_buf()),
                    cache: project.cache_dir().to_path_buf(),
                };
                adopt_legacy_dirs(&dirs);
                dirs
            }
            None => {
                let fallback = std::env::current_dir().unwrap_or_default();
                Self {
                    config: fallback.join("woofer-config"),
                    state: fallback.join("woofer-state"),
                    cache: fallback.join("woofer-cache"),
                }
            }
        }
    }

    pub fn settings_file(&self) -> PathBuf {
        self.config.join("settings.json")
    }

    /// Winamp skins the listener has added, as `.wsz` files or folders.
    pub fn skins_dir(&self) -> PathBuf {
        self.config.join("skins")
    }

    pub fn session_file(&self) -> PathBuf {
        self.state.join("session.json")
    }

    pub fn shared_web_token_file(&self) -> PathBuf {
        self.state.join("shared_web_api_token.json")
    }

    pub fn personal_web_token_file(&self) -> PathBuf {
        self.state.join("personal_web_api_token.json")
    }

    pub fn legacy_web_token_file(&self) -> PathBuf {
        self.state.join("web_api_token.json")
    }

    /// The log of the current run, replaced at every start.
    pub fn log_file(&self) -> PathBuf {
        self.state.join("woofer.log")
    }

    /// Where a panic is recorded before the process dies of it.
    pub fn panic_log(&self) -> PathBuf {
        self.state.join("panic.log")
    }

    pub fn credentials_dir(&self) -> PathBuf {
        self.state.join("credentials")
    }

    pub fn volume_dir(&self) -> PathBuf {
        self.state.join("volume")
    }

    pub fn audio_cache_dir(&self) -> PathBuf {
        self.cache.join("audio")
    }

    pub fn art_cache_dir(&self) -> PathBuf {
        self.cache.join("art")
    }

    pub fn lyrics_cache_dir(&self) -> PathBuf {
        self.cache.join("lyrics")
    }

    pub fn playlist_cache_dir(&self) -> PathBuf {
        self.cache.join("playlists")
    }

    pub fn translations_cache_dir(&self) -> PathBuf {
        self.cache.join("translations")
    }

    /// Where installed plugins are kept, next to the durable state: a
    /// cleared cache must never uninstall one.
    pub fn plugins_dir(&self) -> PathBuf {
        self.state.join("plugins")
    }

    pub fn account_playlist_cache_dir(&self, account_id: &str) -> PathBuf {
        self.playlist_cache_dir().join(account_id)
    }

    pub fn ensure(&self) -> std::io::Result<()> {
        for dir in [&self.config, &self.state, &self.cache] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }
}

/// The directories the project kept under its former name. When they are
/// still there and the new ones are not, they move over, so an update
/// keeps the sign-in, the settings, and the caches it already paid for.
fn adopt_legacy_dirs(dirs: &AppDirs) {
    let Some(project) = ProjectDirs::from("me", "paolino", "fastpotify") else {
        return;
    };
    let legacy = AppDirs {
        config: project.config_dir().to_path_buf(),
        state: project
            .state_dir()
            .map(|path| path.to_path_buf())
            .unwrap_or_else(|| project.data_local_dir().to_path_buf()),
        cache: project.cache_dir().to_path_buf(),
    };
    for (old, new) in [
        (&legacy.config, &dirs.config),
        (&legacy.state, &dirs.state),
        (&legacy.cache, &dirs.cache),
    ] {
        if old.exists() && !new.exists() && move_tree(old, new).is_err() {
            log::warn!(
                "could not move {} to {}; starting fresh there",
                old.display(),
                new.display()
            );
        }
    }
}

/// Moves a directory, crossing a volume the slow way if a rename cannot.
fn move_tree(old: &Path, new: &Path) -> std::io::Result<()> {
    std::fs::rename(old, new).or_else(|_| -> std::io::Result<()> {
        // A rename that fails with an error other than a volume crossing
        // fails the copy too, so trying it costs nothing but a moment.
        std::fs::create_dir_all(new)?;
        for entry in std::fs::read_dir(old)? {
            let entry = entry?;
            let target = new.join(entry.file_name());
            if entry.file_type()?.is_dir() {
                move_tree(&entry.path(), &target)?;
            } else {
                std::fs::copy(entry.path(), &target)?;
            }
        }
        std::fs::remove_dir_all(old)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_update_carries_its_files_to_the_new_name() {
        let root = std::env::temp_dir().join(format!("woofer-migrate-{}", std::process::id()));
        let old = root.join("old");
        std::fs::create_dir_all(old.join("credentials")).unwrap();
        std::fs::write(old.join("settings.json"), "{}").unwrap();
        let new = root.join("new").join("nested");

        move_tree(&old, &new).unwrap();

        assert!(new.join("settings.json").is_file());
        assert!(new.join("credentials").is_dir());
        assert!(!old.exists());
        let _ = std::fs::remove_dir_all(root);
    }
}
