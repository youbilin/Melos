//! Audio playback engine. UI-agnostic.
//!
//! Decoding uses symphonia, output uses cpal, and the HTTP source supports
//! Range requests so seeking works against a Navidrome `format=raw` stream.

mod buffer;
pub mod decoder;
pub mod engine;
pub mod error;
pub mod source;

pub use decoder::Decoder;
pub use engine::{Engine, EngineSnapshot, StreamSpec};
pub use error::PlayerError;
pub use source::{open_file, open_url};
