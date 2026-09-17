use std::collections::HashMap;
use std::sync::Arc;

use anyhow::anyhow;
use async_trait::async_trait;
use rand::distributions::Distribution;
use rand::{thread_rng, Rng};
use tokio::sync::mpsc::Sender;
use tokio::sync::Mutex;

use crate::output::WebRTCOutput;
use crate::signaller::{AuthenticationPayload, DeclineReason};
use crate::Result;

#[async_trait]
pub trait Authenticator: Send + Sync {
    /// Return None if authentication is successful
    /// Return Some(reason) if authentication is unsuccessful
    async fn authenticate(
        &self,
        uuid: String,
        name: String,
        payload: &AuthenticationPayload,
    ) -> Option<DeclineReason>;
}

#[derive(Clone, Debug)]
pub struct ViewerIdentifier {
    pub uuid: String,
    pub name: String,
}

pub struct PasswordAuthenticator {
    password: String,
}

fn random_user_friendly_string(len: usize) -> String {
    pub struct UserFriendlyAlphabet;
    impl Distribution<u8> for UserFriendlyAlphabet {
        fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> u8 {
            const GEN_ASCII_STR_CHARSET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
            GEN_ASCII_STR_CHARSET[(rng.next_u32() >> (32 - 5)) as usize]
        }
    }

    thread_rng()
        .sample_iter(&UserFriendlyAlphabet)
        .take(len)
        .map(char::from)
        .collect()
}

impl PasswordAuthenticator {
    pub fn new(password: String) -> Result<Self> {
        if password.is_empty() {
            return Err(anyhow!("Password cannot be empty"));
        }
        Ok(Self { password })
    }

    pub fn random() -> Result<Self> {
        Self::new(random_user_friendly_string(5))
    }

    pub fn password(&self) -> String {
        self.password.clone()
    }
}

#[async_trait]
impl Authenticator for PasswordAuthenticator {
    async fn authenticate(
        &self,
        _uuid: String,
        _name: String,
        payload: &AuthenticationPayload,
    ) -> Option<DeclineReason> {
        match payload {
            AuthenticationPayload::Password { password } => {
                if *password == self.password {
                    None
                } else {
                    Some(DeclineReason::IncorrectPassword)
                }
            }
            _ => Some(DeclineReason::NoCredentials),
        }
    }
}

pub struct ViewerManager {
    viewing_viewers: Mutex<Vec<ViewerIdentifier>>,
    pending_viewers: Mutex<Vec<ViewerIdentifier>>,
    auth_result_senders: Mutex<HashMap<String, Sender<bool>>>,
    notify_update: Arc<dyn Fn() + Send + Sync>,
    webrtc_output: Mutex<Option<Arc<Mutex<WebRTCOutput>>>>,
    /// Admit viewers without waiting for a GUI decision. Needed for unattended
    /// operation. The password check still runs first -- this removes the human
    /// click, it does not bypass authentication.
    auto_accept: bool,
}

impl ViewerManager {
    pub fn new(notify_update: Arc<dyn Fn() + Send + Sync>, auto_accept: bool) -> ViewerManager {
        ViewerManager {
            viewing_viewers: Mutex::new(Vec::new()),
            pending_viewers: Mutex::new(Vec::new()),
            auth_result_senders: Mutex::new(HashMap::new()),
            notify_update,
            webrtc_output: Mutex::new(None),
            auto_accept,
        }
    }
    pub async fn get_viewing_viewers(&self) -> Vec<ViewerIdentifier> {
        self.viewing_viewers.lock().await.clone()
    }
    pub async fn get_pending_viewers(&self) -> Vec<ViewerIdentifier> {
        self.pending_viewers.lock().await.clone()
    }
    /// Hand the operator's decision to the waiting `authenticate` future.
    ///
    /// Returns `Err` when the viewer is no longer waiting -- it left, was
    /// already decided, or the session stopped. The GUI only ever offered a
    /// decision for a viewer it had just listed, but an admin page can hold a
    /// stale list for minutes, so this has to be a recoverable error rather
    /// than a panic.
    async fn send_viewer_auth_result(
        &self,
        viewer: ViewerIdentifier,
        permit: bool,
    ) -> std::result::Result<(), String> {
        // Clone the sender out of the map so the lock is not held across the
        // `await` below.
        let sender = self
            .auth_result_senders
            .lock()
            .await
            .get(&viewer.uuid)
            .cloned();

        let Some(sender) = sender else {
            return Err(format!(
                "{} is no longer waiting for a decision",
                viewer.uuid
            ));
        };

        sender
            .send(permit)
            .await
            .map_err(|_| format!("{} is no longer connected", viewer.uuid))
    }
    pub async fn permit_viewer(&self, viewer: ViewerIdentifier) -> std::result::Result<(), String> {
        self.send_viewer_auth_result(viewer, true).await?;
        (self.notify_update)();
        Ok(())
    }
    pub async fn decline_viewer(
        &self,
        viewer: ViewerIdentifier,
    ) -> std::result::Result<(), String> {
        self.send_viewer_auth_result(viewer, false).await?;
        (self.notify_update)();
        Ok(())
    }
    pub async fn clear(&self) {
        self.viewing_viewers.lock().await.clear();
        self.pending_viewers.lock().await.clear();
        self.auth_result_senders.lock().await.clear();
        self.webrtc_output.lock().await.take();
    }
    pub async fn kick_viewer(&self, viewer: ViewerIdentifier) {
        let output = self.webrtc_output.lock().await;
        if let Some(output) = output.as_ref() {
            output.lock().await.kick_peer(&viewer.uuid).await;
            self.viewer_left(&viewer.uuid).await;
        }
    }

    pub async fn viewer_left(&self, viewer_uuid: &String) {
        self.viewing_viewers
            .lock()
            .await
            .retain(|v| v.uuid != *viewer_uuid);
        self.pending_viewers
            .lock()
            .await
            .retain(|v| v.uuid != *viewer_uuid);
        (self.notify_update)();
    }

    pub async fn set_webrtc_output(&self, output: Arc<Mutex<WebRTCOutput>>) {
        *self.webrtc_output.lock().await = Some(output);
    }
}

#[async_trait]
impl Authenticator for ViewerManager {
    async fn authenticate(
        &self,
        uuid: String,
        name: String,
        _payload: &AuthenticationPayload,
    ) -> Option<DeclineReason> {
        let viewer = ViewerIdentifier {
            uuid: uuid.clone(),
            name,
        }; // todo: get name

        // Unattended mode: admit straight away. `ComplexAuthenticator` runs the
        // password check before this, so reaching here already means the viewer
        // presented valid credentials.
        if self.auto_accept {
            info!("{} auto-accepted (auto_accept is enabled)", uuid);
            self.viewing_viewers.lock().await.push(viewer);
            (self.notify_update)();
            return None;
        }

        self.pending_viewers.lock().await.push(viewer.clone());
        let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
        self.auth_result_senders
            .lock()
            .await
            .insert(uuid.clone(), sender.clone());
        // wait for a decision
        info!("{} is waiting for authentication", uuid);
        (self.notify_update)();
        let decision = receiver.recv().await.unwrap();
        self.auth_result_senders.lock().await.remove(&uuid);
        self.pending_viewers.lock().await.retain(|v| v.uuid != uuid);
        info!("{} got authentication decision: {}", uuid, decision);
        if decision {
            self.viewing_viewers.lock().await.push(viewer);
            None
        } else {
            Some(DeclineReason::UserDeclined)
        }
    }
}

pub struct ComplexAuthenticator {
    authenticators: Vec<Arc<dyn Authenticator>>,
}

impl ComplexAuthenticator {
    pub fn new(authenticators: Vec<Arc<dyn Authenticator>>) -> Self {
        Self { authenticators }
    }
}

#[async_trait]
impl Authenticator for ComplexAuthenticator {
    async fn authenticate(
        &self,
        uuid: String,
        name: String,
        payload: &AuthenticationPayload,
    ) -> Option<DeclineReason> {
        for authenticator in &self.authenticators {
            if let Some(reason) = authenticator
                .authenticate(uuid.clone(), name.clone(), payload)
                .await
            {
                return Some(reason);
            }
        }
        None
    }
}
