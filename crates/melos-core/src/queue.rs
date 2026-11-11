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

    pub fn len(&self) -> usize {
        self.songs.len()
    }
    pub fn is_empty(&self) -> bool {
        self.songs.is_empty()
    }

    pub fn upcoming(&self) -> &[Song] {
        self.current
            .map(|index| &self.songs[index + 1..])
            .unwrap_or(&[])
    }

    pub fn select(&mut self, index: usize) -> bool {
        if index >= self.songs.len() {
            return false;
        }
        self.current = Some(index);
        true
    }

    pub fn append(&mut self, songs: Vec<Song>) {
        let first_new = self.songs.len();
        self.songs.extend(songs);
        if self.current.is_none() && first_new < self.songs.len() {
            self.current = Some(first_new);
        }
    }

    pub fn insert_next(&mut self, song: Song) {
        let index = self
            .current
            .map(|index| index + 1)
            .unwrap_or(self.songs.len());
        self.songs.insert(index, song);
        if self.current.is_none() {
            self.current = Some(index);
        }
    }

    pub fn remove(&mut self, index: usize) -> Option<Song> {
        if index >= self.songs.len() {
            return None;
        }
        let song = self.songs.remove(index);
        self.current = match self.current {
            Some(current) if index < current => Some(current - 1),
            Some(current) if index == current => (index < self.songs.len()).then_some(index),
            current => current,
        };
        Some(song)
    }

    /// Moves an entry to its final index, preserving the active entry even
    /// when the queue contains repeated copies of the same song.
    pub fn move_song(&mut self, from: usize, to: usize) -> bool {
        if from >= self.songs.len() || to >= self.songs.len() || from == to {
            return false;
        }
        let song = self.songs.remove(from);
        self.songs.insert(to, song);
        self.current = self.current.map(|current| {
            if current == from {
                to
            } else if from < current && to >= current {
                current - 1
            } else if from > current && to <= current {
                current + 1
            } else {
                current
            }
        });
        true
    }

    pub fn clear_upcoming(&mut self) {
        if let Some(current) = self.current {
            self.songs.truncate(current + 1);
        } else {
            self.songs.clear();
        }
    }

    pub fn clear(&mut self) {
        self.songs.clear();
        self.current = None;
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

    #[test]
    fn editing_upcoming_preserves_current_song() {
        let mut q = queue(3);
        q.select(1);
        q.insert_next(song("next"));
        q.append(vec![song("last")]);
        assert_eq!(q.current().unwrap().id, "1");
        assert_eq!(
            q.upcoming()
                .iter()
                .map(|song| song.id.as_str())
                .collect::<Vec<_>>(),
            ["next", "2", "last"]
        );
        q.clear_upcoming();
        assert_eq!(q.current().unwrap().id, "1");
        assert!(q.upcoming().is_empty());
    }

    #[test]
    fn deleting_before_current_or_current_keeps_correct_selection() {
        let mut q = queue(4);
        q.select(2);
        q.remove(0);
        assert_eq!(q.current_index(), Some(1));
        assert_eq!(q.current().unwrap().id, "2");
        q.remove(1);
        assert_eq!(q.current().unwrap().id, "3");
        q.remove(1);
        assert!(q.current().is_none());
    }

    #[test]
    fn reordering_preserves_the_active_duplicate_entry() {
        let mut q = Queue::new();
        q.play_album(vec![song("same"), song("same"), song("other")], 1);
        q.move_song(0, 2);
        assert_eq!(q.current_index(), Some(0));
        q.move_song(0, 2);
        assert_eq!(q.current_index(), Some(2));
        assert_eq!(q.current().unwrap().id, "same");
    }

    #[test]
    fn insertion_into_empty_queue_and_invalid_edits_are_safe() {
        let mut q = Queue::new();
        assert!(q.remove(0).is_none());
        assert!(!q.move_song(0, 1));
        q.insert_next(song("first"));
        assert_eq!(q.current().unwrap().id, "first");
        q.clear();
        q.append(vec![song("second")]);
        assert_eq!(q.current().unwrap().id, "second");
    }
}
