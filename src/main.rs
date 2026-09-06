#![windows_subsystem = "windows"]

extern crate core;
#[macro_use]
extern crate log;

use std::path::Path;

use clap::Parser;
use directories::ProjectDirs;
use iced::{Application, Settings};

use crate::capture::capturer::Args;
use crate::capture::ScreenCapture;
use crate::gui::app::App;
use crate::output::OutputSink;
use crate::result::Result;

mod auth;
mod capture;
mod config;
mod encoder;
mod gui;
mod inputs;
mod output;
mod performance_profiler;
mod result;
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

    if config.webui.enabled {
        let port = config.webui.port;
        tokio::spawn(async move {
            if let Err(e) = webui::start(port).await {
                error!(
                    "Failed to start webui on port {port}: {e} (falling back to configured signaller)"
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
        flags: (args, config),
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
