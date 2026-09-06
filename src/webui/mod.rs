//! Embedded local WebUI + signalling server.
//!
//! Serves a local viewer page (`GET /`) and implements the mirashare
//! signalling protocol (`GET /signaller`, WebSocket) as a room router
//! between the sharer's `WebSocketSignaller` (client) and browser viewers.
//! Messages are relayed as raw JSON; only the `type`/`from`/`to`/`room`
//! envelope fields are parsed for routing.

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

const INDEX_HTML: &str = include_str!("../../webui/index.html");

const IDLE_TIMEOUT: Duration = Duration::from_secs(90);
const REAP_INTERVAL: Duration = Duration::from_secs(10);

static NEXT_CONN_ID: AtomicUsize = AtomicUsize::new(0);

/// Start the embedded webui + signalling server on `127.0.0.1:<port>`.
/// Runs until the process exits.
pub async fn start(port: u16) -> std::io::Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", port)).await?;
    info!("WebUI listening on http://127.0.0.1:{port}");
    serve(listener).await
}

async fn serve(listener: TcpListener) -> std::io::Result<()> {
    let state = Arc::new(SignallerState::default());
    {
        let state = state.clone();
        tokio::spawn(reap_idle(state));
    }

    let router = Router::new()
        .route("/", get(|| async { Html(INDEX_HTML) }))
        .route(
            "/signaller",
            get(move |ws: WebSocketUpgrade| {
                let state = state.clone();
                async move { ws.on_upgrade(move |socket| handle_ws(state, socket)) }
            }),
        );

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
            let room = Uuid::new_v4().to_string();
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
    use futures_util::{SinkExt, StreamExt};
    use serde_json::{json, Value};
    use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};

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
        let server = tokio::spawn(serve(listener));

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

        server.abort();
    }

    // plain TCP GET / to avoid adding a dev-dependency
    async fn reqwest_free_get(port: u16) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        stream
            .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await.unwrap();
        String::from_utf8_lossy(&buf).to_string()
    }
}
