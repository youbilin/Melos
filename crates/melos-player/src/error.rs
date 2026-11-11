//! Errors surfaced by the playback engine.

#[derive(Debug, thiserror::Error)]
pub enum PlayerError {
    #[error("no audio output device")]
    NoDevice,
    #[error("unsupported audio configuration: {0}")]
    Config(String),
    #[error("failed to open media: {0}")]
    Open(String),
    #[error("decode error: {0}")]
    Decode(String),
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("audio stream error: {0}")]
    Stream(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}
