use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::select;
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;
use xcap::Monitor;

use crate::capture::display::DisplaySelector;
use crate::capture::{DisplayInfo, ScreenCaptureImpl, YUVFrame};
use crate::config::Config;
use crate::encoder::{FfmpegEncoder, FrameData};
use crate::performance_profiler::PerformanceProfiler;
use crate::result::Result;
use crate::{OutputSink, ScreenCapture};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DisplayId(u32);

impl DisplayId {
    fn name(&self) -> Option<String> {
        Monitor::all()
            .ok()?
            .into_iter()
            .find(|m| m.id().ok() == Some(self.0))?
            .name()
            .ok()
    }
}

impl ToString for DisplayId {
    fn to_string(&self) -> String {
        self.name().unwrap_or_else(|| format!("{:?}", self.0))
    }
}

pub struct LinuxCapture {
    config: Config,
    display: Monitor,
}

impl LinuxCapture {
    fn find_display(id: &DisplayId) -> Result<Monitor> {
        Monitor::all()
            .map_err(|e| anyhow::anyhow!("Failed to list displays: {}", e))?
            .into_iter()
            .find(|m| m.id().ok() == Some(id.0))
            .ok_or_else(|| anyhow::anyhow!("Display not found"))
    }
}

impl DisplayInfo for LinuxCapture {
    fn resolution(&self) -> (u32, u32) {
        (
            self.display.width().unwrap_or(0),
            self.display.height().unwrap_or(0),
        )
    }

    fn dpi_conversion_factor(&self) -> f64 {
        self.display.scale_factor().unwrap_or(1.0) as f64
    }
}

#[async_trait]
impl ScreenCapture for LinuxCapture {
    fn new(config: Config) -> Result<ScreenCaptureImpl> {
        // Pick the first available monitor; the display can be changed later
        // through DisplaySelector.
        let displays = Monitor::all().map_err(|e| anyhow::anyhow!("No displays found: {}", e))?;
        let display = displays
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("No displays found"))?;

        Ok(Self { config, display })
    }

    fn display(&self) -> &dyn DisplayInfo {
        self
    }

    async fn start_capture(
        &mut self,
        mut encoder: FfmpegEncoder,
        output: Arc<Mutex<impl OutputSink + ?Sized>>,
        mut profiler: PerformanceProfiler,
        shutdown_token: CancellationToken,
    ) -> Result<()> {
        let (video_tx, mut video_rx) = mpsc::channel::<YUVFrame>(1);

        let display = self.display.clone();
        let cancel_capture = shutdown_token.clone();
        let max_fps = self.config.max_fps;
        let frame_interval = Duration::from_millis(1000 / max_fps.max(1) as u64);

        // xcap captures synchronously, so run the capture loop on a dedicated
        // thread and hand frames over the channel.
        std::thread::spawn(move || {
            let start = Instant::now();
            let mut next_frame = Instant::now();
            while !cancel_capture.is_cancelled() {
                match display.capture_image() {
                    Ok(image) => {
                        let (width, height) = (image.width() as i32, image.height() as i32);
                        let rgba = image.as_raw();
                        let mut frame = rgba_to_nv12(rgba, width, height);
                        frame.display_time = start.elapsed().as_millis() as u64;
                        if video_tx.blocking_send(frame).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to capture frame: {}", e);
                    }
                }
                next_frame += frame_interval;
                let sleep = next_frame.saturating_duration_since(Instant::now());
                if sleep > Duration::ZERO {
                    std::thread::sleep(sleep);
                } else {
                    next_frame = Instant::now();
                }
            }
        });

        let cancel_video = shutdown_token.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(frame_interval);
            loop {
                select! {
                    Some(frame) = video_rx.recv() => {
                        let frame_time = frame.display_time as i64;
                        profiler.accept_frame(frame_time);
                        profiler.done_preprocessing();
                        let encoded = encoder
                            .encode(FrameData::NV12(&frame), frame_time)
                            .unwrap();
                        let encoded_len = encoded.len();
                        profiler.done_encoding();
                        output.lock().await.write(encoded).await.unwrap();
                        profiler.done_processing(encoded_len);
                        ticker.tick().await;
                    }
                    _ = cancel_video.cancelled() => {
                        break;
                    }
                }
            }
        });

        Ok(())
    }

    async fn stop_capture(&mut self) -> Result<()> {
        // The capture thread exits when the shutdown token is cancelled.
        Ok(())
    }
}

impl DisplaySelector for LinuxCapture {
    type Display = DisplayId;

    fn available_displays(&mut self) -> Result<Vec<Self::Display>> {
        let monitors =
            Monitor::all().map_err(|e| anyhow::anyhow!("Failed to list displays: {}", e))?;
        Ok(monitors
            .into_iter()
            .filter_map(|m| m.id().ok().map(DisplayId))
            .collect())
    }

    fn select_display(&mut self, display: &Self::Display) -> Result<()> {
        self.display = Self::find_display(display)?;
        Ok(())
    }

    fn selected_display(&self) -> Result<Option<Self::Display>> {
        self.display
            .id()
            .map(DisplayId)
            .map(Some)
            .map_err(|e| anyhow::anyhow!("Failed to get display id: {}", e))
    }
}

/// Convert an RGBA buffer to an NV12 `YUVFrame` (BT.601).
fn rgba_to_nv12(rgba: &[u8], width: i32, height: i32) -> YUVFrame {
    let w = width as usize;
    let h = height as usize;
    let mut y_plane = vec![0u8; w * h];
    let mut uv_plane = vec![0u8; w * h / 2];

    for row in 0..h {
        for col in 0..w {
            let i = row * w + col;
            let r = rgba[i * 4] as i32;
            let g = rgba[i * 4 + 1] as i32;
            let b = rgba[i * 4 + 2] as i32;

            y_plane[i] = ((66 * r + 129 * g + 25 * b + 128) >> 8).clamp(0, 255) as u8;

            if row % 2 == 0 && col % 2 == 0 {
                let uv_i = (row / 2) * (w / 2) + (col / 2);
                uv_plane[2 * uv_i] = ((-38 * r - 74 * g + 112 * b + 128) >> 8).clamp(0, 255) as u8; // U
                uv_plane[2 * uv_i + 1] =
                    ((112 * r - 94 * g - 18 * b + 128) >> 8).clamp(0, 255) as u8;
                // V
            }
        }
    }

    YUVFrame {
        display_time: 0,
        luminance_bytes: y_plane,
        luminance_stride: width,
        chrominance_bytes: uv_plane,
        chrominance_stride: width / 2,
    }
}
