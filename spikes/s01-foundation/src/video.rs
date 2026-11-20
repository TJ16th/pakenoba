//! 回転動画の書き出し（craft-video の H.264 と craft-media の MP4）。

use std::io::Cursor;
use std::path::Path;
use std::time::Instant;

use craft_media::isobmff::{AvcConfig, Brand, Mp4Writer, SampleEntry, TrackConfig, WriteSample, WriterOptions};
use craft_video::h264::{Encoder, EncoderConfig, PacketFormat, Preset, Profile, RateControl, YuvFrame};

use crate::raster::{Frame, Texture, render};
use crate::scene::{BoxDims, Camera};

pub struct VideoSpec {
    pub width: usize,
    pub height: usize,
    pub fps: u32,
    pub seconds: u32,
}

impl Default for VideoSpec {
    fn default() -> Self {
        VideoSpec { width: 1280, height: 720, fps: 30, seconds: 4 }
    }
}

pub struct VideoStats {
    pub frames: u32,
    pub bytes: usize,
    pub render_ms: f64,
    pub encode_ms: f64,
}

/// 箱が1回転する動画を書き出す。`progress` には 0〜1 の進み具合が渡る。
pub fn export_rotation(dims: BoxDims, tex: Option<&Texture>, spec: &VideoSpec, path: &Path, mut progress: impl FnMut(f32)) -> Result<VideoStats, String> {
    // H.264 の 4:2:0 は縦横とも偶数が必要。
    let (w, h) = (spec.width & !1, spec.height & !1);
    if w < 16 || h < 16 || w > u16::MAX as usize || h > u16::MAX as usize {
        return Err(format!("動画の大きさが範囲外です: {w}x{h}"));
    }
    let total = spec.fps * spec.seconds;
    if total == 0 {
        return Err("フレーム数が 0 です".into());
    }

    let mut cfg = EncoderConfig::new(w as u32, h as u32, spec.fps, 1);
    cfg.profile = Profile::High;
    cfg.preset = Preset::Balanced;
    cfg.rate = RateControl::Crf(20.0);
    // B フレームなしにして、表示順と復号順を同じにする（MP4 側の時刻計算が単純になる）。
    cfg.bframes = 0;
    cfg.keyint = spec.fps * 2;
    cfg.format = PacketFormat::LengthPrefixed;
    let mut encoder = Encoder::new(cfg).map_err(|e| format!("エンコーダを作れませんでした: {e}"))?;

    const TIMESCALE: u32 = 90_000;
    let frame_duration = TIMESCALE / spec.fps;
    let avcc = AvcConfig::parse(&encoder.avcc()).map_err(|e| format!("avcC を解釈できませんでした: {e}"))?;
    let mut writer = Mp4Writer::new(Cursor::new(Vec::new()), WriterOptions::new(Brand::Mp4)).map_err(|e| format!("MP4 を開始できませんでした: {e}"))?;
    let track = writer.add_track(TrackConfig::new(SampleEntry::avc(avcc, w as u16, h as u16), TIMESCALE)).map_err(|e| format!("映像トラックを追加できませんでした: {e}"))?;

    let mut yuv = Yuv420::new(w, h);
    let (mut render_ms, mut encode_ms) = (0.0f64, 0.0f64);
    let write = |writer: &mut Mp4Writer<Cursor<Vec<u8>>>, packets: Vec<craft_video::h264::Packet>| -> Result<(), String> {
        for p in packets {
            let offset = (p.pts - p.dts) * frame_duration as i64;
            writer
                .write_sample(track, WriteSample { data: &p.data, duration: frame_duration, composition_offset: offset as i32, is_sync: p.keyframe })
                .map_err(|e| format!("MP4 に書けませんでした: {e}"))?;
        }
        Ok(())
    };

    for i in 0..total {
        let t0 = Instant::now();
        let cam = Camera { yaw: 0.6 + std::f32::consts::TAU * i as f32 / total as f32, pitch: 0.35, zoom: 1.0 };
        let frame = render(dims, cam, tex, w, h, 2);
        yuv.fill_from(&frame);
        render_ms += t0.elapsed().as_secs_f64() * 1000.0;

        let t1 = Instant::now();
        let picture = YuvFrame { y: &yuv.y, u: &yuv.u, v: &yuv.v, y_stride: w, uv_stride: w / 2 };
        let packets = encoder.encode(&picture, i as i64).map_err(|e| format!("{i} フレーム目を圧縮できませんでした: {e}"))?;
        write(&mut writer, packets)?;
        encode_ms += t1.elapsed().as_secs_f64() * 1000.0;
        progress((i + 1) as f32 / total as f32);
    }
    let t1 = Instant::now();
    write(&mut writer, encoder.flush())?;
    let bytes = writer.finish_faststart().map_err(|e| format!("MP4 を閉じられませんでした: {e}"))?.into_inner();
    encode_ms += t1.elapsed().as_secs_f64() * 1000.0;

    std::fs::write(path, &bytes).map_err(|e| format!("{} に保存できませんでした: {e}", path.display()))?;
    Ok(VideoStats { frames: total, bytes: bytes.len(), render_ms, encode_ms })
}

/// 4:2:0 の輝度と色差。BT.709 の限定レンジ（エンコーダの既定の色指定と同じ）。
struct Yuv420 {
    w: usize,
    h: usize,
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
}

impl Yuv420 {
    fn new(w: usize, h: usize) -> Self {
        Yuv420 { w, h, y: vec![0; w * h], u: vec![0; (w / 2) * (h / 2)], v: vec![0; (w / 2) * (h / 2)] }
    }

    fn fill_from(&mut self, frame: &Frame) {
        debug_assert_eq!((frame.w, frame.h), (self.w, self.h));
        let (w, h) = (self.w, self.h);
        for y in 0..h {
            for x in 0..w {
                let o = (y * w + x) * 4;
                let (r, g, b) = (frame.rgba[o] as f32, frame.rgba[o + 1] as f32, frame.rgba[o + 2] as f32);
                self.y[y * w + x] = (16.0 + (0.2126 * r + 0.7152 * g + 0.0722 * b) * 219.0 / 255.0).round() as u8;
            }
        }
        // 色差は 2x2 画素の平均から求める。
        for cy in 0..h / 2 {
            for cx in 0..w / 2 {
                let (mut r, mut g, mut b) = (0.0f32, 0.0f32, 0.0f32);
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let o = ((cy * 2 + dy) * w + cx * 2 + dx) * 4;
                    r += frame.rgba[o] as f32;
                    g += frame.rgba[o + 1] as f32;
                    b += frame.rgba[o + 2] as f32;
                }
                let (r, g, b) = (r * 0.25, g * 0.25, b * 0.25);
                let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
                let cb = (b - luma) / 1.8556;
                let cr = (r - luma) / 1.5748;
                self.u[cy * (w / 2) + cx] = (128.0 + cb * 224.0 / 255.0).round().clamp(16.0, 240.0) as u8;
                self.v[cy * (w / 2) + cx] = (128.0 + cr * 224.0 / 255.0).round().clamp(16.0, 240.0) as u8;
            }
        }
    }
}
