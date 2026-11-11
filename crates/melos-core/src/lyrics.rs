//! Lyrics normalization, LRC parsing and playback-time lookup shared by frontends.

use melos_api::{Client, Error, Song, StructuredLyrics};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub start_ms: Option<u64>,
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Lyrics {
    pub language: Option<String>,
    pub synced: bool,
    pub lines: Vec<Line>,
}

impl Lyrics {
    pub fn from_structured(lyrics: StructuredLyrics) -> Self {
        // Incomplete synchronized tracks are displayed as plain text rather
        // than inventing timing for lines that have no timestamp.
        let synced = lyrics.synced && lyrics.line.iter().all(|line| line.start.is_some());
        let mut lines: Vec<_> = lyrics
            .line
            .into_iter()
            .map(|line| Line {
                start_ms: if synced {
                    line.start
                        .map(|start| start.saturating_add_signed(lyrics.offset))
                } else {
                    None
                },
                text: line.value,
            })
            .collect();
        if synced {
            lines.sort_by_key(|line| line.start_ms);
        }
        Self {
            language: lyrics.lang,
            synced,
            lines,
        }
    }

    pub fn parse(text: &str) -> Self {
        let mut timed = Vec::new();
        let mut plain = Vec::new();
        let mut offset = 0i64;
        for raw in text.trim_start_matches('\u{feff}').lines() {
            let raw = raw.trim();
            if let Some(value) = raw
                .strip_prefix("[offset:")
                .and_then(|s| s.strip_suffix(']'))
            {
                if let Ok(value) = value.trim().parse() {
                    offset = value;
                }
                continue;
            }
            if ["[ar:", "[ti:", "[al:", "[by:", "[re:", "[ve:", "[length:"]
                .iter()
                .any(|prefix| raw.starts_with(prefix))
            {
                continue;
            }
            let mut remaining = raw;
            let mut starts = Vec::new();
            while let Some(after) = remaining.strip_prefix('[') {
                let Some((tag, rest)) = after.split_once(']') else {
                    break;
                };
                let Some(start) = timestamp(tag) else {
                    break;
                };
                starts.push(start);
                remaining = rest;
            }
            if starts.is_empty() {
                plain.push(Line {
                    start_ms: None,
                    text: raw.to_owned(),
                });
            } else {
                timed.extend(starts.into_iter().map(|start| Line {
                    start_ms: Some(start),
                    text: remaining.trim().to_owned(),
                }));
            }
        }
        if timed.is_empty() {
            Self {
                language: None,
                synced: false,
                lines: plain,
            }
        } else {
            for line in &mut timed {
                // LRC positive offsets advance lyrics; OpenSubsonic exposes a
                // signed delay separately and uses start + offset above.
                line.start_ms = line
                    .start_ms
                    .map(|start| start.saturating_add_signed(offset.saturating_neg()));
            }
            timed.sort_by_key(|line| line.start_ms);
            Self {
                language: None,
                synced: true,
                lines: timed,
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.lines.iter().all(|line| line.text.trim().is_empty())
    }

    /// All lines at the current timestamp (including parallel translations).
    pub fn active_range(&self, position_ms: u64) -> std::ops::Range<usize> {
        if !self.synced {
            return 0..0;
        }
        let end = self
            .lines
            .partition_point(|line| line.start_ms.is_some_and(|start| start <= position_ms));
        if end == 0 {
            return 0..0;
        }
        let start =
            self.lines[..end].partition_point(|line| line.start_ms < self.lines[end - 1].start_ms);
        start..end
    }
}

fn timestamp(tag: &str) -> Option<u64> {
    let (minutes, seconds) = tag.split_once(':')?;
    if minutes.is_empty() || !minutes.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let (seconds, fraction) = seconds.split_once('.').unwrap_or((seconds, ""));
    if seconds.len() != 2
        || !seconds.bytes().all(|byte| byte.is_ascii_digit())
        || fraction.len() > 3
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let seconds: u64 = seconds.parse().ok()?;
    if seconds >= 60 {
        return None;
    }
    let fraction_ms = if fraction.is_empty() {
        0
    } else {
        fraction.parse::<u64>().ok()? * 10u64.pow(3 - fraction.len() as u32)
    };
    minutes
        .parse::<u64>()
        .ok()?
        .checked_mul(60_000)?
        .checked_add(seconds * 1000 + fraction_ms)
}

/// Falls back only for unsupported/missing endpoints or an empty result.
/// Authentication and network errors remain visible to the caller.
pub async fn fetch(client: &Client, song: &Song) -> Result<Vec<Lyrics>, Error> {
    let structured = match client.lyrics_by_song_id(&song.id).await {
        Ok(lyrics) => lyrics,
        Err(
            Error::NotFound
            | Error::Server {
                code: 0 | 20 | 30, ..
            },
        ) => Vec::new(),
        Err(error) => return Err(error),
    };
    let mut lyrics: Vec<_> = structured
        .into_iter()
        .map(Lyrics::from_structured)
        .filter(|lyrics| !lyrics.is_empty())
        .collect();
    if lyrics.is_empty()
        && !song.title.trim().is_empty()
        && let Some(plain) = client
            .lyrics(song.artist.as_deref().unwrap_or(""), &song.title)
            .await?
    {
        let parsed = Lyrics::parse(&plain.value);
        if !parsed.is_empty() {
            lyrics.push(parsed);
        }
    }
    // Prefer timed lyrics while preserving the server's order within each kind.
    lyrics.sort_by_key(|lyrics| !lyrics.synced);
    Ok(lyrics)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn lyrics_server(
        first_status: u16,
        first: &'static str,
        expected_requests: usize,
    ) -> (Client, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            for index in 0..expected_requests {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = [0; 8192];
                let len = socket.read(&mut bytes).await.unwrap();
                let request = std::str::from_utf8(&bytes[..len]).unwrap();
                assert!(request.starts_with(if index == 0 {
                    "GET /rest/getLyricsBySongId?"
                } else {
                    "GET /rest/getLyrics?"
                }));
                if index == 0 {
                    assert!(request.contains("id=s1"));
                } else {
                    assert!(request.contains("artist=Artist") && request.contains("title=Title"));
                }
                let (status, body) = if index == 0 {
                    (first_status, first)
                } else {
                    (
                        200,
                        r#"{"subsonic-response":{"status":"ok","lyrics":{"value":"[00:01.00]Legacy"}}}"#,
                    )
                };
                socket.write_all(format!("HTTP/1.1 {status} Response\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            }
        });
        (
            Client::new(&url, melos_api::Credentials::new("qa", "qa")).unwrap(),
            server,
        )
    }

    #[tokio::test]
    async fn fetch_falls_back_for_http_unsupported_empty_and_subsonic_unsupported() {
        for (status, body) in [
            (404, "Missing endpoint"),
            (
                200,
                r#"{"subsonic-response":{"status":"ok","lyricsList":{"structuredLyrics":[]}}}"#,
            ),
            (
                200,
                r#"{"subsonic-response":{"status":"failed","error":{"code":0,"message":"unsupported"}}}"#,
            ),
        ] {
            let (client, server) = lyrics_server(status, body, 2).await;
            let tracks = fetch(
                &client,
                &Song {
                    id: "s1".into(),
                    artist: Some("Artist".into()),
                    title: "Title".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
            assert_eq!(tracks[0].lines[0].start_ms, Some(1000));
            assert_eq!(tracks[0].lines[0].text, "Legacy");
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn fetch_does_not_hide_auth_failure_or_query_legacy_when_embedded_available() {
        let song = Song {
            id: "s1".into(),
            ..Default::default()
        };
        let (client, server) = lyrics_server(
            200,
            r#"{"subsonic-response":{"status":"failed","error":{"code":40}}}"#,
            1,
        )
        .await;
        assert!(matches!(fetch(&client, &song).await, Err(Error::Auth)));
        server.await.unwrap();
        let (client, server) = lyrics_server(200, r#"{"subsonic-response":{"status":"ok","lyricsList":{"structuredLyrics":[{"line":[{"value":"Plain"}]},{"synced":true,"line":[{"start":0,"value":"Timed"}]}]}}}"#, 1).await;
        let tracks = fetch(&client, &song).await.unwrap();
        assert!(tracks[0].synced);
        assert!(!tracks[1].synced);
        server.await.unwrap();
    }

    #[test]
    fn lrc_preserves_repeated_tags_fractional_times_and_late_offset() {
        let lyrics = Lyrics::parse(
            "\u{feff}[ar:Artist]\n[00:05.25][00:10.125]中文\n[00:01.5]Earlier\n[offset:250]",
        );
        assert_eq!(
            lyrics
                .lines
                .iter()
                .map(|line| line.start_ms.unwrap())
                .collect::<Vec<_>>(),
            [1250, 5000, 9875]
        );
        assert!(lyrics.active_range(1249).is_empty());
        assert_eq!(lyrics.active_range(5000), 1..2);
        assert_eq!(lyrics.active_range(1500), 0..1);
        assert_eq!(lyrics.lines[1].text, "中文");
    }

    #[test]
    fn structured_lines_sort_clamp_offsets_and_highlight_simultaneous_lines() {
        let raw = serde_json::from_str(r#"{"synced":true,"offset":-100,"line":[{"start":5000,"value":"Later"},{"start":0,"value":"Start"},{"start":5000,"value":"Translation"}]}"#).unwrap();
        let lyrics = Lyrics::from_structured(raw);
        assert_eq!(lyrics.lines[0].start_ms, Some(0));
        assert_eq!(lyrics.active_range(4900), 1..3);
        assert_eq!(lyrics.active_range(2000), 0..1);
    }

    #[test]
    fn plain_and_incomplete_lyrics_never_invent_seek_positions() {
        let lyrics = Lyrics::parse("First line\n\n中文歌词\n[00:99]Invalid");
        assert!(!lyrics.synced);
        assert_eq!(lyrics.lines.len(), 4);
        assert!(lyrics.active_range(u64::MAX).is_empty());
        let raw = serde_json::from_str(
            r#"{"synced":true,"line":[{"start":1000,"value":"One"},{"value":"No timing"}]}"#,
        )
        .unwrap();
        assert!(!Lyrics::from_structured(raw).synced);
        assert!(Lyrics::parse("[ar:Artist]\n\n").is_empty());
        assert!(timestamp("999999999999999999999999:00").is_none());
    }
}
