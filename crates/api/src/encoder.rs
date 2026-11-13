// Licensed under the GNU Affero General Public License v3.0 — see LICENSE.

//! Shared ffmpeg video-encoder arguments.
//!
//! Upstream Muxshed hardcodes libx264 at every ffmpeg call site. On low-power
//! ARM SoCs (e.g. the Amlogic S905W with the meson V4L2 H.264 encoder driver)
//! libx264 cannot reach realtime at 1080p (veryfast manages ~10 fps), while
//! the hardware encoder (`h264_v4l2m2m`) sustains 50 fps. The encoder is
//! therefore configurable via `OutputConfig::video_encoder`, and every ffmpeg
//! call site builds its encoder arguments through this module so the choice
//! stays consistent.
//!
//! Notes on the v4l2m2m encoder:
//! - It takes no `-preset`/`-tune` (those are x264-only); the driver exposes
//!   its own controls (QP, bitrate) and ffmpeg maps `-b:v` onto them.
//! - It wants NV12 input (`-pix_fmt nv12`); ffmpeg inserts the conversion
//!   from the filter graph's yuv420p automatically.
//! - Decoding stays software everywhere (ffmpeg's default H.264 decoder):
//!   on this hardware, HW decode + HW encode inside a single ffmpeg process
//!   wedges the encoder (the decoder's dmabuf frames must not reach the
//!   encoder zero-copy). SW decode + HW encode in one process is proven.

use crate::routes::output::VideoEncoder;

/// Append the encoder-specific ffmpeg output arguments for `encoder`.
///
/// This covers codec selection, the x264-only preset/tune, and the pixel
/// format. Bitrate (`-b:v`/`-maxrate`/`-bufsize`), GOP (`-g`) and framerate
/// (`-r`) stay at the call site — they vary between call sites.
pub fn push_encoder_args(args: &mut Vec<String>, encoder: VideoEncoder, x264_preset: &str) {
    match encoder {
        VideoEncoder::Libx264 => {
            args.push("-c:v".to_string());
            args.push("libx264".to_string());
            args.push("-preset".to_string());
            args.push(x264_preset.to_string());
            args.push("-tune".to_string());
            args.push("zerolatency".to_string());
            args.push("-pix_fmt".to_string());
            args.push("yuv420p".to_string());
        }
        VideoEncoder::H264V4l2m2m => {
            args.push("-c:v".to_string());
            args.push("h264_v4l2m2m".to_string());
            args.push("-pix_fmt".to_string());
            args.push("nv12".to_string());
        }
    }
}
