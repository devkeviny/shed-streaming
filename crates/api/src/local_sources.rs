// Licensed under the GNU Affero General Public License v3.0 — see LICENSE.

//! OBS-style local sources: devices and generators that run on the Muxshed
//! host itself, one ffmpeg per source, feeding the source's media relay.
//!
//! - VideoCapture: a V4L2 camera (USB webcam) — OBS "Video Capture Device".
//! - AudioInput: an ALSA input (microphone, line-in) — OBS "Audio Input
//!   Capture". Audio-only: it cannot be a scene layer, but it can be
//!   selected as the program audio source.
//! - Text: text rendered on a solid background via drawtext — OBS
//!   "Text (GDI+)". H.264 has no alpha channel, so a transparent text layer
//!   is not possible in this pipeline; the background color is the backdrop.
//! - Color: a solid color video signal — OBS "Color Source".
//! - HttpStream: a remote stream pulled over the network (HLS .m3u8, RTSP,
//!   plain HTTP video) — OBS "Media Source" with a URL. Normalized to the
//!   output canvas like an RTMP source.
//!
//! Images need no separate kind: a MediaFile source already loops still
//! images (png/jpg/gif/webp/...). Window/display capture has no meaning on
//! a headless box and is intentionally not implemented.

use bytes::Bytes;
use muxshed_common::SourceKind;
use serde::Serialize;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::routes::output::OutputConfig;
use crate::state::AppState;

/// A capture device the UI can offer in a dropdown.
#[derive(Debug, Clone, Serialize)]
pub struct CaptureDevice {
    /// ffmpeg device string, e.g. "/dev/video2" or "hw:1".
    pub id: String,
    /// Human-readable name, e.g. "UVC Camera (046d:0825)".
    pub name: String,
}

/// List V4L2 video nodes from sysfs (/sys/class/video4linux/*).
/// Every node is listed with its driver name — the hardware codec nodes
/// (e.g. "meson-venc-h264") show up too, and the name makes them obvious.
pub fn list_v4l2_devices() -> Vec<CaptureDevice> {
    let mut devices = Vec::new();
    let Ok(entries) = std::fs::read_dir("/sys/class/video4linux") else {
        return devices;
    };
    for entry in entries.flatten() {
        let node = entry.file_name().to_string_lossy().into_owned();
        if !node.starts_with("video") {
            continue;
        }
        let name = std::fs::read_to_string(entry.path().join("name"))
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| node.clone());
        devices.push(CaptureDevice {
            id: format!("/dev/{}", node),
            name,
        });
    }
    devices.sort_by(|a, b| a.id.cmp(&b.id));
    devices
}

/// List ALSA capture devices from /proc/asound/cards.
/// The first entry is always the ALSA default device.
pub fn list_alsa_devices() -> Vec<CaptureDevice> {
    let mut devices = vec![CaptureDevice {
        id: "default".to_string(),
        name: "Default ALSA input".to_string(),
    }];
    let Ok(cards) = std::fs::read_to_string("/proc/asound/cards") else {
        return devices;
    };
    for line in cards.lines() {
        // Format: " 0 [PCH  ]: HDA-Intel - HDA Intel PCH"
        let trimmed = line.trim_start();
        let Some((idx, rest)) = trimmed.split_once(' ') else {
            continue;
        };
        if idx.parse::<u32>().is_err() {
            continue;
        }
        let name = rest
            .split("]: ")
            .nth(1)
            .unwrap_or(rest)
            .trim()
            .to_string();
        devices.push(CaptureDevice {
            id: format!("hw:{}", idx),
            name: if name.is_empty() {
                format!("Card {}", idx)
            } else {
                name
            },
        });
    }
    devices
}

/// Start a local source's ffmpeg. Dispatches on the SourceKind; returns an
/// error for kinds this module does not own.
pub async fn start_local_source(
    state: Arc<AppState>,
    source_id: Uuid,
    kind: &SourceKind,
) -> Result<(), String> {
    let cfg = load_output_config(&state).await;
    let relay_tx = state.get_or_create_media_relay(source_id).await;

    let args = build_args(kind, &cfg)?;
    let label = kind_label(kind);

    tracing::info!(
        "starting local source {} ({}) for {}",
        source_id,
        label,
        args.join(" ")
    );

    let mut child = Command::new("ffmpeg")
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("failed to start ffmpeg for {}: {}", label, e))?;

    let stdout = child
        .stdout
        .take()
        .ok_or("failed to capture ffmpeg stdout")?;

    if let Some(stderr) = child.stderr.take() {
        let sid = source_id;
        let lbl = label.to_string();
        tokio::spawn(async move {
            let mut reader = tokio::io::BufReader::new(stderr);
            let mut line = String::new();
            loop {
                line.clear();
                match tokio::io::AsyncBufReadExt::read_line(&mut reader, &mut line).await {
                    Ok(0) => break,
                    Ok(_) => tracing::debug!("ffmpeg {} [{}]: {}", lbl, sid, line.trim()),
                    Err(_) => break,
                }
            }
        });
    }

    {
        let mut players = state.local_sources.write().await;
        if let Some(mut old) = players.remove(&source_id) {
            let _ = old.kill().await;
        }
        players.insert(source_id, child);
    }
    state.source_states.write().await.insert(
        source_id,
        muxshed_common::SourceState::Live,
    );

    let state_clone = state.clone();
    tokio::spawn(async move {
        feed_relay(source_id, stdout, relay_tx, state_clone).await;
    });

    Ok(())
}

/// Stop a local source's ffmpeg and tear down its relay.
pub async fn stop_local_source(state: &AppState, source_id: &Uuid) {
    let mut players = state.local_sources.write().await;
    if let Some(mut child) = players.remove(source_id) {
        let _ = child.kill().await;
        tracing::info!("stopped local source {}", source_id);
    }
    drop(players);
    state.remove_media_relay(source_id).await;
    state.sequence_headers.write().await.remove(source_id);
    state
        .source_states
        .write()
        .await
        .insert(*source_id, muxshed_common::SourceState::Disconnected);
}

fn kind_label(kind: &SourceKind) -> &'static str {
    match kind {
        SourceKind::VideoCapture { .. } => "v4l2",
        SourceKind::AudioInput { .. } => "alsa",
        SourceKind::Text { .. } => "text",
        SourceKind::Color { .. } => "color",
        SourceKind::HttpStream { .. } => "http_stream",
        _ => "local",
    }
}

fn build_args(kind: &SourceKind, cfg: &OutputConfig) -> Result<Vec<String>, String> {
    match kind {
        SourceKind::VideoCapture {
            device,
            width,
            height,
            fps,
        } => {
            // MJPEG: the only way most USB webcams reach HD at full fps over
            // USB 2.0 (raw YUYV at 1080p30 is ~1.2 Gbit/s). Nearly all UVC
            // cameras support it.
            let mut args: Vec<String> = vec![
                "-hide_banner".into(),
                "-loglevel".into(),
                "warning".into(),
                "-f".into(),
                "v4l2".into(),
                "-input_format".into(),
                "mjpeg".into(),
                "-video_size".into(),
                format!("{}x{}", width, height),
                "-framerate".into(),
                format!("{}", fps),
                "-i".into(),
                device.clone(),
            ];
            crate::encoder::push_encoder_args(&mut args, cfg.video_encoder, "veryfast");
            args.extend([
                "-g".into(),
                format!("{}", fps * 2),
                "-r".into(),
                format!("{}", fps),
                "-b:v".into(),
                "2500k".into(),
                "-an".into(),
                "-f".into(),
                "flv".into(),
                "pipe:1".into(),
            ]);
            Ok(args)
        }
        SourceKind::AudioInput { device } => {
            // Audio-only FLV: no video track at all. The compositor's layer
            // filter requires video, so this can never be a layer — it is
            // selected as the program audio source instead.
            Ok(vec![
                "-hide_banner".into(),
                "-loglevel".into(),
                "warning".into(),
                "-f".into(),
                "alsa".into(),
                "-i".into(),
                device.clone(),
                "-vn".into(),
                "-c:a".into(),
                "aac".into(),
                "-b:a".into(),
                "128k".into(),
                "-ar".into(),
                "48000".into(),
                "-ac".into(),
                "2".into(),
                "-f".into(),
                "flv".into(),
                "pipe:1".into(),
            ])
        }
        SourceKind::Text {
            text,
            font_size,
            font_color,
            bg_color,
            width,
            height,
        } => {
            let font = find_font().ok_or(
                "no TTF font found for text sources (install fonts-dejavu-core)",
            )?;
            let vf = format!(
                "drawtext=fontfile='{}':text='{}':fontsize={}:fontcolor={}:x=(w-text_w)/2:y=(h-text_h)/2",
                font.to_string_lossy(),
                escape_drawtext(text),
                font_size,
                normalize_color(font_color),
            );
            let mut args: Vec<String> = vec![
                "-hide_banner".into(),
                "-loglevel".into(),
                "warning".into(),
                // -re: lavfi generates frames as fast as the pipeline takes
                // them; without realtime pacing ffmpeg encodes at full speed
                // and burns a whole core on a static image.
                "-re".into(),
                "-f".into(),
                "lavfi".into(),
                "-i".into(),
                format!(
                    "color=c={}:s={}x{}:r=30",
                    normalize_color(bg_color),
                    width,
                    height
                ),
                "-vf".into(),
                vf,
            ];
            crate::encoder::push_encoder_args(&mut args, cfg.video_encoder, "veryfast");
            args.extend([
                "-g".into(),
                "60".into(),
                "-r".into(),
                "30".into(),
                "-b:v".into(),
                "2000k".into(),
                "-an".into(),
                "-f".into(),
                "flv".into(),
                "pipe:1".into(),
            ]);
            Ok(args)
        }
        SourceKind::HttpStream { url } => {
            // Pull a remote stream: HLS (.m3u8), RTSP, or any HTTP video
            // ffmpeg can read. NO -re: a live stream is already
            // realtime-paced, and -re would throttle a faster-than-realtime
            // file/VOD URL to 1x for no reason. Normalized to the output
            // canvas exactly like the RTMP normalizer (scale + pad + fps +
            // hardware encode). -rw_timeout keeps a dead URL from hanging
            // the ffmpeg forever (10 s, in microseconds).
            let vf = format!(
                "scale={}:{}:force_original_aspect_ratio=decrease,pad={}:{}:(ow-iw)/2:(oh-ih)/2:black",
                cfg.width, cfg.height, cfg.width, cfg.height
            );
            let mut args: Vec<String> = vec![
                "-hide_banner".into(),
                "-loglevel".into(),
                "warning".into(),
                "-rw_timeout".into(),
                "10000000".into(),
                "-i".into(),
                url.clone(),
                "-vf".into(),
                vf,
            ];
            crate::encoder::push_encoder_args(&mut args, cfg.video_encoder, "veryfast");
            args.extend([
                "-g".into(),
                format!("{}", cfg.fps * 2),
                "-r".into(),
                format!("{}", cfg.fps),
                "-b:v".into(),
                format!("{}k", cfg.video_bitrate_kbps),
                "-maxrate".into(),
                format!("{}k", cfg.video_bitrate_kbps),
                "-bufsize".into(),
                format!("{}k", cfg.video_bitrate_kbps * 2),
                "-c:a".into(),
                "aac".into(),
                "-b:a".into(),
                format!("{}k", cfg.audio_bitrate_kbps),
                "-ar".into(),
                "48000".into(),
                "-f".into(),
                "flv".into(),
                "-flvflags".into(),
                "no_duration_filesize".into(),
                "pipe:1".into(),
            ]);
            Ok(args)
        }
        SourceKind::Color {
            color,
            width,
            height,
        } => {
            let mut args: Vec<String> = vec![
                "-hide_banner".into(),
                "-loglevel".into(),
                "warning".into(),
                // -re: lavfi generates frames as fast as the pipeline takes
                // them; without realtime pacing ffmpeg encodes at full speed
                // and burns a whole core on a static image.
                "-re".into(),
                "-f".into(),
                "lavfi".into(),
                "-i".into(),
                format!(
                    "color=c={}:s={}x{}:r=30",
                    normalize_color(color),
                    width,
                    height
                ),
            ];
            crate::encoder::push_encoder_args(&mut args, cfg.video_encoder, "veryfast");
            args.extend([
                "-g".into(),
                "60".into(),
                "-r".into(),
                "30".into(),
                "-b:v".into(),
                "2000k".into(),
                "-an".into(),
                "-f".into(),
                "flv".into(),
                "pipe:1".into(),
            ]);
            Ok(args)
        }
        _ => Err("not a local source kind".to_string()),
    }
}

/// Drawtext splits options on `:` and quotes with `'`, so all three of
/// `\`, `:` and `'` must be backslash-escaped in the text.
fn escape_drawtext(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' | ':' | '\'' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

/// ffmpeg colors accept `0xRRGGBB` and names; `#RRGGBB` is normalized to
/// the `0x` form so the HTML color picker value works directly.
fn normalize_color(color: &str) -> String {
    let c = color.trim();
    if let Some(hex) = c.strip_prefix('#') {
        format!("0x{}", hex)
    } else {
        c.to_string()
    }
}

/// DejaVu Sans ships with Debian's fonts-dejavu-core; fall back to any
/// sans TTF in the usual font directories.
fn find_font() -> Option<PathBuf> {
    const CANDIDATES: &[&str] = &[
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
        "/usr/share/fonts/truetype/freefont/FreeSans.ttf",
    ];
    for c in CANDIDATES {
        let p = PathBuf::from(c);
        if p.exists() {
            return Some(p);
        }
    }
    // Last resort: any .ttf under /usr/share/fonts.
    let mut stack = vec![PathBuf::from("/usr/share/fonts")];
    while let Some(dir) = stack.pop() {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "ttf") {
                    return Some(path);
                }
            }
        }
    }
    None
}

async fn feed_relay(
    source_id: Uuid,
    mut stdout: tokio::process::ChildStdout,
    relay_tx: broadcast::Sender<Bytes>,
    state: Arc<AppState>,
) {
    let mut header = [0u8; 13];
    if stdout.read_exact(&mut header).await.is_err() {
        tracing::warn!("failed to read FLV header for local source {}", source_id);
        cleanup_local_source(&state, source_id).await;
        return;
    }

    let mut frame_count: u64 = 0;

    loop {
        let mut tag_header = [0u8; 11];
        match stdout.read_exact(&mut tag_header).await {
            Ok(_) => {}
            Err(_) => {
                tracing::info!("local source ended for {}", source_id);
                break;
            }
        }

        let tag_type = tag_header[0];
        let data_size = ((tag_header[1] as u32) << 16)
            | ((tag_header[2] as u32) << 8)
            | (tag_header[3] as u32);

        let mut data = vec![0u8; data_size as usize];
        if stdout.read_exact(&mut data).await.is_err() {
            break;
        }

        let mut prev_size = [0u8; 4];
        if stdout.read_exact(&mut prev_size).await.is_err() {
            break;
        }

        let total_size = 11 + data_size as usize + 4;
        let mut tag_buf = Vec::with_capacity(total_size);
        tag_buf.extend_from_slice(&tag_header);
        tag_buf.extend_from_slice(&data);
        tag_buf.extend_from_slice(&prev_size);
        let tag = Bytes::from(tag_buf);

        if tag_type == 9 && !data.is_empty() {
            let frame_type = (data[0] >> 4) & 0x0F;
            let avc_packet_type = if data.len() > 1 { data[1] } else { 255 };

            if avc_packet_type == 0 {
                let mut headers = state.sequence_headers.write().await;
                let entry = headers
                    .entry(source_id)
                    .or_insert_with(crate::state::SequenceHeaders::default);
                entry.video = Some(tag.clone());
            } else if frame_type == 1 {
                let mut headers = state.sequence_headers.write().await;
                if let Some(entry) = headers.get_mut(&source_id) {
                    entry.last_keyframe = Some(tag.clone());
                }
            }
        } else if tag_type == 8 && !data.is_empty() {
            let aac_packet_type = if data.len() > 1 { data[1] } else { 255 };
            if aac_packet_type == 0 {
                let mut headers = state.sequence_headers.write().await;
                let entry = headers
                    .entry(source_id)
                    .or_insert_with(crate::state::SequenceHeaders::default);
                entry.audio = Some(tag.clone());
            }
        }

        let _ = relay_tx.send(tag);

        frame_count += 1;
        if frame_count == 1 {
            tracing::info!("local source streaming for {}", source_id);
            state.source_states.write().await.insert(
                source_id,
                muxshed_common::SourceState::Live,
            );
        }
    }

    cleanup_local_source(&state, source_id).await;
}

/// Remove a local source's ffmpeg from the map, reap it (a tokio Child
/// dropped without waiting becomes a zombie), and mark the source
/// disconnected. Called on every feed_relay exit path.
async fn cleanup_local_source(state: &AppState, source_id: Uuid) {
    let mut players = state.local_sources.write().await;
    if let Some(mut child) = players.remove(&source_id) {
        // The ffmpeg already exited (stdout closed), so this just collects
        // its exit status instead of leaving a zombie behind.
        let _ = child.kill().await;
    }
    drop(players);
    state.source_states.write().await.insert(
        source_id,
        muxshed_common::SourceState::Disconnected,
    );
}

async fn load_output_config(state: &AppState) -> OutputConfig {
    sqlx::query_as::<_, (String,)>("SELECT value FROM settings WHERE key = 'output_config'")
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten()
        .and_then(|(json,)| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}
