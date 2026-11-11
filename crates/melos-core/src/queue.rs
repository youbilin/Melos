//! Playback queue state machine.

use melos_api::Song;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, hash_map::RandomState};
use std::hash::BuildHasher;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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
    entry_ids: Vec<u64>,
    natural_order: Vec<u64>,
    next_entry_id: u64,
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

    pub fn playback_upcoming(&self) -> &[Song] {
        if self.repeat == RepeatMode::One {
            &[]
        } else {
            self.upcoming()
        }
    }

    /// A finite cycle reused by the decoder, never an infinite prefetch list.
    pub fn playback_cycle(&self) -> &[Song] {
        match self.repeat {
            RepeatMode::Off => &[],
            RepeatMode::All => &self.songs,
            RepeatMode::One => self.current().map(std::slice::from_ref).unwrap_or(&[]),
        }
    }

    fn allocate_entry(&mut self) -> u64 {
        let id = self.next_entry_id;
        self.next_entry_id += 1;
        id
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
        for _ in &songs {
            let id = self.allocate_entry();
            self.entry_ids.push(id);
            self.natural_order.push(id);
        }
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
        let id = self.allocate_entry();
        let natural_index = self
            .current
            .and_then(|current| {
                self.natural_order
                    .iter()
                    .position(|entry| *entry == self.entry_ids[current])
            })
            .map(|index| index + 1)
            .unwrap_or(self.natural_order.len());
        self.natural_order.insert(natural_index, id);
        self.entry_ids.insert(index, id);
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
        let id = self.entry_ids.remove(index);
        self.natural_order.retain(|entry| *entry != id);
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
        let id = self.entry_ids.remove(from);
        self.entry_ids.insert(to, id);
        // Explicit sorting becomes the order restored when shuffle is disabled.
        self.natural_order.clone_from(&self.entry_ids);
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
            self.entry_ids.truncate(current + 1);
            self.natural_order.retain(|id| self.entry_ids.contains(id));
        } else {
            self.clear();
        }
    }

    pub fn clear(&mut self) {
        self.songs.clear();
        self.entry_ids.clear();
        self.natural_order.clear();
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
        if self.shuffle == on {
            return;
        }
        self.shuffle = on;
        self.reorder_upcoming();
    }

    fn reorder_upcoming(&mut self) {
        let start = self.current.map(|index| index + 1).unwrap_or(0);
        let mut entries: Vec<_> = self.songs[start..]
            .iter()
            .cloned()
            .zip(self.entry_ids[start..].iter().copied())
            .collect();
        if self.shuffle {
            let random = RandomState::new();
            entries.sort_by_cached_key(|(_, id)| random.hash_one(id));
        } else {
            let positions: HashMap<_, _> = self
                .natural_order
                .iter()
                .enumerate()
                .map(|(index, id)| (*id, index))
                .collect();
            entries.sort_by_key(|(_, id)| positions[id]);
        }
        for (index, (song, id)) in entries.into_iter().enumerate() {
            self.songs[start + index] = song;
            self.entry_ids[start + index] = id;
        }
    }

    /// Replaces the queue and starts at `start`, clamped to the last track.
    pub fn play_album(&mut self, songs: Vec<Song>, start: usize) -> Option<&Song> {
        if songs.is_empty() {
            self.clear();
            return None;
        }
        self.songs = songs;
        self.entry_ids = (0..self.songs.len())
            .map(|_| self.allocate_entry())
            .collect();
        self.natural_order.clone_from(&self.entry_ids);
        self.current = Some(start.min(self.songs.len() - 1));
        if self.shuffle {
            self.reorder_upcoming();
        }
        self.current()
    }

    pub fn advance(&mut self) -> Option<&Song> {
        if self.repeat == RepeatMode::One {
            return self.current();
        }
        self.next_track()
    }

    /// Explicit next skips the track even with single-track repeat enabled.
    pub fn next_track(&mut self) -> Option<&Song> {
        let i = self.current?;
        if self.songs.is_empty() {
            return None;
        }
        let next = match self.repeat {
            RepeatMode::All => (i + 1) % self.songs.len(),
            RepeatMode::Off | RepeatMode::One => {
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
        assert_eq!(q.advance().unwrap().id, "0");
    }

    #[test]
    fn repeats_same_track_with_repeat_one() {
        let mut q = queue(2);
        q.set_repeat(RepeatMode::One);
        assert_eq!(q.advance().unwrap().id, "0");
        assert_eq!(q.next_track().unwrap().id, "1");
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

    #[test]
    fn shuffle_preserves_history_current_and_all_distinct_entries() {
        let mut q = queue(12);
        q.select(3);
        q.set_shuffle(true);
        assert_eq!(q.current_index(), Some(3));
        assert_eq!(
            q.songs()[..4]
                .iter()
                .map(|song| song.id.as_str())
                .collect::<Vec<_>>(),
            ["0", "1", "2", "3"]
        );
        let mut ids = q.entry_ids.clone();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 12);
        q.set_shuffle(false);
        assert_eq!(
            q.songs()
                .iter()
                .map(|song| song.id.clone())
                .collect::<Vec<_>>(),
            (0..12).map(|i| i.to_string()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn shuffle_restoration_retains_duplicate_entries_and_explicit_edits() {
        let mut q = Queue::new();
        q.play_album(
            vec![song("same"), song("same"), song("other"), song("last")],
            0,
        );
        q.set_shuffle(true);
        q.insert_next(song("inserted"));
        q.append(vec![song("appended")]);
        q.set_shuffle(false);
        assert_eq!(
            q.songs()
                .iter()
                .map(|song| song.id.as_str())
                .collect::<Vec<_>>(),
            ["same", "inserted", "same", "other", "last", "appended"]
        );
        q.set_shuffle(true);
        q.move_song(4, 1);
        let edited: Vec<_> = q.songs().iter().map(|song| song.id.clone()).collect();
        q.set_shuffle(false);
        assert_eq!(
            q.songs()
                .iter()
                .map(|song| song.id.clone())
                .collect::<Vec<_>>(),
            edited
        );
        q.remove(2);
        q.clear_upcoming();
        q.set_shuffle(true);
        q.set_shuffle(false);
        assert_eq!(q.songs().len(), 1);
    }

    #[test]
    fn repeat_plan_and_auto_advance_match_manual_skip_behavior() {
        let mut q = queue(3);
        q.select(1);
        q.set_repeat(RepeatMode::One);
        assert!(q.playback_upcoming().is_empty());
        assert_eq!(q.playback_cycle()[0].id, "1");
        assert_eq!(q.advance().unwrap().id, "1");
        assert_eq!(q.next_track().unwrap().id, "2");
        q.set_repeat(RepeatMode::All);
        assert_eq!(q.playback_cycle().len(), 3);
        assert_eq!(q.advance().unwrap().id, "0");
        q.set_repeat(RepeatMode::Off);
        assert!(q.playback_cycle().is_empty());
        assert_eq!(q.playback_upcoming().len(), 2);
    }
}
