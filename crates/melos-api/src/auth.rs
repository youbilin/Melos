//! Subsonic token authentication.
//!
//! Every request carries `u`, `v`, `c`, `s` (salt) and `t` (token), where
//! `t = md5(password + salt)`. The password never leaves the process.

use std::fmt::Write as _;

use md5::{Digest, Md5};

/// Credentials for one Subsonic server.
#[derive(Debug, Clone)]
pub struct Credentials {
    pub username: String,
    password: String,
}

impl Credentials {
    pub fn new(username: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            username: username.into(),
            password: password.into(),
        }
    }

    /// Computes the token for one request: `md5(password + salt)` as lowercase hex.
    pub fn token(&self, salt: &str) -> String {
        let mut hasher = Md5::new();
        hasher.update(self.password.as_bytes());
        hasher.update(salt.as_bytes());
        format!("{:x}", hasher.finalize())
    }
}

/// Produces a salt that is unique per request.
///
/// The salt is not a secret; Subsonic only needs it to stop replay of a token.
/// Using a dependency-free generator keeps the API crate small.
pub fn random_salt(bytes: usize) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let pid = u64::from(std::process::id());

    let mut state = nanos ^ (pid << 32) ^ n.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let mut out = String::with_capacity(bytes.saturating_mul(2));
    for _ in 0..bytes {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let _ = write!(out, "{:02x}", (state & 0xff) as u8);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_matches_subsonic_example() {
        // Documented example: password "sesame", salt "c19b2d".
        let creds = Credentials::new("user", "sesame");
        assert_eq!(creds.token("c19b2d"), "26719a1196d2a940705a59634eb18eab");
    }

    #[test]
    fn salts_are_unique_and_long_enough() {
        let a = random_salt(8);
        let b = random_salt(8);
        assert_eq!(a.len(), 16);
        assert_ne!(a, b);
    }
}
