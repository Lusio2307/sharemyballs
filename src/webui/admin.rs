//! The `/admin` control plane — the operator surface that replaced the desktop
//! GUI's buttons (roadmap M2B).
//!
//! Everything here is **fail closed**: `AdminState::new` returns `None` when no
//! `webui.admin_password` is configured, and `webui::serve` then does not
//! register these routes at all, so both `/admin` and `/api/admin/*` 404. The
//! viewer passcode is known to every viewer and deliberately plays no part in
//! authenticating the operator.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};

use crate::auth::ViewerIdentifier;
use crate::session::SessionControl;

/// Longest admin secret we will compare, to bound the work a hostile caller can
/// make us do. Far above any sane passphrase.
const MAX_SECRET_LEN: usize = 1024;

/// State shared by every admin route.
#[derive(Clone)]
pub struct AdminState {
    control: Arc<dyn SessionControl>,
    password: String,
}

impl AdminState {
    /// `None` when no usable secret is configured (absent or blank), which is
    /// what makes the admin surface fail closed.
    pub fn new(control: Arc<dyn SessionControl>, password: Option<String>) -> Option<Self> {
        let password = password.filter(|password| !password.trim().is_empty())?;
        Some(Self { control, password })
    }
}

/// Byte-wise comparison that does not stop at the first difference, so the
/// secret cannot be recovered one byte at a time from response timing. Written
/// out rather than pulled in as a dependency for what is a dozen lines.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let mut difference = a.len() ^ b.len();

    for index in 0..a.len().max(b.len()) {
        let left = a.get(index).copied().unwrap_or(0);
        let right = b.get(index).copied().unwrap_or(0);
        difference |= (left ^ right) as usize;
    }

    difference == 0
}

/// Routes for the admin page and its API. Merged into the main router only when
/// an admin secret is configured.
pub fn router(state: AdminState) -> Router {
    Router::new()
        .route("/admin", get(admin_page))
        .route("/api/admin/state", get(session_state))
        .route("/api/admin/session/start", post(start_session))
        .route("/api/admin/session/stop", post(stop_session))
        .route("/api/admin/viewers/:uuid/accept", post(accept_viewer))
        .route("/api/admin/viewers/:uuid/decline", post(decline_viewer))
        .route("/api/admin/viewers/:uuid/kick", post(kick_viewer))
        .with_state(state)
}

/// The admin route serves the same single-file bundle as the viewer page; the
/// SPA router picks the screen from the path. No secret is in the document.
async fn admin_page() -> impl IntoResponse {
    axum::response::Html(super::INDEX_HTML)
}

/// `Err(401)` unless the request carries the configured admin secret.
fn authorize(state: &AdminState, headers: &HeaderMap) -> Result<(), StatusCode> {
    let presented = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim);

    let authorized = match presented {
        Some(presented) if presented.len() <= MAX_SECRET_LEN => {
            constant_time_eq(presented.as_bytes(), state.password.as_bytes())
        }
        _ => false,
    };

    if authorized {
        Ok(())
    } else {
        // Never log the header or the secret.
        warn!("admin: rejected a request without valid credentials");
        Err(StatusCode::UNAUTHORIZED)
    }
}

fn viewers_json(viewers: &[ViewerIdentifier]) -> Value {
    Value::Array(
        viewers
            .iter()
            .map(|viewer| json!({ "uuid": viewer.uuid, "name": viewer.name }))
            .collect(),
    )
}

/// The whole operator-visible state in one response. The page polls this; there
/// is no push channel yet (roadmap M2B.3).
async fn session_state(State(state): State<AdminState>, headers: HeaderMap) -> Response {
    if let Err(status) = authorize(&state, &headers) {
        return status.into_response();
    }

    let control = &state.control;
    let pending = viewers_json(&control.pending_viewers().await);
    let viewing = viewers_json(&control.viewing_viewers().await);

    Json(json!({
        "running": control.is_running(),
        "room": control.room_id(),
        "password": control.room_password(),
        "inviteLink": control.invite_link(),
        "autoAccept": control.auto_accept(),
        "pending": pending,
        "viewing": viewing,
    }))
    .into_response()
}

async fn start_session(State(state): State<AdminState>, headers: HeaderMap) -> Response {
    if let Err(status) = authorize(&state, &headers) {
        return status.into_response();
    }

    state.control.start_session().await;
    StatusCode::NO_CONTENT.into_response()
}

async fn stop_session(State(state): State<AdminState>, headers: HeaderMap) -> Response {
    if let Err(status) = authorize(&state, &headers) {
        return status.into_response();
    }

    state.control.stop_session().await;
    StatusCode::NO_CONTENT.into_response()
}

/// The uuid arrives from a page that may be minutes stale, so "unknown viewer"
/// is a client error (404), never a panic and never a 500.
fn viewer_error(message: String) -> Response {
    (StatusCode::NOT_FOUND, Json(json!({ "error": message }))).into_response()
}

async fn accept_viewer(
    State(state): State<AdminState>,
    Path(uuid): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(status) = authorize(&state, &headers) {
        return status.into_response();
    }

    match state.control.accept_viewer(uuid.clone()).await {
        Ok(()) => {
            info!("admin: accepted viewer {uuid}");
            StatusCode::NO_CONTENT.into_response()
        }
        Err(message) => viewer_error(message),
    }
}

async fn decline_viewer(
    State(state): State<AdminState>,
    Path(uuid): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(status) = authorize(&state, &headers) {
        return status.into_response();
    }

    match state.control.decline_viewer(uuid.clone()).await {
        Ok(()) => {
            info!("admin: declined viewer {uuid}");
            StatusCode::NO_CONTENT.into_response()
        }
        Err(message) => viewer_error(message),
    }
}

async fn kick_viewer(
    State(state): State<AdminState>,
    Path(uuid): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(status) = authorize(&state, &headers) {
        return status.into_response();
    }

    match state.control.kick_viewer(uuid.clone()).await {
        Ok(()) => {
            info!("admin: kicked viewer {uuid}");
            StatusCode::NO_CONTENT.into_response()
        }
        Err(message) => viewer_error(message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_eq_matches_only_equal_bytes() {
        assert!(constant_time_eq(b"hunter2", b"hunter2"));
        assert!(!constant_time_eq(b"hunter2", b"hunter3"));
        assert!(!constant_time_eq(b"hunter2", b"hunter22"));
        assert!(!constant_time_eq(b"", b"x"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn blank_secret_disables_the_admin_plane() {
        struct NoControl;
        #[async_trait::async_trait]
        impl SessionControl for NoControl {
            fn is_running(&self) -> bool {
                false
            }
            fn room_id(&self) -> Option<String> {
                None
            }
            fn room_password(&self) -> Option<String> {
                None
            }
            fn invite_link(&self) -> Option<String> {
                None
            }
            fn auto_accept(&self) -> bool {
                false
            }
            async fn start_session(&self) {}
            async fn stop_session(&self) {}
            async fn pending_viewers(&self) -> Vec<ViewerIdentifier> {
                Vec::new()
            }
            async fn viewing_viewers(&self) -> Vec<ViewerIdentifier> {
                Vec::new()
            }
            async fn accept_viewer(&self, _uuid: String) -> Result<(), String> {
                Ok(())
            }
            async fn decline_viewer(&self, _uuid: String) -> Result<(), String> {
                Ok(())
            }
            async fn kick_viewer(&self, _uuid: String) -> Result<(), String> {
                Ok(())
            }
        }

        let control = Arc::new(NoControl);
        assert!(AdminState::new(control.clone(), None).is_none());
        assert!(
            AdminState::new(control.clone(), Some(String::new())).is_none(),
            "an empty secret must not enable the admin plane"
        );
        assert!(
            AdminState::new(control.clone(), Some("   ".to_string())).is_none(),
            "a blank secret must not enable the admin plane"
        );
        assert!(AdminState::new(control, Some("s3cret".to_string())).is_some());
    }
}
