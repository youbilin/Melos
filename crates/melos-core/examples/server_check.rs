//! Read-only, silent checks using the desktop's saved connection. Never saves
//! settings, scrobbles, edits playlists or records credentials/stream URLs.
use melos_api::{AlbumListType, Client, Credentials, Song, StreamQuality};
use melos_core::settings::Settings;
use melos_player::{Engine, StreamSpec};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn wait(engine: &Engine, position: u64) -> bool {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(20) {
        if engine.take_error().is_some() || engine.snapshot().output_failed {
            return false;
        }
        if engine.snapshot().position_ms >= position {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

fn playback(client: &Client, song: &Song, quality: StreamQuality) -> Value {
    let suffix = song.suffix.as_deref().unwrap_or("unknown");
    let Ok(engine) = Engine::new() else {
        return json!({"suffix":suffix,"passed":false,"stage":"audio_device"});
    };
    engine.set_volume(0.0);
    let start = Instant::now();
    engine.play(
        StreamSpec {
            url: client.stream_url(song, quality),
            hint: song.suffix.clone(),
            duration_ms: song.duration.map(|duration| u64::from(duration) * 1000),
        },
        None,
    );
    let started = wait(&engine, 250);
    let startup_ms = start.elapsed().as_millis();
    engine.pause();
    std::thread::sleep(Duration::from_millis(100));
    let checkpoint = engine.snapshot().position_ms;
    std::thread::sleep(Duration::from_millis(150));
    let frozen = engine.snapshot().position_ms == checkpoint;
    let seek_ms = u64::from(song.duration.unwrap_or(2))
        .saturating_mul(500)
        .clamp(500, 5000);
    let resumed = if started {
        engine.seek(seek_ms);
        std::thread::sleep(Duration::from_millis(200));
        engine.resume();
        wait(&engine, seek_ms + 250)
    } else {
        false
    };
    let snapshot = engine.snapshot();
    engine.stop();
    json!({
        "suffix":suffix,"requested_quality":format!("{quality:?}"),
        "started":started,"time_to_250ms_played_ms":startup_ms,"pause_frozen":frozen,
        "seek_requested_ms":seek_ms,"playback_after_seek":resumed,
        "last_position_ms":snapshot.position_ms,"underruns_including_startup":snapshot.underruns,
        "passed":started && frozen && resumed
    })
}

async fn check(albums: u32) -> Result<Value, &'static str> {
    let settings = Settings::load();
    let password = settings.password().ok_or("saved_password_unavailable")?;
    let client = Client::new(
        &settings.server,
        Credentials::new(&settings.username, password),
    )
    .map_err(|_| "invalid_saved_connection")?;
    let info = client.ping().await.map_err(|_| "server_ping_failed")?;
    let list = client
        .album_list(AlbumListType::Newest, albums, 0)
        .await
        .map_err(|_| "album_list_failed")?;
    let mut samples = BTreeMap::new();
    for album in &list {
        let album = client
            .album(&album.id)
            .await
            .map_err(|_| "album_detail_failed")?;
        for song in album.song {
            if song.duration.is_some_and(|duration| duration < 2) {
                continue;
            }
            let suffix = song
                .suffix
                .as_deref()
                .unwrap_or("unknown")
                .to_ascii_lowercase();
            samples.entry(suffix).or_insert(song);
        }
    }
    let mut results = Vec::new();
    for song in samples.values() {
        results.push(playback(&client, song, StreamQuality::Original));
    }
    if let Some(song) = samples.values().next() {
        results.push(playback(
            &client,
            song,
            StreamQuality::Transcode { max_bit_rate: 192 },
        ));
    }
    Ok(json!({
        "scope":"Short silent real-server playback/pause/seek checks; not full tracks, desktop UI or two-hour acceptance",
        "read_only":true,"silent":true,"server_type":info.server_type,"server_version":info.server_version,
        "albums_checked":list.len(),"formats_found":samples.keys().collect::<Vec<_>>(),
        "passed":!results.is_empty() && results.iter().all(|result| result["passed"] == true),
        "checks":results,"missing_formats_remain_unverified":true
    }))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut albums = 30;
    let mut report = PathBuf::from("output/server-check.json");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--albums" => {
                albums = args
                    .next()
                    .ok_or("Missing album count")?
                    .parse::<u32>()?
                    .clamp(1, 80)
            }
            "--report" => report = args.next().ok_or("Missing report path")?.into(),
            "--help" => {
                println!(
                    "server_check [--albums 30] [--report output/server-check.json]\nUses saved connection. Silent read-only sample playback/pause/seek; no scrobbling or saved-setting changes."
                );
                return Ok(());
            }
            _ => return Err("Unknown argument; use --help".into()),
        }
    }
    let result = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(check(albums));
    let result = result.unwrap_or_else(
        |stage| json!({"passed":false,"stage":stage,"read_only":true,"silent":true}),
    );
    if let Some(parent) = report.parent().filter(|path| !path.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&report, serde_json::to_vec_pretty(&result)?)?;
    println!(
        "{}",
        serde_json::json!({"report":report,"passed":result["passed"],"stage":result["stage"],"formats_found":result["formats_found"]})
    );
    if result["passed"] != true {
        return Err("Server sample acceptance failed; see the report".into());
    }
    Ok(())
}
