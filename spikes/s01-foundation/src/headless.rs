//! 画面なしの確認。PSD の往復、静止画、動画書き出しを順に行い、結果を表示する。

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;
use std::time::Instant;

use craft_psd::PsdFile;

use crate::psd_io::{load_psd, make_sample_psd};
use crate::raster::{Frame, render};
use crate::scene::{BoxDims, Camera};
use crate::video::{VideoSpec, export_rotation};

pub fn run(out_dir: &Path, psd_path: Option<&Path>) -> Result<(), String> {
    std::fs::create_dir_all(out_dir).map_err(|e| format!("{} を作れませんでした: {e}", out_dir.display()))?;
    let dims = BoxDims::default();

    // 1. PSD
    let bytes = match psd_path {
        Some(p) => {
            let bytes = std::fs::read(p).map_err(|e| format!("{} を読めませんでした: {e}", p.display()))?;
            println!("[psd] 入力 {}（{} バイト）", p.display(), bytes.len());
            bytes
        }
        None => {
            let bytes = make_sample_psd(dims)?;
            let path = out_dir.join("sample.psd");
            std::fs::write(&path, &bytes).map_err(|e| format!("{} に保存できませんでした: {e}", path.display()))?;
            println!("[psd] サンプルを作成 {}（{} バイト）", path.display(), bytes.len());
            bytes
        }
    };
    let t = Instant::now();
    let rewritten = PsdFile::from_bytes(&bytes).and_then(|f| f.to_bytes()).map_err(|e| format!("PSD の往復に失敗: {e}"))?;
    println!("[psd] 読んで書き戻した結果が元と一致: {}", rewritten == bytes);
    let loaded = load_psd(&bytes)?;
    println!(
        "[psd] 統合画像 {}x{}、レイヤー {:?}、本物の統合画像あり: {:?}（{:.1} ms）",
        loaded.texture.w,
        loaded.texture.h,
        loaded.layer_names,
        loaded.has_real_merged,
        t.elapsed().as_secs_f64() * 1000.0
    );

    // 2. 静止画
    let t = Instant::now();
    let frame = render(dims, Camera::default(), Some(&loaded.texture), 1280, 720, 2);
    println!("[still] 1280x720 を描画（{:.1} ms）", t.elapsed().as_secs_f64() * 1000.0);
    let still = out_dir.join("still.png");
    write_png(&still, &frame)?;
    println!("[still] 保存 {}", still.display());

    // 3. 動画
    let spec = VideoSpec::default();
    let video = out_dir.join("rotation.mp4");
    let t = Instant::now();
    let stats = export_rotation(dims, Some(&loaded.texture), &spec, &video, |_| {})?;
    let wall = t.elapsed().as_secs_f64();
    println!(
        "[video] {}x{} {}fps {} フレーム、{} バイト、全体 {:.2} 秒（描画 {:.0} ms、圧縮 {:.0} ms、圧縮は 1 フレームあたり {:.1} ms）",
        spec.width,
        spec.height,
        spec.fps,
        stats.frames,
        stats.bytes,
        wall,
        stats.render_ms,
        stats.encode_ms,
        stats.encode_ms / stats.frames as f64
    );
    println!("[video] 保存 {}", video.display());
    Ok(())
}

fn write_png(path: &Path, frame: &Frame) -> Result<(), String> {
    let file = File::create(path).map_err(|e| format!("{} を作れませんでした: {e}", path.display()))?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), frame.w as u32, frame.h as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| format!("PNG を開始できませんでした: {e}"))?;
    writer.write_image_data(&frame.rgba).map_err(|e| format!("PNG を書けませんでした: {e}"))
}
