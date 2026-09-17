//! The control plane behind the web admin page.
//!
//! `webui` deliberately does not depend on `capture`/`auth` directly: the HTTP
//! layer drives this trait, so the routes can be tested against a fake without a
//! capture backend, and the app side can be restructured (or replaced, when the
//! desktop GUI goes away) without touching the server.

use std::sync::{Arc, Mutex, MutexGuard};

use async_trait::async_trait;

use crate::auth::{ViewerIdentifier, ViewerManager};
use crate::capture::capturer::Capturer;

/// Everything the admin page can do, in terms an HTTP handler can call.
#[async_trait]
pub trait SessionControl: Send + Sync {
    /// Whether a capture session is currently running.
    fn is_running(&self) -> bool;
    fn room_id(&self) -> Option<String>;
    fn room_password(&self) -> Option<String>;
    /// Viewer-facing invite URL (room + passcode), once a session is live.
    fn invite_link(&self) -> Option<String>;
    /// Whether viewers are admitted without an explicit decision.
    fn auto_accept(&self) -> bool;
    async fn start_session(&self);
    async fn stop_session(&self);
    async fn pending_viewers(&self) -> Vec<ViewerIdentifier>;
    async fn viewing_viewers(&self) -> Vec<ViewerIdentifier>;
    /// `Err` when the viewer is not (or is no longer) waiting for a decision.
    async fn accept_viewer(&self, uuid: String) -> Result<(), String>;
    async fn decline_viewer(&self, uuid: String) -> Result<(), String>;
    /// `Err` when the viewer is not currently viewing.
    async fn kick_viewer(&self, uuid: String) -> Result<(), String>;
}

/// Adapter over the running application: the capture session plus the viewer
/// lists.
pub struct AppControl {
    /// `std::sync::Mutex` on purpose. Every `Capturer` method used here is
    /// synchronous, so no guard is ever held across an `await` and the iced
    /// `view()` can lock it without entering the async runtime. (The one async
    /// path, `kick`, takes the signaller handle out first -- see
    /// `Capturer::signaller_handle`.)
    capturer: Arc<Mutex<Capturer>>,
    viewer_manager: Arc<ViewerManager>,
}

impl AppControl {
    pub fn new(capturer: Arc<Mutex<Capturer>>, viewer_manager: Arc<ViewerManager>) -> Self {
        Self {
            capturer,
            viewer_manager,
        }
    }

    fn capturer(&self) -> MutexGuard<'_, Capturer> {
        match self.capturer.lock() {
            Ok(guard) => guard,
            // Recover rather than propagate: a panic elsewhere while holding
            // this lock must not brick the operator surface for good.
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    /// A viewer that is waiting for a decision, or an error naming the uuid.
    async fn pending_viewer(&self, uuid: &str) -> Result<ViewerIdentifier, String> {
        self.viewer_manager
            .get_pending_viewers()
            .await
            .into_iter()
            .find(|viewer| viewer.uuid == uuid)
            .ok_or_else(|| format!("{uuid} is not waiting for a decision"))
    }

    /// A viewer that is currently watching, or an error naming the uuid.
    async fn viewing_viewer(&self, uuid: &str) -> Result<ViewerIdentifier, String> {
        self.viewer_manager
            .get_viewing_viewers()
            .await
            .into_iter()
            .find(|viewer| viewer.uuid == uuid)
            .ok_or_else(|| format!("{uuid} is not viewing"))
    }
}

#[async_trait]
impl SessionControl for AppControl {
    fn is_running(&self) -> bool {
        self.capturer().is_running()
    }

    fn room_id(&self) -> Option<String> {
        self.capturer().get_room_id()
    }

    fn room_password(&self) -> Option<String> {
        self.capturer().get_room_password()
    }

    fn invite_link(&self) -> Option<String> {
        self.capturer().get_invite_link()
    }

    fn auto_accept(&self) -> bool {
        self.capturer().config.auto_accept
    }

    async fn start_session(&self) {
        {
            let mut capturer = self.capturer();
            // A double-clicked Start must not spawn a second capture task.
            if capturer.is_running() {
                info!("admin: start requested while already sharing; ignoring");
                return;
            }
            info!("admin: starting capture session");
            capturer.run();
        }
    }

    async fn stop_session(&self) {
        {
            let mut capturer = self.capturer();
            info!("admin: stopping capture session");
            capturer.shutdown();
        }
    }

    async fn pending_viewers(&self) -> Vec<ViewerIdentifier> {
        self.viewer_manager.get_pending_viewers().await
    }

    async fn viewing_viewers(&self) -> Vec<ViewerIdentifier> {
        self.viewer_manager.get_viewing_viewers().await
    }

    async fn accept_viewer(&self, uuid: String) -> Result<(), String> {
        let viewer = self.pending_viewer(&uuid).await?;
        self.viewer_manager.permit_viewer(viewer).await
    }

    async fn decline_viewer(&self, uuid: String) -> Result<(), String> {
        let viewer = self.pending_viewer(&uuid).await?;
        self.viewer_manager.decline_viewer(viewer).await
    }

    async fn kick_viewer(&self, uuid: String) -> Result<(), String> {
        let viewer = self.viewing_viewer(&uuid).await?;

        // Step 1: tell the signaller, so the viewer's page sees the room close.
        // `signaller_handle` is taken out of the guard first, so no non-`Send`
        // `MutexGuard` is alive across this await.
        let signaller = self.capturer().signaller_handle();
        let kicked = viewer.uuid.clone();
        if let Some(signaller) = signaller {
            signaller.kick_viewer(kicked).await;
        } else {
            warn!("admin: no signaller to kick {kicked} through; dropping the peer only");
        }

        // Step 2: drop the WebRTC peer and the list entry, exactly as the GUI
        // does. `ViewerManager::kick_viewer`'s future is not `Send` -- it holds
        // a `std::sync::MutexGuard` from inside webrtc's `close()` across an
        // await -- so it cannot be awaited directly from a handler that must
        // produce a `Send` future. `block_in_place` + `block_on` drives it on
        // this worker thread instead; the inner future never becomes part of the
        // outer future's state. Same pattern the GUI uses.
        let viewer_manager = self.viewer_manager.clone();
        tokio::task::block_in_place(move || {
            tokio::runtime::Handle::current().block_on(async move {
                viewer_manager.kick_viewer(viewer).await;
            })
        });

        Ok(())
    }
}
