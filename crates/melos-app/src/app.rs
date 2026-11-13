//! Desktop views and playback orchestration.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use gpui::prelude::*;
use gpui::*;
use melos_api::{
    Album, AlbumWithSongs, Artist, ArtistWithAlbums, Client, Credentials, Playlist,
    PlaylistWithSongs, SearchResult, ServerInfo, Song, StreamQuality,
};
use melos_core::artwork::playback_cover_candidates;
use melos_core::lyrics::Lyrics;
use melos_core::{Queue, RepeatMode, Settings};
use melos_player::{Engine, StreamSpec};

use crate::api::{ApiWorker, Request, Response};
use crate::smtc;
use crate::theme;
use crate::ui::{self, input::TextInput};

const SIDEBAR_WIDTH: f32 = 216.0;
const SIDEBAR_COLLAPSED_WIDTH: f32 = 64.0;
const QUEUE_WIDTH: f32 = 340.0;
const CONTENT_PADDING: f32 = 64.0;
const CARD_MIN: f32 = 160.0;
const CARD_GAP: f32 = 24.0;
const ALBUM_PAGE_SIZE: u32 = 80;
const RANDOM_BATCH: u32 = 100;
const SONG_ROW_HEIGHT: f32 = 76.0;
const QUEUE_ROW_HEIGHT: f32 = 76.0;

#[derive(Clone, Copy)]
enum SongSource {
    Album,
    Playlist,
    Search,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SliderKind {
    Progress,
    Volume,
}

#[derive(Clone, Copy)]
enum PlaylistEditor {
    Create,
    Rename,
}

#[derive(Clone, Copy)]
struct SliderDrag {
    kind: SliderKind,
    bounds: Bounds<Pixels>,
    fraction: f32,
}

#[derive(Clone)]
struct QueueDrag {
    index: usize,
    title: String,
    position: Point<Pixels>,
}

impl Render for QueueDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().pl(self.position.x).pt(self.position.y).child(
            div()
                .w(px(240.))
                .p_3()
                .rounded_md()
                .bg(rgb(theme::HOVER))
                .text_color(rgb(theme::TEXT))
                .text_sm()
                .shadow_md()
                .child(self.title.clone()),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Home,
    Artists,
    Albums,
    Playlists,
    Search,
    Lyrics,
    Settings,
}

impl View {
    fn label(self) -> &'static str {
        match self {
            Self::Home => "首页",
            Self::Artists => "艺人",
            Self::Albums => "专辑",
            Self::Playlists => "歌单",
            Self::Search => "搜索",
            Self::Lyrics => "歌词",
            Self::Settings => "设置",
        }
    }

    fn icon(self) -> ui::Icon {
        match self {
            Self::Home => ui::Icon::Home,
            Self::Artists => ui::Icon::Artist,
            Self::Albums => ui::Icon::Album,
            Self::Playlists => ui::Icon::Playlist,
            Self::Search => ui::Icon::Search,
            Self::Lyrics => ui::Icon::Lyrics,
            Self::Settings => ui::Icon::Settings,
        }
    }
}

pub struct Root {
    focus_handle: FocusHandle,
    api: ApiWorker,
    view: View,
    sidebar_collapsed: bool,
    playing: bool,
    queue_open: bool,
    queue_scroll: UniformListScrollHandle,
    search_results_generation: u64,
    volume: f32,
    slider_drag: Option<SliderDrag>,
    pending_stars: HashMap<String, Option<String>>,

    server: Entity<TextInput>,
    username: Entity<TextInput>,
    password: Entity<TextInput>,
    search: Entity<TextInput>,
    playlist_name: Entity<TextInput>,

    connected: bool,
    connecting: bool,
    error: Option<String>,
    server_info: Option<ServerInfo>,

    albums: Vec<Album>,
    album_offset: u32,
    albums_loading: bool,
    albums_more: bool,
    artists: Vec<Artist>,
    detail: Option<AlbumWithSongs>,
    artist_view: Option<ArtistWithAlbums>,
    search_result: Option<SearchResult>,
    playlists: Vec<Playlist>,
    playlist_detail: Option<PlaylistWithSongs>,
    requested_playlist: Option<String>,
    playlists_loading: bool,
    playlist_pending: bool,
    playlist_editor: Option<PlaylistEditor>,
    playlist_delete_confirm: bool,
    covers: HashMap<String, Arc<Image>>,
    visible_cover_ids: HashSet<String>,
    failed_covers: HashMap<String, Instant>,
    cover_requests: HashSet<String>,
    priority_cover_requests: HashSet<String>,
    playing_cover_ids: Vec<String>,
    settings: Entity<Settings>,
    smtc: Option<smtc::Smtc>,
    #[cfg(windows)]
    smtc_artwork: Option<(String, u64)>,
    #[cfg(windows)]
    smtc_artwork_request: u64,
    smoke_reported: bool,

    client: Option<Client>,
    engine: Option<Engine>,
    queue: Queue,
    now_playing: Option<Song>,
    position_ms: u64,
    duration_ms: u64,
    last_track_index: u64,
    lyrics: Vec<Lyrics>,
    lyrics_track: usize,
    lyrics_request: u64,
    lyrics_loading: bool,
    lyrics_error: Option<String>,
    lyrics_scroll: ScrollHandle,
    lyrics_active: Option<usize>,
    lyrics_follow: bool,
    lyrics_scrolled_to: Option<usize>,
    lyrics_center_height: f32,
    lyrics_scroll_generation: u64,
    lyrics_center_retry: bool,
    lyrics_return: View,
    random_request: u64,
    random_pending: bool,
    random_continue: bool,
    random_retry: Option<Instant>,
    scrobbled_playing: bool,
    scrobbled_submitted: bool,
}

impl Root {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let settings = Settings::load();
        let volume = if settings.volume.is_finite() {
            settings.volume.clamp(0.0, 1.0)
        } else {
            0.8
        };
        let server = cx.new(|cx| {
            let mut input = TextInput::new("http://192.168.1.10:4533", cx);
            input.set_content(settings.server.clone(), cx);
            input
        });
        let username = cx.new(|cx| {
            let mut input = TextInput::new("用户名", cx);
            input.set_content(settings.username.clone(), cx);
            input
        });
        let password = cx.new(|cx| {
            let mut input = TextInput::with_secure("密码", true, cx);
            if let Some(saved) = settings.password() {
                input.set_content(saved, cx);
            }
            input
        });
        let search = cx.new(|cx| TextInput::new("搜索艺人、专辑或歌曲", cx));
        let playlist_name = cx.new(|cx| TextInput::new("歌单名称", cx));
        let mut queue = Queue::new();
        queue.set_repeat(settings.repeat);
        queue.set_shuffle(settings.shuffle);
        let saved_settings = cx.new(|_| settings);

        let root = Self {
            focus_handle: cx.focus_handle(),
            api: ApiWorker::new(),
            view: View::Home,
            sidebar_collapsed: false,
            playing: false,
            queue_open: false,
            queue_scroll: UniformListScrollHandle::new(),
            search_results_generation: 0,
            volume,
            slider_drag: None,
            pending_stars: HashMap::new(),
            server,
            username,
            password,
            search,
            playlist_name,
            connected: false,
            connecting: false,
            error: None,
            server_info: None,
            albums: Vec::new(),
            album_offset: 0,
            albums_loading: false,
            albums_more: true,
            artists: Vec::new(),
            detail: None,
            artist_view: None,
            search_result: None,
            playlists: Vec::new(),
            playlist_detail: None,
            requested_playlist: None,
            playlists_loading: false,
            playlist_pending: false,
            playlist_editor: None,
            playlist_delete_confirm: false,
            covers: HashMap::new(),
            visible_cover_ids: HashSet::new(),
            failed_covers: HashMap::new(),
            cover_requests: HashSet::new(),
            priority_cover_requests: HashSet::new(),
            playing_cover_ids: Vec::new(),
            settings: saved_settings,
            smtc: None,
            #[cfg(windows)]
            smtc_artwork: None,
            #[cfg(windows)]
            smtc_artwork_request: 0,
            smoke_reported: false,
            client: None,
            engine: Engine::new().ok(),
            queue,
            now_playing: None,
            position_ms: 0,
            duration_ms: 0,
            last_track_index: 0,
            lyrics: Vec::new(),
            lyrics_track: 0,
            lyrics_request: 0,
            lyrics_loading: false,
            lyrics_error: None,
            lyrics_scroll: ScrollHandle::new(),
            lyrics_active: None,
            lyrics_follow: true,
            lyrics_scrolled_to: None,
            lyrics_center_height: 0.,
            lyrics_scroll_generation: 0,
            lyrics_center_retry: false,
            lyrics_return: View::Home,
            random_request: 0,
            random_pending: false,
            random_continue: false,
            random_retry: None,
            scrobbled_playing: false,
            scrobbled_submitted: false,
        };

        if let Some(engine) = root.engine.as_ref() {
            engine.set_volume(volume);
        }

        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(60))
                    .await;

                let responses = match this.update(cx, |this, _| this.api.drain()) {
                    Ok(responses) => responses,
                    Err(_) => break,
                };

                let updated = this.update(cx, |this, cx| {
                    let had_responses = !responses.is_empty();
                    for response in responses {
                        this.handle_response(response, cx);
                    }
                    for command in this
                        .smtc
                        .as_ref()
                        .map(|smtc| smtc.drain())
                        .unwrap_or_default()
                    {
                        match command {
                            smtc::MediaCommand::Play => {
                                if !this.playing {
                                    this.toggle_playback(cx);
                                }
                            }
                            smtc::MediaCommand::Pause => {
                                if this.playing {
                                    this.toggle_playback(cx);
                                }
                            }
                            smtc::MediaCommand::Next => this.play_next(cx),
                            smtc::MediaCommand::Previous => this.play_previous(cx),
                        }
                    }
                    let animated = this.sync_engine(cx);
                    if had_responses || animated || (this.now_playing.is_some() && this.playing) {
                        cx.notify();
                    }
                });

                if updated.is_err() {
                    break;
                }
            }
        })
        .detach();

        root
    }

    fn handle_response(&mut self, response: Response, cx: &mut Context<Self>) {
        match response {
            Response::Lyrics {
                request_id,
                lyrics,
                error,
            } => {
                if request_id == self.lyrics_request && self.now_playing.is_some() {
                    self.lyrics = lyrics;
                    self.lyrics_loading = false;
                    self.lyrics_error = error;
                    self.sync_lyrics();
                }
            }
            Response::Playlists(list) => {
                self.playlists = list;
                self.playlists_loading = false;
            }
            Response::Playlist(playlist) => {
                if self.requested_playlist.as_deref() == Some(&playlist.playlist.id) {
                    self.playlist_detail = Some(*playlist);
                    self.playlist_pending = false;
                }
            }
            Response::PlaylistCreated { playlist, error } => {
                self.playlist_pending = false;
                if let Some(error) = error {
                    self.error = Some(error);
                } else {
                    self.playlist_editor = None;
                    self.api.send_priority(Request::Playlists);
                    if let Some(playlist) = playlist
                        && self.view == View::Playlists
                    {
                        self.requested_playlist = Some(playlist.playlist.id.clone());
                        self.playlist_detail = Some(*playlist);
                    }
                }
            }
            Response::PlaylistChanged { id, deleted, error } => {
                self.playlist_pending = false;
                if let Some(error) = error {
                    self.error = Some(error);
                } else {
                    self.playlist_editor = None;
                    self.playlist_delete_confirm = false;
                    self.api.send_priority(Request::Playlists);
                    if self.requested_playlist.as_deref() == Some(&id) {
                        if deleted {
                            self.playlist_detail = None;
                            self.requested_playlist = None;
                        } else {
                            self.playlist_pending = true;
                            self.api.send_priority(Request::Playlist { id });
                        }
                    }
                }
            }
            Response::Connected(info) => {
                self.connected = true;
                self.connecting = false;
                self.error = None;
                self.server_info = Some(*info);
                self.save_settings(cx);
                self.password
                    .update(cx, |input, cx| input.set_content("", cx));
                self.load_more_albums(cx);
                self.api.send(Request::Artists);
            }
            Response::Albums {
                albums,
                offset,
                size,
            } => {
                if offset != self.album_offset {
                    return;
                }
                self.albums_loading = false;
                self.albums_more = albums.len() == size as usize;
                self.album_offset += albums.len() as u32;
                let mut known: HashSet<_> =
                    self.albums.iter().map(|album| album.id.clone()).collect();
                self.albums.extend(
                    albums
                        .into_iter()
                        .filter(|album| known.insert(album.id.clone())),
                );
            }
            Response::Artists(artists) => self.artists = artists,
            Response::Album(album) => {
                if let Some(id) = album.album.cover_art.as_deref() {
                    self.request_covers(std::iter::once(id));
                }
                self.detail = Some(*album);
            }
            Response::Artist(artist) => {
                self.request_covers(
                    artist
                        .album
                        .iter()
                        .filter_map(|album| album.cover_art.as_deref()),
                );
                self.artist_view = Some(*artist);
            }
            Response::Search(result) => {
                self.request_covers(
                    result
                        .album
                        .iter()
                        .filter_map(|album| album.cover_art.as_deref()),
                );
                self.search_result = Some(*result);
                self.search_results_generation = self.search_results_generation.wrapping_add(1);
            }
            Response::RandomSongs {
                request_id,
                songs,
                error,
            } => {
                if request_id != self.random_request {
                    return;
                }
                self.random_pending = false;
                let continue_playback = std::mem::replace(&mut self.random_continue, false);
                if !self.queue.shuffle() {
                    return;
                }
                if let Some(error) = error {
                    self.random_retry = Some(Instant::now() + Duration::from_secs(30));
                    if !continue_playback {
                        self.error = Some(format!("随机播放失败：{error}"));
                    }
                    return;
                }
                let mut known: HashSet<String> = self
                    .queue
                    .songs()
                    .iter()
                    .map(|song| song.id.clone())
                    .collect();
                let fresh: Vec<Song> = songs
                    .into_iter()
                    .filter(|song| known.insert(song.id.clone()))
                    .collect();
                if fresh.is_empty() {
                    self.random_retry = Some(Instant::now() + Duration::from_secs(30));
                    if !continue_playback {
                        self.error = Some("服务器没有返回新的随机歌曲".into());
                    }
                    return;
                }
                let ended = self
                    .engine
                    .as_ref()
                    .is_some_and(|engine| engine.snapshot().ended);
                if continue_playback || ended {
                    self.queue.append(fresh);
                    if self.queue.next_track().is_some() {
                        self.start_current(cx);
                    }
                } else if self.now_playing.is_some() {
                    // Keep the current song; everything after it becomes library-random.
                    self.queue.clear_upcoming();
                    self.queue.append(fresh);
                    self.refresh_upcoming(cx);
                } else {
                    self.play_songs(fresh, 0, cx);
                }
                cx.notify();
            }
            Response::Cover { id, bytes } => {
                self.cover_requests.remove(&id);
                self.priority_cover_requests.remove(&id);
                if let Some(format) = guess_image_format(&bytes) {
                    self.failed_covers.remove(&id);
                    self.covers
                        .insert(id, Arc::new(Image::from_bytes(format, bytes)));
                    while self.covers.len() > 128 {
                        let key = self
                            .covers
                            .keys()
                            .find(|id| {
                                !self.visible_cover_ids.contains(*id)
                                    && !self.playing_cover_ids.contains(*id)
                            })
                            .cloned();
                        if let Some(key) = key {
                            self.covers.remove(&key);
                        } else {
                            break;
                        }
                    }
                } else {
                    self.failed_covers.insert(id, Instant::now());
                }
            }
            Response::CoverFailed { id } => {
                self.cover_requests.remove(&id);
                self.priority_cover_requests.remove(&id);
                self.failed_covers.insert(id, Instant::now());
            }
            Response::Scrobbled => {}
            Response::Starred { id, error } => {
                if let Some(previous) = self.pending_stars.remove(&id)
                    && let Some(error) = error
                {
                    self.patch_album_star(&id, previous);
                    self.error = Some(format!("收藏更新失败：{error}"));
                }
            }
            Response::Error(error) => {
                self.playlist_pending = false;
                self.playlists_loading = false;
                self.albums_loading = false;
                self.error = Some(error);
                self.connecting = false;
            }
        }
    }

    /// Requests covers for the first screenful, once per cover id.
    fn request_covers<'a>(&mut self, ids: impl Iterator<Item = &'a str>) {
        for id in ids {
            if self
                .failed_covers
                .get(id)
                .is_some_and(|failed| failed.elapsed() < Duration::from_secs(30))
            {
                continue;
            }
            if self.covers.contains_key(id) {
                continue;
            }
            if !self.cover_requests.insert(id.to_owned()) {
                continue;
            }
            self.api.send(Request::Cover {
                id: id.to_owned(),
                size: 240,
            });
        }
    }

    fn load_more_albums(&mut self, cx: &mut Context<Self>) {
        if self.albums_loading || !self.albums_more || !self.connected {
            return;
        }
        self.albums_loading = true;
        self.error = None;
        self.api.send_priority(Request::Albums {
            size: ALBUM_PAGE_SIZE,
            offset: self.album_offset,
        });
        cx.notify();
    }

    fn set_now_playing(&mut self, song: Song) {
        if self
            .now_playing
            .as_ref()
            .is_none_or(|current| current.id != song.id)
        {
            self.clear_lyrics();
            self.request_lyrics(&song);
        }
        self.playing_cover_ids = playback_cover_candidates(
            &song,
            &self.albums,
            self.detail.as_ref().map(|detail| &detail.album),
        );
        for id in &self.playing_cover_ids {
            if !self.covers.contains_key(id) && self.priority_cover_requests.insert(id.clone()) {
                self.api.send_priority(Request::Cover {
                    id: id.clone(),
                    size: 240,
                });
            }
        }
        self.now_playing = Some(song);
    }

    fn clear_lyrics(&mut self) {
        self.lyrics_request = self.lyrics_request.wrapping_add(1);
        self.lyrics.clear();
        self.lyrics_track = 0;
        self.lyrics_loading = false;
        self.lyrics_error = None;
        self.lyrics_active = None;
        self.lyrics_scroll = ScrollHandle::new();
        self.lyrics_follow = true;
        self.lyrics_scrolled_to = None;
        self.lyrics_center_height = 0.;
        self.lyrics_scroll_generation = self.lyrics_scroll_generation.wrapping_add(1);
        self.lyrics_center_retry = false;
    }

    #[cfg(windows)]
    fn sync_smtc_artwork(&mut self, cx: &mut Context<Self>) {
        if self.smtc.as_ref().is_none_or(|smtc| !smtc.available()) {
            return;
        }
        let selected = self.now_playing.as_ref().and_then(|_| {
            self.playing_cover_ids.iter().find_map(|id| {
                self.covers
                    .get(id)
                    .map(|image| ((id.clone(), image.id()), image.clone()))
            })
        });
        let key = selected.as_ref().map(|(key, _)| key.clone());
        if self.smtc_artwork == key {
            return;
        }
        self.smtc_artwork = key;
        self.smtc_artwork_request = self.smtc_artwork_request.wrapping_add(1);
        let request = self.smtc_artwork_request;
        if let Some(smtc) = self.smtc.as_ref() {
            let _ = smtc.set_artwork(None);
        }
        let Some((_, image)) = selected else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let prepared = cx
                .background_executor()
                .spawn(async move { smtc::prepare_artwork(&image.bytes).await })
                .await;
            let _ = this.update(cx, |this, _| {
                // A completed conversion must not put a previous song's image
                // back in the system panel after a switch, stop or fallback.
                if this.smtc_artwork_request == request
                    && let Some(smtc) = this.smtc.as_ref()
                {
                    let current = this.now_playing.as_ref().and_then(|_| {
                        this.playing_cover_ids.iter().find_map(|id| {
                            this.covers.get(id).map(|image| (id.clone(), image.id()))
                        })
                    });
                    if current != this.smtc_artwork {
                        return;
                    }
                    match prepared {
                        Ok(image) => {
                            let _ = smtc.set_artwork(Some(&image));
                        }
                        Err(error) => eprintln!("melos: SMTC artwork failed: {error}"),
                    }
                }
            });
        })
        .detach();
    }

    fn request_lyrics(&mut self, song: &Song) {
        self.lyrics_request = self.lyrics_request.wrapping_add(1);
        self.lyrics_loading = true;
        self.lyrics_error = None;
        self.api.send_priority(Request::Lyrics {
            request_id: self.lyrics_request,
            song: Box::new(song.clone()),
        });
    }

    fn sync_lyrics(&mut self) {
        let active = self.lyrics.get(self.lyrics_track).and_then(|lyrics| {
            let range = lyrics.active_range(self.position_ms);
            (!range.is_empty()).then_some(range.start)
        });
        self.lyrics_active = active;
    }

    /// Target offset that puts the given lyric line in the middle of the viewport.
    fn lyrics_center_target(&self, index: usize) -> Option<f32> {
        let viewport = self.lyrics_scroll.bounds();
        if viewport.size.height <= px(0.) {
            return None;
        }
        let line = self.lyrics_scroll.bounds_for_item(index)?;
        Some((viewport.center().y - line.center().y).as_f32())
    }

    /// Centers the active line whenever it changes. Bounds only exist after the
    /// lyrics view has been laid out, so a missing target is retried on the next
    /// frame instead of silently dropping the scroll.
    fn follow_lyrics(&mut self, cx: &mut Context<Self>) {
        if self.view != View::Lyrics || !self.lyrics_follow {
            return;
        }
        let Some(index) = self.lyrics_active else {
            return;
        };
        let height = self.lyrics_scroll.bounds().size.height.as_f32();
        if self.lyrics_scrolled_to == Some(index) && self.lyrics_center_height == height {
            return;
        }
        let Some(target) = self.lyrics_center_target(index) else {
            if !self.lyrics_center_retry {
                self.lyrics_center_retry = true;
                cx.spawn(async move |this, cx| {
                    cx.background_executor()
                        .timer(Duration::from_millis(32))
                        .await;
                    let _ = this.update(cx, |this, cx| {
                        this.lyrics_center_retry = false;
                        cx.notify();
                    });
                })
                .detach();
            }
            return;
        };
        self.lyrics_scrolled_to = Some(index);
        self.lyrics_center_height = height;
        self.scroll_lyrics_to(target, true, cx);
    }

    fn scroll_lyrics_to(&mut self, target: f32, animate: bool, cx: &mut Context<Self>) {
        self.lyrics_scroll_generation = self.lyrics_scroll_generation.wrapping_add(1);
        let generation = self.lyrics_scroll_generation;
        let start = self.lyrics_scroll.offset().y.as_f32();
        if !animate || (target - start).abs() < 1.0 {
            self.lyrics_scroll.set_offset(point(px(0.), px(target)));
            return;
        }
        let started = Instant::now();
        let delta = target - start;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                let step = this.update(cx, |this, cx| {
                    if this.lyrics_scroll_generation != generation {
                        return true;
                    }
                    let progress = (started.elapsed().as_secs_f32() / 0.24).min(1.0);
                    let eased = 1.0 - (1.0 - progress).powi(3);
                    this.lyrics_scroll
                        .set_offset(point(px(0.), px(start + delta * eased)));
                    cx.notify();
                    progress >= 1.0
                });
                if !matches!(step, Ok(false)) {
                    break;
                }
            }
        })
        .detach();
    }

    fn sidebar_width(&self) -> f32 {
        if self.sidebar_collapsed {
            SIDEBAR_COLLAPSED_WIDTH
        } else {
            SIDEBAR_WIDTH
        }
    }

    /// Persists the current connection fields for the next launch.
    fn save_settings(&mut self, cx: &mut Context<Self>) {
        let server = self.server.read(cx).content().trim().to_owned();
        let username = self.username.read(cx).content().trim().to_owned();
        let password = self.password.read(cx).content().to_owned();
        let settings = self.settings.clone();

        let result = settings.update(
            cx,
            |settings, _| -> Result<(), melos_core::settings::SettingsError> {
                settings.server = server;
                settings.username = username;
                if !password.is_empty() {
                    settings.set_password(&password)?;
                }
                settings.save()
            },
        );
        if let Err(error) = result {
            self.error = Some(format!("设置保存失败：{error}"));
        }
    }

    fn save_playback_settings(&mut self, cx: &mut Context<Self>) {
        let result = self.settings.update(cx, |settings, _| {
            settings.volume = self.volume;
            settings.shuffle = self.queue.shuffle();
            settings.repeat = self.queue.repeat();
            settings.save()
        });
        if let Err(error) = result {
            self.error = Some(format!("设置保存失败：{error}"));
        }
    }

    fn stream_quality(&self, cx: &Context<Self>) -> StreamQuality {
        self.settings
            .read(cx)
            .max_bit_rate
            .map(|max_bit_rate| StreamQuality::Transcode { max_bit_rate })
            .unwrap_or(StreamQuality::Original)
    }

    fn set_quality(&mut self, bitrate: Option<u32>, cx: &mut Context<Self>) {
        let result = self.settings.update(cx, |settings, _| {
            settings.max_bit_rate = bitrate;
            settings.save()
        });
        if let Err(error) = result {
            self.error = Some(format!("设置保存失败：{error}"));
        }
        cx.notify();
    }

    fn settings_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = self.settings.read(cx);
        let server = settings.server.clone();
        let username = settings.username.clone();
        let quality = settings.max_bit_rate;
        let mut choices = div().flex().gap_2();
        for (label, bitrate) in [
            ("原始音质", None),
            ("128 kbps", Some(128)),
            ("192 kbps", Some(192)),
            ("320 kbps", Some(320)),
        ] {
            choices = choices.child(ui::ghost_button(
                label,
                label,
                quality == bitrate,
                cx.listener(move |this: &mut Root, _: &ClickEvent, _, cx| {
                    this.set_quality(bitrate, cx)
                }),
            ));
        }
        div()
            .flex()
            .flex_col()
            .gap_6()
            .child(section(
                "服务器",
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(ui::muted_label(server))
                    .child(ui::muted_label(username))
                    .child(ui::ghost_button(
                        "disconnect",
                        "切换服务器",
                        false,
                        cx.listener(|this: &mut Root, _: &ClickEvent, _, cx| {
                            if let Some(engine) = this.engine.as_ref() {
                                engine.stop();
                            }
                            this.playing = false;
                            this.connected = false;
                            this.client = None;
                            this.queue.clear();
                            this.now_playing = None;
                            this.clear_lyrics();
                            this.albums.clear();
                            this.album_offset = 0;
                            this.albums_loading = false;
                            this.albums_more = true;
                            this.artists.clear();
                            this.covers.clear();
                            this.visible_cover_ids.clear();
                            this.failed_covers.clear();
                            this.cover_requests.clear();
                            this.priority_cover_requests.clear();
                            this.playing_cover_ids.clear();
                            this.pending_stars.clear();
                            this.detail = None;
                            this.artist_view = None;
                            this.search_result = None;
                            this.playlists.clear();
                            this.playlist_detail = None;
                            this.requested_playlist = None;
                            this.playlists_loading = false;
                            this.playlist_pending = false;
                            this.playlist_editor = None;
                            this.playlist_delete_confirm = false;
                            this.api = ApiWorker::new();
                            this.error = None;
                            if let Some(saved) = this.settings.read(cx).password() {
                                this.password
                                    .update(cx, |input, cx| input.set_content(saved, cx));
                            }
                            if let Some(smtc) = this.smtc.as_ref() {
                                smtc.set_status(None);
                            }
                            cx.notify();
                        }),
                    ))
                    .into_any_element(),
            ))
            .child(section(
                "播放音质",
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(choices)
                    .into_any_element(),
            ))
            .child(section(
                "音频输出",
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(ui::ghost_button(
                        "reconnect-output",
                        "重新连接音频输出",
                        false,
                        cx.listener(|this: &mut Root, _: &ClickEvent, _, cx| {
                            this.reconnect_output(cx)
                        }),
                    ))
                    .into_any_element(),
            ))
            .child(section(
                "音量",
                div()
                    .flex()
                    .gap_3()
                    .child(self.volume_slider(cx))
                    .child(ui::muted_label(format!(
                        "{}%",
                        (self.volume * 100.0).round() as u32
                    )))
                    .into_any_element(),
            ))
    }

    fn connect(&mut self, cx: &mut Context<Self>) {
        let server = self.server.read(cx).content().trim().to_owned();
        let username = self.username.read(cx).content().trim().to_owned();
        let password = self.password.read(cx).content().to_owned();

        if server.is_empty() || username.is_empty() {
            self.error = Some("请填写服务器地址和用户名".into());
            cx.notify();
            return;
        }

        self.connecting = true;
        self.error = None;

        match Client::new(
            &server,
            Credentials::new(username.clone(), password.clone()),
        ) {
            Ok(client) => self.client = Some(client),
            Err(error) => {
                self.connecting = false;
                self.error = Some(error.to_string());
                cx.notify();
                return;
            }
        }

        self.api.send(Request::Connect {
            server,
            username,
            password,
        });
        cx.notify();
    }

    // ---------------------------------------------------------------- playback

    fn reconnect_output(&mut self, cx: &mut Context<Self>) {
        if let Some(engine) = self.engine.as_ref() {
            engine.pause();
        }
        self.sync_engine(cx);
        let position = self.position_ms;
        let engine = match Engine::new() {
            Ok(engine) => engine,
            Err(error) => {
                self.error = Some(format!("音频输出连接失败：{error}"));
                cx.notify();
                return;
            }
        };
        engine.set_volume(self.volume);
        if let (Some(client), Some(song)) = (self.client.as_ref(), self.queue.current()) {
            let quality = self.stream_quality(cx);
            engine.restore_playlist(
                stream_spec(client, song, quality),
                self.queue
                    .playback_upcoming()
                    .iter()
                    .map(|song| stream_spec(client, song, quality))
                    .collect(),
                self.queue
                    .playback_cycle()
                    .iter()
                    .map(|song| stream_spec(client, song, quality))
                    .collect(),
                position,
            );
        }
        self.engine = Some(engine);
        self.last_track_index = 0;
        self.playing = false;
        self.error = None;
        cx.notify();
    }

    fn play_songs(&mut self, songs: Vec<Song>, start: usize, cx: &mut Context<Self>) {
        if songs.is_empty() {
            return;
        }
        self.queue.play_album(songs, start);
        self.start_current(cx);
    }

    fn start_current(&mut self, cx: &mut Context<Self>) {
        let Some(client) = self.client.as_ref() else {
            return;
        };
        let Some(song) = self.queue.current().cloned() else {
            return;
        };

        let quality = self.stream_quality(cx);
        let spec = stream_spec(client, &song, quality);
        let upcoming = self
            .queue
            .playback_upcoming()
            .iter()
            .map(|song| stream_spec(client, song, quality))
            .collect();
        let duration = spec.duration_ms.unwrap_or(0);

        let Some(engine) = self.engine.as_ref() else {
            self.error = Some("没有可用的音频输出设备".into());
            cx.notify();
            return;
        };

        let cycle = self
            .queue
            .playback_cycle()
            .iter()
            .map(|song| stream_spec(client, song, quality))
            .collect();
        engine.play_playlist(spec, upcoming, cycle);

        self.set_now_playing(song);
        self.position_ms = 0;
        self.duration_ms = duration;
        self.last_track_index = 0;
        self.playing = true;
        self.scrobbled_playing = false;
        self.scrobbled_submitted = false;
        cx.notify();
    }

    fn toggle_playback(&mut self, cx: &mut Context<Self>) {
        if self.now_playing.is_none() {
            if !self.queue.is_empty() {
                self.start_current(cx);
            }
            return;
        }

        if self
            .engine
            .as_ref()
            .is_some_and(|engine| engine.snapshot().ended)
        {
            self.start_current(cx);
            return;
        }

        let Some(engine) = self.engine.as_ref() else {
            return;
        };

        if self.playing {
            engine.pause();
            self.playing = false;
        } else {
            engine.resume();
            self.playing = true;
        }
        cx.notify();
    }

    fn play_next(&mut self, cx: &mut Context<Self>) {
        self.sync_engine(cx);
        if self.queue.next_track().is_some() {
            self.start_current(cx);
        }
    }

    fn play_previous(&mut self, cx: &mut Context<Self>) {
        self.sync_engine(cx);
        if self.queue.previous().is_some() {
            self.start_current(cx);
        }
    }

    fn refresh_upcoming(&mut self, cx: &Context<Self>) {
        if let (Some(client), Some(engine)) = (&self.client, &self.engine) {
            let quality = self.stream_quality(cx);
            engine.replace_playlist(
                self.queue
                    .playback_upcoming()
                    .iter()
                    .map(|song| stream_spec(client, song, quality))
                    .collect(),
                self.queue
                    .playback_cycle()
                    .iter()
                    .map(|song| stream_spec(client, song, quality))
                    .collect(),
            );
        }
    }

    fn enqueue(&mut self, songs: Vec<Song>, next: bool, cx: &mut Context<Self>) {
        if songs.is_empty() {
            return;
        }
        self.sync_engine(cx);
        let ended = self
            .engine
            .as_ref()
            .is_some_and(|engine| engine.snapshot().ended);
        let start = self.now_playing.is_none() || ended;
        if next && self.queue.current().is_some() {
            for song in songs.into_iter().rev() {
                self.queue.insert_next(song);
            }
        } else {
            self.queue.append(songs);
        }
        if ended {
            self.queue.next_track();
        }
        if start {
            self.start_current(cx);
        } else {
            self.refresh_upcoming(cx);
        }
        cx.notify();
    }

    fn select_queue_song(&mut self, index: usize, cx: &mut Context<Self>) {
        self.sync_engine(cx);
        if self.queue.select(index) {
            self.start_current(cx);
        }
    }

    fn remove_queue_song(&mut self, index: usize, cx: &mut Context<Self>) {
        self.sync_engine(cx);
        let removing_current = self.queue.current_index() == Some(index);
        let paused = !self.playing;
        if self.queue.remove(index).is_none() {
            return;
        }
        if removing_current {
            if self.queue.current().is_some() {
                self.start_current(cx);
                if paused {
                    if let Some(engine) = &self.engine {
                        engine.pause();
                    }
                    self.playing = false;
                }
            } else {
                if let Some(engine) = &self.engine {
                    engine.stop();
                }
                self.now_playing = None;
                self.clear_lyrics();
                self.playing_cover_ids.clear();
                self.position_ms = 0;
                self.duration_ms = 0;
                self.last_track_index = 0;
                self.playing = false;
                if let Some(smtc) = &self.smtc {
                    smtc.set_status(None);
                }
            }
        } else {
            self.refresh_upcoming(cx);
        }
        cx.notify();
    }

    fn move_queue_song(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        self.sync_engine(cx);
        if self.queue.move_song(from, to) {
            self.refresh_upcoming(cx);
            cx.notify();
        }
    }

    fn clear_upcoming(&mut self, cx: &mut Context<Self>) {
        self.sync_engine(cx);
        self.queue.clear_upcoming();
        self.refresh_upcoming(cx);
        cx.notify();
    }

    fn toggle_shuffle(&mut self, cx: &mut Context<Self>) {
        self.sync_engine(cx);
        let on = !self.queue.shuffle();
        self.queue.set_shuffle(on);
        self.random_retry = None;
        self.save_playback_settings(cx);
        if on {
            // Shuffle draws from the whole library, not just the current queue.
            self.request_random_songs(false, cx);
        } else {
            self.random_pending = false;
            self.random_continue = false;
            self.random_request = self.random_request.wrapping_add(1);
            self.refresh_upcoming(cx);
        }
        cx.notify();
    }

    fn request_random_songs(&mut self, continue_playback: bool, cx: &mut Context<Self>) {
        self.random_request = self.random_request.wrapping_add(1);
        self.random_pending = true;
        self.random_continue = continue_playback;
        self.api.send(Request::RandomSongs {
            request_id: self.random_request,
            size: RANDOM_BATCH,
        });
        cx.notify();
    }

    fn cycle_repeat(&mut self, cx: &mut Context<Self>) {
        self.sync_engine(cx);
        self.queue.set_repeat(match self.queue.repeat() {
            RepeatMode::Off => RepeatMode::All,
            RepeatMode::All => RepeatMode::One,
            RepeatMode::One => RepeatMode::Off,
        });
        self.refresh_upcoming(cx);
        self.save_playback_settings(cx);
        cx.notify();
    }

    fn toggle_star(&mut self, cx: &mut Context<Self>) {
        let Some(album) = self.detail.as_ref().map(|detail| &detail.album) else {
            return;
        };
        let starred = album.starred.is_none();
        let id = album.id.clone();
        if self.pending_stars.contains_key(&id) {
            return;
        }
        self.pending_stars.insert(id.clone(), album.starred.clone());
        self.patch_album_star(&id, if starred { Some("now".into()) } else { None });
        self.error = None;
        self.api.send(Request::Star { id, starred });
        cx.notify();
    }

    fn patch_album_star(&mut self, id: &str, starred: Option<String>) {
        let patch = |album: &mut Album| {
            if album.id == id {
                album.starred = starred.clone();
            }
        };
        self.albums.iter_mut().for_each(&patch);
        if let Some(detail) = self.detail.as_mut() {
            patch(&mut detail.album);
        }
        if let Some(artist) = self.artist_view.as_mut() {
            artist.album.iter_mut().for_each(&patch);
        }
        if let Some(result) = self.search_result.as_mut() {
            result.album.iter_mut().for_each(patch);
        }
    }

    fn star_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let starred = self
            .detail
            .as_ref()
            .and_then(|detail| detail.album.starred.as_ref())
            .is_some();

        div()
            .id("star-button")
            .size(px(30.))
            .rounded_md()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .hover(|style| style.bg(rgb(theme::HOVER)))
            .text_sm()
            .text_color(if starred {
                rgb(theme::TEXT)
            } else {
                rgb(theme::TEXT_FAINT)
            })
            .child(if starred { "★" } else { "☆" })
            .on_click(
                cx.listener(|this: &mut Root, _: &ClickEvent, _: &mut Window, cx| {
                    this.toggle_star(cx);
                }),
            )
    }

    fn seek_fraction(&mut self, fraction: f32, cx: &mut Context<Self>) {
        self.sync_engine(cx);
        if self.duration_ms == 0 {
            return;
        }
        let Some(engine) = self.engine.as_ref() else {
            return;
        };

        let target = (f64::from(fraction.clamp(0.0, 1.0)) * self.duration_ms as f64) as u64;
        engine.seek(target);
        self.position_ms = target;
        self.sync_lyrics();

        if target < self.duration_ms / 2 {
            self.scrobbled_playing = false;
        }
        if target < self.duration_ms * 9 / 10 {
            self.scrobbled_submitted = false;
        }

        cx.notify();
    }

    fn set_volume_fraction(&mut self, fraction: f32, cx: &mut Context<Self>) {
        self.volume = fraction.clamp(0.0, 1.0);
        if let Some(engine) = self.engine.as_ref() {
            engine.set_volume(self.volume);
        }
        cx.notify();
    }

    fn begin_slider(
        &mut self,
        kind: SliderKind,
        bounds: Bounds<Pixels>,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.slider_drag = Some(SliderDrag {
            kind,
            bounds,
            fraction: 0.0,
        });
        self.move_slider(position, cx);
    }

    fn move_slider(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(drag) = self.slider_drag.as_mut() else {
            return;
        };
        drag.fraction =
            ((position.x - drag.bounds.origin.x) / drag.bounds.size.width).clamp(0.0, 1.0);
        let kind = drag.kind;
        let fraction = drag.fraction;
        match kind {
            SliderKind::Progress => {
                self.position_ms = (f64::from(fraction) * self.duration_ms as f64) as u64
            }
            SliderKind::Volume => self.set_volume_fraction(fraction, cx),
        }
        cx.notify();
    }

    fn end_slider(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.move_slider(position, cx);
        if let Some(drag) = self.slider_drag.take() {
            match drag.kind {
                SliderKind::Progress => self.seek_fraction(drag.fraction, cx),
                SliderKind::Volume => self.save_playback_settings(cx),
            }
        }
    }

    /// Drag previews position; release performs a single network seek.
    fn progress_slider(&self, fraction: f32, cx: &mut Context<Self>) -> impl IntoElement {
        let bounds: Rc<Cell<Option<Bounds<Pixels>>>> = Rc::new(Cell::new(None));
        let paint_bounds = bounds.clone();
        let input_bounds = bounds.clone();
        let painted = fraction;

        div()
            .id("progress-slider")
            .w_full()
            .h(px(16.))
            .flex()
            .items_center()
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this: &mut Root, event: &MouseDownEvent, _, cx| {
                    if let Some(b) = input_bounds.get() {
                        this.begin_slider(SliderKind::Progress, b, event.position, cx);
                    }
                }),
            )
            .child(
                canvas(
                    move |b, _, _| {
                        paint_bounds.set(Some(b));
                    },
                    move |b, _, window, _| {
                        paint_track(b, 4.0, painted, window);
                    },
                )
                .size_full(),
            )
    }

    /// Volume follows the pointer while dragging.
    fn volume_slider(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let bounds: Rc<Cell<Option<Bounds<Pixels>>>> = Rc::new(Cell::new(None));
        let paint_bounds = bounds.clone();
        let input_bounds = bounds.clone();
        let painted = self.volume.clamp(0.0, 1.0);

        div()
            .id("volume-slider")
            .w(px(72.))
            .h(px(16.))
            .flex()
            .items_center()
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this: &mut Root, event: &MouseDownEvent, _, cx| {
                    if let Some(b) = input_bounds.get() {
                        this.begin_slider(SliderKind::Volume, b, event.position, cx);
                    }
                }),
            )
            .child(
                canvas(
                    move |b, _, _| {
                        paint_bounds.set(Some(b));
                    },
                    move |b, _, window, _| {
                        paint_track(b, 3.0, painted, window);
                    },
                )
                .size_full(),
            )
    }

    /// Pulls the engine snapshot into the UI state. Returns true when a redraw
    /// is warranted.
    fn sync_engine(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(engine) = self.engine.as_ref() else {
            return false;
        };
        let snapshot = engine.snapshot();
        let mut changed = false;
        if let Some(error) = engine.take_error() {
            self.error = Some(format!("播放失败：{error}"));
            changed = true;
        }

        if !self
            .slider_drag
            .is_some_and(|drag| drag.kind == SliderKind::Progress)
            && snapshot.position_ms != self.position_ms
        {
            self.position_ms = snapshot.position_ms;
            changed = true;
        }
        if snapshot.duration_ms > 0 && snapshot.duration_ms != self.duration_ms {
            self.duration_ms = snapshot.duration_ms;
            changed = true;
        }
        if snapshot.playing != self.playing {
            self.playing = snapshot.playing;
            changed = true;
        }

        if snapshot.track_index > self.last_track_index {
            let advance = (snapshot.track_index - self.last_track_index) as usize;
            self.last_track_index = snapshot.track_index;
            for _ in 0..advance {
                self.queue.advance();
            }
            if let Some(song) = self.queue.current().cloned() {
                self.set_now_playing(song);
            }
            self.position_ms = 0;
            self.duration_ms = self
                .now_playing
                .as_ref()
                .and_then(|song| song.duration)
                .map(|seconds| u64::from(seconds) * 1000)
                .unwrap_or(0);
            self.scrobbled_playing = false;
            self.scrobbled_submitted = false;

            changed = true;
        }

        if let Some(song_id) = self.now_playing.as_ref().map(|song| song.id.clone())
            && self.duration_ms > 0
        {
            let half = self.duration_ms / 2;
            let ninety = self.duration_ms * 9 / 10;

            if !self.scrobbled_playing && self.position_ms >= half {
                self.scrobbled_playing = true;
                self.api.send(Request::Scrobble {
                    id: song_id.clone(),
                    submission: false,
                });
            }
            if !self.scrobbled_submitted && self.position_ms >= ninety {
                self.scrobbled_submitted = true;
                self.api.send(Request::Scrobble {
                    id: song_id,
                    submission: true,
                });
            }
        }

        self.sync_lyrics();
        let natural_end = snapshot.ended && self.playing;
        if natural_end {
            self.playing = false;
            changed = true;
        }
        // Library shuffle keeps playing when a finite queue reaches its end.
        if natural_end
            && self.queue.shuffle()
            && self.queue.repeat() == RepeatMode::Off
            && !self.random_pending
            && self
                .random_retry
                .is_none_or(|retry| Instant::now() >= retry)
        {
            self.request_random_songs(true, cx);
        }

        self.sync_smtc_playback();
        #[cfg(windows)]
        self.sync_smtc_artwork(cx);
        changed
    }

    fn sync_smtc_playback(&self) {
        if let Some(smtc) = self.smtc.as_ref() {
            match self.now_playing.as_ref() {
                Some(song) => {
                    smtc.update(
                        &song.title,
                        song.artist.as_deref().unwrap_or(""),
                        self.position_ms,
                        self.duration_ms,
                    );
                    smtc.set_status(Some(self.playing));
                }
                None => smtc.set_status(None),
            }
        }
    }

    fn run_search(&mut self, cx: &mut Context<Self>) {
        let query = self.search.read(cx).content().trim().to_owned();
        if query.is_empty() {
            return;
        }
        self.search_result = None;
        self.api.send(Request::Search { query });
        cx.notify();
    }

    fn select(&mut self, view: View, cx: &mut Context<Self>) {
        self.view = view;
        self.detail = None;
        self.artist_view = None;
        self.playlist_detail = None;
        self.requested_playlist = None;
        self.playlist_editor = None;
        self.playlist_delete_confirm = false;
        if view == View::Playlists {
            self.playlists_loading = true;
            self.api.send_priority(Request::Playlists);
        }
        cx.notify();
    }

    /// Opens or closes the lyrics page from the artwork, keeping the browsing
    /// context (album detail, artist page) intact.
    fn toggle_lyrics(&mut self, cx: &mut Context<Self>) {
        if self.view == View::Lyrics {
            self.view = self.lyrics_return;
        } else {
            if self.now_playing.is_none() {
                return;
            }
            self.lyrics_return = self.view;
            self.view = View::Lyrics;
            // The previous page had no lyric children, so start from a clean
            // handle instead of measuring against stale line bounds.
            self.lyrics_scroll = ScrollHandle::new();
            self.lyrics_center_retry = false;
        }
        self.lyrics_scrolled_to = None;
        self.lyrics_center_height = 0.;
        cx.notify();
    }

    fn open_album(&mut self, id: String, cx: &mut Context<Self>) {
        self.api.send(Request::Album { id });
        cx.notify();
    }

    fn open_artist(&mut self, id: String, cx: &mut Context<Self>) {
        self.api.send(Request::Artist { id });
        cx.notify();
    }

    // ---------------------------------------------------------------- connect

    fn connect_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let connect_button = ui::primary_button(
            if self.connecting {
                "连接中…"
            } else {
                "连接"
            },
            !self.connecting,
            cx.listener(|this: &mut Root, _: &ClickEvent, _: &mut Window, cx| {
                this.connect(cx);
            }),
        );

        div()
            .flex()
            .size_full()
            .items_center()
            .justify_center()
            .bg(theme::backdrop())
            .text_color(rgb(theme::TEXT))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .w(px(420.))
                    .p_8()
                    .rounded_md()
                    .bg(rgba(0xFFFFFF08))
                    .border_1()
                    .border_color(rgb(theme::BORDER))
                    .child(div().text_2xl().child("连接服务器"))
                    .child(field("服务器地址", &self.server))
                    .child(field("用户名", &self.username))
                    .child(field("密码", &self.password))
                    .child(connect_button)
                    .child(
                        div()
                            .min_h(px(16.))
                            .text_xs()
                            .text_color(rgb(0xE06C75))
                            .child(self.error.clone().unwrap_or_default()),
                    ),
            )
    }

    // ------------------------------------------------------------------ shell

    fn shell(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut main = div()
            .flex_1()
            .flex()
            .min_h_0()
            .child(self.sidebar(cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .when(self.view != View::Lyrics, |body| body.child(self.header()))
                    .child(self.body(window, cx)),
            );

        if self.queue_open {
            main = main.child(self.queue_drawer(cx));
        }

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(theme::backdrop())
            .text_color(rgb(theme::TEXT))
            .child(main)
            .child(
                div()
                    .px_4()
                    .text_xs()
                    .text_color(rgb(0xE06C75))
                    .child(self.error.clone().unwrap_or_default()),
            )
            .child(self.player_bar(cx))
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut nav = div().flex().flex_col().gap_1();

        for view in [
            View::Home,
            View::Artists,
            View::Albums,
            View::Playlists,
            View::Search,
        ] {
            nav = nav.child(if self.sidebar_collapsed {
                ui::icon_button(
                    view.label(),
                    view.icon(),
                    self.view == view,
                    cx.listener(move |this: &mut Root, _: &ClickEvent, _, cx| {
                        this.select(view, cx);
                    }),
                )
                .into_any_element()
            } else {
                ui::ghost_button(
                    view.label(),
                    view.label(),
                    self.view == view,
                    cx.listener(move |this: &mut Root, _: &ClickEvent, _, cx| {
                        this.select(view, cx);
                    }),
                )
                .into_any_element()
            });
        }

        div()
            .flex()
            .flex_col()
            .w(px(self.sidebar_width()))
            .h_full()
            .flex_shrink_0()
            .px_3()
            .pt_6()
            .pb_4()
            .gap_6()
            .bg(rgba(0x0A0A0ACC))
            .border_r_1()
            .border_color(rgb(theme::BORDER))
            .child(ui::icon_button(
                "toggle-sidebar",
                if self.sidebar_collapsed {
                    ui::Icon::SidebarExpand
                } else {
                    ui::Icon::SidebarCollapse
                },
                false,
                cx.listener(|this: &mut Root, _: &ClickEvent, _, cx| {
                    this.sidebar_collapsed = !this.sidebar_collapsed;
                    cx.notify();
                }),
            ))
            .child(nav)
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(if self.sidebar_collapsed {
                        ui::icon_button(
                            "settings",
                            ui::Icon::Settings,
                            self.view == View::Settings,
                            cx.listener(|this: &mut Root, _: &ClickEvent, _, cx| {
                                this.select(View::Settings, cx)
                            }),
                        )
                        .into_any_element()
                    } else {
                        ui::ghost_button(
                            "settings",
                            View::Settings.label(),
                            self.view == View::Settings,
                            cx.listener(|this: &mut Root, _: &ClickEvent, _, cx| {
                                this.select(View::Settings, cx)
                            }),
                        )
                        .into_any_element()
                    }),
            )
    }

    fn header(&self) -> impl IntoElement {
        div()
            .flex()
            .items_end()
            .justify_between()
            .px_8()
            .pt_8()
            .pb_5()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().text_3xl().child(self.view.label())),
            )
    }

    fn body(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        if self.view == View::Lyrics {
            return self.lyrics_view(window, cx);
        }
        if let Some(detail) = &self.detail {
            return div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .overflow_hidden()
                .px_8()
                .pb_8()
                .child(self.detail_view(detail, cx))
                .into_any_element();
        }
        if self.view == View::Playlists && self.playlist_detail.is_some() {
            return div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .overflow_hidden()
                .px_8()
                .pb_8()
                .child(self.playlists_view(cx))
                .into_any_element();
        }
        if self.detail.is_none()
            && self.artist_view.is_none()
            && matches!(self.view, View::Home | View::Albums)
        {
            return self.library_view(window, cx);
        }
        let content: AnyElement = if let Some(artist) = &self.artist_view {
            self.artist_view(artist, window, cx).into_any_element()
        } else {
            match self.view {
                View::Home | View::Albums => {
                    if self.albums.is_empty() {
                        placeholder("暂无专辑").into_any_element()
                    } else {
                        self.album_grid(&self.albums, window, cx)
                    }
                }
                View::Artists => self.artists_view(cx),
                View::Playlists => self.playlists_view(cx).into_any_element(),
                View::Search => self.search_view(window, cx).into_any_element(),
                View::Settings => self.settings_view(cx).into_any_element(),
                View::Lyrics => unreachable!("lyrics view handled above"),
            }
        };

        div()
            .id("content-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px_8()
            .pb_8()
            .child(content)
            .into_any_element()
    }

    fn library_view(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let columns = grid_columns(window, self.queue_open, self.sidebar_width());
        let available = window.viewport_size().width.as_f32()
            - self.sidebar_width()
            - CONTENT_PADDING
            - if self.queue_open { QUEUE_WIDTH } else { 0. };
        let card_width = ((available - CARD_GAP * (columns - 1) as f32) / columns as f32).max(80.);
        let rows = self.albums.len().div_ceil(columns);
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            .px_8()
            .pb_8()
            .gap_3()
            .child(
                uniform_list(
                    "album-library",
                    rows,
                    cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                        let visible: Vec<_> = range
                            .clone()
                            .flat_map(|row| {
                                row * columns..((row + 1) * columns).min(this.albums.len())
                            })
                            .map(|index| (index, this.albums[index].clone()))
                            .collect();
                        this.visible_cover_ids = visible
                            .iter()
                            .map(|(_, album)| {
                                album.cover_art.clone().unwrap_or_else(|| album.id.clone())
                            })
                            .collect();
                        this.request_covers(
                            visible
                                .iter()
                                .map(|(_, album)| album.cover_art.as_deref().unwrap_or(&album.id)),
                        );
                        range
                            .map(|row| {
                                div()
                                    .h(px(card_width + 78.))
                                    .pb_6()
                                    .grid()
                                    .grid_cols(columns as u16)
                                    .gap_6()
                                    .w_full()
                                    .children(
                                        visible
                                            .iter()
                                            .filter(|(index, _)| *index / columns == row)
                                            .map(|(index, album)| {
                                                this.album_card(*index, album, cx)
                                                    .into_any_element()
                                            }),
                                    )
                            })
                            .collect::<Vec<_>>()
                    }),
                )
                .flex_1()
                .min_h_0()
                .w_full(),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .when(self.albums_more, |bar| {
                        bar.child(ui::ghost_button(
                            "load-more-albums",
                            if self.albums_loading {
                                "加载中…"
                            } else {
                                "加载更多"
                            },
                            false,
                            cx.listener(|this: &mut Root, _: &ClickEvent, _, cx| {
                                this.load_more_albums(cx);
                            }),
                        ))
                    })
                    .child(ui::ghost_button(
                        "retry-covers",
                        "重试封面",
                        false,
                        cx.listener(|this: &mut Root, _: &ClickEvent, _, cx| {
                            this.failed_covers.clear();
                            if let Some(song) = this.now_playing.clone() {
                                this.set_now_playing(song);
                            }
                            cx.notify();
                        }),
                    )),
            )
            .into_any_element()
    }

    fn open_playlist(&mut self, id: String, cx: &mut Context<Self>) {
        self.requested_playlist = Some(id.clone());
        self.playlist_detail = None;
        self.playlist_editor = None;
        self.playlist_delete_confirm = false;
        self.error = None;
        self.api.send_priority(Request::Playlist { id });
        cx.notify();
    }

    fn submit_playlist_name(&mut self, cx: &mut Context<Self>) {
        if self.playlist_pending {
            return;
        }
        let name = self.playlist_name.read(cx).content().trim().to_owned();
        if name.is_empty() {
            self.error = Some("请输入歌单名称".into());
            cx.notify();
            return;
        }
        let request = match self.playlist_editor {
            Some(PlaylistEditor::Create) => Request::CreatePlaylist {
                name,
                songs: self
                    .queue
                    .songs()
                    .iter()
                    .map(|song| song.id.clone())
                    .collect(),
            },
            Some(PlaylistEditor::Rename) => {
                let Some(playlist) = &self.playlist_detail else {
                    return;
                };
                Request::UpdatePlaylist {
                    id: playlist.playlist.id.clone(),
                    name: Some(name),
                    add: vec![],
                    remove: vec![],
                }
            }
            None => return,
        };
        self.playlist_pending = true;
        self.error = None;
        self.api.send_priority(request);
        cx.notify();
    }

    fn edit_playlist_songs(
        &mut self,
        add_queue: bool,
        remove: Option<u32>,
        cx: &mut Context<Self>,
    ) {
        if self.playlist_pending {
            return;
        }
        let Some(playlist) = &self.playlist_detail else {
            return;
        };
        let add = if add_queue {
            self.queue
                .songs()
                .iter()
                .map(|song| song.id.clone())
                .collect()
        } else {
            vec![]
        };
        if add_queue && self.queue.is_empty() {
            return;
        }
        self.playlist_pending = true;
        self.error = None;
        self.api.send_priority(Request::UpdatePlaylist {
            id: playlist.playlist.id.clone(),
            name: None,
            add,
            remove: remove.into_iter().collect(),
        });
        cx.notify();
    }

    fn playlists_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut view = div().flex().flex_col().gap_4();
        if self.playlist_editor.is_some() {
            view = view.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(self.playlist_name.clone())
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(ui::ghost_button(
                                "playlist-save-name",
                                if self.playlist_pending {
                                    "保存中…"
                                } else {
                                    "保存"
                                },
                                false,
                                cx.listener(|this: &mut Root, _: &ClickEvent, _, cx| {
                                    this.submit_playlist_name(cx);
                                }),
                            ))
                            .child(ui::ghost_button(
                                "playlist-cancel-name",
                                "取消",
                                false,
                                cx.listener(|this: &mut Root, _: &ClickEvent, _, cx| {
                                    if !this.playlist_pending {
                                        this.playlist_editor = None;
                                        cx.notify();
                                    }
                                }),
                            )),
                    ),
            );
        }
        if let Some(playlist) = &self.playlist_detail {
            let editable = playlist
                .playlist
                .owner
                .as_deref()
                .is_none_or(|owner| owner == self.settings.read(cx).username);
            view = view
                .child(ui::ghost_button(
                    "playlist-back",
                    "← 所有歌单",
                    false,
                    cx.listener(|this: &mut Root, _: &ClickEvent, _, cx| {
                        this.playlist_detail = None;
                        this.requested_playlist = None;
                        this.playlist_editor = None;
                        this.playlist_delete_confirm = false;
                        cx.notify();
                    }),
                ))
                .child(div().text_2xl().child(playlist.playlist.name.clone()))
                .child(ui::muted_label(format!(
                    "{} 首 · {}",
                    playlist.entry.len(),
                    playlist.playlist.owner.clone().unwrap_or_default()
                )));
            view = view.child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(ui::ghost_button(
                        "playlist-play",
                        "播放全部",
                        false,
                        cx.listener(move |this: &mut Root, _: &ClickEvent, _, cx| {
                            this.play_source(SongSource::Playlist, 0, cx);
                        }),
                    ))
                    .child(ui::ghost_button(
                        "playlist-enqueue",
                        "加入队列",
                        false,
                        cx.listener(move |this: &mut Root, _: &ClickEvent, _, cx| {
                            this.enqueue_source(SongSource::Playlist, cx);
                        }),
                    ))
                    .when(editable, |bar| {
                        bar.child(ui::ghost_button(
                            "playlist-add-queue",
                            "将队列追加到歌单",
                            false,
                            cx.listener(|this: &mut Root, _: &ClickEvent, _, cx| {
                                this.edit_playlist_songs(true, None, cx);
                            }),
                        ))
                        .child(ui::ghost_button(
                            "playlist-rename",
                            "重命名",
                            false,
                            cx.listener(|this: &mut Root, _: &ClickEvent, _, cx| {
                                if this.playlist_pending {
                                    return;
                                }
                                let name = this
                                    .playlist_detail
                                    .as_ref()
                                    .map(|playlist| playlist.playlist.name.clone())
                                    .unwrap_or_default();
                                this.playlist_name
                                    .update(cx, |input, cx| input.set_content(name, cx));
                                this.playlist_editor = Some(PlaylistEditor::Rename);
                                cx.notify();
                            }),
                        ))
                        .child(ui::ghost_button(
                            "playlist-delete",
                            if self.playlist_delete_confirm {
                                "确认删除歌单"
                            } else {
                                "删除歌单"
                            },
                            false,
                            cx.listener(|this: &mut Root, _: &ClickEvent, _, cx| {
                                if this.playlist_pending {
                                    return;
                                }
                                if !this.playlist_delete_confirm {
                                    this.playlist_delete_confirm = true;
                                    cx.notify();
                                    return;
                                }
                                let Some(playlist) = &this.playlist_detail else {
                                    return;
                                };
                                this.api.send_priority(Request::DeletePlaylist {
                                    id: playlist.playlist.id.clone(),
                                });
                                this.playlist_pending = true;
                                cx.notify();
                            }),
                        ))
                    }),
            );
            view = view
                .flex_1()
                .min_h_0()
                .child(self.song_list(SongSource::Playlist, cx));
        } else if self.requested_playlist.is_some() {
            view = view
                .child(placeholder(if self.error.is_some() {
                    "歌单加载失败，返回列表后可重试"
                } else {
                    "正在加载歌单…"
                }))
                .child(ui::ghost_button(
                    "playlist-retry-back",
                    "返回歌单列表",
                    false,
                    cx.listener(|this: &mut Root, _: &ClickEvent, _, cx| {
                        this.requested_playlist = None;
                        cx.notify();
                    }),
                ));
        } else {
            view = view.child(
                div()
                    .flex()
                    .gap_2()
                    .child(ui::ghost_button(
                        "playlist-create",
                        "新建歌单",
                        false,
                        cx.listener(|this: &mut Root, _: &ClickEvent, _, cx| {
                            if this.playlist_pending {
                                return;
                            }
                            this.playlist_name
                                .update(cx, |input, cx| input.set_content("", cx));
                            this.playlist_editor = Some(PlaylistEditor::Create);
                            cx.notify();
                        }),
                    ))
                    .child(ui::ghost_button(
                        "playlist-refresh",
                        "刷新",
                        false,
                        cx.listener(|this: &mut Root, _: &ClickEvent, _, cx| {
                            this.api.send_priority(Request::Playlists);
                            this.playlists_loading = true;
                            cx.notify();
                        }),
                    )),
            );
            if self.playlists_loading {
                view = view.child(ui::muted_label("加载中…"));
            }
            for playlist in &self.playlists {
                let id = playlist.id.clone();
                view = view.child(
                    div()
                        .id(format!("playlist-{id}"))
                        .flex()
                        .justify_between()
                        .p_3()
                        .rounded_md()
                        .cursor_pointer()
                        .hover(|style| style.bg(rgb(theme::HOVER)))
                        .child(div().text_sm().child(playlist.name.clone()))
                        .child(ui::muted_label(format!(
                            "{} 首",
                            playlist.song_count.unwrap_or(0)
                        )))
                        .on_click(cx.listener(move |this: &mut Root, _: &ClickEvent, _, cx| {
                            this.open_playlist(id.clone(), cx);
                        })),
                );
            }
            if self.playlists.is_empty() && !self.playlists_loading {
                view = view.child(placeholder("暂无歌单，可以从当前队列新建"));
            }
        }
        view
    }

    // ------------------------------------------------------------------ albums

    fn source_songs(&self, source: SongSource) -> &[Song] {
        match source {
            SongSource::Album => self.detail.as_ref().map(|album| album.song.as_slice()),
            SongSource::Playlist => self
                .playlist_detail
                .as_ref()
                .map(|playlist| playlist.entry.as_slice()),
            SongSource::Search => self
                .search_result
                .as_ref()
                .map(|result| result.song.as_slice()),
        }
        .unwrap_or(&[])
    }

    fn play_source(&mut self, source: SongSource, index: usize, cx: &mut Context<Self>) {
        let songs = self.source_songs(source);
        if index < songs.len() {
            self.play_songs(songs.to_vec(), index, cx);
        }
    }

    fn enqueue_source(&mut self, source: SongSource, cx: &mut Context<Self>) {
        let songs = self.source_songs(source).to_vec();
        self.enqueue(songs, false, cx);
    }

    fn song_list(&self, source: SongSource, cx: &mut Context<Self>) -> AnyElement {
        let id = match source {
            SongSource::Album => format!(
                "album-songs-{}",
                self.detail
                    .as_ref()
                    .map(|album| album.album.id.as_str())
                    .unwrap_or_default()
            ),
            SongSource::Playlist => format!(
                "playlist-songs-{}",
                self.playlist_detail
                    .as_ref()
                    .map(|playlist| playlist.playlist.id.as_str())
                    .unwrap_or_default()
            ),
            SongSource::Search => format!("search-songs-{}", self.search_results_generation),
        };
        let count = self.source_songs(source).len();
        let editable = matches!(source, SongSource::Playlist)
            && self.playlist_detail.as_ref().is_some_and(|playlist| {
                playlist
                    .playlist
                    .owner
                    .as_deref()
                    .is_none_or(|owner| owner == self.settings.read(cx).username)
            });
        uniform_list(
            id,
            count,
            cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                range
                    .filter_map(|index| {
                        this.source_songs(source).get(index).map(|song| {
                            div()
                                .h(px(SONG_ROW_HEIGHT))
                                .w_full()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .child(song_row(index, song, source, cx)),
                                )
                                .when(editable, |row| {
                                    row.child(ui::ghost_button(
                                        format!("playlist-remove-{index}"),
                                        "移除",
                                        false,
                                        cx.listener(
                                            move |this: &mut Root, _: &ClickEvent, _, cx| {
                                                cx.stop_propagation();
                                                this.edit_playlist_songs(
                                                    false,
                                                    Some(index as u32),
                                                    cx,
                                                );
                                            },
                                        ),
                                    ))
                                })
                        })
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .flex_1()
        .min_h_0()
        .w_full()
        .into_any_element()
    }

    fn album_grid(&self, albums: &[Album], window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let columns = grid_columns(window, self.queue_open, self.sidebar_width());

        let mut cards = Vec::with_capacity(albums.len());
        for (index, album) in albums.iter().enumerate() {
            cards.push(self.album_card(index, album, cx).into_any_element());
        }

        div()
            .grid()
            .grid_cols(columns as u16)
            .gap_6()
            .w_full()
            .children(cards)
            .into_any_element()
    }

    fn cover(&self, id: Option<&str>, index: usize) -> AnyElement {
        if let Some(image) = id.and_then(|id| self.covers.get(id)) {
            return div()
                .w_full()
                .aspect_ratio(1.0)
                .rounded_md()
                .overflow_hidden()
                .child(
                    img(ImageSource::Image(image.clone()))
                        .size_full()
                        .object_fit(ObjectFit::Cover),
                )
                .into_any_element();
        }

        div()
            .w_full()
            .aspect_ratio(1.0)
            .rounded_md()
            .bg(theme::cover(index))
            .into_any_element()
    }

    fn album_card(&self, index: usize, album: &Album, cx: &mut Context<Self>) -> impl IntoElement {
        let id = album.id.clone();
        let artist = album.artist.clone().unwrap_or_default();

        div()
            .id(format!("album-{}", album.id))
            .flex()
            .flex_col()
            .gap_2()
            .min_w_0()
            .cursor_pointer()
            .child(self.cover(Some(album.cover_art.as_deref().unwrap_or(&album.id)), index))
            .child(div().text_sm().truncate().child(album.name.clone()))
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(theme::TEXT_MUTED))
                    .truncate()
                    .child(artist),
            )
            .on_click(
                cx.listener(move |this: &mut Root, _: &ClickEvent, _: &mut Window, cx| {
                    this.open_album(id.clone(), cx);
                }),
            )
    }

    fn detail_view(&self, album: &AlbumWithSongs, cx: &mut Context<Self>) -> impl IntoElement {
        let songs = self.song_list(SongSource::Album, cx);

        let play_all = {
            ui::primary_button(
                "播放全部",
                true,
                cx.listener(move |this: &mut Root, _: &ClickEvent, _: &mut Window, cx| {
                    this.play_source(SongSource::Album, 0, cx);
                }),
            )
        };

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap_5()
            .child(back_button(cx))
            .child(
                div()
                    .flex()
                    .flex_shrink_0()
                    .items_end()
                    .gap_5()
                    .child(
                        div()
                            .size(px(152.))
                            .child(self.cover(album.album.cover_art.as_deref(), 0)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(div().text_3xl().child(album.album.name.clone()))
                            .child(ui::muted_label(
                                album.album.artist.clone().unwrap_or_default(),
                            ))
                            .child(ui::muted_label(format!("{} 首", album.song.len())))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_3()
                                    .child(div().w(px(120.)).child(play_all))
                                    .child(ui::ghost_button(
                                        "album-enqueue",
                                        "加入队列",
                                        false,
                                        cx.listener(
                                            move |this: &mut Root, _: &ClickEvent, _, cx| {
                                                this.enqueue_source(SongSource::Album, cx);
                                            },
                                        ),
                                    ))
                                    .child(self.star_button(cx)),
                            ),
                    ),
            )
            .child(songs)
    }

    fn artist_view(
        &self,
        artist: &ArtistWithAlbums,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(back_button(cx))
            .child(div().text_3xl().child(artist.artist.name.clone()))
            .child(if artist.album.is_empty() {
                placeholder("这位艺人暂无专辑").into_any_element()
            } else {
                self.album_grid(&artist.album, window, cx)
            })
    }

    fn artists_view(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.artists.is_empty() {
            return placeholder("暂无艺人").into_any_element();
        }

        let mut list = div().flex().flex_col().gap_1();

        for artist in &self.artists {
            let id = artist.id.clone();
            list = list.child(
                div()
                    .id(format!("artist-{}", artist.id))
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .py_3()
                    .rounded_md()
                    .cursor_pointer()
                    .hover(|style| style.bg(rgba(0xFFFFFF0A)))
                    .child(div().text_sm().child(artist.name.clone()))
                    .child(ui::muted_label(format!(
                        "{} 张专辑",
                        artist.album_count.unwrap_or(0)
                    )))
                    .on_click(cx.listener(
                        move |this: &mut Root, _: &ClickEvent, _: &mut Window, cx| {
                            this.open_artist(id.clone(), cx);
                        },
                    )),
            );
        }

        list.into_any_element()
    }

    // ------------------------------------------------------------------ search

    fn search_view(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let results: AnyElement = match &self.search_result {
            Some(result) => self.search_results(result, window, cx),
            None => placeholder("输入关键词后点击搜索").into_any_element(),
        };

        div()
            .flex()
            .flex_col()
            .gap_6()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().child(self.search.clone()))
                    .child(div().w(px(88.)).child(ui::primary_button(
                        "搜索",
                        true,
                        cx.listener(|this: &mut Root, _: &ClickEvent, _: &mut Window, cx| {
                            this.run_search(cx);
                        }),
                    ))),
            )
            .child(results)
    }

    fn search_results(
        &self,
        result: &SearchResult,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut sections = div().flex().flex_col().gap_6();

        if !result.artist.is_empty() {
            let mut artists = div().flex().flex_col().gap_1();
            for artist in &result.artist {
                let id = artist.id.clone();
                artists = artists.child(
                    div()
                        .id(format!("search-artist-{}", artist.id))
                        .flex()
                        .items_center()
                        .justify_between()
                        .px_3()
                        .py_2()
                        .rounded_md()
                        .cursor_pointer()
                        .hover(|style| style.bg(rgba(0xFFFFFF0A)))
                        .child(div().text_sm().child(artist.name.clone()))
                        .on_click(cx.listener(
                            move |this: &mut Root, _: &ClickEvent, _: &mut Window, cx| {
                                this.open_artist(id.clone(), cx);
                            },
                        )),
                );
            }
            sections = sections.child(section("艺人", artists.into_any_element()));
        }

        if !result.album.is_empty() {
            sections = sections.child(section("专辑", self.album_grid(&result.album, window, cx)));
        }

        if !result.song.is_empty() {
            let songs = div()
                .flex()
                .flex_col()
                .h(px((result.song.len() as f32 * SONG_ROW_HEIGHT).min(456.)))
                .child(self.song_list(SongSource::Search, cx));
            sections = sections.child(section("歌曲", songs.into_any_element()));
        }

        sections.into_any_element()
    }

    // -------------------------------------------------------------- player bar

    fn player_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let title = self
            .now_playing
            .as_ref()
            .map(|song| song.title.clone())
            .unwrap_or_else(|| "未在播放".into());
        let artist = self
            .now_playing
            .as_ref()
            .and_then(|song| song.artist.clone())
            .unwrap_or_default();
        let cover_id = self
            .playing_cover_ids
            .iter()
            .find(|id| self.covers.contains_key(*id))
            .or_else(|| self.playing_cover_ids.first())
            .map(String::as_str);
        let fraction = if self.duration_ms > 0 {
            self.position_ms as f32 / self.duration_ms as f32
        } else {
            0.0
        };

        div()
            .h(px(76.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_4()
            .px_4()
            .bg(rgba(0x0C0C10E6))
            .border_t_1()
            .border_color(rgb(theme::BORDER))
            .child(
                div()
                    .id("player-artwork")
                    .size(px(48.))
                    .flex_shrink_0()
                    .overflow_hidden()
                    .cursor_pointer()
                    .on_click(cx.listener(|this: &mut Root, _: &ClickEvent, _, cx| {
                        this.toggle_lyrics(cx);
                    }))
                    .child(self.cover(cover_id, 0)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w(px(180.))
                    .child(div().text_sm().child(title))
                    .child(ui::muted_label(artist)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(ui::transport_button(
                        "prev",
                        ui::Icon::Prev,
                        false,
                        cx.listener(|this: &mut Root, _: &ClickEvent, _: &mut Window, cx| {
                            this.play_previous(cx);
                        }),
                    ))
                    .child(ui::transport_button(
                        "play",
                        if self.playing {
                            ui::Icon::Pause
                        } else {
                            ui::Icon::Play
                        },
                        true,
                        cx.listener(|this: &mut Root, _: &ClickEvent, _: &mut Window, cx| {
                            this.toggle_playback(cx);
                        }),
                    ))
                    .child(ui::transport_button(
                        "next",
                        ui::Icon::Next,
                        false,
                        cx.listener(|this: &mut Root, _: &ClickEvent, _: &mut Window, cx| {
                            this.play_next(cx);
                        }),
                    )),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(ui::icon_button(
                        "shuffle",
                        ui::Icon::Shuffle,
                        self.queue.shuffle(),
                        cx.listener(|this: &mut Root, _: &ClickEvent, _, cx| {
                            this.toggle_shuffle(cx);
                        }),
                    ))
                    .child(ui::icon_button(
                        "repeat",
                        match self.queue.repeat() {
                            RepeatMode::Off | RepeatMode::All => ui::Icon::Repeat,
                            RepeatMode::One => ui::Icon::RepeatOne,
                        },
                        self.queue.repeat() != RepeatMode::Off,
                        cx.listener(|this: &mut Root, _: &ClickEvent, _, cx| {
                            this.cycle_repeat(cx);
                        }),
                    )),
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(ui::muted_label(format_duration(Some(
                        (self.position_ms / 1000) as u32,
                    ))))
                    .child(div().flex_1().child(self.progress_slider(fraction, cx)))
                    .child(ui::muted_label(format_duration(Some(
                        (self.duration_ms / 1000) as u32,
                    )))),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .w(px(120.))
                    .child(self.volume_slider(cx)),
            )
            .child(ui::compact_button(
                "queue",
                "队列",
                self.queue_open,
                cx.listener(|this: &mut Root, _: &ClickEvent, _, cx| {
                    this.queue_open = !this.queue_open;
                    cx.notify();
                }),
            ))
    }

    fn lyrics_view(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        if self.now_playing.is_none() {
            return div().flex_1().into_any_element();
        }
        let available = window.viewport_size().width.as_f32()
            - self.sidebar_width()
            - if self.queue_open { QUEUE_WIDTH } else { 0. }
            - CONTENT_PADDING;
        let cover_width = (available * 0.35).clamp(120., 320.);
        let cover_id = self
            .playing_cover_ids
            .iter()
            .find(|id| self.covers.contains_key(*id))
            .or_else(|| self.playing_cover_ids.first())
            .map(String::as_str);
        let mut artwork = div()
            .w(px(cover_width))
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .justify_center()
            .items_center()
            .gap_3()
            .child(
                div()
                    .id("lyrics-artwork")
                    .size(px(cover_width))
                    .rounded_lg()
                    .overflow_hidden()
                    .border_1()
                    .border_color(rgb(theme::BORDER))
                    .shadow_lg()
                    .cursor_pointer()
                    .on_click(cx.listener(|this: &mut Root, _: &ClickEvent, _, cx| {
                        this.toggle_lyrics(cx);
                    }))
                    .child(self.cover(cover_id, 0)),
            );
        if self.lyrics.len() > 1 {
            artwork = artwork.child(ui::icon_button(
                "lyrics-language",
                ui::Icon::Lyrics,
                false,
                cx.listener(|this: &mut Root, _: &ClickEvent, _, cx| {
                    this.lyrics_track = (this.lyrics_track + 1) % this.lyrics.len();
                    this.lyrics_scroll = ScrollHandle::new();
                    this.lyrics_active = None;
                    this.lyrics_scrolled_to = None;
                    this.lyrics_center_height = 0.;
                    this.lyrics_scroll_generation = this.lyrics_scroll_generation.wrapping_add(1);
                    this.sync_lyrics();
                    cx.notify();
                }),
            ));
        }
        let content = div()
            .flex_1()
            .min_h_0()
            .flex()
            .px_8()
            .py_8()
            .gap_6()
            .overflow_hidden()
            .child(artwork);
        if self.lyrics_loading {
            return content.into_any_element();
        }
        if self.lyrics_error.is_some() {
            return content
                .child(div().flex_1().flex().items_center().justify_center().child(
                    ui::ghost_button(
                        "retry-lyrics",
                        "重试歌词",
                        false,
                        cx.listener(|this: &mut Root, _: &ClickEvent, _, cx| {
                            if let Some(song) = this.now_playing.clone() {
                                this.request_lyrics(&song);
                            }
                            cx.notify();
                        }),
                    ),
                ))
                .into_any_element();
        }
        let Some(lyrics) = self.lyrics.get(self.lyrics_track) else {
            return content.into_any_element();
        };
        let active = lyrics.active_range(self.position_ms);
        let height = (window.viewport_size().height.as_f32() - 76. - 64.).max(64.);
        // Space before/after the lyrics lets the first and last line also center.
        let padding = (height - 64.) / 2.;
        let mut lines = div()
            .id("lyrics-scroll")
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_y_scroll()
            .track_scroll(&self.lyrics_scroll)
            // Manual scrolling takes over from the follow animation.
            .on_scroll(cx.listener(
                |this: &mut Root, _: &ScrollEvent, _: &mut Window, _: &mut Context<Self>| {
                    this.lyrics_scroll_generation = this.lyrics_scroll_generation.wrapping_add(1);
                },
            ))
            .flex()
            .flex_col()
            .pt(px(padding))
            .pb(px(padding));
        for (index, line) in lyrics.lines.iter().enumerate() {
            let start = line.start_ms;
            lines = lines.child(
                div()
                    .id(format!("lyric-line-{index}"))
                    .flex_shrink_0()
                    .min_h(px(64.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .p_3()
                    .rounded_md()
                    .text_2xl()
                    .text_center()
                    .text_color(rgb(if active.contains(&index) {
                        theme::TEXT
                    } else {
                        theme::TEXT_MUTED
                    }))
                    .when(active.contains(&index), |style| {
                        style.font_weight(FontWeight::SEMIBOLD)
                    })
                    .when(start.is_some(), |style| {
                        style
                            .cursor_pointer()
                            .hover(|style| style.bg(rgba(0xFFFFFF0A)))
                    })
                    .child(if line.text.is_empty() {
                        " ".to_owned()
                    } else {
                        line.text.clone()
                    })
                    .on_click(cx.listener(move |this: &mut Root, _: &ClickEvent, _, cx| {
                        if let Some(start) = start
                            && this.duration_ms > 0
                        {
                            this.seek_fraction(start as f32 / this.duration_ms as f32, cx);
                        }
                    })),
            );
        }
        content.child(lines).into_any_element()
    }

    fn queue_row(&self, index: usize, song: &Song, cx: &mut Context<Self>) -> impl IntoElement {
        let current = self.queue.current_index() == Some(index);
        let drag = QueueDrag {
            index,
            title: song.title.clone(),
            position: Point::default(),
        };
        div()
            .id(format!("queue-{index}"))
            .h(px(QUEUE_ROW_HEIGHT))
            .w_full()
            .flex()
            .flex_col()
            .gap_1()
            .px_2()
            .py_2()
            .rounded_md()
            .cursor_pointer()
            .when(current, |row| row.bg(rgba(0xFFFFFF14)))
            .hover(|style| style.bg(rgba(0xFFFFFF0A)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(ui::muted_label(if current {
                        "▶".to_owned()
                    } else {
                        format!("{}", index + 1)
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .truncate()
                            .child(song.title.clone()),
                    )
                    .child(ui::muted_label(format_duration(song.duration))),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(div().flex_1())
                    .when(index > 0, |row| {
                        row.child(ui::ghost_button(
                            format!("queue-up-{index}"),
                            "↑",
                            false,
                            cx.listener(move |this: &mut Root, _: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                this.move_queue_song(index, index - 1, cx);
                            }),
                        ))
                    })
                    .when(index + 1 < self.queue.len(), |row| {
                        row.child(ui::ghost_button(
                            format!("queue-down-{index}"),
                            "↓",
                            false,
                            cx.listener(move |this: &mut Root, _: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                this.move_queue_song(index, index + 1, cx);
                            }),
                        ))
                    })
                    .child(ui::ghost_button(
                        format!("queue-remove-{index}"),
                        "移除",
                        false,
                        cx.listener(move |this: &mut Root, _: &ClickEvent, _, cx| {
                            cx.stop_propagation();
                            this.remove_queue_song(index, cx);
                        }),
                    )),
            )
            .on_click(cx.listener(move |this: &mut Root, _: &ClickEvent, _, cx| {
                this.select_queue_song(index, cx);
            }))
            .on_drag(drag, |info: &QueueDrag, position, _, cx| {
                let mut preview = info.clone();
                preview.position = position;
                cx.new(|_| preview)
            })
            .on_drop(
                cx.listener(move |this: &mut Root, info: &QueueDrag, _, cx| {
                    this.move_queue_song(info.index, index, cx);
                }),
            )
    }

    fn queue_drawer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let list = if self.queue.is_empty() {
            div()
                .flex_1()
                .child(placeholder("队列为空"))
                .into_any_element()
        } else {
            uniform_list(
                "queue-scroll",
                self.queue.len(),
                cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                    range
                        .filter_map(|index| {
                            this.queue
                                .songs()
                                .get(index)
                                .map(|song| this.queue_row(index, song, cx).into_any_element())
                        })
                        .collect::<Vec<_>>()
                }),
            )
            .flex_1()
            .min_h_0()
            .w_full()
            .track_scroll(&self.queue_scroll)
            .into_any_element()
        };
        div()
            .flex()
            .flex_col()
            .gap_3()
            .w(px(QUEUE_WIDTH))
            .h_full()
            .min_h_0()
            .flex_shrink_0()
            .p_4()
            .bg(rgba(0x0E0E12E6))
            .border_l_1()
            .border_color(rgb(theme::BORDER))
            .child(
                div()
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_sm()
                            .child(format!("播放队列 · {} 首", self.queue.len())),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .flex_shrink_0()
                    .child(ui::ghost_button(
                        "queue-locate-current",
                        "定位当前",
                        false,
                        cx.listener(|this: &mut Root, _: &ClickEvent, _, cx| {
                            if let Some(index) = this.queue.current_index() {
                                this.queue_scroll
                                    .scroll_to_item(index, ScrollStrategy::Nearest);
                                cx.notify();
                            }
                        }),
                    ))
                    .child(ui::ghost_button(
                        "queue-clear-upcoming",
                        "清空待播",
                        false,
                        cx.listener(|this: &mut Root, _: &ClickEvent, _, cx| {
                            this.clear_upcoming(cx);
                        }),
                    )),
            )
            .child(list)
    }
}

impl Render for Root {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.follow_lyrics(cx);
        if window.focused(cx).is_none() {
            window.focus(&self.focus_handle, cx);
        }
        if self.smtc.is_none() {
            self.smtc = Some(smtc::Smtc::attach(window));
        }
        self.sync_smtc_playback();
        #[cfg(windows)]
        self.sync_smtc_artwork(cx);
        if !self.smoke_reported
            && let Some(path) = std::env::var_os("MELOS_SMOKE_REPORT")
        {
            self.smoke_reported = true;
            let controls = self.smtc.as_ref().expect("SMTC initialized");
            #[cfg(windows)]
            let published = controls
                .publish("Melos 测试曲目", "测试艺人", 1000, 60_000)
                .is_ok();
            #[cfg(not(windows))]
            let published = false;
            controls.set_status(Some(false));
            let report = format!(
                "{{\"smtc_available\":{},\"metadata_published\":{},\"audio_device_available\":{}}}",
                controls.available(),
                published,
                self.engine.is_some()
            );
            // Hidden Windows processes do not present frames. Record view
            // initialization so diagnostics also work in unattended checks.
            std::fs::write(path, report).expect("write smoke report");
            let seconds = std::env::var("MELOS_SMOKE_SECONDS")
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(1);
            cx.spawn(async move |_, cx| {
                cx.background_executor()
                    .timer(Duration::from_secs(seconds))
                    .await;
                cx.update(|cx| cx.quit());
            })
            .detach();
        }
        let content = if self.connected {
            self.shell(window, cx).into_any_element()
        } else {
            self.connect_view(cx).into_any_element()
        };
        div()
            .id("app-root")
            .track_focus(&self.focus_handle)
            .size_full()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    let in_input = if this.connected {
                        (this.view == View::Search
                            && this.search.read(cx).contains_position(event.position))
                            || (this.view == View::Playlists
                                && this.playlist_editor.is_some()
                                && this
                                    .playlist_name
                                    .read(cx)
                                    .contains_position(event.position))
                    } else {
                        [&this.server, &this.username, &this.password]
                            .iter()
                            .any(|input| input.read(cx).contains_position(event.position))
                    };
                    if !in_input {
                        window.focus(&this.focus_handle, cx);
                    }
                }),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                let inputs = [
                    &this.server,
                    &this.username,
                    &this.password,
                    &this.search,
                    &this.playlist_name,
                ];
                let editing = inputs
                    .iter()
                    .any(|input| input.read(cx).focus_handle(cx).is_focused(window));
                let composing = inputs.iter().any(|input| {
                    input.read(cx).focus_handle(cx).is_focused(window)
                        && input.read(cx).is_composing()
                });
                if this.connected
                    && let Some(shortcut) =
                        crate::shortcuts::resolve(&event.keystroke, editing, composing)
                {
                    use crate::shortcuts::Shortcut;
                    match shortcut {
                        Shortcut::TogglePlayback => this.toggle_playback(cx),
                        Shortcut::Next => this.play_next(cx),
                        Shortcut::Previous => this.play_previous(cx),
                        Shortcut::Seek(offset) => {
                            this.sync_engine(cx);
                            if this.duration_ms > 0 {
                                let target = this
                                    .position_ms
                                    .saturating_add_signed(offset)
                                    .min(this.duration_ms);
                                this.seek_fraction(target as f32 / this.duration_ms as f32, cx);
                            }
                        }
                        Shortcut::Volume(offset) => {
                            this.set_volume_fraction(this.volume + f32::from(offset) / 100.0, cx);
                            this.save_playback_settings(cx);
                        }
                        Shortcut::Shuffle => this.toggle_shuffle(cx),
                        Shortcut::Repeat => this.cycle_repeat(cx),
                        Shortcut::Queue => {
                            this.queue_open = !this.queue_open;
                            cx.notify();
                        }
                        Shortcut::Search => {
                            this.select(View::Search, cx);
                            let handle = this.search.read(cx).focus_handle(cx);
                            window.focus(&handle, cx);
                        }
                    }
                    cx.stop_propagation();
                    return;
                }
                if event.keystroke.key != "enter" || event.prefer_character_input {
                    return;
                }
                let focused = |input: &Entity<TextInput>| {
                    input.read(cx).focus_handle(cx).is_focused(window)
                        && !input.read(cx).is_composing()
                };
                if this.connected && this.view == View::Playlists && focused(&this.playlist_name) {
                    this.submit_playlist_name(cx);
                    cx.stop_propagation();
                } else if this.connected && focused(&this.search) {
                    this.run_search(cx);
                    cx.stop_propagation();
                } else if !this.connected
                    && !this.connecting
                    && [
                        this.server.clone(),
                        this.username.clone(),
                        this.password.clone(),
                    ]
                    .iter()
                    .any(focused)
                {
                    this.connect(cx);
                    cx.stop_propagation();
                }
            }))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if event.dragging() {
                    this.move_slider(event.position, cx);
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, _, cx| {
                    this.end_slider(event.position, cx);
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, _, cx| {
                    this.end_slider(event.position, cx);
                }),
            )
            .child(content)
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        // The worker exits when its request channel closes.
    }
}

// ------------------------------------------------------------------- helpers

fn field(label: &'static str, input: &Entity<TextInput>) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_xs()
                .text_color(rgb(theme::TEXT_MUTED))
                .child(label),
        )
        .child(input.clone())
}

fn back_button(cx: &mut Context<Root>) -> impl IntoElement {
    ui::ghost_button(
        "back",
        "← 返回",
        false,
        cx.listener(|this: &mut Root, _: &ClickEvent, _: &mut Window, cx| {
            if this.detail.take().is_none() {
                this.artist_view = None;
            }
            cx.notify();
        }),
    )
}

fn section(title: &'static str, content: AnyElement) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .text_sm()
                .text_color(rgb(theme::TEXT_MUTED))
                .child(title),
        )
        .child(content)
}

fn song_row(
    index: usize,
    song: &Song,
    source: SongSource,
    cx: &mut Context<Root>,
) -> impl IntoElement {
    let next_song = song.clone();
    let append_song = song.clone();
    div()
        .id(format!("song-{}-{}", song.id, index))
        .flex()
        .flex_col()
        .gap_1()
        .px_3()
        .py_2()
        .rounded_md()
        .cursor_pointer()
        .hover(|style| style.bg(rgba(0xFFFFFF0A)))
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .w(px(48.))
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(rgb(theme::TEXT_FAINT))
                        .child(format!("{}", song.track.unwrap_or(index as u32 + 1))),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_sm()
                        .truncate()
                        .child(song.title.clone()),
                )
                .child(
                    div()
                        .max_w(px(130.))
                        .min_w_0()
                        .text_xs()
                        .text_color(rgb(theme::TEXT_MUTED))
                        .truncate()
                        .child(song.artist.clone().unwrap_or_default()),
                )
                .child(ui::muted_label(format_duration(song.duration))),
        )
        .child(
            div()
                .flex()
                .justify_end()
                .gap_1()
                .child(ui::ghost_button(
                    format!("song-next-{index}"),
                    "下一首播放",
                    false,
                    cx.listener(move |this: &mut Root, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        this.enqueue(vec![next_song.clone()], true, cx);
                    }),
                ))
                .child(ui::ghost_button(
                    format!("song-append-{index}"),
                    "加入队列",
                    false,
                    cx.listener(move |this: &mut Root, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        this.enqueue(vec![append_song.clone()], false, cx);
                    }),
                )),
        )
        .on_click(
            cx.listener(move |this: &mut Root, _: &ClickEvent, _: &mut Window, cx| {
                this.play_source(source, index, cx);
            }),
        )
}

fn format_duration(seconds: Option<u32>) -> String {
    let seconds = seconds.unwrap_or(0);
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

fn paint_track(bounds: Bounds<Pixels>, track: f32, fraction: f32, window: &mut Window) {
    let height = px(track);
    let y = bounds.origin.y + (bounds.size.height - height) * 0.5;
    window.paint_quad(quad(
        Bounds {
            origin: point(bounds.origin.x, y),
            size: size(bounds.size.width, height),
        },
        height * 0.5,
        rgb(theme::BORDER),
        px(0.),
        transparent_black(),
        Default::default(),
    ));
    window.paint_quad(quad(
        Bounds {
            origin: point(bounds.origin.x, y),
            size: size(bounds.size.width * fraction.clamp(0.0, 1.0), height),
        },
        height * 0.5,
        rgb(theme::TEXT),
        px(0.),
        transparent_black(),
        Default::default(),
    ));
}

fn stream_spec(client: &Client, song: &Song, quality: StreamQuality) -> StreamSpec {
    StreamSpec {
        url: client.stream_url(song, quality),
        hint: song.suffix.clone(),
        duration_ms: song.duration.map(|seconds| u64::from(seconds) * 1000),
    }
}

fn guess_image_format(bytes: &[u8]) -> Option<ImageFormat> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some(ImageFormat::Png)
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(ImageFormat::Jpeg)
    } else if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some(ImageFormat::Webp)
    } else if bytes.starts_with(b"GIF8") {
        Some(ImageFormat::Gif)
    } else if bytes.starts_with(b"BM") {
        Some(ImageFormat::Bmp)
    } else {
        None
    }
}

fn placeholder(message: &'static str) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .justify_center()
        .h(px(180.))
        .rounded_md()
        .bg(rgba(0xFFFFFF08))
        .text_sm()
        .text_color(rgb(theme::TEXT_MUTED))
        .child(message)
}

/// Picks a column count from the window width so the grid always fills the
/// content area instead of leaving a strip of empty space on the right.
fn grid_columns(window: &Window, queue_open: bool, sidebar_width: f32) -> usize {
    let width = window.viewport_size().width.as_f32();
    let reserved = sidebar_width + if queue_open { QUEUE_WIDTH } else { 0.0 } + CONTENT_PADDING;
    let available = (width - reserved).max(CARD_MIN);
    (((available + CARD_GAP) / (CARD_MIN + CARD_GAP)).floor() as usize).clamp(1, 10)
}
