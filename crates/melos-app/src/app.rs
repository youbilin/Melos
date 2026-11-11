//! Application shell and the M4 data views.
//!
//! Playback is still driven by placeholder state; wiring the player engine and
//! the queue happens in M5.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::prelude::*;
use gpui::*;
use melos_api::{
    Album, AlbumWithSongs, Artist, ArtistWithAlbums, Client, Credentials, SearchResult,
    ServerInfo, Song, StreamQuality,
};
use melos_core::Settings;
use melos_player::{Engine, StreamSpec};

use crate::api::{ApiWorker, Request, Response};
use crate::smtc;
use crate::theme;
use crate::ui::{self, input::TextInput};

const SIDEBAR_WIDTH: f32 = 216.0;
const QUEUE_WIDTH: f32 = 288.0;
const CONTENT_PADDING: f32 = 64.0;
const CARD_MIN: f32 = 160.0;
const CARD_GAP: f32 = 24.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Home,
    Artists,
    Albums,
    Playlists,
    Search,
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
            Self::Settings => "设置",
        }
    }

    fn subtitle(self) -> &'static str {
        match self {
            Self::Home => "最近添加与常听",
            Self::Artists => "按艺人浏览",
            Self::Albums => "全部专辑",
            Self::Playlists => "你的歌单",
            Self::Search => "搜索艺人、专辑与歌曲",
            Self::Settings => "连接与播放偏好",
        }
    }
}

pub struct Root {
    api: ApiWorker,
    view: View,
    playing: bool,
    queue_open: bool,
    volume: f32,

    server: Entity<TextInput>,
    username: Entity<TextInput>,
    password: Entity<TextInput>,
    search: Entity<TextInput>,

    connected: bool,
    connecting: bool,
    error: Option<String>,
    server_info: Option<ServerInfo>,

    albums: Vec<Album>,
    artists: Vec<Artist>,
    detail: Option<AlbumWithSongs>,
    artist_view: Option<ArtistWithAlbums>,
    search_result: Option<SearchResult>,
    covers: HashMap<String, Arc<Image>>,
    cover_requests: HashSet<String>,
    settings: Entity<Settings>,
    smtc: Option<smtc::Smtc>,

    client: Option<Client>,
    engine: Option<Engine>,
    queue: Vec<Song>,
    queue_index: usize,
    now_playing: Option<Song>,
    position_ms: u64,
    duration_ms: u64,
    last_track_index: u64,
    scrobbled_playing: bool,
    scrobbled_submitted: bool,
}

impl Root {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let settings = Settings::load();
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
        let saved_settings = cx.new(|_| settings);

        let root = Self {
            api: ApiWorker::new(),
            view: View::Home,
            playing: false,
            queue_open: false,
            volume: 0.8,
            server,
            username,
            password,
            search,
            connected: false,
            connecting: false,
            error: None,
            server_info: None,
            albums: Vec::new(),
            artists: Vec::new(),
            detail: None,
            artist_view: None,
            search_result: None,
            covers: HashMap::new(),
            cover_requests: HashSet::new(),
            settings: saved_settings,
            smtc: None,
            client: None,
            engine: Engine::new().ok(),
            queue: Vec::new(),
            queue_index: 0,
            now_playing: None,
            position_ms: 0,
            duration_ms: 0,
            last_track_index: 0,
            scrobbled_playing: false,
            scrobbled_submitted: false,
        };

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
                    let animated = this.sync_engine();
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
            Response::Connected(info) => {
                self.connected = true;
                self.connecting = false;
                self.error = None;
                self.server_info = Some(*info);
                self.save_settings(cx);
                self.password
                    .update(cx, |input, cx| input.set_content("", cx));
                self.api.send(Request::Albums {
                    size: 200,
                    offset: 0,
                });
                self.api.send(Request::Artists);
            }
            Response::Albums(albums) => {
                self.request_covers(albums.iter().filter_map(|album| album.cover_art.as_deref()));
                self.albums = albums;
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
            }
            Response::Cover { id, bytes } => {
                if let Some(format) = guess_image_format(&bytes) {
                    self.covers
                        .insert(id, Arc::new(Image::from_bytes(format, bytes)));
                }
            }
            Response::CoverFailed { id } => {
                self.cover_requests.remove(&id);
            }
            Response::Scrobbled => {}
            Response::Starred => {}
            Response::Error(error) => {
                self.error = Some(error);
                self.connecting = false;
            }
        }
    }

    /// Requests covers for the first screenful, once per cover id.
    fn request_covers<'a>(&mut self, ids: impl Iterator<Item = &'a str>) {
        for id in ids.take(80) {
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

    /// Persists the current connection fields for the next launch.
    fn save_settings(&mut self, cx: &mut Context<Self>) {
        let server = self.server.read(cx).content().trim().to_owned();
        let username = self.username.read(cx).content().trim().to_owned();
        let password = self.password.read(cx).content().to_owned();
        let settings = self.settings.clone();

        let _ = settings.update(cx, |settings, _| {
            settings.server = server;
            settings.username = username;
            settings.set_password(&password);
            settings.save().ok();
        });
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

        match Client::new(&server, Credentials::new(username.clone(), password.clone())) {
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

    fn play_songs(&mut self, songs: Vec<Song>, start: usize, cx: &mut Context<Self>) {
        if songs.is_empty() {
            return;
        }
        self.queue = songs;
        self.queue_index = start.min(self.queue.len() - 1);
        self.start_current(cx);
    }

    fn start_current(&mut self, cx: &mut Context<Self>) {
        let Some(client) = self.client.as_ref() else {
            return;
        };
        let Some(song) = self.queue.get(self.queue_index).cloned() else {
            return;
        };

        let spec = stream_spec(client, &song);
        let next = self
            .queue
            .get(self.queue_index + 1)
            .map(|song| stream_spec(client, song));
        let duration = spec.duration_ms.unwrap_or(0);

        let Some(engine) = self.engine.as_ref() else {
            self.error = Some("没有可用的音频输出设备".into());
            cx.notify();
            return;
        };

        engine.play(spec, next);

        self.now_playing = Some(song);
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
        if self.queue_index + 1 < self.queue.len() {
            self.queue_index += 1;
            self.start_current(cx);
        }
    }

    fn play_previous(&mut self, cx: &mut Context<Self>) {
        if self.queue_index > 0 {
            self.queue_index -= 1;
            self.start_current(cx);
        }
    }

    fn toggle_star(&mut self, cx: &mut Context<Self>) {
        let Some(album) = self.detail.as_ref().map(|detail| &detail.album) else {
            return;
        };
        let starred = album.starred.is_none();
        let id = album.id.clone();

        if let Some(detail) = self.detail.as_mut() {
            detail.album.starred = if starred {
                Some("now".to_owned())
            } else {
                None
            };
        }

        self.api.send(Request::Star { id, starred });
        cx.notify();
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
            .on_click(cx.listener(|this: &mut Root, _: &ClickEvent, _: &mut Window, cx| {
                this.toggle_star(cx);
            }))
    }

    fn seek_fraction(&mut self, fraction: f32, cx: &mut Context<Self>) {
        if self.duration_ms == 0 {
            return;
        }
        let Some(engine) = self.engine.as_ref() else {
            return;
        };

        let target = (f64::from(fraction.clamp(0.0, 1.0)) * self.duration_ms as f64) as u64;
        engine.seek(target);
        self.position_ms = target;

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

    /// Click-to-seek progress bar.
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
                        let fraction =
                            ((event.position.x - b.origin.x) / b.size.width).clamp(0.0, 1.0);
                        this.seek_fraction(fraction, cx);
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

    /// Click-to-set volume bar.
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
                        let fraction =
                            ((event.position.x - b.origin.x) / b.size.width).clamp(0.0, 1.0);
                        this.set_volume_fraction(fraction, cx);
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
    fn sync_engine(&mut self) -> bool {
        let Some(engine) = self.engine.as_ref() else {
            return false;
        };
        let snapshot = engine.snapshot();
        let mut changed = false;

        if snapshot.position_ms != self.position_ms {
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
            self.queue_index =
                (self.queue_index + advance).min(self.queue.len().saturating_sub(1));
            self.now_playing = self.queue.get(self.queue_index).cloned();
            self.position_ms = 0;
            self.duration_ms = self
                .now_playing
                .as_ref()
                .and_then(|song| song.duration)
                .map(|seconds| u64::from(seconds) * 1000)
                .unwrap_or(0);
            self.scrobbled_playing = false;
            self.scrobbled_submitted = false;

            if let (Some(client), Some(engine)) = (self.client.as_ref(), self.engine.as_ref()) {
                if let Some(next) = self.queue.get(self.queue_index + 1) {
                    engine.set_next(stream_spec(client, next));
                }
            }
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

        if snapshot.ended && self.playing {
            self.playing = false;
            changed = true;
        }

        if changed
            && let Some(smtc) = self.smtc.as_ref()
        {
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

        changed
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
            if self.connecting { "连接中…" } else { "连接" },
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
                    .child(ui::muted_label("Navidrome 或其他 OpenSubsonic 服务"))
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
                    .child(self.header())
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
            nav = nav.child(ui::ghost_button(
                view.label(),
                view.label(),
                self.view == view,
                cx.listener(move |this: &mut Root, _: &ClickEvent, _: &mut Window, cx| {
                    this.select(view, cx);
                }),
            ));
        }

        div()
            .flex()
            .flex_col()
            .w(px(SIDEBAR_WIDTH))
            .h_full()
            .flex_shrink_0()
            .px_3()
            .pt_6()
            .pb_4()
            .gap_6()
            .bg(rgba(0x0A0A0ACC))
            .border_r_1()
            .border_color(rgb(theme::BORDER))
            .child(nav)
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(ui::ghost_button(
                        "settings",
                        View::Settings.label(),
                        self.view == View::Settings,
                        cx.listener(|this: &mut Root, _: &ClickEvent, _: &mut Window, cx| {
                            this.select(View::Settings, cx);
                        }),
                    ))
                    .child(ui::muted_label("melos v0.1.0")),
            )
    }

    fn header(&self) -> impl IntoElement {
        let status = match &self.server_info {
            Some(info) => format!(
                "{} {}",
                if info.server_type.is_empty() {
                    "subsonic"
                } else {
                    &info.server_type
                },
                info.server_version.as_deref().unwrap_or("")
            ),
            None => "未连接".to_owned(),
        };

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
                    .child(div().text_3xl().child(self.view.label()))
                    .child(ui::muted_label(self.view.subtitle())),
            )
            .child(ui::muted_label(status))
    }

    fn body(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content: AnyElement = if let Some(detail) = &self.detail {
            self.detail_view(detail, cx).into_any_element()
        } else if let Some(artist) = &self.artist_view {
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
                View::Playlists => placeholder("歌单视图将在 v1.1 接入").into_any_element(),
                View::Search => self.search_view(window, cx).into_any_element(),
                View::Settings => placeholder("设置页将在 M5 接入").into_any_element(),
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
    }

    // ------------------------------------------------------------------ albums

    fn album_grid(
        &self,
        albums: &[Album],
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let columns = grid_columns(window, self.queue_open);

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
            .child(self.cover(album.cover_art.as_deref(), index))
            .child(div().text_sm().child(album.name.clone()))
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(theme::TEXT_MUTED))
                    .child(artist),
            )
            .on_click(cx.listener(move |this: &mut Root, _: &ClickEvent, _: &mut Window, cx| {
                this.open_album(id.clone(), cx);
            }))
    }

    fn detail_view(&self, album: &AlbumWithSongs, cx: &mut Context<Self>) -> impl IntoElement {
        let songs_arc = Arc::new(album.song.clone());
        let mut songs = div().flex().flex_col().gap_1();

        for (index, song) in album.song.iter().enumerate() {
            songs = songs.child(song_row(index, song, songs_arc.clone(), cx));
        }

        let play_all = {
            let songs = songs_arc.clone();
            ui::primary_button(
                "播放全部",
                true,
                cx.listener(move |this: &mut Root, _: &ClickEvent, _: &mut Window, cx| {
                    this.play_songs((*songs).clone(), 0, cx);
                }),
            )
        };

        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(back_button(cx))
            .child(
                div()
                    .flex()
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
                    .child(
                        div().w(px(88.)).child(ui::primary_button(
                            "搜索",
                            true,
                            cx.listener(|this: &mut Root, _: &ClickEvent, _: &mut Window, cx| {
                                this.run_search(cx);
                            }),
                        )),
                    ),
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
            sections = sections.child(section(
                "专辑",
                self.album_grid(&result.album, window, cx),
            ));
        }

        if !result.song.is_empty() {
            let songs_arc = Arc::new(result.song.clone());
            let mut songs = div().flex().flex_col().gap_1();
            for (index, song) in result.song.iter().enumerate() {
                songs = songs.child(song_row(index, song, songs_arc.clone(), cx));
            }
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
            .unwrap_or_else(|| "选择一首歌开始".into());
        let cover_id = self
            .now_playing
            .as_ref()
            .and_then(|song| song.cover_art.as_deref());
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
                    .size(px(48.))
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
                    .flex_1()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(ui::muted_label(format_duration(Some(
                        (self.position_ms / 1000) as u32,
                    ))))
                    .child(
                        div().flex_1().child(self.progress_slider(fraction, cx)),
                    )
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
                    .child(ui::muted_label("VOL"))
                    .child(self.volume_slider(cx)),
            )
            .child(ui::ghost_button(
                "queue",
                "队列",
                self.queue_open,
                cx.listener(|this: &mut Root, _: &ClickEvent, _: &mut Window, cx| {
                    this.queue_open = !this.queue_open;
                    cx.notify();
                }),
            ))
    }

    fn queue_drawer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut list = div().flex().flex_col().gap_1();

        if self.queue.is_empty() {
            list = list.child(placeholder("队列为空"));
        } else {
            for (index, song) in self.queue.iter().enumerate() {
                let songs = self.queue.clone();
                list = list.child(
                    div()
                        .id(format!("queue-{}", index))
                        .flex()
                        .items_center()
                        .justify_between()
                        .px_2()
                        .py_2()
                        .rounded_md()
                        .cursor_pointer()
                        .hover(|style| style.bg(rgba(0xFFFFFF0A)))
                        .child(div().text_xs().child(song.title.clone()))
                        .child(ui::muted_label(format_duration(song.duration)))
                        .on_click(cx.listener(
                            move |this: &mut Root, _: &ClickEvent, _: &mut Window, cx| {
                                this.play_songs(songs.clone(), index, cx);
                            },
                        )),
                );
            }
        }

        div()
            .flex()
            .flex_col()
            .gap_3()
            .w(px(QUEUE_WIDTH))
            .h_full()
            .flex_shrink_0()
            .p_4()
            .bg(rgba(0x0E0E12E6))
            .border_l_1()
            .border_color(rgb(theme::BORDER))
            .child(div().text_sm().child("播放队列"))
            .child(list)
    }
}

impl Render for Root {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.smtc.is_none() {
            self.smtc = Some(smtc::Smtc::attach(window));
        }

        if self.connected {
            self.shell(window, cx).into_any_element()
        } else {
            self.connect_view(cx).into_any_element()
        }
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
            this.detail = None;
            this.artist_view = None;
            cx.notify();
        }),
    )
}

fn section(title: &'static str, content: AnyElement) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(div().text_sm().text_color(rgb(theme::TEXT_MUTED)).child(title))
        .child(content)
}

    fn song_row(
    index: usize,
    song: &Song,
    songs: Arc<Vec<Song>>,
    cx: &mut Context<Root>,
) -> impl IntoElement {
        div()
            .id(format!("song-{}-{}", song.id, index))
            .flex()
            .items_center()
            .gap_3()
            .px_3()
            .py_2()
            .rounded_md()
            .cursor_pointer()
            .hover(|style| style.bg(rgba(0xFFFFFF0A)))
            .child(
                div()
                    .w(px(28.))
                    .text_xs()
                    .text_color(rgb(theme::TEXT_FAINT))
                    .child(format!("{}", song.track.unwrap_or(index as u32 + 1))),
            )
            .child(div().flex_1().text_sm().child(song.title.clone()))
            .child(ui::muted_label(song.artist.clone().unwrap_or_default()))
            .child(ui::muted_label(format_duration(song.duration)))
            .on_click(
                cx.listener(move |this: &mut Root, _: &ClickEvent, _: &mut Window, cx| {
                    this.play_songs((*songs).clone(), index, cx);
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

fn stream_spec(client: &Client, song: &Song) -> StreamSpec {
    StreamSpec {
        url: client.stream_url(song, StreamQuality::Original),
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
fn grid_columns(window: &Window, queue_open: bool) -> usize {
    let width = window.viewport_size().width.as_f32();
    let reserved =
        SIDEBAR_WIDTH + if queue_open { QUEUE_WIDTH } else { 0.0 } + CONTENT_PADDING;
    let available = (width - reserved).max(CARD_MIN);
    (((available + CARD_GAP) / (CARD_MIN + CARD_GAP)).floor() as usize).clamp(1, 10)
}
