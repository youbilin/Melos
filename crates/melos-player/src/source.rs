//! Media sources: a local file, or an HTTP stream with Range seeking.

use std::io::{self, Read, Seek, SeekFrom};
use std::sync::Mutex;
use std::time::Duration;

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
        let inner = self.inner.get_mut().expect("http source lock");
        let Some(response) = inner.response.as_mut() else {
            return Ok(0);
        };
        let read = response.read(buf)?;
        inner.position += read as u64;
        Ok(read)
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

        inner.response = None;
        let response = inner
            .client
            .get(&self.url)
            .header(reqwest::header::RANGE, format!("bytes={target}-"))
            .send()
            .map_err(|error| io::Error::new(io::ErrorKind::Other, error))?;

        if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "server ignored the range request",
            ));
        }

        inner.position = target;
        inner.response = Some(response);
        Ok(target)
    }
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
    }
}
