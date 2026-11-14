// Licensed under the GNU Affero General Public License v3.0 — see LICENSE.

//! Browser source — render any web page as a live video source.
//!
//! Spawns a headless Chrome, drives it over the DevTools protocol
//! (`Page.captureScreenshot` in a loop for a steady frame rate even when the
//! page is static), and pipes the frames through ffmpeg into the source's media
//! relay. From there it behaves like any other source — switchable, and
//! composable as a scene layer. This is the substrate for dynamic text and chat
//! overlays (both are just HTML pages rendered as browser sources).

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::process::Command;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::routes::output::OutputConfig;
use crate::state::AppState;

fn chrome_path() -> String {
    if let Ok(p) = std::env::var("MUXSHED_CHROME_PATH") {
        return p;
    }
    for c in [
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
        "/usr/bin/google-chrome",
        "/usr/bin/google-chrome-stable",
    ] {
        if std::path::Path::new(c).exists() {
            return c.to_string();
        }
    }
    "chromium".to_string()
}

pub async fn start_browser_source(
    state: Arc<AppState>,
    source_id: Uuid,
    url: String,
) -> Result<(), String> {
    let cfg = load_output_config(&state).await;
    let port = pick_port().await?;

    // Low-power cap: headless Chrome at the full output canvas (e.g. 1920x1080
    // with 30 fps screenshots) is too heavy for small SoCs — Chrome alone eats
    // 200-400 MB and the screenshot loop saturates the little cores. Cap the
    // render size and screenshot rate via env vars; unset = upstream behavior
    // (full canvas). The ffmpeg scale/pad + fps filter still brings the source
    // up to the output canvas, so a capped browser source composites normally.
    let cap = |name: &str| std::env::var(name).ok()?.parse::<u32>().ok().filter(|v| *v > 0);
    let bw = cap("MUXSHED_BROWSER_MAX_WIDTH").map_or(cfg.width, |m| cfg.width.min(m));
    let bh = cap("MUXSHED_BROWSER_MAX_HEIGHT").map_or(cfg.height, |m| cfg.height.min(m));
    let bfps = cap("MUXSHED_BROWSER_MAX_FPS").map_or(cfg.fps, |m| cfg.fps.min(m));
    if bw != cfg.width || bh != cfg.height || bfps != cfg.fps {
        tracing::info!(
            "browser source {}: capped to {}x{}@{}fps (canvas {}x{}@{}fps)",
            source_id, bw, bh, bfps, cfg.width, cfg.height, cfg.fps
        );
    }

    // Headless Chrome rendering the page at the (possibly capped) size.
    // Each source gets its own profile directory: Chrome locks its profile,
    // so two sources (or a manual Chrome) sharing the default profile would
    // contend for the SingletonLock and one of them would die.
    let profile_dir = state
        .config
        .read()
        .await
        .data_dir
        .join(format!("chrome-{}", source_id));
    let mut chrome = Command::new(chrome_path())
        .args([
            "--headless=new",
            "--disable-gpu",
            "--hide-scrollbars",
            "--no-sandbox",
            "--mute-audio",
            "--autoplay-policy=no-user-gesture-required",
            "--disable-dev-shm-usage",
            &format!("--user-data-dir={}", profile_dir.display()),
            &format!("--remote-debugging-port={}", port),
            &format!("--window-size={},{}", bw, bh),
            &url,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("failed to launch chrome: {}", e))?;

    // Find the page's DevTools WebSocket endpoint.
    let ws_url = match wait_for_page_ws(port).await {
        Ok(u) => u,
        Err(e) => {
            let _ = chrome.kill().await;
            return Err(e);
        }
    };

    // ffmpeg: JPEG frames in (paced by wallclock) -> normalized FLV out.
    let public_tx = state.get_or_create_media_relay(source_id).await;
    let vf = format!(
        "scale={}:{}:force_original_aspect_ratio=decrease,pad={}:{}:(ow-iw)/2:(oh-ih)/2:black,fps={},format=yuv420p",
        cfg.width, cfg.height, cfg.width, cfg.height, cfg.fps
    );
    let bv = format!("{}k", cfg.video_bitrate_kbps);
    let bufsize = format!("{}k", cfg.video_bitrate_kbps * 2);
    let gop = format!("{}", cfg.fps * 2);
    let mut args: Vec<String> = vec![
        "-hide_banner".into(), "-loglevel".into(), "warning".into(),
        "-f".into(), "image2pipe".into(), "-use_wallclock_as_timestamps".into(), "1".into(), "-i".into(), "pipe:0".into(),
        "-f".into(), "lavfi".into(), "-i".into(), "anullsrc=r=48000:cl=stereo".into(),
        "-vf".into(), vf,
    ];
    // Encoder from the output config (libx264 upstream default,
    // h264_v4l2m2m on SoCs with a V4L2 hardware encoder).
    crate::encoder::push_encoder_args(&mut args, cfg.video_encoder, "veryfast");
    args.extend([
        "-b:v".into(), bv.clone(), "-maxrate".into(), bv, "-bufsize".into(), bufsize,
        "-g".into(), gop,
        "-c:a".into(), "aac".into(), "-b:a".into(), "128k".into(), "-ar".into(), "48000".into(),
        "-shortest".into(), "-f".into(), "flv".into(), "-flvflags".into(), "no_duration_filesize".into(), "pipe:1".into(),
    ]);

    let mut ff = Command::new("ffmpeg")
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("failed to start browser ffmpeg: {}", e))?;

    let ff_stdin = ff.stdin.take().ok_or("no ffmpeg stdin")?;
    let ff_stdout = ff.stdout.take().ok_or("no ffmpeg stdout")?;

    // Track both processes for teardown (chrome under browser_sources, ffmpeg under normalizers).
    state.browser_sources.write().await.insert(source_id, chrome);
    {
        let mut n = state.source_normalizers.write().await;
        if let Some(mut old) = n.insert(source_id, ff) {
            let _ = old.kill().await;
        }
    }

    tracing::info!("starting browser source {} -> {}", source_id, url);

    // Drive screenshots into ffmpeg.
    let st = state.clone();
    tokio::spawn(async move {
        screenshot_loop(ws_url, ff_stdin, bfps, source_id, st).await;
    });

    // Read the encoded output into the relay.
    let st = state.clone();
    tokio::spawn(async move {
        read_flv_output(source_id, ff_stdout, public_tx, st).await;
    });

    Ok(())
}

async fn pick_port() -> Result<u16, String> {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| e.to_string())?;
    let p = l.local_addr().map_err(|e| e.to_string())?.port();
    Ok(p)
}

/// Poll the DevTools HTTP endpoint until the page target's WebSocket URL appears.
///
/// 30 s budget: a cold Chrome start on a small SoC (Amlogic S905W) takes
/// 5-10 s even warm, longer on first run (profile creation) or under load.
/// The old 5 s budget killed Chrome before DevTools ever came up.
async fn wait_for_page_ws(port: u16) -> Result<String, String> {
    for _ in 0..300 {
        if let Ok(body) = http_get(&format!("127.0.0.1:{}", port), "/json").await {
            if let Ok(targets) = serde_json::from_str::<serde_json::Value>(&body) {
                if let Some(arr) = targets.as_array() {
                    for t in arr {
                        if t.get("type").and_then(|v| v.as_str()) == Some("page") {
                            if let Some(ws) = t.get("webSocketDebuggerUrl").and_then(|v| v.as_str())
                            {
                                return Ok(ws.to_string());
                            }
                        }
                    }
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    Err("chrome devtools did not come up".to_string())
}

async fn screenshot_loop(
    ws_url: String,
    mut ff_stdin: tokio::process::ChildStdin,
    fps: u32,
    source_id: Uuid,
    state: Arc<AppState>,
) {
    let (ws, _) = match tokio_tungstenite::connect_async(&ws_url).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("browser {}: devtools connect failed: {}", source_id, e);
            return;
        }
    };
    let (mut sink, mut stream) = ws.split();
    use tokio_tungstenite::tungstenite::Message;

    let _ = sink
        .send(Message::Text(r#"{"id":1,"method":"Page.enable"}"#.into()))
        .await;

    let frame_interval = std::time::Duration::from_millis((1000 / fps.max(1)).max(20) as u64);
    let mut id: u64 = 2;
    // Consecutive failed captures. A single failure must NOT kill the
    // source: the first capture often fails while the page is still loading
    // (slow SoC, heavy page). Only give up after several in a row — a truly
    // dead Chrome fails the WS send immediately anyway.
    let mut consecutive_failures: u32 = 0;
    loop {
        if state.browser_sources.read().await.get(&source_id).is_none() {
            break; // torn down
        }
        let req = format!(
            r#"{{"id":{},"method":"Page.captureScreenshot","params":{{"format":"jpeg","quality":70}}}}"#,
            id
        );
        if sink.send(Message::Text(req.into())).await.is_err() {
            break;
        }
        // Read until the matching response, with a timeout: if Chrome dies,
        // the WebSocket can stay half-open forever and this loop would hang
        // with ffmpeg waiting on stdin. Breaking here drops ff_stdin, which
        // ends the image2pipe input and lets ffmpeg exit (via -shortest).
        let mut data: Option<String> = None;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        while tokio::time::Instant::now() < deadline {
            let next = tokio::time::timeout_at(deadline, stream.next()).await;
            let Some(Ok(msg)) = next.ok().flatten() else {
                break;
            };
            if let Message::Text(txt) = msg {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&txt) {
                    if v.get("id").and_then(|x| x.as_u64()) == Some(id) {
                        data = v
                            .get("result")
                            .and_then(|r| r.get("data"))
                            .and_then(|d| d.as_str())
                            .map(|s| s.to_string());
                        break;
                    }
                }
            }
        }
        id += 1;
        if let Some(b64) = data {
            consecutive_failures = 0;
            use base64::Engine;
            if let Ok(jpeg) = base64::engine::general_purpose::STANDARD.decode(b64.as_bytes()) {
                if ff_stdin.write_all(&jpeg).await.is_err() {
                    break;
                }
            }
        } else {
            consecutive_failures += 1;
            tracing::debug!(
                "browser {}: screenshot failed ({}/5)",
                source_id, consecutive_failures
            );
            if consecutive_failures >= 5 {
                tracing::warn!(
                    "browser {}: screenshots failing repeatedly, giving up",
                    source_id
                );
                break;
            }
        }
        tokio::time::sleep(frame_interval).await;
    }
}

async fn read_flv_output(
    source_id: Uuid,
    mut stdout: tokio::process::ChildStdout,
    public_tx: broadcast::Sender<Bytes>,
    state: Arc<AppState>,
) {
    let mut header = [0u8; 13];
    if stdout.read_exact(&mut header).await.is_err() {
        tracing::warn!("browser: failed to read FLV header for {}", source_id);
        return;
    }
    let mut frame_count: u64 = 0;
    loop {
        let mut tag_header = [0u8; 11];
        if stdout.read_exact(&mut tag_header).await.is_err() {
            break;
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
        let mut tag_buf = Vec::with_capacity(11 + data_size as usize + 4);
        tag_buf.extend_from_slice(&tag_header);
        tag_buf.extend_from_slice(&data);
        tag_buf.extend_from_slice(&prev_size);
        let tag = Bytes::from(tag_buf);

        if tag_type == 9 && !data.is_empty() {
            let avc = if data.len() > 1 { data[1] } else { 255 };
            let frame_type = (data[0] >> 4) & 0x0F;
            if avc == 0 {
                state.sequence_headers.write().await.entry(source_id).or_default().video =
                    Some(tag.clone());
            } else if frame_type == 1 {
                let mut h = state.sequence_headers.write().await;
                if let Some(e) = h.get_mut(&source_id) {
                    e.last_keyframe = Some(tag.clone());
                }
            }
        } else if tag_type == 8 && !data.is_empty() {
            let aac = if data.len() > 1 { data[1] } else { 255 };
            if aac == 0 {
                state.sequence_headers.write().await.entry(source_id).or_default().audio =
                    Some(tag.clone());
            }
        }

        if frame_count == 0 {
            state
                .source_states
                .write()
                .await
                .insert(source_id, muxshed_common::SourceState::Live);
            let _ = state.ws_tx.send(muxshed_common::WsEvent::SourceState {
                id: source_id,
                state: muxshed_common::SourceState::Live,
            });
            tracing::info!("browser source {} is live", source_id);
        }
        let _ = public_tx.send(tag);
        frame_count += 1;
    }
    tracing::info!("browser source {} ended after {} frames", source_id, frame_count);
}

pub async fn stop_browser_source(state: &AppState, source_id: &Uuid) {
    if let Some(mut chrome) = state.browser_sources.write().await.remove(source_id) {
        let _ = chrome.kill().await;
    }
    if let Some(mut ff) = state.source_normalizers.write().await.remove(source_id) {
        let _ = ff.kill().await;
    }
    state.remove_media_relay(source_id).await;
    state.sequence_headers.write().await.remove(source_id);
    state
        .source_states
        .write()
        .await
        .insert(*source_id, muxshed_common::SourceState::Disconnected);
}

async fn http_get(authority: &str, path: &str) -> Result<String, String> {
    use std::time::Duration;
    let mut s = TcpStream::connect(authority).await.map_err(|e| e.to_string())?;
    let req = format!("GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n", path, authority);
    s.write_all(req.as_bytes()).await.map_err(|e| e.to_string())?;
    // Chrome's DevTools HTTP keeps the socket open — read with a timeout rather
    // than waiting for EOF, and stop once we have the full JSON body.
    let mut buf = Vec::new();
    let mut tmp = [0u8; 8192];
    loop {
        match tokio::time::timeout(Duration::from_millis(700), s.read(&mut tmp)).await {
            Ok(Ok(0)) | Ok(Err(_)) | Err(_) => break,
            Ok(Ok(n)) => {
                buf.extend_from_slice(&tmp[..n]);
                let text = String::from_utf8_lossy(&buf);
                if let Some((_, body)) = text.split_once("\r\n\r\n") {
                    let b = body.trim();
                    if (b.starts_with('[') && b.ends_with(']'))
                        || (b.starts_with('{') && b.ends_with('}'))
                    {
                        return Ok(b.to_string());
                    }
                }
            }
        }
    }
    let text = String::from_utf8_lossy(&buf);
    Ok(text.split_once("\r\n\r\n").map(|(_, b)| b.trim().to_string()).unwrap_or_default())
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
