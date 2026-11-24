//! 柄の調整を、あとから直せる形で効かせるところ。
//!
//! 調整は、レイヤーの設定（`Layer::adjust`）として持ち、元の絵は変えない。描くたびに、元の絵と調整の並びから、調整した絵を作る。
//! 作った絵は、元の絵と調整の並びを印にして覚えておく。重い調整（被写体を抽出など）は別のスレッドで作り、できるまでは元の絵で描く。
//! できたら、画面を描き直す合図を出す（`generation` が増える）。

use pakenoba_i18n::t;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, OnceLock};

use pakenoba_core::project::{AdjustKind, Adjustment};

use crate::art::PX_PER_MM;
use crate::paint::{Picture, Placed};

/// 画素ごとの調整を、色（0〜1）に効かせる。
pub fn pixel(kind: AdjustKind, c: [f32; 3]) -> [f32; 3] {
    match kind {
        AdjustKind::BrightnessContrast { brightness, contrast } => {
            // 明るさは、黒と白を動かさずに中間を持ち上げる（Photoshop の新しい「明るさ」と同じ考え）。
            let g = 2f32.powf(-(brightness as f32) / 100.0);
            // コントラストは、50% の灰色を軸に広げる（-50 で半分、100 で 2 倍）。
            let k = 1.0 + contrast as f32 / 100.0;
            c.map(|v| (0.5 + (v.clamp(0.0, 1.0).powf(g) - 0.5) * k).clamp(0.0, 1.0))
        }
        AdjustKind::HueSaturation { hue, saturation, lightness } => {
            let [mut h, mut s, mut l] = to_hsl(c);
            h = (h + hue as f32 / 360.0).rem_euclid(1.0);
            let sat = saturation as f32 / 100.0;
            // 灰色（彩度 0）は、上げても灰色のまま。
            s *= 1.0 + sat;
            let light = lightness as f32 / 100.0;
            l = if light < 0.0 { l * (1.0 + light) } else { l + (1.0 - l) * light };
            from_hsl([h, s.clamp(0.0, 1.0), l.clamp(0.0, 1.0)])
        }
        AdjustKind::Levels { black, white, gamma } => {
            let (b, w) = (black as f32 / 255.0, (white as f32 / 255.0).max(black as f32 / 255.0 + 1e-3));
            let g = gamma as f32 / 100.0;
            c.map(|v| ((v - b) / (w - b)).clamp(0.0, 1.0).powf(1.0 / g))
        }
        AdjustKind::Invert => c.map(|v| 1.0 - v),
        AdjustKind::Curves { points, count } => {
            let lut = curve_lut(&points[..count as usize]);
            c.map(|v| lookup(&lut, v))
        }
        AdjustKind::Exposure { exposure, offset, gamma } => {
            // 光の量で計算する（Photoshop と同じく、sRGB をいったん直線にもどす）。
            let gain = 2f32.powf(exposure as f32 / 100.0);
            let g = (gamma as f32 / 100.0).max(0.01);
            c.map(|v| to_srgb((to_linear(v) * gain + offset as f32 / 1000.0).max(0.0).powf(1.0 / g).min(1.0)))
        }
        AdjustKind::Vibrance { vibrance, saturation } => {
            let (mx, mn) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
            let avg = (c[0] + c[1] + c[2]) / 3.0;
            // 自然な彩度: 色の薄い所ほど強く効かせる（肌のような、もともと濃い色はあまり変えない）。
            let amount = vibrance as f32 / 100.0 * if vibrance > 0 { 1.0 - (mx - mn) } else { 1.0 };
            let v = c.map(|x| x + (x - avg) * amount);
            let l = lum3(v);
            let k = 1.0 + saturation as f32 / 100.0;
            v.map(|x| (l + (x - l) * k).clamp(0.0, 1.0))
        }
        AdjustKind::ColorBalance { shadows, midtones, highlights, preserve } => {
            let l = lum3(c);
            let ws = (1.0 - l * 2.0).clamp(0.0, 1.0);
            let wh = (l * 2.0 - 1.0).clamp(0.0, 1.0);
            let wm = 1.0 - (l * 2.0 - 1.0).abs();
            let shift = |k: usize| (shadows[k] as f32 * ws + midtones[k] as f32 * wm + highlights[k] as f32 * wh) / 100.0 * 0.35;
            let v = [c[0] + shift(0), c[1] + shift(1), c[2] + shift(2)].map(|x| x.clamp(0.0, 1.0));
            if preserve { keep_lum(v, l) } else { v }
        }
        AdjustKind::BlackWhite { weights } => {
            // いちばん強い色と、次に強い色の組み合わせ（R、Y、G、C、B、M）で、明るさの重みを決める。
            let w = weights.map(|x| x as f32 / 100.0);
            let mut order = [0usize, 1, 2];
            order.sort_by(|&a, &b| c[b].total_cmp(&c[a]));
            let (hi, mid, lo) = (c[order[0]], c[order[1]], c[order[2]]);
            let primary = [0, 2, 4][order[0]];
            let secondary = match (order[0].min(order[1]), order[0].max(order[1])) {
                (0, 1) => 1,
                (1, 2) => 3,
                _ => 5,
            };
            let gray = (lo + (hi - mid) * w[primary] + (mid - lo) * w[secondary]).clamp(0.0, 1.0);
            [gray; 3]
        }
        AdjustKind::PhotoFilter { color, density, preserve } => {
            let col = color.map(|x| x as f32 / 255.0);
            let norm = lum3(col).max(0.05);
            let d = density as f32 / 100.0;
            let v = [0, 1, 2].map(|i| (c[i] * (1.0 - d + d * col[i] / norm)).clamp(0.0, 1.0));
            if preserve { keep_lum(v, lum3(c)) } else { v }
        }
        AdjustKind::Posterize { levels } => {
            let n = levels.max(2) as f32 - 1.0;
            c.map(|v| (v * n).round() / n)
        }
        AdjustKind::Threshold { level } => [if lum3(c) * 255.0 >= level as f32 { 1.0 } else { 0.0 }; 3],
        AdjustKind::GradientMap { from, to, reverse } => {
            let t = if reverse { 1.0 - lum3(c) } else { lum3(c) };
            [0, 1, 2].map(|i| (from[i] as f32 + (to[i] as f32 - from[i] as f32) * t) / 255.0)
        }
        _ => c,
    }
}

fn lum3(c: [f32; 3]) -> f32 {
    0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]
}

/// 明るさを `l` に保つ。
fn keep_lum(v: [f32; 3], l: f32) -> [f32; 3] {
    let d = l - lum3(v);
    v.map(|x| (x + d).clamp(0.0, 1.0))
}

fn to_linear(v: f32) -> f32 {
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

fn to_srgb(v: f32) -> f32 {
    if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

/// トーンカーブの点（0〜255）から曲線の表を作る。値は 0〜1。
///
/// Photoshop と同じく、自然な 3 次スプライン（両端で曲がりがない、点をすべて通るなめらかな曲線）。
/// 0〜255 の外へ出た所は端で止める（Photoshop でも、上や下に張り付く）。最初の点より左と最後の点より右は、端の点の高さのまま。
pub fn curve_lut(points: &[[u8; 2]]) -> [f32; 256] {
    let mut pts: Vec<(f32, f32)> = points.iter().map(|p| (p[0] as f32, p[1] as f32)).collect();
    pts.sort_by(|a, b| a.0.total_cmp(&b.0));
    pts.dedup_by(|a, b| a.0 == b.0);
    let mut lut = [0f32; 256];
    if pts.len() < 2 {
        for (i, v) in lut.iter_mut().enumerate() {
            *v = i as f32 / 255.0;
        }
        return lut;
    }
    let n = pts.len();
    let h: Vec<f32> = (0..n - 1).map(|i| pts[i + 1].0 - pts[i].0).collect();
    // 各点での 2 階の微分 m を、3 重対角の連立式で解く（両端は 0）。
    let mut m = vec![0f32; n];
    if n > 2 {
        let k = n - 2;
        let mut diag = vec![0f32; k];
        let mut rhs = vec![0f32; k];
        for j in 0..k {
            let i = j + 1;
            diag[j] = 2.0 * (h[i - 1] + h[i]);
            rhs[j] = 6.0 * ((pts[i + 1].1 - pts[i].1) / h[i] - (pts[i].1 - pts[i - 1].1) / h[i - 1]);
        }
        // 前から消して、後ろから代入する（トーマス法）。
        for j in 1..k {
            let w = h[j] / diag[j - 1];
            diag[j] -= w * h[j];
            rhs[j] -= w * rhs[j - 1];
        }
        for j in (0..k).rev() {
            let upper = if j + 1 < k { h[j + 1] * m[j + 2] } else { 0.0 };
            m[j + 1] = (rhs[j] - upper) / diag[j];
        }
    }
    for (x, v) in lut.iter_mut().enumerate() {
        let x = x as f32;
        let y = if x <= pts[0].0 {
            pts[0].1
        } else if x >= pts[n - 1].0 {
            pts[n - 1].1
        } else {
            let i = (0..n - 1).find(|&i| x <= pts[i + 1].0).unwrap_or(n - 2);
            let a = (pts[i + 1].0 - x) / h[i];
            let b = 1.0 - a;
            a * pts[i].1 + b * pts[i + 1].1 + ((a * a * a - a) * m[i] + (b * b * b - b) * m[i + 1]) * h[i] * h[i] / 6.0
        };
        *v = (y / 255.0).clamp(0.0, 1.0);
    }
    lut
}

fn lookup(lut: &[f32; 256], v: f32) -> f32 {
    let f = (v.clamp(0.0, 1.0) * 255.0).min(254.999);
    let i = f as usize;
    lut[i] + (lut[i + 1] - lut[i]) * (f - i as f32)
}

/// 画素ごとの調整を、描く前に 1 回だけ用意したもの（トーンカーブは表にしておく）。
pub enum Prepared {
    Kind(AdjustKind),
    Lut(Box<[f32; 256]>),
}

pub fn prepare(stack: &[AdjustKind]) -> Vec<Prepared> {
    stack.iter().map(|&k| match k {
        AdjustKind::Curves { points, count } => Prepared::Lut(Box::new(curve_lut(&points[..count as usize]))),
        k => Prepared::Kind(k),
    }).collect()
}

/// 用意した調整を、色（0〜255）に効かせる。
pub fn run(ops: &[Prepared], c: [f32; 3]) -> [f32; 3] {
    let mut v = c.map(|x| x / 255.0);
    for op in ops {
        v = match op {
            Prepared::Kind(k) => pixel(*k, v),
            Prepared::Lut(lut) => v.map(|x| lookup(lut, x)),
        };
    }
    v.map(|x| x * 255.0)
}

fn to_hsl([r, g, b]: [f32; 3]) -> [f32; 3] {
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) * 0.5;
    if max - min < 1e-6 {
        return [0.0, 0.0, l];
    }
    let d = max - min;
    let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
    let h = if max == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    [h / 6.0, s, l]
}

fn from_hsl([h, s, l]: [f32; 3]) -> [f32; 3] {
    if s <= 0.0 {
        return [l; 3];
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let f = |t: f32| {
        let t = t.rem_euclid(1.0);
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    [f(h + 1.0 / 3.0), f(h), f(h - 1.0 / 3.0)]
}

/// 調整の並びを、RGBA の絵（アルファは乗算なし）に効かせる。失敗した調整（被写体が見つからないなど）は、飛ばして知らせを返す。
pub fn apply(stack: &[AdjustKind], w: usize, h: usize, rgba: &[u8]) -> (Vec<u8>, Vec<String>) {
    let mut out = rgba.to_vec();
    let mut notes = Vec::new();
    let mut run: Vec<AdjustKind> = Vec::new();
    let flush = |run: &mut Vec<AdjustKind>, out: &mut Vec<u8>| {
        if run.is_empty() {
            return;
        }
        let ops = prepare(run);
        for px in out.chunks_exact_mut(4) {
            let c = self::run(&ops, [px[0] as f32, px[1] as f32, px[2] as f32]);
            for i in 0..3 {
                px[i] = c[i].round().clamp(0.0, 255.0) as u8;
            }
        }
        run.clear();
    };
    for &kind in stack {
        if kind.per_pixel() {
            run.push(kind);
            continue;
        }
        flush(&mut run, &mut out);
        match kind {
            AdjustKind::ClearBackground { tolerance } => match crate::app::adjust::clear_background(w, h, &out, tolerance as f32) {
                Some(done) => out = done,
                None => notes.push(t!("No background that could be made transparent was found along the edges.").to_owned()),
            },
            AdjustKind::FlattenShading => out = crate::app::adjust::flatten_shading(w, h, &out),
            AdjustKind::ReduceColors { colors } => out = crate::app::adjust::posterize(w, h, &out, colors as usize),
            AdjustKind::Subject { fine } => match subject_mask(w, h, &out, fine) {
                Ok(mask) => {
                    for (px, m) in out.chunks_exact_mut(4).zip(mask) {
                        px[3] = ((px[3] as u32 * m as u32 + 127) / 255) as u8;
                    }
                }
                Err(why) => notes.push(why),
            },
            _ => {}
        }
    }
    flush(&mut run, &mut out);
    (out, notes)
}

// ---- 被写体を抽出（U²-Net） ----

/// 軽いモデル（u2netp、Apache-2.0）。アプリに入れてある。
const SMALL_MODEL: &[u8] = include_bytes!("../assets/u2netp.onnx");
/// モデルの入力の大きさ（正方形）。
const SIDE: usize = 320;

type Plan = Arc<tract_onnx::prelude::TypedRunnableModel>;

fn load_plan(bytes: &[u8]) -> Result<Plan, String> {
    use tract_onnx::prelude::*;
    let fail = |e: TractError| t!("Could not read the subject model: {e}");
    tract_onnx::onnx()
        .model_for_read(&mut std::io::Cursor::new(bytes))
        .and_then(|m| m.with_input_fact(0, f32::fact([1, 3, SIDE, SIDE]).into()))
        .and_then(|m| m.into_optimized())
        .and_then(|m| m.into_runnable())
        .map_err(fail)
}

fn small_plan() -> Result<Plan, String> {
    static PLAN: OnceLock<Result<Plan, String>> = OnceLock::new();
    PLAN.get_or_init(|| load_plan(SMALL_MODEL)).clone()
}

fn fine_plan() -> Result<Plan, String> {
    static PLAN: Mutex<Option<Plan>> = Mutex::new(None);
    let mut plan = PLAN.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(plan) = plan.as_ref() {
        return Ok(plan.clone());
    }
    let path = crate::models::fine_model_path().filter(|p| p.is_file()).ok_or_else(|| t!("The high-quality subject model has not been downloaded yet.").to_owned())?;
    let bytes = std::fs::read(&path).map_err(|e| t!("Could not read the subject model: {e}"))?;
    let loaded = load_plan(&bytes)?;
    *plan = Some(loaded.clone());
    Ok(loaded)
}

/// 主な被写体の、白黒のマスク（`w × h`、255 が被写体）。透明な所は白地として見る。
pub fn subject_mask(w: usize, h: usize, rgba: &[u8], fine: bool) -> Result<Vec<u8>, String> {
    use tract_onnx::prelude::*;
    if w == 0 || h == 0 || rgba.len() != w * h * 4 {
        return Err(t!("The picture is empty.").to_owned());
    }
    let plan = if fine { fine_plan()? } else { small_plan()? };
    // 320 × 320 に縮め（縦横比は変える。U²-Net の学習と同じ）、白地に重ねてから、ImageNet の平均と幅でならす。
    let mut input = vec![0f32; 3 * SIDE * SIDE];
    let mut peak = 1e-6f32;
    for y in 0..SIDE {
        for x in 0..SIDE {
            let sx = ((x as f32 + 0.5) * w as f32 / SIDE as f32).min(w as f32 - 1.0) as usize;
            let sy = ((y as f32 + 0.5) * h as f32 / SIDE as f32).min(h as f32 - 1.0) as usize;
            let o = (sy * w + sx) * 4;
            let a = rgba[o + 3] as f32 / 255.0;
            for c in 0..3 {
                let v = (rgba[o + c] as f32 * a + 255.0 * (1.0 - a)) / 255.0;
                input[c * SIDE * SIDE + y * SIDE + x] = v;
                peak = peak.max(v);
            }
        }
    }
    const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
    const STD: [f32; 3] = [0.229, 0.224, 0.225];
    for c in 0..3 {
        for v in &mut input[c * SIDE * SIDE..(c + 1) * SIDE * SIDE] {
            *v = (*v / peak - MEAN[c]) / STD[c];
        }
    }
    let tensor: Tensor = tract_ndarray::Array4::from_shape_vec((1, 3, SIDE, SIDE), input).map_err(|e| e.to_string())?.into();
    let out = plan.run(tvec!(tensor.into())).map_err(|e| t!("Could not find the subject: {e}"))?;
    let view = out.first().and_then(|t| t.to_plain_array_view::<f32>().ok().and_then(|v| v.as_slice().map(<[f32]>::to_vec))).ok_or_else(|| t!("Could not find the subject: {e}", e = "no output"))?;
    if view.len() != SIDE * SIDE {
        return Err(t!("Could not find the subject: {e}", e = "unexpected output"));
    }
    let (lo, hi) = view.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    let span = (hi - lo).max(1e-6);
    // 元の大きさへ、なめらかに戻す。
    let at = |x: usize, y: usize| (view[y.min(SIDE - 1) * SIDE + x.min(SIDE - 1)] - lo) / span;
    let mut mask = vec![0u8; w * h];
    for y in 0..h {
        let fy = ((y as f32 + 0.5) * SIDE as f32 / h as f32 - 0.5).max(0.0);
        let (y0, ty) = (fy.floor() as usize, fy.fract());
        for x in 0..w {
            let fx = ((x as f32 + 0.5) * SIDE as f32 / w as f32 - 0.5).max(0.0);
            let (x0, tx) = (fx.floor() as usize, fx.fract());
            let v = (at(x0, y0) * (1.0 - tx) + at(x0 + 1, y0) * tx) * (1.0 - ty) + (at(x0, y0 + 1) * (1.0 - tx) + at(x0 + 1, y0 + 1) * tx) * ty;
            mask[y * w + x] = (v * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
    Ok(mask)
}

// ---- 絵にする ----

/// 柄の絵を、RGBA の絵にする。画像と文字は、持っている絵をそのまま。ロゴ、帯、図形は、大きさに合わせて描く（長い辺は 2048 px まで）。
pub fn raster(picture: &Picture, size: [f32; 2]) -> Option<(usize, usize, Arc<Vec<u8>>)> {
    match picture {
        Picture::Image { w, h, rgba, .. } => Some((*w, *h, rgba.clone())),
        Picture::Text(art) => Some((art.w, art.h, art.rgba.clone())),
        Picture::Group { .. } | Picture::Adjustment => None,
        _ => {
            let scale = PX_PER_MM.min(2048.0 / size[0].max(size[1]).max(1e-3));
            let (w, h) = (((size[0] * scale).round() as usize).max(1), ((size[1] * scale).round() as usize).max(1));
            let mut out = vec![0u8; w * h * 4];
            for y in 0..h {
                for x in 0..w {
                    let q = [(x as f32 + 0.5) / w as f32 * size[0] - size[0] * 0.5, size[1] * 0.5 - (y as f32 + 0.5) / h as f32 * size[1]];
                    let c = picture.sample(q, size);
                    let o = (y * w + x) * 4;
                    out[o..o + 4].copy_from_slice(&[c[0].round() as u8, c[1].round() as u8, c[2].round() as u8, (c[3] * 255.0).round() as u8]);
                }
            }
            Some((w, h, Arc::new(out)))
        }
    }
}

// ---- 作った絵を覚えておく ----

enum Slot {
    /// 別のスレッドで作っている。
    Running,
    Ready { picture: Picture, used: u64 },
}

struct Cache {
    slots: HashMap<String, Slot>,
    /// 元の絵を、作り終わるまで手放さないように持つ（印に使う場所が、別の絵に使い回されないように）。
    keep: HashMap<String, Picture>,
    tick: u64,
}

static CACHE: LazyLock<Mutex<Cache>> = LazyLock::new(|| Mutex::new(Cache { slots: HashMap::new(), keep: HashMap::new(), tick: 0 }));
/// 別のスレッドで絵ができるたびに増える。画面の側は、これが変わったら描き直す。
static GENERATION: AtomicU64 = AtomicU64::new(0);
/// 絵ができたときに、画面を描き直してもらう相手。
static WAKE: OnceLock<eframe::egui::Context> = OnceLock::new();
/// 調整できなかったときの知らせ（画面の下の帯に出す）。
static NOTES: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// 画面を描き直してもらう相手を覚える（起動したとき）。
pub fn set_wake(ctx: &eframe::egui::Context) {
    let _ = WAKE.set(ctx.clone());
}

/// 画面を描き直してもらう。
pub fn wake() {
    GENERATION.fetch_add(1, Ordering::AcqRel);
    if let Some(ctx) = WAKE.get() {
        ctx.request_repaint();
    }
}

/// 別のスレッドで絵ができた回数。
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Acquire)
}

/// 調整できなかったときの知らせを、取り出す。
pub fn take_notes() -> Vec<String> {
    std::mem::take(&mut *NOTES.lock().unwrap_or_else(|e| e.into_inner()))
}

/// 作っている途中の調整があるか。
pub fn busy() -> bool {
    CACHE.lock().unwrap_or_else(|e| e.into_inner()).slots.values().any(|s| matches!(s, Slot::Running))
}

fn picture_key(picture: &Picture, size: [f32; 2]) -> String {
    match picture {
        Picture::Image { rgba, .. } => format!("i{:p}", Arc::as_ptr(rgba)),
        Picture::Text(art) => format!("t{:p}", Arc::as_ptr(art)),
        Picture::Shape(spec) => format!("s{spec:?}{:?}", size.map(f32::to_bits)),
        Picture::Logo => format!("l{:?}", size.map(f32::to_bits)),
        Picture::Band => format!("b{:?}", size.map(f32::to_bits)),
        Picture::Group { .. } | Picture::Adjustment => String::new(),
    }
}

/// 効かせる調整（目を閉じたものを除く）。
pub fn enabled(stack: &[Adjustment]) -> Vec<AdjustKind> {
    stack.iter().filter(|a| a.on).map(|a| a.kind).collect()
}

/// 柄の、調整を効かせた絵。調整がなければ、元の絵。`wait` なら、できるまで待つ（書き出すとき）。
/// 待たないときは、できていなければ別のスレッドで作り始め、いまは元の絵を返す。
pub fn shown(placed: &Placed, wait: bool) -> Picture {
    let stack = enabled(&placed.layer.adjust);
    // ベクターの柄の、色ごとの見せ方（もとの SVG を覚えているときだけ）。
    let restyle = match (&placed.picture, placed.layer.vector.as_ref().filter(|v| !v.is_plain())) {
        (Picture::Image { rgba, vector: true, .. }, Some(style)) => crate::vector::svg_of(rgba).map(|svg| (svg, style.clone())),
        _ => None,
    };
    if (stack.is_empty() && restyle.is_none()) || matches!(placed.picture, Picture::Group { .. } | Picture::Adjustment) {
        return placed.picture.clone();
    }
    let key = format!("{}|{stack:?}|{:?}", picture_key(&placed.picture, placed.size), restyle.as_ref().map(|(_, style)| style));
    {
        let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        cache.tick += 1;
        let tick = cache.tick;
        match cache.slots.get_mut(&key) {
            Some(Slot::Ready { picture, used }) => {
                *used = tick;
                return picture.clone();
            }
            Some(Slot::Running) if !wait => return placed.picture.clone(),
            Some(Slot::Running) => {}
            None => {
                if !wait {
                    cache.slots.insert(key.clone(), Slot::Running);
                    cache.keep.insert(key.clone(), placed.picture.clone());
                }
            }
        }
    }
    let (picture, size) = (placed.picture.clone(), placed.size);
    let make = move || {
        // 先に、色ごとの見せ方を効かせて SVG から描き直す。
        let picture = match &restyle {
            Some((svg, style)) => match crate::vector::decode_svg(crate::vector::restyle(svg, style).as_bytes()) {
                Ok((w, h, rgba)) => Picture::Image { w, h, rgba: Arc::new(rgba), vector: true },
                Err(_) => picture,
            },
            None => picture,
        };
        if stack.is_empty() {
            return (picture, Vec::new());
        }
        let made = raster(&picture, size).map(|(w, h, rgba)| {
            let (out, notes) = apply(&stack, w, h, &rgba);
            (Picture::Image { w, h, rgba: Arc::new(out), vector: false }, notes)
        });
        made.unwrap_or((picture, Vec::new()))
    };
    if wait {
        let (picture, notes) = make();
        NOTES.lock().unwrap_or_else(|e| e.into_inner()).extend(notes);
        store(key, picture.clone());
        return picture;
    }
    std::thread::spawn(move || {
        let (picture, notes) = make();
        NOTES.lock().unwrap_or_else(|e| e.into_inner()).extend(notes);
        store(key, picture);
        wake();
    });
    placed.picture.clone()
}

/// 作った絵を、すべて忘れる（高画質のモデルを取ってきたときなど、同じ調整でも結果が変わるとき）。
pub fn forget_all() {
    CACHE.lock().unwrap_or_else(|e| e.into_inner()).slots.retain(|_, s| matches!(s, Slot::Running));
}

fn store(key: String, picture: Picture) {
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    cache.tick += 1;
    let tick = cache.tick;
    cache.keep.remove(&key);
    cache.slots.insert(key, Slot::Ready { picture, used: tick });
    // 覚えておくのは 48 個まで。いちばん長く使っていないものから忘れる。
    while cache.slots.len() > 48 {
        let Some(oldest) = cache.slots.iter().filter_map(|(k, s)| if let Slot::Ready { used, .. } = s { Some((k.clone(), *used)) } else { None }).min_by_key(|(_, used)| *used).map(|(k, _)| k) else { break };
        cache.slots.remove(&oldest);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 画素ごとの調整() {
        // 明るさ 0、コントラスト 0 は、変えない。
        let c = [0.2, 0.5, 0.9];
        let same = pixel(AdjustKind::BrightnessContrast { brightness: 0, contrast: 0 }, c);
        assert!(same.iter().zip(c).all(|(a, b)| (a - b).abs() < 1e-5));
        // 明るくすると、中間は上がり、黒と白は動かない。
        let up = pixel(AdjustKind::BrightnessContrast { brightness: 100, contrast: 0 }, [0.0, 0.5, 1.0]);
        assert!(up[0] < 1e-5 && up[1] > 0.6 && (up[2] - 1.0).abs() < 1e-5);
        // 色相を 120 度回すと、赤は緑に。
        let g = pixel(AdjustKind::HueSaturation { hue: 120, saturation: 0, lightness: 0 }, [1.0, 0.0, 0.0]);
        assert!(g[1] > 0.99 && g[0] < 0.01 && g[2] < 0.01, "{g:?}");
        // 彩度 -100 は、灰色。
        let gray = pixel(AdjustKind::HueSaturation { hue: 0, saturation: -100, lightness: 0 }, [1.0, 0.0, 0.0]);
        assert!((gray[0] - gray[1]).abs() < 1e-5 && (gray[1] - gray[2]).abs() < 1e-5);
        // レベル補正: 黒の点より暗いものは黒、白の点より明るいものは白。
        let l = pixel(AdjustKind::Levels { black: 51, white: 204, gamma: 100 }, [0.1, 0.5, 0.9]);
        assert_eq!(l[0], 0.0);
        assert!((l[1] - 0.5).abs() < 0.01 && l[2] == 1.0);
        assert_eq!(pixel(AdjustKind::Invert, [0.25, 0.5, 1.0]), [0.75, 0.5, 0.0]);
    }

    #[test]
    fn 並べた順に効く() {
        // 反転してから黒の点を上げるのと、その逆とでは、結果が違う。
        let px = [40u8, 40, 40, 255];
        let a = apply(&[AdjustKind::Invert, AdjustKind::Levels { black: 100, white: 255, gamma: 100 }], 1, 1, &px).0;
        let b = apply(&[AdjustKind::Levels { black: 100, white: 255, gamma: 100 }, AdjustKind::Invert], 1, 1, &px).0;
        assert_ne!(a, b);
        assert_eq!(b, vec![255, 255, 255, 255]);
    }

    #[test]
    fn 被写体を抽出すると_まわりが透明になる() {
        // 白地の中央に、赤い丸。
        let (w, h) = (96usize, 96usize);
        let mut rgba = vec![255u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let (dx, dy) = (x as f32 - 48.0, y as f32 - 48.0);
                if dx * dx + dy * dy < 24.0 * 24.0 {
                    rgba[(y * w + x) * 4..(y * w + x) * 4 + 3].copy_from_slice(&[220, 30, 40]);
                }
            }
        }
        let mask = subject_mask(w, h, &rgba, false).expect("動く");
        assert!(mask[48 * w + 48] > 200, "中央 {}", mask[48 * w + 48]);
        assert!(mask[2 * w + 2] < 60, "隅 {}", mask[2 * w + 2]);
    }

    /// 高画質のモデル（約 176 MB）を取ってある PC でだけ動かす: `cargo test -- --ignored 高画質`。
    #[test]
    #[ignore]
    fn 高画質のモデルでも_被写体を抽出できる() {
        assert!(crate::models::fine_model_ready(), "モデルがない");
        let (w, h) = (64usize, 64usize);
        let mut rgba = vec![255u8; w * h * 4];
        for y in 16..48 {
            for x in 16..48 {
                rgba[(y * w + x) * 4..(y * w + x) * 4 + 3].copy_from_slice(&[20, 60, 200]);
            }
        }
        let mask = subject_mask(w, h, &rgba, true).expect("動く");
        assert!(mask[32 * w + 32] > 200 && mask[w + 1] < 60, "{} {}", mask[32 * w + 32], mask[w + 1]);
    }

    #[test]
    fn 新しい調整の計算() {
        // トーンカーブ: まっすぐなら変えない。真ん中を持ち上げると明るくなり、両端は動かない。
        let straight = curve_lut(&[[0, 0], [255, 255]]);
        assert!((straight[128] - 128.0 / 255.0).abs() < 1e-4);
        let lifted = curve_lut(&[[0, 0], [128, 180], [255, 255]]);
        assert!(lifted[128] > 0.69 && lifted[0] == 0.0 && (lifted[255] - 1.0).abs() < 1e-5);
        assert!(lifted.windows(2).all(|w| w[1] >= w[0] - 1e-6), "この点なら上がり続ける");
        // Photoshop と同じ自然な 3 次スプライン: 点を通り、外へ出た所は 1 で止まる。
        let bent = curve_lut(&[[0, 0], [51, 153], [255, 255]]);
        assert!((bent[51] - 0.6).abs() < 1e-4);
        assert!((bent[17] * 255.0 - 55.0).abs() < 1.0, "{}", bent[17] * 255.0);
        assert!((bent[68] * 255.0 - 191.0).abs() < 1.0, "{}", bent[68] * 255.0);
        assert_eq!(bent[170], 1.0, "上に張り付く");
        // 露光量 +1 で、直線の光の量が 2 倍。
        let e = pixel(AdjustKind::Exposure { exposure: 100, offset: 0, gamma: 100 }, [0.2; 3]);
        assert!((to_linear(e[0]) - to_linear(0.2) * 2.0).abs() < 1e-3);
        // 白黒: 灰色は灰色のまま。赤は、赤の重みで決まる。
        let w = AdjustKind::BlackWhite { weights: pakenoba_core::project::BLACK_WHITE_DEFAULT };
        assert!((pixel(w, [0.5; 3])[0] - 0.5).abs() < 1e-5);
        assert!((pixel(w, [1.0, 0.0, 0.0])[0] - 0.4).abs() < 1e-5);
        // 2 階調化、ポスタリゼーション、グラデーションマップ。
        assert_eq!(pixel(AdjustKind::Threshold { level: 128 }, [0.6; 3]), [1.0; 3]);
        assert_eq!(pixel(AdjustKind::Threshold { level: 128 }, [0.4; 3]), [0.0; 3]);
        assert_eq!(pixel(AdjustKind::Posterize { levels: 2 }, [0.3, 0.7, 0.5]), [0.0, 1.0, 1.0]);
        let g = pixel(AdjustKind::GradientMap { from: [255, 0, 0], to: [0, 0, 255], reverse: false }, [1.0; 3]);
        assert!(g[2] > 0.99 && g[0] < 0.01);
        // 自然な彩度は、灰色を変えない。カラーバランスで明るさを保つと、明るさは変わらない。
        assert_eq!(pixel(AdjustKind::Vibrance { vibrance: 80, saturation: 0 }, [0.5; 3]), [0.5; 3]);
        let cb = pixel(AdjustKind::ColorBalance { shadows: [0; 3], midtones: [100, 0, -100], highlights: [0; 3], preserve: true }, [0.5; 3]);
        assert!((lum3(cb) - 0.5).abs() < 0.01 && cb[0] > cb[2]);
    }
}
