//! Background API worker.
//!
//! `melos-api` is async and `reqwest` needs a tokio reactor, while GPUI owns the
//! main thread with its own executor. The worker gives tokio its own thread and
//! communicates with the UI through plain channels.

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use melos_api::{
    Album, AlbumListType, AlbumWithSongs, Artist, ArtistWithAlbums, Client, Credentials,
    SearchResult, ServerInfo,
};

pub enum Request {
    Connect {
        server: String,
        username: String,
        password: String,
    },
    Albums {
        size: u32,
        offset: u32,
    },
    Artists,
    Album {
        id: String,
    },
    Artist {
        id: String,
    },
    Search {
        query: String,
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
    Connected(Box<ServerInfo>),
    Albums(Vec<Album>),
    Artists(Vec<Artist>),
    Album(Box<AlbumWithSongs>),
    Artist(Box<ArtistWithAlbums>),
    Search(Box<SearchResult>),
    Cover { id: String, bytes: Vec<u8> },
    CoverFailed { id: String },
    Scrobbled,
    Starred { id: String, error: Option<String> },
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

    loop {
        let request = match priority.try_recv() {
            Ok(request) => request,
            Err(_) => match requests.recv_timeout(std::time::Duration::from_millis(25)) {
                Ok(request) => request,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            },
        };
        let response = runtime.block_on(handle(&mut client, request));
        if responses.send(response).is_err() {
            break;
        }
    }
}

async fn handle(client: &mut Option<Client>, request: Request) -> Response {
    match request {
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
                Ok(albums) => Response::Albums(albums),
                Err(error) => Response::Error(error.to_string()),
            }
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
