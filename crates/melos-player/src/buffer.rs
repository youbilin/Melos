//! Fixed-capacity PCM ring. The callback never acquires the producer mutex.
//!
//! Publishing tail makes written samples visible; publishing head permits
//! their slots to be reused. A separate gate excludes callbacks only while
//! seek/queue edits change the ring and its associated playback counters.
use std::sync::atomic::{AtomicU8, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};

const IDLE: u8 = 0;
const CONSUMING: u8 = 1;
const EDITING: u8 = 2;

pub(crate) struct PcmQueue {
    samples: Box<[AtomicU32]>,
    head: AtomicUsize,
    tail: AtomicUsize,
    producer: Mutex<()>,
    gate: AtomicU8,
}

impl PcmQueue {
    pub(crate) fn new(capacity: usize) -> Self {
        assert!(capacity > 0 && capacity <= usize::MAX / 2);
        Self {
            samples: (0..capacity).map(|_| AtomicU32::new(0)).collect(),
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
            producer: Mutex::new(()),
            gate: AtomicU8::new(IDLE),
        }
    }

    pub(crate) fn capacity(&self) -> usize {
        self.samples.len()
    }

    // Counters wrap at twice capacity so slot indices also wrap correctly,
    // including when capacity is not a power of two.
    fn advance(&self, index: usize, count: usize) -> usize {
        let remaining = self.capacity() * 2 - index;
        if count >= remaining {
            count - remaining
        } else {
            index + count
        }
    }

    fn distance(&self, head: usize, tail: usize) -> usize {
        if tail >= head {
            tail - head
        } else {
            self.capacity() * 2 - head + tail
        }
    }

    pub(crate) fn producer(&self) -> Producer<'_> {
        Producer {
            queue: self,
            _lock: self.producer.lock().expect("PCM producer lock"),
        }
    }

    pub(crate) fn edit(&self) -> Editor<'_> {
        let producer = self.producer();
        while self
            .gate
            .compare_exchange(IDLE, EDITING, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            // Queue edits run outside the real-time thread. The callback
            // itself never waits for this gate or for an unscheduled producer.
            std::thread::yield_now();
        }
        Editor { producer }
    }

    pub(crate) fn try_consumer(&self) -> Option<Consumer<'_>> {
        self.gate
            .compare_exchange(IDLE, CONSUMING, Ordering::Acquire, Ordering::Relaxed)
            .ok()?;
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        Some(Consumer {
            queue: self,
            head,
            slot: head % self.capacity(),
            remaining: self.distance(head, tail),
            consumed: 0,
        })
    }
}

pub(crate) struct Producer<'a> {
    queue: &'a PcmQueue,
    _lock: MutexGuard<'a, ()>,
}

impl Producer<'_> {
    pub(crate) fn len(&self) -> usize {
        let head = self.queue.head.load(Ordering::Acquire);
        let tail = self.queue.tail.load(Ordering::Relaxed);
        self.queue.distance(head, tail)
    }

    pub(crate) fn push(&mut self, samples: &[f32]) -> bool {
        if samples.len() > self.queue.capacity() - self.len() {
            return false;
        }
        let tail = self.queue.tail.load(Ordering::Relaxed);
        let mut slot = tail % self.queue.capacity();
        for sample in samples {
            self.queue.samples[slot].store(sample.to_bits(), Ordering::Relaxed);
            slot += 1;
            if slot == self.queue.capacity() {
                slot = 0;
            }
        }
        self.queue
            .tail
            .store(self.queue.advance(tail, samples.len()), Ordering::Release);
        true
    }
}

pub(crate) struct Editor<'a> {
    producer: Producer<'a>,
}

impl Editor<'_> {
    pub(crate) fn len(&self) -> usize {
        self.producer.len()
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub(crate) fn clear(&mut self) {
        self.producer.queue.head.store(0, Ordering::Relaxed);
        self.producer.queue.tail.store(0, Ordering::Release);
    }

    pub(crate) fn truncate(&mut self, len: usize) {
        if len < self.len() {
            let queue = self.producer.queue;
            let head = queue.head.load(Ordering::Relaxed);
            queue
                .tail
                .store(queue.advance(head, len), Ordering::Release);
        }
    }

    #[cfg(test)]
    pub(crate) fn iter(&self) -> impl Iterator<Item = f32> + '_ {
        let queue = self.producer.queue;
        let head = queue.head.load(Ordering::Relaxed);
        (0..self.len()).map(move |offset| {
            f32::from_bits(
                queue.samples[queue.advance(head, offset) % queue.capacity()]
                    .load(Ordering::Relaxed),
            )
        })
    }
}

impl Drop for Editor<'_> {
    fn drop(&mut self) {
        self.producer.queue.gate.store(IDLE, Ordering::Release);
    }
}

pub(crate) struct Consumer<'a> {
    queue: &'a PcmQueue,
    head: usize,
    slot: usize,
    remaining: usize,
    consumed: usize,
}

impl Consumer<'_> {
    pub(crate) fn pop_front(&mut self) -> Option<f32> {
        if self.remaining == 0 {
            // Decoding can publish another packet during this callback.
            let head = self.queue.advance(self.head, self.consumed);
            self.remaining = self
                .queue
                .distance(head, self.queue.tail.load(Ordering::Acquire));
            if self.remaining == 0 {
                return None;
            }
        }
        let value = f32::from_bits(self.queue.samples[self.slot].load(Ordering::Relaxed));
        self.slot += 1;
        if self.slot == self.queue.capacity() {
            self.slot = 0;
        }
        self.remaining -= 1;
        self.consumed += 1;
        Some(value)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.queue.advance(self.head, self.consumed) == self.queue.tail.load(Ordering::Acquire)
    }
}

impl Drop for Consumer<'_> {
    fn drop(&mut self) {
        self.queue.head.store(
            self.queue.advance(self.head, self.consumed),
            Ordering::Release,
        );
        self.queue.gate.store(IDLE, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(queue: &PcmQueue, count: usize) -> Vec<f32> {
        let mut consumer = queue.try_consumer().unwrap();
        (0..count).map(|_| consumer.pop_front().unwrap()).collect()
    }

    #[test]
    fn wrap_truncate_and_clear_preserve_order_and_capacity() {
        let queue = PcmQueue::new(5);
        for _ in 0..20 {
            assert!(queue.producer().push(&[1., 2., 3., 4., 5.]));
            assert!(!queue.producer().push(&[6.]));
            assert_eq!(read(&queue, 3), [1., 2., 3.]);
            assert!(queue.producer().push(&[6., 7., 8.]));
            queue.edit().truncate(3);
            assert!(queue.producer().push(&[9., 10.]));
            assert_eq!(read(&queue, 5), [4., 5., 6., 9., 10.]);
        }
        assert!(queue.producer().push(&[11., 12.]));
        queue.edit().clear();
        assert!(queue.producer().push(&[13., 14., 15., 16., 17.]));
        assert_eq!(read(&queue, 5), [13., 14., 15., 16., 17.]);
    }

    #[test]
    fn callback_is_exclusive_and_sees_new_publications() {
        let queue = PcmQueue::new(5);
        assert!(queue.producer().push(&[1., 2.]));
        let mut consumer = queue.try_consumer().unwrap();
        assert!(queue.try_consumer().is_none());
        assert!(queue.producer().push(&[3., 4., 5.]));
        assert_eq!(consumer.pop_front(), Some(1.));
        assert_eq!(consumer.pop_front(), Some(2.));
        assert_eq!(consumer.pop_front(), Some(3.));
        drop(consumer);
        assert_eq!(read(&queue, 2), [4., 5.]);
    }

    #[test]
    fn unpublished_slots_are_invisible_until_tail_is_published() {
        let queue = PcmQueue::new(5);
        let mut producer = queue.producer();
        queue.samples[0].store(42.0f32.to_bits(), Ordering::Relaxed);
        assert_eq!(queue.try_consumer().unwrap().pop_front(), None);
        assert!(producer.push(&[43.]));
        assert_eq!(read(&queue, 1), [43.]);
    }

    #[test]
    fn concurrent_small_ring_keeps_every_sample_across_wraps() {
        let queue = PcmQueue::new(17);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                for index in 0..20_000 {
                    while !queue.producer().push(&[index as f32]) {
                        std::thread::yield_now();
                    }
                }
            });
            for expected in 0..20_000 {
                loop {
                    let value = queue.try_consumer().unwrap().pop_front();
                    if let Some(value) = value {
                        assert_eq!(value, expected as f32);
                        break;
                    }
                    std::thread::yield_now();
                }
            }
        });
        assert!(queue.edit().is_empty());
    }
}
