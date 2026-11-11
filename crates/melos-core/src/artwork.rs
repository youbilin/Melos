//! Playback artwork is independent of whichever album page is open.

use melos_api::{Album, Song};

pub fn playback_cover_candidates(
    song: &Song,
    albums: &[Album],
    detail: Option<&Album>,
) -> Vec<String> {
    let album = detail
        .filter(|album| song.album_id.as_deref() == Some(album.id.as_str()))
        .or_else(|| {
            albums
                .iter()
                .find(|album| song.album_id.as_deref() == Some(album.id.as_str()))
        });
    let mut ids = Vec::new();
    for id in [
        song.cover_art.as_deref(),
        album.and_then(|album| album.cover_art.as_deref()),
        song.album_id.as_deref(),
        Some(song.id.as_str()),
    ]
    .into_iter()
    .flatten()
    {
        if !id.is_empty() && !ids.iter().any(|existing| existing == id) {
            ids.push(id.to_owned());
        }
    }
    ids
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn song_artwork_is_requested_even_when_album_artwork_is_different() {
        let song = Song {
            id: "song".into(),
            album_id: Some("album".into()),
            cover_art: Some("song-art".into()),
            ..Default::default()
        };
        let album = Album {
            id: "album".into(),
            cover_art: Some("album-art".into()),
            ..Default::default()
        };
        assert_eq!(
            playback_cover_candidates(&song, &[album], None),
            ["song-art", "album-art", "album", "song"]
        );
    }

    #[test]
    fn search_song_without_cover_uses_album_or_song_identifier() {
        let song = Song {
            id: "search-song".into(),
            album_id: Some("album".into()),
            ..Default::default()
        };
        assert_eq!(
            playback_cover_candidates(&song, &[], None),
            ["album", "search-song"]
        );
        let album = Album {
            id: "unrelated".into(),
            cover_art: Some("wrong-art".into()),
            ..Default::default()
        };
        assert_eq!(
            playback_cover_candidates(&song, &[], Some(&album)),
            ["album", "search-song"]
        );
    }
}
