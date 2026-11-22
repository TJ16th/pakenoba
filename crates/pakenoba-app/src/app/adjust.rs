//! 画像の柄の調整。背景を透明にする、陰影をならす、色数を絞る、余白を切り落とす。
//!
//! 元の素材は変えない。調整した絵を新しい素材として足し、選んでいる柄 1 つだけを、それに差し替える。
//! Ctrl+Z で戻せる。見た目の位置と大きさは保つ（余白を切り落としたときも、残った部分は同じ場所に見える）。

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use eframe::egui;

use pakenoba_core::project;

use super::assets::Asset;
use super::{App, file};
use crate::paint::Picture;

/// 調整の種類。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Adjust {
    /// 縁につながる背景の色を、透明にする。数は、同じ色とみなす幅（0〜255）。
    ClearBackground(f32),
    /// 照明のむらによる明るさの差を、ならす（撮った写真の影を薄くする）。
    FlattenShading,
    /// 色の数を、この数まで減らす。
    Posterize(usize),
    /// まわりの透明な余白を、切り落とす。
    Trim,
}

impl Adjust {
    fn label(self) -> &'static str {
        match self {
            Adjust::ClearBackground(_) => "背景を透明に",
            Adjust::FlattenShading => "陰影をならす",
            Adjust::Posterize(_) => "色数を絞る",
            Adjust::Trim => "余白を切る",
        }
    }
}

/// 調整した絵。`crop` は、元の絵のどこを切り出したか（左、上、幅、高さ。px）。
pub(super) struct Adjusted {
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
    pub crop: [usize; 4],
}

fn rgb_at(rgba: &[u8], i: usize) -> [f32; 3] {
    [
        rgba[i * 4] as f32,
        rgba[i * 4 + 1] as f32,
        rgba[i * 4 + 2] as f32,
    ]
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// 縁で一番多い色（16 段階にまとめて数え、その段の平均）。縁がすべて透明なら None。
fn border_color(w: usize, h: usize, rgba: &[u8]) -> Option<[f32; 3]> {
    let mut bins: HashMap<[u8; 3], (usize, [f32; 3])> = HashMap::new();
    let border = (0..w)
        .flat_map(|x| [x, (h - 1) * w + x])
        .chain((0..h).flat_map(|y| [y * w, y * w + w - 1]));
    for i in border {
        if rgba[i * 4 + 3] < 128 {
            continue;
        }
        let c = rgb_at(rgba, i);
        let entry = bins
            .entry([rgba[i * 4] >> 4, rgba[i * 4 + 1] >> 4, rgba[i * 4 + 2] >> 4])
            .or_insert((0, [0.0; 3]));
        entry.0 += 1;
        for (sum, v) in entry.1.iter_mut().zip(c) {
            *sum += v;
        }
    }
    let (count, sum) = bins.into_values().max_by_key(|(n, _)| *n)?;
    Some(sum.map(|v| v / count as f32))
}

/// 縁からつながっている背景の色を、透明にする。背景との境目は、近さに応じて半ば透かす。
pub(super) fn clear_background(w: usize, h: usize, rgba: &[u8], tolerance: f32) -> Option<Vec<u8>> {
    let seed = border_color(w, h, rgba)?;
    let tolerance = tolerance.max(1.0);
    let mut out = rgba.to_vec();
    let mut seen = vec![false; w * h];
    let mut queue = VecDeque::new();
    let near = |i: usize| rgba[i * 4 + 3] < 8 || distance(rgb_at(rgba, i), seed) <= tolerance;
    let border = (0..w)
        .flat_map(|x| [x, (h - 1) * w + x])
        .chain((0..h).flat_map(|y| [y * w, y * w + w - 1]));
    for i in border {
        if !seen[i] && near(i) {
            seen[i] = true;
            queue.push_back(i);
        }
    }
    let mut cleared = 0;
    while let Some(i) = queue.pop_front() {
        out[i * 4 + 3] = 0;
        cleared += 1;
        let (x, y) = (i % w, i / w);
        let neighbors = [
            (x > 0).then(|| i - 1),
            (x + 1 < w).then(|| i + 1),
            (y > 0).then(|| i - w),
            (y + 1 < h).then(|| i + w),
        ];
        for j in neighbors.into_iter().flatten() {
            if seen[j] {
                continue;
            }
            if near(j) {
                seen[j] = true;
                queue.push_back(j);
            } else {
                // 背景に接する、背景に近い色の画素は、近さに応じて透かす（縁のぎざぎざを和らげる）。
                let t = ((distance(rgb_at(rgba, j), seed) - tolerance) / tolerance).clamp(0.0, 1.0);
                let a = &mut out[j * 4 + 3];
                *a = (*a as f32 * t).round().min(*a as f32) as u8;
            }
        }
    }
    (cleared > 0).then_some(out)
}

/// 照明のむらを見積もって割り、明るさをならす。色みは保つ。
///
/// 照明は、紙の地（明るいところ）の明るさから見積もる。絵を粗いマスに分けて、マスの中でいちばん明るい値を取り、
/// まわりのマスの最大でならしてから、ぼかす。暗い絵柄に引っぱられて、まわりが明るく浮くのを防ぐ。
pub(super) fn flatten_shading(w: usize, h: usize, rgba: &[u8]) -> Vec<u8> {
    let luma = |i: usize| {
        let c = rgb_at(rgba, i);
        0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]
    };
    // 長いほうが 64 マスくらいになる粗いマス。
    let cell = w.max(h).div_ceil(64).max(1);
    let (gw, gh) = (w.div_ceil(cell), h.div_ceil(cell));
    let mut grid = vec![0.0f32; gw * gh];
    for y in 0..h {
        for x in 0..w {
            let g = (y / cell) * gw + x / cell;
            grid[g] = grid[g].max(luma(y * w + x));
        }
    }
    let reach = (gw.max(gh) / 6).max(1);
    let grid = max_filter(gw, gh, &grid, reach);
    let blur = (reach / 2).max(1);
    let light = box_blur(gw, gh, &box_blur(gw, gh, &grid, blur), blur);
    // ならした後の明るさは、いちばん明るい側（紙の白など）にそろえる。
    let target = light.iter().copied().fold(1.0f32, f32::max);
    // マスの中心どうしを、なめらかにつなぐ。
    let sample = |x: usize, y: usize| {
        let fx = ((x as f32 + 0.5) / cell as f32 - 0.5).clamp(0.0, (gw - 1) as f32);
        let fy = ((y as f32 + 0.5) / cell as f32 - 0.5).clamp(0.0, (gh - 1) as f32);
        let (x0, y0) = (fx as usize, fy as usize);
        let (x1, y1) = ((x0 + 1).min(gw - 1), (y0 + 1).min(gh - 1));
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
        let at = |x: usize, y: usize| light[y * gw + x];
        (at(x0, y0) * (1.0 - tx) + at(x1, y0) * tx) * (1.0 - ty) + (at(x0, y1) * (1.0 - tx) + at(x1, y1) * tx) * ty
    };
    let mut out = rgba.to_vec();
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let gain = (target / sample(x, y).max(1.0)).clamp(1.0, 3.0);
            for k in 0..3 {
                out[i * 4 + k] = (rgba[i * 4 + k] as f32 * gain).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

/// まわり `reach` マスの最大（縦と横に分けて）。
fn max_filter(w: usize, h: usize, values: &[f32], reach: usize) -> Vec<f32> {
    let pass = |values: &[f32], len: usize, count: usize, at: &dyn Fn(usize, usize) -> usize| {
        let mut out = vec![0.0; values.len()];
        for line in 0..count {
            for k in 0..len {
                let (lo, hi) = (k.saturating_sub(reach), (k + reach + 1).min(len));
                out[at(line, k)] = (lo..hi).map(|j| values[at(line, j)]).fold(f32::MIN, f32::max);
            }
        }
        out
    };
    let rows = pass(values, w, h, &|y, x| y * w + x);
    pass(&rows, h, w, &|x, y| y * w + x)
}

/// 箱形のぼかし（縦と横、累積和で）。
fn box_blur(w: usize, h: usize, values: &[f32], radius: usize) -> Vec<f32> {
    let pass = |values: &[f32], len: usize, count: usize, at: &dyn Fn(usize, usize) -> usize| {
        let mut out = vec![0.0; values.len()];
        for line in 0..count {
            let mut sums = vec![0.0f64; len + 1];
            for k in 0..len {
                sums[k + 1] = sums[k] + values[at(line, k)] as f64;
            }
            for k in 0..len {
                let (lo, hi) = (k.saturating_sub(radius), (k + radius + 1).min(len));
                out[at(line, k)] = ((sums[hi] - sums[lo]) / (hi - lo) as f64) as f32;
            }
        }
        out
    };
    let rows = pass(values, w, h, &|y, x| y * w + x);
    pass(&rows, h, w, &|x, y| y * w + x)
}

/// 色を `count` 色に減らす（k 平均法）。透明な画素は、そのまま。
pub(super) fn posterize(w: usize, h: usize, rgba: &[u8], count: usize) -> Vec<u8> {
    let count = count.clamp(2, 64);
    let opaque: Vec<usize> = (0..w * h).filter(|&i| rgba[i * 4 + 3] >= 8).collect();
    let mut out = rgba.to_vec();
    if opaque.is_empty() {
        return out;
    }
    // 速さのため、多いときは間引いた画素で色を決める。
    let step = (opaque.len() / 40_000).max(1);
    let samples: Vec<[f32; 3]> = opaque
        .iter()
        .step_by(step)
        .map(|&i| rgb_at(rgba, i))
        .collect();
    // はじめの色は、明るさの順に並べて、等間隔に取る。
    let mut by_light = samples.clone();
    by_light.sort_by(|a, b| (a[0] + a[1] + a[2]).total_cmp(&(b[0] + b[1] + b[2])));
    let mut centers: Vec<[f32; 3]> = (0..count)
        .map(|k| by_light[(k * 2 + 1) * by_light.len() / (count * 2)])
        .collect();
    let nearest = |centers: &[[f32; 3]], c: [f32; 3]| {
        (0..centers.len())
            .min_by(|&a, &b| distance(centers[a], c).total_cmp(&distance(centers[b], c)))
            .unwrap_or(0)
    };
    for _ in 0..10 {
        let mut sums = vec![([0.0f64; 3], 0usize); count];
        for &c in &samples {
            let k = nearest(&centers, c);
            for (sum, v) in sums[k].0.iter_mut().zip(c) {
                *sum += v as f64;
            }
            sums[k].1 += 1;
        }
        for (center, (sum, n)) in centers.iter_mut().zip(&sums) {
            if *n > 0 {
                *center = sum.map(|v| (v / *n as f64) as f32);
            }
        }
    }
    for &i in &opaque {
        let c = centers[nearest(&centers, rgb_at(rgba, i))];
        for k in 0..3 {
            out[i * 4 + k] = c[k].round() as u8;
        }
    }
    out
}

/// 透明でない部分を囲む四角（左、上、幅、高さ）。すべて透明なら None。
pub(super) fn opaque_bounds(w: usize, h: usize, rgba: &[u8]) -> Option<[usize; 4]> {
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for y in 0..h {
        for x in 0..w {
            if rgba[(y * w + x) * 4 + 3] > 8 {
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
            }
        }
    }
    (x1 > x0).then_some([x0, y0, x1 - x0, y1 - y0])
}

/// 調整した絵を作る。変わらないとき（背景が見つからない、余白がない）は、理由を返す。
pub(super) fn run(how: Adjust, w: usize, h: usize, rgba: &[u8]) -> Result<Adjusted, &'static str> {
    let whole = [0, 0, w, h];
    match how {
        Adjust::ClearBackground(tolerance) => {
            let rgba = clear_background(w, h, rgba, tolerance)
                .ok_or("縁に、透明にできる背景が見つかりませんでした。")?;
            Ok(Adjusted {
                w,
                h,
                rgba,
                crop: whole,
            })
        }
        Adjust::FlattenShading => Ok(Adjusted {
            w,
            h,
            rgba: flatten_shading(w, h, rgba),
            crop: whole,
        }),
        Adjust::Posterize(count) => Ok(Adjusted {
            w,
            h,
            rgba: posterize(w, h, rgba, count),
            crop: whole,
        }),
        Adjust::Trim => {
            let crop = opaque_bounds(w, h, rgba).ok_or("すべて透明なので、切り落とせません。")?;
            if crop == whole {
                return Err("切り落とせる透明な余白がありません。");
            }
            let [x0, y0, cw, ch] = crop;
            let mut out = Vec::with_capacity(cw * ch * 4);
            for y in y0..y0 + ch {
                out.extend_from_slice(&rgba[(y * w + x0) * 4..(y * w + x0 + cw) * 4]);
            }
            Ok(Adjusted {
                w: cw,
                h: ch,
                rgba: out,
                crop,
            })
        }
    }
}

/// 調整の欄で覚えておく値。
#[derive(Clone, Copy, Debug)]
pub(super) struct AdjustSettings {
    pub tolerance: f32,
    pub colors: usize,
}

impl Default for AdjustSettings {
    fn default() -> Self {
        AdjustSettings {
            tolerance: 40.0,
            colors: 6,
        }
    }
}

impl App {
    /// 画像の柄 `index` を調整する。調整した絵を新しい素材として足し、この柄だけを差し替える。
    pub(super) fn adjust_decal(&mut self, index: usize, how: Adjust) {
        let Some(placed) = self.decals.get(index) else {
            return;
        };
        let Picture::Image { w, h, rgba, vector: false } = &placed.picture else {
            return;
        };
        let (w, h) = (*w, *h);
        let adjusted = match run(how, w, h, rgba) {
            Ok(adjusted) => adjusted,
            Err(why) => return self.say(why),
        };
        let bytes =
            match super::outputs::png_bytes(adjusted.w as u32, adjusted.h as u32, &adjusted.rgba) {
                Ok(bytes) => bytes,
                Err(e) => return self.say(format!("調整した絵を作れませんでした: {e}")),
            };
        // 名前は「元の名前（調整）.png」。
        let base = self
            .asset_of(&placed.picture)
            .map_or_else(|| "画像".to_owned(), |i| self.assets[i].name.clone());
        let stem = base
            .rsplit_once('.')
            .map_or(base.as_str(), |(stem, _)| stem);
        let name = format!("{stem}（{}）.png", how.label());
        // 大きさと位置。切り落とした部分のぶん小さくし、残った部分の中心が同じ場所に見えるようにずらす。
        let template = self.template.clone();
        let mut decal = placed.decal;
        let [x0, y0, cw, ch] = adjusted.crop;
        let (sx, sy) = (decal.size[0] / w as f32, decal.size[1] / h as f32);
        let offset = [
            ((x0 as f32 + cw as f32 * 0.5) - w as f32 * 0.5) * sx,
            (h as f32 * 0.5 - (y0 as f32 + ch as f32 * 0.5)) * sy,
        ];
        let center = decal.place(&template).point(offset);
        decal.size = [cw as f32 * sx, ch as f32 * sy];
        decal.set_center(&template, center);

        let stored = project::Asset::new(name.clone(), bytes, decal.size, file::now_local());
        self.checkpoint();
        let known = self
            .assets
            .iter()
            .find(|a| a.stored.as_ref().is_some_and(|s| s.id() == stored.id()))
            .map(|a| a.picture.clone());
        let picture = known.unwrap_or_else(|| {
            let picture = Picture::Image {
                w: adjusted.w,
                h: adjusted.h,
                rgba: Arc::new(adjusted.rgba),
                vector: false,
            };
            self.assets.push(Asset {
                name: name.clone(),
                picture: picture.clone(),
                size: stored.size,
                thumb: None,
                bundled: false,
                stored: Some(stored),
            });
            picture
        });
        let placed = &mut self.decals[index];
        placed.picture = picture;
        placed.decal = decal;
        (self.art_dirty, self.back_dirty) = (true, true);
        self.say(format!("「{}」をしました。新しい素材「{name}」になり、元の素材はそのままです。Ctrl+Z で戻せます。", how.label()));
    }

    /// 右の欄の、画像の調整。画像の柄を選んでいるときに出す。
    pub(super) fn adjust_controls(&mut self, ui: &mut egui::Ui, index: usize, locked: bool) {
        if !matches!(
            self.decals.get(index).map(|d| &d.picture),
            Some(Picture::Image { vector: false, .. })
        ) {
            return;
        }
        let mut chosen = None;
        egui::CollapsingHeader::new("画像の調整")
            .id_salt("adjust")
            .show(ui, |ui| {
                ui.add_enabled_ui(
                    !locked && !self.is_editing(&self.decals[index].picture),
                    |ui| {
                        let settings = &mut self.adjust;
                        ui.horizontal(|ui| {
                            if ui
                                .button("背景を透明にする")
                                .on_hover_text("縁につながっている、縁と同じような色を透明にします")
                                .clicked()
                            {
                                chosen = Some(Adjust::ClearBackground(settings.tolerance));
                            }
                            ui.add(
                                egui::DragValue::new(&mut settings.tolerance)
                                    .range(1.0..=200.0)
                                    .speed(0.5)
                                    .max_decimals(0),
                            )
                            .on_hover_text("同じ色とみなす幅。大きいほど、多く透明になります");
                        });
                        if ui
                            .button("陰影をならす")
                            .on_hover_text("撮った写真の、照明のむらや影を薄くします")
                            .clicked()
                        {
                            chosen = Some(Adjust::FlattenShading);
                        }
                        ui.horizontal(|ui| {
                            if ui
                                .button("色数を絞る")
                                .on_hover_text("似た色をまとめて、この数の色にします")
                                .clicked()
                            {
                                chosen = Some(Adjust::Posterize(settings.colors));
                            }
                            ui.add(
                                egui::DragValue::new(&mut settings.colors)
                                    .range(2..=64)
                                    .suffix(" 色"),
                            );
                        });
                        if ui
                            .button("余白を切り落とす")
                            .on_hover_text(
                                "まわりの透明な部分を切り落とします。見える位置は変わりません",
                            )
                            .clicked()
                        {
                            chosen = Some(Adjust::Trim);
                        }
                        ui.weak(
                            "元の素材は変えず、この柄だけを、調整した新しい素材に差し替えます。",
                        );
                    },
                );
                let enabled = !locked && !self.is_editing(&self.decals[index].picture);
                self.vectorize_controls(ui, index, enabled);
            });
        if let Some(how) = chosen {
            self.adjust_decal(index, how);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::Initial;
    use super::*;
    use crate::paint::Placed;
    use pakenoba_core::surface::Decal;

    /// 白地の中央に、赤い四角がある絵。
    fn red_on_white(w: usize, h: usize) -> Vec<u8> {
        let mut rgba = vec![255u8; w * h * 4];
        for y in h / 4..h * 3 / 4 {
            for x in w / 4..w * 3 / 4 {
                let o = (y * w + x) * 4;
                rgba[o..o + 4].copy_from_slice(&[220, 20, 20, 255]);
            }
        }
        rgba
    }

    #[test]
    fn 背景を透明にすると_中の絵は残る() {
        let (w, h) = (40, 20);
        let out = clear_background(w, h, &red_on_white(w, h), 40.0).unwrap();
        assert_eq!(out[3], 0, "角は透明");
        assert_eq!(out[((h / 2) * w + w / 2) * 4 + 3], 255, "中は残る");
        // 透明にして切り落とすと、赤い四角だけになる。
        let trimmed = run(Adjust::Trim, w, h, &out).unwrap();
        assert_eq!(trimmed.crop, [10, 5, 20, 10]);
    }

    #[test]
    fn 色数を絞ると_その数以下の色になる() {
        let (w, h) = (32, 32);
        let rgba: Vec<u8> = (0..w * h)
            .flat_map(|i| {
                [
                    (i % 256) as u8,
                    (i * 7 % 256) as u8,
                    (i * 13 % 256) as u8,
                    255,
                ]
            })
            .collect();
        let out = posterize(w, h, &rgba, 4);
        let colors: std::collections::HashSet<[u8; 3]> =
            out.chunks(4).map(|c| [c[0], c[1], c[2]]).collect();
        assert!(colors.len() <= 4, "{}", colors.len());
    }

    #[test]
    fn 陰影をならすと_明るさの差が減る() {
        // 左が暗く、右が明るい、灰色の紙。
        let (w, h) = (60, 20);
        let rgba: Vec<u8> = (0..w * h)
            .flat_map(|i| {
                let v = (100 + (i % w) * 2) as u8;
                [v, v, v, 255]
            })
            .collect();
        let out = flatten_shading(w, h, &rgba);
        let spread =
            |img: &[u8]| img[((h / 2) * w + w - 6) * 4] as i32 - img[((h / 2) * w + 5) * 4] as i32;
        assert!(
            spread(&out).abs() < spread(&rgba).abs() / 2,
            "{} → {}",
            spread(&rgba),
            spread(&out)
        );
    }

    #[test]
    fn 切り落としても_見える位置は変わらず_元に戻せる() {
        let mut app = App::new(&Initial::default());
        let panel = app
            .template
            .panels
            .iter()
            .position(|p| p.name == "正面")
            .unwrap();
        let (w, h) = (40, 20);
        let mut rgba = clear_background(w, h, &red_on_white(w, h), 40.0).unwrap();
        // 赤い四角を、右上へ寄せた絵にする（切り落とすと、中心がずれる）。
        rgba = {
            let mut moved = vec![0u8; w * h * 4];
            for y in 0..h - 4 {
                for x in 6..w {
                    let (from, to) = ((y + 4) * w + x - 6, y * w + x);
                    moved[to * 4..to * 4 + 4].copy_from_slice(&rgba[from * 4..from * 4 + 4]);
                }
            }
            moved
        };
        let picture = Picture::Image {
            w,
            h,
            rgba: Arc::new(rgba),
            vector: false,
        };
        let decal = Decal {
            panel,
            at: [0.5, 0.5],
            size: [40.0, 20.0],
            angle: 0.3,
        };
        app.decals.push(Placed {
            decal,
            picture: picture.clone(),
            back: false,
            layer: Default::default(),
        });
        let index = app.decals.len() - 1;
        // 赤い四角の中心（柄の座標）が、展開図のどこにあるか。
        let red_center = [(16.0 + 10.0) - 20.0, 10.0 - (1.0 + 5.0)];
        let before = app.decals[index]
            .decal
            .place(&app.template)
            .point(red_center);
        let assets = app.assets.len();
        app.adjust_decal(index, Adjust::Trim);
        let after = &app.decals[index];
        assert_eq!(after.decal.size, [20.0, 10.0]);
        let now = after.decal.place(&app.template).point([0.0, 0.0]);
        assert!(
            (now[0] - before[0]).abs() < 1e-3 && (now[1] - before[1]).abs() < 1e-3,
            "{before:?} → {now:?}"
        );
        assert_eq!(app.assets.len(), assets + 1, "新しい素材が足される");
        app.undo();
        assert!(crate::app::assets::same_picture(
            &app.decals[index].picture,
            &picture
        ));
        assert_eq!(app.assets.len(), assets);
    }
}
