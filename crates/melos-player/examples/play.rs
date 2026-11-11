//! Manual playback check: `cargo run -p melos-player --example play -- a.mp3 [b.mp3]`
//!
//! Set `MELOS_PLAY_SECS` to cap the run (default 12).

use std::path::Path;
use std::time::{Duration, Instant};

use melos_player::{Engine, StreamSpec};

fn spec(path: &str) -> StreamSpec {
    let hint = Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_owned);

    StreamSpec {
        url: path.to_owned(),
        hint,
        duration_ms: None,
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: play <file-or-url> [next]");
        std::process::exit(2);
    }

    let cap = std::env::var("MELOS_PLAY_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(12);

    let engine = Engine::new().expect("audio device");
    let volume = std::env::var("MELOS_PLAY_VOLUME")
        .ok()
        .and_then(|value| value.parse::<f32>().ok())
        .unwrap_or(0.25);
    engine.set_volume(volume);

    let current = spec(&args[0]);
    let next = args.get(1).map(|path| spec(path));
    engine.play(current, next);

    let started = Instant::now();
    loop {
        let snapshot = engine.snapshot();
        println!(
            "{:>6} ms / {:>6} ms  playing={} track={} underruns={}",
            snapshot.position_ms,
            snapshot.duration_ms,
            snapshot.playing,
            snapshot.track_index,
            snapshot.underruns,
        );

        if let Some(error) = engine.take_error() {
            eprintln!("error: {error}");
            break;
        }
        if snapshot.ended || started.elapsed() > Duration::from_secs(cap) {
            break;
        }

        std::thread::sleep(Duration::from_millis(500));
    }
}
