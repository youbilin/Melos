//! Silent real-server soak: plays random library tracks for a fixed duration
//! and records transport errors, track transitions and position stalls.
use melos_api::{Client, Credentials, StreamQuality};
use melos_core::Settings;
use melos_player::{Engine, StreamSpec};
use serde_json::json;
use std::time::{Duration, Instant};

#[cfg(windows)]
fn memory_mb() -> (f64, f64) {
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows::Win32::System::Threading::GetCurrentProcess;
    unsafe {
        let mut counters = PROCESS_MEMORY_COUNTERS::default();
        let size = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        if GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, size).is_ok() {
            return (
                counters.WorkingSetSize as f64 / 1e6,
                counters.PagefileUsage as f64 / 1e6,
            );
        }
    }
    (0.0, 0.0)
}

#[cfg(not(windows))]
fn memory_mb() -> (f64, f64) {
    (0.0, 0.0)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut seconds = 4800u64;
    let mut report = "output/server-soak.json".to_owned();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--seconds" => seconds = args.next().ok_or("--seconds needs a value")?.parse()?,
            "--report" => report = args.next().ok_or("--report needs a path")?,
            other => return Err(format!("unknown argument: {other}").into()),
        }
    }

    let settings = Settings::load();
    let password = settings.password().ok_or("no saved password")?;
    let client = Client::new(
        &settings.server,
        Credentials::new(&settings.username, password),
    )?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let songs = runtime.block_on(client.random_songs(300))?;
    if songs.is_empty() {
        return Err("server returned no songs".into());
    }

    let quality = settings
        .max_bit_rate
        .map(|max_bit_rate| StreamQuality::Transcode { max_bit_rate })
        .unwrap_or(StreamQuality::Original);
    let specs: Vec<StreamSpec> = songs
        .iter()
        .map(|song| StreamSpec {
            url: client.stream_url(song, quality),
            hint: song.suffix.clone(),
            duration_ms: song.duration.map(|seconds| u64::from(seconds) * 1000),
        })
        .collect();

    let engine = Engine::new()?;
    engine.set_volume(0.0);
    engine.play_playlist(specs[0].clone(), specs[1..].to_vec(), vec![]);

    let started = Instant::now();
    let mut errors: Vec<String> = Vec::new();
    let mut transitions = 0u64;
    let mut last_index = 0u64;
    let mut last_position = 0u64;
    let mut last_change = Instant::now();
    let mut max_stall_ms = 0u64;
    let mut playing_samples = 0u64;
    let mut memory: Vec<(u64, f64, f64)> = Vec::new();
    let mut next_memory = Duration::ZERO;

    while started.elapsed() < Duration::from_secs(seconds) {
        if let Some(error) = engine.take_error() {
            errors.push(format!("{}s: {error}", started.elapsed().as_secs()));
            break;
        }
        let snapshot = engine.snapshot();
        if snapshot.track_index > last_index {
            transitions += snapshot.track_index - last_index;
            last_index = snapshot.track_index;
        }
        if started.elapsed() >= next_memory {
            let (working_set, private) = memory_mb();
            memory.push((started.elapsed().as_secs(), working_set, private));
            next_memory = started.elapsed() + Duration::from_secs(30);
        }
        if snapshot.playing {
            playing_samples += 1;
            if snapshot.position_ms != last_position {
                last_position = snapshot.position_ms;
                last_change = Instant::now();
            } else {
                let stall = last_change.elapsed().as_millis() as u64;
                max_stall_ms = max_stall_ms.max(stall);
            }
        } else {
            last_change = Instant::now();
        }
        if seconds <= 60 && snapshot.ended {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }

    let elapsed = started.elapsed().as_secs();
    let snapshot = engine.snapshot();
    engine.stop();
    let result = json!({
        "passed": errors.is_empty() && transitions > 0,
        "silent": true,
        "scope": "random library stream from the saved server; no scrobbling or writes",
        "requested_seconds": seconds,
        "elapsed_seconds": elapsed,
        "songs_in_playlist": songs.len(),
        "quality": if settings.max_bit_rate.is_some() { "transcode" } else { "original" },
        "transitions": transitions,
        "final_track_index": snapshot.track_index,
        "final_position_ms": snapshot.position_ms,
        "playing_samples": playing_samples,
        "max_stall_ms": max_stall_ms,
        "memory_samples": memory,
        "errors": errors,
    });
    if let Some(parent) = std::path::Path::new(&report).parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&report, serde_json::to_vec_pretty(&result)?)?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    if errors.is_empty() {
        Ok(())
    } else {
        Err("soak finished with errors".into())
    }
}
