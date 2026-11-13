//! Targeted, read-only real-server EOF diagnostics. Never records signed URLs.
use melos_api::{Client, Credentials, StreamQuality};
use melos_core::Settings;
use melos_player::{Decoder, Engine, PlayerError, StreamSpec, source};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn stage(error: PlayerError) -> &'static str {
    match error {
        PlayerError::Decode(message) if message.contains("unexpected end of file") => "decode_eof",
        PlayerError::Decode(_) => "decode_error",
        PlayerError::Network(_) | PlayerError::Io(_) => "transport_error",
        _ => "player_error",
    }
}

async fn check(query: &str) -> Result<Value, &'static str> {
    let settings = Settings::load();
    let client = Client::new(
        &settings.server,
        Credentials::new(
            &settings.username,
            settings.password().ok_or("saved_password_unavailable")?,
        ),
    )
    .map_err(|_| "invalid_saved_connection")?;
    let songs = client
        .search(query)
        .await
        .map_err(|_| "search_failed")?
        .song;
    let song = songs
        .iter()
        .find(|song| song.title == query)
        .ok_or("exact_song_not_found")?;
    let url = client.stream_url(song, StreamQuality::Original);
    let mut decoder = Decoder::open(
        source::open_url(&url).map_err(stage)?,
        song.suffix.as_deref(),
    )
    .map_err(stage)?;
    let declared_ms = decoder.duration_ms();
    let mut frames = 0u64;
    let mut chunk = Vec::new();
    while decoder.next_chunk(&mut chunk).map_err(stage)? {
        frames += (chunk.len() / usize::from(decoder.channels())) as u64;
    }
    let decoded_ms = frames * 1000 / u64::from(decoder.sample_rate());
    let engine = Engine::new().map_err(stage)?;
    engine.set_volume(0.0);
    let duration_ms = song.duration.map(|seconds| u64::from(seconds) * 1000);
    engine.restore_playlist(
        StreamSpec {
            url,
            hint: song.suffix.clone(),
            duration_ms,
        },
        vec![],
        vec![],
        decoded_ms.saturating_sub(10_000),
    );
    engine.resume();
    let start = Instant::now();
    let mut failure = None;
    while start.elapsed() < Duration::from_secs(30) {
        if engine.take_error().is_some() {
            failure = Some("tail_playback_error");
            break;
        }
        let snapshot = engine.snapshot();
        if snapshot.output_failed {
            failure = Some("output_failed");
            break;
        }
        if snapshot.ended {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let snapshot = engine.snapshot();
    engine.stop();
    Ok(
        json!({"passed":snapshot.ended && failure.is_none(),"read_only":true,"silent":true,
        "suffix":song.suffix,"server_duration_ms":duration_ms,"decoder_declared_duration_ms":declared_ms,
        "decoded_frames":frames,"decoded_duration_ms":decoded_ms,"tail_playback_ended":snapshot.ended,
        "tail_playback_elapsed_ms":start.elapsed().as_millis(),"tail_failure_stage":failure,
        "scope":"Full original-stream decode and silent final ten seconds; no scrobbling or playlist writes"}),
    )
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let query = args
        .next()
        .ok_or("track_check <exact title> [report path]")?;
    let report = PathBuf::from(
        args.next()
            .unwrap_or_else(|| "output/track-check.json".into()),
    );
    let result = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(check(&query))
        .unwrap_or_else(|stage| json!({"passed":false,"stage":stage}));
    if let Some(parent) = report.parent().filter(|path| !path.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&report, serde_json::to_vec_pretty(&result)?)?;
    println!(
        "{}",
        json!({"report":report,"passed":result["passed"],"stage":result["stage"]})
    );
    if result["passed"] != true {
        return Err("Track check failed; see the sanitized report".into());
    }
    Ok(())
}
