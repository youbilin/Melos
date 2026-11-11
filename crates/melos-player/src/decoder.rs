//! Symphonia decoder using packet-aware gapless trimming.
//!
//! Symphonia trims encoder delay and padding itself. Applying the track-level
//! values again would cut valid samples from the beginning and end of MP3s.

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

        let (track_id, sample_rate, channels, duration_ms, decoder) = {
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

            (track.id, sample_rate, channels, duration_ms, decoder)
        };

        Ok(Self {
            format,
            decoder,
            track_id,
            sample_rate,
            channels: channels.max(1),
            duration_ms,
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
        self.eof = false;
        Ok(())
    }
}
