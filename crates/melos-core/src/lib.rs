//! Domain logic shared by every frontend.
//!
//! This crate must never depend on a UI toolkit. CI checks that neither gpui
//! nor ratatui appear in its dependency graph.

pub mod artwork;
pub mod cover_cache;
pub mod lyrics;
pub mod queue;
pub mod settings;

pub use queue::{Queue, RepeatMode};
pub use settings::Settings;
