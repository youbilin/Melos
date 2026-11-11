//! Wire types for the Subsonic / OpenSubsonic JSON API.
//!
//! Field names follow the API (`camelCase`); every optional field is defaulted
//! so a missing attribute from a less complete server does not fail the parse.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Song {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub album: Option<String>,
    #[serde(default)]
    pub album_id: Option<String>,
    #[serde(default)]
    pub artist: Option<String>,
    #[serde(default)]
    pub artist_id: Option<String>,
    #[serde(default)]
    pub track: Option<u32>,
    #[serde(default)]
    pub disc_number: Option<u32>,
    #[serde(default)]
    pub year: Option<u32>,
    #[serde(default)]
    pub genre: Option<String>,
    #[serde(default)]
    pub duration: Option<u32>,
    #[serde(default)]
    pub suffix: Option<String>,
    #[serde(default)]
    pub content_type: Option<String>,
    #[serde(default)]
    pub bit_rate: Option<u32>,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub cover_art: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub starred: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Album {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub artist: Option<String>,
    #[serde(default)]
    pub artist_id: Option<String>,
    #[serde(default)]
    pub cover_art: Option<String>,
    #[serde(default)]
    pub song_count: Option<u32>,
    #[serde(default)]
    pub duration: Option<u32>,
    #[serde(default)]
    pub year: Option<u32>,
    #[serde(default)]
    pub created: Option<String>,
    #[serde(default)]
    pub play_count: Option<u64>,
    #[serde(default)]
    pub starred: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Artist {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub album_count: Option<u32>,
    #[serde(default)]
    pub cover_art: Option<String>,
    #[serde(default)]
    pub artist_image_url: Option<String>,
    #[serde(default)]
    pub starred: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Playlist {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub comment: Option<String>,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default, rename = "public")]
    pub is_public: Option<bool>,
    #[serde(default)]
    pub song_count: Option<u32>,
    #[serde(default)]
    pub duration: Option<u32>,
    #[serde(default)]
    pub created: Option<String>,
    #[serde(default)]
    pub changed: Option<String>,
    #[serde(default)]
    pub cover_art: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfo {
    #[serde(default)]
    pub version: String,
    #[serde(default, rename = "type")]
    pub server_type: String,
    #[serde(default)]
    pub server_version: Option<String>,
    #[serde(default)]
    pub open_subsonic: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    #[serde(default)]
    pub artist: Vec<Artist>,
    #[serde(default)]
    pub album: Vec<Album>,
    #[serde(default)]
    pub song: Vec<Song>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AlbumWithSongs {
    #[serde(flatten)]
    pub album: Album,
    #[serde(default)]
    pub song: Vec<Song>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ArtistWithAlbums {
    #[serde(flatten)]
    pub artist: Artist,
    #[serde(default)]
    pub album: Vec<Album>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlaylistWithSongs {
    #[serde(flatten)]
    pub playlist: Playlist,
    #[serde(default)]
    pub entry: Vec<Song>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlbumListType {
    Newest,
    Recent,
    Frequent,
    Random,
    Starred,
    AlphabeticalByName,
    AlphabeticalByArtist,
    ByYear,
    ByGenre,
}

impl AlbumListType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Newest => "newest",
            Self::Recent => "recent",
            Self::Frequent => "frequent",
            Self::Random => "random",
            Self::Starred => "starred",
            Self::AlphabeticalByName => "alphabeticalByName",
            Self::AlphabeticalByArtist => "alphabeticalByArtist",
            Self::ByYear => "byYear",
            Self::ByGenre => "byGenre",
        }
    }
}

/// `getArtists` groups artists under an index letter.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ArtistsPayload {
    #[serde(default)]
    pub index: Vec<ArtistIndex>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ArtistIndex {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub artist: Vec<Artist>,
}

impl ArtistsPayload {
    pub fn into_artists(self) -> Vec<Artist> {
        self.index
            .into_iter()
            .flat_map(|index| index.artist)
            .collect()
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AlbumListPayload {
    #[serde(default)]
    pub album: Vec<Album>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PlaylistsPayload {
    #[serde(default)]
    pub playlist: Vec<Playlist>,
}
