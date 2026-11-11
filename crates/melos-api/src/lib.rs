//! Subsonic / OpenSubsonic API client.
//!
//! This crate has no UI dependency: both the GPUI and the TUI frontends use it.

pub mod auth;
pub mod client;
pub mod error;
pub mod model;

pub use auth::Credentials;
pub use client::{
    API_VERSION, CLIENT_ID, Client, DEFAULT_TRANSCODE_BITRATE, StreamQuality, can_decode,
};
pub use error::Error;
pub use model::{
    Album, AlbumListType, AlbumWithSongs, Artist, ArtistWithAlbums, LyricsLine, PlainLyrics,
    Playlist, PlaylistWithSongs, SearchResult, ServerInfo, Song, StructuredLyrics,
};
