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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Settings {
    /// Last connected server URL, e.g. `http://10.159.39.64:4533/`.
    #[serde(default)]
    pub server: String,
    /// Last username.
    #[serde(default)]
    pub username: String,
    /// DPAPI-protected password, hex encoded. Empty when unset.
    #[serde(default)]
    pub password_protected: String,
}

impl Settings {
    fn path() -> Option<PathBuf> {
        dirs::config_dir().map(|dir| dir.join("melos").join(FILE_NAME))
    }

    /// Loads the saved settings, or defaults when the file does not exist.
    pub fn load() -> Self {
        let Some(path) = Self::path() else {
            return Self::default();
        };
        match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
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
        fs::write(&path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    /// Sets the plaintext password; it is DPAPI-encrypted before storage.
    pub fn set_password(&mut self, password: &str) {
        self.password_protected = match protect(password.as_bytes()) {
            Ok(blob) => hex_encode(&blob),
            Err(_) => String::new(),
        };
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
    use windows::Win32::Security::Cryptography::{
        CryptProtectData, CRYPT_INTEGER_BLOB,
    };

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

        let slice =
            std::slice::from_raw_parts(output.pbData, output.cbData as usize);
        let bytes = slice.to_vec();
        windows::Win32::Foundation::LocalFree(Some(
            windows::Win32::Foundation::HLOCAL(output.pbData as *mut _),
        ));
        Ok(bytes)
    }
}

#[cfg(windows)]
fn unprotect(data: &[u8]) -> Result<Vec<u8>, SettingsError> {
    use windows::Win32::Security::Cryptography::{
        CryptUnprotectData, CRYPT_INTEGER_BLOB,
    };

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

        let slice =
            std::slice::from_raw_parts(output.pbData, output.cbData as usize);
        let bytes = slice.to_vec();
        windows::Win32::Foundation::LocalFree(Some(
            windows::Win32::Foundation::HLOCAL(output.pbData as *mut _),
        ));
        Ok(bytes)
    }
}

#[cfg(windows)]
fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(windows)]
fn hex_decode(text: &str) -> Option<Vec<u8>> {
    (0..text.len() / 2)
        .map(|i| u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_password() {
        let mut settings = Settings::default();
        settings.set_password("996699");
        assert_eq!(settings.password().as_deref(), Some("996699"));
    }

    #[test]
    fn empty_password_is_none() {
        let settings = Settings::default();
        assert!(settings.password().is_none());
    }
}
