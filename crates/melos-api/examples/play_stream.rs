//! End-to-end stream check:
//! `cargo run -p melos-api --example play_stream -- <server> <user> <password>`

use melos_api::{AlbumListType, Client, Credentials, StreamQuality};
use melos_player::{Engine, StreamSpec};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 3 {
        eprintln!("usage: play_stream <server> <user> <password>");
        std::process::exit(2);
    }

    let client = Client::new(&args[0], Credentials::new(&args[1], &args[2])).unwrap();
    let albums = client.album_list(AlbumListType::Newest, 1, 0).await.unwrap();
    let Some(album) = albums.first() else {
        eprintln!("no albums");
        return;
    };

    let album = client.album(&album.id).await.unwrap();
    let Some(song) = album.song.first() else {
        eprintln!("no songs");
        return;
    };

    println!(
        "playing {} — {} ({})",
        song.artist.clone().unwrap_or_default(),
        song.title,
        song.suffix.clone().unwrap_or_default()
    );

    let engine = Engine::new().expect("audio device");
    engine.set_volume(0.15);

    let spec = StreamSpec {
        url: client.stream_url(song, StreamQuality::Original),
        hint: song.suffix.clone(),
        duration_ms: song.duration.map(|seconds| u64::from(seconds) * 1000),
    };
    engine.play(spec, None);

    for _ in 0..10 {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let snapshot = engine.snapshot();
        println!(
            "{} / {} ms  playing={} underruns={}",
            snapshot.position_ms, snapshot.duration_ms, snapshot.playing, snapshot.underruns
        );
        if let Some(error) = engine.take_error() {
            eprintln!("error: {error}");
            break;
        }
    }
}
