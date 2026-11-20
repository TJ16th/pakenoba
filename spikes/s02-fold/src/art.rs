//! 展開図の絵柄とガイド線、PSD との行き来。
//!
//! 絵柄は展開図まるごと 1 枚の画像として持つ（左上が展開図の左上）。

use craft_psd::{BlendMode, LayerSpec, PixelData, PsdBuilder, PsdFile};

use crate::template::{P2, Template};

pub const PX_PER_MM: f32 = 4.0;
const PAPER: [u8; 3] = [240, 236, 228];

/// 展開図の画像。どちらも RGBA 8bit（アルファは乗算なし）で、同じ大きさ。
pub struct SheetArt {
    pub w: usize,
    pub h: usize,
    pub art: Vec<u8>,
    pub guides: Vec<u8>,
}

fn inside(outline: &[P2], p: P2) -> bool {
    // 凸多角形（反時計回り）の内側か。
    (0..outline.len()).all(|i| {
        let (a, b) = (outline[i], outline[(i + 1) % outline.len()]);
        (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]) >= -1e-3
    })
}

fn sheet_size(tpl: &Template) -> (usize, usize) {
    (((tpl.max[0] - tpl.min[0]) * PX_PER_MM).ceil().max(2.0) as usize, ((tpl.max[1] - tpl.min[1]) * PX_PER_MM).ceil().max(2.0) as usize)
}

fn to_px(tpl: &Template, p: P2) -> [f32; 2] {
    [(p[0] - tpl.min[0]) * PX_PER_MM, (tpl.max[1] - p[1]) * PX_PER_MM]
}

/// 確認用の絵柄とガイド線を作る。
pub fn make_sheet_art(tpl: &Template) -> SheetArt {
    let (w, h) = sheet_size(tpl);
    // 透明な部分にも紙の色を入れておく（拡大縮小したときに縁が黒ずまないように）。
    let mut art: Vec<u8> = [PAPER[0], PAPER[1], PAPER[2], 0].repeat(w * h);

    for panel in &tpl.panels {
        let px: Vec<[f32; 2]> = panel.outline.iter().map(|&p| to_px(tpl, p)).collect();
        let x0 = px.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min).floor().max(0.0) as usize;
        let x1 = (px.iter().map(|p| p[0]).fold(f32::NEG_INFINITY, f32::max).ceil() as usize).min(w);
        let y0 = px.iter().map(|p| p[1]).fold(f32::INFINITY, f32::min).floor().max(0.0) as usize;
        let y1 = (px.iter().map(|p| p[1]).fold(f32::NEG_INFINITY, f32::max).ceil() as usize).min(h);
        for y in y0..y1 {
            for x in x0..x1 {
                let (u, v) = (tpl.min[0] + (x as f32 + 0.5) / PX_PER_MM, tpl.max[1] - (y as f32 + 0.5) / PX_PER_MM);
                if !inside(&panel.outline, [u, v]) {
                    continue;
                }
                let color = if panel.hidden_when_closed {
                    // 組み上がると隠れる部分は、紙の色に近い淡い色にする。
                    [232.0, 214.0, 220.0]
                } else {
                    let g = ((v - tpl.min[1]) / (tpl.max[1] - tpl.min[1])).clamp(0.0, 1.0);
                    let mut c = [212.0 + 26.0 * g, 82.0 + 22.0 * g, 130.0 + 18.0 * g];
                    // 斜めの縞。展開図の座標で描くので、折り線をまたいでも続く。
                    if ((u + v) / 9.0).rem_euclid(1.0) < 0.28 {
                        c = [c[0] + (255.0 - c[0]) * 0.18, c[1] + (255.0 - c[1]) * 0.18, c[2] + (255.0 - c[2]) * 0.18];
                    }
                    // 折り線をまたぐ円。
                    for &(center, r) in &tpl.marks {
                        let dist = ((u - center[0]).powi(2) + (v - center[1]).powi(2)).sqrt();
                        if dist < r {
                            c = [255.0, 252.0, 245.0];
                        } else if dist < r * 1.12 {
                            c = [120.0, 20.0, 60.0];
                        }
                    }
                    c
                };
                let o = (y * w + x) * 4;
                art[o..o + 4].copy_from_slice(&[color[0] as u8, color[1] as u8, color[2] as u8, 255]);
            }
        }
    }

    // ガイド線。切り線（輪郭）を先に描き、折り線を上から重ねる。
    let mut guides = vec![0u8; w * h * 4];
    for panel in &tpl.panels {
        for i in 0..panel.outline.len() {
            let (a, b) = (to_px(tpl, panel.outline[i]), to_px(tpl, panel.outline[(i + 1) % panel.outline.len()]));
            draw_line(&mut guides, w, h, a, b, 1.6, [30, 30, 34, 255]);
        }
    }
    for hinge in tpl.panels.iter().filter_map(|p| p.hinge.as_ref()) {
        draw_line(&mut guides, w, h, to_px(tpl, hinge.a), to_px(tpl, hinge.b), 1.6, [0, 150, 220, 255]);
    }
    SheetArt { w, h, art, guides }
}

fn draw_line(buf: &mut [u8], w: usize, h: usize, a: [f32; 2], b: [f32; 2], width: f32, color: [u8; 4]) {
    let half = width * 0.5;
    let x0 = (a[0].min(b[0]) - half).floor().max(0.0) as usize;
    let x1 = ((a[0].max(b[0]) + half).ceil().max(0.0) as usize).min(w.saturating_sub(1));
    let y0 = (a[1].min(b[1]) - half).floor().max(0.0) as usize;
    let y1 = ((a[1].max(b[1]) + half).ceil().max(0.0) as usize).min(h.saturating_sub(1));
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = (dx * dx + dy * dy).max(1e-6);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let (px, py) = (x as f32 + 0.5 - a[0], y as f32 + 0.5 - a[1]);
            let s = ((px * dx + py * dy) / len2).clamp(0.0, 1.0);
            let dist = ((px - s * dx).powi(2) + (py - s * dy).powi(2)).sqrt();
            if dist <= half {
                let o = (y * w + x) * 4;
                buf[o..o + 4].copy_from_slice(&color);
            }
        }
    }
}

/// `over` を `base` の上に重ねる（どちらも RGBA、同じ大きさ）。
fn alpha_over(base: &mut [u8], over: &[u8]) {
    for (b, o) in base.chunks_exact_mut(4).zip(over.chunks_exact(4)) {
        let a = o[3] as f32 / 255.0;
        if a <= 0.0 {
            continue;
        }
        for i in 0..3 {
            b[i] = (o[i] as f32 * a + b[i] as f32 * (1.0 - a)).round() as u8;
        }
        b[3] = (255.0 * (a + b[3] as f32 / 255.0 * (1.0 - a))).round() as u8;
    }
}

/// 箱に貼る画像を作る。`custom` があれば確認用の絵柄の代わりに使う。
pub fn compose(sheet: &SheetArt, custom: Option<&[u8]>, show_guides: bool) -> Vec<u8> {
    let mut out = custom.unwrap_or(&sheet.art).to_vec();
    if show_guides {
        alpha_over(&mut out, &sheet.guides);
    }
    out
}

/// 型紙を PSD にする。レイヤーは「絵柄」と「ガイド」。
pub fn save_psd(sheet: &SheetArt) -> Result<Vec<u8>, String> {
    let (w, h) = (sheet.w as u32, sheet.h as u32);
    let mut composite: Vec<u8> = [255u8, 255, 255, 255].repeat(sheet.w * sheet.h);
    alpha_over(&mut composite, &sheet.art);
    alpha_over(&mut composite, &sheet.guides);
    let mut builder = PsdBuilder::new(w, h);
    builder.push_layer(LayerSpec::new("絵柄", 0, 0, w, h, PixelData::Rgba8(sheet.art.clone())));
    builder.push_layer(LayerSpec::new(&format!("{GUIDE_LAYER_PREFIX}（切り線と折り線）"), 0, 0, w, h, PixelData::Rgba8(sheet.guides.clone())));
    builder.composite(PixelData::Rgba8(composite));
    builder.to_bytes().map_err(|e| format!("PSD を書けませんでした: {e}"))
}

/// ガイドのレイヤーの名前は、この文字で始まる。読み込むときに、この名前で見分けて除く。
pub const GUIDE_LAYER_PREFIX: &str = "ガイド";

/// 読み込んだ PSD に入っていたガイドの扱い。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GuideHandling {
    /// ガイドのレイヤーはなかった。
    NotPresent,
    /// ガイドのレイヤーを除いて、残りのレイヤーを重ねた。
    Removed,
    /// ガイドのレイヤーはあったが、除けなかった（統合画像をそのまま使った）。
    Baked,
}

/// 読み込んだ PSD。画像は展開図の大きさに合わせてある。
pub struct LoadedPsd {
    pub rgba: Vec<u8>,
    pub source_size: [usize; 2],
    pub layer_names: Vec<String>,
    pub has_real_merged: Option<bool>,
    pub guides: GuideHandling,
}

/// ガイド以外のレイヤーを下から順に重ねる。
///
/// 扱えるのは、通常の重ね方（不透明度つき）の画素レイヤーだけ。グループ、マスク、クリッピング、
/// 通常以外の合成モードがあれば None を返す（本番では PhotoCraft のエンジンに任せる部分）。
fn composite_without_guides(file: &PsdFile, w: usize, h: usize) -> Option<Vec<u8>> {
    let mut canvas: Vec<u8> = [PAPER[0], PAPER[1], PAPER[2], 0].repeat(w * h);
    for layer in file.iter_layers() {
        let record = layer.record;
        if layer.name().starts_with(GUIDE_LAYER_PREFIX) || !record.is_visible() {
            continue;
        }
        if record.blend_mode != BlendMode::Normal || record.clipping != 0 || record.section_divider().is_some() || layer.user_mask().is_some() {
            return None;
        }
        let image = layer.rgba8().ok()?;
        let (lw, lh) = (image.width as usize, image.height as usize);
        if image.data.len() != lw * lh * 4 {
            return None;
        }
        let opacity = record.opacity as f32 / 255.0 * record.fill_opacity() as f32 / 255.0;
        for ly in 0..lh {
            let y = image.top as i64 + ly as i64;
            if y < 0 || y >= h as i64 {
                continue;
            }
            for lx in 0..lw {
                let x = image.left as i64 + lx as i64;
                if x < 0 || x >= w as i64 {
                    continue;
                }
                let src = &image.data[(ly * lw + lx) * 4..(ly * lw + lx) * 4 + 4];
                let a = src[3] as f32 / 255.0 * opacity;
                if a <= 0.0 {
                    continue;
                }
                let o = (y as usize * w + x as usize) * 4;
                let below = canvas[o + 3] as f32 / 255.0;
                let out_a = a + below * (1.0 - a);
                for i in 0..3 {
                    canvas[o + i] = ((src[i] as f32 * a + canvas[o + i] as f32 * below * (1.0 - a)) / out_a).round() as u8;
                }
                canvas[o + 3] = (out_a * 255.0).round() as u8;
            }
        }
    }
    Some(canvas)
}

/// PSD を読み、展開図の大きさ（`w` x `h`）に合わせる。
///
/// ガイドのレイヤーが入っていれば、それを除いた絵にする。外部アプリでそのまま書き出すと、
/// 統合画像にはガイドの線が焼き込まれているため。
pub fn load_psd(bytes: &[u8], w: usize, h: usize) -> Result<LoadedPsd, String> {
    let file = PsdFile::from_bytes(bytes).map_err(|e| format!("PSD を読めませんでした: {e}"))?;
    let image = file.composite_rgba8().map_err(|e| format!("統合画像を取り出せませんでした: {e}"))?;
    let (sw, sh) = (image.width as usize, image.height as usize);
    if sw == 0 || sh == 0 || image.data.len() != sw * sh * 4 {
        return Err(format!("統合画像の大きさが合いません（{sw}x{sh}）"));
    }
    let layer_names: Vec<String> = file.iter_layers().map(|l| l.name()).collect();
    let has_guides = layer_names.iter().any(|n| n.starts_with(GUIDE_LAYER_PREFIX));
    let (source, guides) = match (has_guides, has_guides.then(|| composite_without_guides(&file, sw, sh)).flatten()) {
        (false, _) => (image.data, GuideHandling::NotPresent),
        (true, Some(clean)) => (clean, GuideHandling::Removed),
        (true, None) => (image.data, GuideHandling::Baked),
    };
    // 近い画素を拾うだけの単純な拡大縮小。
    let mut rgba = vec![0u8; w * h * 4];
    for y in 0..h {
        let sy = (y * sh / h).min(sh - 1);
        for x in 0..w {
            let sx = (x * sw / w).min(sw - 1);
            rgba[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&source[(sy * sw + sx) * 4..(sy * sw + sx) * 4 + 4]);
        }
    }
    Ok(LoadedPsd { rgba, source_size: [sw, sh], layer_names, has_real_merged: file.has_real_merged_data(), guides })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dieline_psd_round_trips() {
        let tpl = Template::tuck_box(120.0, 60.0, 160.0, 1.5);
        let sheet = make_sheet_art(&tpl);
        assert_eq!(sheet.art.len(), sheet.w * sheet.h * 4);
        assert_eq!(sheet.guides.len(), sheet.w * sheet.h * 4);
        // 絵柄にもガイドにも、何かが描かれている。
        assert!(sheet.art.chunks_exact(4).any(|p| p[3] == 255));
        assert!(sheet.guides.chunks_exact(4).any(|p| p[3] == 255));

        let bytes = save_psd(&sheet).unwrap();
        let loaded = load_psd(&bytes, sheet.w, sheet.h).unwrap();
        assert_eq!(loaded.source_size, [sheet.w, sheet.h]);
        assert_eq!(loaded.layer_names, vec!["絵柄".to_owned(), "ガイド（切り線と折り線）".to_owned()]);
        assert_eq!(loaded.rgba.len(), sheet.w * sheet.h * 4);
        // 貼る画像は、絵柄とガイドを重ねたものと同じ大きさになる。
        assert_eq!(compose(&sheet, Some(&loaded.rgba), true).len(), sheet.art.len());
    }

    #[test]
    fn guides_are_removed_on_load() {
        let tpl = Template::tuck_box(120.0, 60.0, 160.0, 1.5);
        let sheet = make_sheet_art(&tpl);
        let loaded = load_psd(&save_psd(&sheet).unwrap(), sheet.w, sheet.h).unwrap();
        assert_eq!(loaded.guides, GuideHandling::Removed);
        // ガイドの線があった画素でも、読み込んだ絵は元の絵柄と同じになる（線が焼き込まれていない）。
        let mut on_guides = 0;
        for i in 0..sheet.w * sheet.h {
            if sheet.guides[i * 4 + 3] == 255 && sheet.art[i * 4 + 3] == 255 {
                assert_eq!(&loaded.rgba[i * 4..i * 4 + 4], &sheet.art[i * 4..i * 4 + 4]);
                on_guides += 1;
            }
        }
        assert!(on_guides > 1000);
    }
}
