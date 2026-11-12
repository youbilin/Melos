//! Threaded playback engine: decode thread -> bounded queue -> cpal callback.
//!
//! The queue is a `Mutex<VecDeque<f32>>` that the audio callback only ever
//! `try_lock`s; if the producer holds it, the callback emits silence for that
//! period instead of blocking the device. This keeps the real-time path safe
//! without an unsafe lock-free ring in the first iteration.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig};

use crate::decoder::Decoder;
use crate::error::PlayerError;
use crate::source;

/// How long the engine keeps decoded audio ahead of playback.
const QUEUE_SECONDS: usize = 3;
// Retry only a fixed number of times for a short producer critical section.
// Never wait for an unscheduled producer or take a blocking callback lock.
const CALLBACK_QUEUE_TRIES: usize = 64;

#[derive(Clone, Debug)]
pub struct StreamSpec {
    pub url: String,
    pub hint: Option<String>,
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct EngineSnapshot {
    pub position_ms: u64,
    pub duration_ms: u64,
    pub playing: bool,
    pub ended: bool,
    pub track_index: u64,
    pub underruns: u64,
    pub volume: f32,
    pub output_failed: bool,
}

#[derive(Debug, Clone)]
struct Boundary {
    spec: StreamSpec,
    start_frame: u64,
    base_ms: u64,
    duration_ms: u64,
}

struct Desired {
    generation: u64,
    current: Option<StreamSpec>,
    upcoming: VecDeque<StreamSpec>,
    cycle: Vec<StreamSpec>,
    resume: Option<DecodeTask>,
    decoding_start: u64,
    decoding_spec: Option<StreamSpec>,
    seek: Option<u64>,
}

#[derive(Clone)]
struct DecodeTask {
    spec: StreamSpec,
    start_frame: u64,
}

#[derive(Clone, Copy)]
struct DecodeToken {
    generation: u64,
    revision: u64,
}

struct Shared {
    queue: Mutex<VecDeque<f32>>,
    desired: Mutex<Desired>,
    wake: Condvar,
    boundaries: Mutex<VecDeque<Boundary>>,
    error: Mutex<Option<String>>,
    output_failed: AtomicBool,
    consumed_frames: AtomicU64,
    playing: AtomicBool,
    ended: AtomicBool,
    decoder_done: AtomicBool,
    volume_bits: AtomicU32,
    rate: AtomicU32,
    channels: AtomicU32,
    position_ms: AtomicU64,
    duration_ms: AtomicU64,
    track_index: AtomicU64,
    underruns: AtomicU64,
    shutdown: AtomicBool,
    generation: AtomicU64,
    revision: AtomicU64,
}

impl Shared {
    fn new(rate: u32, channels: u16) -> Self {
        Self {
            queue: Mutex::new(VecDeque::with_capacity(
                rate.max(1) as usize * usize::from(channels.max(1)) * QUEUE_SECONDS,
            )),
            desired: Mutex::new(Desired {
                generation: 0,
                current: None,
                upcoming: VecDeque::new(),
                cycle: Vec::new(),
                resume: None,
                decoding_start: 0,
                decoding_spec: None,
                seek: None,
            }),
            wake: Condvar::new(),
            boundaries: Mutex::new(VecDeque::new()),
            error: Mutex::new(None),
            output_failed: AtomicBool::new(false),
            consumed_frames: AtomicU64::new(0),
            playing: AtomicBool::new(false),
            ended: AtomicBool::new(false),
            decoder_done: AtomicBool::new(false),
            volume_bits: AtomicU32::new(0.8f32.to_bits()),
            rate: AtomicU32::new(rate.max(1)),
            channels: AtomicU32::new(u32::from(channels.max(1))),
            position_ms: AtomicU64::new(0),
            duration_ms: AtomicU64::new(0),
            track_index: AtomicU64::new(0),
            underruns: AtomicU64::new(0),
            shutdown: AtomicBool::new(false),
            generation: AtomicU64::new(0),
            revision: AtomicU64::new(0),
        }
    }

    fn set_error(&self, error: PlayerError) {
        *self.error.lock().expect("error lock") = Some(error.to_string());
        self.playing.store(false, Ordering::Relaxed);
    }

    fn set_output_error(&self, error: PlayerError) {
        self.output_failed.store(true, Ordering::Release);
        self.set_error(error);
    }

    fn rate(&self) -> u32 {
        self.rate.load(Ordering::Relaxed).max(1)
    }

    fn channels(&self) -> usize {
        self.channels.load(Ordering::Relaxed).max(1) as usize
    }
}

pub struct Engine {
    shared: Arc<Shared>,
}

impl Engine {
    /// Opens the default output device and starts the audio, decode and monitor
    /// threads.
    pub fn new() -> Result<Self, PlayerError> {
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or(PlayerError::NoDevice)?;
        let supported = device
            .default_output_config()
            .map_err(|e| PlayerError::Config(e.to_string()))?;

        let config = supported.config();
        let sample_format = supported.sample_format();

        let shared = Arc::new(Shared::new(config.sample_rate, config.channels));

        {
            let shared = Arc::clone(&shared);
            thread::spawn(move || audio_loop(shared, device, config, sample_format));
        }
        {
            let shared = Arc::clone(&shared);
            thread::spawn(move || decode_loop(shared));
        }
        {
            let shared = Arc::clone(&shared);
            thread::spawn(move || monitor_loop(shared));
        }

        Ok(Self { shared })
    }

    /// Starts a new playback generation.
    pub fn play(&self, current: StreamSpec, next: Option<StreamSpec>) {
        self.play_queue(current, next.into_iter().collect());
    }

    pub fn play_queue(&self, current: StreamSpec, upcoming: Vec<StreamSpec>) {
        self.play_playlist(current, upcoming, vec![]);
    }

    pub fn play_playlist(
        &self,
        current: StreamSpec,
        upcoming: Vec<StreamSpec>,
        cycle: Vec<StreamSpec>,
    ) {
        self.start_playlist(current, upcoming, cycle, 0, true);
    }

    /// Restores the current track at a checkpoint on a freshly opened output.
    /// It starts paused so reconnecting headphones never resumes unexpectedly.
    pub fn restore_playlist(
        &self,
        current: StreamSpec,
        upcoming: Vec<StreamSpec>,
        cycle: Vec<StreamSpec>,
        position_ms: u64,
    ) {
        self.start_playlist(current, upcoming, cycle, position_ms, false);
    }

    fn start_playlist(
        &self,
        current: StreamSpec,
        upcoming: Vec<StreamSpec>,
        cycle: Vec<StreamSpec>,
        position_ms: u64,
        playing: bool,
    ) {
        let mut desired = self.shared.desired.lock().expect("desired lock");
        let mut boundaries = self.shared.boundaries.lock().expect("boundaries lock");
        let mut queue = self.shared.queue.lock().expect("queue lock");
        desired.generation = desired.generation.wrapping_add(1);
        self.shared
            .generation
            .store(desired.generation, Ordering::Release);
        self.shared.revision.fetch_add(1, Ordering::AcqRel);
        queue.clear();
        boundaries.clear();

        self.shared.consumed_frames.store(0, Ordering::Relaxed);
        self.shared
            .position_ms
            .store(position_ms, Ordering::Relaxed);
        self.shared
            .duration_ms
            .store(current.duration_ms.unwrap_or(0), Ordering::Relaxed);
        self.shared.track_index.store(0, Ordering::Relaxed);
        self.shared.ended.store(false, Ordering::Relaxed);
        self.shared.decoder_done.store(false, Ordering::Relaxed);
        self.shared.playing.store(
            playing && !self.shared.output_failed.load(Ordering::Acquire),
            Ordering::Relaxed,
        );

        desired.current = Some(current);
        desired.upcoming = upcoming.into();
        desired.cycle = cycle;
        desired.resume = None;
        desired.decoding_start = 0;
        desired.decoding_spec = None;
        desired.seek = (position_ms > 0).then_some(position_ms);
        drop(queue);
        drop(boundaries);
        drop(desired);
        self.shared.wake.notify_all();
    }

    /// Seeks within the current track.
    pub fn seek(&self, position_ms: u64) {
        update_position(&self.shared);
        let mut desired = self.shared.desired.lock().expect("desired lock");
        let mut boundaries = self.shared.boundaries.lock().expect("boundaries lock");
        let mut queue = self.shared.queue.lock().expect("queue lock");
        advance_boundaries(&self.shared, &mut boundaries);
        let Some(current) = boundaries
            .front()
            .map(|boundary| boundary.spec.clone())
            .or_else(|| desired.current.clone())
        else {
            return;
        };
        let mut upcoming: VecDeque<_> = boundaries
            .iter()
            .skip(1)
            .map(|boundary| boundary.spec.clone())
            .collect();
        if !boundaries.is_empty()
            && !self.shared.decoder_done.load(Ordering::Relaxed)
            && !boundaries
                .iter()
                .any(|boundary| boundary.start_frame == desired.decoding_start)
            && let Some(spec) = desired.decoding_spec.clone()
        {
            upcoming.push_back(spec);
        }
        upcoming.extend(desired.upcoming.drain(..));
        desired.generation = desired.generation.wrapping_add(1);
        self.shared
            .generation
            .store(desired.generation, Ordering::Release);
        self.shared.revision.fetch_add(1, Ordering::AcqRel);
        desired.current = Some(current);
        desired.upcoming = upcoming;
        desired.resume = None;
        desired.decoding_start = 0;
        desired.decoding_spec = None;
        desired.seek = Some(position_ms);
        boundaries.clear();
        queue.clear();
        self.shared.consumed_frames.store(0, Ordering::Relaxed);
        self.shared
            .position_ms
            .store(position_ms, Ordering::Relaxed);
        self.shared.decoder_done.store(false, Ordering::Relaxed);
        self.shared.ended.store(false, Ordering::Relaxed);
        drop(queue);
        drop(boundaries);
        drop(desired);
        self.shared.wake.notify_all();
    }

    /// Replaces the next track, including any audio already prefetched for it.
    pub fn set_next(&self, next: StreamSpec) {
        self.replace_upcoming(vec![next]);
    }

    /// Keeps all remaining samples of the audible track and invalidates only
    /// future decoding. Editing the queue never restarts the audible track.
    pub fn replace_upcoming(&self, upcoming: Vec<StreamSpec>) {
        self.replace_playlist(upcoming, vec![]);
    }

    pub fn replace_playlist(&self, upcoming: Vec<StreamSpec>, cycle: Vec<StreamSpec>) {
        replace_upcoming(&self.shared, upcoming, cycle);
    }

    pub fn pause(&self) {
        self.shared.playing.store(false, Ordering::Relaxed);
    }

    pub fn resume(&self) {
        self.shared.playing.store(
            !self.shared.output_failed.load(Ordering::Acquire),
            Ordering::Relaxed,
        );
    }

    pub fn stop(&self) {
        self.shared.playing.store(false, Ordering::Relaxed);
        if let Ok(mut desired) = self.shared.desired.lock() {
            desired.generation = desired.generation.wrapping_add(1);
            self.shared
                .generation
                .store(desired.generation, Ordering::Release);
            self.shared.revision.fetch_add(1, Ordering::AcqRel);
            desired.current = None;
            desired.upcoming.clear();
            desired.cycle.clear();
            desired.resume = None;
            desired.seek = None;
            self.shared
                .boundaries
                .lock()
                .expect("boundaries lock")
                .clear();
            self.shared.queue.lock().expect("queue lock").clear();
        }
        self.shared.position_ms.store(0, Ordering::Relaxed);
        self.shared.ended.store(false, Ordering::Relaxed);
        self.shared.wake.notify_all();
    }

    pub fn set_volume(&self, volume: f32) {
        self.shared
            .volume_bits
            .store(volume.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    pub fn volume(&self) -> f32 {
        f32::from_bits(self.shared.volume_bits.load(Ordering::Relaxed))
    }

    pub fn snapshot(&self) -> EngineSnapshot {
        update_position(&self.shared);
        EngineSnapshot {
            position_ms: self.shared.position_ms.load(Ordering::Relaxed),
            duration_ms: self.shared.duration_ms.load(Ordering::Relaxed),
            playing: self.is_playing(),
            ended: self.shared.ended.load(Ordering::Relaxed),
            track_index: self.shared.track_index.load(Ordering::Relaxed),
            underruns: self.shared.underruns.load(Ordering::Relaxed),
            volume: self.volume(),
            output_failed: self.shared.output_failed.load(Ordering::Acquire),
        }
    }

    pub fn take_error(&self) -> Option<String> {
        self.shared
            .error
            .lock()
            .ok()
            .and_then(|mut error| error.take())
    }

    pub fn is_playing(&self) -> bool {
        self.shared.playing.load(Ordering::Relaxed)
            && !self.shared.output_failed.load(Ordering::Acquire)
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.shared.shutdown.store(true, Ordering::Relaxed);
        self.shared.wake.notify_all();
    }
}

fn audio_loop(
    shared: Arc<Shared>,
    device: cpal::Device,
    config: StreamConfig,
    sample_format: SampleFormat,
) {
    match sample_format {
        SampleFormat::F32 => build::<f32>(&device, &config, &shared),
        SampleFormat::I16 => build::<i16>(&device, &config, &shared),
        SampleFormat::I32 => build::<i32>(&device, &config, &shared),
        SampleFormat::U16 => build::<u16>(&device, &config, &shared),
        SampleFormat::F64 => build::<f64>(&device, &config, &shared),
        other => shared.set_output_error(PlayerError::Config(format!(
            "unsupported device sample format: {other}"
        ))),
    }
}

fn build<T>(device: &cpal::Device, config: &StreamConfig, shared: &Arc<Shared>)
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels as usize;
    let shared_cb = Arc::clone(shared);
    let shared_error = Arc::clone(shared);

    let stream = device.build_output_stream(
        *config,
        move |data: &mut [T], _| fill(data, &shared_cb, channels),
        move |error| shared_error.set_output_error(PlayerError::Stream(error.to_string())),
        None,
    );

    let stream = match stream {
        Ok(stream) => stream,
        Err(error) => {
            shared.set_output_error(PlayerError::Stream(error.to_string()));
            return;
        }
    };

    if let Err(error) = stream.play() {
        shared.set_output_error(PlayerError::Stream(error.to_string()));
        return;
    }

    while !shared.shutdown.load(Ordering::Relaxed) && !shared.output_failed.load(Ordering::Acquire)
    {
        thread::sleep(Duration::from_millis(50));
    }
}

fn fill<T>(data: &mut [T], shared: &Shared, channels: usize)
where
    T: SizedSample + FromSample<f32>,
{
    if !shared.playing.load(Ordering::Relaxed) || shared.output_failed.load(Ordering::Acquire) {
        data.fill(T::from_sample(0.0));
        return;
    }
    let mut lock = shared.queue.try_lock();
    for _ in 1..CALLBACK_QUEUE_TRIES {
        if lock.is_ok() {
            break;
        }
        std::hint::spin_loop();
        lock = shared.queue.try_lock();
    }
    let mut queue = match lock {
        Ok(queue) => queue,
        Err(_) => {
            if shared.playing.load(Ordering::Relaxed)
                && !shared.decoder_done.load(Ordering::Relaxed)
            {
                shared.underruns.fetch_add(1, Ordering::Relaxed);
            }
            for sample in data.iter_mut() {
                *sample = T::from_sample(0.0);
            }
            return;
        }
    };

    if !shared.playing.load(Ordering::Relaxed) || shared.output_failed.load(Ordering::Acquire) {
        for sample in data.iter_mut() {
            *sample = T::from_sample(0.0);
        }
        return;
    }

    let volume = f32::from_bits(shared.volume_bits.load(Ordering::Relaxed));
    let mut consumed = 0usize;
    let mut starved = false;

    for sample in data.iter_mut() {
        match queue.pop_front() {
            Some(value) => {
                *sample = T::from_sample(value * volume);
                consumed += 1;
            }
            None => {
                *sample = T::from_sample(0.0);
                starved = true;
            }
        }
    }

    if starved && !shared.decoder_done.load(Ordering::Relaxed) {
        shared.underruns.fetch_add(1, Ordering::Relaxed);
    }
    if queue.is_empty() && shared.decoder_done.load(Ordering::Relaxed) {
        shared.ended.store(true, Ordering::Relaxed);
    }

    shared
        .consumed_frames
        .fetch_add((consumed / channels) as u64, Ordering::Relaxed);
    drop(queue);
}

fn take_next_spec(desired: &mut Desired) -> Option<StreamSpec> {
    if desired.upcoming.is_empty() {
        desired.upcoming.extend(desired.cycle.iter().cloned());
    }
    desired.upcoming.pop_front()
}

fn replace_upcoming(shared: &Shared, upcoming: Vec<StreamSpec>, cycle: Vec<StreamSpec>) {
    update_position(shared);
    let mut desired = shared.desired.lock().expect("desired lock");
    let mut boundaries = shared.boundaries.lock().expect("boundaries lock");
    let mut queue = shared.queue.lock().expect("queue lock");
    desired.upcoming = upcoming.into();
    desired.cycle = cycle;
    advance_boundaries(shared, &mut boundaries);
    let Some(current) = boundaries.front().cloned() else {
        return;
    };
    let future_decoding = desired.decoding_start > current.start_frame;
    if boundaries.len() > 1 || future_decoding || shared.decoder_done.load(Ordering::Relaxed) {
        let consumed = shared.consumed_frames.load(Ordering::Relaxed);
        let end = boundaries
            .get(1)
            .map(|boundary| boundary.start_frame)
            .or_else(|| future_decoding.then_some(desired.decoding_start))
            .unwrap_or(consumed + (queue.len() / shared.channels()) as u64);
        let keep = end.saturating_sub(consumed) as usize * shared.channels();
        queue.truncate(keep);
        boundaries.truncate(1);
        shared.revision.fetch_add(1, Ordering::AcqRel);
        desired.resume = take_next_spec(&mut desired).map(|spec| DecodeTask {
            spec,
            start_frame: end,
        });
        desired.decoding_spec = desired.resume.as_ref().map(|task| task.spec.clone());
        desired.decoding_start = end;
        let has_more = desired.resume.is_some();
        shared.decoder_done.store(!has_more, Ordering::Relaxed);
        shared
            .ended
            .store(!has_more && queue.is_empty(), Ordering::Relaxed);
    }
    drop(queue);
    drop(boundaries);
    drop(desired);
    shared.wake.notify_all();
}

fn decode_loop(shared: Arc<Shared>) {
    let mut last_generation = u64::MAX;
    let mut last_revision = u64::MAX;

    loop {
        if shared.shutdown.load(Ordering::Relaxed) {
            break;
        }

        let (token, task) = match shared.desired.lock() {
            Ok(mut desired) => {
                let token = DecodeToken {
                    generation: desired.generation,
                    revision: shared.revision.load(Ordering::Acquire),
                };
                let task = if token.generation != last_generation {
                    desired.current.clone().map(|spec| DecodeTask {
                        spec,
                        start_frame: 0,
                    })
                } else if token.revision != last_revision {
                    desired.resume.take()
                } else {
                    None
                };
                (token, task)
            }
            Err(_) => break,
        };

        last_generation = token.generation;
        last_revision = token.revision;
        if let Some(task) = task {
            decode_playlist(&shared, token, task);
        }

        if let Ok(desired) = shared.desired.lock() {
            let _ = shared.wake.wait_timeout(desired, Duration::from_millis(50));
        }
    }
}

fn decode_playlist(shared: &Arc<Shared>, token: DecodeToken, task: DecodeTask) {
    let mut next_spec = Some(task.spec);
    let mut produced = task.start_frame;

    while let Some(spec) = next_spec {
        if !is_current(shared, token) {
            return;
        }
        {
            let mut desired = shared.desired.lock().expect("desired lock");
            if !is_current(shared, token) {
                return;
            }
            desired.decoding_start = produced;
            desired.decoding_spec = Some(spec.clone());
        }
        match decode_track(shared, token, &spec, produced) {
            Some(0) => {
                set_decode_error(
                    shared,
                    token,
                    PlayerError::Decode("track contains no audio frames".into()),
                );
                return;
            }
            Some(frames) => produced += frames,
            None => return,
        }

        next_spec = match shared.desired.lock() {
            Ok(mut desired) if is_current(shared, token) => {
                desired.decoding_start = produced;
                let next = take_next_spec(&mut desired);
                desired.decoding_spec = next.clone();
                next
            }
            _ => None,
        };
    }

    let _desired = shared.desired.lock().expect("desired lock");
    if is_current(shared, token) {
        shared.decoder_done.store(true, Ordering::Relaxed);
    }
}

/// Returns the number of device frames pushed, or `None` when playback was
/// superseded or failed.
fn decode_track(
    shared: &Arc<Shared>,
    token: DecodeToken,
    spec: &StreamSpec,
    start_frame: u64,
) -> Option<u64> {
    let source = match open_spec(spec, shared, token) {
        Ok(source) => source,
        Err(error) => {
            set_decode_error(shared, token, error);
            return None;
        }
    };

    let mut decoder = match Decoder::open(source, spec.hint.as_deref()) {
        Ok(decoder) => decoder,
        Err(error) => {
            set_decode_error(shared, token, error);
            return None;
        }
    };

    if let Some(position_ms) = take_seek(shared, token)
        && let Err(error) = decoder.seek_ms(position_ms)
    {
        set_decode_error(shared, token, error);
        return None;
    }
    if !is_current(shared, token) {
        return None;
    }
    let base_ms = shared.position_ms.load(Ordering::Relaxed);
    let duration_ms = spec.duration_ms.or(decoder.duration_ms()).unwrap_or(0);

    if let Ok(mut boundaries) = shared.boundaries.lock() {
        if !is_current(shared, token) {
            return None;
        }
        boundaries.push_back(Boundary {
            spec: spec.clone(),
            start_frame,
            base_ms: if start_frame == 0 { base_ms } else { 0 },
            duration_ms,
        });
    }

    let source_rate = decoder.sample_rate();
    let source_channels = decoder.channels() as usize;
    let device_rate = shared.rate();
    let device_channels = shared.channels();

    let mut resampler = LinearResampler::default();
    let mut chunk = Vec::new();
    let mut converted = Vec::new();
    let mut output = Vec::new();
    let mut pushed_frames = 0u64;

    loop {
        if !is_current(shared, token) {
            return None;
        }

        let has_more = match decoder.next_chunk(&mut chunk) {
            Ok(has_more) => has_more,
            Err(error) => {
                set_decode_error(shared, token, error);
                return None;
            }
        };
        if !has_more {
            break;
        }

        convert_channels(&chunk, source_channels, device_channels, &mut converted);
        resample_linear(
            &converted,
            source_rate,
            device_rate,
            device_channels,
            &mut resampler,
            &mut output,
        );

        if !push_samples(shared, token, &output) {
            return None;
        }

        pushed_frames += (output.len() / device_channels) as u64;
    }

    Some(pushed_frames)
}

fn open_spec(
    spec: &StreamSpec,
    shared: &Arc<Shared>,
    token: DecodeToken,
) -> Result<Box<dyn symphonia::core::io::MediaSource>, PlayerError> {
    if spec.url.starts_with("http://") || spec.url.starts_with("https://") {
        let shared = Arc::clone(shared);
        source::open_url_cancellable(&spec.url, Arc::new(move || !is_current(&shared, token)))
    } else {
        source::open_file(std::path::Path::new(&spec.url))
    }
}

fn is_current(shared: &Shared, token: DecodeToken) -> bool {
    if shared.shutdown.load(Ordering::Relaxed) {
        return false;
    }
    shared.generation.load(Ordering::Acquire) == token.generation
        && shared.revision.load(Ordering::Acquire) == token.revision
}

fn take_seek(shared: &Shared, token: DecodeToken) -> Option<u64> {
    let mut desired = shared.desired.lock().ok()?;
    if !is_current(shared, token) {
        return None;
    }
    desired.seek.take()
}

fn push_samples(shared: &Shared, token: DecodeToken, samples: &[f32]) -> bool {
    if samples.is_empty() {
        return true;
    }

    let capacity = shared.rate() as usize * shared.channels() * QUEUE_SECONDS;

    loop {
        if !is_current(shared, token) {
            return false;
        }

        match shared.queue.lock() {
            Ok(mut queue) => {
                if !is_current(shared, token) {
                    return false;
                }
                if queue.len() + samples.len() <= capacity || queue.is_empty() {
                    queue.extend(samples.iter().copied());
                    return true;
                }
            }
            Err(_) => return false,
        }

        thread::sleep(Duration::from_millis(5));
    }
}

fn monitor_loop(shared: Arc<Shared>) {
    while !shared.shutdown.load(Ordering::Relaxed) {
        update_position(&shared);
        thread::sleep(Duration::from_millis(50));
    }
}

fn set_decode_error(shared: &Shared, token: DecodeToken, error: PlayerError) {
    let _desired = shared.desired.lock().expect("desired lock");
    if is_current(shared, token) {
        shared.set_error(error);
    }
}

fn advance_boundaries(shared: &Shared, boundaries: &mut VecDeque<Boundary>) {
    let consumed = shared.consumed_frames.load(Ordering::Relaxed);
    while boundaries.len() > 1 && boundaries[1].start_frame <= consumed {
        boundaries.pop_front();
        shared.track_index.fetch_add(1, Ordering::Relaxed);
    }
}

fn update_position(shared: &Shared) {
    let rate = u64::from(shared.rate());
    if let Ok(mut boundaries) = shared.boundaries.lock() {
        advance_boundaries(shared, &mut boundaries);
        let consumed = shared.consumed_frames.load(Ordering::Relaxed);

        if let Some(current) = boundaries.front() {
            let played = consumed.saturating_sub(current.start_frame);
            let position = current.base_ms + played.saturating_mul(1000) / rate;
            let position = if current.duration_ms > 0 {
                position.min(current.duration_ms)
            } else {
                position
            };
            shared.position_ms.store(position, Ordering::Relaxed);
            shared
                .duration_ms
                .store(current.duration_ms, Ordering::Relaxed);
        }
    }
    // Only the output callback decides when playback has drained. Position
    // polling must not hold the PCM queue lock and cause silent callbacks.
}

#[derive(Default)]
struct LinearResampler {
    // Source position in units of 1 / target_rate of a source frame. Integer
    // phase avoids adding a device frame at exact track boundaries through
    // cumulative floating-point rounding.
    position: u64,
}

/// Converts interleaved channels to the device channel count.
fn convert_channels(input: &[f32], source: usize, target: usize, out: &mut Vec<f32>) {
    out.clear();
    if source == 0 || input.is_empty() {
        return;
    }

    let frames = input.len() / source;

    if source == target {
        out.extend_from_slice(input);
        return;
    }

    if source == 1 {
        for &sample in input.iter().take(frames) {
            for _ in 0..target {
                out.push(sample);
            }
        }
        return;
    }

    if target == 1 {
        for frame in 0..frames {
            let sum: f32 = input[frame * source..(frame + 1) * source].iter().sum();
            out.push(sum / source as f32);
        }
        return;
    }

    for frame in 0..frames {
        for channel in 0..target {
            out.push(input[frame * source + channel.min(source - 1)]);
        }
    }
}

/// Linear-interpolation resampler.
///
/// The device rate is usually 48 kHz while the library is 44.1 kHz, so some
/// conversion is unavoidable. Linear interpolation is a first-iteration
/// tradeoff; a windowed-sinc resampler can replace this without touching the
/// engine API.
fn resample_linear(
    input: &[f32],
    source_rate: u32,
    target_rate: u32,
    channels: usize,
    state: &mut LinearResampler,
    out: &mut Vec<f32>,
) {
    out.clear();
    if source_rate == target_rate || input.is_empty() {
        out.extend_from_slice(input);
        return;
    }

    let frames = input.len() / channels;
    if frames == 0 {
        return;
    }

    let limit = frames as u64 * u64::from(target_rate);
    let mut position = state.position;

    while position < limit {
        let index = (position / u64::from(target_rate)) as usize;
        let fraction = (position % u64::from(target_rate)) as f32 / target_rate as f32;
        let next = (index + 1).min(frames - 1);

        for channel in 0..channels {
            let a = input[index * channels + channel];
            let b = input[next * channels + channel];
            out.push(a + (b - a) * fraction);
        }

        position += u64::from(source_rate);
    }

    state.position = position - limit;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mono_is_duplicated_to_stereo() {
        let mut out = Vec::new();
        convert_channels(&[0.25, -0.5], 1, 2, &mut out);
        assert_eq!(out, vec![0.25, 0.25, -0.5, -0.5]);
    }

    #[test]
    fn stereo_is_averaged_to_mono() {
        let mut out = Vec::new();
        convert_channels(&[0.5, -0.5, 0.25, 0.75], 2, 1, &mut out);
        assert_eq!(out.len(), 2);
        assert!((out[0] - 0.0).abs() < f32::EPSILON);
        assert!((out[1] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn resampling_produces_the_expected_frame_count() {
        let input = vec![0.0f32; 44_100 * 2];
        let mut state = LinearResampler::default();
        let mut out = Vec::new();
        resample_linear(&input, 44_100, 48_000, 2, &mut state, &mut out);

        let frames = out.len() / 2;
        assert!((frames as i64 - 48_000).abs() <= 2, "got {frames} frames");
    }

    #[test]
    fn resampling_is_a_noop_at_the_same_rate() {
        let input = vec![0.1, 0.2, 0.3, 0.4];
        let mut state = LinearResampler::default();
        let mut out = Vec::new();
        resample_linear(&input, 48_000, 48_000, 2, &mut state, &mut out);
        assert_eq!(out, input);
    }

    #[test]
    fn chunked_resampling_has_exact_duration_across_track_boundaries() {
        for target_rate in [32_000, 48_000, 96_000] {
            for total in [22_050usize, 44_100, 88_200] {
                let mut state = LinearResampler::default();
                let mut out = Vec::new();
                let mut produced = 0;
                for chunk in vec![0.0; total].chunks(1152) {
                    resample_linear(chunk, 44_100, target_rate, 1, &mut state, &mut out);
                    produced += out.len();
                }
                assert_eq!(produced, total * target_rate as usize / 44_100);
                assert_eq!(state.position, 0);
            }
        }
    }

    #[test]
    fn decoder_reads_bundled_format_fixtures() {
        for extension in ["wav", "flac", "mp3", "m4a", "ogg"] {
            check_fixture(extension);
        }
    }

    fn fixture(extension: &str) -> String {
        format!(
            "{}/tests/fixtures/tone.{extension}",
            env!("CARGO_MANIFEST_DIR")
        )
    }

    fn check_fixture(extension: &str) {
        let path = fixture(extension);
        let hint = std::path::Path::new(&path)
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_owned);
        let source = crate::source::open_file(std::path::Path::new(&path)).unwrap();
        let mut decoder = Decoder::open(source, hint.as_deref()).unwrap();

        assert!(decoder.sample_rate() > 0);
        assert!(decoder.channels() > 0);

        let mut chunk = Vec::new();
        let mut samples = 0usize;
        while decoder.next_chunk(&mut chunk).unwrap() {
            samples += chunk.len();
        }
        let frames = samples / decoder.channels() as usize;
        if matches!(extension, "wav" | "flac" | "mp3") {
            assert_eq!(
                frames, 44_100,
                "{extension}: no duplicate delay/padding trim"
            );
        }
        assert!(
            (frames as i64 - 44_100).abs() < 4096,
            "{extension}: {frames} frames"
        );
    }

    #[test]
    fn decoder_seeks_discard_preroll_and_keep_the_exact_remaining_audio() {
        for extension in ["wav", "flac", "mp3"] {
            let path = fixture(extension);
            let source = crate::source::open_file(std::path::Path::new(&path)).unwrap();
            let mut decoder = Decoder::open(source, Some(extension)).unwrap();
            let mut original = Vec::new();
            let mut chunk = Vec::new();
            while decoder.next_chunk(&mut chunk).unwrap() {
                original.extend_from_slice(&chunk);
            }
            for position in [123, 500, 901, 0] {
                // The engine opens a fresh decoder for each seek generation.
                let source = crate::source::open_file(std::path::Path::new(&path)).unwrap();
                let mut decoder = Decoder::open(source, Some(extension)).unwrap();
                decoder.seek_ms(position).unwrap();
                let mut remaining = Vec::new();
                while decoder
                    .next_chunk(&mut chunk)
                    .unwrap_or_else(|error| panic!("{extension} at {position}: {error}"))
                {
                    remaining.extend_from_slice(&chunk);
                }
                let skip = (position * 44_100 / 1000) as usize * decoder.channels() as usize;
                assert_eq!(
                    remaining.len(),
                    original.len() - skip,
                    "{extension} at {position}"
                );
                if extension != "mp3" {
                    assert_eq!(remaining, original[skip..], "{extension} at {position}");
                }
            }
        }
    }

    #[test]
    fn gapless_output_keeps_every_sample_and_advances_track() {
        let shared = Arc::new(Shared::new(44_100, 2));
        shared.playing.store(true, Ordering::Relaxed);
        shared
            .desired
            .lock()
            .unwrap()
            .upcoming
            .push_back(StreamSpec {
                url: fixture("mp3"),
                hint: Some("mp3".into()),
                duration_ms: Some(1000),
            });
        // Both one-second tracks fit in the three-second output buffer.
        decode_playlist(
            &shared,
            DecodeToken {
                generation: 0,
                revision: 0,
            },
            DecodeTask {
                start_frame: 0,
                spec: StreamSpec {
                    url: fixture("flac"),
                    hint: Some("flac".into()),
                    duration_ms: Some(1000),
                },
            },
        );
        let mut samples = Vec::new();
        while !shared.decoder_done.load(Ordering::Relaxed)
            || !shared.queue.lock().unwrap().is_empty()
        {
            let len = shared.queue.lock().unwrap().len().min(4096);
            if len == 0 {
                thread::yield_now();
                continue;
            }
            let mut out = vec![0.0f32; len - len % 2];
            fill(&mut out, &shared, 2);
            samples.extend(out);
            update_position(&shared);
        }
        assert!(shared.error.lock().unwrap().is_none());
        assert_eq!(shared.track_index.load(Ordering::Relaxed), 1);
        assert_eq!(shared.underruns.load(Ordering::Relaxed), 0);
        assert_eq!(samples.len(), 44_100 * 2 * 2);
        let boundary = 44_100 * 2;
        let rms = |values: &[f32]| {
            (values.iter().map(|value| value * value).sum::<f32>() / values.len() as f32).sqrt()
        };
        assert!(
            rms(&samples[boundary - 882..boundary + 882]) > 0.02,
            "silence at boundary"
        );
    }

    fn spec(extension: &str) -> StreamSpec {
        StreamSpec {
            url: fixture(extension),
            hint: Some(extension.into()),
            duration_ms: Some(1000),
        }
    }

    // Uses the real decoder and audio callback, without opening an OS device.
    fn prepared_engine(extensions: &[&str]) -> Engine {
        let engine = Engine {
            shared: Arc::new(Shared::new(44_100, 2)),
        };
        engine.play_queue(
            spec(extensions[0]),
            extensions[1..].iter().map(|ext| spec(ext)).collect(),
        );
        decode_selected(&engine);
        engine
    }

    #[test]
    fn output_failure_freezes_audio_and_requires_a_new_output() {
        let engine = prepared_engine(&["wav", "flac"]);
        let mut initial = vec![0.0f32; 44_100];
        fill(&mut initial, &engine.shared, 2);
        assert_eq!(engine.snapshot().position_ms, 500);
        let queued = engine.shared.queue.lock().unwrap().len();
        engine
            .shared
            .set_output_error(PlayerError::Stream("device disconnected".into()));
        let snapshot = engine.snapshot();
        assert!(snapshot.output_failed);
        assert!(!snapshot.playing);
        assert!(engine.take_error().unwrap().contains("device disconnected"));
        engine.resume();
        let mut silent = vec![1.0f32; 1024];
        fill(&mut silent, &engine.shared, 2);
        assert!(silent.iter().all(|sample| *sample == 0.0));
        assert_eq!(engine.shared.queue.lock().unwrap().len(), queued);
        assert_eq!(engine.snapshot().position_ms, 500);
        engine.play_queue(spec("mp3"), vec![]);
        assert!(
            !engine.snapshot().playing,
            "a dead output must not report playback after a new request"
        );
    }

    #[test]
    fn callback_returns_silence_without_waiting_for_a_busy_producer() {
        let engine = prepared_engine(&["wav"]);
        let queue = engine.shared.queue.lock().unwrap();
        let original_samples = queue.len();
        let shared = Arc::clone(&engine.shared);
        let (sender, receiver) = std::sync::mpsc::channel();
        let callback = thread::spawn(move || {
            let mut output = [1.0f32; 1024];
            fill(&mut output, &shared, 2);
            sender.send(output).unwrap();
        });
        // A slow producer must not block the device thread. It must also leave
        // queued PCM intact so a later callback can recover without sample loss.
        let result = receiver.recv_timeout(Duration::from_millis(100));
        assert_eq!(queue.len(), original_samples);
        drop(queue);
        callback.join().unwrap();
        assert!(result.unwrap().iter().all(|sample| *sample == 0.0));
        assert_eq!(engine.snapshot().position_ms, 0);
        assert!(
            consume(&engine, 512)
                .iter()
                .any(|sample| sample.abs() > 0.01)
        );
    }

    #[test]
    fn restoring_on_a_different_output_format_keeps_checkpoint_and_upcoming() {
        let restored = Engine {
            shared: Arc::new(Shared::new(48_000, 1)),
        };
        restored.restore_playlist(spec("wav"), vec![spec("flac")], vec![], 500);
        decode_selected(&restored);
        assert_eq!(
            restored.shared.boundaries.lock().unwrap()[1].start_frame,
            24_000,
            "restored half-second must have exactly 24,000 device frames"
        );
        assert!(!restored.snapshot().playing);
        assert_eq!(restored.snapshot().position_ms, 500);
        let mut paused = [1.0f32; 1000];
        fill(&mut paused, &restored.shared, 1);
        assert!(paused.iter().all(|sample| *sample == 0.0));
        assert_eq!(restored.snapshot().position_ms, 500);
        restored.resume();
        let mut remainder = vec![0.0f32; 24_000];
        fill(&mut remainder, &restored.shared, 1);
        let snapshot = restored.snapshot();
        assert_eq!(snapshot.track_index, 1);
        assert_eq!(snapshot.position_ms, 0);
        let mut next = vec![0.0f32; 48_000];
        fill(&mut next, &restored.shared, 1);
        let snapshot = restored.snapshot();
        assert!(snapshot.ended);
        assert_eq!(snapshot.position_ms, 1000);
        assert_eq!(snapshot.underruns, 0);
    }

    fn decode_selected(engine: &Engine) {
        let desired = engine.shared.desired.lock().unwrap();
        let token = DecodeToken {
            generation: desired.generation,
            revision: engine.shared.revision.load(Ordering::Acquire),
        };
        let task = DecodeTask {
            spec: desired.current.clone().unwrap(),
            start_frame: 0,
        };
        drop(desired);
        decode_playlist(&engine.shared, token, task);
    }

    fn decode_pending(engine: &Engine) {
        let mut desired = engine.shared.desired.lock().unwrap();
        let token = DecodeToken {
            generation: desired.generation,
            revision: engine.shared.revision.load(Ordering::Acquire),
        };
        let task = desired.resume.take();
        drop(desired);
        if let Some(task) = task {
            decode_playlist(&engine.shared, token, task);
        }
    }

    fn consume(engine: &Engine, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0; frames * 2];
        fill(&mut out, &engine.shared, 2);
        update_position(&engine.shared);
        out
    }

    #[test]
    fn replacing_prefetched_queue_keeps_current_samples_and_pause_state() {
        let engine = prepared_engine(&["flac", "wav", "mp3"]);
        consume(&engine, 11_025);
        engine.pause();
        let remaining: Vec<_> = engine
            .shared
            .queue
            .lock()
            .unwrap()
            .iter()
            .take(33_075 * 2)
            .copied()
            .collect();
        engine.replace_upcoming(vec![spec("mp3")]);
        decode_pending(&engine);
        assert!(!engine.snapshot().playing);
        let queue = engine.shared.queue.lock().unwrap();
        assert_eq!(queue.len(), (33_075 + 44_100) * 2);
        assert_eq!(
            queue
                .iter()
                .take(remaining.len())
                .copied()
                .collect::<Vec<_>>(),
            remaining
        );
        drop(queue);
        assert!(consume(&engine, 100).iter().all(|sample| *sample == 0.0));
        assert_eq!(
            engine.shared.consumed_frames.load(Ordering::Relaxed),
            11_025
        );
        engine.resume();
        consume(&engine, 33_075 + 44_100);
        let snapshot = engine.snapshot();
        assert!(snapshot.ended);
        assert_eq!(snapshot.track_index, 1);
        assert_eq!(snapshot.underruns, 0);
    }

    #[test]
    fn clearing_prefetched_tracks_preserves_every_remaining_current_sample() {
        let engine = prepared_engine(&["flac", "mp3"]);
        consume(&engine, 22_050);
        let remaining: Vec<_> = engine
            .shared
            .queue
            .lock()
            .unwrap()
            .iter()
            .take(22_050 * 2)
            .copied()
            .collect();
        engine.replace_upcoming(vec![]);
        let output = consume(&engine, 22_050);
        assert!(
            output
                .iter()
                .zip(&remaining)
                .all(|(out, raw)| *out == *raw * engine.volume())
        );
        assert!(engine.snapshot().ended);
        assert_eq!(engine.snapshot().track_index, 0);
        assert_eq!(engine.snapshot().underruns, 0);
    }

    #[test]
    fn appending_after_decoder_and_output_finish_starts_new_audio() {
        let engine = prepared_engine(&["flac"]);
        consume(&engine, 44_100);
        assert!(engine.snapshot().ended);
        engine.replace_upcoming(vec![spec("wav")]);
        assert!(!engine.snapshot().ended);
        decode_pending(&engine);
        let output = consume(&engine, 44_100);
        assert!(output.iter().any(|value| value.abs() > 0.05));
        assert!(engine.snapshot().ended);
        assert_eq!(engine.snapshot().track_index, 1);
    }

    #[test]
    fn edits_during_future_open_cancel_stale_errors_and_keep_latest_order() {
        let engine = prepared_engine(&["flac"]);
        let stale = DecodeToken {
            generation: engine.shared.generation.load(Ordering::Acquire),
            revision: engine.shared.revision.load(Ordering::Acquire),
        };
        {
            let mut desired = engine.shared.desired.lock().unwrap();
            desired.decoding_start = 44_100;
            desired.decoding_spec = Some(spec("missing"));
        }
        engine.shared.decoder_done.store(false, Ordering::Relaxed);
        engine.replace_upcoming(vec![spec("mp3")]);
        engine.replace_upcoming(vec![spec("wav")]);
        assert!(decode_track(&engine.shared, stale, &spec("missing"), 44_100).is_none());
        assert!(engine.take_error().is_none());
        assert!(engine.is_playing());
        decode_pending(&engine);
        let boundaries = engine.shared.boundaries.lock().unwrap();
        assert_eq!(boundaries.len(), 2);
        assert_eq!(boundaries[1].spec.hint.as_deref(), Some("wav"));
    }

    #[test]
    fn seek_before_revised_prefetch_starts_keeps_the_new_song() {
        let engine = prepared_engine(&["flac", "mp3"]);
        engine.replace_upcoming(vec![spec("wav")]);
        engine.seek(500);
        decode_selected(&engine);
        let boundaries = engine.shared.boundaries.lock().unwrap();
        assert_eq!(boundaries.len(), 2);
        assert_eq!(boundaries[1].spec.hint.as_deref(), Some("wav"));
    }

    #[test]
    fn seeking_during_initial_open_does_not_append_current_song_twice() {
        let engine = Engine {
            shared: Arc::new(Shared::new(44_100, 2)),
        };
        engine.play_queue(spec("flac"), vec![spec("wav")]);
        engine.shared.desired.lock().unwrap().decoding_spec = Some(spec("flac"));
        engine.seek(500);
        decode_selected(&engine);
        let boundaries = engine.shared.boundaries.lock().unwrap();
        assert_eq!(boundaries.len(), 2);
        assert_eq!(boundaries[0].spec.hint.as_deref(), Some("flac"));
        assert_eq!(boundaries[1].spec.hint.as_deref(), Some("wav"));
    }

    #[test]
    fn seek_targets_audible_track_after_transition_and_keeps_upcoming() {
        let engine = prepared_engine(&["flac", "mp3", "wav"]);
        consume(&engine, 55_125);
        assert_eq!(engine.snapshot().track_index, 1);
        engine.pause();
        engine.seek(500);
        decode_selected(&engine);
        assert!(engine.take_error().is_none());
        assert!(!engine.snapshot().playing);
        assert_eq!(engine.snapshot().position_ms, 500);
        let boundaries = engine.shared.boundaries.lock().unwrap();
        assert_eq!(boundaries.len(), 2);
        assert_eq!(boundaries[0].spec.hint.as_deref(), Some("mp3"));
        assert_eq!(boundaries[1].spec.hint.as_deref(), Some("wav"));
        drop(boundaries);
        engine.replace_upcoming(vec![]);
        engine.resume();
        let frames = engine.shared.queue.lock().unwrap().len() / 2;
        assert!(frames > 0 && frames < 44_100);
        consume(&engine, frames);
        assert_eq!(engine.snapshot().track_index, 1);
        assert!(engine.snapshot().ended);
    }

    #[test]
    fn worker_plays_four_tracks_across_bounded_buffer_without_losing_samples() {
        let engine = Engine {
            shared: Arc::new(Shared::new(44_100, 2)),
        };
        engine.play_queue(spec("flac"), vec![spec("mp3"), spec("wav"), spec("flac")]);
        let shared = Arc::clone(&engine.shared);
        let worker = thread::spawn(move || decode_loop(shared));
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let available = engine.shared.queue.lock().unwrap().len() / 2;
            if available > 0 {
                consume(&engine, available.min(2048));
            }
            if engine.snapshot().ended {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "decoder stalled");
            if available == 0 {
                thread::sleep(Duration::from_millis(1));
            }
        }
        assert!(engine.take_error().is_none());
        // try_lock contention can emit silence, but must never consume or lose
        // PCM. Count the actual device frames, excluding those silent calls.
        assert_eq!(
            engine.shared.consumed_frames.load(Ordering::Relaxed),
            44_100 * 4
        );
        assert_eq!(engine.snapshot().track_index, 3);
        drop(engine);
        worker.join().unwrap();
    }

    #[test]
    fn repeated_audio_crosses_cycles_and_disabling_repeat_preserves_current() {
        for single in [false, true] {
            let engine = Engine {
                shared: Arc::new(Shared::new(44_100, 2)),
            };
            let cycle = if single {
                vec![spec("flac")]
            } else {
                vec![spec("flac"), spec("mp3")]
            };
            engine.play_playlist(
                spec("flac"),
                if single { vec![] } else { vec![spec("mp3")] },
                cycle,
            );
            let shared = Arc::clone(&engine.shared);
            let worker = thread::spawn(move || decode_loop(shared));
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while engine.shared.consumed_frames.load(Ordering::Relaxed) < 44_100 * 4 + 11_025 {
                let available = engine.shared.queue.lock().unwrap().len() / 2;
                if available > 0 {
                    consume(&engine, available.min(1024));
                }
                assert!(!engine.snapshot().ended);
                assert!(std::time::Instant::now() < deadline);
                if available == 0 {
                    thread::sleep(Duration::from_millis(1));
                }
            }
            engine.pause();
            assert!(engine.snapshot().track_index >= 4);
            let before = engine.snapshot().position_ms;
            engine.replace_playlist(vec![], vec![]);
            assert_eq!(engine.snapshot().position_ms, before);
            assert!(!engine.snapshot().playing);
            assert_eq!(engine.shared.boundaries.lock().unwrap().len(), 1);
            engine.resume();
            while !engine.snapshot().ended {
                let available = engine.shared.queue.lock().unwrap().len() / 2;
                if available > 0 {
                    consume(&engine, available.min(1024));
                }
                assert!(std::time::Instant::now() < deadline);
                if available == 0 {
                    thread::sleep(Duration::from_millis(1));
                }
            }
            assert!(engine.snapshot().ended);
            assert_eq!(
                engine.shared.consumed_frames.load(Ordering::Relaxed),
                44_100 * 5
            );
            assert!(engine.take_error().is_none());
            drop(engine);
            worker.join().unwrap();
        }
    }
}
