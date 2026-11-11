//! Errors surfaced by the Subsonic client.

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("wrong credentials (code 40)")]
    Auth,
    #[error("not authorized (code 50)")]
    NotAuthorized,
    #[error("not found (code 70)")]
    NotFound,
    #[error("server error (code {code}): {message}")]
    Server { code: i32, message: String },
    #[error("server reported failure without an error body")]
    UnknownFailure,
    #[error("invalid server URL: {0}")]
    InvalidUrl(String),
    #[error("transport error: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("malformed response: {0}")]
    Decode(String),
}

impl Error {
    /// Maps a Subsonic error code onto a typed variant.
    pub(crate) fn from_code(code: i32, message: String) -> Self {
        match code {
            // 41 is "token auth not supported for LDAP users"; from the caller's
            // point of view it is still an authentication problem.
            40 | 41 => Self::Auth,
            50 => Self::NotAuthorized,
            70 => Self::NotFound,
            _ => Self::Server { code, message },
        }
    }
}
