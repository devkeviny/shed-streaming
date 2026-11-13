# Muxshed on the Aquario STV-2000 (S905W)

Native (non-Docker) install of this fork on the Aquario TV box, using the
meson V4L2 hardware H.264 encoder (`h264_v4l2m2m`).

## Ports on this box

| Service | Port | Notes |
| --- | --- | --- |
| Muxshed web UI + API | **8090** | 8080 is taken by the closeli camera relay |
| Muxshed RTMP ingest | 1935 | OBS / hardware encoders point here |
| Muxshed SRT ingest | 9000+ | one port per SRT source |
| MiBeeNvr | 9090 | unrelated service on the same box |

## Build (on a dev machine, not the box)

The box cannot compile this itself (Rust release builds need >1 GB RAM).

```sh
# Backend (cross-compile for aarch64 with zig as the C toolchain)
cargo zigbuild --release --target aarch64-unknown-linux-gnu -p muxshed-api
# Frontend (static files, architecture-independent)
cd web && npm ci && npm run build
```

## Install

Copy `target/aarch64-unknown-linux-gnu/release/muxshed-api`,
`web/build/` (as `web/`), and this directory to the box, then run
`install.sh` as root. It installs the binary, the web UI, and the systemd
unit, and starts the service.

## First run

Open `http://<box-ip>:8090` and complete the setup wizard (creates the admin
API key). Then in Output Config select:

- **Video encoder:** `h264_v4l2m2m` (hardware) — libx264 cannot reach
  realtime at 1080p on this SoC (~10 fps vs ~50 fps hardware).
- **Pass program video through to RTMP:** on, when the program comes from
  a scene — saves one full encode per destination.

## Box-specific behaviour in this fork

- The scene compositor has a watchdog: the V4L2 encoder can wedge at
  startup (firmware stalls in the first frames, ~20-40% of starts), and
  the only reliable recovery is a fresh ffmpeg process. The watchdog
  restarts the compositor after 6 s without a frame (10 s grace), up to
  5 attempts.
- Decoding stays software everywhere: HW decode + HW encode inside one
  ffmpeg process wedges this hardware.
- Browser sources are capped via `MUXSHED_BROWSER_MAX_WIDTH/HEIGHT/FPS`
  (960x540 @ 10 fps here) — full-canvas headless Chrome does not fit the
  box's 787 MB RAM.
