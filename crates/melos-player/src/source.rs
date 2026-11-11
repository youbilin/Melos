//! Media sources: a local file, or an HTTP stream with Range seeking.

use std::io::{self, Read, Seek, SeekFrom};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use reqwest::blocking::{Client, Response};
use symphonia::core::io::MediaSource;

use crate::error::PlayerError;

pub fn open_file(path: &std::path::Path) -> Result<Box<dyn MediaSource>, PlayerError> {
    let file = std::fs::File::open(path).map_err(|e| PlayerError::Open(e.to_string()))?;
    Ok(Box::new(file))
}

pub fn open_url(url: &str) -> Result<Box<dyn MediaSource>, PlayerError> {
    Ok(Box::new(HttpSource::open(url)?))
}

struct HttpInner {
    client: Client,
    response: Option<Response>,
    position: u64,
    len: Option<u64>,
    seekable: bool,
}

/// A blocking HTTP media source.
///
/// The first GET streams the body. A seek drops the connection and issues a
/// new request with `Range: bytes=N-`; servers that ignore Range (for example
/// an on-the-fly transcode) are treated as non-seekable.
pub struct HttpSource {
    url: String,
    inner: Mutex<HttpInner>,
}

impl HttpSource {
    fn open(url: &str) -> Result<Self, PlayerError> {
        let client = Client::builder()
            .user_agent(concat!("Melos/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(5))
            .build()?;

        let response = client.get(url).send()?.error_for_status()?;
        let len = response.content_length();
        let seekable = response
            .headers()
            .get(reqwest::header::ACCEPT_RANGES)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.eq_ignore_ascii_case("bytes"))
            .unwrap_or(false);

        Ok(Self {
            url: url.to_owned(),
            inner: Mutex::new(HttpInner {
                client,
                response: Some(response),
                position: 0,
                len,
                seekable,
            }),
        })
    }
}

impl Read for HttpSource {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let inner = self.inner.get_mut().expect("http source lock");
        if inner.len.is_some_and(|len| inner.position >= len) {
            return Ok(0);
        }
        let deadline = Instant::now() + Duration::from_secs(45);
        loop {
            let read = match inner.response.as_mut() {
                Some(response) => response.read(buf),
                None => Err(io::Error::new(
                    io::ErrorKind::NotConnected,
                    "stream disconnected",
                )),
            };
            match read {
                Ok(0) if inner.len.is_some_and(|len| inner.position < len) => {}
                Ok(count) => {
                    inner.position += count as u64;
                    return Ok(count);
                }
                Err(error) if !inner.seekable || Instant::now() >= deadline => return Err(error),
                Err(_) => {}
            }
            if !inner.seekable {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "non-seekable stream interrupted",
                ));
            }
            inner.response = None;
            loop {
                match reopen(&self.url, inner, inner.position) {
                    Ok(()) => break,
                    Err(error)
                        if matches!(
                            error.kind(),
                            io::ErrorKind::Unsupported | io::ErrorKind::InvalidData
                        ) || Instant::now() >= deadline =>
                    {
                        return Err(error);
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(250)),
                }
            }
        }
    }
}

impl Seek for HttpSource {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let inner = self.inner.get_mut().expect("http source lock");

        let target = match pos {
            SeekFrom::Start(n) => n,
            SeekFrom::Current(n) => inner.position.saturating_add_signed(n),
            SeekFrom::End(n) => inner
                .len
                .ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, "unknown length"))?
                .saturating_add_signed(n),
        };

        if !inner.seekable {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "server does not support range requests",
            ));
        }
        if target == inner.position {
            return Ok(target);
        }
        if inner.len.is_some_and(|len| target >= len) {
            inner.response = None;
            inner.position = target;
            return Ok(target);
        }
        reopen(&self.url, inner, target)?;
        Ok(target)
    }
}

fn reopen(url: &str, inner: &mut HttpInner, target: u64) -> io::Result<()> {
    let response = inner
        .client
        .get(url)
        .header(reqwest::header::RANGE, format!("bytes={target}-"))
        .send()
        .map_err(io::Error::other)?;
    if response.status().is_server_error() {
        return Err(io::Error::other(format!("HTTP {}", response.status())));
    }
    if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "server ignored range request",
        ));
    }
    let expected = format!("bytes {target}-");
    if !response
        .headers()
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with(&expected))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "server returned a different range",
        ));
    }
    inner.position = target;
    inner.response = Some(response);
    Ok(())
}

impl MediaSource for HttpSource {
    fn is_seekable(&self) -> bool {
        self.inner
            .lock()
            .map(|inner| inner.seekable)
            .unwrap_or(false)
    }

    fn byte_len(&self) -> Option<u64> {
        self.inner.lock().ok().and_then(|inner| inner.len)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::net::TcpListener;
    use std::sync::Arc;
    use std::thread;

    use super::*;

    /// Serves `data` with byte-range support, so `HttpSource` can seek.
    fn range_server(data: Arc<Vec<u8>>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();

        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut socket) = stream else {
                    break;
                };
                let data = Arc::clone(&data);

                thread::spawn(move || {
                    let mut buffer = [0u8; 4096];
                    let read = socket.read(&mut buffer).unwrap_or(0);
                    let request = String::from_utf8_lossy(&buffer[..read]).to_string();

                    let start = request
                        .lines()
                        .find_map(|line| {
                            let lower = line.to_ascii_lowercase();
                            lower
                                .strip_prefix("range: bytes=")
                                .map(|value| value.trim_end_matches('-').to_owned())
                        })
                        .and_then(|value| value.parse::<usize>().ok());

                    let (status, body, extra) = match start {
                        Some(start) if start < data.len() => (
                            "206 Partial Content",
                            &data[start..],
                            format!(
                                "Content-Range: bytes {}-{}/{}\r\n",
                                start,
                                data.len() - 1,
                                data.len()
                            ),
                        ),
                        _ => ("200 OK", data.as_slice(), String::new()),
                    };

                    let header = format!(
                        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\n{extra}Connection: close\r\n\r\n",
                        body.len()
                    );

                    let _ = socket.write_all(header.as_bytes());
                    let _ = socket.write_all(body);
                });
            }
        });

        format!("http://{address}/audio")
    }

    #[test]
    fn http_source_streams_and_seeks() {
        let data: Arc<Vec<u8>> = Arc::new((0..=255u8).collect());
        let url = range_server(data);

        let mut source = open_url(&url).unwrap();
        let mut buffer = [0u8; 8];

        source.read_exact(&mut buffer).unwrap();
        assert_eq!(buffer, [0, 1, 2, 3, 4, 5, 6, 7]);

        source.seek(SeekFrom::Start(100)).unwrap();
        source.read_exact(&mut buffer).unwrap();
        assert_eq!(buffer, [100, 101, 102, 103, 104, 105, 106, 107]);
        source.seek(SeekFrom::End(0)).unwrap();
        assert_eq!(source.read(&mut buffer).unwrap(), 0);
    }

    fn interrupted_server(downtime: Duration) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let worker = thread::spawn(move || {
            let data: Vec<u8> = (0..=255).collect();
            let (mut first, _) = listener.accept().unwrap();
            let mut request = [0; 4096];
            assert!(first.read(&mut request).unwrap() > 0);
            first.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 256\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n").unwrap();
            first.write_all(&data[..64]).unwrap();
            drop(first);
            let started = Instant::now();
            loop {
                let (mut socket, _) = listener.accept().unwrap();
                let len = socket.read(&mut request).unwrap();
                let request = String::from_utf8_lossy(&request[..len]).to_ascii_lowercase();
                assert!(
                    request.contains("range: bytes=64-"),
                    "resume must preserve byte position"
                );
                if started.elapsed() < downtime {
                    socket.write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                    continue;
                }
                socket.write_all(b"HTTP/1.1 206 Partial Content\r\nContent-Length: 192\r\nContent-Range: bytes 64-255/256\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n").unwrap();
                socket.write_all(&data[64..]).unwrap();
                break;
            }
        });
        (format!("http://{address}/audio"), worker)
    }

    fn assert_resume(downtime: Duration) {
        let (url, server) = interrupted_server(downtime);
        let mut source = open_url(&url).unwrap();
        let mut bytes = Vec::new();
        source.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, (0..=255u8).collect::<Vec<_>>());
        server.join().unwrap();
    }

    #[test]
    fn interrupted_stream_resumes_without_losing_or_repeating_bytes() {
        assert_resume(Duration::from_millis(100));
    }

    #[test]
    #[ignore = "30-second network recovery acceptance test"]
    fn stream_recovers_after_thirty_seconds_offline() {
        assert_resume(Duration::from_secs(30));
    }
}
