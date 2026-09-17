//! Embedded local WebUI + signalling server.
//!
//! Serves a local viewer page (`GET /`) and implements the mirashare
//! signalling protocol (`GET /signaller`, WebSocket) as a room router
//! between the sharer's `WebSocketSignaller` (client) and browser viewers.
//! Messages are relayed as raw JSON; only the `type`/`from`/`to`/`room`
//! envelope fields are parsed for routing.
//!
//! When `webui.admin_password` is configured it also serves the operator
//! control plane (`GET /admin` and `/api/admin/*`), implemented in `admin.rs`.
//! Without a secret those routes are not registered at all.

mod admin;

pub use admin::AdminState;

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::Html;
use axum::routing::get;
use axum::Router;
use serde::Deserialize;
use tokio::net::TcpListener;
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
use uuid::Uuid;

/// The built SPA, inlined into the binary so the packaged `.exe`/`.app` needs
/// no runtime file next to it. `webui/sharemyballs-webui/dist/` is produced by
/// `bun run build`; see `build.rs` for the check that it exists.
pub(crate) const INDEX_HTML: &str = include_str!("../../webui/sharemyballs-webui/dist/index.html");

const IDLE_TIMEOUT: Duration = Duration::from_secs(90);
const REAP_INTERVAL: Duration = Duration::from_secs(10);

static NEXT_CONN_ID: AtomicUsize = AtomicUsize::new(0);

/// Start the embedded webui + signalling server on `bind`:`port`.
/// Runs until the process exits.
///
/// `admin` carries the `/admin` control plane. Pass `None` when no
/// `webui.admin_password` is configured: the admin routes are then not served
/// at all (fail closed), rather than served and checked.
pub async fn start(bind: &str, port: u16, admin: Option<AdminState>) -> std::io::Result<()> {
    let listener = TcpListener::bind((bind, port)).await?;
    info!("WebUI listening on http://{bind}:{port}");
    if admin.is_some() {
        info!("WebUI admin control plane enabled at http://{bind}:{port}/admin");
    }
    serve(listener, admin).await
}

async fn serve(listener: TcpListener, admin: Option<AdminState>) -> std::io::Result<()> {
    let state = Arc::new(SignallerState::default());
    {
        let state = state.clone();
        tokio::spawn(reap_idle(state));
    }

    let mut router = Router::new()
        .route("/", get(|| async { Html(INDEX_HTML) }))
        .route(
            "/signaller",
            get(move |ws: WebSocketUpgrade| {
                let state = state.clone();
                async move { ws.on_upgrade(move |socket| handle_ws(state, socket)) }
            }),
        );

    if let Some(admin) = admin {
        router = router.merge(admin::router(admin));
    }

    axum::serve(listener, router).await
}

struct Room {
    sharer_id: usize,
    sharer: UnboundedSender<String>,
    viewers: Vec<String>,
}

struct Viewer {
    id: usize,
    sender: UnboundedSender<String>,
    room: String,
}

#[derive(Default)]
struct SignallerState {
    rooms: Mutex<HashMap<String, Room>>,
    viewers: Mutex<HashMap<String, Viewer>>,
    last_activity: Mutex<HashMap<usize, Instant>>,
}

impl SignallerState {
    /// Drop all registrations for a connection that went away (explicit
    /// leave, WS drop, or idle timeout). If the connection was the room's
    /// sharer, its viewers are told the room is closed.
    fn cleanup(&self, id: usize) {
        self.last_activity.lock().unwrap().remove(&id);

        let gone_rooms: Vec<(String, Vec<String>)> = {
            let mut rooms = self.rooms.lock().unwrap();
            let gone: Vec<(String, Vec<String>)> = rooms
                .iter()
                .filter(|(_, r)| r.sharer_id == id)
                .map(|(k, r)| (k.clone(), r.viewers.clone()))
                .collect();
            rooms.retain(|_, r| r.sharer_id != id);
            gone
        };
        for (room, viewers) in gone_rooms {
            for viewer in &viewers {
                let msg = serde_json::json!({
                    "type": "room_closed",
                    "to": viewer,
                    "room": room,
                })
                .to_string();
                if let Some(v) = self.viewers.lock().unwrap().get(viewer) {
                    let _ = v.sender.send(msg);
                }
            }
        }

        self.viewers.lock().unwrap().retain(|_, v| v.id != id);
    }
}

/// Routing envelope — the only fields the router needs from a message.
#[derive(Deserialize)]
struct Envelope {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    from: Option<String>,
    #[serde(default)]
    to: Option<String>,
    #[serde(default)]
    room: Option<String>,
}

async fn handle_ws(state: Arc<SignallerState>, mut ws: WebSocket) {
    let id = NEXT_CONN_ID.fetch_add(1, Ordering::Relaxed);
    let (tx, mut rx) = unbounded_channel::<String>();
    let mut tx = Some(tx);
    state
        .last_activity
        .lock()
        .unwrap()
        .insert(id, Instant::now());

    loop {
        tokio::select! {
            out = rx.recv() => {
                match out {
                    Some(text) => {
                        if ws.send(Message::Text(text)).await.is_err() {
                            break;
                        }
                    }
                    // our registration was dropped (cleanup) — stop
                    None => break,
                }
            }
            msg = ws.recv() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        state
                            .last_activity
                            .lock()
                            .unwrap()
                            .insert(id, Instant::now());
                        if let Some(reply) = route(&state, id, &mut tx, &text) {
                            if ws.send(Message::Text(reply)).await.is_err() {
                                break;
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {} // ping/pong/binary — ignore
                    Some(Err(e)) => {
                        warn!("webui signaller: connection error: {e}");
                        break;
                    }
                }
            }
        }
    }

    state.cleanup(id);
}

/// Route one inbound message. Returns `Some(text)` for replies that must go
/// directly back to the sender (`start_response`, `join_declined`,
/// `ice_servers_response`).
fn route(
    state: &SignallerState,
    id: usize,
    tx: &mut Option<UnboundedSender<String>>,
    text: &str,
) -> Option<String> {
    let envelope: Envelope = match serde_json::from_str(text) {
        Ok(e) => e,
        Err(e) => {
            warn!("webui signaller: invalid message ({e}): {text}");
            return None;
        }
    };

    match envelope.kind.as_str() {
        // sharer: begin a session
        "start" => {
            let Some(tx) = tx.take() else { return None };
            // Honour a requested room so a bookmarked invite URL keeps working
            // across restarts. `None` (the historical behaviour) still gets a
            // random room.
            let requested = envelope.room.filter(|room| !room.trim().is_empty());
            let room = match requested {
                Some(room) if !state.rooms.lock().unwrap().contains_key(&room) => room,
                Some(room) => {
                    warn!(
                        "webui signaller: requested room {room} is already in use; assigning a random room"
                    );
                    Uuid::new_v4().to_string()
                }
                None => Uuid::new_v4().to_string(),
            };
            state.rooms.lock().unwrap().insert(
                room.clone(),
                Room {
                    sharer_id: id,
                    sharer: tx,
                    viewers: vec![],
                },
            );
            Some(
                serde_json::json!({
                    "type": "start_response",
                    "room": room,
                })
                .to_string(),
            )
        }
        // viewer: join a room — relay as-is to the sharer
        "join" => {
            let (Some(room), Some(from)) = (envelope.room, envelope.from) else {
                return None;
            };
            let sharer = match state.rooms.lock().unwrap().get_mut(&room) {
                Some(r) => {
                    r.viewers.push(from.clone());
                    r.sharer.clone()
                }
                // unknown room / no active session
                None => {
                    return Some(
                        serde_json::json!({
                            "type": "join_declined",
                            "reason": 0,
                            "to": from,
                        })
                        .to_string(),
                    )
                }
            };
            let Some(tx) = tx.take() else { return None };
            state.viewers.lock().unwrap().insert(
                from,
                Viewer {
                    id,
                    sender: tx,
                    room,
                },
            );
            let _ = sharer.send(text.to_string());
            None
        }
        // sharer → viewer
        "offer" => {
            let Some(to) = envelope.to else { return None };
            if let Some(v) = state.viewers.lock().unwrap().get(&to) {
                let _ = v.sender.send(text.to_string());
            }
            None
        }
        // either direction — rooms and viewer UUIDs are both random v4,
        // so check viewers first (a viewer's `to` is a viewer UUID)
        "ice" => {
            let Some(to) = envelope.to else { return None };
            let viewers = state.viewers.lock().unwrap();
            if let Some(v) = viewers.get(&to) {
                let _ = v.sender.send(text.to_string());
            } else {
                drop(viewers);
                if let Some(r) = state.rooms.lock().unwrap().get(&to) {
                    let _ = r.sharer.send(text.to_string());
                }
            }
            None
        }
        // viewer → sharer (viewer's `to` is the room id)
        "answer" => {
            let Some(to) = envelope.to else { return None };
            if let Some(r) = state.rooms.lock().unwrap().get(&to) {
                let _ = r.sharer.send(text.to_string());
            }
            None
        }
        "leave" => {
            let Some(from) = envelope.from else {
                return None;
            };
            if state.rooms.lock().unwrap().contains_key(&from) {
                // sharer left: close the room, notify its viewers
                let Some(room) = state.rooms.lock().unwrap().remove(&from) else {
                    return None;
                };
                for viewer in &room.viewers {
                    let msg = serde_json::json!({
                        "type": "room_closed",
                        "to": viewer,
                        "room": from,
                    })
                    .to_string();
                    if let Some(v) = state.viewers.lock().unwrap().get(viewer) {
                        let _ = v.sender.send(msg);
                    }
                }
                let viewers = room.viewers;
                let mut viewers_map = state.viewers.lock().unwrap();
                for viewer in viewers {
                    viewers_map.remove(&viewer);
                }
            } else if let Some(v) = state.viewers.lock().unwrap().remove(&from) {
                // viewer left: relay `leave { from }` to the sharer
                if let Some(r) = state.rooms.lock().unwrap().get(&v.room) {
                    let _ = r.sharer.send(text.to_string());
                }
            }
            None
        }
        // sharer → addressed viewer (kick / decline)
        "join_declined" | "room_closed" => {
            let Some(to) = envelope.to else { return None };
            let viewers = state.viewers.lock().unwrap();
            if let Some(v) = viewers.get(&to) {
                let _ = v.sender.send(text.to_string());
            }
            drop(viewers);
            state.viewers.lock().unwrap().remove(&to);
            None
        }
        "ice_servers" => Some(
            serde_json::json!({
                "type": "ice_servers_response",
                "ice_servers": [],
            })
            .to_string(),
        ),
        "keep_alive" => None,
        other => {
            trace!("webui signaller: unhandled message type: {other}");
            None
        }
    }
}

/// Drop connections idle for longer than `IDLE_TIMEOUT`.
async fn reap_idle(state: Arc<SignallerState>) {
    let mut ticker = tokio::time::interval(REAP_INTERVAL);
    loop {
        ticker.tick().await;
        let stale: Vec<usize> = state
            .last_activity
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, t)| t.elapsed() > IDLE_TIMEOUT)
            .map(|(id, _)| *id)
            .collect();
        for id in stale {
            debug!("webui signaller: dropping idle connection {id}");
            state.cleanup(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::ViewerIdentifier;
    use crate::session::SessionControl;
    use futures_util::{SinkExt, StreamExt};
    use serde_json::{json, Value};
    use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};

    /// The admin secret used by the tests below.
    const SECRET: &str = "s3cret-admin";

    type Conn = tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >;

    async fn recv_json(ws: &mut Conn) -> Value {
        let msg = ws.next().await.unwrap().unwrap();
        serde_json::from_str(&msg.into_text().unwrap()).unwrap()
    }

    async fn send_json(ws: &mut Conn, value: Value) {
        ws.send(WsMessage::text(value.to_string())).await.unwrap();
    }

    /// Full relay roundtrip: start → join → offer → answer/ice → leave.
    #[tokio::test]
    async fn signaller_relay_roundtrip() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(serve(listener, None));

        let url = format!("ws://127.0.0.1:{port}/signaller");
        let (mut sharer, _) = connect_async(&url).await.unwrap();
        let (mut viewer, _) = connect_async(&url).await.unwrap();

        // sharer starts a session
        send_json(&mut sharer, json!({"type": "start"})).await;
        let resp = recv_json(&mut sharer).await;
        assert_eq!(resp["type"], "start_response");
        let room = resp["room"].as_str().unwrap().to_string();
        assert!(!room.is_empty());

        // viewer joins (unknown extra fields are relayed as-is to the sharer)
        send_json(
            &mut viewer,
            json!({"type": "join", "room": room, "from": "viewer-1",
                   "name": "WebUI", "auth": {"type": "password", "password": "x"}}),
        )
        .await;
        let join = recv_json(&mut sharer).await;
        assert_eq!(join["type"], "join");
        assert_eq!(join["from"], "viewer-1");
        assert_eq!(join["room"], room);

        // sharer → viewer: offer
        send_json(
            &mut sharer,
            json!({"type": "offer", "sdp": {"type": "offer", "sdp": "v=0"},
                   "from": "0", "to": "viewer-1", "ice_servers": []}),
        )
        .await;
        let offer = recv_json(&mut viewer).await;
        assert_eq!(offer["type"], "offer");
        assert_eq!(offer["sdp"]["sdp"], "v=0");

        // viewer → sharer: answer (addressed to the room) + ice
        send_json(
            &mut viewer,
            json!({"type": "answer", "sdp": {"type": "answer", "sdp": "v=0"},
                   "from": "viewer-1", "to": room}),
        )
        .await;
        let answer = recv_json(&mut sharer).await;
        assert_eq!(answer["type"], "answer");
        assert_eq!(answer["sdp"]["sdp"], "v=0");

        send_json(
            &mut viewer,
            json!({"type": "ice", "ice": {"candidate": "cand-v"},
                   "from": "viewer-1", "to": room}),
        )
        .await;
        let ice_to_sharer = recv_json(&mut sharer).await;
        assert_eq!(ice_to_sharer["type"], "ice");
        assert_eq!(ice_to_sharer["ice"]["candidate"], "cand-v");

        // sharer → viewer: ice
        send_json(
            &mut sharer,
            json!({"type": "ice", "ice": {"candidate": "cand-s"},
                   "from": "0", "to": "viewer-1"}),
        )
        .await;
        let ice_to_viewer = recv_json(&mut viewer).await;
        assert_eq!(ice_to_viewer["type"], "ice");
        assert_eq!(ice_to_viewer["ice"]["candidate"], "cand-s");

        // sharer leaves → viewer is told the room closed
        send_json(&mut sharer, json!({"type": "leave", "from": room})).await;
        let closed = recv_json(&mut viewer).await;
        assert_eq!(closed["type"], "room_closed");
        assert_eq!(closed["room"], room);
        assert_eq!(closed["to"], "viewer-1");

        // joining a closed/unknown room is declined
        let (mut late, _) = connect_async(&url).await.unwrap();
        send_json(
            &mut late,
            json!({"type": "join", "room": "nonexistent", "from": "viewer-2",
                   "name": "x", "auth": {"type": "none"}}),
        )
        .await;
        let declined = recv_json(&mut late).await;
        assert_eq!(declined["type"], "join_declined");
        assert_eq!(declined["to"], "viewer-2");

        // ice_servers → empty response
        let (mut probe, _) = connect_async(&url).await.unwrap();
        send_json(&mut probe, json!({"type": "ice_servers"})).await;
        let ice_resp = recv_json(&mut probe).await;
        assert_eq!(ice_resp["type"], "ice_servers_response");
        assert!(ice_resp["ice_servers"].as_array().unwrap().is_empty());

        // index page is served
        let body = reqwest_free_get(port).await;
        assert!(body.contains("Mira Sharer"));
        // `INDEX_HTML` must be the *built* page. If the Vite dev template gets
        // embedded instead, `/src/main.tsx` is referenced but never served, so
        // the browser shows a blank page while this test still passes.
        assert!(
            !body.contains("/src/main.tsx"),
            "GET / served the unbundled Vite entry (`/src/main.tsx`): \
             webui/sharemyballs-webui/dist/index.html is a dev template, not a build. \
             Run `bun run build` in webui/sharemyballs-webui."
        );
        // Chakra's semantic tokens (`fg`, `bg`, `border`, ...) resolve to their
        // *light* values unless `.dark` is on an ancestor, and the app paints a
        // black background -- so dropping the class renders near-black text on
        // black. Nothing else catches that: it type-checks, lints and builds
        // cleanly.
        assert!(
            body.contains("class=\"dark\""),
            "GET / served a page without `class=\"dark\"` on <html>: every Chakra \
             semantic token would fall back to its light value (dark text on the \
             black background). See index.html; run `bun run build` in \
             webui/sharemyballs-webui."
        );

        server.abort();
    }

    /// A requested room is used verbatim, which is what lets a bookmarked
    /// invite URL keep working across sharer restarts.
    #[tokio::test]
    async fn start_honours_requested_room() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(serve(listener, None));

        let url = format!("ws://127.0.0.1:{port}/signaller");
        let (mut sharer, _) = connect_async(&url).await.unwrap();

        send_json(&mut sharer, json!({"type": "start", "room": "desk"})).await;
        let resp = recv_json(&mut sharer).await;
        assert_eq!(resp["type"], "start_response");
        assert_eq!(resp["room"], "desk");

        server.abort();
    }

    /// Two sharers asking for the same room must not collide. The second gets a
    /// generated room rather than hijacking the first one's viewers.
    #[tokio::test]
    async fn start_falls_back_when_requested_room_is_taken() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(serve(listener, None));

        let url = format!("ws://127.0.0.1:{port}/signaller");
        let (mut first, _) = connect_async(&url).await.unwrap();
        send_json(&mut first, json!({"type": "start", "room": "desk"})).await;
        assert_eq!(recv_json(&mut first).await["room"], "desk");

        let (mut second, _) = connect_async(&url).await.unwrap();
        send_json(&mut second, json!({"type": "start", "room": "desk"})).await;
        let resp = recv_json(&mut second).await;
        assert_eq!(resp["type"], "start_response");
        assert_ne!(
            resp["room"], "desk",
            "second sharer must not be handed a room that is already live"
        );
        assert!(!resp["room"].as_str().unwrap_or_default().is_empty());

        server.abort();
    }

    /// A blank requested room counts as "assign one", so a stray `room = ""` in
    /// config.toml cannot create an unusable session.
    #[tokio::test]
    async fn blank_requested_room_is_assigned_one() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(serve(listener, None));

        let url = format!("ws://127.0.0.1:{port}/signaller");
        let (mut sharer, _) = connect_async(&url).await.unwrap();

        send_json(&mut sharer, json!({"type": "start", "room": "   "})).await;
        let resp = recv_json(&mut sharer).await;
        assert_eq!(resp["type"], "start_response");
        assert!(!resp["room"].as_str().unwrap_or_default().is_empty());
        assert_ne!(resp["room"], "   ");

        server.abort();
    }

    // The old `viewer_page_reads_kind_from_the_track` test asserted on the
    // hand-written viewer page's source. That page is gone: the page is now a
    // bundled React app, and grepping minified output for `track.kind` would be
    // vacuous -- it would pass for the wrong reasons. The guard now lives in the
    // frontend suite, next to the `pc.ontrack` handler it protects
    // (`src/lib/viewerSession.test.ts`); WEBUI.md keeps the checklist.

    // ---- admin control plane -------------------------------------------------

    /// In-memory `SessionControl`, so the admin API can be exercised without a
    /// capture backend (which would need a real display and a live session).
    #[derive(Default)]
    struct FakeControl {
        running: Mutex<bool>,
        pending: Mutex<Vec<ViewerIdentifier>>,
        viewing: Mutex<Vec<ViewerIdentifier>>,
    }

    impl FakeControl {
        fn with_pending(uuid: &str, name: &str) -> Self {
            let control = FakeControl::default();
            control.push_pending(uuid, name);
            control
        }

        fn push_pending(&self, uuid: &str, name: &str) {
            self.pending.lock().unwrap().push(ViewerIdentifier {
                uuid: uuid.to_string(),
                name: name.to_string(),
            });
        }
    }

    /// Remove `uuid` from `list`, or explain why it is not there.
    fn take_viewer(
        list: &Mutex<Vec<ViewerIdentifier>>,
        uuid: &str,
        reason: &str,
    ) -> Result<ViewerIdentifier, String> {
        let mut list = list.lock().unwrap();
        let index = list
            .iter()
            .position(|viewer| viewer.uuid == uuid)
            .ok_or_else(|| format!("{uuid} is {reason}"))?;
        Ok(list.remove(index))
    }

    #[async_trait::async_trait]
    impl SessionControl for FakeControl {
        fn is_running(&self) -> bool {
            *self.running.lock().unwrap()
        }

        fn room_id(&self) -> Option<String> {
            Some("desk".to_string())
        }

        fn room_password(&self) -> Option<String> {
            Some("pass".to_string())
        }

        fn invite_link(&self) -> Option<String> {
            Some("http://127.0.0.1:8765/?room=desk&pwd=pass".to_string())
        }

        fn auto_accept(&self) -> bool {
            false
        }

        async fn start_session(&self) {
            *self.running.lock().unwrap() = true;
        }

        async fn stop_session(&self) {
            *self.running.lock().unwrap() = false;
        }

        async fn pending_viewers(&self) -> Vec<ViewerIdentifier> {
            self.pending.lock().unwrap().clone()
        }

        async fn viewing_viewers(&self) -> Vec<ViewerIdentifier> {
            self.viewing.lock().unwrap().clone()
        }

        async fn accept_viewer(&self, uuid: String) -> Result<(), String> {
            let viewer = take_viewer(&self.pending, &uuid, "not waiting for a decision")?;
            self.viewing.lock().unwrap().push(viewer);
            Ok(())
        }

        async fn decline_viewer(&self, uuid: String) -> Result<(), String> {
            take_viewer(&self.pending, &uuid, "not waiting for a decision")?;
            Ok(())
        }

        async fn kick_viewer(&self, uuid: String) -> Result<(), String> {
            take_viewer(&self.viewing, &uuid, "not viewing")?;
            Ok(())
        }
    }

    /// Without `webui.admin_password` the admin surface does not exist at all --
    /// not "exists and denies", but absent.
    #[tokio::test]
    async fn admin_routes_are_absent_without_a_secret() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(serve(listener, None));

        let (status, _) = admin_request(port, "GET", "/admin", None, None).await;
        assert_eq!(
            status, 404,
            "the admin page must not be served without a secret"
        );

        let (status, _) = admin_request(port, "GET", "/api/admin/state", None, None).await;
        assert_eq!(
            status, 404,
            "the admin API must not be served without a secret"
        );

        let (status, body) = admin_request(port, "GET", "/", None, None).await;
        assert_eq!(status, 200, "the viewer page must keep working");
        assert!(body.contains("Mira Sharer"));

        server.abort();
    }

    /// The viewer passcode is handed to viewers, so it must not authenticate the
    /// operator -- and neither may a non-Bearer scheme.
    #[tokio::test]
    async fn admin_requires_the_bearer_token() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let admin = AdminState::new(Arc::new(FakeControl::default()), Some(SECRET.to_string()))
            .expect("a non-empty secret enables the admin plane");
        let server = tokio::spawn(serve(listener, Some(admin)));

        let (status, _) = admin_request(port, "GET", "/api/admin/state", None, None).await;
        assert_eq!(status, 401, "missing credentials must be rejected");

        let (status, _) = admin_request(port, "GET", "/api/admin/state", Some("wrong"), None).await;
        assert_eq!(status, 401, "a wrong secret must be rejected");

        let (status, _) = http_request(
            port,
            "GET",
            "/api/admin/state",
            &[("Authorization", "Basic czNjcmV0")],
            None,
        )
        .await;
        assert_eq!(status, 401, "only the Bearer scheme may authenticate");

        let (status, body) =
            admin_request(port, "GET", "/api/admin/state", Some(SECRET), None).await;
        assert_eq!(status, 200);
        let state: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(state["running"], false);
        assert_eq!(state["room"], "desk");
        assert_eq!(state["autoAccept"], false);
        assert!(state["pending"].as_array().unwrap().is_empty());

        // The page itself carries no secret, so it needs no token.
        let (status, body) = admin_request(port, "GET", "/admin", None, None).await;
        assert_eq!(status, 200);
        assert!(body.contains("Mira Sharer"));

        server.abort();
    }

    #[tokio::test]
    async fn admin_session_commands_reach_the_control() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let control = Arc::new(FakeControl::default());
        let admin =
            AdminState::new(control.clone(), Some(SECRET.to_string())).expect("admin enabled");
        let server = tokio::spawn(serve(listener, Some(admin)));

        let (status, _) =
            admin_request(port, "POST", "/api/admin/session/start", Some(SECRET), None).await;
        assert_eq!(status, 204);
        assert!(
            *control.running.lock().unwrap(),
            "start must reach the control"
        );

        let (_, body) = admin_request(port, "GET", "/api/admin/state", Some(SECRET), None).await;
        let state: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(state["running"], true);
        assert_eq!(
            state["inviteLink"],
            "http://127.0.0.1:8765/?room=desk&pwd=pass"
        );

        let (status, _) =
            admin_request(port, "POST", "/api/admin/session/stop", Some(SECRET), None).await;
        assert_eq!(status, 204);
        assert!(
            !*control.running.lock().unwrap(),
            "stop must reach the control"
        );

        server.abort();
    }

    /// An admin page can hold a viewer list that is minutes stale, so every verb
    /// has to answer 404 for a viewer that is gone -- never panic, never 500.
    #[tokio::test]
    async fn admin_viewer_commands_tolerate_stale_uuids() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let control = Arc::new(FakeControl::with_pending("viewer-1", "Phone"));
        control.push_pending("viewer-2", "Laptop");
        let admin =
            AdminState::new(control.clone(), Some(SECRET.to_string())).expect("admin enabled");
        let server = tokio::spawn(serve(listener, Some(admin)));

        // decline drops a pending viewer
        let (status, _) = admin_request(
            port,
            "POST",
            "/api/admin/viewers/viewer-2/decline",
            Some(SECRET),
            None,
        )
        .await;
        assert_eq!(status, 204);
        assert_eq!(control.pending.lock().unwrap().len(), 1);

        // accept moves one from pending to viewing
        let (status, _) = admin_request(
            port,
            "POST",
            "/api/admin/viewers/viewer-1/accept",
            Some(SECRET),
            None,
        )
        .await;
        assert_eq!(status, 204);

        let (_, body) = admin_request(port, "GET", "/api/admin/state", Some(SECRET), None).await;
        let state: Value = serde_json::from_str(&body).unwrap();
        assert!(state["pending"].as_array().unwrap().is_empty());
        assert_eq!(state["viewing"][0]["uuid"], "viewer-1");
        assert_eq!(state["viewing"][0]["name"], "Phone");

        // The page is now stale: the same click must be a clean 404.
        let (status, body) = admin_request(
            port,
            "POST",
            "/api/admin/viewers/viewer-1/accept",
            Some(SECRET),
            None,
        )
        .await;
        assert_eq!(status, 404);
        assert!(
            body.contains("not waiting"),
            "the error should name the reason, got: {body}"
        );

        for verb in ["accept", "decline", "kick"] {
            let path = format!("/api/admin/viewers/ghost/{verb}");
            let (status, _) = admin_request(port, "POST", &path, Some(SECRET), None).await;
            assert_eq!(status, 404, "{verb} on an unknown viewer must be 404");
        }

        // kick drops it from viewing, and a second kick is a clean 404
        let (status, _) = admin_request(
            port,
            "POST",
            "/api/admin/viewers/viewer-1/kick",
            Some(SECRET),
            None,
        )
        .await;
        assert_eq!(status, 204);
        assert!(control.viewing.lock().unwrap().is_empty());

        let (status, _) = admin_request(
            port,
            "POST",
            "/api/admin/viewers/viewer-1/kick",
            Some(SECRET),
            None,
        )
        .await;
        assert_eq!(status, 404);

        server.abort();
    }

    // ---- dependency-free HTTP client -----------------------------------------

    /// `http_request` with the admin `Authorization` header filled in.
    async fn admin_request(
        port: u16,
        method: &str,
        path: &str,
        token: Option<&str>,
        body: Option<&str>,
    ) -> (u16, String) {
        let bearer = token.map(|token| format!("Bearer {token}"));
        let headers: Vec<(&str, &str)> = match bearer.as_deref() {
            Some(value) => vec![("Authorization", value)],
            None => Vec::new(),
        };

        http_request(port, method, path, &headers, body).await
    }

    /// A hand-rolled HTTP/1.1 client, so the tests need no `reqwest`.
    async fn http_request(
        port: u16,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: Option<&str>,
    ) -> (u16, String) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let body = body.unwrap_or("");
        let mut request =
            format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n");
        for (name, value) in headers {
            request.push_str(name);
            request.push_str(": ");
            request.push_str(value);
            request.push_str("\r\n");
        }
        request.push_str(&format!("Content-Length: {}\r\n\r\n", body.len()));
        request.push_str(body);

        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        stream.write_all(request.as_bytes()).await.unwrap();

        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let raw = String::from_utf8_lossy(&raw).to_string();

        let status = raw
            .split_whitespace()
            .nth(1)
            .and_then(|code| code.parse().ok())
            .unwrap_or(0);
        let body = raw
            .split_once("\r\n\r\n")
            .map(|(_, body)| body.to_string())
            .unwrap_or_default();

        (status, body)
    }

    // plain TCP GET / to avoid adding a dev-dependency
    async fn reqwest_free_get(port: u16) -> String {
        http_request(port, "GET", "/", &[], None).await.1
    }
}
