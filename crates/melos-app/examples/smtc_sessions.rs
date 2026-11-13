//! Print the media sessions Windows exposes through SMTC.
use std::future::Future;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use windows::Media::Control::GlobalSystemMediaTransportControlsSessionManager;

struct ThreadWaker(std::thread::Thread);

impl Wake for ThreadWaker {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}

fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    let waker = Waker::from(Arc::new(ThreadWaker(std::thread::current())));
    let mut context = Context::from_waker(&waker);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => return value,
            Poll::Pending => std::thread::park(),
        }
    }
}

fn main() -> windows::core::Result<()> {
    let manager =
        block_on(GlobalSystemMediaTransportControlsSessionManager::RequestAsync()?.into_future())?;
    let sessions = manager.GetSessions()?;
    println!("sessions={}", sessions.Size()?);
    for index in 0..sessions.Size()? {
        let session = sessions.GetAt(index)?;
        let properties = block_on(session.TryGetMediaPropertiesAsync()?.into_future())?;
        println!(
            "{} | {} | {} | {:?}",
            session.SourceAppUserModelId()?.to_string_lossy(),
            properties.Title()?.to_string_lossy(),
            properties.Artist()?.to_string_lossy(),
            session.GetPlaybackInfo()?.PlaybackStatus()?
        );
    }
    Ok(())
}
