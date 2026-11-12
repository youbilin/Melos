//! Inspects and exercises only the isolated Melos QA session, not other apps.
#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    tokio::runtime::Builder::new_current_thread().enable_all().build()?.block_on(async {
        use windows::Media::Control::GlobalSystemMediaTransportControlsSessionManager;
        use windows::Storage::Streams::DataReader;
        let command = std::env::args().nth(1).unwrap_or_else(|| "inspect".into());
        if !matches!(command.as_str(), "inspect" | "play" | "pause" | "next" | "previous") {
            return Err("Use inspect, play, pause, next or previous".into());
        }
        let manager = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()?.await?;
        let mut found = Vec::new();
        for session in manager.GetSessions()? {
            let source = session.SourceAppUserModelId()?.to_string();
            if !source.to_ascii_lowercase().contains("melos") {
                continue;
            }
            let media = session.TryGetMediaPropertiesAsync()?.await?;
            if matches!(media.Title()?.to_string().as_str(), "Song 0" | "Song 1" | "Song 2")
                && media.Artist()? == "QA Artist"
            {
                found.push(session);
            }
        }
        if found.len() != 1 {
            return Err("Expected exactly one isolated Melos QA media session".into());
        }
        let session = found.remove(0);
        let accepted = match command.as_str() {
            "play" => Some(session.TryPlayAsync()?.await?),
            "pause" => Some(session.TryPauseAsync()?.await?),
            "next" => Some(session.TrySkipNextAsync()?.await?),
            "previous" => Some(session.TrySkipPreviousAsync()?.await?),
            _ => None,
        };
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let media = session.TryGetMediaPropertiesAsync()?.await?;
        let mut bytes = Vec::new();
        if let Ok(reference) = media.Thumbnail() {
            let stream = reference.OpenReadAsync()?.await?;
            let size = u32::try_from(stream.Size()?)?;
            let reader = DataReader::CreateDataReader(&stream)?;
            reader.LoadAsync(size)?.await?;
            bytes.resize(size as usize, 0);
            reader.ReadBytes(&mut bytes)?;
        }
        let timeline = session.GetTimelineProperties()?;
        println!("{}", serde_json::json!({
            "command":command,"accepted":accepted,"title":media.Title()?.to_string(),
            "artist":media.Artist()?.to_string(),"status":session.GetPlaybackInfo()?.PlaybackStatus()?.0,
            "position_ms":timeline.Position()?.Duration / 10_000,
            "duration_ms":timeline.EndTime()?.Duration / 10_000,
            "thumbnail_bytes":bytes.len(),"thumbnail_is_png":bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
            "scope":"Programmatic Windows media session API and isolated Melos UI, not physical media keys."
        }));
        Ok(())
    })
}

#[cfg(not(windows))]
fn main() {
    eprintln!("This QA tool requires Windows.");
    std::process::exit(1);
}
