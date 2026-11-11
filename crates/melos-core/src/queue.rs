//! Playback queue state machine.

use melos_api::Song;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RepeatMode {
    #[default]
    Off,
    All,
    One,
}

#[derive(Debug, Default)]
pub struct Queue {
    songs: Vec<Song>,
    current: Option<usize>,
    repeat: RepeatMode,
    shuffle: bool,
}

impl Queue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn songs(&self) -> &[Song] {
        &self.songs
    }

    pub fn current_index(&self) -> Option<usize> {
        self.current
    }

    pub fn current(&self) -> Option<&Song> {
        self.current.and_then(|i| self.songs.get(i))
    }

    pub fn repeat(&self) -> RepeatMode {
        self.repeat
    }

    pub fn set_repeat(&mut self, mode: RepeatMode) {
        self.repeat = mode;
    }

    pub fn shuffle(&self) -> bool {
        self.shuffle
    }

    pub fn set_shuffle(&mut self, on: bool) {
        self.shuffle = on;
    }

    /// Replaces the queue and starts at `start`, clamped to the last track.
    pub fn play_album(&mut self, songs: Vec<Song>, start: usize) -> Option<&Song> {
        if songs.is_empty() {
            self.songs.clear();
            self.current = None;
            return None;
        }
        self.songs = songs;
        self.current = Some(start.min(self.songs.len() - 1));
        self.current()
    }

    pub fn next_track(&mut self) -> Option<&Song> {
        let i = self.current?;
        if self.songs.is_empty() {
            return None;
        }
        let next = match self.repeat {
            RepeatMode::One => i,
            RepeatMode::All => (i + 1) % self.songs.len(),
            RepeatMode::Off => {
                if i + 1 >= self.songs.len() {
                    return None;
                }
                i + 1
            }
        };
        self.current = Some(next);
        self.current()
    }

    pub fn previous(&mut self) -> Option<&Song> {
        let i = self.current?;
        if self.songs.is_empty() {
            return None;
        }
        let prev = match self.repeat {
            RepeatMode::All => (i + self.songs.len() - 1) % self.songs.len(),
            _ => i.saturating_sub(1),
        };
        self.current = Some(prev);
        self.current()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(id: &str) -> Song {
        Song {
            id: id.to_string(),
            title: format!("track {id}"),
            duration: Some(180),
            suffix: Some("flac".into()),
            content_type: Some("audio/flac".into()),
            bit_rate: Some(1000),
            ..Song::default()
        }
    }

    fn queue(n: usize) -> Queue {
        let mut q = Queue::new();
        q.play_album((0..n).map(|i| song(&i.to_string())).collect(), 0);
        q
    }

    #[test]
    fn clamps_start_index() {
        let mut q = Queue::new();
        let songs = vec![song("a"), song("b")];
        let current = q.play_album(songs, 99).unwrap();
        assert_eq!(current.id, "b");
    }

    #[test]
    fn stops_at_end_without_repeat() {
        let mut q = queue(2);
        assert_eq!(q.next_track().unwrap().id, "1");
        assert!(q.next_track().is_none());
    }

    #[test]
    fn wraps_with_repeat_all() {
        let mut q = queue(2);
        q.set_repeat(RepeatMode::All);
        assert_eq!(q.next_track().unwrap().id, "1");
        assert_eq!(q.next_track().unwrap().id, "0");
    }

    #[test]
    fn repeats_same_track_with_repeat_one() {
        let mut q = queue(2);
        q.set_repeat(RepeatMode::One);
        assert_eq!(q.next_track().unwrap().id, "0");
    }

    #[test]
    fn previous_clamps_at_start() {
        let mut q = queue(2);
        assert_eq!(q.previous().unwrap().id, "0");
    }
}
