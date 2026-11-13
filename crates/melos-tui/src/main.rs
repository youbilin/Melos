//! Terminal frontend using the same API, queue and audio engine as the desktop.
mod worker;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use melos_api::{Album, Client, Credentials, Playlist, Song, StreamQuality};
use melos_core::lyrics::Lyrics;
use melos_core::{Queue, RepeatMode, Settings};
use melos_player::{Engine, EngineSnapshot, StreamSpec};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph, Wrap};
use std::io::{self, IsTerminal};
use std::time::Duration;
use worker::{Data, Job, Worker};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Page {
    Albums,
    Songs,
    Playlists,
    Queue,
    Lyrics,
}

struct App {
    client: Client,
    worker: Worker,
    engine: Option<Engine>,
    settings: Settings,
    dirty: bool,
    queue: Queue,
    snapshot: EngineSnapshot,
    boundary: u64,
    started: bool,
    page: Page,
    history: Vec<Page>,
    albums: Vec<Album>,
    offset: u32,
    more: bool,
    playlists: Vec<Playlist>,
    songs: Vec<Song>,
    songs_library: bool,
    songs_offset: u32,
    songs_more: bool,
    title: String,
    selection: ListState,
    query: Option<String>,
    status: String,
    browse_ticket: u64,
    lyrics_ticket: u64,
    lyrics_song: Option<String>,
    lyrics: Vec<Lyrics>,
    lyrics_status: Option<String>,
    lyrics_track: usize,
    lyrics_selection: ListState,
    scrobbled: bool,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!(
            "Melos terminal player\nUsage: melos-tui [--help|--version]\nUses desktop connection settings; MELOS_CONFIG_DIR selects another configuration.\n1 albums · 2 playlists · 3 queue · 4 lyrics · 5 all songs · / search · Enter open/play\nSpace pause · n/p next/previous · Left/Right seek · +/- volume · s shuffle · r repeat\na append song · i play next · d remove queue entry · J/K reorder · D clear upcoming\nL load more albums · t switch lyrics · e reconnect audio (paused) · Esc cancel/back · q or Ctrl+C quit"
        );
        return Ok(());
    }
    if args.iter().any(|arg| arg == "--version" || arg == "-V") {
        println!("melos-tui {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if !args.is_empty() {
        return Err("Unknown argument; use --help".into());
    }
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err("Run melos-tui in an interactive terminal.".into());
    }
    let settings = Settings::load();
    let password = settings
        .password()
        .ok_or("Connect and save credentials in the desktop app first.")?;
    if settings.server.is_empty() || settings.username.is_empty() {
        return Err(
            "Connect in the desktop app first, or select settings with MELOS_CONFIG_DIR.".into(),
        );
    }
    let client = Client::new(
        &settings.server,
        Credentials::new(&settings.username, password),
    )?;
    let mut app = App::new(client, settings, true);
    app.request(Job::Connect);
    // Ratatui restores raw mode, alternate screen and cursor on errors;
    // its panic hook also restores the terminal before reporting a panic.
    let result = ratatui::run(|terminal| -> io::Result<()> {
        loop {
            app.tick();
            terminal.draw(|frame| app.render(frame))?;
            if event::poll(Duration::from_millis(60))?
                && let Event::Key(key) = event::read()?
                && key.kind != KeyEventKind::Release
                && app.key(key)
            {
                break;
            }
        }
        Ok(())
    });
    if let Some(engine) = app.engine.as_ref() {
        engine.stop();
    }
    if app.dirty {
        app.settings.save()?;
    }
    result?;
    Ok(())
}

impl App {
    fn new(client: Client, mut settings: Settings, audio: bool) -> Self {
        settings.volume = if settings.volume.is_finite() {
            settings.volume.clamp(0., 1.)
        } else {
            0.8
        };
        let mut queue = Queue::new();
        queue.set_shuffle(settings.shuffle);
        queue.set_repeat(settings.repeat);
        let engine = if audio { Engine::new().ok() } else { None };
        if let Some(engine) = engine.as_ref() {
            engine.set_volume(settings.volume);
        }
        Self {
            worker: Worker::new(client.clone()),
            client,
            settings,
            engine,
            dirty: false,
            queue,
            snapshot: EngineSnapshot::default(),
            boundary: 0,
            started: false,
            page: Page::Albums,
            history: Vec::new(),
            albums: Vec::new(),
            offset: 0,
            more: true,
            playlists: Vec::new(),
            songs_library: false,
            songs_offset: 0,
            songs_more: false,
            songs: Vec::new(),
            title: String::new(),
            selection: ListState::default().with_selected(Some(0)),
            query: None,
            status: "Connecting…".into(),
            browse_ticket: 0,
            lyrics_ticket: 0,
            lyrics_song: None,
            lyrics: Vec::new(),
            lyrics_status: None,
            lyrics_track: 0,
            lyrics_selection: ListState::default(),
            scrobbled: false,
        }
    }
    fn request(&mut self, job: Job) {
        if matches!(job, Job::Lyrics(_)) {
            self.lyrics_ticket = self.worker.send(job);
            self.lyrics_status = Some("Loading lyrics…".into());
        } else {
            self.browse_ticket = self.worker.send(job);
            self.status = "Loading…".into();
        }
    }
    fn page(&mut self, page: Page) {
        if self.page != page {
            self.history.push(self.page);
        }
        self.show(page);
    }
    fn show(&mut self, page: Page) {
        self.page = page;
        self.selection.select(Some(0));
        self.browse_ticket = 0;
        self.status.clear();
        if page == Page::Playlists {
            self.request(Job::Playlists);
        }
        if page == Page::Albums && self.albums.is_empty() {
            self.request(Job::Albums(0));
        }
    }
    /// Esc goes one level up: playlist/album songs return to their list.
    fn back(&mut self) {
        let page = self.history.pop().unwrap_or(Page::Albums);
        self.show(page);
    }
    /// Full library song list, paged with L.
    fn library(&mut self) {
        if self.page != Page::Songs {
            self.history.push(self.page);
        }
        self.show(Page::Songs);
        self.songs.clear();
        self.songs_library = true;
        self.songs_offset = 0;
        self.songs_more = true;
        self.title = "全部歌曲".into();
        self.request(Job::Library(0));
    }
    fn tick(&mut self) {
        for response in self.worker.drain() {
            if response.ticket
                != if response.lyrics {
                    self.lyrics_ticket
                } else {
                    self.browse_ticket
                }
            {
                continue;
            }
            match response.data {
                Err(error) => {
                    if response.lyrics {
                        self.lyrics_status = Some(error);
                    } else {
                        self.status = error;
                    }
                }
                Ok(Data::Connected) => self.request(Job::Albums(0)),
                Ok(Data::Albums {
                    albums,
                    offset,
                    received,
                }) => {
                    if offset != self.offset {
                        continue;
                    }
                    self.more = received == 80;
                    self.offset += received as u32;
                    for album in albums {
                        if !self.albums.iter().any(|item| item.id == album.id) {
                            self.albums.push(album);
                        }
                    }
                    self.status = format!("{} albums loaded · L load more", self.albums.len());
                }
                Ok(Data::Library {
                    songs,
                    offset,
                    received,
                }) => {
                    if !self.songs_library {
                        continue;
                    }
                    if offset == 0 {
                        self.songs = songs;
                    } else {
                        for song in songs {
                            if !self.songs.iter().any(|item| item.id == song.id) {
                                self.songs.push(song);
                            }
                        }
                    }
                    self.songs_offset = offset + received as u32;
                    self.songs_more = received == 200;
                    self.status = format!("{} songs loaded · L load more", self.songs.len());
                }
                Ok(Data::Songs { songs, title }) => {
                    self.songs = songs;
                    self.songs_library = false;
                    self.songs_more = false;
                    self.title = title;
                    if self.page != Page::Songs {
                        self.history.push(self.page);
                    }
                    self.page = Page::Songs;
                    self.selection.select(Some(0));
                    self.status.clear();
                }
                Ok(Data::Playlists(playlists)) => {
                    self.playlists = playlists;
                    self.status.clear();
                }
                Ok(Data::Lyrics(lyrics)) => {
                    self.lyrics = lyrics;
                    self.lyrics_track = 0;
                    self.lyrics_status = None;
                }
            }
        }
        self.sync();
    }
    fn sync(&mut self) {
        if let Some(engine) = self.engine.as_ref() {
            self.snapshot = engine.snapshot();
            if let Some(error) = engine.take_error() {
                self.status = error;
            }
            let advanced = self.boundary < self.snapshot.track_index;
            while self.boundary < self.snapshot.track_index {
                self.queue.advance();
                self.boundary += 1;
                self.scrobbled = false;
            }
            if advanced && let Some(song) = self.queue.current() {
                self.worker.send(Job::NowPlaying(song.id.clone()));
            }
        }
        let song_id = self.queue.current().map(|song| song.id.clone());
        if self.started && song_id != self.lyrics_song {
            self.lyrics.clear();
            self.lyrics_song = song_id;
            self.lyrics_track = 0;
            self.lyrics_selection.select(None);
            self.lyrics_ticket = 0;
            if let Some(song) = self.queue.current().cloned() {
                self.request(Job::Lyrics(Box::new(song)));
            }
        }
        if let Some(lyrics) = self.lyrics.get(self.lyrics_track) {
            let active = lyrics.active_range(self.snapshot.position_ms);
            if lyrics.synced {
                self.lyrics_selection.select(if active.is_empty() {
                    None
                } else {
                    Some(active.start)
                });
            }
        }
        if self.started
            && !self.scrobbled
            && self.snapshot.duration_ms > 0
            && self.snapshot.position_ms >= (self.snapshot.duration_ms / 2).min(240_000)
        {
            self.scrobbled = true;
            if let Some(song) = self.queue.current() {
                self.worker.send(Job::Scrobble(song.id.clone()));
            }
        }
    }
    fn spec(&self, song: &Song) -> StreamSpec {
        let quality = self
            .settings
            .max_bit_rate
            .map(|max_bit_rate| StreamQuality::Transcode { max_bit_rate })
            .unwrap_or(StreamQuality::Original);
        let url = self.client.stream_url(song, quality);
        let hint = if url.contains("format=raw") {
            song.suffix.clone()
        } else {
            Some("mp3".into())
        };
        StreamSpec {
            url,
            hint,
            duration_ms: song.duration.map(|duration| u64::from(duration) * 1000),
        }
    }
    fn start(&mut self, paused: bool) {
        let Some(song) = self.queue.current() else {
            return;
        };
        let Some(engine) = self.engine.as_ref() else {
            self.status = "No audio output device available".into();
            return;
        };
        engine.play_playlist(
            self.spec(song),
            self.queue
                .playback_upcoming()
                .iter()
                .map(|song| self.spec(song))
                .collect(),
            self.queue
                .playback_cycle()
                .iter()
                .map(|song| self.spec(song))
                .collect(),
        );
        if paused {
            engine.pause();
        }
        self.started = true;
        self.boundary = 0;
        self.scrobbled = false;
        self.snapshot = engine.snapshot();
        self.worker.send(Job::NowPlaying(song.id.clone()));
        self.sync();
    }
    fn refresh(&mut self) {
        if !self.started {
            self.start(false);
            return;
        }
        if let Some(engine) = self.engine.as_ref() {
            engine.replace_playlist(
                self.queue
                    .playback_upcoming()
                    .iter()
                    .map(|song| self.spec(song))
                    .collect(),
                self.queue
                    .playback_cycle()
                    .iter()
                    .map(|song| self.spec(song))
                    .collect(),
            );
        }
    }
    fn len(&self) -> usize {
        match self.page {
            Page::Albums => self.albums.len(),
            Page::Songs => self.songs.len(),
            Page::Playlists => self.playlists.len(),
            Page::Queue => self.queue.len(),
            Page::Lyrics => self
                .lyrics
                .get(self.lyrics_track)
                .map(|lyrics| lyrics.lines.len())
                .unwrap_or(0),
        }
    }
    fn reconnect_output(&mut self) {
        if let Some(engine) = self.engine.as_ref() {
            engine.pause();
        }
        self.sync();
        let position = self.snapshot.position_ms;
        let engine = match Engine::new() {
            Ok(engine) => engine,
            Err(error) => {
                self.status = format!("Audio output unavailable: {error}");
                return;
            }
        };
        engine.set_volume(self.settings.volume);
        if self.started
            && let Some(song) = self.queue.current()
        {
            engine.restore_playlist(
                self.spec(song),
                self.queue
                    .playback_upcoming()
                    .iter()
                    .map(|song| self.spec(song))
                    .collect(),
                self.queue
                    .playback_cycle()
                    .iter()
                    .map(|song| self.spec(song))
                    .collect(),
                position,
            );
        }
        self.engine = Some(engine);
        self.boundary = 0;
        self.status = "Audio reconnected · Space to resume".into();
        self.sync();
    }
    fn key(&mut self, key: KeyEvent) -> bool {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return true;
        }
        if let Some(query) = self.query.as_mut() {
            match key.code {
                KeyCode::Esc => self.query = None,
                KeyCode::Backspace => {
                    query.pop();
                }
                KeyCode::Enter => {
                    let query = self.query.take().unwrap();
                    if !query.trim().is_empty() {
                        self.request(Job::Search(query));
                    }
                }
                KeyCode::Char(ch)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    query.push(ch)
                }
                _ => {}
            }
            return false;
        }
        self.sync();
        let index = if self.page == Page::Lyrics {
            self.lyrics_selection.selected()
        } else {
            self.selection.selected()
        }
        .unwrap_or(0);
        match key.code {
            KeyCode::Char('q') => return true,
            KeyCode::Char('e') => self.reconnect_output(),
            KeyCode::Char('/') => self.query = Some(String::new()),
            KeyCode::Char('1') => self.page(Page::Albums),
            KeyCode::Esc => self.back(),
            KeyCode::Char('2') => self.page(Page::Playlists),
            KeyCode::Char('3') => self.page(Page::Queue),
            KeyCode::Char('4') => self.page(Page::Lyrics),
            KeyCode::Char('5') => self.library(),
            KeyCode::Up | KeyCode::Char('k') => {
                if self.page == Page::Lyrics {
                    self.lyrics_selection.select(Some(index.saturating_sub(1)));
                } else {
                    self.selection.select(Some(index.saturating_sub(1)));
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let index = (index + 1).min(self.len().saturating_sub(1));
                if self.page == Page::Lyrics {
                    self.lyrics_selection.select(Some(index));
                } else {
                    self.selection.select(Some(index));
                }
            }
            KeyCode::Char('L')
                if self.page == Page::Songs
                    && self.songs_library
                    && self.songs_more
                    && self.status != "Loading…" =>
            {
                self.request(Job::Library(self.songs_offset));
            }
            KeyCode::Char('L')
                if self.page == Page::Albums && self.more && self.status != "Loading…" =>
            {
                self.request(Job::Albums(self.offset))
            }
            KeyCode::Enter => match self.page {
                Page::Albums => {
                    if let Some(album) = self.albums.get(index) {
                        self.request(Job::Album(album.id.clone()));
                    }
                }
                Page::Playlists => {
                    if let Some(playlist) = self.playlists.get(index) {
                        self.request(Job::Playlist(playlist.id.clone()));
                    }
                }
                Page::Songs => {
                    if index < self.songs.len() {
                        self.queue.play_album(self.songs.clone(), index);
                        self.start(false);
                    }
                }
                Page::Queue => {
                    if self.queue.select(index) {
                        self.start(false);
                    }
                }
                Page::Lyrics => {}
            },
            KeyCode::Char(' ') => {
                if !self.started || self.snapshot.ended {
                    self.start(false);
                } else if let Some(engine) = self.engine.as_ref() {
                    if self.snapshot.playing {
                        engine.pause();
                    } else {
                        engine.resume();
                    }
                }
            }
            KeyCode::Char('n') => {
                if self.queue.next_track().is_some() {
                    self.start(false);
                }
            }
            KeyCode::Char('p') => {
                if self.queue.previous().is_some() {
                    self.start(false);
                }
            }
            KeyCode::Left | KeyCode::Right => {
                if let Some(engine) = self.engine.as_ref() {
                    engine.seek(
                        self.snapshot
                            .position_ms
                            .saturating_add_signed(if key.code == KeyCode::Left {
                                -5000
                            } else {
                                5000
                            })
                            .min(self.snapshot.duration_ms),
                    );
                }
            }
            KeyCode::Char('+') | KeyCode::Char('=') | KeyCode::Char('-') => {
                self.settings.volume = (self.settings.volume
                    + if key.code == KeyCode::Char('-') {
                        -0.05
                    } else {
                        0.05
                    })
                .clamp(0., 1.);
                self.dirty = true;
                if let Some(engine) = self.engine.as_ref() {
                    engine.set_volume(self.settings.volume);
                }
            }
            KeyCode::Char('s') => {
                self.queue.set_shuffle(!self.queue.shuffle());
                self.settings.shuffle = self.queue.shuffle();
                self.dirty = true;
                self.refresh();
            }
            KeyCode::Char('r') => {
                self.queue.set_repeat(match self.queue.repeat() {
                    RepeatMode::Off => RepeatMode::All,
                    RepeatMode::All => RepeatMode::One,
                    RepeatMode::One => RepeatMode::Off,
                });
                self.settings.repeat = self.queue.repeat();
                self.dirty = true;
                self.refresh();
            }
            KeyCode::Char('a') | KeyCode::Char('i') if self.page == Page::Songs => {
                if let Some(song) = self.songs.get(index).cloned() {
                    if key.code == KeyCode::Char('i') {
                        self.queue.insert_next(song);
                    } else {
                        self.queue.append(vec![song]);
                    }
                    self.refresh();
                }
            }
            KeyCode::Char('d') if self.page == Page::Queue => {
                let current = self.queue.current_index() == Some(index);
                let paused = !self.snapshot.playing;
                if self.queue.remove(index).is_some() {
                    if current {
                        if self.queue.current().is_some() {
                            self.start(paused);
                        } else {
                            if let Some(engine) = self.engine.as_ref() {
                                engine.stop();
                            }
                            self.started = false;
                            self.lyrics.clear();
                            self.lyrics_ticket = 0;
                            self.lyrics_song = None;
                        }
                    } else {
                        self.refresh();
                    }
                    self.selection
                        .select(Some(index.min(self.len().saturating_sub(1))));
                }
            }
            KeyCode::Char('J') | KeyCode::Char('K') if self.page == Page::Queue => {
                let target = if key.code == KeyCode::Char('K') {
                    index.saturating_sub(1)
                } else {
                    (index + 1).min(self.len().saturating_sub(1))
                };
                if self.queue.move_song(index, target) {
                    self.selection.select(Some(target));
                    self.refresh();
                }
            }
            KeyCode::Char('D') if self.page == Page::Queue => {
                self.queue.clear_upcoming();
                self.refresh();
            }
            KeyCode::Char('t') if self.page == Page::Lyrics && !self.lyrics.is_empty() => {
                self.lyrics_track = (self.lyrics_track + 1) % self.lyrics.len();
                self.lyrics_selection = ListState::default();
            }
            _ => {}
        }
        false
    }
    fn render(&mut self, frame: &mut Frame) {
        let [tabs, body, player, footer] = Layout::vertical([
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(3),
            Constraint::Length(4),
        ])
        .areas(frame.area());
        frame.render_widget(
            Paragraph::new(format!(
                "Melos {} · 1 Albums · 2 Playlists · 3 Queue · 4 Lyrics · 5 Songs · / Search",
                env!("CARGO_PKG_VERSION")
            ))
            .block(Block::bordered()),
            tabs,
        );
        let (title, rows): (String, Vec<ListItem>) = match self.page {
            Page::Albums => (
                "Albums · Enter open · L load more".into(),
                self.albums
                    .iter()
                    .map(|album| {
                        ListItem::new(format!(
                            "{} · {}",
                            album.name,
                            album.artist.as_deref().unwrap_or("")
                        ))
                    })
                    .collect(),
            ),
            Page::Songs => (
                if self.songs_library {
                    format!(
                        "{} · {} songs · Enter play · a append · i play next · L load more",
                        self.title,
                        self.songs.len()
                    )
                } else {
                    format!("{} · Enter play · a append · i play next", self.title)
                },
                self.songs
                    .iter()
                    .map(|song| {
                        ListItem::new(format!(
                            "{} · {}",
                            song.title,
                            song.artist.as_deref().unwrap_or("")
                        ))
                    })
                    .collect(),
            ),
            Page::Playlists => (
                "Playlists · Enter open".into(),
                self.playlists
                    .iter()
                    .map(|playlist| {
                        ListItem::new(format!(
                            "{} · {} songs",
                            playlist.name,
                            playlist.song_count.unwrap_or(0)
                        ))
                    })
                    .collect(),
            ),
            Page::Queue => (
                "Queue · Enter play · d remove · J/K reorder · D clear upcoming".into(),
                self.queue
                    .songs()
                    .iter()
                    .enumerate()
                    .map(|(index, song)| {
                        ListItem::new(format!(
                            "{} {}",
                            if self.queue.current_index() == Some(index) {
                                "▶"
                            } else {
                                " "
                            },
                            song.title
                        ))
                    })
                    .collect(),
            ),
            Page::Lyrics => {
                let lyrics = self.lyrics.get(self.lyrics_track);
                let active = lyrics
                    .map(|lyrics| lyrics.active_range(self.snapshot.position_ms))
                    .unwrap_or(0..0);
                let title = format!(
                    "Lyrics · {} · t switch",
                    lyrics
                        .and_then(|lyrics| lyrics.language.as_deref())
                        .unwrap_or("")
                );
                let rows = lyrics
                    .map(|lyrics| {
                        lyrics
                            .lines
                            .iter()
                            .enumerate()
                            .map(|(index, line)| {
                                ListItem::new(line.text.clone()).style(if active.contains(&index) {
                                    Style::default().fg(Color::Cyan)
                                } else {
                                    Style::default()
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_else(|| {
                        vec![ListItem::new(
                            self.lyrics_status
                                .clone()
                                .unwrap_or_else(|| "No lyrics".into()),
                        )]
                    });
                (title, rows)
            }
        };
        // Keep the active lyric line in the middle instead of only scrolling
        // it into view; plain lyrics stay manually scrollable.
        if self.page == Page::Lyrics
            && self
                .lyrics
                .get(self.lyrics_track)
                .is_some_and(|lyrics| lyrics.synced)
            && let Some(index) = self.lyrics_selection.selected()
        {
            let height = body.height.saturating_sub(2) as usize;
            *self.lyrics_selection.offset_mut() = index.saturating_sub(height / 2);
        }
        frame.render_stateful_widget(
            List::new(rows)
                .block(Block::bordered().title(title))
                .highlight_style(Style::default().bg(Color::DarkGray))
                .highlight_symbol("› "),
            body,
            if self.page == Page::Lyrics {
                &mut self.lyrics_selection
            } else {
                &mut self.selection
            },
        );
        let song = if self.started {
            self.queue
                .current()
                .map(|song| song.title.as_str())
                .unwrap_or("Stopped")
        } else {
            "Choose a song"
        };
        frame.render_widget(
            Paragraph::new(format!(
                "{} · {} · {} / {} · Vol {:.0}% · Shuffle {} · Repeat {:?}",
                song,
                if self.snapshot.playing {
                    "Playing"
                } else {
                    "Paused"
                },
                clock(self.snapshot.position_ms),
                clock(self.snapshot.duration_ms),
                self.settings.volume * 100.,
                self.queue.shuffle(),
                self.queue.repeat()
            ))
            .block(Block::bordered()),
            player,
        );
        let status = self
            .query
            .as_ref()
            .map(|query| format!("Search: {query} · Enter submit · Esc cancel"))
            .unwrap_or_else(|| {
                format!(
                    "{}\nSpace pause · n/p skip · ←/→ seek · +/- volume · e audio · Esc back · q quit",
                    self.status
                )
            });
        frame.render_widget(
            Paragraph::new(status)
                .wrap(Wrap { trim: false })
                .block(Block::bordered()),
            footer,
        );
    }
}
fn clock(ms: u64) -> String {
    format!("{}:{:02}", ms / 60_000, ms / 1000 % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};
    fn app() -> App {
        App::new(
            Client::new("http://127.0.0.1:1", Credentials::new("qa", "qa")).unwrap(),
            Settings::default(),
            false,
        )
    }
    #[test]
    fn search_text_never_triggers_transport_or_quit() {
        let mut app = app();
        app.key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
        for ch in "q sr 中文".chars() {
            assert!(!app.key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE)));
        }
        assert_eq!(app.query.as_deref(), Some("q sr 中文"));
        assert!(!app.dirty);
        assert!(!app.queue.shuffle());
        assert_eq!(app.queue.repeat(), RepeatMode::Off);
    }
    #[test]
    fn esc_returns_to_the_previous_page() {
        let mut app = app();
        app.page(Page::Playlists);
        assert_eq!(app.page, Page::Playlists);
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.page, Page::Albums);
        assert!(app.history.is_empty());
    }

    #[test]
    fn empty_pages_and_small_terminals_are_safe() {
        let mut app = app();
        for page in [
            Page::Albums,
            Page::Songs,
            Page::Playlists,
            Page::Queue,
            Page::Lyrics,
        ] {
            app.page = page;
            for key in [
                KeyCode::Down,
                KeyCode::Enter,
                KeyCode::Char('d'),
                KeyCode::Char('J'),
                KeyCode::Char('K'),
                KeyCode::Char('t'),
            ] {
                app.key(KeyEvent::new(key, KeyModifiers::NONE));
            }
            for (width, height) in [(80, 24), (20, 8), (5, 3)] {
                Terminal::new(TestBackend::new(width, height))
                    .unwrap()
                    .draw(|frame| app.render(frame))
                    .unwrap();
            }
        }
    }

    #[test]
    fn plain_lyrics_can_scroll_beyond_the_first_screen() {
        let mut app = app();
        app.page = Page::Lyrics;
        app.lyrics = vec![Lyrics::parse(
            &(0..30)
                .map(|index| format!("Line {index}\n"))
                .collect::<String>(),
        )];
        for _ in 0..28 {
            app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        }
        app.sync();
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
        let screen: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(screen.contains("Line 28"));
        assert!(!screen.contains("Line 0"));
    }
}
