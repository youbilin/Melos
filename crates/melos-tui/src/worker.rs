//! Network I/O runs on a Tokio thread so the terminal remains responsive.
use melos_api::{Album, AlbumListType, Client, Playlist, Song};
use melos_core::lyrics::{self, Lyrics};
use std::sync::mpsc::{self, Receiver, Sender};

pub enum Job {
    Connect,
    Albums(u32),
    Album(String),
    Playlists,
    Playlist(String),
    Search(String),
    Lyrics(Box<Song>),
    NowPlaying(String),
    Scrobble(String),
}
pub enum Data {
    Connected,
    Albums {
        albums: Vec<Album>,
        offset: u32,
        received: usize,
    },
    Songs {
        songs: Vec<Song>,
        title: String,
    },
    Playlists(Vec<Playlist>),
    Lyrics(Vec<Lyrics>),
}
pub struct Response {
    pub ticket: u64,
    pub lyrics: bool,
    pub data: Result<Data, String>,
}
pub struct Worker {
    requests: Sender<(u64, Job)>,
    responses: Receiver<Response>,
    next_ticket: u64,
}
impl Worker {
    pub fn new(client: Client) -> Self {
        let (requests, jobs) = mpsc::channel::<(u64, Job)>();
        let (responses, results) = mpsc::channel();
        std::thread::spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    let _ = responses.send(Response {
                        ticket: 1,
                        lyrics: false,
                        data: Err(error.to_string()),
                    });
                    return;
                }
            };
            while let Ok((ticket, job)) = jobs.recv() {
                let lyric_job = matches!(job, Job::Lyrics(_));
                let data = runtime.block_on(async {
                    let data = match job {
                        Job::Connect => {
                            client.ping().await?;
                            Data::Connected
                        }
                        Job::Albums(offset) => {
                            let albums =
                                client.album_list(AlbumListType::Newest, 80, offset).await?;
                            let received = albums.len();
                            Data::Albums {
                                albums,
                                offset,
                                received,
                            }
                        }
                        Job::Album(id) => {
                            let album = client.album(&id).await?;
                            Data::Songs {
                                songs: album.song,
                                title: album.album.name,
                            }
                        }
                        Job::Playlist(id) => {
                            let playlist = client.playlist(&id).await?;
                            Data::Songs {
                                songs: playlist.entry,
                                title: playlist.playlist.name,
                            }
                        }
                        Job::Playlists => Data::Playlists(client.playlists().await?),
                        Job::Search(query) => {
                            let result = client.search(&query).await?;
                            Data::Songs {
                                songs: result.song,
                                title: format!("Search: {query}"),
                            }
                        }
                        Job::Lyrics(song) => Data::Lyrics(lyrics::fetch(&client, &song).await?),
                        Job::NowPlaying(id) => {
                            let _ = client.scrobble(&id, false).await;
                            return Ok::<_, melos_api::Error>(None);
                        }
                        Job::Scrobble(id) => {
                            let _ = client.scrobble(&id, true).await;
                            return Ok(None);
                        }
                    };
                    Ok(Some(data))
                });
                match data {
                    Ok(None) => {}
                    result => {
                        let data = result
                            .map(|data| data.expect("response data"))
                            .map_err(|error| error.to_string());
                        if responses
                            .send(Response {
                                ticket,
                                lyrics: lyric_job,
                                data,
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                }
            }
        });
        Self {
            requests,
            responses: results,
            next_ticket: 0,
        }
    }
    pub fn send(&mut self, job: Job) -> u64 {
        self.next_ticket = self.next_ticket.wrapping_add(1);
        let _ = self.requests.send((self.next_ticket, job));
        self.next_ticket
    }
    pub fn drain(&self) -> Vec<Response> {
        self.responses.try_iter().collect()
    }
}
