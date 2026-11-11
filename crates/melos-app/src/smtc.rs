//! Windows System Media Transport Controls integration.
//!
//! Registers the app window with SMTC so the volume overlay, the media keys
//! (play/pause, next, previous) and the lock screen show what is playing.
//! On non-Windows builds this module compiles to nothing and the app plays
//! without system integration.

use std::sync::Arc;

use gpui::Window;
use windows::core::HSTRING;
use windows::Foundation::TypedEventHandler;
use windows::Media::{
    MediaPlaybackStatus, MediaPlaybackType, MusicDisplayProperties,
    SystemMediaTransportControls, SystemMediaTransportControlsButton,
    SystemMediaTransportControlsButtonPressedEventArgs,
    SystemMediaTransportControlsTimelineProperties,
};

/// Commands SMTC asks the app to run. Drained by the UI loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaCommand {
    Play,
    Pause,
    Next,
    Previous,
}

#[derive(Default)]
pub struct Smtc {
    /// Commands queued by the system, newest last.
    commands: Arc<std::sync::Mutex<Vec<MediaCommand>>>,
    controls: Option<SystemMediaTransportControls>,
}

impl Smtc {
    /// Attaches SMTC to the given window. Must run on the UI thread.
    pub fn attach(window: &Window) -> Self {
        let mut smtc = Self::default();
        if let Err(error) = smtc.bind(window) {
            tracing::warn!("SMTC unavailable: {error}");
        }
        smtc
    }

    #[cfg(target_os = "windows")]
    fn bind(&mut self, window: &Window) -> windows::core::Result<()> {
        // GetForCurrentView reads the current app view; GPUI threads its HWND
        // through the window handle, so re-attach on the window's thread.
        let controls = SystemMediaTransportControls::GetForCurrentView()?;
        controls.SetIsEnabled(true)?;
        controls
            .DisplayUpdater()?
            .SetType(MediaPlaybackType::Music)?;

        let commands = self.commands.clone();
        controls.ButtonPressed(&TypedEventHandler::<
            SystemMediaTransportControls,
            SystemMediaTransportControlsButtonPressedEventArgs,
        >::new(move |_, args| {
            if let Some(args) = args.as_ref() {
                let button = args.Button()?;
                let command = match button {
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
        }));

        self.controls = Some(controls);
        Ok(())
    }

    #[cfg(not(target_os = "windows"))]
    fn bind(&mut self, _window: &Window) -> windows::core::Result<()> {
        Ok(())
    }

    /// Removes and returns every queued command.
    pub fn drain(&self) -> Vec<MediaCommand> {
        self.commands
            .lock()
            .map(|mut queue| queue.drain(..).collect())
            .unwrap_or_default()
    }

    /// Publishes the current track metadata and timeline.
    pub fn update(&self, title: &str, artist: &str, position_ms: u64, duration_ms: u64) {
        #[cfg(target_os = "windows")]
        if let Some(controls) = self.controls.as_ref() {
            let _ = self.publish(controls, title, artist, position_ms, duration_ms);
        }
        #[cfg(not(target_os = "windows"))]
        let _ = (title, artist, position_ms, duration_ms);
    }

    /// Reflects the playback state (playing / paused / stopped).
    pub fn set_status(&self, playing: Option<bool>) {
        #[cfg(target_os = "windows")]
        if let Some(controls) = self.controls.as_ref() {
            let status = match playing {
                Some(true) => MediaPlaybackStatus::Playing,
                Some(false) => MediaPlaybackStatus::Paused,
                None => MediaPlaybackStatus::Stopped,
            };
            let _ = controls.SetPlaybackStatus(status);
        }
        #[cfg(not(target_os = "windows"))]
        let _ = playing;
    }
}

#[cfg(target_os = "windows")]
impl Smtc {
    fn publish(
        &self,
        controls: &SystemMediaTransportControls,
        title: &str,
        artist: &str,
        position_ms: u64,
        duration_ms: u64,
    ) -> windows::core::Result<()> {
        let properties: MusicDisplayProperties = controls.DisplayUpdater()?.MusicProperties()?;
        properties.SetTitle(&HSTRING::from(title))?;
        properties.SetArtist(&HSTRING::from(artist))?;
        controls.DisplayUpdater()?.Update()?;

        let timeline = SystemMediaTransportControlsTimelineProperties::new()?;
        timeline.SetPosition(windows::Foundation::TimeSpan {
            // 100ns ticks, the WinRT media clock unit.
            Duration: (position_ms * 10_000) as i64,
        });
        timeline.SetEndTime(windows::Foundation::TimeSpan {
            Duration: (duration_ms * 10_000) as i64,
        });
        controls.UpdateTimelineProperties(&timeline)?;
        Ok(())
    }
}
