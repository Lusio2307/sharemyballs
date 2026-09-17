#![windows_subsystem = "windows"]

extern crate core;
#[macro_use]
extern crate log;

use std::path::Path;
use std::sync::{Arc, Mutex};

use clap::Parser;
use directories::ProjectDirs;
use iced::{Application, Settings};

use crate::capture::capturer::{Args, Capturer};
use crate::capture::ScreenCapture;
use crate::gui::app::App;
use crate::output::OutputSink;
use crate::result::Result;
use crate::session::AppControl;
use crate::webui::AdminState;

mod auth;
mod capture;
mod config;
mod encoder;
mod gui;
mod inputs;
mod output;
mod performance_profiler;
mod result;
mod session;
mod signaller;
mod webui;

#[tokio::main]
async fn main() {
    fern::Dispatch::new()
        .format(|out, message, record| {
            out.finish(format_args!(
                "[{} {} {}] {}",
                humantime::format_rfc3339(std::time::SystemTime::now()),
                record.level(),
                record.target(),
                message
            ))
        })
        .level(log::LevelFilter::Info)
        .level_for("wgpu_core", log::LevelFilter::Warn)
        .level_for("wgpu_hal", log::LevelFilter::Off)
        .level_for("iced_wgpu", log::LevelFilter::Warn)
        .chain(std::io::stdout())
        .apply()
        .unwrap_or_else(|_| {
            eprintln!("Failed to initialize logger");
        });
    let args = Args::parse();
    let config = config::load(config_path(&args).as_path()).unwrap();

    // One capturer, two operator surfaces: the iced GUI drives it directly, and
    // the embedded server reaches it through `AppControl` for `/admin`.
    //
    // `notify_update` still pokes the GUI's update subscription. The sender is
    // leaked so that it lives as long as the process, matching the previous
    // version that built it inside `App::new`; a full channel means a refresh is
    // already queued, so a dropped poke is correct and must not panic.
    let (update_sender, update_receiver) = tokio::sync::mpsc::channel::<()>(10);
    let update_sender = Box::leak(Box::new(update_sender));
    let capturer = Arc::new(Mutex::new(Capturer::new(
        args,
        config.clone(),
        Arc::new(move || {
            let _ = update_sender.try_send(());
        }),
    )));
    let viewer_manager = capturer.lock().unwrap().get_viewer_manager();

    if config.webui.enabled {
        let port = config.webui.port;
        let bind = config.webui.bind.clone();
        // `AdminState::new` yields None when no secret is configured, which is
        // what leaves `/admin` and `/api/admin/*` unregistered (fail closed).
        let admin = AdminState::new(
            Arc::new(AppControl::new(capturer.clone(), viewer_manager)),
            config.webui.admin_password.clone(),
        );
        tokio::spawn(async move {
            if let Err(e) = webui::start(&bind, port, admin).await {
                error!(
                    "Failed to start webui on {bind}:{port}: {e} (falling back to configured signaller)"
                );
            }
        });
    }

    App::run(Settings {
        id: None,
        window: iced::window::Settings {
            size: (640, 373),
            min_size: Some((400, 300)),
            icon: Some(
                iced::window::icon::from_file_data(
                    include_bytes!("../resources/icons/256x256.png"),
                    None,
                )
                .unwrap(),
            ),
            ..Default::default()
        },
        flags: (capturer, update_receiver),
        default_font: Default::default(),
        default_text_size: 20.0,
        text_multithreading: false,
        antialiasing: false,
        exit_on_close_request: true,
        try_opengles_first: false,
    })
    .unwrap();
}

fn config_path(args: &Args) -> std::path::PathBuf {
    if let Some(config_path) = &args.config {
        Path::new(config_path).to_path_buf()
    } else {
        if cfg!(target_os = "windows") {
            Path::new("config.toml").to_path_buf()
        } else if cfg!(target_os = "macos") {
            let config_dir = ProjectDirs::from("", "", "Mira Sharer")
                .unwrap()
                .config_dir()
                .to_path_buf();
            if !config_dir.exists() {
                std::fs::create_dir_all(&config_dir).unwrap();
            }
            config_dir.join("config.toml")
        } else {
            panic!("Unsupported OS")
        }
    }
}
