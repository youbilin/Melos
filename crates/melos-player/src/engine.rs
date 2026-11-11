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
}

#[derive(Debug, Clone, Copy)]
struct Boundary {
    start_frame: u64,
    base_ms: u64,
    duration_ms: u64,
}

struct Desired {
    generation: u64,
    current: Option<StreamSpec>,
    next: Option<StreamSpec>,
    seek: Option<u64>,
}

struct Shared {
    queue: Mutex<VecDeque<f32>>,
    desired: Mutex<Desired>,
    wake: Condvar,
    boundaries: Mutex<VecDeque<Boundary>>,
    error: Mutex<Option<String>>,
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
}

impl Shared {
    fn new(rate: u32, channels: u16) -> Self {
        Self {
            queue: Mutex::new(VecDeque::new()),
            desired: Mutex::new(Desired {
                generation: 0,
                current: None,
                next: None,
                seek: None,
            }),
            wake: Condvar::new(),
            boundaries: Mutex::new(VecDeque::new()),
            error: Mutex::new(None),
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
        }
    }

    fn set_error(&self, error: PlayerError) {
        *self.error.lock().expect("error lock") = Some(error.to_string());
        self.playing.store(false, Ordering::Relaxed);
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
        let device = host
            .default_output_device()
            .ok_or(PlayerError::NoDevice)?;
        let supported = device
            .default_output_config()
            .map_err(|e| PlayerError::Config(e.to_string()))?;

        let config = supported.config();
        let sample_format = supported.sample_format();

        let shared = Arc::new(Shared::new(config.sample_rate, config.channels));

        {
            let shared = Arc::clone(&shared);
            thread::spawn(move || audio_loop(shared, config, sample_format));
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
        if let Ok(mut queue) = self.shared.queue.lock() {
            queue.clear();
        }
        if let Ok(mut boundaries) = self.shared.boundaries.lock() {
            boundaries.clear();
        }

        self.shared.consumed_frames.store(0, Ordering::Relaxed);
        self.shared.position_ms.store(0, Ordering::Relaxed);
        self.shared.duration_ms.store(0, Ordering::Relaxed);
        self.shared.track_index.store(0, Ordering::Relaxed);
        self.shared.ended.store(false, Ordering::Relaxed);
        self.shared.decoder_done.store(false, Ordering::Relaxed);
        self.shared.playing.store(true, Ordering::Relaxed);

        if let Ok(mut desired) = self.shared.desired.lock() {
            desired.generation = desired.generation.wrapping_add(1);
            desired.current = Some(current);
            desired.next = next;
            desired.seek = None;
        }
        self.shared.wake.notify_all();
    }

    /// Seeks within the current track.
    pub fn seek(&self, position_ms: u64) {
        if let Ok(mut desired) = self.shared.desired.lock() {
            if desired.current.is_some() {
                desired.seek = Some(position_ms);
            }
        }
        self.shared.wake.notify_all();
    }

    /// Sets the track that should follow the current one. Takes effect if the
    /// decoder has not reached the end of the current track yet.
    pub fn set_next(&self, next: StreamSpec) {
        if let Ok(mut desired) = self.shared.desired.lock() {
            desired.next = Some(next);
        }
        self.shared.wake.notify_all();
    }

    pub fn pause(&self) {
        self.shared.playing.store(false, Ordering::Relaxed);
    }

    pub fn resume(&self) {
        self.shared.playing.store(true, Ordering::Relaxed);
    }

    pub fn stop(&self) {
        self.shared.playing.store(false, Ordering::Relaxed);
        if let Ok(mut queue) = self.shared.queue.lock() {
            queue.clear();
        }
        if let Ok(mut desired) = self.shared.desired.lock() {
            desired.generation = desired.generation.wrapping_add(1);
            desired.current = None;
            desired.next = None;
            desired.seek = None;
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
        EngineSnapshot {
            position_ms: self.shared.position_ms.load(Ordering::Relaxed),
            duration_ms: self.shared.duration_ms.load(Ordering::Relaxed),
            playing: self.shared.playing.load(Ordering::Relaxed),
            ended: self.shared.ended.load(Ordering::Relaxed),
            track_index: self.shared.track_index.load(Ordering::Relaxed),
            underruns: self.shared.underruns.load(Ordering::Relaxed),
            volume: self.volume(),
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
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.shared.shutdown.store(true, Ordering::Relaxed);
        self.shared.wake.notify_all();
    }
}

fn audio_loop(shared: Arc<Shared>, config: StreamConfig, sample_format: SampleFormat) {
    let host = cpal::default_host();
    let Some(device) = host.default_output_device() else {
        shared.set_error(PlayerError::NoDevice);
        return;
    };

    match sample_format {
        SampleFormat::F32 => build::<f32>(&device, &config, &shared),
        SampleFormat::I16 => build::<i16>(&device, &config, &shared),
        SampleFormat::I32 => build::<i32>(&device, &config, &shared),
        SampleFormat::U16 => build::<u16>(&device, &config, &shared),
        SampleFormat::F64 => build::<f64>(&device, &config, &shared),
        other => shared.set_error(PlayerError::Config(format!(
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

    let stream = device.build_output_stream(
        config.clone(),
        move |data: &mut [T], _| fill(data, &shared_cb, channels),
        |error| eprintln!("melos: audio stream error: {error}"),
        None,
    );

    let stream = match stream {
        Ok(stream) => stream,
        Err(error) => {
            shared.set_error(PlayerError::Stream(error.to_string()));
            return;
        }
    };

    if let Err(error) = stream.play() {
        shared.set_error(PlayerError::Stream(error.to_string()));
        return;
    }

    while !shared.shutdown.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_millis(50));
    }
}

fn fill<T>(data: &mut [T], shared: &Shared, channels: usize)
where
    T: SizedSample + FromSample<f32>,
{
    let mut queue = match shared.queue.try_lock() {
        Ok(queue) => queue,
        Err(_) => {
            for sample in data.iter_mut() {
                *sample = T::from_sample(0.0);
            }
            return;
        }
    };

    if !shared.playing.load(Ordering::Relaxed) {
        for sample in data.iter_mut() {
            *sample = T::from_sample(0.0);
        }
        return;
    }

    let volume = f32::from_bits(shared.volume_bits.load(Ordering::Relaxed));
    let mut consumed = 0usize;

    for sample in data.iter_mut() {
        match queue.pop_front() {
            Some(value) => {
                *sample = T::from_sample(value * volume);
                consumed += 1;
            }
            None => {
                *sample = T::from_sample(0.0);
                shared.underruns.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    drop(queue);
    shared
        .consumed_frames
        .fetch_add((consumed / channels) as u64, Ordering::Relaxed);
}

fn decode_loop(shared: Arc<Shared>) {
    let mut last_generation = u64::MAX;

    loop {
        if shared.shutdown.load(Ordering::Relaxed) {
            break;
        }

        let (generation, current) = match shared.desired.lock() {
            Ok(desired) => (desired.generation, desired.current.clone()),
            Err(_) => break,
        };

        if let Some(current) = current {
            if generation != last_generation {
                last_generation = generation;
                decode_playlist(&shared, generation, current);
            }
        }

        if let Ok(desired) = shared.desired.lock() {
            let _ = shared
                .wake
                .wait_timeout(desired, Duration::from_millis(50));
        }
    }
}

fn decode_playlist(shared: &Arc<Shared>, generation: u64, first: StreamSpec) {
    let mut next_spec = Some(first);
    let mut produced = 0u64;

    while let Some(spec) = next_spec {
        if !is_current(shared, generation) {
            return;
        }
        match decode_track(shared, generation, &spec, produced) {
            Some(frames) => produced += frames,
            None => return,
        }

        next_spec = match shared.desired.lock() {
            Ok(mut desired) if desired.generation == generation => desired.next.take(),
            _ => None,
        };
    }

    shared.decoder_done.store(true, Ordering::Relaxed);
}

/// Returns the number of device frames pushed, or `None` when playback was
/// superseded or failed.
fn decode_track(
    shared: &Arc<Shared>,
    generation: u64,
    spec: &StreamSpec,
    start_frame: u64,
) -> Option<u64> {
    let source = match open_spec(spec) {
        Ok(source) => source,
        Err(error) => {
            shared.set_error(error);
            return None;
        }
    };

    let mut decoder = match Decoder::open(source, spec.hint.as_deref()) {
        Ok(decoder) => decoder,
        Err(error) => {
            shared.set_error(error);
            return None;
        }
    };

    let duration_ms = spec.duration_ms.or(decoder.duration_ms()).unwrap_or(0);

    if let Ok(mut boundaries) = shared.boundaries.lock() {
        boundaries.push_back(Boundary {
            start_frame,
            base_ms: 0,
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
        if !is_current(shared, generation) {
            return None;
        }

        if let Some(position_ms) = take_seek(shared, generation) {
            if let Err(error) = decoder.seek_ms(position_ms) {
                shared.set_error(error);
                return None;
            }
            if let Ok(mut queue) = shared.queue.lock() {
                queue.clear();
            }
            shared.consumed_frames.store(0, Ordering::Relaxed);
            if let Ok(mut boundaries) = shared.boundaries.lock() {
                boundaries.clear();
                boundaries.push_back(Boundary {
                    start_frame: 0,
                    base_ms: position_ms,
                    duration_ms,
                });
            }
            resampler = LinearResampler::default();
            pushed_frames = 0;
            continue;
        }

        let has_more = match decoder.next_chunk(&mut chunk) {
            Ok(has_more) => has_more,
            Err(error) => {
                shared.set_error(error);
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

        if !push_samples(shared, generation, &output) {
            return None;
        }

        pushed_frames += (output.len() / device_channels) as u64;
    }

    Some(pushed_frames)
}

fn open_spec(spec: &StreamSpec) -> Result<Box<dyn symphonia::core::io::MediaSource>, PlayerError> {
    if spec.url.starts_with("http://") || spec.url.starts_with("https://") {
        source::open_url(&spec.url)
    } else {
        source::open_file(std::path::Path::new(&spec.url))
    }
}

fn is_current(shared: &Shared, generation: u64) -> bool {
    if shared.shutdown.load(Ordering::Relaxed) {
        return false;
    }
    shared
        .desired
        .lock()
        .map(|desired| desired.generation == generation)
        .unwrap_or(false)
}

fn take_seek(shared: &Shared, generation: u64) -> Option<u64> {
    let mut desired = shared.desired.lock().ok()?;
    if desired.generation != generation {
        return None;
    }
    desired.seek.take()
}

fn push_samples(shared: &Shared, generation: u64, samples: &[f32]) -> bool {
    if samples.is_empty() {
        return true;
    }

    let capacity = shared.rate() as usize * shared.channels() * QUEUE_SECONDS;

    loop {
        if !is_current(shared, generation) {
            return false;
        }

        match shared.queue.lock() {
            Ok(mut queue) => {
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

fn update_position(shared: &Shared) {
    let rate = u64::from(shared.rate());
    let consumed = shared.consumed_frames.load(Ordering::Relaxed);

    if let Ok(mut boundaries) = shared.boundaries.lock() {
        while boundaries.len() > 1 && boundaries[1].start_frame <= consumed {
            boundaries.pop_front();
            shared.track_index.fetch_add(1, Ordering::Relaxed);
        }

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

    let done = shared.decoder_done.load(Ordering::Relaxed);
    let empty = shared
        .queue
        .lock()
        .map(|queue| queue.is_empty())
        .unwrap_or(true);

    if done && empty && shared.playing.load(Ordering::Relaxed) {
        shared.ended.store(true, Ordering::Relaxed);
    }
}

#[derive(Default)]
struct LinearResampler {
    position: f64,
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
        for frame in 0..frames {
            let sample = input[frame];
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

    let ratio = f64::from(source_rate) / f64::from(target_rate);
    let mut position = state.position;

    while position < frames as f64 {
        let index = position.floor() as usize;
        let fraction = (position - index as f64) as f32;
        let next = (index + 1).min(frames - 1);

        for channel in 0..channels {
            let a = input[index * channels + channel];
            let b = input[next * channels + channel];
            out.push(a + (b - a) * fraction);
        }

        position += ratio;
    }

    state.position = position - frames as f64;
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
    fn decoder_reads_a_fixture_when_provided() {
        let Ok(path) = std::env::var("MELOS_TEST_AUDIO") else {
            return;
        };
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
        assert!(samples > 0);
    }

    #[test]
    fn decoder_can_seek_when_a_fixture_is_provided() {
        let Ok(path) = std::env::var("MELOS_TEST_AUDIO") else {
            return;
        };
        let hint = std::path::Path::new(&path)
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_owned);
        let source = crate::source::open_file(std::path::Path::new(&path)).unwrap();
        let mut decoder = Decoder::open(source, hint.as_deref()).unwrap();

        decoder.seek_ms(30_000).unwrap();
        let mut chunk = Vec::new();
        assert!(decoder.next_chunk(&mut chunk).unwrap());
        assert!(!chunk.is_empty());
    }
}
