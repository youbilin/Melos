//! Background API worker.
//!
//! `melos-api` is async and `reqwest` needs a tokio reactor, while GPUI owns the
//! main thread with its own executor. The worker gives tokio its own thread and
//! communicates with the UI through plain channels.

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use melos_api::{
    Album, AlbumListType, AlbumWithSongs, Artist, ArtistWithAlbums, Client, Credentials, Playlist,
    PlaylistWithSongs, SearchResult, ServerInfo,
};
use melos_core::cover_cache::CoverCache;
use melos_core::lyrics::{self, Lyrics};

pub enum Request {
    Lyrics {
        request_id: u64,
        song: Box<melos_api::Song>,
    },
    Connect {
        server: String,
        username: String,
        password: String,
    },
    Albums {
        size: u32,
        offset: u32,
    },
    LibrarySongs {
        size: u32,
        offset: u32,
    },
    ScopeSongs {
        albums: Vec<String>,
        artists: Vec<String>,
    },
    Artists,
    Playlists,
    Playlist {
        id: String,
    },
    CreatePlaylist {
        name: String,
        songs: Vec<String>,
    },
    UpdatePlaylist {
        id: String,
        name: Option<String>,
        add: Vec<String>,
        remove: Vec<u32>,
    },
    DeletePlaylist {
        id: String,
    },
    Album {
        id: String,
    },
    Artist {
        id: String,
    },
    Search {
        query: String,
    },
    RandomSongs {
        request_id: u64,
        size: u32,
    },
    Cover {
        id: String,
        size: u32,
    },
    Scrobble {
        id: String,
        submission: bool,
    },
    Star {
        id: String,
        starred: bool,
    },
}

pub enum Response {
    Lyrics {
        request_id: u64,
        lyrics: Vec<Lyrics>,
        error: Option<String>,
    },
    Connected(Box<ServerInfo>),
    Albums {
        albums: Vec<Album>,
        offset: u32,
        size: u32,
    },
    LibrarySongs {
        songs: Vec<melos_api::Song>,
        offset: u32,
        size: u32,
    },
    ScopeSongs(Vec<melos_api::Song>),
    Artists(Vec<Artist>),
    Playlists(Vec<Playlist>),
    Playlist(Box<PlaylistWithSongs>),
    PlaylistCreated {
        playlist: Option<Box<PlaylistWithSongs>>,
        error: Option<String>,
    },
    PlaylistChanged {
        id: String,
        deleted: bool,
        error: Option<String>,
    },
    Album(Box<AlbumWithSongs>),
    Artist(Box<ArtistWithAlbums>),
    Search(Box<SearchResult>),
    RandomSongs {
        request_id: u64,
        songs: Vec<melos_api::Song>,
        error: Option<String>,
    },
    Cover {
        id: String,
        bytes: Vec<u8>,
    },
    CoverFailed {
        id: String,
    },
    Scrobbled,
    Starred {
        id: String,
        error: Option<String>,
    },
    Error(String),
}

pub struct ApiWorker {
    requests: Sender<Request>,
    priority: Sender<Request>,
    responses: Receiver<Response>,
}

impl ApiWorker {
    pub fn new() -> Self {
        let (requests, request_rx) = mpsc::channel();
        let (priority, priority_rx) = mpsc::channel();
        let (response_tx, responses) = mpsc::channel();

        thread::spawn(move || run(request_rx, priority_rx, response_tx));

        Self {
            requests,
            priority,
            responses,
        }
    }

    pub fn send(&self, request: Request) {
        let _ = self.requests.send(request);
    }

    pub fn send_priority(&self, request: Request) {
        let _ = self.priority.send(request);
    }

    /// Non-blocking drain used by the UI poller.
    pub fn drain(&self) -> Vec<Response> {
        let mut out = Vec::new();
        while let Ok(response) = self.responses.try_recv() {
            out.push(response);
        }
        out
    }
}

fn run(requests: Receiver<Request>, priority: Receiver<Request>, responses: Sender<Response>) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            let _ = responses.send(Response::Error(format!("failed to start runtime: {error}")));
            return;
        }
    };

    let mut client: Option<Client> = None;
    let mut cache: Option<CoverCache> = None;

    loop {
        let request = match priority.try_recv() {
            Ok(request) => request,
            Err(_) => match requests.recv_timeout(std::time::Duration::from_millis(25)) {
                Ok(request) => request,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            },
        };
        let scope = match &request {
            Request::Connect {
                server, username, ..
            } => CoverCache::for_server(server, username),
            _ => None,
        };
        let cover_key = match &request {
            Request::Cover { id, size } => Some((id.clone(), *size)),
            _ => None,
        };
        let cached = cover_key.as_ref().and_then(|(id, size)| {
            cache
                .as_ref()?
                .read(id, *size)
                .map(|bytes| Response::Cover {
                    id: id.clone(),
                    bytes,
                })
        });
        let from_cache = cached.is_some();
        let response = cached.unwrap_or_else(|| runtime.block_on(handle(&mut client, request)));
        if matches!(response, Response::Connected(_)) {
            cache = scope;
        }
        if !from_cache
            && let (Some(cache), Some((id, size)), Response::Cover { bytes, .. }) =
                (&cache, &cover_key, &response)
        {
            cache.write(id, *size, bytes);
        }
        if responses.send(response).is_err() {
            break;
        }
    }
}

async fn handle(client: &mut Option<Client>, request: Request) -> Response {
    match request {
        Request::Lyrics { request_id, song } => {
            let result = match client.as_ref() {
                Some(client) => lyrics::fetch(client, &song).await,
                None => {
                    return Response::Lyrics {
                        request_id,
                        lyrics: Vec::new(),
                        error: Some("尚未连接服务器".into()),
                    };
                }
            };
            match result {
                Ok(lyrics) => Response::Lyrics {
                    request_id,
                    lyrics,
                    error: None,
                },
                Err(error) => Response::Lyrics {
                    request_id,
                    lyrics: Vec::new(),
                    error: Some(error.to_string()),
                },
            }
        }
        Request::Playlists => {
            let Some(client) = client.as_ref() else {
                return Response::Error("尚未连接服务器".into());
            };
            match client.playlists().await {
                Ok(list) => Response::Playlists(list),
                Err(error) => Response::Error(error.to_string()),
            }
        }
        Request::Playlist { id } => {
            let Some(client) = client.as_ref() else {
                return Response::Error("尚未连接服务器".into());
            };
            match client.playlist(&id).await {
                Ok(playlist) => Response::Playlist(Box::new(playlist)),
                Err(error) => Response::Error(error.to_string()),
            }
        }
        Request::CreatePlaylist { name, songs } => {
            let Some(client) = client.as_ref() else {
                return Response::PlaylistCreated {
                    playlist: None,
                    error: Some("尚未连接服务器".into()),
                };
            };
            match client.create_playlist(&name, &songs).await {
                Ok(playlist) => Response::PlaylistCreated {
                    playlist: playlist.map(Box::new),
                    error: None,
                },
                Err(error) => Response::PlaylistCreated {
                    playlist: None,
                    error: Some(error.to_string()),
                },
            }
        }
        Request::UpdatePlaylist {
            id,
            name,
            add,
            remove,
        } => {
            let error = match client.as_ref() {
                Some(client) => client
                    .update_playlist(&id, name.as_deref(), &add, &remove)
                    .await
                    .err()
                    .map(|error| error.to_string()),
                None => Some("尚未连接服务器".into()),
            };
            Response::PlaylistChanged {
                id,
                deleted: false,
                error,
            }
        }
        Request::DeletePlaylist { id } => {
            let error = match client.as_ref() {
                Some(client) => client
                    .delete_playlist(&id)
                    .await
                    .err()
                    .map(|error| error.to_string()),
                None => Some("尚未连接服务器".into()),
            };
            Response::PlaylistChanged {
                id,
                deleted: true,
                error,
            }
        }
        Request::Connect {
            server,
            username,
            password,
        } => match Client::new(&server, Credentials::new(username, password)) {
            Ok(candidate) => match candidate.ping().await {
                Ok(info) => {
                    *client = Some(candidate);
                    Response::Connected(Box::new(info))
                }
                Err(error) => Response::Error(error.to_string()),
            },
            Err(error) => Response::Error(error.to_string()),
        },

        Request::Albums { size, offset } => {
            let Some(client) = client.as_ref() else {
                return Response::Error("尚未连接服务器".into());
            };
            match client.album_list(AlbumListType::Newest, size, offset).await {
                Ok(albums) => Response::Albums {
                    albums,
                    offset,
                    size,
                },
                Err(error) => Response::Error(error.to_string()),
            }
        }

        Request::LibrarySongs { size, offset } => {
            let Some(client) = client.as_ref() else {
                return Response::Error("尚未连接服务器".into());
            };
            match client.songs_page(size, offset).await {
                Ok(songs) => Response::LibrarySongs {
                    songs,
                    offset,
                    size,
                },
                Err(error) => Response::Error(error.to_string()),
            }
        }

        Request::ScopeSongs { albums, artists } => {
            let Some(client) = client.as_ref() else {
                return Response::Error("尚未连接服务器".into());
            };
            let mut songs = Vec::new();
            for id in albums {
                match client.album(&id).await {
                    Ok(album) => songs.extend(album.song),
                    Err(error) => return Response::Error(error.to_string()),
                }
            }
            for id in artists {
                let artist = match client.artist(&id).await {
                    Ok(artist) => artist,
                    Err(error) => return Response::Error(error.to_string()),
                };
                for album in artist.album.iter().take(40) {
                    match client.album(&album.id).await {
                        Ok(detail) => songs.extend(detail.song),
                        Err(error) => return Response::Error(error.to_string()),
                    }
                }
            }
            if songs.is_empty() {
                return Response::Error("所选范围没有歌曲".into());
            }
            songs.truncate(2000);
            Response::ScopeSongs(songs)
        }

        Request::Artists => {
            let Some(client) = client.as_ref() else {
                return Response::Error("尚未连接服务器".into());
            };
            match client.artists().await {
                Ok(artists) => Response::Artists(artists),
                Err(error) => Response::Error(error.to_string()),
            }
        }

        Request::Album { id } => {
            let Some(client) = client.as_ref() else {
                return Response::Error("尚未连接服务器".into());
            };
            match client.album(&id).await {
                Ok(album) => Response::Album(Box::new(album)),
                Err(error) => Response::Error(error.to_string()),
            }
        }

        Request::Artist { id } => {
            let Some(client) = client.as_ref() else {
                return Response::Error("尚未连接服务器".into());
            };
            match client.artist(&id).await {
                Ok(artist) => Response::Artist(Box::new(artist)),
                Err(error) => Response::Error(error.to_string()),
            }
        }

        Request::Search { query } => {
            let Some(client) = client.as_ref() else {
                return Response::Error("尚未连接服务器".into());
            };
            match client.search(&query).await {
                Ok(result) => Response::Search(Box::new(result)),
                Err(error) => Response::Error(error.to_string()),
            }
        }

        Request::RandomSongs { request_id, size } => {
            let Some(client) = client.as_ref() else {
                return Response::RandomSongs {
                    request_id,
                    songs: Vec::new(),
                    error: Some("尚未连接服务器".into()),
                };
            };
            match client.random_songs(size).await {
                Ok(songs) => Response::RandomSongs {
                    request_id,
                    songs,
                    error: None,
                },
                Err(error) => Response::RandomSongs {
                    request_id,
                    songs: Vec::new(),
                    error: Some(error.to_string()),
                },
            }
        }

        Request::Cover { id, size } => {
            let Some(client) = client.as_ref() else {
                return Response::Error("尚未连接服务器".into());
            };
            match client.cover_art(&id, Some(size)).await {
                Ok(bytes) => Response::Cover { id, bytes },
                Err(_) => Response::CoverFailed { id },
            }
        }

        Request::Scrobble { id, submission } => {
            let Some(client) = client.as_ref() else {
                return Response::Error("尚未连接服务器".into());
            };
            match client.scrobble(&id, submission).await {
                Ok(()) => Response::Scrobbled,
                Err(_) => Response::Scrobbled,
            }
        }

        Request::Star { id, starred } => {
            let Some(client) = client.as_ref() else {
                return Response::Starred {
                    id,
                    error: Some("尚未连接服务器".into()),
                };
            };
            let error = client
                .set_album_starred(&id, starred)
                .await
                .err()
                .map(|error| error.to_string());
            Response::Starred { id, error }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_lyrics_retains_request_generation() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let response = runtime.block_on(handle(
            &mut None,
            Request::Lyrics {
                request_id: 42,
                song: Box::default(),
            },
        ));
        assert!(matches!(
            response,
            Response::Lyrics {
                request_id: 42,
                error: Some(_),
                ..
            }
        ));
    }

    #[test]
    fn failed_star_retains_request_identity_for_rollback() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let response = runtime.block_on(handle(
            &mut None,
            Request::Star {
                id: "album-42".into(),
                starred: true,
            },
        ));
        assert!(matches!(response, Response::Starred { id, error: Some(_) } if id == "album-42"));
    }
}
