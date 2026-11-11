//! Subsonic / OpenSubsonic HTTP client.
//!
//! Authentication uses the salted token scheme from API 1.13+: every request
//! gets a fresh salt and `t = md5(password + salt)`. The password never appears
//! in a URL or log line.

use std::time::Duration;

use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::auth::{self, Credentials};
use crate::error::Error;
use crate::model::*;

/// Client identifier reported to the server (`c` parameter).
pub const CLIENT_ID: &str = "melos";

/// Protocol version reported to the server (`v` parameter).
pub const API_VERSION: &str = "1.16.1";

/// Default bitrate used when the caller asks for a transcode without a value.
pub const DEFAULT_TRANSCODE_BITRATE: u32 = 192;

/// How the audio stream should be requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamQuality {
    /// Serve the original bytes when the local decoder supports them.
    Original,
    /// Ask the server to transcode to MP3 at this bitrate.
    Transcode { max_bit_rate: u32 },
}

impl Default for StreamQuality {
    fn default() -> Self {
        Self::Transcode {
            max_bit_rate: DEFAULT_TRANSCODE_BITRATE,
        }
    }
}

/// Returns true when the bundled decoder (symphonia 0.6) can decode `suffix`.
///
/// Opus is deliberately absent: symphonia 0.6 does not ship an Opus decoder, and
/// Navidrome defaults its transcodes to Opus. Callers must request `format=mp3`
/// explicitly when transcoding.
pub fn can_decode(suffix: &str) -> bool {
    matches!(
        suffix.trim_start_matches('.').to_ascii_lowercase().as_str(),
        "mp3"
            | "flac"
            | "ogg"
            | "oga"
            | "m4a"
            | "mp4"
            | "aac"
            | "alac"
            | "wav"
            | "aiff"
            | "aif"
            | "caf"
            | "mka"
    )
}

#[derive(Debug, Clone)]
pub struct Client {
    base: reqwest::Url,
    credentials: Credentials,
    http: reqwest::Client,
}

impl Client {
    /// Builds a client for `server`, for example `http://192.168.1.10:4533`.
    ///
    /// A trailing `/rest` path is tolerated so a URL copied from the web UI
    /// still works.
    pub fn new(server: &str, credentials: Credentials) -> Result<Self, Error> {
        let mut base = reqwest::Url::parse(server).map_err(|e| Error::InvalidUrl(e.to_string()))?;

        if !matches!(base.scheme(), "http" | "https") {
            return Err(Error::InvalidUrl(format!(
                "scheme must be http or https, got {}",
                base.scheme()
            )));
        }

        let trimmed = base.path().trim_end_matches('/').to_string();
        let trimmed = trimmed
            .strip_suffix("/rest")
            .unwrap_or(&trimmed)
            .to_string();
        let path = if trimmed.is_empty() {
            "/".to_string()
        } else {
            format!("{trimmed}/")
        };
        base.set_path(&path);
        base.set_query(None);
        base.set_fragment(None);

        let http = reqwest::Client::builder()
            .user_agent(concat!("Melos/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()?;

        Ok(Self {
            base,
            credentials,
            http,
        })
    }

    pub fn base_url(&self) -> &reqwest::Url {
        &self.base
    }

    /// Builds a request URL with authentication and protocol parameters.
    ///
    /// Kept public for callers that hand the URL to a streaming backend.
    pub fn endpoint(&self, name: &str, params: &[(&str, String)]) -> reqwest::Url {
        let mut url = self
            .base
            .join(&format!("rest/{name}"))
            .expect("base URL always ends with a slash");

        let salt = auth::random_salt(8);
        let token = self.credentials.token(&salt);

        {
            let mut query = url.query_pairs_mut();
            query
                .append_pair("u", &self.credentials.username)
                .append_pair("t", &token)
                .append_pair("s", &salt)
                .append_pair("v", API_VERSION)
                .append_pair("c", CLIENT_ID)
                .append_pair("f", "json");
            for (key, value) in params {
                query.append_pair(key, value);
            }
        }

        url
    }

    /// Sends a request, retrying transient connection failures twice with a
    /// short backoff. Server-side errors are never retried.
    async fn send(&self, url: reqwest::Url) -> Result<reqwest::Response, Error> {
        let mut attempt = 0u32;
        loop {
            match self.http.get(url.clone()).send().await {
                Ok(response) => return Ok(response),
                Err(error) if attempt < 2 && (error.is_timeout() || error.is_connect()) => {
                    attempt += 1;
                    tokio::time::sleep(Duration::from_millis(200 * u64::from(attempt))).await;
                }
                Err(error) => return Err(error.into()),
            }
        }
    }

    async fn get(
        &self,
        name: &str,
        params: &[(&str, String)],
    ) -> Result<Map<String, Value>, Error> {
        let url = self.endpoint(name, params);
        let response = self.send(url).await?;
        Self::decode_response(response).await
    }

    async fn post(
        &self,
        name: &str,
        params: &[(&str, String)],
    ) -> Result<Map<String, Value>, Error> {
        // Playlist writes are never automatically retried: creating a playlist
        // twice after a lost response would duplicate remote user data.
        let response = self
            .http
            .post(self.endpoint(name, &[]))
            .form(params)
            .send()
            .await?;
        Self::decode_response(response).await
    }

    async fn decode_response(response: reqwest::Response) -> Result<Map<String, Value>, Error> {
        let status = response.status();
        let body = response.text().await?;

        let envelope: Envelope = match serde_json::from_str(&body) {
            Ok(envelope) => envelope,
            Err(_) => {
                let snippet: String = body.chars().take(200).collect();
                return Err(Error::Decode(format!("HTTP {status}: {snippet}")));
            }
        };

        let response = envelope.response;
        if response.status == "ok" {
            Ok(response.payload)
        } else {
            Err(match response.error {
                Some(error) => Error::from_code(error.code, error.message.unwrap_or_default()),
                None => Error::UnknownFailure,
            })
        }
    }

    /// `ping`: connectivity and server identity.
    pub async fn ping(&self) -> Result<ServerInfo, Error> {
        let payload = self.get("ping", &[]).await?;
        let value = Value::Object(payload);
        serde_json::from_value(value).map_err(|e| Error::Decode(format!("ping: {e}")))
    }

    /// `getArtists`: all artists, flattened out of the index letter groups.
    pub async fn artists(&self) -> Result<Vec<Artist>, Error> {
        let mut payload = self.get("getArtists", &[]).await?;
        let artists: ArtistsPayload = take(&mut payload, "artists")?;
        Ok(artists.into_artists())
    }

    /// `getArtist`: one artist with their albums.
    pub async fn artist(&self, id: &str) -> Result<ArtistWithAlbums, Error> {
        let mut payload = self.get("getArtist", &[("id", id.to_owned())]).await?;
        take(&mut payload, "artist")
    }

    /// `getAlbum`: one album with its songs.
    pub async fn album(&self, id: &str) -> Result<AlbumWithSongs, Error> {
        let mut payload = self.get("getAlbum", &[("id", id.to_owned())]).await?;
        take(&mut payload, "album")
    }

    /// `getAlbumList2`: paged album lists. `size` is clamped to the API maximum.
    pub async fn album_list(
        &self,
        kind: AlbumListType,
        size: u32,
        offset: u32,
    ) -> Result<Vec<Album>, Error> {
        let mut payload = self
            .get(
                "getAlbumList2",
                &[
                    ("type", kind.as_str().to_owned()),
                    ("size", size.min(500).to_string()),
                    ("offset", offset.to_string()),
                ],
            )
            .await?;

        match payload.remove("albumList2") {
            Some(value) => {
                let list: AlbumListPayload = serde_json::from_value(value)
                    .map_err(|e| Error::Decode(format!("albumList2: {e}")))?;
                Ok(list.album)
            }
            // An empty list is omitted by some servers.
            None => Ok(Vec::new()),
        }
    }

    /// `search3`: artists, albums and songs in one round trip.
    pub async fn search(&self, query: &str) -> Result<SearchResult, Error> {
        let mut payload = self.get("search3", &[("query", query.to_owned())]).await?;

        match payload.remove("searchResult3") {
            Some(value) => serde_json::from_value(value)
                .map_err(|e| Error::Decode(format!("searchResult3: {e}"))),
            None => Ok(SearchResult::default()),
        }
    }

    /// `getPlaylists`
    pub async fn playlists(&self) -> Result<Vec<Playlist>, Error> {
        let mut payload = self.get("getPlaylists", &[]).await?;
        match payload.remove("playlists") {
            Some(value) => {
                let list: PlaylistsPayload = serde_json::from_value(value)
                    .map_err(|e| Error::Decode(format!("playlists: {e}")))?;
                Ok(list.playlist)
            }
            None => Ok(Vec::new()),
        }
    }

    /// `getPlaylist`
    pub async fn playlist(&self, id: &str) -> Result<PlaylistWithSongs, Error> {
        let mut payload = self.get("getPlaylist", &[("id", id.to_owned())]).await?;
        take(&mut payload, "playlist")
    }

    pub async fn create_playlist(
        &self,
        name: &str,
        songs: &[String],
    ) -> Result<Option<PlaylistWithSongs>, Error> {
        let mut params = vec![("name", name.to_owned())];
        params.extend(songs.iter().cloned().map(|id| ("songId", id)));
        let mut payload = self.post("createPlaylist", &params).await?;
        if payload.contains_key("playlist") {
            take(&mut payload, "playlist").map(Some)
        } else {
            Ok(None)
        }
    }

    pub async fn update_playlist(
        &self,
        id: &str,
        name: Option<&str>,
        add: &[String],
        remove: &[u32],
    ) -> Result<(), Error> {
        let mut params = vec![("playlistId", id.to_owned())];
        if let Some(name) = name {
            params.push(("name", name.to_owned()));
        }
        params.extend(add.iter().cloned().map(|id| ("songIdToAdd", id)));
        params.extend(
            remove
                .iter()
                .map(|index| ("songIndexToRemove", index.to_string())),
        );
        self.post("updatePlaylist", &params).await?;
        Ok(())
    }

    pub async fn delete_playlist(&self, id: &str) -> Result<(), Error> {
        self.post("deletePlaylist", &[("id", id.to_owned())])
            .await?;
        Ok(())
    }

    /// Builds a `stream` URL for one song.
    ///
    /// The format is always explicit: leaving it out lets the server pick its
    /// default, which for Navidrome is Opus and is not decodable by symphonia.
    pub fn stream_url(&self, song: &Song, quality: StreamQuality) -> String {
        let local = song.suffix.as_deref().map(can_decode).unwrap_or(false);
        let mut params = vec![("id", song.id.clone())];

        match quality {
            StreamQuality::Original if local => {
                params.push(("format", "raw".to_owned()));
            }
            StreamQuality::Original => {
                // Local decoder cannot handle this file: let the server convert
                // it, but ask for MP3 explicitly.
                params.push(("format", "mp3".to_owned()));
                params.push(("maxBitRate", DEFAULT_TRANSCODE_BITRATE.to_string()));
                params.push(("estimateContentLength", "true".to_owned()));
            }
            StreamQuality::Transcode { max_bit_rate } => {
                params.push(("format", "mp3".to_owned()));
                params.push(("maxBitRate", max_bit_rate.to_string()));
                params.push(("estimateContentLength", "true".to_owned()));
            }
        }

        self.endpoint("stream", &params).to_string()
    }

    /// Builds a `getCoverArt` URL.
    pub fn cover_art_url(&self, id: &str, size: Option<u32>) -> String {
        let mut params = vec![("id", id.to_owned())];
        if let Some(size) = size {
            params.push(("size", size.to_string()));
        }
        self.endpoint("getCoverArt", &params).to_string()
    }

    /// `getCoverArt`: downloads the image bytes.
    pub async fn cover_art(&self, id: &str, size: Option<u32>) -> Result<Vec<u8>, Error> {
        let url = reqwest::Url::parse(&self.cover_art_url(id, size))
            .map_err(|error| Error::InvalidUrl(error.to_string()))?;
        let response = self.send(url).await?.error_for_status()?;
        let bytes = response.bytes().await?;
        Ok(bytes.to_vec())
    }

    /// `scrobble`: `submission = false` marks "now playing", `true` counts a play.
    pub async fn scrobble(&self, id: &str, submission: bool) -> Result<(), Error> {
        self.get(
            "scrobble",
            &[
                ("id", id.to_owned()),
                ("submission", submission.to_string()),
            ],
        )
        .await?;
        Ok(())
    }

    /// `star` / `unstar`
    pub async fn set_starred(&self, id: &str, starred: bool) -> Result<(), Error> {
        let endpoint = if starred { "star" } else { "unstar" };
        self.get(endpoint, &[("id", id.to_owned())]).await?;
        Ok(())
    }

    /// Albums use `albumId`; `id` is reserved for songs in Subsonic.
    pub async fn set_album_starred(&self, id: &str, starred: bool) -> Result<(), Error> {
        let endpoint = if starred { "star" } else { "unstar" };
        self.get(endpoint, &[("albumId", id.to_owned())]).await?;
        Ok(())
    }
}

fn take<T: DeserializeOwned>(payload: &mut Map<String, Value>, key: &str) -> Result<T, Error> {
    let value = payload
        .remove(key)
        .ok_or_else(|| Error::Decode(format!("missing '{key}'")))?;
    serde_json::from_value(value).map_err(|e| Error::Decode(format!("{key}: {e}")))
}

#[derive(Debug, Deserialize)]
struct Envelope {
    #[serde(rename = "subsonic-response")]
    response: ResponseBody,
}

#[derive(Debug, Deserialize)]
struct ResponseBody {
    status: String,
    #[serde(default)]
    error: Option<ApiError>,
    #[serde(flatten)]
    payload: Map<String, Value>,
}

#[derive(Debug, Deserialize)]
struct ApiError {
    code: i32,
    #[serde(default)]
    message: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn client(server: &str) -> Client {
        Client::new(server, Credentials::new("alice", "secret")).unwrap()
    }

    /// Serves exactly one HTTP request with a fixed JSON body.
    async fn serve_once(body: &'static str) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = vec![0u8; 8192];
            let _ = socket.read(&mut buffer).await;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = socket.write_all(response.as_bytes()).await;
        });
        format!("http://{address}")
    }

    #[test]
    fn endpoint_carries_auth_but_not_the_password() {
        let url = client("http://example.test:4533").endpoint("ping", &[]);
        let params: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();

        assert_eq!(params.get("u").map(String::as_str), Some("alice"));
        assert_eq!(params.get("c").map(String::as_str), Some(CLIENT_ID));
        assert_eq!(params.get("v").map(String::as_str), Some(API_VERSION));
        assert_eq!(params.get("f").map(String::as_str), Some("json"));
        assert!(params.contains_key("t"));
        assert!(params.contains_key("s"));
        assert!(!params.contains_key("p"));
        assert!(!url.as_str().contains("secret"));
    }

    #[test]
    fn base_url_tolerates_rest_suffix_and_missing_slash() {
        let url = client("http://example.test:4533/rest").endpoint("ping", &[]);
        assert_eq!(url.path(), "/rest/ping");

        let url = client("http://example.test:4533/music").endpoint("ping", &[]);
        assert_eq!(url.path(), "/music/rest/ping");
    }

    #[test]
    fn decode_capability_matrix() {
        for suffix in ["mp3", "FLAC", "ogg", "m4a", "alac", "wav", "aiff"] {
            assert!(can_decode(suffix), "{suffix} should decode");
        }
        for suffix in ["opus", "wma", "ape", "dsf"] {
            assert!(!can_decode(suffix), "{suffix} should not decode");
        }
    }

    #[test]
    fn stream_url_is_explicit_about_format() {
        let client = client("http://example.test:4533");

        let mut song = Song {
            id: "42".into(),
            title: "track".into(),
            suffix: Some("flac".into()),
            ..Song::default()
        };
        let raw = client.stream_url(&song, StreamQuality::Original);
        assert!(raw.contains("format=raw"));
        assert!(!raw.contains("maxBitRate"));

        song.suffix = Some("wma".into());
        let transcoded = client.stream_url(&song, StreamQuality::Original);
        assert!(transcoded.contains("format=mp3"));
        assert!(transcoded.contains("maxBitRate="));

        let forced = client.stream_url(&song, StreamQuality::Transcode { max_bit_rate: 128 });
        assert!(forced.contains("format=mp3"));
        assert!(forced.contains("maxBitRate=128"));
    }

    #[tokio::test]
    async fn ping_parses_server_info() {
        let server = serve_once(
            r#"{"subsonic-response":{"status":"ok","version":"1.16.1","type":"navidrome","serverVersion":"0.54.0","openSubsonic":true}}"#,
        )
        .await;

        let info = client(&server).ping().await.unwrap();
        assert_eq!(info.server_type, "navidrome");
        assert_eq!(info.server_version.as_deref(), Some("0.54.0"));
        assert_eq!(info.open_subsonic, Some(true));
    }

    #[tokio::test]
    async fn error_code_maps_to_auth() {
        let server = serve_once(
            r#"{"subsonic-response":{"status":"failed","version":"1.16.1","error":{"code":40,"message":"Wrong username or password"}}}"#,
        )
        .await;

        let error = client(&server).ping().await.unwrap_err();
        assert!(matches!(error, Error::Auth));
    }

    #[tokio::test]
    async fn album_list_flattens_and_clamps() {
        let server = serve_once(
            r#"{"subsonic-response":{"status":"ok","version":"1.16.1","albumList2":{"album":[{"id":"a1","name":"One"},{"id":"a2","name":"Two"}]}}}"#,
        )
        .await;

        let albums = client(&server)
            .album_list(AlbumListType::Newest, 9999, 0)
            .await
            .unwrap();
        assert_eq!(albums.len(), 2);
        assert_eq!(albums[1].name, "Two");
    }

    #[tokio::test]
    async fn album_flattens_its_song_list() {
        let server = serve_once(
            r#"{"subsonic-response":{"status":"ok","version":"1.16.1","album":{"id":"a1","name":"Album","artist":"Artist","song":[{"id":"s1","title":"Track","suffix":"flac"}]}}}"#,
        )
        .await;

        let album = client(&server).album("a1").await.unwrap();
        assert_eq!(album.album.name, "Album");
        assert_eq!(album.album.artist.as_deref(), Some("Artist"));
        assert_eq!(album.song.len(), 1);
        assert_eq!(album.song[0].suffix.as_deref(), Some("flac"));
    }

    #[tokio::test]
    async fn search_parses_all_three_sections() {
        let server = serve_once(
            r#"{"subsonic-response":{"status":"ok","version":"1.16.1","searchResult3":{"artist":[{"id":"ar1","name":"Artist"}],"album":[{"id":"al1","name":"Album"}],"song":[{"id":"s1","title":"Song"}]}}}"#,
        )
        .await;

        let result = client(&server).search("q").await.unwrap();
        assert_eq!(result.artist.len(), 1);
        assert_eq!(result.album.len(), 1);
        assert_eq!(result.song.len(), 1);
    }

    #[tokio::test]
    async fn album_star_uses_album_id_and_surfaces_server_failure() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = [0; 4096];
            let len = socket.read(&mut buffer).await.unwrap();
            let request = String::from_utf8_lossy(&buffer[..len]);
            assert!(request.starts_with("GET /rest/star?"));
            assert!(request.contains("albumId=album-42"));
            assert!(!request.contains("&id="));
            let body = r#"{"subsonic-response":{"status":"failed","error":{"code":50,"message":"not allowed"}}}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        });
        assert!(
            client(&format!("http://{address}"))
                .set_album_starred("album-42", true)
                .await
                .is_err()
        );
        server.await.unwrap();
    }

    async fn capture_post(body: &'static str) -> (String, tokio::task::JoinHandle<String>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0; 4096];
            loop {
                let len = socket.read(&mut buffer).await.unwrap();
                assert!(len > 0);
                bytes.extend_from_slice(&buffer[..len]);
                if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                    let body_len: usize = header
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length: "))
                        .unwrap()
                        .parse()
                        .unwrap();
                    if bytes.len() >= end + 4 + body_len {
                        break;
                    }
                }
                assert!(bytes.len() < 1_000_000);
            }
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            String::from_utf8(bytes).unwrap()
        });
        (format!("http://{address}"), server)
    }

    #[tokio::test]
    async fn playlist_update_posts_duplicates_and_zero_based_removals() {
        let (url, server) = capture_post(r#"{"subsonic-response":{"status":"ok"}}"#).await;
        client(&url)
            .update_playlist(
                "p1",
                Some("new name"),
                &["same".into(), "same".into()],
                &[0, 3],
            )
            .await
            .unwrap();
        let request = server.await.unwrap();
        assert!(request.starts_with("POST /rest/updatePlaylist?"));
        assert!(request.contains("content-type: application/x-www-form-urlencoded"));
        assert!(request.ends_with("playlistId=p1&name=new+name&songIdToAdd=same&songIdToAdd=same&songIndexToRemove=0&songIndexToRemove=3"));
    }

    #[tokio::test]
    async fn playlist_create_preserves_entries_and_legacy_empty_success() {
        let (url, server) = capture_post(r#"{"subsonic-response":{"status":"ok","playlist":{"id":"p1","name":"created","entry":[{"id":"s1","title":"One"},{"id":"s1","title":"One"}]}}}"#).await;
        let playlist = client(&url)
            .create_playlist("created", &["s1".into(), "s1".into()])
            .await
            .unwrap()
            .unwrap();
        assert_eq!(playlist.entry.len(), 2);
        assert!(
            server
                .await
                .unwrap()
                .ends_with("name=created&songId=s1&songId=s1")
        );
        let (url, server) = capture_post(r#"{"subsonic-response":{"status":"ok"}}"#).await;
        assert!(
            client(&url)
                .create_playlist("empty", &[])
                .await
                .unwrap()
                .is_none()
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn playlist_delete_uses_id_and_surfaces_permission_failure() {
        let (url, server) =
            capture_post(r#"{"subsonic-response":{"status":"failed","error":{"code":50}}}"#).await;
        assert!(matches!(
            client(&url).delete_playlist("p1").await,
            Err(Error::NotAuthorized)
        ));
        let request = server.await.unwrap();
        assert!(request.starts_with("POST /rest/deletePlaylist?"));
        assert!(request.ends_with("id=p1"));
    }
}
