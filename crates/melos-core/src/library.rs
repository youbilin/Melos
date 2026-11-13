//! Ordering helpers for the whole-library song list.

use melos_api::Song;

/// Groups a library listing by artist, album, disc and track so the flat
/// "all songs" page reads like a collection instead of server order.
pub fn sort_library_songs(songs: &mut [Song]) {
    songs.sort_by_cached_key(|song| {
        (
            song.artist.is_none(),
            song.artist.as_deref().unwrap_or_default().to_lowercase(),
            song.album.is_none(),
            song.album.as_deref().unwrap_or_default().to_lowercase(),
            song.disc_number.unwrap_or(0),
            song.track.unwrap_or(0),
            song.title.to_lowercase(),
        )
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(title: &str, artist: Option<&str>, album: Option<&str>, track: Option<u32>) -> Song {
        Song {
            id: title.into(),
            title: title.into(),
            artist: artist.map(str::to_owned),
            album: album.map(str::to_owned),
            track,
            ..Song::default()
        }
    }

    #[test]
    fn groups_by_artist_album_and_track() {
        let mut songs = vec![
            song("B2", Some("Beta"), Some("Second"), Some(2)),
            song("A1", Some("Alpha"), Some("First"), Some(2)),
            song("A0", Some("Alpha"), Some("First"), Some(1)),
            song("Unknown", None, Some("Misc"), Some(1)),
        ];
        sort_library_songs(&mut songs);
        let titles: Vec<_> = songs.iter().map(|song| song.title.as_str()).collect();
        assert_eq!(titles, ["A0", "A1", "B2", "Unknown"]);
    }
}
