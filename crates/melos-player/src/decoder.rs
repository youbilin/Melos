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
            && self.decoded_end_ms > 0
            && self
                .duration_ms
                .is_none_or(|duration| self.decoded_end_ms.saturating_add(100) >= duration)
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
