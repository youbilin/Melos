//! Manual connectivity probe:
//! `cargo run -p melos-api --example probe -- <server> <user> <password>`

use melos_api::{AlbumListType, Client, Credentials};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 3 {
        eprintln!("usage: probe <server> <user> <password>");
        std::process::exit(2);
    }

    let client = Client::new(&args[0], Credentials::new(&args[1], &args[2]))
        .expect("failed to build client");

    match client.ping().await {
        Ok(info) => println!(
            "ping ok: type={} version={} openSubsonic={:?}",
            info.server_type,
            info.server_version.unwrap_or_default(),
            info.open_subsonic
        ),
        Err(error) => {
            eprintln!("ping failed: {error}");
            return;
        }
    }

    match client.album_list(AlbumListType::Newest, 5, 0).await {
        Ok(albums) => {
            println!("albums: {}", albums.len());
            let cover_id = albums.iter().find_map(|album| album.cover_art.clone());
            for album in &albums {
                println!(
                    "  {} — {} ({})",
                    album.artist.clone().unwrap_or_default(),
                    album.name,
                    album.year.unwrap_or(0)
                );
            }

            if let Some(id) = cover_id {
                match client.cover_art(&id, Some(240)).await {
                    Ok(bytes) => println!(
                        "cover: {id} -> {} bytes, magic {:02x?}",
                        bytes.len(),
                        &bytes[..bytes.len().min(8)]
                    ),
                    Err(error) => eprintln!("cover failed: {error}"),
                }
            }
        }
        Err(error) => eprintln!("album list failed: {error}"),
    }
}
