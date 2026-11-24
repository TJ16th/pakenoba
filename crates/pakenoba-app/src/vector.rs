//! ベクターの柄（SVG）。読む、描く、画像をトレースして SVG にする。
//!
//! 読むのと描くのは craft-svg（Craft のアプリが共有する、制限つきの usvg と resvg）、トレースは VectorCraft の
//! 画像トレース（vectorcraft-trace）に任せる。どちらも、パケノバと同じ MIT か Apache-2.0。
//!
//! 柄としては、SVG を細かい絵（長いほうが [`RASTER_LONG`] px）に描いて持つ。素材には SVG のまま入るので、
//! 外のアプリ（VectorCraft など）で直せて、案件のファイルの中でも線のまま残る。

use pakenoba_i18n::t;
use craft_svg::tree::{self, usvg};

/// SVG を描く絵の、長いほうの画素数。200 mm の柄でも 1 mm あたり 12 px（約 300 dpi）。
pub const RASTER_LONG: f32 = 2400.0;

/// SVG を読んで描くときに使ってよいメモリ（バイト）。
const DRAW_BUDGET: u64 = 512 << 20;

/// トレースで作る経路の点の数の上限。写真を細かくトレースすると、数百万になることがある。
const MAX_ANCHORS: usize = 400_000;

/// SVG（または SVGZ）のファイルか。中身の先頭で見る。
pub fn is_svg(bytes: &[u8]) -> bool {
    craft_svg::sniff::is_svg(bytes) || craft_svg::sniff::is_svgz(bytes)
}

/// usvg は要素の入れ子 1 段ごとにスタックを使うので、大きなスタックの別のスレッドで動かす（tree の説明による）。
fn on_big_stack<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
    std::thread::Builder::new()
        .name("svg".to_owned())
        .stack_size(16 << 20)
        .spawn(work)
        .map_err(|e| e.to_string())?
        .join()
        .map_err(|_| t!("Reading the SVG stopped partway").to_owned())
}

/// SVG のファイルの中身を、RGBA（アルファは乗算なし）に描く。大きさは、SVG の縦横の比のまま、長いほうを [`RASTER_LONG`] px に。
pub fn decode_svg(bytes: &[u8]) -> Result<(usize, usize, Vec<u8>), String> {
    let bytes = bytes.to_vec();
    on_big_stack(move || {
        let tree = tree::parse(&bytes, &tree::Limits::default(), &usvg::Options::default()).map_err(|e| t!("Could not read the SVG: {e}"))?;
        let size = tree.size();
        let (sw, sh) = (size.width(), size.height());
        if !(sw > 0.0 && sh > 0.0) {
            return Err(t!("The SVG has no size").to_owned());
        }
        let scale = RASTER_LONG / sw.max(sh);
        let (w, h) = (((sw * scale).round() as u32).max(1), ((sh * scale).round() as u32).max(1));
        let pixmap = tree::draw::render(&tree, usvg::Transform::from_scale(w as f32 / sw, h as f32 / sh), w, h, 0, DRAW_BUDGET).map_err(|e| t!("Could not draw the SVG: {}", e.message))?;
        // resvg の絵はアルファを乗算しているので、戻す。
        let mut rgba = pixmap.data().to_vec();
        for px in rgba.chunks_exact_mut(4) {
            let a = px[3];
            if a != 0 && a != 255 {
                for c in &mut px[..3] {
                    *c = ((*c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8;
                }
            }
        }
        Ok((w as usize, h as usize, rgba))
    })?
}

/// トレースのプリセット。VectorCraft（と Illustrator）のプリセット名と、画面に出す名前。
pub const PRESETS: [(&str, &str); 15] = [
    ("Auto Color", "Auto Color"),
    ("Flat Logo", "Flat Logo"),
    ("3 Colors", "3 Colors"),
    ("6 Colors", "6 Colors"),
    ("16 Colors", "16 Colors"),
    ("Black and White Logo", "Black and White Logo"),
    ("Line Art", "Line Art"),
    ("Sketched Art", "Sketched Art"),
    ("Silhouettes", "Silhouettes"),
    ("Technical Drawing", "Technical Drawing"),
    ("Outline", "Outline (strokes only)"),
    ("Shades of Gray", "Shades of Gray"),
    ("Low Fidelity Photo", "Photo (low fidelity)"),
    ("High Fidelity Photo", "Photo (high fidelity)"),
    ("Default", "Default (black and white)"),
];

/// トレースの細かい値（Illustrator の画像トレースの「詳細」）。プリセットを選ぶと、プリセットの値で埋まる。
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Tune {
    /// 白黒: この明るさより暗い所を黒にする（0〜255）。
    pub threshold: u8,
    /// カラー（色数の限られたパレット）とグレー: 色の数。
    pub colors: u32,
    /// カラー（フルトーン、自動）: 色の多さ（0〜100）。
    pub color_detail: f32,
    /// パス（0〜100）。大きいほど、元の絵に細かく沿う。
    pub paths: f32,
    /// コーナー（0〜100）。大きいほど、角を多く残す。
    pub corners: f32,
    /// ノイズ（px）。これより小さいまとまりは、形にしない。
    pub noise: u32,
    /// 塗りを作る。
    pub fills: bool,
    /// 細い線を、線（ストローク）にする。
    pub strokes: bool,
}

impl Default for Tune {
    fn default() -> Self {
        Tune::of(&vectorcraft_trace::TraceParams::default())
    }
}

impl Tune {
    fn of(p: &vectorcraft_trace::TraceParams) -> Tune {
        Tune { threshold: p.threshold, colors: p.colors, color_detail: p.color_detail as f32, paths: p.paths as f32, corners: p.corners as f32, noise: p.noise, fills: p.fills, strokes: p.strokes }
    }

    fn apply(&self, p: &mut vectorcraft_trace::TraceParams) {
        p.threshold = self.threshold;
        p.colors = self.colors.clamp(2, 256);
        p.color_detail = self.color_detail.clamp(0.0, 100.0) as f64;
        p.paths = self.paths.clamp(0.0, 100.0) as f64;
        p.corners = self.corners.clamp(0.0, 100.0) as f64;
        p.noise = self.noise.min(1000);
        // 塗りも線も作らないと、何も残らない。
        p.fills = self.fills || !self.strokes;
        p.strokes = self.strokes;
    }
}

/// 色の値として、どれを動かせるか（プリセットの方式で決まる）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ColorKnob {
    /// 白黒: しきい値。
    Threshold,
    /// カラー（限られたパレット）、グレー: 色の数。
    Colors,
    /// カラー（フルトーン、自動）: 色の多さ。
    Detail,
    /// フラットなロゴ: 色は絵から決まる。
    None,
}

/// プリセット `preset`（[`PRESETS`] の番号）の、細かい値と、動かせる色の値。
pub fn preset_tune(preset: usize) -> (Tune, ColorKnob) {
    use vectorcraft_trace::{Mode, Palette};
    let params = PRESETS.get(preset).and_then(|p| vectorcraft_trace::preset(p.0)).unwrap_or_default();
    let knob = match (params.mode, params.palette) {
        (Mode::BlackAndWhite, _) => ColorKnob::Threshold,
        (Mode::Grayscale, _) | (Mode::Color, Palette::Limited | Palette::DocumentLibrary) => ColorKnob::Colors,
        (Mode::Color, _) => ColorKnob::Detail,
        (Mode::Logo, _) => ColorKnob::None,
    };
    (Tune::of(&params), knob)
}

/// ベクター化（画像トレース）の設定。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TraceSettings {
    /// [`PRESETS`] の番号。
    pub preset: usize,
    /// 縁につながる背景の色を、形にしない。
    pub drop_background: bool,
    /// 細かい値。None ならプリセットのまま。
    pub tune: Option<Tune>,
}

impl Default for TraceSettings {
    fn default() -> Self {
        TraceSettings { preset: 0, drop_background: true, tune: None }
    }
}

/// 設定を書くときの形（プリセットは名前で書く。番号は並びを変えると変わるので）。
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SettingsEntry {
    preset: String,
    #[serde(default = "yes")]
    drop_background: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tune: Option<Tune>,
}

fn yes() -> bool {
    true
}

impl TraceSettings {
    /// レイヤーに残す JSON。
    pub fn to_json(self) -> String {
        let preset = PRESETS.get(self.preset).map_or("Auto Color", |p| p.0).to_owned();
        serde_json::to_string(&SettingsEntry { preset, drop_background: self.drop_background, tune: self.tune }).unwrap_or_default()
    }

    /// レイヤーに残した JSON から。読めなければ None。
    pub fn from_json(text: &str) -> Option<TraceSettings> {
        let entry: SettingsEntry = serde_json::from_str(text).ok()?;
        let preset = PRESETS.iter().position(|p| p.0 == entry.preset).unwrap_or(0);
        Some(TraceSettings { preset, drop_background: entry.drop_background, tune: entry.tune })
    }

    /// いまの細かい値（プリセットのままなら、プリセットの値）。
    pub fn tune_now(&self) -> Tune {
        self.tune.unwrap_or_else(|| preset_tune(self.preset).0)
    }
}

/// トレースした結果。
pub struct Traced {
    /// SVG の中身。
    pub svg: String,
    /// アウトライン（パスの形だけを、細い黒い線で描いた SVG。確かめるため）。
    pub outline: String,
    /// 形の数。
    pub paths: usize,
    /// 色の数。
    pub colors: usize,
}

fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// 縁で一番多い色（透明でないもの）。
fn border_color(w: usize, h: usize, rgba: &[u8]) -> Option<[u8; 3]> {
    let mut counts = std::collections::HashMap::<[u8; 3], usize>::new();
    let border = (0..w).flat_map(|x| [x, (h - 1) * w + x]).chain((0..h).flat_map(|y| [y * w, y * w + w - 1]));
    for i in border {
        if rgba[i * 4 + 3] >= 128 {
            *counts.entry([rgba[i * 4] & 0xF8, rgba[i * 4 + 1] & 0xF8, rgba[i * 4 + 2] & 0xF8]).or_default() += 1;
        }
    }
    counts.into_iter().max_by_key(|(_, n)| *n).map(|(c, _)| c)
}

/// RGBA（アルファは乗算なし）の絵を、VectorCraft の画像トレースで SVG にする。`size_mm` は SVG に書く実寸。
pub fn trace(w: usize, h: usize, rgba: &[u8], settings: TraceSettings, size_mm: [f32; 2]) -> Result<Traced, String> {
    let name = PRESETS.get(settings.preset).map_or("Auto Color", |p| p.0);
    let mut params = vectorcraft_trace::preset(name).ok_or(t!("The trace preset was not found"))?;
    if let Some(tune) = settings.tune {
        tune.apply(&mut params);
    }
    if settings.drop_background {
        params.ignore_white = true;
        params.ignore_background = true;
        params.ignore_color = border_color(w, h, rgba).map(hex);
    }
    let raster = vectorcraft_trace::Raster::new(w as u32, h as u32, rgba.to_vec()).map_err(|e| t!("Could not trace: {e}"))?;
    let result = vectorcraft_trace::trace_within(&raster, &params, MAX_ANCHORS).map_err(|e| match e {
        vectorcraft_trace::TraceError::TooLarge { .. } => t!("The image is too large to trace.").to_owned(),
        vectorcraft_trace::TraceError::NotFlat { .. } => t!("This is not a flat-color logo, so Flat Logo cannot be used. Choose Auto Color or a photo preset.").to_owned(),
        other => t!("Could not trace: {other}"),
    })?;
    if result.paths.is_empty() {
        return Err(t!("Tracing made no shapes. Try another preset or change the background setting.").to_owned());
    }
    // 実寸（mm）と、画素の座標の見る範囲。外のアプリで開くと、柄と同じ大きさで出る。
    let head = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{:.2}mm\" height=\"{:.2}mm\" viewBox=\"0 0 {w} {h}\">\n",
        size_mm[0], size_mm[1]
    );
    let mut svg = head.clone();
    svg.push_str("<!-- PAKENOBA: traced with VectorCraft's Image Trace -->\n");
    // アウトラインの線の太さ（画素）。絵の大きさによらず、同じくらいの細さに見えるように。
    let hair = (w.max(h) as f32 / 700.0).max(0.6);
    let mut outline = head;
    let mut colors = std::collections::HashSet::new();
    for path in &result.paths {
        let d = craft_svg::write::path_data_fixed(&path.path.to_bezpath(), 2);
        let color = hex(path.color);
        colors.insert(path.color);
        match path.stroke {
            Some(width) => {
                let cap = if path.square_caps { "square" } else { "round" };
                svg.push_str(&format!("<path fill=\"none\" stroke=\"{color}\" stroke-width=\"{width:.2}\" stroke-linecap=\"{cap}\" stroke-linejoin=\"round\" d=\"{d}\"/>\n"));
            }
            None => svg.push_str(&format!("<path fill=\"{color}\" d=\"{d}\"/>\n")),
        }
        outline.push_str(&format!("<path fill=\"none\" stroke=\"#1b1b1b\" stroke-width=\"{hair:.2}\" stroke-linejoin=\"round\" d=\"{d}\"/>\n"));
    }
    svg.push_str("</svg>\n");
    outline.push_str("</svg>\n");
    Ok(Traced { svg, outline, paths: result.paths.len(), colors: colors.len() })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 白地に、赤い丸と青い四角。
    fn sample(w: usize, h: usize) -> Vec<u8> {
        (0..w * h)
            .flat_map(|i| {
                let (x, y) = ((i % w) as f32, (i / w) as f32);
                if (x - 40.0).hypot(y - 30.0) < 18.0 {
                    [200, 40, 40, 255]
                } else if (55.0..85.0).contains(&x) && (25.0..50.0).contains(&y) {
                    [30, 90, 180, 255]
                } else {
                    [245, 244, 240, 255]
                }
            })
            .collect()
    }

    #[test]
    fn トレースした_svg_を読んで描ける() {
        let (w, h) = (100, 60);
        let traced = trace(w, h, &sample(w, h), TraceSettings::default(), [50.0, 30.0]).unwrap();
        assert!(traced.paths >= 2, "{}", traced.paths);
        assert!(traced.svg.contains("width=\"50.00mm\""));
        assert!(is_svg(traced.svg.as_bytes()));
        let (rw, rh, rgba) = decode_svg(traced.svg.as_bytes()).unwrap();
        assert_eq!((rw, rh), (2400, 1440));
        let at = |x: usize, y: usize| &rgba[(y * rw + x) * 4..(y * rw + x) * 4 + 4];
        // 背景は形にしないので、角は透明。丸の中は赤。
        assert_eq!(at(5, 5)[3], 0);
        let red = at(40 * 24, 30 * 24);
        assert!(red[0] > 150 && red[2] < 100 && red[3] == 255, "{red:?}");
    }

    #[test]
    fn 背景を残すと_角も形になる() {
        let (w, h) = (100, 60);
        let settings = TraceSettings { drop_background: false, ..Default::default() };
        let traced = trace(w, h, &sample(w, h), settings, [50.0, 30.0]).unwrap();
        let (rw, _, rgba) = decode_svg(traced.svg.as_bytes()).unwrap();
        assert_eq!(rgba[(5 * rw + 5) * 4 + 3], 255);
    }

    #[test]
    fn すべてのプリセットが_vectorcraft_にある() {
        for (name, _) in PRESETS {
            assert!(vectorcraft_trace::preset(name).is_some(), "{name}");
        }
    }

    #[test]
    fn 壊れた_svg_は_落ちずに知らせる() {
        assert!(decode_svg(b"<svg xmlns=\"http://www.w3.org/2000/svg\"><path d=").is_err());
        assert!(!is_svg(b"\x89PNG...."));
    }
}

/// SVG のいちばん外の `width` と `height` が mm（か cm）で書いてあれば、その実寸（mm）。
pub fn size_mm(bytes: &[u8]) -> Option<[f32; 2]> {
    // 先頭だけを見る（途中で切れた文字は、置き換えて読む）。
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]);
    let start = text.find("<svg")?;
    let tag = &text[start..start + text[start..].find('>')?];
    let read = |key: &str| -> Option<f32> {
        let at = tag.find(&format!(" {key}=\""))? + key.len() + 3;
        let value = &tag[at..at + tag[at..].find('"')?];
        let (number, factor) = match value.strip_suffix("mm") {
            Some(n) => (n, 1.0),
            None => (value.strip_suffix("cm")?, 10.0),
        };
        number.trim().parse::<f32>().ok().filter(|v| *v > 0.0).map(|v| v * factor)
    };
    Some([read("width")?, read("height")?])
}

#[cfg(test)]
mod size_tests {
    #[test]
    fn svg_の実寸を読む() {
        assert_eq!(super::size_mm(br#"<svg xmlns="x" width="120mm" height="60mm" viewBox="0 0 10 5">"#), Some([120.0, 60.0]));
        assert_eq!(super::size_mm(br#"<svg width="2cm" height="1cm">"#), Some([20.0, 10.0]));
        assert_eq!(super::size_mm(br#"<svg width="2048" height="2048">"#), None);
    }
}
