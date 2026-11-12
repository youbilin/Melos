//! System media controls bound to the native desktop window.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaCommand {
    Play,
    Pause,
    Next,
    Previous,
}

#[cfg(windows)]
mod platform {
    use super::MediaCommand;
    use gpui::Window;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::sync::{Arc, Mutex};
    use windows::Foundation::{TimeSpan, TypedEventHandler};
    use windows::Media::{
        MediaPlaybackStatus, MediaPlaybackType, SystemMediaTransportControls,
        SystemMediaTransportControlsButton, SystemMediaTransportControlsButtonPressedEventArgs,
        SystemMediaTransportControlsTimelineProperties,
    };
    use windows::Storage::Streams::{
        DataWriter, InMemoryRandomAccessStream, RandomAccessStreamReference,
    };
    use windows::Win32::Foundation::{E_FAIL, HWND};
    use windows::Win32::System::WinRT::ISystemMediaTransportControlsInterop;
    use windows::core::{Error, HSTRING, factory};

    pub async fn prepare_artwork(
        bytes: &[u8],
    ) -> windows::core::Result<RandomAccessStreamReference> {
        let stream = InMemoryRandomAccessStream::new()?;
        let writer = DataWriter::CreateDataWriter(&stream)?;
        writer.WriteBytes(bytes)?;
        writer.StoreAsync()?.await?;
        writer.DetachStream()?;
        stream.Seek(0)?;
        RandomAccessStreamReference::CreateFromStream(&stream)
    }

    #[derive(Default)]
    pub struct Smtc {
        commands: Arc<Mutex<Vec<MediaCommand>>>,
        controls: Option<SystemMediaTransportControls>,
        button_token: Option<i64>,
        metadata: Mutex<Option<(String, String)>>,
        timeline: Mutex<Option<(u64, u64)>>,
        status: Mutex<Option<MediaPlaybackStatus>>,
    }

    impl Smtc {
        pub fn attach(window: &Window) -> Self {
            let mut smtc = Self::default();
            if let Err(error) = smtc.bind(window) {
                eprintln!("melos: SMTC unavailable: {error}");
            }
            smtc
        }

        fn bind(&mut self, window: &Window) -> windows::core::Result<()> {
            let raw = HasWindowHandle::window_handle(window)
                .map_err(|_| Error::new(E_FAIL, "native window handle unavailable"))?
                .as_raw();
            let RawWindowHandle::Win32(handle) = raw else {
                return Err(Error::new(E_FAIL, "expected Win32 window"));
            };
            let interop =
                factory::<SystemMediaTransportControls, ISystemMediaTransportControlsInterop>()?;
            // The GPUI window owns the HWND and outlives this registration.
            // Desktop apps use GetForWindow instead of UWP GetForCurrentView.
            let controls: SystemMediaTransportControls =
                unsafe { interop.GetForWindow(HWND(handle.hwnd.get() as *mut _))? };
            controls.SetIsEnabled(true)?;
            controls.SetIsPlayEnabled(true)?;
            controls.SetIsPauseEnabled(true)?;
            controls.SetIsNextEnabled(true)?;
            controls.SetIsPreviousEnabled(true)?;
            controls
                .DisplayUpdater()?
                .SetType(MediaPlaybackType::Music)?;
            let commands = Arc::clone(&self.commands);
            let token = controls.ButtonPressed(&TypedEventHandler::<
                SystemMediaTransportControls,
                SystemMediaTransportControlsButtonPressedEventArgs,
            >::new(move |_, args| {
                if let Some(args) = args.as_ref() {
                    let command = match args.Button()? {
                        SystemMediaTransportControlsButton::Play => Some(MediaCommand::Play),
                        SystemMediaTransportControlsButton::Pause => Some(MediaCommand::Pause),
                        SystemMediaTransportControlsButton::Next => Some(MediaCommand::Next),
                        SystemMediaTransportControlsButton::Previous => {
                            Some(MediaCommand::Previous)
                        }
                        _ => None,
                    };
                    if let Some(command) = command
                        && let Ok(mut queue) = commands.lock()
                    {
                        queue.push(command);
                    }
                }
                Ok(())
            }))?;
            self.button_token = Some(token);
            self.controls = Some(controls);
            Ok(())
        }

        pub fn drain(&self) -> Vec<MediaCommand> {
            self.commands
                .lock()
                .map(|mut queue| queue.drain(..).collect())
                .unwrap_or_default()
        }
        pub fn available(&self) -> bool {
            self.controls.is_some()
        }
        pub fn set_artwork(
            &self,
            image: Option<&RandomAccessStreamReference>,
        ) -> windows::core::Result<()> {
            let controls = self
                .controls
                .as_ref()
                .ok_or_else(|| Error::new(E_FAIL, "SMTC unavailable"))?;
            let updater = controls.DisplayUpdater()?;
            updater.SetThumbnail(image)?;
            updater.Update()
        }
        pub fn update(&self, title: &str, artist: &str, position_ms: u64, duration_ms: u64) {
            if let Err(error) = self.publish(title, artist, position_ms, duration_ms) {
                eprintln!("melos: SMTC update failed: {error}");
            }
        }
        pub fn publish(
            &self,
            title: &str,
            artist: &str,
            position_ms: u64,
            duration_ms: u64,
        ) -> windows::core::Result<()> {
            let Some(controls) = self.controls.as_ref() else {
                return Err(Error::new(E_FAIL, "SMTC unavailable"));
            };
            if let Ok(mut metadata) = self.metadata.lock()
                && metadata.as_ref().is_none_or(|(old_title, old_artist)| {
                    old_title != title || old_artist != artist
                })
            {
                let updater = controls.DisplayUpdater()?;
                let properties = updater.MusicProperties()?;
                properties.SetTitle(&HSTRING::from(title))?;
                properties.SetArtist(&HSTRING::from(artist))?;
                updater.Update()?;
                *metadata = Some((title.to_owned(), artist.to_owned()));
            }
            let mut cached = self
                .timeline
                .lock()
                .map_err(|_| Error::new(E_FAIL, "timeline lock"))?;
            let next = (position_ms.min(duration_ms), duration_ms);
            if cached.as_ref() == Some(&next) {
                return Ok(());
            }
            let ticks = |ms: u64| TimeSpan {
                Duration: ms.saturating_mul(10_000).min(i64::MAX as u64) as i64,
            };
            let timeline = SystemMediaTransportControlsTimelineProperties::new()?;
            timeline.SetStartTime(ticks(0))?;
            timeline.SetMinSeekTime(ticks(0))?;
            timeline.SetPosition(ticks(position_ms.min(duration_ms)))?;
            timeline.SetMaxSeekTime(ticks(duration_ms))?;
            timeline.SetEndTime(ticks(duration_ms))?;
            controls.UpdateTimelineProperties(&timeline)?;
            *cached = Some(next);
            Ok(())
        }
        pub fn set_status(&self, playing: Option<bool>) {
            if let Some(controls) = self.controls.as_ref() {
                let status = match playing {
                    Some(true) => MediaPlaybackStatus::Playing,
                    Some(false) => MediaPlaybackStatus::Paused,
                    None => MediaPlaybackStatus::Stopped,
                };
                if let Ok(mut cached) = self.status.lock() {
                    if cached.as_ref() == Some(&status) {
                        return;
                    }
                    match controls.SetPlaybackStatus(status) {
                        Ok(()) => *cached = Some(status),
                        Err(error) => eprintln!("melos: SMTC status failed: {error}"),
                    }
                }
            }
        }
    }
    impl Drop for Smtc {
        fn drop(&mut self) {
            if let Some(controls) = self.controls.take() {
                if let Some(token) = self.button_token.take() {
                    let _ = controls.RemoveButtonPressed(token);
                }
                let _ = controls.SetIsEnabled(false);
            }
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::MediaCommand;
    pub struct Smtc;
    impl Smtc {
        pub fn attach(_: &gpui::Window) -> Self {
            Self
        }
        pub fn drain(&self) -> Vec<MediaCommand> {
            Vec::new()
        }
        pub fn available(&self) -> bool {
            false
        }
        pub fn update(&self, _: &str, _: &str, _: u64, _: u64) {}
        pub fn set_status(&self, _: Option<bool>) {}
    }
}
pub use platform::Smtc;
#[cfg(windows)]
pub use platform::prepare_artwork;

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn artwork_reference_keeps_encoded_bytes_after_writer_and_stream_drop() {
        let bytes = include_bytes!("../tests/fixtures/cover.png");
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(async {
                let image = super::prepare_artwork(bytes).await.unwrap();
                let stream = image.OpenReadAsync().unwrap().await.unwrap();
                assert_eq!(stream.Size().unwrap(), bytes.len() as u64);
                let reader =
                    windows::Storage::Streams::DataReader::CreateDataReader(&stream).unwrap();
                reader.LoadAsync(bytes.len() as u32).unwrap().await.unwrap();
                let mut restored = vec![0; bytes.len()];
                reader.ReadBytes(&mut restored).unwrap();
                assert_eq!(restored, bytes);
            });
    }
}
