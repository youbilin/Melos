//! Symphonia decoder using packet-aware gapless trimming.
//!
//! Symphonia trims encoder delay and padding itself. Applying the track-level
//! values again would cut valid samples from the beginning and end of MP3s.

use std::io::{self, Read, Seek, SeekFrom};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType};
use symphonia::core::io::MediaSource;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::{Time, TimeBase, Timestamp};

use crate::error::PlayerError;

pub struct Decoder {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
    sample_rate: u32,
    channels: u16,
    duration_ms: Option<u64>,
    expected_ms: Option<u64>,
    time_base: TimeBase,
    seek_target: Option<Timestamp>,
    eof: bool,
    source_end: Arc<SourceEnd>,
    decoded_end_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    const FLAC: &[u8] = include_bytes!("../tests/fixtures/tone.flac");

    fn decode_all(bytes: Vec<u8>) -> Result<usize, PlayerError> {
        let mut decoder = Decoder::open(Box::new(io::Cursor::new(bytes)), Some("flac"))?;
        let mut chunk = Vec::new();
        let mut samples = 0;
        while decoder.next_chunk(&mut chunk)? {
            samples += chunk.len();
        }
        Ok(samples / usize::from(decoder.channels()))
    }

    #[test]
    fn flac_slightly_overstated_length_finishes_without_a_tail_error() {
        let mut bytes = FLAC.to_vec();
        // STREAMINFO's last 36 bits hold the declared total sample count.
        let packed = u64::from_be_bytes(bytes[18..26].try_into().unwrap());
        bytes[18..26].copy_from_slice(&(packed + 1).to_be_bytes());
        let frames = decode_all(bytes).unwrap();
        assert!((40_000..=44_100).contains(&frames));
        assert_eq!(decode_all(FLAC.to_vec()).unwrap(), 44_100);
    }

    #[test]
    fn significantly_truncated_flac_remains_an_error() {
        let bytes = FLAC[..FLAC.len() / 2].to_vec();
        assert!(decode_all(bytes).is_err());
    }

    /// Delivers the whole file, then fails instead of returning a clean EOF,
    /// like a server or transcoder that drops the connection after the last
    /// frame.
    struct DroppedTail {
        data: Vec<u8>,
        position: usize,
        fail_at: usize,
    }

    impl Read for DroppedTail {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if self.position >= self.fail_at {
                return Err(io::Error::other("request or response body error"));
            }
            let end = (self.position + buffer.len()).min(self.fail_at);
            let count = end - self.position;
            buffer[..count].copy_from_slice(&self.data[self.position..end]);
            self.position = end;
            Ok(count)
        }
    }

    impl Seek for DroppedTail {
        fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
            let target = match position {
                SeekFrom::Start(offset) => offset as i64,
                SeekFrom::Current(offset) => self.position as i64 + offset,
                SeekFrom::End(offset) => self.data.len() as i64 + offset,
            };
            self.position = target.max(0) as usize;
            Ok(self.position as u64)
        }
    }

    impl MediaSource for DroppedTail {
        fn is_seekable(&self) -> bool {
            true
        }
        fn byte_len(&self) -> Option<u64> {
            Some(self.data.len() as u64)
        }
    }

    #[test]
    fn dropped_connection_after_the_last_frame_finishes_normally() {
        let mut decoder = Decoder::open(
            Box::new(DroppedTail {
                data: FLAC.to_vec(),
                position: 0,
                fail_at: FLAC.len(),
            }),
            Some("flac"),
        )
        .unwrap();
        let mut chunk = Vec::new();
        let mut frames = 0;
        while decoder.next_chunk(&mut chunk).unwrap() {
            frames += chunk.len() / usize::from(decoder.channels());
        }
        // The failed read can take the final packet with it; losing one frame
        // group at the tail is acceptable, stopping the queue is not.
        assert!((41_000..=44_100).contains(&frames));
    }

    #[test]
    fn server_duration_covers_a_stream_without_container_duration() {
        let mut decoder = Decoder::open_with_duration(
            Box::new(DroppedTail {
                data: FLAC.to_vec(),
                position: 0,
                fail_at: FLAC.len(),
            }),
            Some("flac"),
            Some(1_000),
        )
        .unwrap();
        // Transcoded streams usually expose no duration of their own.
        decoder.duration_ms = None;
        let mut chunk = Vec::new();
        let mut frames = 0;
        while decoder.next_chunk(&mut chunk).unwrap() {
            frames += chunk.len() / usize::from(decoder.channels());
        }
        assert!(frames >= 41_000);
    }

    #[test]
    fn dropped_connection_mid_track_remains_an_error() {
        let cut = FLAC.len() / 2;
        let mut decoder = Decoder::open(
            Box::new(DroppedTail {
                data: FLAC[..cut].to_vec(),
                position: 0,
                fail_at: cut,
            }),
            Some("flac"),
        )
        .unwrap();
        let mut chunk = Vec::new();
        loop {
            match decoder.next_chunk(&mut chunk) {
                Ok(true) => {}
                Ok(false) => panic!("truncated track reported a clean end"),
                Err(_) => break,
            }
        }
    }

    #[test]
    fn transport_interruption_is_not_a_clean_audio_end() {
        let mut decoder = Decoder::open(Box::new(io::Cursor::new(FLAC)), Some("flac")).unwrap();
        decoder.decoded_end_ms = 1000;
        decoder.source_end.clean.store(true, Ordering::Relaxed);
        assert!(decoder.at_audio_end());
        decoder
            .source_end
            .interrupted
            .store(true, Ordering::Relaxed);
        assert!(!decoder.at_audio_end());
    }
}

#[derive(Default)]
struct SourceEnd {
    clean: AtomicBool,
    interrupted: AtomicBool,
}

/// A connection that dies this close to the declared end still produced the
/// whole track in practice; real truncation is far larger.
const INTERRUPTED_END_TOLERANCE_MS: u64 = 1_000;

struct ObservedSource {
    inner: Box<dyn MediaSource>,
    end: Arc<SourceEnd>,
}

impl Read for ObservedSource {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let result = self.inner.read(buffer);
        if !buffer.is_empty() {
            self.end
                .clean
                .store(matches!(result, Ok(0)), Ordering::Relaxed);
            if result.is_err() {
                self.end.interrupted.store(true, Ordering::Relaxed);
            }
        }
        result
    }
}

impl Seek for ObservedSource {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let result = self.inner.seek(position);
        if result.is_ok() {
            self.end.clean.store(false, Ordering::Relaxed);
            self.end.interrupted.store(false, Ordering::Relaxed);
        }
        result
    }
}

impl MediaSource for ObservedSource {
    fn is_seekable(&self) -> bool {
        self.inner.is_seekable()
    }
    fn byte_len(&self) -> Option<u64> {
        self.inner.byte_len()
    }
}

impl Decoder {
    pub fn open(source: Box<dyn MediaSource>, hint: Option<&str>) -> Result<Self, PlayerError> {
        Self::open_with_duration(source, hint, None)
    }

    /// Opens a decoder with the duration reported by the server. Streamed
    /// transcodes often carry no usable duration in the container, so the
    /// server value is the only way to recognise the end of the track.
    pub fn open_with_duration(
        source: Box<dyn MediaSource>,
        hint: Option<&str>,
        expected_ms: Option<u64>,
    ) -> Result<Self, PlayerError> {
        let source_end = Arc::new(SourceEnd::default());
        let stream = MediaSourceStream::new(
            Box::new(ObservedSource {
                inner: source,
                end: source_end.clone(),
            }),
            Default::default(),
        );

        let mut probe_hint = Hint::new();
        if let Some(extension) = hint {
            probe_hint.with_extension(extension);
        }

        let format = symphonia::default::get_probe()
            .probe(
                &probe_hint,
                stream,
                FormatOptions::default(),
                MetadataOptions::default(),
            )
            .map_err(|e| PlayerError::Open(e.to_string()))?;

        let (track_id, sample_rate, channels, duration_ms, time_base, decoder) = {
            let track = format
                .default_track(TrackType::Audio)
                .ok_or_else(|| PlayerError::Open("no audio track".into()))?;

            let params = track
                .codec_params
                .as_ref()
                .and_then(|params| params.audio())
                .ok_or_else(|| PlayerError::Open("no audio codec parameters".into()))?;

            let sample_rate = params
                .sample_rate
                .ok_or_else(|| PlayerError::Config("missing sample rate".into()))?;
            let channels = params
                .channels
                .as_ref()
                .map(|channels| channels.count())
                .unwrap_or(2) as u16;

            let duration_ms = track
                .num_frames
                .map(|frames| frames.saturating_mul(1000) / u64::from(sample_rate))
                .filter(|ms| *ms > 0);

            let decoder = symphonia::default::get_codecs()
                .make_audio_decoder(params, &AudioDecoderOptions::default())
                .map_err(|e| PlayerError::Decode(format!("unsupported codec: {e}")))?;

            let time_base = track.time_base.unwrap_or_else(|| {
                TimeBase::try_from_recip(sample_rate).expect("nonzero sample rate")
            });
            (
                track.id,
                sample_rate,
                channels,
                duration_ms,
                time_base,
                decoder,
            )
        };

        Ok(Self {
            format,
            decoder,
            track_id,
            sample_rate,
            channels: channels.max(1),
            duration_ms,
            time_base,
            seek_target: None,
            eof: false,
            source_end,
            decoded_end_ms: 0,
            expected_ms: expected_ms.filter(|ms| *ms > 0),
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    pub fn duration_ms(&self) -> Option<u64> {
        self.duration_ms
    }

    /// Decodes the next chunk of interleaved f32 samples.
    ///
    /// Returns `false` at end of stream. Decoder options retain Symphonia's
    /// default gapless behavior; this layer never applies a second trim.
    pub fn next_chunk(&mut self, out: &mut Vec<f32>) -> Result<bool, PlayerError> {
        out.clear();

        loop {
            if self.eof {
                return Ok(false);
            }

            match self.format.next_packet() {
                Ok(Some(packet)) => {
                    if packet.track_id != self.track_id {
                        continue;
                    }

                    match self.decoder.decode(&packet) {
                        Ok(buffer) => {
                            out.resize(buffer.samples_interleaved(), 0.0);
                            buffer.copy_to_slice_interleaved(out.as_mut_slice());
                            let start_ticks =
                                packet.pts.saturating_add(packet.trim_start).get().max(0) as u64;
                            let start_ms = (u128::from(start_ticks)
                                * u128::from(self.time_base.numer.get())
                                * 1000
                                / u128::from(self.time_base.denom.get()))
                                as u64;
                            self.decoded_end_ms = self.decoded_end_ms.max(
                                start_ms
                                    + (out.len() / usize::from(self.channels)) as u64 * 1000
                                        / u64::from(self.sample_rate),
                            );
                            if let Some(target) = self.seek_target {
                                // Accurate demuxer seeks land at a packet before
                                // the target. Decode preroll for codec state,
                                // then discard only samples before the target.
                                // Gapless delay was already trimmed by Symphonia.
                                let start = packet.pts.saturating_add(packet.trim_start);
                                let ticks = target.saturating_delta(start).get().max(0) as u64;
                                let frames = (u128::from(ticks)
                                    * u128::from(self.time_base.numer.get())
                                    * u128::from(self.sample_rate))
                                .div_ceil(u128::from(self.time_base.denom.get()));
                                let skip = frames
                                    .min((out.len() / usize::from(self.channels)) as u128)
                                    as usize
                                    * usize::from(self.channels);
                                out.drain(..skip);
                                if !out.is_empty() {
                                    self.seek_target = None;
                                }
                            }
                            if !out.is_empty() {
                                return Ok(true);
                            }
                        }
                        Err(SymphoniaError::DecodeError(_)) | Err(SymphoniaError::IoError(_)) => {
                            continue;
                        }
                        Err(error) => return Err(PlayerError::Decode(error.to_string())),
                    }
                }
                Ok(None) => self.eof = true,
                Err(SymphoniaError::ResetRequired) => self.eof = true,
                Err(SymphoniaError::IoError(error))
                    if error.kind() == io::ErrorKind::UnexpectedEof && self.at_audio_end() =>
                {
                    self.eof = true;
                }
                // Servers and transcoders sometimes drop the connection right
                // after the last frame. If nearly all declared audio was
                // decoded, treat the failed tail read as the end of the track.
                Err(SymphoniaError::IoError(_))
                    if self.decoded_to_end(INTERRUPTED_END_TOLERANCE_MS) =>
                {
                    self.eof = true;
                }
                Err(error) => return Err(PlayerError::Decode(error.to_string())),
            }
        }
    }

    fn at_audio_end(&self) -> bool {
        // Some files have a slightly overstated frame count or an incomplete
        // tail packet. Accept at most 100 ms at a clean source end; a real
        // transport error or a substantially truncated track stays an error.
        self.source_end.clean.load(Ordering::Relaxed)
            && !self.source_end.interrupted.load(Ordering::Relaxed)
            && self.decoded_to_end(100)
    }

    fn decoded_to_end(&self, tolerance_ms: u64) -> bool {
        let Some(duration) = self.duration_ms.or(self.expected_ms) else {
            return false;
        };
        // Require at least 90% of the declared audio so a track truncated in
        // the middle cannot pass as a finished one.
        self.decoded_end_ms > 0
            && u128::from(self.decoded_end_ms) * 10 >= u128::from(duration) * 9
            && self.decoded_end_ms.saturating_add(tolerance_ms) >= duration
    }

    pub fn seek_ms(&mut self, position_ms: u64) -> Result<(), PlayerError> {
        let seconds = position_ms as f64 / 1000.0;
        let time = Time::try_from_secs_f64(seconds)
            .ok_or_else(|| PlayerError::Config("seek position out of range".into()))?;

        let seeked = self
            .format
            .seek(
                SeekMode::Accurate,
                SeekTo::Time {
                    time,
                    track_id: Some(self.track_id),
                },
            )
            .map_err(|e| PlayerError::Decode(format!("seek: {e}")))?;

        self.decoder.reset();
        self.seek_target = Some(seeked.required_ts);
        self.eof = false;
        self.decoded_end_ms = 0;
        Ok(())
    }
}
