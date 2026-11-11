//! Phase 0 audio smoke test.
//!
//! Decodes one or more local files and plays them back to back through cpal,
//! reporting underruns at the join. If the decoder and the output pipeline are
//! sound, the boundary is sample-exact and the underrun count stays at zero.
//!
//! Usage:
//!   spike-audio <file1> [file2]
//!
//! Both files must share sample rate and channel count. The tool refuses to
//! resample for now: resampling is a Phase 1 concern.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, FromSample, SampleFormat, SizedSample, StreamConfig};
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

struct Decoded {
    samples: Vec<f32>,
    sample_rate: u32,
    channels: u16,
}

impl Decoded {
    fn frames(&self) -> usize {
        self.samples.len() / self.channels as usize
    }
}

fn decode(path: &Path) -> Result<Decoded, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("open: {e}"))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| format!("probe: {e}"))?;

    let track = format
        .default_track(TrackType::Audio)
        .ok_or_else(|| "no audio track".to_string())?;

    let params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or_else(|| "no audio codec parameters".to_string())?;

    let sample_rate = params
        .sample_rate
        .ok_or_else(|| "no sample rate in codec parameters".to_string())?;
    let channels = params
        .channels
        .as_ref()
        .map(|c| c.count())
        .unwrap_or(2) as u16;

    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(params, &AudioDecoderOptions::default())
        .map_err(|e| format!("unsupported codec: {e}"))?;

    let track_id = track.id;
    let mut samples = Vec::new();
    let mut scratch: Vec<f32> = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(SymphoniaError::ResetRequired) => break,
            Err(e) => return Err(format!("read packet: {e}")),
        };

        if packet.track_id != track_id {
            continue;
        }

        match decoder.decode(&packet) {
            Ok(buf) => {
                scratch.resize(buf.samples_interleaved(), 0.0);
                buf.copy_to_slice_interleaved(&mut scratch);
                samples.extend_from_slice(&scratch);
            }
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(SymphoniaError::IoError(_)) => continue,
            Err(e) => return Err(format!("decode: {e}")),
        }
    }

    if samples.is_empty() {
        return Err("decoded zero samples".into());
    }

    Ok(Decoded {
        samples,
        sample_rate,
        channels,
    })
}

fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f32 = samples.iter().map(|s| s * s).sum();
    (sum / samples.len() as f32).sqrt()
}

/// Prints 10 ms RMS windows around the join so a silent gap is visible in the
/// numbers, not only by ear. A clean join keeps the RMS in the same range as
/// the surrounding music.
fn report_join(pcm: &[f32], boundary: usize, channels: u16, rate: u32) {
    let ch = channels as usize;
    let window = (rate as usize / 100) * ch; // 10 ms
    let span = rate as usize / 10 * ch; // 100 ms each side
    let start = boundary.saturating_sub(span);
    let end = (boundary + span).min(pcm.len());

    println!("join analysis (10 ms RMS, boundary at 0):");
    let mut offset = start;
    while offset < end {
        let seg_end = (offset + window).min(end);
        let rel_ms = (offset as isize - boundary as isize) * 1000
            / (rate as isize * ch as isize);
        println!(
            "  {rel_ms:+5} ms  rms={:.6}",
            rms(&pcm[offset..seg_end])
        );
        offset = seg_end;
    }
}

fn pick_config(
    device: &Device,
    rate: u32,
    channels: u16,
) -> Result<(StreamConfig, SampleFormat), String> {
    let ranges = device
        .supported_output_configs()
        .map_err(|e| format!("enumerate configs: {e}"))?;

    for range in ranges {
        if range.sample_format() == SampleFormat::F32
            && range.channels() == channels
            && range.min_sample_rate() <= rate
            && rate <= range.max_sample_rate()
        {
            let supported = range.with_sample_rate(rate);
            return Ok((supported.config(), SampleFormat::F32));
        }
    }

    Err(format!(
        "device has no f32 config for {rate} Hz / {channels} ch; refusing to resample in Phase 0"
    ))
}

fn play(
    device: &Device,
    config: &StreamConfig,
    sample_format: SampleFormat,
    pcm: Arc<Vec<f32>>,
) -> Result<(), String> {
    match sample_format {
        SampleFormat::F32 => run::<f32>(device, config, pcm),
        SampleFormat::I16 => run::<i16>(device, config, pcm),
        SampleFormat::I32 => run::<i32>(device, config, pcm),
        other => Err(format!("unsupported device sample format: {other}")),
    }
}

fn run<T>(device: &Device, config: &StreamConfig, pcm: Arc<Vec<f32>>) -> Result<(), String>
where
    T: SizedSample + FromSample<f32>,
{
    let total = pcm.len();
    let pos = Arc::new(AtomicUsize::new(0));
    let underruns = Arc::new(AtomicU64::new(0));
    let done = Arc::new(AtomicBool::new(false));

    let pos_cb = pos.clone();
    let under_cb = underruns.clone();
    let done_cb = done.clone();

    let stream = device
        .build_output_stream(
            config.clone(),
            move |data: &mut [T], _| {
                let mut p = pos_cb.load(Ordering::Relaxed);
                let finished = done_cb.load(Ordering::Relaxed);
                let exhausted_at_start = p >= total;
                let mut starved = 0u64;

                for out in data.iter_mut() {
                    if p < total {
                        // Fixed 25% gain so a hot master does not blast the speakers.
                        *out = T::from_sample(pcm[p] * 0.25);
                        p += 1;
                    } else {
                        *out = T::from_sample(0.0);
                        // Only a callback that starts with no data left is a real
                        // starvation; tail padding in the final buffer is expected.
                        if exhausted_at_start && !finished {
                            starved += 1;
                        }
                    }
                }

                pos_cb.store(p, Ordering::Relaxed);
                if starved > 0 {
                    under_cb.fetch_add(starved, Ordering::Relaxed);
                }
                if p >= total {
                    done_cb.store(true, Ordering::Relaxed);
                }
            },
            |err| eprintln!("stream error: {err}"),
            None,
        )
        .map_err(|e| format!("build stream: {e}"))?;

    stream.play().map_err(|e| format!("play: {e}"))?;

    let started = Instant::now();
    while !done.load(Ordering::Relaxed) && started.elapsed() < Duration::from_secs(120) {
        std::thread::sleep(Duration::from_millis(20));
    }
    // Let the final buffer drain before dropping the stream.
    std::thread::sleep(Duration::from_millis(300));

    let elapsed = started.elapsed();
    let frames = total / config.channels as usize;
    let audio_secs = frames as f64 / config.sample_rate as f64;

    println!("--- playback report ---");
    println!("device config: {} Hz, {} ch, {:?}", config.sample_rate, config.channels, config.buffer_size);
    println!("frames: {frames}");
    println!("audio duration: {:.3}s", audio_secs);
    println!("wall clock: {:.3}s", elapsed.as_secs_f64());
    println!("drift: {:+.1} ms", (elapsed.as_secs_f64() - audio_secs) * 1000.0);
    println!("underrun samples: {}", underruns.load(Ordering::Relaxed));

    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: spike-audio <file1> [file2]");
        std::process::exit(2);
    }

    let mut decoded = Vec::new();
    for arg in &args {
        let d = match decode(Path::new(arg)) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("{arg}: {e}");
                std::process::exit(1);
            }
        };
        println!(
            "decoded {arg}: {} frames, {} Hz, {} ch, {:.2}s",
            d.frames(),
            d.sample_rate,
            d.channels,
            d.frames() as f64 / d.sample_rate as f64
        );
        decoded.push(d);
    }

    let first_rate = decoded[0].sample_rate;
    let first_channels = decoded[0].channels;
    for d in &decoded {
        if d.sample_rate != first_rate || d.channels != first_channels {
            eprintln!(
                "format mismatch: {} Hz/{} ch vs {} Hz/{} ch",
                d.sample_rate, d.channels, first_rate, first_channels
            );
            std::process::exit(1);
        }
    }

    let mut pcm = Vec::new();
    let mut boundary = None;
    for d in decoded {
        if boundary.is_none() && !pcm.is_empty() {
            boundary = Some(pcm.len() / first_channels as usize);
        }
        pcm.extend_from_slice(&d.samples);
    }

    if let Some(b) = boundary {
        println!("join at frame {b} (sample-exact by construction)");
        report_join(&pcm, b, first_channels, first_rate);
    }

    // Play a short window around the join instead of the whole material.
    let ch = first_channels as usize;
    let rate = first_rate as usize;
    let total_frames = pcm.len() / ch;
    let (start_frame, end_frame) = match boundary {
        Some(b) => (b.saturating_sub(rate * 5), (b + rate * 3).min(total_frames)),
        None => (0, (rate * 10).min(total_frames)),
    };
    println!(
        "playing frames {start_frame}..{end_frame} ({:.1}s)",
        (end_frame - start_frame) as f64 / rate as f64
    );
    let pcm = pcm[start_frame * ch..end_frame * ch].to_vec();

    let host = cpal::default_host();
    let device = match host.default_output_device() {
        Some(d) => d,
        None => {
            eprintln!("no default output device");
            std::process::exit(1);
        }
    };
    println!(
        "output device: {}",
        device.id().map(|id| id.to_string()).unwrap_or_else(|_| "<unknown>".into())
    );

    let (config, sample_format) = match pick_config(&device, first_rate, first_channels) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };

    if let Err(e) = play(&device, &config, sample_format, Arc::new(pcm)) {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
