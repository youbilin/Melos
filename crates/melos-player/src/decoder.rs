//! Symphonia-backed decoder with encoder delay / padding trimming.
//!
//! `Track::delay` and `Track::padding` matter for MP3: a decoder that ignores
//! them inserts silence at every track boundary, which breaks gapless playback.

use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType};
use symphonia::core::io::MediaSource;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::Time;

use crate::error::PlayerError;

pub struct Decoder {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
    sample_rate: u32,
    channels: u16,
    duration_ms: Option<u64>,
    delay_remaining: u64,
    padding_samples: usize,
    scratch: Vec<f32>,
    tail: Vec<f32>,
    eof: bool,
}

impl Decoder {
    pub fn open(source: Box<dyn MediaSource>, hint: Option<&str>) -> Result<Self, PlayerError> {
        let stream = MediaSourceStream::new(source, Default::default());

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

        let (track_id, sample_rate, channels, duration_ms, delay, padding, decoder) = {
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

            (
                track.id,
                sample_rate,
                channels,
                duration_ms,
                u64::from(track.delay.unwrap_or(0)),
                u64::from(track.padding.unwrap_or(0)),
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
            delay_remaining: delay,
            padding_samples: padding as usize * channels.max(1) as usize,
            scratch: Vec::new(),
            tail: Vec::new(),
            eof: false,
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
    /// Returns `false` at end of stream. The encoder delay is dropped from the
    /// front and the padding is held back from the tail, so concatenating two
    /// tracks is sample-exact.
    pub fn next_chunk(&mut self, out: &mut Vec<f32>) -> Result<bool, PlayerError> {
        out.clear();

        loop {
            if self.eof {
                let drain = self.tail.len().saturating_sub(self.padding_samples);
                out.extend(self.tail.drain(..drain));
                self.tail.clear();
                return Ok(!out.is_empty());
            }

            match self.format.next_packet() {
                Ok(Some(packet)) => {
                    if packet.track_id != self.track_id {
                        continue;
                    }

                    match self.decoder.decode(&packet) {
                        Ok(buffer) => {
                            self.scratch.resize(buffer.samples_interleaved(), 0.0);
                            buffer.copy_to_slice_interleaved(&mut self.scratch);

                            let channels = self.channels as usize;
                            let frames = self.scratch.len() / channels;
                            let mut start = 0usize;

                            if self.delay_remaining > 0 {
                                let skip = self.delay_remaining.min(frames as u64) as usize;
                                start = skip * channels;
                                self.delay_remaining -= skip as u64;
                            }

                            self.tail.extend_from_slice(&self.scratch[start..]);

                            let drain = self.tail.len().saturating_sub(self.padding_samples);
                            out.extend(self.tail.drain(..drain));

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
                Err(error) => return Err(PlayerError::Decode(error.to_string())),
            }
        }
    }

    pub fn seek_ms(&mut self, position_ms: u64) -> Result<(), PlayerError> {
        let seconds = position_ms as f64 / 1000.0;
        let time = Time::try_from_secs_f64(seconds)
            .ok_or_else(|| PlayerError::Config("seek position out of range".into()))?;

        self.format
            .seek(
                SeekMode::Accurate,
                SeekTo::Time {
                    time,
                    track_id: Some(self.track_id),
                },
            )
            .map_err(|e| PlayerError::Decode(format!("seek: {e}")))?;

        self.decoder.reset();
        self.tail.clear();
        self.scratch.clear();
        self.delay_remaining = 0;
        self.eof = false;
        Ok(())
    }
}
