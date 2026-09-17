use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::Arc;

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use twilio::TwilioAuthentication;
use webrtc::ice_transport::ice_credential_type::RTCIceCredentialType;
use webrtc::ice_transport::ice_server::RTCIceServer;

use crate::signaller::{Signaller, SignallerIceServer};
use crate::Result;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    #[serde(default = "default_signaller")]
    pub signaller_url: String,

    #[serde(default = "default_viewer")]
    pub viewer_url: String,

    #[serde(default = "default_max_fps")]
    pub max_fps: u32,

    #[serde(default = "default_ice_servers")]
    pub ice_servers: Vec<IceServer>,

    #[serde(default = "libx264")]
    pub encoder: EncoderConfig,

    #[serde(default = "default_webui")]
    pub webui: WebuiConfig,

    /// Fixed session password. When unset (or empty) a random one is generated
    /// per session and shown in the GUI's Invite tab.
    #[serde(default)]
    pub password: Option<String>,

    /// Fixed room id. When unset, a random one is requested from the signaller
    /// each session. A fixed room keeps the invite URL stable across restarts,
    /// which is what makes a bookmarked viewer page useful.
    #[serde(default)]
    pub room: Option<String>,

    /// Admit any viewer that passes the password check, without waiting for a
    /// click in the GUI. Required for unattended operation. This does *not*
    /// bypass authentication -- the password check still runs.
    #[serde(default)]
    pub auto_accept: bool,

    /// Start sharing as soon as the app launches, instead of waiting for the
    /// "Start Sharing" button.
    #[serde(default)]
    pub auto_start: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WebuiConfig {
    #[serde(default = "default_webui_enabled")]
    pub enabled: bool,
    #[serde(default = "default_webui_port")]
    pub port: u16,
    /// Address the embedded server binds to. Defaults to loopback; use
    /// `0.0.0.0` to expose the viewer page to the rest of the LAN.
    #[serde(default = "default_webui_bind")]
    pub bind: String,
    /// Base URL used to build the invite link, e.g.
    /// `https://stream.example.com/`. Defaults to `http://<bind>:<port>/`.
    #[serde(default)]
    pub public_url: Option<String>,
    /// Secret for the `/admin` control plane. When unset (or empty) the admin
    /// page and its API are not served at all.
    ///
    /// Failing closed is deliberate: the viewer passcode is handed to every
    /// viewer, so it must never double as the admin secret.
    #[serde(default)]
    pub admin_password: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EncoderConfig {
    pub encoder: String,
    pub pixel_format: String,
    pub encoding: String,
    pub options: HashMap<String, String>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum IceCredentialType {
    Unspecified,
    #[default]
    Password,
    Oauth,
    Twilio,    // TODO refactor
    Signaller, // TODO refactor
}

#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct IceServer {
    pub urls: Vec<String>,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub credential: String,
    #[serde(default)]
    pub credential_type: IceCredentialType,
}

impl From<SignallerIceServer> for IceServer {
    fn from(val: SignallerIceServer) -> Self {
        IceServer {
            urls: vec![val.url],
            username: val.username,
            credential: val.password,
            credential_type: IceCredentialType::Password,
        }
    }
}

impl Config {
    async fn fetch_ice_servers_from_signaller(
        &self,
        signaller: Arc<dyn Signaller + Send + Sync>,
    ) -> Vec<IceServer> {
        signaller
            .fetch_ice_servers()
            .await
            .iter()
            .map(|s| s.clone().into())
            .collect()
    }

    pub async fn fetch_ice_servers(&self, signaller: Arc<dyn Signaller + Send + Sync>) -> Self {
        Self {
            ice_servers: futures_util::future::join_all(self.ice_servers.clone().into_iter().map(
                |s| async {
                    match s.credential_type {
                        IceCredentialType::Twilio => get_twilio_ice_servers(s).await,
                        IceCredentialType::Signaller => {
                            self.fetch_ice_servers_from_signaller(signaller.clone())
                                .await
                        }
                        _ => vec![s],
                    }
                },
            ))
            .await
            .iter()
            .flatten()
            .cloned()
            .collect(),
            ..self.clone()
        }
    }
}

impl From<IceCredentialType> for RTCIceCredentialType {
    fn from(t: IceCredentialType) -> Self {
        match t {
            IceCredentialType::Unspecified => RTCIceCredentialType::Unspecified,
            IceCredentialType::Password => RTCIceCredentialType::Password,
            IceCredentialType::Oauth => RTCIceCredentialType::Oauth,
            IceCredentialType::Twilio => RTCIceCredentialType::Password,
            IceCredentialType::Signaller => RTCIceCredentialType::Password,
        }
    }
}

impl From<IceServer> for RTCIceServer {
    fn from(server: IceServer) -> RTCIceServer {
        RTCIceServer {
            urls: server.urls,
            username: server.username,
            credential: server.credential,
            credential_type: server.credential_type.into(),
        }
    }
}

pub fn load(path: &Path) -> Result<Config> {
    // create a new file if it does not exist
    if !path.exists() {
        let mut file = File::create(path)?;
        let config = toml::from_str::<Config>("")?;
        file.write_all("# for more sample configs, see https://github.com/mira-screen-share/sharer/tree/main/configs\n".as_bytes())?;
        file.write_all(toml::to_string(&config)?.as_ref())?;
        return Ok(config);
    }

    let mut file = File::open(path)?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;
    Ok(toml::from_str(&contents)?)
}

fn libx264() -> EncoderConfig {
    EncoderConfig {
        encoder: "libx264".to_string(),
        pixel_format: "nv12".to_string(),
        encoding: "video/H264".to_string(),
        options: HashMap::from([
            ("profile".into(), "baseline".into()),
            ("preset".into(), "ultrafast".into()),
            ("tune".into(), "zerolatency".into()),
        ]),
    }
}

fn default_signaller() -> String {
    "wss://ws.mirashare.app".to_string()
}

fn default_viewer() -> String {
    "https://mirashare.app/".to_string()
}

fn default_max_fps() -> u32 {
    60
}

fn default_webui() -> WebuiConfig {
    WebuiConfig {
        enabled: default_webui_enabled(),
        port: default_webui_port(),
        bind: default_webui_bind(),
        public_url: None,
        admin_password: None,
    }
}

fn default_webui_enabled() -> bool {
    true
}

fn default_webui_port() -> u16 {
    8765
}

fn default_webui_bind() -> String {
    "127.0.0.1".to_string()
}

/// No ICE servers by default.
///
/// The previous defaults pointed at `stun.l.google.com` plus a
/// `Signaller`-credential entry, which resolved to nothing without Twilio.
/// For a self-hosted deployment the only ICE server that should ever be
/// contacted is the operator's own coturn instance, so that is left to
/// `config.toml` rather than baked in here. With no ICE servers at all, WebRTC
/// still uses host candidates, which is enough for viewers on the same LAN.
fn default_ice_servers() -> Vec<IceServer> {
    Vec::new()
}

async fn get_twilio_ice_servers(s: IceServer) -> Vec<IceServer> {
    if s.credential_type != IceCredentialType::Twilio {
        return vec![];
    }
    let base64_engine = base64::engine::GeneralPurpose::new(
        &base64::alphabet::STANDARD,
        base64::engine::general_purpose::PAD,
    );
    let client = twilio::TwilioClient::new(
        "https://api.twilio.com",
        TwilioAuthentication::BasicAuth {
            basic_auth: base64_engine.encode(format!("{}:{}", s.username, s.credential).as_bytes()),
        },
    );
    let response = client.create_token(s.username.as_str()).send().await;
    match response {
        Ok(token) => token
            .ice_servers
            .unwrap_or_default()
            .iter()
            .map(|s| match s {
                Value::Object(s) => {
                    let url = s.get("url").unwrap().as_str().unwrap().to_owned();
                    IceServer {
                        urls: vec![url],
                        username: token.username.clone().unwrap(),
                        credential: token.password.clone().unwrap(),
                        credential_type: IceCredentialType::Password,
                    }
                }
                _ => panic!("Expected object"),
            })
            .collect(),
        Err(e) => {
            error!("Failed to get Twilio ICE servers: {:?}", e);
            vec![]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Defaults must not reach out to any third-party service, and unattended
    /// behaviour must be opt-in.
    #[test]
    fn defaults_are_self_hosted() {
        let config: Config = toml::from_str("").unwrap();

        assert_eq!(config.password, None);
        assert_eq!(config.room, None);
        assert!(!config.auto_accept, "auto_accept must be opt-in");
        assert!(!config.auto_start, "auto_start must be opt-in");

        assert!(config.webui.enabled);
        assert_eq!(config.webui.port, 8765);
        assert_eq!(config.webui.bind, "127.0.0.1");
        assert_eq!(config.webui.public_url, None);
        assert_eq!(
            config.webui.admin_password, None,
            "the admin control plane must fail closed unless a secret is configured"
        );

        assert!(
            config.ice_servers.is_empty(),
            "no STUN/TURN server may be configured by default"
        );
    }

    #[test]
    fn session_overrides_parse() {
        let config: Config = toml::from_str(
            r#"
room = "desk"
password = "hunter2"
auto_accept = true
auto_start = true

[webui]
bind = "0.0.0.0"
public_url = "https://stream.example.com/"
admin_password = "s3cret-admin"
"#,
        )
        .unwrap();

        assert_eq!(config.room.as_deref(), Some("desk"));
        assert_eq!(config.password.as_deref(), Some("hunter2"));
        assert!(config.auto_accept);
        assert!(config.auto_start);
        assert_eq!(config.webui.bind, "0.0.0.0");
        assert_eq!(
            config.webui.public_url.as_deref(),
            Some("https://stream.example.com/")
        );
        assert_eq!(config.webui.admin_password.as_deref(), Some("s3cret-admin"));
    }

    /// The bundled presets are copied verbatim by users. `toml` rejects
    /// duplicate keys outright, so this fails here rather than at startup --
    /// both config.nvenc.toml and config.vp9.toml shipped with one.
    #[test]
    fn bundled_presets_parse() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/configs");
        let mut checked = 0;

        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            toml::from_str::<Config>(&text)
                .unwrap_or_else(|e| panic!("{} is not a valid config: {e}", path.display()));
            checked += 1;
        }

        assert!(checked >= 4, "expected the bundled presets to be present");
    }

    /// The documented example is what users copy first, so keep it loadable.
    #[test]
    fn example_config_parses() {
        let text =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/config.toml.example"))
                .unwrap();
        toml::from_str::<Config>(&text).unwrap();
    }
}
