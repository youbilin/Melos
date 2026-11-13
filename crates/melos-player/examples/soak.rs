//! Silent, real-device acceptance using bundled audio. No server or settings.
//! Default: 80 minutes. A shorter run is explicitly reported as a short check.
use melos_player::{Engine, StreamSpec};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn spec(extension: &str) -> StreamSpec {
    StreamSpec {
        url: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(format!("tone.{extension}"))
            .to_string_lossy()
            .into_owned(),
        hint: Some(extension.into()),
        duration_ms: Some(1000),
    }
}

fn wait_for(engine: &Engine, predicate: impl Fn() -> bool) -> Result<(), String> {
    let start = Instant::now();
    while !predicate() {
        if let Some(error) = engine.take_error() {
            return Err(error);
        }
        if start.elapsed() > Duration::from_secs(10) {
            return Err("Audio progress timed out".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

fn exercise_controls(engine: &mut Engine) -> Result<(), String> {
    for index in 0..50 {
        engine.play_queue(
            spec(if index % 2 == 0 { "flac" } else { "mp3" }),
            vec![spec("wav")],
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    engine.play_playlist(
        spec("flac"),
        vec![spec("mp3")],
        vec![spec("flac"), spec("mp3")],
    );
    wait_for(engine, || engine.snapshot().position_ms >= 100)?;
    engine.pause();
    let checkpoint = engine.snapshot().position_ms;
    for index in 0..10 {
        engine.replace_playlist(
            vec![spec(if index % 2 == 0 { "wav" } else { "mp3" })],
            vec![],
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_secs(1));
    if engine.snapshot().position_ms != checkpoint {
        return Err("Paused audio moved while editing the queue".into());
    }
    engine.seek(500);
    std::thread::sleep(Duration::from_millis(150));
    if engine.snapshot().position_ms != 500 {
        return Err("Paused seek did not retain checkpoint".into());
    }
    let reopened = Engine::new().map_err(|error| error.to_string())?;
    reopened.set_volume(0.0);
    reopened.restore_playlist(spec("flac"), vec![spec("mp3")], vec![], 500);
    *engine = reopened;
    std::thread::sleep(Duration::from_millis(250));
    if engine.snapshot().playing || engine.snapshot().position_ms != 500 {
        return Err("Output reconnection changed pause/checkpoint".into());
    }
    engine.resume();
    wait_for(engine, || engine.snapshot().track_index >= 1)?;
    Ok(())
}

fn run(seconds: u64) -> Result<Value, String> {
    let mut engine = Engine::new().map_err(|error| error.to_string())?;
    engine.set_volume(0.0);
    exercise_controls(&mut engine)?;
    engine.play_playlist(
        spec("flac"),
        vec![spec("mp3")],
        vec![spec("flac"), spec("mp3")],
    );
    wait_for(&engine, || engine.snapshot().position_ms >= 100)?;
    let baseline = engine.snapshot();
    let base_played = baseline.track_index * 1000 + baseline.position_ms;
    let start = Instant::now();
    let mut max_drift = 0i64;
    let mut samples = Vec::new();
    let mut first_private = Vec::new();
    let mut last_private = std::collections::VecDeque::new();
    let mut next_sample = 0;
    while start.elapsed() < Duration::from_secs(seconds) {
        if let Some(error) = engine.take_error() {
            return Err(error);
        }
        let snapshot = engine.snapshot();
        if snapshot.output_failed || snapshot.ended || !snapshot.playing {
            return Err("Continuous playback stopped".into());
        }
        let elapsed = start.elapsed().as_millis() as u64;
        let played =
            (snapshot.track_index * 1000 + snapshot.position_ms).saturating_sub(base_played);
        let drift = elapsed as i64 - played as i64;
        max_drift = max_drift.max(drift.abs());
        if elapsed >= next_sample {
            let memory = memory();
            if let Some((_, private)) = memory
                && elapsed >= 10_000
            {
                if elapsed < 70_000 {
                    first_private.push(private);
                }
                last_private.push_back(private);
                if last_private.len() > 12 {
                    last_private.pop_front();
                }
            }
            samples.push(json!({"elapsed_ms":elapsed,"played_ms":played,"drift_ms":drift,"track_index":snapshot.track_index,"underruns":snapshot.underruns.saturating_sub(baseline.underruns),"working_set_bytes":memory.map(|sample| sample.0),"private_bytes":memory.map(|sample| sample.1)}));
            if next_sample.is_multiple_of(30_000) {
                println!(
                    "{} / {} seconds · transitions={} · drift={} ms · underruns={}",
                    elapsed / 1000,
                    seconds,
                    snapshot.track_index,
                    drift,
                    snapshot.underruns.saturating_sub(baseline.underruns)
                );
            }
            next_sample += 5000;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let final_snapshot = engine.snapshot();
    engine.stop();
    let growth = if first_private.is_empty() || last_private.is_empty() {
        None
    } else {
        let first = first_private.iter().sum::<u64>() / first_private.len() as u64;
        let last = last_private.iter().sum::<u64>() / last_private.len() as u64;
        Some(last as i64 - first as i64)
    };
    let underruns = final_snapshot.underruns.saturating_sub(baseline.underruns);
    let passed =
        underruns == 0 && max_drift <= 250 && growth.is_some_and(|growth| growth <= 8_000_000);
    Ok(json!({
        "passed":passed,"requested_seconds":seconds,"elapsed_ms":start.elapsed().as_millis(),
        "acceptance_duration_met":seconds >= 4800,
        "scope":if seconds >= 4800 { "80-minute real-device local-fixture acceptance" } else { "short real-device local-fixture check; not 80-minute acceptance" },
        "silent":true,"rapid_switches":50,"pause_queue_edits":10,"reconnect_checkpoint_ms":500,
        "track_transitions":final_snapshot.track_index,"underruns":underruns,"max_abs_clock_drift_ms":max_drift,
        "private_memory_growth_bytes":growth,"limits":{"max_clock_drift_ms":250,"max_private_memory_growth_bytes":8_000_000},"samples":samples,
        "limitations":"Measures the player and default output using local FLAC/MP3 fixtures, not desktop UI, Navidrome or physical unplug/replug."
    }))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut seconds = 4800;
    let mut report = PathBuf::from("output/playback-soak.json");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--seconds" => seconds = args.next().ok_or("Missing seconds")?.parse::<u64>()?,
            "--report" => report = args.next().ok_or("Missing report path")?.into(),
            "--help" => {
                println!(
                    "soak [--seconds 4800] [--report output/playback-soak.json]\nSilent default-device FLAC/MP3 playback, queue/seek/reconnect checks and memory/clock samples."
                );
                return Ok(());
            }
            _ => return Err("Unknown argument; use --help".into()),
        }
    }
    if seconds < 30 {
        return Err("Use at least 30 seconds; 80-minute acceptance requires 4800.".into());
    }
    let result = match run(seconds) {
        Ok(result) => result,
        Err(error) => json!({"passed":false,"requested_seconds":seconds,"error":error}),
    };
    if let Some(parent) = report
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&report, serde_json::to_vec_pretty(&result)?)?;
    println!("Report: {}", report.display());
    if result["passed"] != true {
        return Err("Playback acceptance failed; see the report.".into());
    }
    Ok(())
}

#[cfg(windows)]
fn memory() -> Option<(u64, u64)> {
    #[repr(C)]
    struct Counters {
        cb: u32,
        faults: u32,
        values: [usize; 9],
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        #[link_name = "GetCurrentProcess"]
        fn current_process() -> *mut std::ffi::c_void;
        #[link_name = "K32GetProcessMemoryInfo"]
        fn process_memory(
            process: *mut std::ffi::c_void,
            counters: *mut Counters,
            size: u32,
        ) -> i32;
    }
    let size = std::mem::size_of::<Counters>() as u32;
    let mut counters = Counters {
        cb: size,
        faults: 0,
        values: [0; 9],
    };
    // The pseudo-handle belongs to this process; the buffer layout and cb match
    // PROCESS_MEMORY_COUNTERS_EX. No other process is inspected or modified.
    let ok = unsafe { process_memory(current_process(), &mut counters, size) };
    (ok != 0).then_some((counters.values[1] as u64, counters.values[8] as u64))
}

#[cfg(not(windows))]
fn memory() -> Option<(u64, u64)> {
    None
}
