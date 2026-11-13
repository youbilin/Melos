//! Settings persistence.
//!
//! A small JSON file holds the non-secret settings; the password is encrypted
//! with Windows DPAPI (CryptProtectData) before it is written beside them.
//! No database dependency — the settings file is tiny by design.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use thiserror::Error;

const FILE_NAME: &str = "settings.json";

#[derive(Debug, Error)]
pub enum SettingsError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization: {0}")]
    Json(#[from] serde_json::Error),
    #[error("dpapi: {0}")]
    Dpapi(&'static str),
}

/// One saved music source. The password is DPAPI-protected like the legacy
/// single-connection fields.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Source {
    #[serde(default)]
    pub server: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password_protected: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// Last connected server URL.
    #[serde(default)]
    pub server: String,
    /// Last username.
    #[serde(default)]
    pub username: String,
    /// DPAPI-protected password, hex encoded. Empty when unset.
    #[serde(default)]
    pub password_protected: String,
    #[serde(default = "default_volume")]
    pub volume: f32,
    /// None preserves the original format; otherwise request MP3 at this bitrate.
    #[serde(default)]
    pub max_bit_rate: Option<u32>,
    #[serde(default)]
    pub shuffle: bool,
    #[serde(default)]
    pub repeat: crate::RepeatMode,
    /// Saved music sources. The fields above mirror the active source.
    #[serde(default)]
    pub sources: Vec<Source>,
    #[serde(default)]
    pub active_source: usize,
}

fn default_volume() -> f32 {
    0.8
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            server: String::new(),
            username: String::new(),
            password_protected: String::new(),
            volume: default_volume(),
            max_bit_rate: None,
            shuffle: false,
            repeat: crate::RepeatMode::Off,
            sources: Vec::new(),
            active_source: 0,
        }
    }
}

impl Settings {
    fn path() -> Option<PathBuf> {
        std::env::var_os("MELOS_CONFIG_DIR")
            .map(PathBuf::from)
            .or_else(|| dirs::config_dir().map(|dir| dir.join("melos")))
            .map(|dir| dir.join(FILE_NAME))
    }

    /// Loads the saved settings, or defaults when the file does not exist.
    pub fn load() -> Self {
        let Some(path) = Self::path() else {
            return Self::default();
        };
        match fs::read_to_string(&path) {
            Ok(text) => {
                let mut settings: Self = serde_json::from_str(&text).unwrap_or_default();
                settings.normalize();
                settings
            }
            Err(_) => Self::default(),
        }
    }

    /// Persists the settings, protecting the password when present.
    pub fn save(&self) -> Result<(), SettingsError> {
        let Some(path) = Self::path() else {
            return Err(SettingsError::Dpapi("no config directory"));
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut snapshot = self.clone();
        snapshot.register_current();
        let temporary = path.with_extension("json.tmp");
        fs::write(&temporary, serde_json::to_string_pretty(&snapshot)?)?;
        fs::rename(&temporary, &path)?;
        Ok(())
    }

    /// Migrates the legacy single connection into the source list and mirrors
    /// the active source into the legacy fields.
    fn normalize(&mut self) {
        if self.sources.is_empty() && !self.server.is_empty() {
            self.sources.push(Source {
                server: self.server.clone(),
                username: self.username.clone(),
                password_protected: self.password_protected.clone(),
            });
            self.active_source = 0;
        }
        if self.sources.is_empty() {
            return;
        }
        if self.active_source >= self.sources.len() {
            self.active_source = 0;
        }
        let active = &self.sources[self.active_source];
        self.server = active.server.clone();
        self.username = active.username.clone();
        self.password_protected = active.password_protected.clone();
    }

    /// Makes sure the connection currently in the legacy fields is saved.
    fn register_current(&mut self) {
        if self.sources.is_empty() && !self.server.is_empty() {
            self.sources.push(Source {
                server: self.server.clone(),
                username: self.username.clone(),
                password_protected: self.password_protected.clone(),
            });
            self.active_source = 0;
        } else if let Some(active) = self.sources.get_mut(self.active_source) {
            active.server.clone_from(&self.server);
            active.username.clone_from(&self.username);
            active
                .password_protected
                .clone_from(&self.password_protected);
        }
    }

    pub fn sources(&self) -> &[Source] {
        &self.sources
    }

    pub fn active_source(&self) -> usize {
        self.active_source
    }

    /// Saves a new source without switching to it.
    pub fn add_source(
        &mut self,
        server: &str,
        username: &str,
        password: &str,
    ) -> Result<usize, SettingsError> {
        let protected = hex_encode(&protect(password.as_bytes())?);
        self.register_current();
        if let Some(index) = self
            .sources
            .iter()
            .position(|source| source.server == server && source.username == username)
        {
            self.sources[index].password_protected = protected;
            return Ok(index);
        }
        self.sources.push(Source {
            server: server.to_owned(),
            username: username.to_owned(),
            password_protected: protected,
        });
        Ok(self.sources.len() - 1)
    }

    /// Stores the connection the user just authenticated and makes it active.
    pub fn adopt_connection(
        &mut self,
        server: &str,
        username: &str,
        password: &str,
    ) -> Result<(), SettingsError> {
        let protected = hex_encode(&protect(password.as_bytes())?);
        self.register_current();
        let index = match self
            .sources
            .iter()
            .position(|source| source.server == server && source.username == username)
        {
            Some(index) => {
                self.sources[index].password_protected = protected;
                index
            }
            None => {
                self.sources.push(Source {
                    server: server.to_owned(),
                    username: username.to_owned(),
                    password_protected: protected,
                });
                self.sources.len() - 1
            }
        };
        self.active_source = index;
        let active = &self.sources[index];
        self.server = active.server.clone();
        self.username = active.username.clone();
        self.password_protected = active.password_protected.clone();
        Ok(())
    }

    /// Mirrors the active source into the connection fields.
    pub fn switch_source(&mut self, index: usize) -> bool {
        if index >= self.sources.len() {
            return false;
        }
        if index != self.active_source {
            self.register_current();
            self.active_source = index;
        }
        let active = &self.sources[index];
        self.server = active.server.clone();
        self.username = active.username.clone();
        self.password_protected = active.password_protected.clone();
        true
    }

    /// Removes an inactive source; the active one must be switched away first.
    pub fn remove_source(&mut self, index: usize) -> bool {
        if self.sources.len() <= 1 || index >= self.sources.len() || index == self.active_source {
            return false;
        }
        self.sources.remove(index);
        if index < self.active_source {
            self.active_source -= 1;
        }
        true
    }

    /// Sets the plaintext password; it is DPAPI-encrypted before storage.
    pub fn set_password(&mut self, password: &str) -> Result<(), SettingsError> {
        self.password_protected = hex_encode(&protect(password.as_bytes())?);
        Ok(())
    }

    /// Decrypts the stored password, or `None` when absent or undecryptable.
    pub fn password(&self) -> Option<String> {
        if self.password_protected.is_empty() {
            return None;
        }
        let blob = hex_decode(&self.password_protected)?;
        unprotect(&blob)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
    }
}

#[cfg(windows)]
fn protect(data: &[u8]) -> Result<Vec<u8>, SettingsError> {
    use windows::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CryptProtectData};

    unsafe {
        let input = CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB::default();
        CryptProtectData(
            &input,
            None,
            None,
            None,
            None,
            Default::default(),
            &mut output,
        )
        .map_err(|_| SettingsError::Dpapi("CryptProtectData failed"))?;

        let slice = std::slice::from_raw_parts(output.pbData, output.cbData as usize);
        let bytes = slice.to_vec();
        windows::Win32::Foundation::LocalFree(Some(windows::Win32::Foundation::HLOCAL(
            output.pbData as *mut _,
        )));
        Ok(bytes)
    }
}

#[cfg(windows)]
fn unprotect(data: &[u8]) -> Result<Vec<u8>, SettingsError> {
    use windows::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CryptUnprotectData};

    unsafe {
        let input = CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB::default();
        CryptUnprotectData(
            &input,
            None,
            None,
            None,
            None,
            Default::default(),
            &mut output,
        )
        .map_err(|_| SettingsError::Dpapi("CryptUnprotectData failed"))?;

        let slice = std::slice::from_raw_parts(output.pbData, output.cbData as usize);
        let bytes = slice.to_vec();
        windows::Win32::Foundation::LocalFree(Some(windows::Win32::Foundation::HLOCAL(
            output.pbData as *mut _,
        )));
        Ok(bytes)
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hex_decode(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    (0..text.len() / 2)
        .map(|i| u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).ok())
        .collect()
}

#[cfg(not(windows))]
fn protect(_: &[u8]) -> Result<Vec<u8>, SettingsError> {
    Err(SettingsError::Dpapi(
        "credential storage is currently Windows-only",
    ))
}

#[cfg(not(windows))]
fn unprotect(_: &[u8]) -> Result<Vec<u8>, SettingsError> {
    Err(SettingsError::Dpapi(
        "credential storage is currently Windows-only",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(windows)]
    fn roundtrip_password() {
        let mut settings = Settings::default();
        settings.set_password("test-password-中文").unwrap();
        assert_eq!(settings.password().as_deref(), Some("test-password-中文"));
    }

    #[test]
    fn empty_password_is_none() {
        let settings = Settings::default();
        assert!(settings.password().is_none());
    }

    #[test]
    fn old_settings_get_playback_defaults() {
        let settings: Settings =
            serde_json::from_str(r#"{"server":"https://example.test"}"#).unwrap();
        assert_eq!(settings.volume, 0.8);
        assert_eq!(settings.max_bit_rate, None);
        assert!(!settings.shuffle);
        assert_eq!(settings.repeat, crate::RepeatMode::Off);
    }

    #[test]
    fn playback_modes_survive_settings_roundtrip() {
        let settings = Settings {
            shuffle: true,
            repeat: crate::RepeatMode::One,
            ..Default::default()
        };
        let restored: Settings =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert!(restored.shuffle);
        assert_eq!(restored.repeat, crate::RepeatMode::One);
    }

    #[test]
    fn legacy_connection_becomes_a_saved_source() {
        let mut settings: Settings =
            serde_json::from_str(r#"{"server":"https://a.test","username":"u"}"#).unwrap();
        settings.normalize();
        assert_eq!(settings.sources().len(), 1);
        assert_eq!(settings.active_source(), 0);
        assert_eq!(settings.server, "https://a.test");
    }

    #[test]
    #[cfg(windows)]
    fn adding_a_source_keeps_the_current_connection() {
        let mut settings = Settings {
            server: "https://old".into(),
            username: "old".into(),
            ..Default::default()
        };
        settings.set_password("oldpass").unwrap();

        let index = settings
            .add_source("https://new", "new", "newpass")
            .unwrap();
        assert_eq!(index, 1);
        assert_eq!(settings.active_source(), 0);
        assert_eq!(settings.server, "https://old");
        assert_eq!(settings.password().as_deref(), Some("oldpass"));

        assert!(settings.switch_source(1));
        assert_eq!(settings.server, "https://new");
        assert_eq!(settings.password().as_deref(), Some("newpass"));

        assert!(settings.switch_source(0));
        assert_eq!(settings.password().as_deref(), Some("oldpass"));
    }

    #[test]
    #[cfg(windows)]
    fn a_second_login_joins_the_source_list() {
        let mut settings = Settings::default();
        settings.adopt_connection("https://a", "u", "p").unwrap();
        settings.adopt_connection("https://b", "u", "q").unwrap();

        assert_eq!(settings.sources().len(), 2);
        assert_eq!(settings.active_source(), 1);
        assert_eq!(settings.server, "https://b");
        assert!(settings.switch_source(0));
        assert_eq!(settings.server, "https://a");
        assert_eq!(settings.password().as_deref(), Some("p"));
    }

    #[test]
    fn the_active_source_cannot_be_removed() {
        let mut settings = Settings {
            sources: vec![
                Source {
                    server: "https://a".into(),
                    username: "u".into(),
                    ..Default::default()
                },
                Source {
                    server: "https://b".into(),
                    username: "u".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        assert!(!settings.remove_source(0));
        assert!(settings.remove_source(1));
        assert_eq!(settings.sources().len(), 1);
    }

    #[test]
    fn malformed_password_blob_does_not_panic() {
        for blob in ["0", "xx", "你", "é", "aé"] {
            assert!(hex_decode(blob).is_none());
        }
    }
}
